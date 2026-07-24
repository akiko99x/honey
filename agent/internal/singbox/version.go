package singbox

import (
	"context"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// Version runs `sing-box version` and returns the parsed version string.
func Version(ctx context.Context, binPath string) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	out, err := exec.CommandContext(ctx, binPath, "version").Output()
	if err != nil {
		return "", fmt.Errorf("sing-box version: %w", err)
	}

	// first line looks like: "sing-box version 1.9.3"
	line := strings.SplitN(strings.TrimSpace(string(out)), "\n", 2)[0]
	if f := strings.Fields(line); len(f) >= 3 && f[0] == "sing-box" && f[1] == "version" {
		return f[2], nil
	}
	return line, nil
}

// Check validates a config file with `sing-box check -c <path>`.
// Returns the tool's stderr on failure so the master sees a useful reason.
func Check(ctx context.Context, binPath, configPath string) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()

	out, err := exec.CommandContext(ctx, binPath, "check", "-c", configPath).CombinedOutput()
	if err != nil {
		return fmt.Errorf("sing-box check: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}
