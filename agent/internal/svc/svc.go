//go:build linux

// Package svc runs managed external services (a separate data-plane from
// sing-box/xray): a Telegram MTProto proxy via `mtg`, and a NaiveProxy via
// `caddy`. Best-effort, like the wg package: the agent writes a config and
// (re)starts the daemon; a missing binary is logged, never fatal. Requires the
// respective binary on the node (mtg / caddy with forwardproxy).
package svc

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/akiko99x/honey/agent/internal/core"
)

const runDir = "/etc/honey/services"

// Apply (re)configures every desired service. Best-effort per service.
func Apply(services []core.NodeService) error {
	var firstErr error
	for _, s := range services {
		if err := applyOne(s); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func applyOne(s core.NodeService) error {
	switch s.Kind {
	case "mtproto":
		return applyMtproto(s)
	case "naive":
		return applyNaive(s)
	default:
		return fmt.Errorf("unknown service kind %q", s.Kind)
	}
}

func ifName(kind, name string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(name) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
		}
	}
	s := b.String()
	if s == "" {
		s = "0"
	}
	return kind + "-" + s
}

// mtg: `mtg run <toml>` with the operator's global options.
func applyMtproto(s core.NodeService) error {
	if _, err := exec.LookPath("mtg"); err != nil {
		return fmt.Errorf("mtg not installed: %w", err)
	}
	if err := os.MkdirAll(runDir, 0o700); err != nil {
		return err
	}
	var cfg struct {
		Concurrency        int    `json:"concurrency"`
		PreferIP           string `json:"prefer_ip"`
		DomainFrontingPort int    `json:"domain_fronting_port"`
		AntiReplay         bool   `json:"anti_replay"`
	}
	_ = json.Unmarshal([]byte(s.ConfigJSON), &cfg)

	var b strings.Builder
	fmt.Fprintf(&b, "secret = %q\n", s.Secret)
	fmt.Fprintf(&b, "bind-to = \"0.0.0.0:%d\"\n", s.ListenPort)
	if cfg.Concurrency > 0 {
		fmt.Fprintf(&b, "concurrency = %d\n", cfg.Concurrency)
	}
	switch cfg.PreferIP {
	case "prefer-ipv4", "prefer-ipv6", "only-ipv4", "only-ipv6":
		fmt.Fprintf(&b, "prefer-ip = %q\n", cfg.PreferIP)
	}
	if cfg.DomainFrontingPort > 0 {
		fmt.Fprintf(&b, "domain-fronting-port = %d\n", cfg.DomainFrontingPort)
	}
	if cfg.AntiReplay {
		b.WriteString("\n[defense.anti-replay]\nenabled = true\n")
	}

	toml := b.String()
	path := filepath.Join(runDir, ifName("mtg", s.Name)+".toml")
	if err := os.WriteFile(path, []byte(toml), 0o600); err != nil {
		return err
	}
	return supervise("mtg", ifName("mtg", s.Name), "mtg", "run", path)
}

// naive: Caddy with a forward_proxy site, TLS via ACME for the config domain.
func applyNaive(s core.NodeService) error {
	if _, err := exec.LookPath("caddy"); err != nil {
		return fmt.Errorf("caddy not installed: %w", err)
	}
	var cfg struct {
		Username string `json:"username"`
		Domain   string `json:"domain"`
	}
	_ = json.Unmarshal([]byte(s.ConfigJSON), &cfg)
	if cfg.Username == "" {
		cfg.Username = "user"
	}
	if err := os.MkdirAll(runDir, 0o700); err != nil {
		return err
	}
	caddyfile := fmt.Sprintf(
		"{\n  order forward_proxy before reverse_proxy\n}\n:%d, %s {\n  tls internal\n  forward_proxy {\n    basic_auth %s %s\n    hide_ip\n    hide_via\n    probe_resistance\n  }\n}\n",
		s.ListenPort, cfg.Domain, cfg.Username, s.Secret,
	)
	path := filepath.Join(runDir, ifName("naive", s.Name)+".caddy")
	if err := os.WriteFile(path, []byte(caddyfile), 0o600); err != nil {
		return err
	}
	return supervise("caddy", ifName("naive", s.Name), "caddy", "run", "--config", path, "--adapter", "caddyfile")
}

// supervise (re)starts a detached daemon, tracking its PID in a file so a
// re-apply replaces the old process. Best-effort.
func supervise(_ /*tool*/, tag, bin string, args ...string) error {
	pidPath := filepath.Join(runDir, tag+".pid")
	if data, err := os.ReadFile(pidPath); err == nil {
		if pid := strings.TrimSpace(string(data)); pid != "" {
			_ = exec.Command("kill", pid).Run()
		}
	}
	cmd := exec.Command(bin, args...)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start %s: %w", bin, err)
	}
	_ = os.WriteFile(pidPath, []byte(fmt.Sprintf("%d", cmd.Process.Pid)), 0o600)
	// reap asynchronously so the process doesn't become a zombie on exit.
	go func() { _ = cmd.Wait() }()
	return nil
}
