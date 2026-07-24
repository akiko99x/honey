package core

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCandidateConfigKeepsJSONExtension(t *testing.T) {
	p := NewProcess("xray", "xray", filepath.Join(t.TempDir(), "xray.json"), nil, nil)

	for _, stage := range []string{"validate", "next"} {
		candidate, err := os.CreateTemp(filepath.Dir(p.configPath), p.candidatePattern(stage))
		if err != nil {
			t.Fatalf("create %s candidate: %v", stage, err)
		}
		path := candidate.Name()
		candidate.Close()
		if got := filepath.Ext(path); got != ".json" {
			t.Fatalf("%s candidate extension = %q, want .json (path %q)", stage, got, path)
		}
	}
}

func TestApplyRejectsInvalidConfigWithoutStoppingRunningProcess(t *testing.T) {
	t.Setenv("GO_WANT_CORE_HELPER", "1")
	p := NewProcess(
		"test-core",
		os.Args[0],
		t.TempDir()+"/config.json",
		func(cfg string) []string { return helperArgs("run", cfg) },
		func(cfg string) []string { return helperArgs("check", cfg) },
	)

	// The validation failure must happen before Apply reaches Stop. A real
	// process is unnecessary here; StateRunning is the invariant under test.
	p.state = StateRunning
	if err := p.Apply("invalid"); err == nil {
		t.Fatal("Apply accepted an invalid candidate config")
	}
	state, _, _ := p.Status()
	if state != StateRunning {
		t.Fatalf("invalid candidate stopped the running process: state=%v", state)
	}
}

func helperArgs(mode, cfg string) []string {
	return []string{"-test.run=TestCoreHelperProcess", "--", mode, cfg}
}

func TestCoreHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_CORE_HELPER") != "1" {
		return
	}
	separator := -1
	for i, arg := range os.Args {
		if arg == "--" {
			separator = i
			break
		}
	}
	if separator < 0 || len(os.Args) <= separator+2 {
		os.Exit(2)
	}
	mode, configPath := os.Args[separator+1], os.Args[separator+2]
	if mode != "check" {
		os.Exit(2)
	}
	config, err := os.ReadFile(configPath)
	if err != nil {
		os.Exit(2)
	}
	if strings.Contains(string(config), "invalid") {
		os.Exit(1)
	}
	os.Exit(0)
}
