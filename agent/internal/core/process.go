package core

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/akiko99x/honey/agent/internal/logx"
)

const stopGrace = 10 * time.Second

// Process is the generic lifecycle for a core binary: write config, validate,
// run, graceful stop, crash detection. sing-box and xray differ only in their
// run/check args, passed in here.
type Process struct {
	name       string
	binPath    string
	configPath string
	runArgs    func(cfg string) []string // e.g. {"run","-c",cfg}
	checkArgs  func(cfg string) []string // e.g. {"check","-c",cfg}; nil = skip

	mu       sync.Mutex
	cmd      *exec.Cmd
	waitCh   chan struct{}
	stopping bool
	state    State
	pid      int
	lastErr  string
}

func NewProcess(name, binPath, configPath string, runArgs, checkArgs func(string) []string) *Process {
	return &Process{
		name:       name,
		binPath:    binPath,
		configPath: configPath,
		runArgs:    runArgs,
		checkArgs:  checkArgs,
		state:      StateStopped,
	}
}

// Start writes the config, validates it, then launches the core.
func (p *Process) Start(configJSON string) error {
	p.mu.Lock()
	defer p.mu.Unlock()

	if p.state == StateRunning {
		return nil
	}
	logx.Info(logx.CoreStarting, "%s will try to start up...", p.name)
	if err := p.writeConfig(configJSON); err != nil {
		logx.Error(logx.CoreStartFailed, "%s refused to start: %v", p.name, err)
		return p.failLocked(err)
	}
	if p.checkArgs != nil {
		if err := p.check(); err != nil {
			return p.failLocked(err)
		}
	}

	cmd := exec.Command(p.binPath, p.runArgs(p.configPath)...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		logx.Error(logx.CoreStartFailed, "%s refused to start: %v", p.name, err)
		return p.failLocked(fmt.Errorf("start %s: %w", p.name, err))
	}

	p.cmd = cmd
	p.pid = cmd.Process.Pid
	p.state = StateRunning
	p.stopping = false
	p.lastErr = ""
	logx.Info(logx.CoreStarted, "%s is running, pid=%d", p.name, p.pid)

	waitCh := make(chan struct{})
	p.waitCh = waitCh
	go p.reap(cmd, waitCh)
	return nil
}

func (p *Process) check() error {
	return p.checkPath(p.configPath)
}

func (p *Process) checkPath(configPath string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, p.binPath, p.checkArgs(configPath)...).CombinedOutput()
	if err != nil {
		logx.Error(logx.CoreConfigInvalid, "%s says the config is bad: %v", p.name, err)
		return fmt.Errorf("%s config check: %w: %s", p.name, err, strings.TrimSpace(string(out)))
	}
	return nil
}

// candidatePattern keeps the managed config's extension because some cores
// (notably Xray) use it to detect the configuration format.
func (p *Process) candidatePattern(stage string) string {
	ext := filepath.Ext(p.configPath)
	if ext == "" {
		ext = ".json"
	}
	return ".config-*." + stage + ext
}

// Validate writes a disposable candidate next to the live config and asks the
// core binary to check it. The running process and its config are untouched.
func (p *Process) Validate(configJSON string) error {
	if configJSON == "" || p.checkArgs == nil {
		return nil
	}
	dir := filepath.Dir(p.configPath)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	candidate, err := os.CreateTemp(dir, p.candidatePattern("validate"))
	if err != nil {
		return fmt.Errorf("create candidate %s config: %w", p.name, err)
	}
	path := candidate.Name()
	defer os.Remove(path)
	if err := candidate.Chmod(0o600); err != nil {
		candidate.Close()
		return err
	}
	if _, err := candidate.WriteString(configJSON); err != nil {
		candidate.Close()
		return err
	}
	if err := candidate.Close(); err != nil {
		return err
	}
	return p.checkPath(path)
}

func (p *Process) reap(cmd *exec.Cmd, waitCh chan struct{}) {
	err := cmd.Wait()

	p.mu.Lock()
	if p.cmd == cmd {
		switch {
		case p.stopping:
			p.state = StateStopped
			logx.Info(logx.CoreStopped, "%s stopped", p.name)
		case err != nil:
			p.state = StateErrored
			p.lastErr = err.Error()
			logx.Warn(logx.CoreCrashed, "%s died on its own: %v", p.name, err)
		default:
			p.state = StateStopped
			logx.Info(logx.CoreStopped, "%s stopped", p.name)
		}
		p.cmd = nil
		p.pid = 0
	}
	p.mu.Unlock()
	close(waitCh)
}

// Stop terminates the core gracefully (SIGTERM), then hard-kills after a grace.
func (p *Process) Stop() error {
	p.mu.Lock()
	if p.cmd == nil || p.cmd.Process == nil {
		p.state = StateStopped
		p.mu.Unlock()
		return nil
	}
	proc := p.cmd.Process
	waitCh := p.waitCh
	p.stopping = true
	p.mu.Unlock()

	logx.Info(logx.CoreStopping, "stopping %s nicely...", p.name)
	_ = proc.Signal(syscall.SIGTERM)
	select {
	case <-waitCh:
	case <-time.After(stopGrace):
		logx.Warn(logx.CoreKilled, "%s ignored sigterm, killed it", p.name)
		_ = proc.Kill()
		<-waitCh
	}
	return nil
}

// Apply validates a candidate config before stopping the healthy process, then
// swaps it in and restarts. If the new process cannot start, restore the old
// config and make a best-effort attempt to bring the previous process back.
func (p *Process) Apply(configJSON string) error {
	logx.Info(logx.CoreApplying, "swapping %s config, hold on...", p.name)
	if configJSON == "" {
		if p.checkArgs != nil {
			if err := p.check(); err != nil {
				return err
			}
		}
		if err := p.Stop(); err != nil {
			return err
		}
		return p.Start("")
	}

	dir := filepath.Dir(p.configPath)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	candidate, err := os.CreateTemp(dir, p.candidatePattern("next"))
	if err != nil {
		return fmt.Errorf("create candidate %s config: %w", p.name, err)
	}
	candidatePath := candidate.Name()
	defer os.Remove(candidatePath)
	if err := candidate.Chmod(0o600); err != nil {
		candidate.Close()
		return err
	}
	if _, err := candidate.WriteString(configJSON); err != nil {
		candidate.Close()
		return err
	}
	if err := candidate.Close(); err != nil {
		return err
	}
	if p.checkArgs != nil {
		if err := p.checkPath(candidatePath); err != nil {
			return err
		}
	}

	oldConfig, readErr := os.ReadFile(p.configPath)
	hadOldConfig := readErr == nil
	if readErr != nil && !os.IsNotExist(readErr) {
		return fmt.Errorf("read current %s config: %w", p.name, readErr)
	}
	if err := p.Stop(); err != nil {
		return err
	}
	if err := os.Rename(candidatePath, p.configPath); err != nil {
		logx.Warn(logx.CoreInstallFailed, "couldn't put %s config in place: %v", p.name, err)
		_ = p.Start("")
		return fmt.Errorf("install candidate %s config: %w", p.name, err)
	}
	if err := p.Start(""); err == nil {
		return nil
	} else if !hadOldConfig {
		return err
	} else {
		applyErr := err
		if restoreErr := os.WriteFile(p.configPath, oldConfig, 0o600); restoreErr != nil {
			logx.Error(logx.CoreRollbackFailed, "%s apply failed AND restore failed: %v", p.name, restoreErr)
			return fmt.Errorf("apply new %s config: %v; restore config: %w", p.name, applyErr, restoreErr)
		}
		if restartErr := p.Start(""); restartErr != nil {
			logx.Error(logx.CoreRollbackFailed, "%s apply failed AND restart of previous config failed: %v", p.name, restartErr)
			return fmt.Errorf(
				"apply new %s config: %v; restart previous config: %w",
				p.name, applyErr, restartErr,
			)
		}
		logx.Warn(logx.CoreRolledBack, "%s apply failed, rolled back: %v", p.name, applyErr)
		return fmt.Errorf("apply new %s config: %w; previous config restored", p.name, applyErr)
	}
}

func (p *Process) Status() (State, int, string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.state, p.pid, p.lastErr
}

func (p *Process) writeConfig(configJSON string) error {
	if configJSON == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(p.configPath), 0o755); err != nil {
		return err
	}
	return os.WriteFile(p.configPath, []byte(configJSON), 0o600)
}

func (p *Process) failLocked(err error) error {
	p.state = StateErrored
	p.lastErr = err.Error()
	return err
}
