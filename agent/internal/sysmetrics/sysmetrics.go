//go:build linux

// Package sysmetrics reads live host system metrics from Linux /proc and
// statfs. Rates (CPU%, network) are sampled over a short window inside Collect.
package sysmetrics

import (
	"bufio"
	"context"
	"os"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// Sample is a point-in-time host snapshot.
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

// Collect gathers a full snapshot. CPU% and net rates need two reads, so it
// sleeps briefly (respecting ctx) between them.
func Collect(ctx context.Context) (Sample, error) {
	s := Sample{Supported: true, CPUCores: uint32(runtime.NumCPU())}

	busy1, total1 := readCPU()
	rx1, tx1 := readNet()
	start := time.Now()

	select {
	case <-ctx.Done():
		return Sample{Supported: true}, ctx.Err()
	case <-time.After(300 * time.Millisecond):
	}

	busy2, total2 := readCPU()
	rx2, tx2 := readNet()
	dt := time.Since(start).Seconds()

	if dtotal := total2 - total1; dtotal > 0 {
		s.CPUPercent = float64(busy2-busy1) / float64(dtotal) * 100
		if s.CPUPercent < 0 {
			s.CPUPercent = 0
		}
	}
	if dt > 0 {
		if rx2 >= rx1 {
			s.NetRxSpeed = uint64(float64(rx2-rx1) / dt)
		}
		if tx2 >= tx1 {
			s.NetTxSpeed = uint64(float64(tx2-tx1) / dt)
		}
	}

	s.MemTotal, s.MemUsed = readMem()
	s.DiskTotal, s.DiskUsed = readDisk("/")
	s.Load1 = readLoad1()
	s.UptimeSecs = readUptime()
	return s, nil
}

// readCPU returns (busy, total) jiffies from the aggregate /proc/stat cpu line.
func readCPU() (uint64, uint64) {
	f, err := os.Open("/proc/stat")
	if err != nil {
		return 0, 0
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := sc.Text()
		if !strings.HasPrefix(line, "cpu ") {
			continue
		}
		fields := strings.Fields(line)[1:]
		var total, idle uint64
		for i, fld := range fields {
			v, _ := strconv.ParseUint(fld, 10, 64)
			total += v
			if i == 3 || i == 4 { // idle + iowait
				idle += v
			}
		}
		return total - idle, total
	}
	return 0, 0
}

// readNet sums rx/tx bytes across interfaces except loopback.
func readNet() (uint64, uint64) {
	f, err := os.Open("/proc/net/dev")
	if err != nil {
		return 0, 0
	}
	defer f.Close()
	var rx, tx uint64
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := sc.Text()
		colon := strings.IndexByte(line, ':')
		if colon < 0 {
			continue
		}
		iface := strings.TrimSpace(line[:colon])
		if iface == "lo" || iface == "" {
			continue
		}
		fields := strings.Fields(line[colon+1:])
		if len(fields) < 9 {
			continue
		}
		r, _ := strconv.ParseUint(fields[0], 10, 64)
		t, _ := strconv.ParseUint(fields[8], 10, 64)
		rx += r
		tx += t
	}
	return rx, tx
}

func readMem() (total, used uint64) {
	f, err := os.Open("/proc/meminfo")
	if err != nil {
		return 0, 0
	}
	defer f.Close()
	var memTotal, memAvail uint64
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		fields := strings.Fields(sc.Text())
		if len(fields) < 2 {
			continue
		}
		v, _ := strconv.ParseUint(fields[1], 10, 64) // kB
		switch fields[0] {
		case "MemTotal:":
			memTotal = v * 1024
		case "MemAvailable:":
			memAvail = v * 1024
		}
	}
	if memAvail > memTotal {
		memAvail = memTotal
	}
	return memTotal, memTotal - memAvail
}

func readDisk(path string) (total, used uint64) {
	var st syscall.Statfs_t
	if err := syscall.Statfs(path, &st); err != nil {
		return 0, 0
	}
	bsize := uint64(st.Bsize)
	total = st.Blocks * bsize
	used = (st.Blocks - st.Bfree) * bsize
	return total, used
}

func readLoad1() float64 {
	b, err := os.ReadFile("/proc/loadavg")
	if err != nil {
		return 0
	}
	fields := strings.Fields(string(b))
	if len(fields) == 0 {
		return 0
	}
	v, _ := strconv.ParseFloat(fields[0], 64)
	return v
}

func readUptime() int64 {
	b, err := os.ReadFile("/proc/uptime")
	if err != nil {
		return 0
	}
	fields := strings.Fields(string(b))
	if len(fields) == 0 {
		return 0
	}
	v, _ := strconv.ParseFloat(fields[0], 64)
	return int64(v)
}
