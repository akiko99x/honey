package core

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// Recovery permission is separate from the core JSON: a stale JSON file must
// never by itself authorize a core to restart after an agent reboot.
type recoveryState struct {
	Version int    `json:"version"`
	Active  bool   `json:"active"`
	SHA256  string `json:"sha256,omitempty"`
}

func recoveryStatePath(configPath string) string { return configPath + ".honey-state.json" }

// ShouldRecover requires an explicit active marker and a matching config hash.
// Missing state is intentionally inactive for safe upgrades.
func ShouldRecover(configPath string) (bool, error) {
	data, err := os.ReadFile(recoveryStatePath(configPath))
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	var state recoveryState
	if err := json.Unmarshal(data, &state); err != nil {
		return false, fmt.Errorf("parse recovery state: %w", err)
	}
	if state.Version != 1 || !state.Active {
		return false, nil
	}
	config, err := os.ReadFile(configPath)
	if err != nil {
		return false, fmt.Errorf("read active core config: %w", err)
	}
	if got := configHash(config); got != state.SHA256 {
		return false, fmt.Errorf("recovery state hash mismatch: want %s, got %s", state.SHA256, got)
	}
	return true, nil
}

func MarkActive(configPath string) error {
	config, err := os.ReadFile(configPath)
	if err != nil {
		return fmt.Errorf("read active core config: %w", err)
	}
	return writeRecoveryState(configPath, recoveryState{Version: 1, Active: true, SHA256: configHash(config)})
}

// Mark inactive before a deliberate stop. If stopping later fails, a reboot
// still prefers keeping the core off to resurrecting a config no longer desired.
func MarkInactive(configPath string) error {
	return writeRecoveryState(configPath, recoveryState{Version: 1, Active: false})
}

func configHash(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

func writeRecoveryState(configPath string, state recoveryState) error {
	dir := filepath.Dir(recoveryStatePath(configPath))
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	data, err := json.Marshal(state)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".honey-state-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	dst := recoveryStatePath(configPath)
	if err := os.Rename(tmpPath, dst); err != nil {
		// Windows cannot replace an existing destination; production Linux uses
		// the atomic rename above.
		if removeErr := os.Remove(dst); removeErr != nil && !errors.Is(removeErr, os.ErrNotExist) {
			return err
		}
		return os.Rename(tmpPath, dst)
	}
	return nil
}
