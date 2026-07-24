package grpcserver

import (
	"context"
	"errors"
	"testing"
	"time"

	honeyv1 "github.com/akiko99x/honey/agent/gen/honey/v1"
	"github.com/akiko99x/honey/agent/internal/core"
)

type dryRunManager struct {
	builds, validates, starts, stops, applies int
	validateErr                               error
}

func (m *dryRunManager) BuildConfig(core.Spec) (string, error)   { m.builds++; return `{}`, nil }
func (m *dryRunManager) Validate(string) error                   { m.validates++; return m.validateErr }
func (m *dryRunManager) Start(string) error                      { m.starts++; return nil }
func (m *dryRunManager) Stop() error                             { m.stops++; return nil }
func (m *dryRunManager) Apply(string) error                      { m.applies++; return nil }
func (m *dryRunManager) Status() (core.State, int, string)       { return core.StateRunning, 42, "" }
func (m *dryRunManager) Version(context.Context) (string, error) { return "test", nil }
func (m *dryRunManager) StatsLoop(context.Context, time.Duration, func(core.Stat) error) error {
	return nil
}

func dryRunRequest() *honeyv1.ApplyRequest {
	return &honeyv1.ApplyRequest{Spec: &honeyv1.NodeSpec{Inbounds: []*honeyv1.Inbound{{
		Tag: "candidate", Type: "vless", Core: "xray", ListenPort: 443,
	}}}}
}

func TestValidateDoesNotMutateCoreLifecycle(t *testing.T) {
	mgr := &dryRunManager{}
	server := New("node", map[string]core.Manager{"xray": mgr})
	status, err := server.Validate(context.Background(), dryRunRequest())
	if err != nil {
		t.Fatal(err)
	}
	if status.GetState() == honeyv1.CoreState_CORE_STATE_ERRORED {
		t.Fatalf("candidate unexpectedly rejected: %s", status.GetMessage())
	}
	if mgr.builds != 1 || mgr.validates != 1 || mgr.starts != 0 || mgr.stops != 0 || mgr.applies != 0 {
		t.Fatalf("dry-run counters: %+v", mgr)
	}
}

func TestValidateHidesThirdPartyErrorFromResponse(t *testing.T) {
	mgr := &dryRunManager{validateErr: errors.New("secret=/tmp/private-config.json")}
	server := New("node", map[string]core.Manager{"xray": mgr})
	status, err := server.Validate(context.Background(), dryRunRequest())
	if err != nil {
		t.Fatal(err)
	}
	if status.GetState() != honeyv1.CoreState_CORE_STATE_ERRORED {
		t.Fatalf("expected rejected candidate, got %s", status.GetState())
	}
	if got := status.GetMessage(); got != "candidate configuration rejected; inspect agent logs" {
		t.Fatalf("unsafe validation response: %q", got)
	}
}
