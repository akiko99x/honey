// Package logx is honey's tiny structured logger for the agent and its cores.
//
// Every line is `<level> <code> <message>`, where code is a machine-readable
// taxonomy entry (see docs/error-codes.md) and message is loose lowercase
// english. Levels stay in the line so logs are greppable without a json parser.
package logx

import (
	"fmt"
	"log"
	"os"
	"regexp"
	"sync"
	"time"
)

// Agent codes (A###) — the agent process itself.
const (
	AgentBooting         = "A0101"
	AgentMTLSFailed      = "A0102"
	AgentTransportWiring = "A0103"
	AgentTransportUp     = "A0104"
	AgentTransportDown   = "A0105"
	AgentShutdown        = "A0106"
	AgentResumed         = "A0107"
	AgentResumeFailed    = "A0108"
	AgentResumeEmpty     = "A0109"
	AgentResumeStat      = "A0110"
	AgentDialRetry       = "A0201"
	AgentDialUp          = "A0202"
	AgentListenFailed    = "A0203"
	AgentWhoRU           = "A0301"
	AgentApplyRecv       = "A0302"
	AgentStartRecv       = "A0303"
	AgentStopRecv        = "A0304"
	AgentStatsRecv       = "A0305"
	AgentUnknownCore     = "A0306"
	AgentLogsRecv        = "A0307"
	EnrollStart          = "A0401"
	EnrollKeygen         = "A0402"
	EnrollRejected       = "A0403"
	EnrollBadResponse    = "A0404"
	EnrollDone           = "A0405"
	EnrollFatal          = "A0499"
)

// Node/core codes (N###) — the sing-box / xray cores driven by the agent.
const (
	CoreConfigBuilding    = "N0101"
	CoreConfigBuildFailed = "N0102"
	CoreConfigInvalid     = "N0103"
	CoreStarting          = "N0104"
	CoreStarted           = "N0105"
	CoreStartFailed       = "N0106"
	CoreStopping          = "N0107"
	CoreStopped           = "N0108"
	CoreCrashed           = "N0109"
	CoreKilled            = "N0110"
	CoreApplying          = "N0111"
	CoreRolledBack        = "N0112"
	CoreRollbackFailed    = "N0113"
	CoreInstallFailed     = "N0114"
	CoreVersion           = "N0201"
	CoreVersionFailed     = "N0202"
	CoreStatsPaused       = "N0301"
	CoreStatsResumed      = "N0302"
	CoreStatsXrayFailed   = "N0303"
	CoreFirewall          = "N0304"
	CoreWireguard         = "N0305"
	CoreQuota             = "N0306"
	CoreService           = "N0307"
)

var logger = log.New(os.Stderr, "", log.LstdFlags|log.LUTC)

var sensitiveValue = regexp.MustCompile(`(?i)(private[_-]?key|password|secret|token|authorization)(["'\s]*[:=][\s]*["']?)[^"',;\s}]+`)

const ringCapacity = 1000

// Record is the safe, structured representation exposed to the authenticated
// master. The ring is process-local and intentionally excludes core stdout:
// callers must not ship generated configs or credential-bearing raw output.
type Record struct {
	Seq      uint64
	AtUnixMS int64
	Level    string
	Code     string
	Message  string
}

var ring = struct {
	sync.Mutex
	next    uint64
	records []Record
}{next: 1}

func emit(level, code, format string, args ...any) {
	message := fmt.Sprintf(format, args...)
	logger.Printf("%-5s %s  %s", level, code, message)
	ring.Lock()
	record := Record{
		Seq:      ring.next,
		AtUnixMS: time.Now().UnixMilli(),
		Level:    level,
		Code:     code,
		Message:  sanitizeForShipping(message),
	}
	ring.next++
	if len(ring.records) == ringCapacity {
		copy(ring.records, ring.records[1:])
		ring.records[len(ring.records)-1] = record
	} else {
		ring.records = append(ring.records, record)
	}
	ring.Unlock()
}

func sanitizeForShipping(message string) string {
	return sensitiveValue.ReplaceAllString(message, `${1}${2}[redacted]`)
}

// Snapshot returns retained records after the exclusive cursor. A zero limit
// uses a practical default; large requests are bounded to keep the RPC cheap.
func Snapshot(afterSeq uint64, limit uint32) []Record {
	if limit == 0 {
		limit = 200
	}
	if limit > 500 {
		limit = 500
	}
	ring.Lock()
	defer ring.Unlock()
	start := 0
	for start < len(ring.records) && ring.records[start].Seq <= afterSeq {
		start++
	}
	if remaining := len(ring.records) - start; remaining > int(limit) {
		start = len(ring.records) - int(limit)
	}
	result := make([]Record, len(ring.records)-start)
	copy(result, ring.records[start:])
	return result
}

// Debug is for chatty, expected events.
func Debug(code, format string, args ...any) { emit("debug", code, format, args...) }

// Info is for normal milestones.
func Info(code, format string, args ...any) { emit("info", code, format, args...) }

// Warn is for recoverable trouble that deserves attention.
func Warn(code, format string, args ...any) { emit("warn", code, format, args...) }

// Error is for actions that failed.
func Error(code, format string, args ...any) { emit("error", code, format, args...) }

// Fatal logs at error and exits non-zero. Boot-time / CLI use only.
func Fatal(code, format string, args ...any) {
	emit("error", code, format, args...)
	os.Exit(1)
}
