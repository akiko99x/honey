// Package grpcserver implements the AgentService the master calls into.
// It fans control calls out to the node's cores (sing-box + xray).
package grpcserver

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	honeyv1 "github.com/akiko99x/honey/agent/gen/honey/v1"
	"github.com/akiko99x/honey/agent/internal/core"
	"github.com/akiko99x/honey/agent/internal/firewall"
	"github.com/akiko99x/honey/agent/internal/localquota"
	"github.com/akiko99x/honey/agent/internal/logx"
	"github.com/akiko99x/honey/agent/internal/svc"
	"github.com/akiko99x/honey/agent/internal/sysmetrics"
	"github.com/akiko99x/honey/agent/internal/wg"
	"github.com/akiko99x/honey/agent/internal/xrayacme"
)

const agentVersion = "0.1.2"

// Server implements honeyv1.AgentServiceServer.
type Server struct {
	honeyv1.UnimplementedAgentServiceServer

	nodeID     string
	startedAt  int64
	statsEpoch string
	cores      map[string]core.Manager // "singbox", "xray"
	configPath map[string]string
	acme       *xrayacme.Manager

	quotaMu       sync.Mutex
	quotaBytes    map[string]uint64 // per-user remaining quota from the last push
	quotaBaseline map[string]uint64 // per-user cumulative at the last push
	suppressed    map[string]bool   // users currently over their local quota
}

// accountingCore is the subset of the sing-box manager the quota/accounting
// loop needs. Implemented by *singbox.Manager.
type accountingCore interface {
	Poll(ctx context.Context) bool
	UserTotals() map[string]uint64
	UserCounters() (map[string]uint64, map[string]uint64)
	Connections(ctx context.Context) ([]core.LiveConn, error)
	CloseConnections(ctx context.Context, ids []string) (uint32, error)
}

func (s *Server) singboxAccounting() (accountingCore, bool) {
	mgr, ok := s.cores["singbox"]
	if !ok {
		return nil, false
	}
	ac, ok := mgr.(accountingCore)
	return ac, ok
}

// SetStatsEpoch overrides the generated epoch with a persisted one so counters
// stay continuous across an agent restart.
func (s *Server) SetStatsEpoch(epoch string) {
	if epoch != "" {
		s.statsEpoch = epoch
	}
}

// SetACMEManager enables Honey-managed HTTP-01 certificates for Xray.
func (s *Server) SetACMEManager(manager *xrayacme.Manager) {
	s.acme = manager
}

// Accounting exposes the sing-box accounting core to the agent's background
// loop (nil if sing-box is absent).
func (s *Server) Accounting() (accountingCore, bool) { return s.singboxAccounting() }

// captureQuota snapshots per-user quota + baseline on a successful push, and
// clears any local suppression (the master's fresh state is authoritative).
func (s *Server) captureQuota(spec core.Spec) {
	quota := map[string]uint64{}
	for _, ib := range spec.Inbounds {
		for _, u := range ib.Users {
			if u.QuotaBytes > 0 {
				quota[u.Name] = u.QuotaBytes
			}
		}
	}
	baseline := map[string]uint64{}
	if ac, ok := s.singboxAccounting(); ok {
		baseline = ac.UserTotals()
	}
	s.quotaMu.Lock()
	s.quotaBytes = quota
	s.quotaBaseline = baseline
	s.suppressed = map[string]bool{}
	s.quotaMu.Unlock()
}

// EnforceQuota cuts the active connections of users who have exceeded their
// pushed remaining quota since the last push. This is a local stopgap between
// master pushes (which apply the authoritative disable); creds stay valid, so
// the effect is repeated session cutoff until the master catches up.
func (s *Server) EnforceQuota(ctx context.Context) {
	ac, ok := s.singboxAccounting()
	if !ok {
		return
	}
	totals := ac.UserTotals()
	s.quotaMu.Lock()
	suppressed := localquota.Decide(totals, s.quotaBaseline, s.quotaBytes)
	changed := localquota.Changed(suppressed, s.suppressed)
	s.suppressed = suppressed
	s.quotaMu.Unlock()
	if len(suppressed) == 0 {
		return
	}
	conns, err := ac.Connections(ctx)
	if err != nil {
		return
	}
	var ids []string
	for _, c := range conns {
		if suppressed[c.User] {
			ids = append(ids, c.ID)
		}
	}
	if len(ids) == 0 {
		return
	}
	if n, err := ac.CloseConnections(ctx, ids); err == nil && n > 0 && changed {
		logx.Info(logx.CoreQuota, "local quota: cut %d connection(s) for %d over-quota user(s)", n, len(suppressed))
	}
}

func New(nodeID string, cores map[string]core.Manager, configPath ...map[string]string) *Server {
	paths := map[string]string{}
	if len(configPath) > 0 {
		paths = configPath[0]
	}
	return &Server{
		nodeID:        nodeID,
		startedAt:     time.Now().Unix(),
		statsEpoch:    newStatsEpoch(),
		cores:         cores,
		configPath:    paths,
		quotaBytes:    map[string]uint64{},
		quotaBaseline: map[string]uint64{},
		suppressed:    map[string]bool{},
	}
}

func (s *Server) markActive(kind string) error {
	if path := s.configPath[kind]; path != "" {
		return core.MarkActive(path)
	}
	return nil
}

func (s *Server) markInactive(kind string) error {
	if path := s.configPath[kind]; path != "" {
		return core.MarkInactive(path)
	}
	return nil
}

// WhoRU — introduce this node, with both core versions.
func (s *Server) WhoRU(ctx context.Context, _ *honeyv1.WhoRURequest) (*honeyv1.NodeIdentity, error) {
	logx.Debug(logx.AgentWhoRU, "master asked who r u, telling them")
	host, _ := os.Hostname()
	return &honeyv1.NodeIdentity{
		NodeId:         s.nodeID,
		Hostname:       host,
		AgentVersion:   agentVersion,
		SingboxVersion: s.version(ctx, "singbox"),
		XrayVersion:    s.version(ctx, "xray"),
		Os:             runtime.GOOS + "/" + runtime.GOARCH,
		StartedAt:      s.startedAt,
	}, nil
}

// Ping — echo the client clock, add ours.
func (s *Server) Ping(_ context.Context, req *honeyv1.PingRequest) (*honeyv1.PingReply, error) {
	return &honeyv1.PingReply{SentAt: req.GetSentAt(), RecvAt: time.Now().UnixMilli()}, nil
}

// Apply — node-level: group inbounds by core, (re)build each core's config, and
// stop any core left with no inbounds. this is what the master reconcile calls.
func (s *Server) Apply(ctx context.Context, req *honeyv1.ApplyRequest) (*honeyv1.CoreStatus, error) {
	spec := specFromProto(req.GetSpec())
	logx.Info(logx.AgentApplyRecv, "apply came in: %d inbound(s)", len(spec.Inbounds))
	s.syncPortHopping(spec.Inbounds)
	// WireGuard is a separate data-plane; failures here never block the cores.
	if len(spec.Wireguard) > 0 {
		if err := wg.Apply(spec.Wireguard); err != nil {
			logx.Warn(logx.CoreWireguard, "wireguard apply: %v", err)
		} else {
			logx.Info(logx.CoreWireguard, "wireguard: %d interface(s) up", len(spec.Wireguard))
		}
	}
	// managed external services (mtproto/naive) — separate daemons, best-effort.
	if len(spec.Services) > 0 {
		if err := svc.Apply(spec.Services); err != nil {
			logx.Warn(logx.CoreService, "service apply: %v", err)
		} else {
			logx.Info(logx.CoreService, "services: %d daemon(s) up", len(spec.Services))
		}
	}

	cleanup, err := s.prepareACME(ctx, &spec, false)
	if err != nil {
		return s.aggregateStatus(fmt.Errorf("prepare ACME candidate: %w", err)), nil
	}
	configs, kinds, err := s.validateCandidate(spec)
	cleanup()
	if err != nil {
		return s.aggregateStatus(err), nil
	}
	if s.acme != nil {
		cleanup, err = s.prepareACME(ctx, &spec, true)
		if err != nil {
			return s.aggregateStatus(fmt.Errorf("activate ACME certificate: %w", err)), nil
		}
		defer cleanup()
		configs, kinds, err = s.validateCandidate(spec)
		if err != nil {
			return s.aggregateStatus(err), nil
		}
	}

	// Stop obsolete cores only after every replacement passed validation.
	stopped := make([]core.Manager, 0, len(kinds))
	for _, kind := range kinds {
		if _, wanted := configs[kind]; wanted {
			continue
		}
		mgr := s.cores[kind]
		state, _, _ := mgr.Status()
		if err := s.markInactive(kind); err != nil {
			return s.aggregateStatus(fmt.Errorf("mark %s inactive: %w", kind, err)), nil
		}
		if err := mgr.Stop(); err != nil {
			if state == core.StateRunning {
				_ = s.markActive(kind)
			}
			return s.aggregateStatus(fmt.Errorf("stop %s: %w", kind, err)), nil
		}
		if state == core.StateRunning {
			stopped = append(stopped, mgr)
		}
	}

	for _, kind := range kinds {
		cfg, wanted := configs[kind]
		if !wanted {
			continue
		}
		if err := s.cores[kind].Apply(cfg); err != nil {
			restoreErrs := make([]string, 0, len(stopped))
			for _, old := range stopped {
				if restoreErr := old.Start(""); restoreErr != nil {
					restoreErrs = append(restoreErrs, restoreErr.Error())
				}
			}
			if len(restoreErrs) > 0 {
				err = fmt.Errorf("%w; restore previous cores: %s", err, strings.Join(restoreErrs, "; "))
			}
			for _, restoredKind := range kinds {
				if _, wanted := configs[restoredKind]; wanted {
					continue
				}
				if state, _, _ := s.cores[restoredKind].Status(); state == core.StateRunning {
					_ = s.markActive(restoredKind)
				}
			}
			return s.aggregateStatus(fmt.Errorf("apply %s config: %w", kind, err)), nil
		}
		if err := s.markActive(kind); err != nil {
			return s.aggregateStatus(fmt.Errorf("mark %s active: %w", kind, err)), nil
		}
	}
	for _, kind := range kinds {
		if _, wanted := configs[kind]; wanted {
			continue
		}
		if state, _, _ := s.cores[kind].Status(); state == core.StateRunning {
			if err := s.markActive(kind); err != nil {
				return s.aggregateStatus(fmt.Errorf("mark %s active: %w", kind, err)), nil
			}
		}
	}
	// snapshot per-user quota + baseline for local (offline) enforcement.
	s.captureQuota(spec)
	return s.aggregateStatus(nil), nil
}

// Validate performs the complete build/check phase without touching firewall,
// marker files, live configs, or core processes.
func (s *Server) Validate(ctx context.Context, req *honeyv1.ApplyRequest) (*honeyv1.CoreStatus, error) {
	spec := specFromProto(req.GetSpec())
	logx.Info(logx.AgentApplyRecv, "dry-run came in: %d inbound(s)", len(spec.Inbounds))
	cleanup, err := s.prepareACME(ctx, &spec, false)
	if err != nil {
		logx.Warn(logx.CoreConfigBuildFailed, "ACME candidate rejected: %v", err)
		return errStatus(honeyv1.CoreKind_CORE_KIND_UNSPECIFIED, fmt.Errorf("candidate configuration rejected; inspect agent logs")), nil
	}
	defer cleanup()
	_, _, err = s.validateCandidate(spec)
	if err != nil {
		logx.Warn(logx.CoreConfigBuildFailed, "candidate rejected: %v", err)
		return errStatus(honeyv1.CoreKind_CORE_KIND_UNSPECIFIED, fmt.Errorf("candidate configuration rejected; inspect agent logs")), nil
	}
	return &honeyv1.CoreStatus{
		Core:    honeyv1.CoreKind_CORE_KIND_UNSPECIFIED,
		State:   honeyv1.CoreState_CORE_STATE_STOPPED,
		Message: "candidate configuration is valid; no changes applied",
	}, nil
}

func (s *Server) validateCandidate(spec core.Spec) (map[string]string, []string, error) {
	byCore := map[string][]core.Inbound{}
	for _, ib := range spec.Inbounds {
		kind := coreOfInbound(ib)
		byCore[kind] = append(byCore[kind], ib)
	}

	kinds := make([]string, 0, len(s.cores))
	for kind := range s.cores {
		kinds = append(kinds, kind)
	}
	sort.Strings(kinds)

	// Phase one is read-only: build and validate every desired core config.
	// A bad candidate must never take an unrelated healthy core down.
	configs := make(map[string]string, len(kinds))
	for _, kind := range kinds {
		mgr := s.cores[kind]
		subset := byCore[kind]
		if len(subset) == 0 {
			continue
		}
		coreSpec := spec
		coreSpec.Inbounds = subset
		logx.Debug(logx.CoreConfigBuilding, "building %s config, %d inbound(s)", kind, len(subset))
		cfg, err := mgr.BuildConfig(coreSpec)
		if err != nil {
			logx.Error(logx.CoreConfigBuildFailed, "%s config build blew up: %v", kind, err)
			return nil, kinds, fmt.Errorf("build %s config: %w", kind, err)
		}
		if err := mgr.Validate(cfg); err != nil {
			return nil, kinds, fmt.Errorf("validate %s config: %w", kind, err)
		}
		configs[kind] = cfg
	}
	return configs, kinds, nil
}

// Start — bring one core up from the spec's inbounds for that core.
func (s *Server) Start(ctx context.Context, req *honeyv1.StartRequest) (*honeyv1.CoreStatus, error) {
	kind := coreKey(req.GetCore())
	logx.Info(logx.AgentStartRecv, "master says start %s", kind)
	mgr, ok := s.cores[kind]
	if !ok {
		logx.Warn(logx.AgentUnknownCore, "master asked for core %q, don't have it", kind)
		return errStatus(req.GetCore(), fmt.Errorf("unknown core %q", kind)), nil
	}
	spec := specFromProto(req.GetSpec())
	spec.Inbounds = filterCore(spec.Inbounds, kind)

	cleanup := func() {}
	var err error
	if kind == "xray" || kind == "hysteria" {
		cleanup, err = s.prepareACME(ctx, &spec, true)
	}
	if err != nil {
		return coreStatus(req.GetCore(), mgr, err), nil
	}
	defer cleanup()
	cfg, err := mgr.BuildConfig(spec)
	if err == nil {
		err = mgr.Start(cfg)
	}
	if err == nil {
		err = s.markActive(kind)
	}
	return coreStatus(req.GetCore(), mgr, err), nil
}

// Stop — take one core down.
func (s *Server) Stop(_ context.Context, req *honeyv1.StopRequest) (*honeyv1.CoreStatus, error) {
	kind := coreKey(req.GetCore())
	logx.Info(logx.AgentStopRecv, "master says stop %s", kind)
	mgr, ok := s.cores[kind]
	if !ok {
		logx.Warn(logx.AgentUnknownCore, "master asked for core %q, don't have it", kind)
		return errStatus(req.GetCore(), fmt.Errorf("unknown core %q", kind)), nil
	}
	if err := s.markInactive(kind); err != nil {
		return coreStatus(req.GetCore(), mgr, err), nil
	}
	return coreStatus(req.GetCore(), mgr, mgr.Stop()), nil
}

// Stats — stream live traffic from one core (sing-box today).
func (s *Server) Stats(req *honeyv1.StatsRequest, stream honeyv1.AgentService_StatsServer) error {
	kind := coreKey(req.GetCore())
	logx.Debug(logx.AgentStatsRecv, "stats stream opened for %s", kind)
	mgr, ok := s.cores[kind]
	if !ok {
		logx.Warn(logx.AgentUnknownCore, "master asked stats for core %q, don't have it", kind)
		return fmt.Errorf("unknown core")
	}

	interval := time.Duration(req.GetIntervalMs()) * time.Millisecond
	if interval < 500*time.Millisecond {
		interval = time.Second
	}

	return mgr.StatsLoop(stream.Context(), interval, func(st core.Stat) error {
		if kind == "singbox" {
			if native, ok := s.cores["hysteria"].(interface{ Latest() core.Stat }); ok {
				st = mergeStats(st, native.Latest())
			}
		}
		users := make([]*honeyv1.UserStat, 0, len(st.Users))
		for _, ut := range st.Users {
			users = append(users, &honeyv1.UserStat{Name: ut.Name, UpBytes: ut.Up, DownBytes: ut.Down})
		}
		return stream.Send(&honeyv1.StatSample{
			At:          time.Now().UnixMilli(),
			UpBytes:     st.NodeUp,
			DownBytes:   st.NodeDown,
			UpSpeed:     st.UpSpeed,
			DownSpeed:   st.DownSpeed,
			Connections: st.Connections,
			Epoch:       s.statsEpoch,
			Users:       users,
		})
	})
}

func mergeStats(left, right core.Stat) core.Stat {
	left.NodeUp += right.NodeUp
	left.NodeDown += right.NodeDown
	left.UpSpeed += right.UpSpeed
	left.DownSpeed += right.DownSpeed
	left.Connections += right.Connections
	users := make(map[string]core.UserTraffic, len(left.Users)+len(right.Users))
	for _, user := range left.Users {
		users[user.Name] = user
	}
	for _, user := range right.Users {
		current := users[user.Name]
		current.Name = user.Name
		current.Up += user.Up
		current.Down += user.Down
		users[user.Name] = current
	}
	left.Users = left.Users[:0]
	for _, user := range users {
		left.Users = append(left.Users, user)
	}
	sort.Slice(left.Users, func(i, j int) bool { return left.Users[i].Name < left.Users[j].Name })
	return left
}

// Connections returns a point-in-time snapshot of active connections for the
// requested core. Only cores that implement core.ConnLister (sing-box, via the
// Clash API) report; others return an empty set.
func (s *Server) Connections(ctx context.Context, req *honeyv1.ConnectionsRequest) (*honeyv1.ConnectionsReply, error) {
	kind := coreKey(req.GetCore())
	mgr, ok := s.cores[kind]
	if !ok {
		logx.Warn(logx.AgentUnknownCore, "master asked connections for core %q, don't have it", kind)
		return &honeyv1.ConnectionsReply{}, nil
	}
	lister, ok := mgr.(core.ConnLister)
	if !ok {
		return &honeyv1.ConnectionsReply{}, nil
	}
	live, err := lister.Connections(ctx)
	if err != nil {
		logx.Debug(logx.AgentStatsRecv, "connections snapshot unavailable: %v", err)
		return &honeyv1.ConnectionsReply{}, nil
	}
	conns := make([]*honeyv1.LiveConn, 0, len(live))
	for _, c := range live {
		conns = append(conns, &honeyv1.LiveConn{
			Id:          c.ID,
			User:        c.User,
			SourceIp:    c.SourceIP,
			Destination: c.Destination,
			Network:     c.Network,
			Chain:       c.Chain,
			UpBytes:     c.Up,
			DownBytes:   c.Down,
			StartedAt:   c.StartedAtMS,
		})
	}
	return &honeyv1.ConnectionsReply{Conns: conns}, nil
}

// CloseConnections closes specific active connections by id, to enforce device
// limits. Cores without a Clash API report zero closed.
func (s *Server) CloseConnections(ctx context.Context, req *honeyv1.CloseConnectionsRequest) (*honeyv1.CloseConnectionsReply, error) {
	kind := coreKey(req.GetCore())
	mgr, ok := s.cores[kind]
	if !ok {
		return &honeyv1.CloseConnectionsReply{}, nil
	}
	closer, ok := mgr.(core.ConnCloser)
	if !ok {
		return &honeyv1.CloseConnectionsReply{}, nil
	}
	closed, err := closer.CloseConnections(ctx, req.GetIds())
	if err != nil {
		return &honeyv1.CloseConnectionsReply{Closed: closed}, nil
	}
	return &honeyv1.CloseConnectionsReply{Closed: closed}, nil
}

// Metrics returns a live host snapshot (cpu/mem/disk/bandwidth) from the node.
func (s *Server) Metrics(ctx context.Context, _ *honeyv1.MetricsRequest) (*honeyv1.MetricsReply, error) {
	m, err := sysmetrics.Collect(ctx)
	if err != nil {
		return nil, err
	}
	return &honeyv1.MetricsReply{
		CpuPercent: m.CPUPercent,
		CpuCores:   m.CPUCores,
		MemTotal:   m.MemTotal,
		MemUsed:    m.MemUsed,
		DiskTotal:  m.DiskTotal,
		DiskUsed:   m.DiskUsed,
		NetRxSpeed: m.NetRxSpeed,
		NetTxSpeed: m.NetTxSpeed,
		Load1:      m.Load1,
		UptimeSecs: m.UptimeSecs,
		Supported:  m.Supported,
	}, nil
}

// benchmarkMaxBytes bounds each leg so a benchmark can never exceed the gRPC
// message limit (default 4 MiB) or be used to exhaust node memory.
const benchmarkMaxBytes = 4 << 20

// Benchmark measures coarse master<->node throughput over the control channel:
// the master times its upload (this request) and the download (the reply).
func (s *Server) Benchmark(_ context.Context, req *honeyv1.BenchmarkRequest) (*honeyv1.BenchmarkReply, error) {
	received := uint64(len(req.GetPayload()))
	size := int(req.GetRespondBytes())
	if size < 0 {
		size = 0
	}
	if size > benchmarkMaxBytes {
		size = benchmarkMaxBytes
	}
	// non-uniform filler so an enabled transport compressor cannot flatter the
	// measurement the way an all-zero buffer would.
	payload := make([]byte, size)
	for i := range payload {
		payload[i] = byte(i % 251)
	}
	return &honeyv1.BenchmarkReply{
		Payload:       payload,
		ReceivedBytes: received,
		RecvAtMs:      time.Now().UnixMilli(),
	}, nil
}

// ConfigDrift compares, per core, the config the agent would build from the
// master's spec against what is actually on disk. A mismatch means the running
// config was tampered with or a push half-applied. Only cores that are supposed
// to be running (have inbounds in the spec) are checked.
func (s *Server) ConfigDrift(_ context.Context, req *honeyv1.ConfigDriftRequest) (*honeyv1.ConfigDriftReply, error) {
	spec := specFromProto(req.GetSpec())
	if s.acme != nil {
		if err := s.acme.InjectPaths(&spec); err != nil {
			return nil, err
		}
	}
	byCore := map[string][]core.Inbound{}
	for _, ib := range spec.Inbounds {
		kind := coreOfInbound(ib)
		byCore[kind] = append(byCore[kind], ib)
	}
	kinds := make([]string, 0, len(s.cores))
	for kind := range s.cores {
		kinds = append(kinds, kind)
	}
	sort.Strings(kinds)

	reply := &honeyv1.ConfigDriftReply{}
	for _, kind := range kinds {
		subset := byCore[kind]
		if len(subset) == 0 {
			continue
		}
		coreSpec := spec
		coreSpec.Inbounds = subset
		desired, err := s.cores[kind].BuildConfig(coreSpec)
		if err != nil {
			continue // an unbuildable candidate is a Validate concern, not drift
		}
		desiredHash := canonicalHash(desired)
		runningHash, present := "", false
		if path := s.configPath[kind]; path != "" {
			if data, err := os.ReadFile(path); err == nil {
				runningHash = canonicalHash(string(data))
				present = true
			}
		}
		reply.Cores = append(reply.Cores, &honeyv1.CoreDrift{
			Core:           protoCoreKind(kind),
			DesiredHash:    desiredHash,
			RunningHash:    runningHash,
			Drifted:        present && runningHash != desiredHash,
			RunningPresent: present,
		})
	}
	return reply, nil
}

func (s *Server) prepareACME(ctx context.Context, spec *core.Spec, issue bool) (func(), error) {
	if s.acme == nil {
		return func() {}, nil
	}
	return s.acme.Prepare(ctx, spec, issue)
}

// canonicalHash hashes a JSON config after re-marshalling (Go sorts object
// keys), so formatting-only differences never count as drift. Non-JSON falls
// back to a raw byte hash.
func canonicalHash(cfg string) string {
	var v interface{}
	if err := json.Unmarshal([]byte(cfg), &v); err == nil {
		if b, err := json.Marshal(v); err == nil {
			sum := sha256.Sum256(b)
			return hex.EncodeToString(sum[:])
		}
	}
	sum := sha256.Sum256([]byte(cfg))
	return hex.EncodeToString(sum[:])
}

func protoCoreKind(kind string) honeyv1.CoreKind {
	if kind == "xray" {
		return honeyv1.CoreKind_CORE_KIND_XRAY
	}
	return honeyv1.CoreKind_CORE_KIND_SINGBOX
}

// Logs streams a finite snapshot from the structured agent log ring. The
// stream ends after the snapshot so the master can poll with the last sequence
// as a cursor without keeping another long-lived channel open.
func (s *Server) Logs(req *honeyv1.AgentLogsRequest, stream honeyv1.AgentService_LogsServer) error {
	logx.Debug(logx.AgentLogsRecv, "log snapshot requested after %d", req.GetAfterSeq())
	for _, record := range logx.Snapshot(req.GetAfterSeq(), req.GetLimit()) {
		if err := stream.Send(&honeyv1.AgentLogEntry{
			Seq:      record.Seq,
			AtUnixMs: record.AtUnixMS,
			Level:    record.Level,
			Code:     record.Code,
			Message:  record.Message,
		}); err != nil {
			return err
		}
	}
	return nil
}

// syncPortHopping keeps the isolated nft redirect table in sync with the
// hysteria2 inbounds that request UDP port hopping (extra_json "hop_ports").
// Best-effort: firewall trouble is logged, never fatal.
func (s *Server) syncPortHopping(inbounds []core.Inbound) {
	var rules []firewall.HopRule
	for _, ib := range inbounds {
		if ib.Type != "hysteria2" || len(ib.ExtraJSON) == 0 {
			continue
		}
		var ex struct {
			HopPorts string `json:"hop_ports"`
		}
		if json.Unmarshal(ib.ExtraJSON, &ex) != nil || ex.HopPorts == "" {
			continue
		}
		if start, end, ok := parseHopRange(ex.HopPorts); ok {
			rules = append(rules, firewall.HopRule{Start: start, End: end, To: int(ib.Port)})
		}
	}
	if err := firewall.Apply(rules); err != nil {
		logx.Warn(logx.CoreFirewall, "hysteria2 port-hopping firewall: %v", err)
	}
}

func parseHopRange(s string) (int, int, bool) {
	parts := strings.SplitN(s, "-", 2)
	if len(parts) != 2 {
		return 0, 0, false
	}
	start, err1 := strconv.Atoi(strings.TrimSpace(parts[0]))
	end, err2 := strconv.Atoi(strings.TrimSpace(parts[1]))
	if err1 != nil || err2 != nil || start <= 0 || end < start || end > 65535 {
		return 0, 0, false
	}
	return start, end, true
}

func newStatsEpoch() string {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err == nil {
		return hex.EncodeToString(bytes[:])
	}
	return fmt.Sprintf("fallback-%d", time.Now().UnixNano())
}

// --- helpers ---------------------------------------------------------------

func (s *Server) version(ctx context.Context, kind string) string {
	if mgr, ok := s.cores[kind]; ok {
		if v, err := mgr.Version(ctx); err == nil {
			return v
		}
	}
	return "" // not installed / not reachable
}

// aggregateStatus summarises all cores into one CoreStatus for a node-level Apply.
func (s *Server) aggregateStatus(err error) *honeyv1.CoreStatus {
	state := honeyv1.CoreState_CORE_STATE_STOPPED
	pid := int32(0)

	kinds := make([]string, 0, len(s.cores))
	for k := range s.cores {
		kinds = append(kinds, k)
	}
	sort.Strings(kinds)

	parts := make([]string, 0, len(kinds))
	for _, kind := range kinds {
		st, p, _ := s.cores[kind].Status()
		ps := toProtoState(st)
		if ps == honeyv1.CoreState_CORE_STATE_RUNNING {
			state = honeyv1.CoreState_CORE_STATE_RUNNING
			if pid == 0 {
				pid = int32(p)
			}
		} else if ps == honeyv1.CoreState_CORE_STATE_ERRORED && state != honeyv1.CoreState_CORE_STATE_RUNNING {
			state = honeyv1.CoreState_CORE_STATE_ERRORED
		}
		parts = append(parts, fmt.Sprintf("%s=%s", kind, stateName(ps)))
	}

	msg := strings.Join(parts, " ")
	if err != nil {
		state = honeyv1.CoreState_CORE_STATE_ERRORED
		msg = err.Error()
	}
	return &honeyv1.CoreStatus{
		Core:    honeyv1.CoreKind_CORE_KIND_SINGBOX,
		State:   state,
		Pid:     pid,
		Message: msg,
	}
}

func coreStatus(pc honeyv1.CoreKind, mgr core.Manager, err error) *honeyv1.CoreStatus {
	st, pid, last := mgr.Status()
	msg := last
	if err != nil {
		msg = err.Error()
	}
	return &honeyv1.CoreStatus{Core: pc, State: toProtoState(st), Pid: int32(pid), Message: msg}
}

func errStatus(pc honeyv1.CoreKind, err error) *honeyv1.CoreStatus {
	return &honeyv1.CoreStatus{
		Core:    pc,
		State:   honeyv1.CoreState_CORE_STATE_ERRORED,
		Message: err.Error(),
	}
}

// coreKey maps the proto enum to our internal core key.
func coreKey(k honeyv1.CoreKind) string {
	if k == honeyv1.CoreKind_CORE_KIND_XRAY {
		return "xray"
	}
	return "singbox"
}

// coreOf keeps the existing database/API default ("singbox") compatible while
// routing Hysteria2 through the official Hysteria server process.
func coreOf(k string) string {
	if k == "xray" {
		return "xray"
	}
	if k == "hysteria" {
		return "hysteria"
	}
	return "singbox"
}

func coreOfInbound(ib core.Inbound) string {
	if ib.Type == "hysteria2" && (ib.Core == "" || ib.Core == "singbox") {
		return "hysteria"
	}
	return coreOf(ib.Core)
}

func filterCore(ins []core.Inbound, kind string) []core.Inbound {
	out := make([]core.Inbound, 0, len(ins))
	for _, ib := range ins {
		if coreOfInbound(ib) == kind {
			out = append(out, ib)
		}
	}
	return out
}

func toProtoState(st core.State) honeyv1.CoreState {
	switch st {
	case core.StateRunning:
		return honeyv1.CoreState_CORE_STATE_RUNNING
	case core.StateErrored:
		return honeyv1.CoreState_CORE_STATE_ERRORED
	default:
		return honeyv1.CoreState_CORE_STATE_STOPPED
	}
}

func stateName(s honeyv1.CoreState) string {
	switch s {
	case honeyv1.CoreState_CORE_STATE_RUNNING:
		return "running"
	case honeyv1.CoreState_CORE_STATE_ERRORED:
		return "errored"
	default:
		return "stopped"
	}
}

func specFromProto(p *honeyv1.NodeSpec) core.Spec {
	if p == nil {
		return core.Spec{}
	}
	s := core.Spec{
		LogLevel:    p.GetLogLevel(),
		ClashListen: p.GetClashListen(),
		ClashSecret: p.GetClashSecret(),
	}
	for _, in := range p.GetInbounds() {
		ib := core.Inbound{
			Core:     in.GetCore(),
			Tag:      in.GetTag(),
			Type:     in.GetType(),
			Listen:   in.GetListen(),
			Port:     in.GetListenPort(),
			UpMbps:   in.GetUpMbps(),
			DownMbps: in.GetDownMbps(),

			UpstreamOutboundJSON: in.GetUpstreamOutboundJson(),
		}
		if ej := in.GetExtraJson(); ej != "" {
			ib.ExtraJSON = json.RawMessage(ej)
		}
		for _, u := range in.GetUsers() {
			ib.Users = append(ib.Users, core.User{
				Name:       u.GetName(),
				UUID:       u.GetUuid(),
				Password:   u.GetPassword(),
				Flow:       u.GetFlow(),
				QuotaBytes: u.GetQuotaBytes(),
			})
		}
		if tr := in.GetTransport(); tr != nil && tr.GetNetwork() != "" {
			ib.Transport = &core.Transport{
				Network:     tr.GetNetwork(),
				Path:        tr.GetPath(),
				Host:        tr.GetHost(),
				ServiceName: tr.GetServiceName(),
				Mode:        tr.GetMode(),
			}
		}
		if t := in.GetTls(); t != nil {
			tls := &core.TLS{
				Enabled:                  t.GetEnabled(),
				ServerName:               t.GetServerName(),
				CertPath:                 t.GetCertPath(),
				KeyPath:                  t.GetKeyPath(),
				ECH:                      t.GetEch(),
				UTLSFingerprint:          t.GetUtlsFingerprint(),
				ShadowTLSHandshakeServer: t.GetShadowtlsHandshakeServer(),
				ShadowTLSHandshakePort:   t.GetShadowtlsHandshakePort(),
			}
			if r := t.GetReality(); r != nil {
				tls.Reality = &core.Reality{
					PrivateKey:      r.GetPrivateKey(),
					ShortIDs:        r.GetShortIds(),
					HandshakeServer: r.GetHandshakeServer(),
					HandshakePort:   r.GetHandshakePort(),
				}
			}
			ib.TLS = tls
		}
		s.Inbounds = append(s.Inbounds, ib)
	}
	for _, w := range p.GetWireguard() {
		iface := core.WgInterface{
			Name:              w.GetName(),
			ListenPort:        w.GetListenPort(),
			PrivateKey:        w.GetPrivateKey(),
			Address:           w.GetAddress(),
			MTU:               w.GetMtu(),
			Amnezia:           w.GetAmnezia(),
			AmneziaParamsJSON: w.GetAmneziaParamsJson(),
		}
		for _, pr := range w.GetPeers() {
			iface.Peers = append(iface.Peers, core.WgPeer{
				PublicKey: pr.GetPublicKey(),
				AllowedIP: pr.GetAllowedIp(),
			})
		}
		s.Wireguard = append(s.Wireguard, iface)
	}
	for _, sv := range p.GetServices() {
		s.Services = append(s.Services, core.NodeService{
			Kind:       sv.GetKind(),
			Name:       sv.GetName(),
			ListenPort: sv.GetListenPort(),
			Secret:     sv.GetSecret(),
			ConfigJSON: sv.GetConfigJson(),
		})
	}
	return s
}
