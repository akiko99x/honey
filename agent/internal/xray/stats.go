package xray

import (
	"context"
	"fmt"
	"strings"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	statscmd "github.com/akiko99x/honey/agent/gen/xray/statscmd"
	"github.com/akiko99x/honey/agent/internal/core"
)

// statsLoop polls xray's StatsService for per-user traffic. counters are
// cumulative since xray start; the agent forwards them as-is and the master
// does delta accounting (keyed by the process epoch).
func (m *Manager) statsLoop(ctx context.Context, interval time.Duration, fn func(core.Stat) error) error {
	conn, err := grpc.NewClient(m.apiAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return fmt.Errorf("xray stats dial %s: %w", m.apiAddr, err)
	}
	defer conn.Close()
	client := statscmd.NewStatsServiceClient(conn)

	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	var prevUp, prevDown uint64
	first := true

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			resp, err := client.QueryStats(ctx, &statscmd.QueryStatsRequest{Pattern: "user>>>"})
			if err != nil {
				return err // stream ends; master retries on the next poll
			}

			up := map[string]uint64{}
			down := map[string]uint64{}
			for _, st := range resp.GetStat() {
				email, dir, ok := parseUserStat(st.GetName())
				if !ok {
					continue
				}
				v := st.GetValue()
				if v < 0 {
					continue
				}
				switch dir {
				case "uplink":
					up[email] += uint64(v)
				case "downlink":
					down[email] += uint64(v)
				}
			}

			var nodeUp, nodeDown uint64
			merged := map[string]struct{}{}
			for e := range up {
				merged[e] = struct{}{}
			}
			for e := range down {
				merged[e] = struct{}{}
			}
			users := make([]core.UserTraffic, 0, len(merged))
			for e := range merged {
				u, d := up[e], down[e]
				nodeUp += u
				nodeDown += d
				users = append(users, core.UserTraffic{Name: e, Up: u, Down: d})
			}

			var upSpeed, downSpeed uint64
			if !first {
				secs := interval.Seconds()
				upSpeed = ratePerSec(nodeUp, prevUp, secs)
				downSpeed = ratePerSec(nodeDown, prevDown, secs)
			}
			first = false
			prevUp, prevDown = nodeUp, nodeDown

			stat := core.Stat{
				NodeUp:    nodeUp,
				NodeDown:  nodeDown,
				UpSpeed:   upSpeed,
				DownSpeed: downSpeed,
				Users:     users,
			}
			if err := fn(stat); err != nil {
				return err
			}
		}
	}
}

// parseUserStat parses "user>>>email>>>traffic>>>uplink|downlink".
func parseUserStat(name string) (email, direction string, ok bool) {
	parts := strings.Split(name, ">>>")
	if len(parts) != 4 || parts[0] != "user" || parts[2] != "traffic" {
		return "", "", false
	}
	return parts[1], parts[3], true
}

func ratePerSec(cur, prev uint64, secs float64) uint64 {
	if cur < prev || secs <= 0 {
		return 0
	}
	return uint64(float64(cur-prev) / secs)
}
