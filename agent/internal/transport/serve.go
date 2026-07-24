package transport

import (
	"context"
	"errors"
	"fmt"
	"net"

	"google.golang.org/grpc"

	"github.com/akiko99x/honey/agent/internal/logx"
)

// Serve makes the agent listen; the master dials in.
type Serve struct {
	addr string
}

func NewServe(addr string) *Serve { return &Serve{addr: addr} }

func (s *Serve) Name() string { return fmt.Sprintf("serve(%s)", s.addr) }

func (s *Serve) Run(ctx context.Context, srv *grpc.Server) error {
	lis, err := net.Listen("tcp", s.addr)
	if err != nil {
		logx.Error(logx.AgentListenFailed, "can't listen on %s: %v", s.addr, err)
		return fmt.Errorf("listen %s: %w", s.addr, err)
	}

	go func() {
		<-ctx.Done()
		_ = lis.Close()
	}()

	err = srv.Serve(lis)
	if ctx.Err() != nil || errors.Is(err, grpc.ErrServerStopped) {
		return nil // clean shutdown
	}
	return err
}
