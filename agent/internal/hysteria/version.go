package hysteria

import (
	"context"
	"os/exec"
	"strings"
)

func version(ctx context.Context, bin string) (string, error) {
	out, err := exec.CommandContext(ctx, bin, "version").CombinedOutput()
	return strings.TrimSpace(string(out)), err
}
