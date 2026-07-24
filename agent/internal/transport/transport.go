// Package transport decides *how* the agent's gRPC server gets its connections.
//
// mTLS lives on the grpc.Server credentials, so a transport is pure connection
// plumbing. two impls:
//   - Serve: the agent listens, the master dials in (classic).
//   - Dial:  the agent dials out to the master and the master drives it back
//     over that socket (for nodes behind NAT / firewalls).
//
// both feed raw net.Conns to the SAME grpc.Server, and can run at once.
package transport

import (
	"context"
	"net"
	"sync"

	"google.golang.org/grpc"
)

// Transport runs until ctx is cancelled or the grpc server stops.
type Transport interface {
	Name() string
	Run(ctx context.Context, srv *grpc.Server) error
}

// singleConnListener adapts a single, already-open net.Conn into a net.Listener
// so grpc.Server.Serve can drive it. used by the dial transport: the agent
// opens the socket, but the grpc server still speaks the server side of TLS/h2.
type singleConnListener struct {
	ch   chan net.Conn
	done chan struct{}
	addr net.Addr
	once sync.Once
}

func newSingleConnListener(c net.Conn) *singleConnListener {
	l := &singleConnListener{
		ch:   make(chan net.Conn, 1),
		done: make(chan struct{}),
		addr: c.LocalAddr(),
	}
	// hand the conn out on the first Accept; closing it closes the listener,
	// which makes grpc.Server.Serve return so we can reconnect.
	l.ch <- &notifyConn{Conn: c, l: l}
	return l
}

func (l *singleConnListener) Accept() (net.Conn, error) {
	select {
	case c := <-l.ch:
		return c, nil
	case <-l.done:
		return nil, net.ErrClosed
	}
}

func (l *singleConnListener) Close() error {
	l.once.Do(func() { close(l.done) })
	return nil
}

func (l *singleConnListener) Addr() net.Addr { return l.addr }

// notifyConn closes its parent listener when the conn is closed, so a dropped
// tunnel makes Serve return.
type notifyConn struct {
	net.Conn
	l    *singleConnListener
	once sync.Once
}

func (c *notifyConn) Close() error {
	c.once.Do(func() { c.l.Close() })
	return c.Conn.Close()
}
