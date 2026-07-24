package xray

import (
	"context"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// Version runs `xray version` and returns the parsed version string.
// output looks like: "Xray 1.8.4 (Xray, Penetrates Everything.) ..."
func Version(ctx context.Context, binPath string) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	out, err := exec.CommandContext(ctx, binPath, "version").Output()
	if err != nil {
		return "", fmt.Errorf("xray version: %w", err)
	}

	line := strings.SplitN(strings.TrimSpace(string(out)), "\n", 2)[0]
	if f := strings.Fields(line); len(f) >= 2 && strings.EqualFold(f[0], "xray") {
		return f[1], nil
	}
	return line, nil
}
