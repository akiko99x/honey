//go:build !linux

// Package svc has no daemon runner outside Linux; Apply is a no-op so
// cross-platform builds succeed. Managed services need Linux + the daemon binary.
package svc

import "github.com/akiko99x/honey/agent/internal/core"

// Apply is a no-op on non-Linux platforms.
func Apply(_ []core.NodeService) error { return nil }
