package core

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestRecoveryRequiresExplicitMarkerAndMatchingConfig(t *testing.T) {
	dir := t.TempDir()
	config := filepath.Join(dir, "sing-box.json")
	if err := os.WriteFile(config, []byte(`{"inbounds":[]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, err := ShouldRecover(config); err != nil || got {
		t.Fatalf("missing marker should not recover: got=%v err=%v", got, err)
	}
	if err := MarkActive(config); err != nil {
		t.Fatal(err)
	}
	if got, err := ShouldRecover(config); err != nil || !got {
		t.Fatalf("active matching marker should recover: got=%v err=%v", got, err)
	}
	if err := os.WriteFile(config, []byte(`{"inbounds":[{"tag":"removed"}]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, err := ShouldRecover(config); err == nil || got {
		t.Fatalf("modified config must not recover: got=%v err=%v", got, err)
	}
	if err := MarkInactive(config); err != nil {
		t.Fatal(err)
	}
	if got, err := ShouldRecover(config); err != nil || got {
		t.Fatalf("inactive marker must not recover: got=%v err=%v", got, err)
	}
}

func TestRecoveryRejectsCorruptUnsupportedAndMissingState(t *testing.T) {
	tests := []struct {
		name       string
		state      string
		withConfig bool
		wantErr    bool
	}{
		{name: "corrupt marker", state: `{`, withConfig: true, wantErr: true},
		{name: "unsupported marker", state: `{"version":2,"active":true}`, withConfig: true},
		{name: "active missing config", state: `{"version":1,"active":true,"sha256":"deadbeef"}`, wantErr: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			dir := t.TempDir()
			config := filepath.Join(dir, "core.json")
			if tt.withConfig {
				if err := os.WriteFile(config, []byte(`{"ok":true}`), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			if err := os.WriteFile(recoveryStatePath(config), []byte(tt.state), 0o600); err != nil {
				t.Fatal(err)
			}
			got, err := ShouldRecover(config)
			if got || (err != nil) != tt.wantErr {
				t.Fatalf("ShouldRecover() = %v, %v; want false, wantErr=%v", got, err, tt.wantErr)
			}
		})
	}
}

func TestMarkActiveReplacesPreviousInactiveMarker(t *testing.T) {
	dir := t.TempDir()
	config := filepath.Join(dir, "xray.json")
	if err := os.WriteFile(config, []byte(`{"inbounds":[]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := MarkInactive(config); err != nil {
		t.Fatal(err)
	}
	if err := MarkActive(config); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(recoveryStatePath(config))
	if err != nil {
		t.Fatal(err)
	}
	var state recoveryState
	if err := json.Unmarshal(data, &state); err != nil {
		t.Fatal(err)
	}
	if !state.Active || state.Version != 1 || state.SHA256 != configHash([]byte(`{"inbounds":[]}`)) {
		t.Fatalf("unexpected marker: %#v", state)
	}
}
