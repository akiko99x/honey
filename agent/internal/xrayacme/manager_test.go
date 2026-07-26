package xrayacme

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/akiko99x/honey/agent/internal/core"
)

func xrayACMESpec(extra string) core.Spec {
	return core.Spec{Inbounds: []core.Inbound{{
		Core: "xray", Tag: "xray-tls", Type: "vless", Port: 443,
		TLS:       &core.TLS{Enabled: true, ServerName: "vpn.example.com"},
		ExtraJSON: json.RawMessage(extra),
	}}}
}

func TestPrepareValidationUsesDisposableCertificate(t *testing.T) {
	root := t.TempDir()
	manager := New(root, "127.0.0.1:19080", "127.0.0.1:19082")
	spec := xrayACMESpec(`{"acme":{"email":"ops@example.com"}}`)

	cleanup, err := manager.Prepare(context.Background(), &spec, false)
	if err != nil {
		t.Fatal(err)
	}
	defer cleanup()
	if !filepath.IsAbs(spec.Inbounds[0].TLS.CertPath) ||
		!filepath.IsAbs(spec.Inbounds[0].TLS.KeyPath) {
		t.Fatalf("expected temporary absolute certificate paths: %#v", spec.Inbounds[0].TLS)
	}
	if _, err := os.Stat(spec.Inbounds[0].TLS.CertPath); err != nil {
		t.Fatal(err)
	}
}

func TestPrepareRejectsDisabledHTTPChallengeAndWrongPort(t *testing.T) {
	for _, extra := range []string{
		`{"acme":{"email":"ops@example.com","disable_http_challenge":true}}`,
		`{"acme":{"email":"ops@example.com","alternative_http_port":19082}}`,
	} {
		manager := New(t.TempDir(), "127.0.0.1:19080", "127.0.0.1:19082")
		spec := xrayACMESpec(extra)
		_, err := manager.Prepare(context.Background(), &spec, false)
		if err == nil {
			t.Fatalf("expected rejection for %s", extra)
		}
	}
}

func TestRewriteSingboxChallengePort(t *testing.T) {
	manager := New(t.TempDir(), "127.0.0.1:19080", "127.0.0.1:19082")
	spec := core.Spec{Inbounds: []core.Inbound{{
		Core: "singbox", Tag: "sb", Type: "vless", Port: 443,
		TLS:       &core.TLS{Enabled: true, ServerName: "vpn.example.com"},
		ExtraJSON: json.RawMessage(`{"acme":{"email":"ops@example.com","alternative_http_port":19080}}`),
	}}}
	if _, err := manager.Prepare(context.Background(), &spec, false); err != nil {
		t.Fatal(err)
	}
	var extra map[string]any
	if err := json.Unmarshal(spec.Inbounds[0].ExtraJSON, &extra); err != nil {
		t.Fatal(err)
	}
	if got := extra["acme"].(map[string]any)["alternative_http_port"]; got != float64(19082) {
		t.Fatalf("expected sing-box upstream port 19082, got %#v", got)
	}
}

func TestServeHTTPServesOnlyActiveHTTP01Token(t *testing.T) {
	manager := New(t.TempDir(), "127.0.0.1:19080", "127.0.0.1:19082")
	manager.domains["vpn.example.com"] = &managedDomain{
		challenges: map[string]string{"token-123": "token-123.key"},
	}
	req := httptest.NewRequest(http.MethodGet, "http://vpn.example.com/.well-known/acme-challenge/token-123", nil)
	req.Host = "vpn.example.com"
	rec := httptest.NewRecorder()
	manager.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK || rec.Body.String() != "token-123.key" {
		t.Fatalf("unexpected challenge response: %d %q", rec.Code, rec.Body.String())
	}
}
