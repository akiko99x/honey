// Package localquota decides which users have exceeded their pushed remaining
// quota since the last master push, for agent-side enforcement between pushes.
package localquota

// Decide returns the set of user names whose usage since baseline meets or
// exceeds their quota. Users with quota 0 (unlimited) are never flagged.
func Decide(totals, baseline, quota map[string]uint64) map[string]bool {
	out := map[string]bool{}
	for name, q := range quota {
		if q == 0 {
			continue
		}
		var delta uint64
		if used := totals[name]; used > baseline[name] {
			delta = used - baseline[name]
		}
		if delta >= q {
			out[name] = true
		}
	}
	return out
}

// Changed reports whether two suppression sets differ.
func Changed(a, b map[string]bool) bool {
	if len(a) != len(b) {
		return true
	}
	for k := range a {
		if !b[k] {
			return true
		}
	}
	return false
}
