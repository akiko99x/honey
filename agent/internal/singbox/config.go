package singbox

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/akiko99x/honey/agent/internal/core"
)

// BuildConfig turns a core.Spec into a sing-box config.json byte slice.
// The Clash API block is always emitted so the agent can read live stats.
func BuildConfig(spec core.Spec) ([]byte, error) {
	if spec.LogLevel == "" {
		spec.LogLevel = "info"
	}
	if spec.ClashListen == "" {
		spec.ClashListen = "127.0.0.1:9090"
	}

	inbounds := make([]map[string]any, 0, len(spec.Inbounds))
	seen := map[string]bool{}
	// multihop: chain outbounds (one per entry inbound) + route rules that send
	// that inbound's traffic through its chain outbound instead of direct.
	outbounds := []map[string]any{{"type": "direct", "tag": "direct"}}
	var routeRules []map[string]any
	for _, in := range spec.Inbounds {
		if in.Tag == "" || in.Type == "" {
			return nil, fmt.Errorf("inbound needs both tag and type")
		}
		if seen[in.Tag] {
			return nil, fmt.Errorf("duplicate inbound tag %q", in.Tag)
		}
		seen[in.Tag] = true

		m, err := buildInbound(in)
		if err != nil {
			return nil, err
		}
		inbounds = append(inbounds, m)

		if in.UpstreamOutboundJSON != "" {
			var ob map[string]any
			if err := json.Unmarshal([]byte(in.UpstreamOutboundJSON), &ob); err != nil {
				return nil, fmt.Errorf("inbound %q upstream outbound: %w", in.Tag, err)
			}
			tag, _ := ob["tag"].(string)
			if tag == "" {
				return nil, fmt.Errorf("inbound %q upstream outbound has no tag", in.Tag)
			}
			outbounds = append(outbounds, ob)
			routeRules = append(routeRules, map[string]any{
				"inbound":  []string{in.Tag},
				"outbound": tag,
			})
		}
	}

	cfg := map[string]any{
		"log": map[string]any{"level": spec.LogLevel, "timestamp": true},
		"experimental": map[string]any{
			"clash_api": map[string]any{
				"external_controller": spec.ClashListen,
				"secret":              spec.ClashSecret,
			},
		},
		"inbounds":  inbounds,
		"outbounds": outbounds,
	}
	if len(routeRules) > 0 {
		cfg["route"] = map[string]any{"rules": routeRules, "final": "direct"}
	}
	return json.MarshalIndent(cfg, "", "  ")
}

func buildInbound(in core.Inbound) (map[string]any, error) {
	listen := in.Listen
	if listen == "" {
		listen = "::"
	}

	m := map[string]any{
		"type":        in.Type,
		"tag":         in.Tag,
		"listen":      listen,
		"listen_port": in.Port,
	}

	if users := buildUsers(in.Type, in.Users); len(users) > 0 {
		m["users"] = users
	}
	// traffic shaping: hysteria2 takes native per-inbound bandwidth caps that
	// drive its congestion control. Other protocols have no core-level limiter.
	if in.Type == "hysteria2" {
		if in.UpMbps > 0 {
			m["up_mbps"] = in.UpMbps
		}
		if in.DownMbps > 0 {
			m["down_mbps"] = in.DownMbps
		}
	}
	// shadowtls is a tls-masquerade wrapper: no tls block, it carries a handshake.
	if in.TLS != nil && in.TLS.Enabled && in.Type != "shadowtls" {
		m["tls"] = buildTLS(in.TLS)
	}
	tr, err := buildTransport(in.Transport)
	if err != nil {
		return nil, fmt.Errorf("inbound %q: %w", in.Tag, err)
	}
	if tr != nil {
		m["transport"] = tr
	}
	if in.Type == "shadowtls" {
		m["version"] = 3
		if in.TLS != nil && in.TLS.ShadowTLSHandshakeServer != "" {
			port := in.TLS.ShadowTLSHandshakePort
			if port == 0 {
				port = 443
			}
			m["handshake"] = map[string]any{
				"server":      in.TLS.ShadowTLSHandshakeServer,
				"server_port": port,
			}
		}
	}

	if len(in.ExtraJSON) > 0 {
		var extra map[string]any
		if err := json.Unmarshal(in.ExtraJSON, &extra); err != nil {
			return nil, fmt.Errorf("inbound %q extra_json: %w", in.Tag, err)
		}
		// "acme" is routed into tls.acme (sing-box's native ACME), not the inbound
		// top level. sing-box then obtains + auto-renews the cert itself.
		if raw, ok := extra["acme"]; ok {
			delete(extra, "acme")
			if tlsMap, ok := m["tls"].(map[string]any); ok {
				delete(tlsMap, "certificate_path")
				delete(tlsMap, "key_path")
				tlsMap["acme"] = acmeBlock(raw, in.TLS)
			}
		}
		// Subscription presentation metadata belongs to the control plane and
		// is not part of sing-box's inbound schema.
		delete(extra, "happ")
		for k, v := range extra {
			m[k] = v
		}
	}
	return m, nil
}

// acmeBlock turns an inbound's extra_json "acme" value into sing-box's tls.acme
// object. The value may be a bool or an object carrying email/domain/etc.; the
// domain defaults to the inbound's server_name.
func acmeBlock(raw any, t *core.TLS) map[string]any {
	block := map[string]any{}
	if obj, ok := raw.(map[string]any); ok {
		for k, v := range obj {
			block[k] = v
		}
	}
	if _, ok := block["domain"]; !ok && t != nil && t.ServerName != "" {
		block["domain"] = []string{t.ServerName}
	}
	// honey-agent runs with ProtectSystem=strict. CertMagic's default data
	// directory resolves below the service account's home and is not writable
	// inside that sandbox, so issuance used to fail before completing a
	// challenge. Keep account keys and renewed certificates in the directory
	// explicitly writable by honey-agent.service.
	if _, ok := block["data_directory"]; !ok {
		domain := "default"
		if t != nil && t.ServerName != "" {
			domain = safePathPart(t.ServerName)
		}
		block["data_directory"] = "/etc/honey/sing-box/acme/" + domain
	}
	if _, ok := block["default_server_name"]; !ok && t != nil && t.ServerName != "" {
		block["default_server_name"] = t.ServerName
	}
	return block
}

func safePathPart(value string) string {
	var out strings.Builder
	for _, r := range strings.ToLower(value) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '.' || r == '-' {
			out.WriteRune(r)
		} else {
			out.WriteByte('-')
		}
	}
	clean := strings.Trim(out.String(), ".-")
	if clean == "" {
		return "default"
	}
	return clean
}

func buildUsers(inboundType string, users []core.User) []map[string]any {
	out := make([]map[string]any, 0, len(users))
	for _, u := range users {
		um := map[string]any{}
		if u.Name != "" {
			um["name"] = u.Name
		}
		switch inboundType {
		case "vless", "vmess":
			um["uuid"] = u.UUID
			if inboundType == "vless" && u.Flow != "" {
				um["flow"] = u.Flow
			}
		case "trojan", "hysteria2", "tuic", "shadowsocks", "anytls", "shadowtls":
			um["password"] = u.Password
		default:
			if u.UUID != "" {
				um["uuid"] = u.UUID
			}
			if u.Password != "" {
				um["password"] = u.Password
			}
			if u.Flow != "" {
				um["flow"] = u.Flow
			}
		}
		out = append(out, um)
	}
	return out
}

// buildTransport emits sing-box's v2ray-transport object (vless/vmess/trojan).
// xhttp/mkcp are xray-only; reject them instead of silently emitting raw TCP.
func buildTransport(t *core.Transport) (map[string]any, error) {
	if t == nil || t.Network == "" || t.Network == "tcp" {
		return nil, nil
	}
	switch t.Network {
	case "ws":
		m := map[string]any{"type": "ws"}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["headers"] = map[string]any{"Host": t.Host}
		}
		return m, nil
	case "grpc":
		return map[string]any{"type": "grpc", "service_name": t.ServiceName}, nil
	case "http", "h2":
		m := map[string]any{"type": "http"}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["host"] = []string{t.Host}
		}
		return m, nil
	case "httpupgrade":
		m := map[string]any{"type": "httpupgrade"}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["host"] = t.Host
		}
		return m, nil
	case "quic":
		return map[string]any{"type": "quic"}, nil
	case "xhttp", "mkcp":
		return nil, fmt.Errorf("transport %q is supported only by xray", t.Network)
	default:
		return nil, fmt.Errorf("unsupported transport %q", t.Network)
	}
}

func buildTLS(t *core.TLS) map[string]any {
	tls := map[string]any{"enabled": true}
	if t.ServerName != "" {
		tls["server_name"] = t.ServerName
	}
	if t.CertPath != "" {
		tls["certificate_path"] = t.CertPath
	}
	if t.KeyPath != "" {
		tls["key_path"] = t.KeyPath
	}
	if t.Reality != nil {
		port := t.Reality.HandshakePort
		if port == 0 {
			port = 443
		}
		reality := map[string]any{
			"enabled":     true,
			"private_key": t.Reality.PrivateKey,
			"handshake": map[string]any{
				"server":      t.Reality.HandshakeServer,
				"server_port": port,
			},
		}
		if len(t.Reality.ShortIDs) > 0 {
			reality["short_id"] = t.Reality.ShortIDs
		}
		tls["reality"] = reality
	}
	return tls
}
