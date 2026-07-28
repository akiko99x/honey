package hysteria

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/akiko99x/honey/agent/internal/core"
)

func TestBuildConfigUsesOfficialUserpassAndTimeout(t *testing.T) {
	manager := NewManager("hysteria", "config.json")
	config, err := manager.BuildConfig(core.Spec{Inbounds: []core.Inbound{{
		Tag:    "hy2",
		Type:   "hysteria2",
		Listen: "::",
		Port:   443,
		Users:  []core.User{{Name: "alice", Password: "secret"}},
		TLS: &core.TLS{
			Enabled:  true,
			CertPath: "/cert.pem",
			KeyPath:  "/key.pem",
		},
		ExtraJSON: json.RawMessage(`{"udpIdleTimeout":"5m","hop_ports":"20000-30000","happ":{"name":"test"}}`),
	}}})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		`"listen": ":443"`,
		`"udpIdleTimeout": "5m"`,
		`"alice": "secret"`,
		`"type": "userpass"`,
	} {
		if !strings.Contains(config, want) {
			t.Fatalf("config does not contain %s:\n%s", want, config)
		}
	}
	if strings.Contains(config, "hop_ports") || strings.Contains(config, `"happ"`) {
		t.Fatalf("control-plane metadata leaked into Hysteria config:\n%s", config)
	}
}
