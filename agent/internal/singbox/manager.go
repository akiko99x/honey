// Package singbox drives a sing-box process and reads its stats via the Clash API.
package singbox

import (
	"context"
	"sort"
	"sync"
	"time"

	"github.com/akiko99x/honey/agent/internal/core"
	"github.com/akiko99x/honey/agent/internal/logx"
)

// Manager implements core.Manager for sing-box.
type Manager struct {
	proc    *core.Process
	clash   *Clash
	binPath string

	statsMu sync.Mutex
	stats   trafficState
}

type trafficState struct {
	prev     *Snapshot
	prevAt   time.Time
	connUp   map[string]uint64
	connDown map[string]uint64
	userUp   map[string]uint64
	userDown map[string]uint64
	latest   core.Stat
}

func NewManager(binPath, configPath string, clash *Clash) *Manager {
	proc := core.NewProcess(
		"sing-box", binPath, configPath,
		func(cfg string) []string { return []string{"run", "-c", cfg} },
		func(cfg string) []string { return []string{"check", "-c", cfg} },
	)
	return &Manager{
		proc: proc, clash: clash, binPath: binPath,
		stats: trafficState{
			connUp: map[string]uint64{}, connDown: map[string]uint64{},
			userUp: map[string]uint64{}, userDown: map[string]uint64{},
		},
	}
}

func (m *Manager) BuildConfig(spec core.Spec) (string, error) {
	config, err := BuildConfig(spec)
	if err != nil {
		return "", err
	}
	return string(config), nil
}

func (m *Manager) Start(configJSON string) error               { return m.proc.Start(configJSON) }
func (m *Manager) Validate(configJSON string) error            { return m.proc.Validate(configJSON) }
func (m *Manager) Stop() error                                 { return m.proc.Stop() }
func (m *Manager) Apply(configJSON string) error               { return m.proc.Apply(configJSON) }
func (m *Manager) Status() (core.State, int, string)           { return m.proc.Status() }
func (m *Manager) Version(ctx context.Context) (string, error) { return Version(ctx, m.binPath) }

// Connections returns a point-in-time snapshot of active connections from the
// Clash API, mapped to the core-level shape for the master's live view.
func (m *Manager) Connections(ctx context.Context) ([]core.LiveConn, error) {
	snapshot, err := m.clash.Read(ctx)
	if err != nil {
		return nil, err
	}
	conns := make([]core.LiveConn, 0, len(snapshot.Conns))
	for _, c := range snapshot.Conns {
		conns = append(conns, core.LiveConn{
			ID:          c.ID,
			User:        c.User,
			SourceIP:    c.SourceIP,
			Destination: c.Destination,
			Network:     c.Network,
			Chain:       c.Chain,
			Up:          c.Up,
			Down:        c.Down,
			StartedAtMS: c.StartedAtMS,
		})
	}
	return conns, nil
}

// CloseConnections closes the given connection ids via the Clash API. Best
// effort: individual failures are counted out, not fatal.
func (m *Manager) CloseConnections(ctx context.Context, ids []string) (uint32, error) {
	var closed uint32
	for _, id := range ids {
		if id == "" {
			continue
		}
		if err := m.clash.Close(ctx, id); err != nil {
			logx.Debug(logx.CoreStatsPaused, "clash close %s: %v", id, err)
			continue
		}
		closed++
	}
	return closed, nil
}

// StatsLoop only owns delivery to a connected master. The accumulator is driven
// separately by the always-on Poll loop, so this just forwards the latest cached
// sample at the master's requested cadence — reconnecting the stream never
// resets counters or double-counts.
func (m *Manager) StatsLoop(ctx context.Context, interval time.Duration, fn func(core.Stat) error) error {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			if err := fn(m.Latest()); err != nil {
				return err
			}
		}
	}
}

func (m *Manager) accumulate(snapshot Snapshot, now time.Time) core.Stat {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	state := &m.stats

	var upSpeed, downSpeed uint64
	if state.prev != nil {
		seconds := now.Sub(state.prevAt).Seconds()
		upSpeed = perSec(snapshot.UpBytes, state.prev.UpBytes, seconds)
		downSpeed = perSec(snapshot.DownBytes, state.prev.DownBytes, seconds)
	}
	current := snapshot
	state.prev = &current
	state.prevAt = now

	seen := make(map[string]struct{}, len(snapshot.Conns))
	for _, connection := range snapshot.Conns {
		seen[connection.ID] = struct{}{}
		up := increment(connection.Up, state.connUp[connection.ID])
		down := increment(connection.Down, state.connDown[connection.ID])
		state.connUp[connection.ID] = connection.Up
		state.connDown[connection.ID] = connection.Down
		if connection.User != "" {
			state.userUp[connection.User] += up
			state.userDown[connection.User] += down
		}
	}
	for id := range state.connUp {
		if _, ok := seen[id]; !ok {
			delete(state.connUp, id)
			delete(state.connDown, id)
		}
	}

	names := make([]string, 0, len(state.userUp))
	for name := range state.userUp {
		names = append(names, name)
	}
	sort.Strings(names)
	users := make([]core.UserTraffic, 0, len(names))
	for _, name := range names {
		users = append(users, core.UserTraffic{Name: name, Up: state.userUp[name], Down: state.userDown[name]})
	}
	stat := core.Stat{
		NodeUp: snapshot.UpBytes, NodeDown: snapshot.DownBytes,
		UpSpeed: upSpeed, DownSpeed: downSpeed,
		Connections: snapshot.Connections, Users: users,
	}
	state.latest = stat
	return stat
}

// Poll reads the Clash API once and advances the accumulator. Runs always-on
// (independent of any master Stats stream) so per-user counters keep growing
// even while the master is offline. Returns false when the API is unreachable.
func (m *Manager) Poll(ctx context.Context) bool {
	snapshot, err := m.clash.Read(ctx)
	if err != nil {
		return false
	}
	m.accumulate(snapshot, time.Now())
	return true
}

// Latest returns the most recent accumulated sample without advancing state.
func (m *Manager) Latest() core.Stat {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	return m.stats.latest
}

// UserTotals returns each user's cumulative up+down bytes since accounting began.
func (m *Manager) UserTotals() map[string]uint64 {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	out := make(map[string]uint64, len(m.stats.userUp))
	for name, up := range m.stats.userUp {
		out[name] = up + m.stats.userDown[name]
	}
	return out
}

// UserCounters returns the per-user cumulative up and down maps (for persistence).
func (m *Manager) UserCounters() (map[string]uint64, map[string]uint64) {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	up := make(map[string]uint64, len(m.stats.userUp))
	down := make(map[string]uint64, len(m.stats.userDown))
	for name, v := range m.stats.userUp {
		up[name] = v
	}
	for name, v := range m.stats.userDown {
		down[name] = v
	}
	return up, down
}

// SeedUsers restores persisted per-user cumulative counters at startup so the
// accounting survives an agent restart.
func (m *Manager) SeedUsers(up, down map[string]uint64) {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	for name, v := range up {
		m.stats.userUp[name] = v
	}
	for name, v := range down {
		m.stats.userDown[name] = v
	}
}

func perSec(cur, old uint64, seconds float64) uint64 {
	if cur < old || seconds <= 0 {
		return 0
	}
	return uint64(float64(cur-old) / seconds)
}

func increment(cur, prev uint64) uint64 {
	if cur >= prev {
		return cur - prev
	}
	return cur
}
