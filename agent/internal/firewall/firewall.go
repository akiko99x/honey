// Package firewall installs the UDP redirect rules for Hysteria2 port hopping.
//
// It owns a dedicated, isolated nftables table ("inet honey") that is fully
// replaced on every Apply, so it never touches the operator's own rules and is
// trivially reversible. Everything here is best-effort: a missing nft or a
// failed command is returned for the caller to log, never fatal — the client
// still learns the port range from its subscription.
package firewall

import (
	"fmt"
	"os/exec"
	"strings"
)

const table = "inet honey"

// HopRule redirects a UDP port range to the inbound's real listen port.
type HopRule struct {
	Start int
	End   int
	To    int
}

// Apply replaces honey's nftables table with these rules. An empty slice removes
// the table (cleanup).
func Apply(rules []HopRule) error {
	if _, err := exec.LookPath("nft"); err != nil {
		if len(rules) == 0 {
			return nil
		}
		return fmt.Errorf("nft not found — set the UDP redirect manually")
	}

	// start from a clean slate every time (ignore "no such table").
	_ = runArgs("delete", "table", "inet", "honey")
	if len(rules) == 0 {
		return nil
	}

	var b strings.Builder
	fmt.Fprintf(&b, "add table %s\n", table)
	fmt.Fprintf(&b, "add chain %s hop { type nat hook prerouting priority dstnat; }\n", table)
	for _, r := range rules {
		fmt.Fprintf(&b, "add rule %s hop udp dport %d-%d redirect to :%d\n", table, r.Start, r.End, r.To)
	}
	return runScript(b.String())
}

func runArgs(args ...string) error {
	out, err := exec.Command("nft", args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("nft %s: %v: %s", strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}
	return nil
}

func runScript(script string) error {
	cmd := exec.Command("nft", "-f", "-")
	cmd.Stdin = strings.NewReader(script)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("nft -f: %v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}
