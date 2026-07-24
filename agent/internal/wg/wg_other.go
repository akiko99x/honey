//go:build !linux

// Package wg has no data-plane outside Linux; Apply is a no-op so cross-platform
// builds succeed. WireGuard interface management needs Linux + wireguard-tools.
package wg

import "github.com/akiko99x/honey/agent/internal/core"

// Apply is a no-op on non-Linux platforms.
func Apply(_ []core.WgInterface) error { return nil }
