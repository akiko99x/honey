package singbox

import (
	"testing"
	"time"
)

func TestTrafficAccumulatorSurvivesStatsReconnect(t *testing.T) {
	manager := NewManager("sing-box", "config.json", nil)
	started := time.Unix(100, 0)

	first := manager.accumulate(Snapshot{
		UpBytes: 100, DownBytes: 50, Connections: 1,
		Conns: []Conn{{ID: "conn-1", User: "alice", Up: 100, Down: 50}},
	}, started)
	if first.Users[0].Up != 100 || first.Users[0].Down != 50 {
		t.Fatalf("unexpected first total: %+v", first.Users[0])
	}

	// A new Stats RPC sees the same live connection. Persistent state must not
	// count its already-seen bytes for a second time.
	same := manager.accumulate(Snapshot{
		UpBytes: 100, DownBytes: 50, Connections: 1,
		Conns: []Conn{{ID: "conn-1", User: "alice", Up: 100, Down: 50}},
	}, started.Add(time.Second))
	if same.Users[0].Up != 100 || same.Users[0].Down != 50 {
		t.Fatalf("reconnect double-counted traffic: %+v", same.Users[0])
	}

	next := manager.accumulate(Snapshot{
		UpBytes: 125, DownBytes: 60, Connections: 1,
		Conns: []Conn{{ID: "conn-1", User: "alice", Up: 125, Down: 60}},
	}, started.Add(2*time.Second))
	if next.Users[0].Up != 125 || next.Users[0].Down != 60 {
		t.Fatalf("new delta was not counted exactly once: %+v", next.Users[0])
	}
}
