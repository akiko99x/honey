package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/akiko99x/honey/agent/internal/core"
)

type recoveryManager struct {
	starts   []string
	startErr error
}

func (m *recoveryManager) BuildConfig(core.Spec) (string, error) { return "", nil }
func (m *recoveryManager) Validate(string) error                 { return nil }
func (m *recoveryManager) Start(config string) error {
	m.starts = append(m.starts, config)
	return m.startErr
}
func (m *recoveryManager) Stop() error                             { return nil }
func (m *recoveryManager) Apply(string) error                      { return nil }
func (m *recoveryManager) Status() (core.State, int, string)       { return core.StateStopped, 0, "" }
func (m *recoveryManager) Version(context.Context) (string, error) { return "test", nil }
func (m *recoveryManager) StatsLoop(context.Context, time.Duration, func(core.Stat) error) error {
	return nil
}

func TestRecoverCoreStartsOnlyExplicitUnchangedConfig(t *testing.T) {
	dir := t.TempDir()
	config := filepath.Join(dir, "core.json")
	if err := os.WriteFile(config, []byte(`{"inbounds":[]}`), 0o600); err != nil {
		t.Fatal(err)
	}

	manager := &recoveryManager{}
	recoverCore("test", manager, config)
	if len(manager.starts) != 0 {
		t.Fatal("config without an active marker was resumed")
	}
	if err := core.MarkActive(config); err != nil {
		t.Fatal(err)
	}
	recoverCore("test", manager, config)
	if len(manager.starts) != 1 || manager.starts[0] != "" {
		t.Fatalf("valid recovery starts = %#v, want one existing-config start", manager.starts)
	}

	if err := os.WriteFile(config, []byte(`{"inbounds":[{"tag":"changed"}]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	recoverCore("test", manager, config)
	if len(manager.starts) != 1 {
		t.Fatal("hash-mismatched config was resumed")
	}
}

func TestRecoverCoreIsIndependentOfMasterAndNonFatalOnStartError(t *testing.T) {
	dir := t.TempDir()
	config := filepath.Join(dir, "core.json")
	if err := os.WriteFile(config, []byte(`{"inbounds":[]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := core.MarkActive(config); err != nil {
		t.Fatal(err)
	}
	manager := &recoveryManager{startErr: errors.New("core unavailable")}
	// recoverCore has no registry/network dependency. A failed start is logged
	// and returned to the normal agent startup path rather than terminating it.
	recoverCore("test", manager, config)
	if len(manager.starts) != 1 {
		t.Fatalf("start attempts = %d, want 1", len(manager.starts))
	}
}
