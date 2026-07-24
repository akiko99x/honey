//go:build linux

// Package wg brings up WireGuard / AmneziaWG interfaces from the master's spec.
// It writes a wg-quick(8)/awg-quick(8) config per interface and (re)starts it,
// enabling IP forwarding + NAT for the pool. This is a separate data-plane from
// sing-box/xray. Requires wireguard-tools (or amneziawg-tools) on the node.
package wg

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/akiko99x/honey/agent/internal/core"
)

type awgParams struct {
	Jc   uint32 `json:"jc"`
	Jmin uint32 `json:"jmin"`
	Jmax uint32 `json:"jmax"`
	S1   uint32 `json:"s1"`
	S2   uint32 `json:"s2"`
	H1   uint32 `json:"h1"`
	H2   uint32 `json:"h2"`
	H3   uint32 `json:"h3"`
	H4   uint32 `json:"h4"`
}

// Apply (re)configures every desired interface. Best-effort per interface; the
// first failure is returned but the rest are still attempted.
func Apply(ifaces []core.WgInterface) error {
	var firstErr error
	for _, w := range ifaces {
		if err := applyOne(w); err != nil && firstErr == nil {
			firstErr = err
		}
	}
	return firstErr
}

func applyOne(w core.WgInterface) error {
	tool, confDir := "wg-quick", "/etc/wireguard"
	if w.Amnezia {
		tool, confDir = "awg-quick", "/etc/amnezia/amneziawg"
	}
	if _, err := exec.LookPath(tool); err != nil {
		return fmt.Errorf("%s not installed: %w", tool, err)
	}
	name := ifName(w)
	if err := os.MkdirAll(confDir, 0o700); err != nil {
		return fmt.Errorf("mkdir %s: %w", confDir, err)
	}
	path := filepath.Join(confDir, name+".conf")
	if err := os.WriteFile(path, []byte(serverConf(w)), 0o600); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	// down is best-effort (interface may not be up yet); up is authoritative.
	_ = exec.Command(tool, "down", name).Run()
	if out, err := exec.Command(tool, "up", name).CombinedOutput(); err != nil {
		return fmt.Errorf("%s up %s: %w: %s", tool, name, err, strings.TrimSpace(string(out)))
	}
	return nil
}

func serverConf(w core.WgInterface) string {
	var b strings.Builder
	b.WriteString("[Interface]\n")
	fmt.Fprintf(&b, "Address = %s\n", w.Address)
	fmt.Fprintf(&b, "ListenPort = %d\n", w.ListenPort)
	fmt.Fprintf(&b, "PrivateKey = %s\n", w.PrivateKey)
	if w.MTU > 0 {
		fmt.Fprintf(&b, "MTU = %d\n", w.MTU)
	}
	if w.Amnezia && w.AmneziaParamsJSON != "" {
		var p awgParams
		if json.Unmarshal([]byte(w.AmneziaParamsJSON), &p) == nil {
			fmt.Fprintf(&b, "Jc = %d\nJmin = %d\nJmax = %d\nS1 = %d\nS2 = %d\nH1 = %d\nH2 = %d\nH3 = %d\nH4 = %d\n",
				p.Jc, p.Jmin, p.Jmax, p.S1, p.S2, p.H1, p.H2, p.H3, p.H4)
		}
	}
	// forwarding + NAT for the pool (source = the interface's own subnet).
	b.WriteString("PostUp = sysctl -w net.ipv4.ip_forward=1\n")
	fmt.Fprintf(&b, "PostUp = iptables -t nat -A POSTROUTING -s %s -j MASQUERADE\n", w.Address)
	fmt.Fprintf(&b, "PostDown = iptables -t nat -D POSTROUTING -s %s -j MASQUERADE\n", w.Address)
	for _, peer := range w.Peers {
		b.WriteString("\n[Peer]\n")
		fmt.Fprintf(&b, "PublicKey = %s\n", peer.PublicKey)
		fmt.Fprintf(&b, "AllowedIPs = %s\n", peer.AllowedIP)
	}
	return b.String()
}

// ifName derives a stable, valid (<=15 char) interface name from the interface
// name, prefixed by the kind so wg and awg interfaces never collide.
func ifName(w core.WgInterface) string {
	prefix := "wg-"
	if w.Amnezia {
		prefix = "awg-"
	}
	var sane strings.Builder
	for _, r := range strings.ToLower(w.Name) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			sane.WriteRune(r)
		}
	}
	s := sane.String()
	if s == "" {
		s = "0"
	}
	name := prefix + s
	if len(name) > 15 {
		name = name[:15]
	}
	return name
}
