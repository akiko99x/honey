//go:build !linux

// Package sysmetrics has no host-metrics source outside Linux; Collect reports
// unsupported so the master can render "n/a" instead of misleading zeros.
package sysmetrics

import "context"

// Sample mirrors the Linux build's shape.
type Sample struct {
	CPUPercent float64
	CPUCores   uint32
	MemTotal   uint64
	MemUsed    uint64
	DiskTotal  uint64
	DiskUsed   uint64
	NetRxSpeed uint64
	NetTxSpeed uint64
	Load1      float64
	UptimeSecs int64
	Supported  bool
}

// Collect reports an unsupported snapshot on non-Linux platforms.
func Collect(_ context.Context) (Sample, error) {
	return Sample{Supported: false}, nil
}
