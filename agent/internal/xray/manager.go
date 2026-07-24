// Package xray drives an xray-core process (second core, runs alongside sing-box).
package xray

import (
	"context"
	"time"

	"github.com/akiko99x/honey/agent/internal/core"
)

// Manager implements core.Manager for xray.
type Manager struct {
	proc    *core.Process
	binPath string
	apiAddr string // xray gRPC stats api addr (host:port)
}

func NewManager(binPath, configPath, apiAddr string) *Manager {
	proc := core.NewProcess(
		"xray", binPath, configPath,
		func(cfg string) []string { return []string{"run", "-c", cfg} },
		func(cfg string) []string { return []string{"run", "-test", "-c", cfg} },
	)
	return &Manager{proc: proc, binPath: binPath, apiAddr: apiAddr}
}

func (m *Manager) BuildConfig(spec core.Spec) (string, error) {
	b, err := BuildConfig(spec, m.apiAddr)
	if err != nil {
		return "", err
	}
	return string(b), nil
}

func (m *Manager) Start(configJSON string) error     { return m.proc.Start(configJSON) }
func (m *Manager) Validate(configJSON string) error  { return m.proc.Validate(configJSON) }
func (m *Manager) Stop() error                       { return m.proc.Stop() }
func (m *Manager) Apply(configJSON string) error     { return m.proc.Apply(configJSON) }
func (m *Manager) Status() (core.State, int, string) { return m.proc.Status() }

func (m *Manager) Version(ctx context.Context) (string, error) {
	return Version(ctx, m.binPath)
}

// StatsLoop queries xray's gRPC StatsService (see stats.go).
func (m *Manager) StatsLoop(ctx context.Context, interval time.Duration, fn func(core.Stat) error) error {
	return m.statsLoop(ctx, interval, fn)
}
