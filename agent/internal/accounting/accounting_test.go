package accounting

import (
	"path/filepath"
	"testing"
)

func TestSaveLoadRoundtrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "stats.json")
	s := Load(path)
	epoch := s.Epoch
	if epoch == "" {
		t.Fatal("expected a generated epoch")
	}
	s.SetCounters(map[string]uint64{"alice": 100, "bob": 5}, map[string]uint64{"alice": 200})
	if err := s.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	reloaded := Load(path)
	if reloaded.Epoch != epoch {
		t.Fatalf("epoch not preserved: %q != %q", reloaded.Epoch, epoch)
	}
	up, down := reloaded.Counters()
	if up["alice"] != 100 || down["alice"] != 200 || up["bob"] != 5 {
		t.Fatalf("counters not preserved: up=%v down=%v", up, down)
	}
}

func TestFreshStoreHasEpoch(t *testing.T) {
	s := Load(filepath.Join(t.TempDir(), "missing.json"))
	if s.Epoch == "" {
		t.Fatal("fresh store must have an epoch")
	}
	if len(s.Users) != 0 {
		t.Fatal("fresh store must start empty")
	}
}
