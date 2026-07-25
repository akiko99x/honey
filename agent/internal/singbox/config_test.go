package singbox

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/akiko99x/honey/agent/internal/core"
)

// one sing-box, two protocols at once: vless+reality on 443, hysteria2 on 8443.
func TestBuildConfig_MultiProtocol(t *testing.T) {
	spec := core.Spec{
		ClashListen: "127.0.0.1:9090",
		Inbounds: []core.Inbound{
			{
				Tag: "vless-in", Type: "vless", Port: 443,
				Users: []core.User{{Name: "u1", UUID: "11111111-1111-1111-1111-111111111111", Flow: "xtls-rprx-vision"}},
				TLS: &core.TLS{
					Enabled: true, ServerName: "example.com",
					Reality: &core.Reality{
						PrivateKey:      "PRIV",
						ShortIDs:        []string{"0123abcd"},
						HandshakeServer: "example.com",
					},
				},
			},
			{
				Tag: "hy2-in", Type: "hysteria2", Port: 8443,
				Users: []core.User{{Name: "u1", Password: "s3cret"}},
				TLS:   &core.TLS{Enabled: true, ServerName: "example.com", CertPath: "/etc/honey/tls/fullchain.pem", KeyPath: "/etc/honey/tls/key.pem"},
			},
		},
	}

	raw, err := BuildConfig(spec)
	if err != nil {
		t.Fatalf("BuildConfig: %v", err)
	}

	var cfg struct {
		Experimental struct {
			ClashAPI struct {
				ExternalController string `json:"external_controller"`
			} `json:"clash_api"`
		} `json:"experimental"`
		Inbounds []struct {
			Type string `json:"type"`
			Tag  string `json:"tag"`
			Port int    `json:"listen_port"`
			TLS  struct {
				Reality map[string]any `json:"reality"`
			} `json:"tls"`
		} `json:"inbounds"`
	}
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatalf("generated config is not valid json: %v\n%s", err, raw)
	}

	if cfg.Experimental.ClashAPI.ExternalController != "127.0.0.1:9090" {
		t.Errorf("clash_api not wired: %q", cfg.Experimental.ClashAPI.ExternalController)
	}
	if len(cfg.Inbounds) != 2 {
		t.Fatalf("want 2 inbounds, got %d", len(cfg.Inbounds))
	}
	if cfg.Inbounds[0].Type != "vless" || cfg.Inbounds[0].TLS.Reality == nil {
		t.Errorf("vless/reality inbound malformed: %+v", cfg.Inbounds[0])
	}
	if cfg.Inbounds[1].Type != "hysteria2" || cfg.Inbounds[1].Port != 8443 {
		t.Errorf("hysteria2 inbound malformed: %+v", cfg.Inbounds[1])
	}
}

// multihop: an entry inbound with an upstream outbound gets that outbound added
// and a route rule sending its traffic through it.
func TestBuildConfig_MultihopChain(t *testing.T) {
	spec := core.Spec{
		ClashListen: "127.0.0.1:9090",
		Inbounds: []core.Inbound{
			{
				Tag: "entry", Type: "vless", Port: 443,
				Users:                []core.User{{Name: "u1", UUID: "11111111-1111-1111-1111-111111111111"}},
				UpstreamOutboundJSON: `{"type":"vless","tag":"chain-entry","server":"1.2.3.4","server_port":8443,"uuid":"deadbeef"}`,
			},
		},
	}
	raw, err := BuildConfig(spec)
	if err != nil {
		t.Fatalf("build: %v", err)
	}
	var cfg map[string]any
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	outbounds := cfg["outbounds"].([]any)
	if len(outbounds) != 2 { // direct + chain
		t.Fatalf("expected direct + chain outbound, got %d", len(outbounds))
	}
	if cfg["route"] == nil {
		t.Fatal("expected a route section with the chain rule")
	}
	rules := cfg["route"].(map[string]any)["rules"].([]any)
	rule := rules[0].(map[string]any)
	if rule["outbound"] != "chain-entry" {
		t.Fatalf("route rule points at %v, want chain-entry", rule["outbound"])
	}
	inbounds := rule["inbound"].([]any)
	if inbounds[0] != "entry" {
		t.Fatalf("route rule matches inbound %v, want entry", inbounds[0])
	}
}

func TestBuildConfig_DuplicateTag(t *testing.T) {
	_, err := BuildConfig(core.Spec{Inbounds: []core.Inbound{
		{Tag: "dup", Type: "vless", Port: 1},
		{Tag: "dup", Type: "trojan", Port: 2},
	}})
	if err == nil {
		t.Fatal("expected duplicate tag error")
	}
}

func TestBuildConfig_ACMEUsesWritablePersistentDirectory(t *testing.T) {
	raw, err := BuildConfig(core.Spec{Inbounds: []core.Inbound{{
		Tag: "tls-in", Type: "hysteria2", Port: 8443,
		TLS:       &core.TLS{Enabled: true, ServerName: "PL.Example.com"},
		ExtraJSON: []byte(`{"acme":{"email":"ops@example.com","disable_tls_alpn_challenge":true},"happ":{"name":"Poland"}}`),
	}}})
	if err != nil {
		t.Fatal(err)
	}
	var cfg map[string]any
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatal(err)
	}
	inbound := cfg["inbounds"].([]any)[0].(map[string]any)
	if _, ok := inbound["happ"]; ok {
		t.Fatalf("subscription-only metadata leaked into sing-box config: %#v", inbound)
	}
	tls := inbound["tls"].(map[string]any)
	acme := tls["acme"].(map[string]any)
	if acme["data_directory"] != "/etc/honey/sing-box/acme/pl.example.com" {
		t.Fatalf("unexpected ACME data directory: %#v", acme["data_directory"])
	}
	if acme["default_server_name"] != "PL.Example.com" {
		t.Fatalf("default_server_name not set: %#v", acme)
	}
	if _, ok := tls["certificate_path"]; ok {
		t.Fatalf("certificate_path must be removed when ACME is enabled: %#v", tls)
	}
}

func TestBuildConfig_RejectsXrayOnlyTransport(t *testing.T) {
	_, err := BuildConfig(core.Spec{Inbounds: []core.Inbound{{
		Tag: "bad-xhttp", Type: "vless", Port: 443,
		Transport: &core.Transport{Network: "xhttp"},
	}}})
	if err == nil {
		t.Fatal("expected xhttp transport to be rejected by sing-box builder")
	}
}

func TestGeneratedRealityConfigWithInstalledSingBox(t *testing.T) {
	bin := os.Getenv("HONEY_SINGBOX_BIN")
	if bin == "" {
		t.Skip("HONEY_SINGBOX_BIN is not set")
	}
	spec := core.Spec{Inbounds: []core.Inbound{{
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
	}}}
	data, err := BuildConfig(spec)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "sing-box.json")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	if output, err := exec.Command(bin, "check", "-c", path).CombinedOutput(); err != nil {
		t.Fatalf("sing-box rejected generated config: %v\n%s", err, output)
	}
}
