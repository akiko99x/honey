package localquota

import "testing"

func TestDecide(t *testing.T) {
	totals := map[string]uint64{"alice": 1000, "bob": 500, "carol": 9000}
	baseline := map[string]uint64{"alice": 900, "bob": 100, "carol": 0}
	quota := map[string]uint64{"alice": 50, "bob": 1000, "carol": 0}

	got := Decide(totals, baseline, quota)
	// alice: 100 used >= 50 quota -> suppressed
	if !got["alice"] {
		t.Error("alice should be suppressed")
	}
	// bob: 400 used < 1000 quota -> ok
	if got["bob"] {
		t.Error("bob should not be suppressed")
	}
	// carol: quota 0 (unlimited) -> never suppressed
	if got["carol"] {
		t.Error("carol is unlimited, must not be suppressed")
	}
}

func TestDecideHandlesCounterReset(t *testing.T) {
	// current below baseline (core restarted) must not underflow to a huge delta.
	got := Decide(map[string]uint64{"a": 10}, map[string]uint64{"a": 1000}, map[string]uint64{"a": 5})
	if got["a"] {
		t.Error("a must not be suppressed after a counter reset")
	}
}

func TestChanged(t *testing.T) {
	if Changed(map[string]bool{"a": true}, map[string]bool{"a": true}) {
		t.Error("identical sets are not changed")
	}
	if !Changed(map[string]bool{"a": true}, map[string]bool{"b": true}) {
		t.Error("different sets are changed")
	}
}
