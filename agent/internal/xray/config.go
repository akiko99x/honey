package xray

import (
	"encoding/json"
	"fmt"
	"net"
	"strconv"

	"github.com/akiko99x/honey/agent/internal/core"
)

// BuildConfig turns a core.Spec into an xray config.json.
//
// apiAddr (host:port) enables the gRPC StatsService via a dokodemo-door "api"
// inbound + routing rule, so the agent can read per-user traffic. stats/policy
// are always emitted so counters are actually kept.
//
// NOTE: xray's schema differs from sing-box; this covers the common inbounds
// (vless/vmess/trojan/shadowsocks) with tls/reality. protocol-specific extras
// go through extra_json (merged into the inbound).
func BuildConfig(spec core.Spec, apiAddr string) ([]byte, error) {
	inbounds := make([]map[string]any, 0, len(spec.Inbounds)+1)
	seen := map[string]bool{}
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
	}

	logLevel := spec.LogLevel
	if logLevel == "" {
		logLevel = "warning"
	}

	cfg := map[string]any{
		"log":   map[string]any{"loglevel": logLevel},
		"stats": map[string]any{},
		"policy": map[string]any{
			"levels": map[string]any{
				"0": map[string]any{"statsUserUplink": true, "statsUserDownlink": true},
			},
			"system": map[string]any{"statsInboundUplink": true, "statsInboundDownlink": true},
		},
		"inbounds": inbounds,
		"outbounds": []map[string]any{
			{"protocol": "freedom", "tag": "direct"},
		},
	}

	// wire the gRPC stats api when we have an address for it.
	if _, _, ok := splitHostPort(apiAddr); ok {
		// Xray 1.8.12+ can expose the API directly. This is the current
		// documented form and avoids a synthetic tunnel inbound + route.
		cfg["api"] = map[string]any{
			"tag": "api", "listen": apiAddr, "services": []string{"StatsService"},
		}
	}

	return json.MarshalIndent(cfg, "", "  ")
}

func splitHostPort(addr string) (string, int, bool) {
	host, portStr, err := net.SplitHostPort(addr)
	if err != nil {
		return "", 0, false
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return "", 0, false
	}
	if host == "" {
		host = "127.0.0.1"
	}
	return host, port, true
}

func buildInbound(in core.Inbound) (map[string]any, error) {
	listen := in.Listen
	if listen == "" {
		listen = "::"
	}

	protocol := in.Type
	if in.Type == "hysteria2" {
		// Xray calls both its Hysteria2 protocol and QUIC transport "hysteria".
		protocol = "hysteria"
		if in.TLS == nil || !in.TLS.Enabled {
			return nil, fmt.Errorf("xray hysteria2 inbound %q requires TLS", in.Tag)
		}
		if in.TLS.Reality != nil {
			return nil, fmt.Errorf("xray hysteria2 inbound %q does not support REALITY", in.Tag)
		}
	}

	m := map[string]any{
		"tag":      in.Tag,
		"listen":   listen,
		"port":     in.Port,
		"protocol": protocol,
		"settings": buildSettings(in.Type, in.Users),
	}

	if stream := buildStream(in); stream != nil {
		m["streamSettings"] = stream
	}

	if len(in.ExtraJSON) > 0 {
		var extra map[string]any
		if err := json.Unmarshal(in.ExtraJSON, &extra); err != nil {
			return nil, fmt.Errorf("inbound %q extra_json: %w", in.Tag, err)
		}
		// Used by Honey control-plane features; Xray rejects unknown top-level
		// inbound fields. ACME has already been translated to certificate paths
		// by the agent's Xray certificate manager.
		delete(extra, "happ")
		delete(extra, "acme")
		for k, v := range extra {
			m[k] = v
		}
	}
	return m, nil
}

func buildSettings(inboundType string, users []core.User) map[string]any {
	if inboundType == "hysteria2" {
		hysteriaUsers := make([]map[string]any, 0, len(users))
		for _, u := range users {
			user := map[string]any{"auth": u.Name + ":" + u.Password}
			if u.Name != "" {
				user["email"] = u.Name
			}
			hysteriaUsers = append(hysteriaUsers, user)
		}
		return map[string]any{
			"version": 2,
			"users":   hysteriaUsers,
		}
	}

	clients := make([]map[string]any, 0, len(users))
	for _, u := range users {
		c := map[string]any{}
		if u.Name != "" {
			c["email"] = u.Name // xray keys per-user stats by email
		}
		switch inboundType {
		case "vless", "vmess":
			c["id"] = u.UUID
			if inboundType == "vless" && u.Flow != "" {
				c["flow"] = u.Flow
			}
		case "trojan", "shadowsocks":
			c["password"] = u.Password
		default:
			if u.UUID != "" {
				c["id"] = u.UUID
			}
			if u.Password != "" {
				c["password"] = u.Password
			}
		}
		clients = append(clients, c)
	}

	settings := map[string]any{"clients": clients}
	if inboundType == "vless" {
		settings["decryption"] = "none"
	}
	return settings
}

// buildStream assembles xray streamSettings: the network transport plus its
// per-network settings and the security layer (reality / tls / none).
func buildStream(in core.Inbound) map[string]any {
	network := "tcp"
	if in.Type == "hysteria2" {
		network = "hysteria"
	} else if in.Transport != nil && in.Transport.Network != "" {
		network = in.Transport.Network
	}
	if network == "hysteria" {
		return buildHysteriaStream(in.TLS)
	}
	switch network { // normalise aliases to xray names
	case "mkcp":
		network = "kcp"
	case "h2":
		network = "http"
	}

	stream := map[string]any{}
	content := false

	if network != "tcp" {
		stream["network"] = network
		content = true
		if s := buildNetworkSettings(network, in.Transport); s != nil {
			stream[networkSettingsKey(network)] = s
		}
	}

	if t := in.TLS; t != nil && t.Enabled {
		content = true
		if t.Reality != nil {
			port := t.Reality.HandshakePort
			if port == 0 {
				port = 443
			}
			stream["security"] = "reality"
			stream["realitySettings"] = map[string]any{
				"show":        false,
				"target":      fmt.Sprintf("%s:%d", t.Reality.HandshakeServer, port),
				"privateKey":  t.Reality.PrivateKey,
				"shortIds":    t.Reality.ShortIDs,
				"serverNames": []string{t.ServerName},
			}
		} else {
			tls := map[string]any{}
			if t.ServerName != "" {
				tls["serverName"] = t.ServerName
			}
			if t.CertPath != "" || t.KeyPath != "" {
				tls["certificates"] = []map[string]any{
					{"certificateFile": t.CertPath, "keyFile": t.KeyPath},
				}
			}
			stream["security"] = "tls"
			stream["tlsSettings"] = tls
		}
	}

	if !content {
		return nil
	}
	return stream
}

func buildHysteriaStream(t *core.TLS) map[string]any {
	stream := map[string]any{
		"method":           "hysteria",
		"hysteriaSettings": map[string]any{"version": 2},
		"security":         "tls",
	}
	tls := map[string]any{}
	if t != nil {
		if t.ServerName != "" {
			tls["serverName"] = t.ServerName
		}
		if t.CertPath != "" || t.KeyPath != "" {
			tls["certificates"] = []map[string]any{
				{"certificateFile": t.CertPath, "keyFile": t.KeyPath},
			}
		}
	}
	stream["tlsSettings"] = tls
	return stream
}

func networkSettingsKey(network string) string {
	switch network {
	case "ws":
		return "wsSettings"
	case "grpc":
		return "grpcSettings"
	case "http":
		return "httpSettings"
	case "httpupgrade":
		return "httpupgradeSettings"
	case "xhttp":
		return "xhttpSettings"
	case "kcp":
		return "kcpSettings"
	case "quic":
		return "quicSettings"
	default:
		return network + "Settings"
	}
}

func buildNetworkSettings(network string, t *core.Transport) map[string]any {
	if t == nil {
		return nil
	}
	switch network {
	case "ws":
		m := map[string]any{}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["headers"] = map[string]any{"Host": t.Host}
		}
		return m
	case "grpc":
		return map[string]any{"serviceName": t.ServiceName}
	case "http":
		m := map[string]any{}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["host"] = []string{t.Host}
		}
		return m
	case "httpupgrade":
		m := map[string]any{}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["host"] = t.Host
		}
		return m
	case "xhttp":
		m := map[string]any{}
		if t.Path != "" {
			m["path"] = t.Path
		}
		if t.Host != "" {
			m["host"] = t.Host
		}
		if t.Mode != "" {
			m["mode"] = t.Mode
		}
		return m
	case "kcp":
		return map[string]any{"header": map[string]any{"type": "none"}}
	case "quic":
		return map[string]any{"security": "none", "header": map[string]any{"type": "none"}}
	default:
		return nil
	}
}
