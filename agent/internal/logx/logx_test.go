package logx

import "testing"

func resetRingForTest() {
	ring.Lock()
	defer ring.Unlock()
	ring.next = 1
	ring.records = nil
}

func TestSnapshotCursorAndLimit(t *testing.T) {
	resetRingForTest()
	Info("T0001", "first")
	Warn("T0002", "second %d", 2)
	Error("T0003", "third")

	got := Snapshot(1, 1)
	if len(got) != 1 || got[0].Seq != 3 || got[0].Message != "third" {
		t.Fatalf("unexpected limited snapshot: %#v", got)
	}
	got = Snapshot(2, 20)
	if len(got) != 1 || got[0].Code != "T0003" {
		t.Fatalf("unexpected cursor snapshot: %#v", got)
	}
}

func TestRingKeepsNewestRecords(t *testing.T) {
	resetRingForTest()
	for i := 0; i < ringCapacity+5; i++ {
		Debug("T0010", "entry %d", i)
	}
	got := Snapshot(0, 500)
	if len(got) != 500 {
		t.Fatalf("got %d records, want 500", len(got))
	}
	if got[len(got)-1].Seq != ringCapacity+5 {
		t.Fatalf("last sequence = %d", got[len(got)-1].Seq)
	}
}

func TestSnapshotRedactsCredentialShapedValues(t *testing.T) {
	resetRingForTest()
	Error("T0020", `core rejected {"private_key":"super-secret","password":"hunter2"} token=abc123`)
	got := Snapshot(0, 20)
	if len(got) != 1 {
		t.Fatalf("got %d records", len(got))
	}
	if got[0].Message != `core rejected {"private_key":"[redacted]","password":"[redacted]"} token=[redacted]` {
		t.Fatalf("unexpected sanitized message: %q", got[0].Message)
	}
}
