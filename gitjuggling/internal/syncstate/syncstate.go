// Package syncstate persists unfinished sync plans between gitjuggling runs.
package syncstate

import (
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
)

// Plan is the durable portion of a sync operation. Actions are removed as
// they succeed, which makes a later resume retry only unfinished work.
type Plan struct {
	Actions []syncplan.Action `json:"actions"`
}

type workspaceID struct {
	Name   string `json:"name"`
	Config string `json:"config"`
	Root   string `json:"root"`
}

type savedPlan struct {
	Workspace workspaceID `json:"workspace"`
	Plan
}

// Store owns the plan for one named workspace in one config and root.
type Store struct {
	workspace workspaceID
	path      string
}

func NewStore(plansDir, name, configPath, root string) (Store, error) {
	configPath, err := canonicalPath(configPath)
	if err != nil {
		return Store{}, fmt.Errorf("resolving config path: %w", err)
	}
	root, err = canonicalPath(root)
	if err != nil {
		return Store{}, fmt.Errorf("resolving workspace root: %w", err)
	}
	id := workspaceID{Name: name, Config: configPath, Root: root}
	// Old name-only plans carry no evidence of which root created them.
	key := sha256.Sum256([]byte(name + "\x00" + configPath + "\x00" + root))
	return Store{workspace: id, path: filepath.Join(plansDir, fmt.Sprintf("%x.json", key))}, nil
}

func canonicalPath(path string) (string, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	resolved, err := filepath.EvalSymlinks(abs)
	if errors.Is(err, os.ErrNotExist) {
		return abs, nil
	}
	return resolved, err
}

func (s Store) WorkspaceName() string { return s.workspace.Name }

// Load returns the pending actions, or nil when this workspace has no plan.
func (s Store) Load() (*Plan, error) {
	data, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("reading saved sync plan: %w", err)
	}

	var plan savedPlan
	if err := json.Unmarshal(data, &plan); err != nil {
		return nil, fmt.Errorf("parsing saved sync plan: %w", err)
	}
	if plan.Workspace != s.workspace {
		return nil, fmt.Errorf("saved sync plan %s does not match the selected workspace", s.path)
	}
	if len(plan.Actions) == 0 {
		return nil, nil
	}
	// Older planners could save CWD-relative paths or give two repositories
	// the same destination. Let sync build a new plan for those checkouts.
	destinations := make(map[string]bool)
	for _, a := range plan.Actions {
		for _, path := range []string{a.LocalPath, a.CurrentPath, a.ExpectedPath, a.MainRepoPath, a.WorktreePath} {
			if path != "" && !filepath.IsAbs(path) {
				return nil, nil
			}
		}
		if a.ExpectedPath != "" {
			path := filepath.Clean(a.ExpectedPath)
			if destinations[path] {
				return nil, nil
			}
			destinations[path] = true
		}
	}
	return &plan.Plan, nil
}

// Save atomically replaces the pending actions for this workspace.
func (s Store) Save(plan Plan) error {
	dir := filepath.Dir(s.path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("creating sync plan directory: %w", err)
	}
	data, err := json.MarshalIndent(savedPlan{Workspace: s.workspace, Plan: plan}, "", "  ")
	if err != nil {
		return fmt.Errorf("encoding sync plan: %w", err)
	}
	tmp, err := os.CreateTemp(dir, ".sync-plan-*")
	if err != nil {
		return fmt.Errorf("creating temporary sync plan: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return fmt.Errorf("writing sync plan: %w", err)
	}
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return fmt.Errorf("setting sync plan permissions: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("closing sync plan: %w", err)
	}
	if err := os.Rename(tmpName, s.path); err != nil {
		return fmt.Errorf("saving sync plan: %w", err)
	}
	return nil
}

// Delete removes this workspace's saved plan.
func (s Store) Delete() error {
	if err := os.Remove(s.path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("removing saved sync plan: %w", err)
	}
	return nil
}
