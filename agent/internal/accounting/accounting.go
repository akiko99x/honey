// Package accounting persists per-user traffic counters and a stable stats
// epoch to disk, so the agent's accounting survives a restart and keeps growing
// while the master is offline. The counters are cumulative up/down bytes; the
// epoch is reused across restarts so the master treats a reconnect as a
// continuation, not a fresh baseline.
package accounting

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

type Counter struct {
	Up   uint64 `json:"up"`
	Down uint64 `json:"down"`
}

type Store struct {
	mu    sync.Mutex
	path  string
	Epoch string             `json:"epoch"`
	Users map[string]Counter `json:"users"`
}

func newEpoch() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "epoch"
	}
	return hex.EncodeToString(b[:])
}

// Load reads the store from disk, or returns a fresh one with a new epoch.
func Load(path string) *Store {
	s := &Store{path: path, Users: map[string]Counter{}}
	if data, err := os.ReadFile(path); err == nil {
		_ = json.Unmarshal(data, s)
	}
	if s.Epoch == "" {
		s.Epoch = newEpoch()
	}
	if s.Users == nil {
		s.Users = map[string]Counter{}
	}
	return s
}

// SetCounters replaces the stored per-user counters from the live accumulator.
func (s *Store) SetCounters(up, down map[string]uint64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	users := make(map[string]Counter, len(up))
	for name, u := range up {
		c := users[name]
		c.Up = u
		users[name] = c
	}
	for name, d := range down {
		c := users[name]
		c.Down = d
		users[name] = c
	}
	s.Users = users
}

// Counters returns the persisted per-user up/down maps (for seeding at startup).
func (s *Store) Counters() (map[string]uint64, map[string]uint64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	up := make(map[string]uint64, len(s.Users))
	down := make(map[string]uint64, len(s.Users))
	for name, c := range s.Users {
		up[name] = c.Up
		down[name] = c.Down
	}
	return up, down
}

// Save atomically writes the store to disk.
func (s *Store) Save() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.path == "" {
		return nil
	}
	data, err := json.Marshal(s)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return err
	}
	tmp := s.path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, s.path)
}
