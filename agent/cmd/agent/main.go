// honey agent.
//
// runs on each node. serves AgentService over gRPC+mTLS and owns the local
// sing-box process. two transports (pick with --mode):
//   - serve: agent listens, master dials in
//   - dial:  agent dials the master (for nodes behind NAT)
//   - both:  run them at once
package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"

	honeyv1 "github.com/akiko99x/honey/agent/gen/honey/v1"
	"github.com/akiko99x/honey/agent/internal/accounting"
	"github.com/akiko99x/honey/agent/internal/config"
	"github.com/akiko99x/honey/agent/internal/core"
	"github.com/akiko99x/honey/agent/internal/grpcserver"
	"github.com/akiko99x/honey/agent/internal/logx"
	"github.com/akiko99x/honey/agent/internal/mtls"
	"github.com/akiko99x/honey/agent/internal/singbox"
	"github.com/akiko99x/honey/agent/internal/transport"
	"github.com/akiko99x/honey/agent/internal/xray"
	"github.com/akiko99x/honey/agent/internal/xrayacme"
)

func main() {
	cfg := config.Parse()
	logx.Info(logx.AgentBooting, "agent booting up, node=%s", cfg.NodeID)

	tlsCfg, err := mtls.ServerConfig(cfg.CAFile, cfg.CertFile, cfg.KeyFile)
	if err != nil {
		logx.Fatal(logx.AgentMTLSFailed, "mtls setup failed, can't trust anyone: %v", err)
	}

	// two parallel cores on one node: sing-box (priority) + xray.
	clash := singbox.NewClash(cfg.ClashURL, cfg.ClashSecret)
	sb := singbox.NewManager(cfg.SingboxBin, cfg.SingboxConfig, clash)
	xr := xray.NewManager(cfg.XrayBin, cfg.XrayConfig, cfg.XrayAPI)
	cores := map[string]core.Manager{
		"singbox": sb,
		"xray":    xr,
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	certManager := xrayacme.New(
		cfg.XrayACMERoot,
		cfg.XrayACMEListen,
		cfg.SingboxACMEUpstream,
	)
	certManager.SetReload(func() error {
		state, _, _ := xr.Status()
		if state != core.StateRunning {
			return nil
		}
		return xr.Apply("")
	})
	if err := certManager.Start(ctx); err != nil {
		logx.Fatal(logx.AgentACMEGateway, "Xray ACME gateway failed: %v", err)
	}

	// durable per-user accounting: reload persisted counters + epoch so stats
	// survive an agent restart and keep growing while the master is offline.
	statsStore := accounting.Load(filepath.Join(filepath.Dir(cfg.SingboxConfig), "honey-stats.json"))
	seedUp, seedDown := statsStore.Counters()
	sb.SeedUsers(seedUp, seedDown)

	// restart recovery: the data-plane must survive a node reboot without the
	// master. each core persisted its last-applied config on disk, so resume it
	// now instead of waiting in StateStopped for the next push. sing-box first.
	recoverCore("singbox", cores["singbox"], cfg.SingboxConfig)
	recoverCore("xray", cores["xray"], cfg.XrayConfig)

	// one grpc server, shared by every transport. mTLS is enforced here, so the
	// transports stay pure connection plumbing.
	srv := grpc.NewServer(grpc.Creds(credentials.NewTLS(tlsCfg)))
	agentSrv := grpcserver.New(cfg.NodeID, cores, map[string]string{
		"singbox": cfg.SingboxConfig,
		"xray":    cfg.XrayConfig,
	})
	agentSrv.SetStatsEpoch(statsStore.Epoch)
	agentSrv.SetACMEManager(certManager)
	honeyv1.RegisterAgentServiceServer(srv, agentSrv)

	transports, err := buildTransports(cfg)
	if err != nil {
		logx.Fatal(logx.AgentTransportWiring, "transport wiring failed: %v", err)
	}

	var wg sync.WaitGroup
	for _, t := range transports {
		wg.Add(1)
		go func(t transport.Transport) {
			defer wg.Done()
			logx.Info(logx.AgentTransportUp, "transport %s is up, listening", t.Name())
			if err := t.Run(ctx, srv); err != nil {
				logx.Warn(logx.AgentTransportDown, "transport %s went down: %v", t.Name(), err)
			}
		}(t)
	}

	// always-on accounting + local-quota loop: independent of any master Stats
	// stream, so per-user counters advance and over-quota users are cut even
	// while the master is offline. Persist counters periodically for durability.
	wg.Add(1)
	go func() {
		defer wg.Done()
		accountingLoop(ctx, sb, agentSrv, statsStore)
	}()

	<-ctx.Done()
	logx.Info(logx.AgentShutdown, "shutting down, waving goodbye...")
	_ = statsStore.Save()
	srv.GracefulStop()
	wg.Wait()
}

func accountingLoop(ctx context.Context, sb *singbox.Manager, srv *grpcserver.Server, store *accounting.Store) {
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	var ticks uint64
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			if !sb.Poll(ctx) {
				continue // clash unreachable this tick
			}
			srv.EnforceQuota(ctx)
			up, down := sb.UserCounters()
			store.SetCounters(up, down)
			ticks++
			if ticks%15 == 0 { // ~every 30s
				if err := store.Save(); err != nil {
					logx.Warn(logx.CoreQuota, "stats persist failed: %v", err)
				}
			}
		}
	}
}

// recoverCore resumes a core from its persisted config after an agent restart.
// Start("") re-runs the core against the existing config file (validating it
// first) without rewriting it. A missing config means nothing was applied yet;
// a broken one is logged but never fatal — the master can re-push either way.
func recoverCore(name string, m core.Manager, configPath string) {
	resume, err := core.ShouldRecover(configPath)
	if err != nil {
		logx.Warn(logx.AgentResumeFailed, "could not resume %s: %v", name, err)
		return
	}
	if !resume {
		logx.Info(logx.AgentResumeEmpty, "no active saved config for %s, waiting for a push", name)
		return
	}
	if err := m.Start(""); err != nil {
		logx.Warn(logx.AgentResumeFailed, "could not resume %s: %v", name, err)
		return
	}
	logx.Info(logx.AgentResumed, "resumed %s from last config, vpn is back", name)
}

func buildTransports(cfg *config.Config) ([]transport.Transport, error) {
	switch cfg.Mode {
	case "serve":
		return []transport.Transport{transport.NewServe(cfg.Listen)}, nil
	case "dial":
		if cfg.MasterAddr == "" {
			return nil, fmt.Errorf("dial mode needs --master-addr")
		}
		return []transport.Transport{transport.NewDial(cfg.MasterAddr)}, nil
	case "both":
		if cfg.MasterAddr == "" {
			return nil, fmt.Errorf("both mode needs --master-addr")
		}
		return []transport.Transport{
			transport.NewServe(cfg.Listen),
			transport.NewDial(cfg.MasterAddr),
		}, nil
	default:
		return nil, fmt.Errorf("unknown mode %q (want serve|dial|both)", cfg.Mode)
	}
}
