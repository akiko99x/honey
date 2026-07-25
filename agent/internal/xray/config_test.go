package xray

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/akiko99x/honey/agent/internal/core"
)

func realitySpec() core.Spec {
	return core.Spec{Inbounds: []core.Inbound{{
		Tag: "vless-in", Type: "vless", Port: 443,
		Users: []core.User{{
			Name: "alice", UUID: "11111111-1111-1111-1111-111111111111",
			Flow: "xtls-rprx-vision",
		}},
		TLS: &core.TLS{
			Enabled: true, ServerName: "www.cloudflare.com",
			Reality: &core.Reality{
				PrivateKey:      "UuMBgl7MXTPx9inmQp2UC7Jcnwc6XYbwDNebonM-FCc",
				ShortIDs:        []string{"0123456789abcdef"},
				HandshakeServer: "www.cloudflare.com", HandshakePort: 443,
			},
		},
		ExtraJSON: json.RawMessage(`{"happ":{"name":"Poland"}}`),
	}}}
}

func decodeConfig(t *testing.T, data []byte) map[string]any {
	t.Helper()
	var cfg map[string]any
	if err := json.Unmarshal(data, &cfg); err != nil {
		t.Fatal(err)
	}
	return cfg
}

func TestBuildRealityVLESSUsesCurrentXraySchema(t *testing.T) {
	data, err := BuildConfig(realitySpec(), "127.0.0.1:8081")
	if err != nil {
		t.Fatal(err)
	}
	cfg := decodeConfig(t, data)

	api := cfg["api"].(map[string]any)
	if api["listen"] != "127.0.0.1:8081" {
		t.Fatalf("unexpected API listener: %#v", api)
	}
	if _, exists := cfg["routing"]; exists {
		t.Fatal("direct API mode must not add a synthetic API route")
	}

	inbounds := cfg["inbounds"].([]any)
	if len(inbounds) != 1 {
		t.Fatalf("expected one VPN inbound, got %d", len(inbounds))
	}
	inbound := inbounds[0].(map[string]any)
	if _, exists := inbound["happ"]; exists {
		t.Fatalf("subscription-only metadata leaked into Xray config: %#v", inbound)
	}
	if inbound["protocol"] != "vless" {
		t.Fatalf("unexpected protocol: %v", inbound["protocol"])
	}
	settings := inbound["settings"].(map[string]any)
	if settings["decryption"] != "none" {
		t.Fatalf("VLESS decryption must be none: %#v", settings)
	}
	reality := inbound["streamSettings"].(map[string]any)["realitySettings"].(map[string]any)
	if reality["target"] != "www.cloudflare.com:443" {
		t.Fatalf("unexpected REALITY target: %#v", reality)
	}
	if _, legacy := reality["dest"]; legacy {
		t.Fatal("generator should emit documented target instead of legacy dest")
	}
}

func TestBuildHysteria2UsesXrayHysteriaProtocolAndTransport(t *testing.T) {
	spec := core.Spec{Inbounds: []core.Inbound{{
		Tag: "hy2-in", Type: "hysteria2", Port: 18443,
		Users: []core.User{{Name: "alice", Password: "secret"}},
		TLS: &core.TLS{
			Enabled: true, ServerName: "vpn.example.com",
			CertPath: "/etc/honey/fullchain.pem", KeyPath: "/etc/honey/privkey.pem",
		},
	}}}
	data, err := BuildConfig(spec, "")
	if err != nil {
		t.Fatal(err)
	}
	inbound := decodeConfig(t, data)["inbounds"].([]any)[0].(map[string]any)
	if inbound["protocol"] != "hysteria" {
		t.Fatalf("Xray protocol must be hysteria: %#v", inbound)
	}
	settings := inbound["settings"].(map[string]any)
	if settings["version"] != float64(2) {
		t.Fatalf("Xray Hysteria version must be 2: %#v", settings)
	}
	user := settings["users"].([]any)[0].(map[string]any)
	if user["auth"] != "secret" {
		t.Fatalf("Xray Hysteria user must use auth: %#v", user)
	}
	stream := inbound["streamSettings"].(map[string]any)
	if stream["method"] != "hysteria" || stream["security"] != "tls" {
		t.Fatalf("unexpected Xray Hysteria stream: %#v", stream)
	}
}

func TestXrayRejectsHysteria2WithoutTLS(t *testing.T) {
	_, err := BuildConfig(core.Spec{Inbounds: []core.Inbound{{
		Tag: "hy2-in", Type: "hysteria2", Port: 18443,
	}}}, "")
	if err == nil {
		t.Fatal("expected TLS validation error")
	}
}

func TestGeneratedRealityConfigWithInstalledXray(t *testing.T) {
	bin := os.Getenv("HONEY_XRAY_BIN")
	if bin == "" {
		t.Skip("HONEY_XRAY_BIN is not set")
	}
	data, err := BuildConfig(realitySpec(), "127.0.0.1:18081")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "xray.json")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	if output, err := exec.Command(bin, "run", "-test", "-c", path).CombinedOutput(); err != nil {
		t.Fatalf("xray rejected generated config: %v\n%s", err, output)
	}
}
