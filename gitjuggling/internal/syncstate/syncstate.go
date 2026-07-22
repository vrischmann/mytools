// Package syncstate persists unfinished sync plans between gitjuggling runs.
package syncstate

import (
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

// Load reads the saved plan for workspace. A missing plan is reported as
// (nil, nil).
func Load(workspace string) (*Plan, error) {
	path, err := pathFor(workspace)
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("reading saved sync plan: %w", err)
	}

	var plan Plan
	if err := json.Unmarshal(data, &plan); err != nil {
		return nil, fmt.Errorf("parsing saved sync plan: %w", err)
	}
	if len(plan.Actions) == 0 {
		return nil, nil
	}
	return &plan, nil
}

// Save atomically writes plan for workspace.
func Save(workspace string, plan Plan) error {
	path, err := pathFor(workspace)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("creating sync plan directory: %w", err)
	}
	data, err := json.MarshalIndent(plan, "", "  ")
	if err != nil {
		return fmt.Errorf("encoding sync plan: %w", err)
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".sync-plan-*")
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
	if err := os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("saving sync plan: %w", err)
	}
	return nil
}

// Delete removes the saved plan once every pending action has completed.
func Delete(workspace string) error {
	path, err := pathFor(workspace)
	if err != nil {
		return err
	}
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("removing saved sync plan: %w", err)
	}
	return nil
}

func pathFor(workspace string) (string, error) {
	configDir, err := os.UserConfigDir()
	if err != nil {
		return "", fmt.Errorf("determining config directory for saved sync plan: %w", err)
	}
	return filepath.Join(configDir, "gitjuggling", "plans", workspace+".json"), nil
}
