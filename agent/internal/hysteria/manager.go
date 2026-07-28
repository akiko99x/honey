// Package hysteria drives the official Hysteria 2 server binary.
package hysteria

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/akiko99x/honey/agent/internal/core"
)

type Manager struct {
	proc    *core.Process
	binPath string
	client  *http.Client
	statsMu sync.Mutex
	latest  core.Stat
	lastAt  time.Time
}

const (
	statsListen = "127.0.0.1:9091"
	statsSecret = "honey-hysteria-local"
)

func NewManager(binPath, configPath string) *Manager {
	return &Manager{
		proc: core.NewProcess(
			"hysteria", binPath, configPath,
			func(cfg string) []string { return []string{"server", "-c", cfg} },
			nil,
		),
		binPath: binPath,
		client:  &http.Client{Timeout: 3 * time.Second},
	}
}

func (m *Manager) BuildConfig(spec core.Spec) (string, error) {
	if len(spec.Inbounds) == 0 {
		return "", nil
	}
	if len(spec.Inbounds) != 1 {
		return "", fmt.Errorf("official hysteria server supports one hysteria2 inbound per node")
	}
	in := spec.Inbounds[0]
	if in.Type != "hysteria2" {
		return "", fmt.Errorf("official hysteria core only supports hysteria2")
	}
	listen := in.Listen
	if listen == "" {
		listen = "::"
	}
	address := fmt.Sprintf("%s:%d", listen, in.Port)
	if listen == "::" || listen == "*" {
		address = fmt.Sprintf(":%d", in.Port)
	}
	cfg := map[string]any{
		"listen": address,
		"auth": map[string]any{
			"type":     "userpass",
			"userpass": userpass(in.Users),
		},
		"udpIdleTimeout": timeout(in.UdpIdleTimeout),
		"trafficStats": map[string]any{
			"listen": statsListen,
			"secret": statsSecret,
		},
	}
	if in.UdpIdleTimeout == "" {
		if raw, ok := extraString(in.ExtraJSON, "udpIdleTimeout"); ok {
			cfg["udpIdleTimeout"] = raw
		}
	}
	if in.TLS == nil || !in.TLS.Enabled || in.TLS.CertPath == "" || in.TLS.KeyPath == "" {
		return "", fmt.Errorf("hysteria2 inbound %q requires TLS certificate and key", in.Tag)
	}
	cfg["tls"] = map[string]any{
		"cert": in.TLS.CertPath,
		"key":  in.TLS.KeyPath,
	}
	if in.UpMbps > 0 || in.DownMbps > 0 {
		cfg["bandwidth"] = map[string]any{
			"up":   fmt.Sprintf("%d mbps", in.UpMbps),
			"down": fmt.Sprintf("%d mbps", in.DownMbps),
		}
	}
	return marshalWithExtra(cfg, in.ExtraJSON)
}

func extraString(raw json.RawMessage, key string) (string, bool) {
	var extra map[string]any
	if len(raw) == 0 || json.Unmarshal(raw, &extra) != nil {
		return "", false
	}
	value, ok := extra[key].(string)
	return value, ok && strings.TrimSpace(value) != ""
}

func userpass(users []core.User) map[string]string {
	out := make(map[string]string, len(users))
	for _, u := range users {
		if u.Name != "" && u.Password != "" {
			out[u.Name] = u.Password
		}
	}
	return out
}

func timeout(value string) string {
	if strings.TrimSpace(value) == "" {
		return "60s"
	}
	return value
}

func marshalWithExtra(cfg map[string]any, raw json.RawMessage) (string, error) {
	if len(raw) > 0 {
		var extra map[string]any
		if err := json.Unmarshal(raw, &extra); err != nil {
			return "", fmt.Errorf("hysteria extra_json: %w", err)
		}
		for k, v := range extra {
			if k == "acme" || k == "happ" || k == "hop_ports" {
				continue
			}
			cfg[k] = v
		}
	}
	data, err := json.MarshalIndent(cfg, "", "  ")
	return string(data), err
}

func (m *Manager) BuildConfigString(spec core.Spec) (string, error) {
	return m.BuildConfig(spec)
}
func (m *Manager) Validate(config string) error {
	if strings.TrimSpace(config) == "" {
		return nil
	}
	var v map[string]any
	if err := json.Unmarshal([]byte(config), &v); err != nil {
		return fmt.Errorf("hysteria config is not valid JSON: %w", err)
	}
	if _, ok := v["listen"]; !ok {
		return fmt.Errorf("hysteria config has no listen")
	}
	if raw, ok := v["udpIdleTimeout"].(string); ok {
		if _, err := time.ParseDuration(raw); err != nil {
			return fmt.Errorf("invalid Hysteria udpIdleTimeout %q: %w", raw, err)
		}
	}
	return nil
}
func (m *Manager) Start(config string) error         { return m.proc.Start(config) }
func (m *Manager) Stop() error                       { return m.proc.Stop() }
func (m *Manager) Apply(config string) error         { return m.proc.Apply(config) }
func (m *Manager) Status() (core.State, int, string) { return m.proc.Status() }
func (m *Manager) Version(ctx context.Context) (string, error) {
	return version(ctx, m.binPath)
}
func (m *Manager) StatsLoop(ctx context.Context, interval time.Duration, fn func(core.Stat) error) error {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			m.Poll(ctx)
			if err := fn(m.Latest()); err != nil {
				return err
			}
		}
	}
}

type traffic struct {
	TX uint64 `json:"tx"`
	RX uint64 `json:"rx"`
}

// Poll collects and clears native Hysteria counters, accumulating them into the
// same monotonic shape the master already expects from sing-box.
func (m *Manager) Poll(ctx context.Context) bool {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "http://"+statsListen+"/traffic?clear=1", nil)
	if err != nil {
		return false
	}
	req.Header.Set("Authorization", statsSecret)
	resp, err := m.client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return false
	}
	var sample map[string]traffic
	if err := json.NewDecoder(resp.Body).Decode(&sample); err != nil {
		return false
	}

	now := time.Now()
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	seconds := now.Sub(m.lastAt).Seconds()
	var deltaUp, deltaDown uint64
	users := make(map[string]core.UserTraffic, len(m.latest.Users))
	for _, user := range m.latest.Users {
		users[user.Name] = user
	}
	for name, value := range sample {
		user := users[name]
		user.Name = name
		// Hysteria reports tx/rx from the server's perspective.
		user.Up += value.RX
		user.Down += value.TX
		users[name] = user
		deltaUp += value.RX
		deltaDown += value.TX
	}
	out := make([]core.UserTraffic, 0, len(users))
	for _, user := range users {
		out = append(out, user)
	}
	m.latest.NodeUp += deltaUp
	m.latest.NodeDown += deltaDown
	if seconds > 0 {
		m.latest.UpSpeed = uint64(float64(deltaUp) / seconds)
		m.latest.DownSpeed = uint64(float64(deltaDown) / seconds)
	}
	m.latest.Users = out
	m.lastAt = now
	return true
}

func (m *Manager) Latest() core.Stat {
	m.statsMu.Lock()
	defer m.statsMu.Unlock()
	return m.latest
}
