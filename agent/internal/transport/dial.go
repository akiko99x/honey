package transport

import (
	"context"
	"errors"
	"fmt"
	"net"
	"time"

	"google.golang.org/grpc"

	"github.com/akiko99x/honey/agent/internal/logx"
)

// Dial makes the agent connect out to the master and serve the AgentService
// back over that socket. good for nodes behind NAT / firewalls that can't be
// dialed into. reconnects with capped exponential backoff.
type Dial struct {
	master     string
	dialTO     time.Duration
	minBackoff time.Duration
	maxBackoff time.Duration
}

func NewDial(master string) *Dial {
	return &Dial{
		master:     master,
		dialTO:     10 * time.Second,
		minBackoff: time.Second,
		maxBackoff: 30 * time.Second,
	}
}

func (d *Dial) Name() string { return fmt.Sprintf("dial(%s)", d.master) }

func (d *Dial) Run(ctx context.Context, srv *grpc.Server) error {
	backoff := d.minBackoff
	dialer := &net.Dialer{Timeout: d.dialTO}

	for {
		if ctx.Err() != nil {
			return nil
		}

		conn, err := dialer.DialContext(ctx, "tcp", d.master)
		if err != nil {
			logx.Warn(logx.AgentDialRetry, "dial to master %s failed, retry in %s: %v", d.master, backoff, err)
			if !sleep(ctx, backoff) {
				return nil
			}
			backoff = nextBackoff(backoff, d.maxBackoff)
			continue
		}
		backoff = d.minBackoff // connected — reset
		logx.Info(logx.AgentDialUp, "dialed master %s, tunnel is up", d.master)

		// serve the tunnel; blocks until the master drops it or we stop.
		lis := newSingleConnListener(conn)
		serveErr := srv.Serve(lis)
		if errors.Is(serveErr, grpc.ErrServerStopped) {
			return nil
		}

		if !sleep(ctx, d.minBackoff) {
			return nil
		}
	}
}

// sleep waits d or until ctx is done; returns false if ctx was cancelled.
func sleep(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}

func nextBackoff(cur, max time.Duration) time.Duration {
	cur *= 2
	if cur > max {
		return max
	}
	return cur
}
