package execute

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
)

// ActionResult holds the outcome of executing a single action.
type ActionResult struct {
	Description string
	Path        string
	Success     bool
	Message     string
}

// IsNoTrackingError returns true if the error message indicates a missing
// upstream/tracking branch.
func IsNoTrackingError(msg string) bool {
	return strings.Contains(msg, "no tracking information") ||
		strings.Contains(msg, "There is no tracking information")
}

// GetCurrentBranch returns the current branch name for a git repo.
func GetCurrentBranch(localPath string) (string, error) {
	cmd := exec.Command("git", "rev-parse", "--abbrev-ref", "HEAD")
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("git rev-parse failed: %w", err)
	}
	branch := strings.TrimSpace(string(output))
	if branch == "HEAD" {
		return "", fmt.Errorf("repository is in detached HEAD state")
	}
	return branch, nil
}

// RemoteBranchExists checks whether a branch exists on the given remote.
func RemoteBranchExists(localPath, remoteName, branch string) (bool, error) {
	cmd := exec.Command("git", "ls-remote", "--heads", remoteName)
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return false, fmt.Errorf("git ls-remote failed: %w", err)
	}
	target := fmt.Sprintf("refs/heads/%s", branch)
	for _, line := range strings.Split(string(output), "\n") {
		if strings.HasSuffix(strings.TrimSpace(line), target) {
			return true, nil
		}
	}
	return false, nil
}

// PushAndSetUpstream pushes the current branch to the remote with -u,
// creating the remote branch and setting the upstream tracking branch.
func PushAndSetUpstream(localPath, remoteName, branch string) ActionResult {
	desc := filepath.Base(filepath.Dir(localPath)) + "/" + filepath.Base(localPath)

	cmd := exec.Command("git", "push", "-u", remoteName, branch)
	cmd.Dir = localPath
	if output, err := cmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git push -u failed: %s", strings.TrimSpace(string(output))),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        localPath,
		Success:     true,
		Message:     fmt.Sprintf("pushed (tracking set to %s/%s)", remoteName, branch),
	}
}

// SetUpstreamAndPull sets the upstream tracking branch and runs git pull --rebase.
func SetUpstreamAndPull(localPath, remoteName, branch string) ActionResult {
	desc := filepath.Base(filepath.Dir(localPath)) + "/" + filepath.Base(localPath)

	// git branch --set-upstream-to=origin/<branch> <branch>
	upstream := fmt.Sprintf("%s/%s", remoteName, branch)
	cmd := exec.Command("git", "branch", "--set-upstream-to", upstream, branch)
	cmd.Dir = localPath
	if output, err := cmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git branch --set-upstream-to failed: %s", strings.TrimSpace(string(output))),
		}
	}

	// git pull --rebase
	pullCmd := exec.Command("git", "pull", "--rebase")
	pullCmd.Dir = localPath
	if output, err := pullCmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git pull --rebase failed: %s", strings.TrimSpace(string(output))),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        localPath,
		Success:     true,
		Message:     "updated (tracking set to " + upstream + ")",
	}
}

// ConfirmFunc is a callback for interactive confirmation prompts.
// It should display the prompt and return true if the user confirms.
type ConfirmFunc func(prompt string) bool

// ExecuteActions runs a list of actions with bounded concurrency.
// Results are sent on the returned channel as they complete.
// The channel is closed when all actions are done.
func ExecuteActions(ctx context.Context, actions []syncplan.Action, dryRun bool, confirmFn ConfirmFunc, concurrency int) <-chan ActionResult {
	out := make(chan ActionResult, len(actions))

	if concurrency < 1 {
		concurrency = 1
	}

	go func() {
		defer close(out)

		if len(actions) == 0 {
			return
		}

		sem := make(chan struct{}, concurrency)

		for _, action := range actions {
			select {
			case <-ctx.Done():
				return
			case sem <- struct{}{}:
			}

			go func(a syncplan.Action) {
				defer func() { <-sem }()

				var result ActionResult
				if dryRun {
					result = dryRunAction(a)
				} else {
					result = executeAction(a, confirmFn)
				}
				out <- result
			}(action)
		}

		// Wait for all goroutines to finish by filling the semaphore
		for i := 0; i < cap(sem); i++ {
			sem <- struct{}{}
		}
	}()

	return out
}

// ---------------------------------------------------------------------------
// Action execution
// ---------------------------------------------------------------------------

func dryRunAction(action syncplan.Action) ActionResult {
	desc := fmt.Sprintf("%s/%s (%s)", action.Repo.Owner, action.Repo.Name, action.Repo.SourceLabel())

	switch action.Type {
	case syncplan.ActionUpdate:
		return ActionResult{
			Description: desc,
			Path:        action.LocalPath,
			Success:     true,
			Message:     "would update (stash + pull --rebase)",
		}
	case syncplan.ActionMove:
		return ActionResult{
			Description: desc,
			Path:        action.ExpectedPath,
			Success:     true,
			Message:     fmt.Sprintf("would move: %s → %s", action.CurrentPath, action.ExpectedPath),
		}
	case syncplan.ActionClone:
		return ActionResult{
			Description: desc,
			Path:        action.ExpectedPath,
			Success:     true,
			Message:     "would clone",
		}
	default:
		return ActionResult{Description: desc, Success: false, Message: "unknown action type"}
	}
}

func executeAction(action syncplan.Action, confirmFn ConfirmFunc) ActionResult {
	switch action.Type {
	case syncplan.ActionUpdate:
		return executeUpdate(action.Repo, action.LocalPath)
	case syncplan.ActionMove:
		return executeMove(action.Repo, action.CurrentPath, action.ExpectedPath, confirmFn)
	case syncplan.ActionClone:
		return executeClone(action.Repo, action.ExpectedPath)
	default:
		return ActionResult{
			Description: fmt.Sprintf("%s/%s (%s)", action.Repo.Owner, action.Repo.Name, action.Repo.SourceLabel()),
			Success:     false,
			Message:     "unknown action type",
		}
	}
}

func executeUpdate(repo *remote.RemoteRepo, localPath string) ActionResult {
	desc := fmt.Sprintf("%s/%s (%s)", repo.Owner, repo.Name, repo.SourceLabel())

	// git stash -u
	stashCmd := exec.Command("git", "stash", "-u")
	stashCmd.Dir = localPath
	if output, err := stashCmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git stash failed: %s", strings.TrimSpace(string(output))),
		}
	}

	// git pull --rebase
	pullCmd := exec.Command("git", "pull", "--rebase")
	pullCmd.Dir = localPath
	if output, err := pullCmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git pull --rebase failed: %s", strings.TrimSpace(string(output))),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        localPath,
		Success:     true,
		Message:     "updated",
	}
}

func executeMove(repo *remote.RemoteRepo, currentPath, expectedPath string, confirmFn ConfirmFunc) ActionResult {
	desc := fmt.Sprintf("%s/%s (%s)", repo.Owner, repo.Name, repo.SourceLabel())

	if confirmFn != nil {
		prompt := fmt.Sprintf("Move %s from %s to %s?", desc, currentPath, expectedPath)
		if !confirmFn(prompt) {
			return ActionResult{
				Description: desc,
				Path:        currentPath,
				Success:     true,
				Message:     "skipped (user declined)",
			}
		}
	}

	// Ensure parent directory exists
	if err := os.MkdirAll(filepath.Dir(expectedPath), 0o755); err != nil {
		return ActionResult{
			Description: desc,
			Path:        expectedPath,
			Success:     false,
			Message:     fmt.Sprintf("failed to create parent directory: %v", err),
		}
	}

	if err := os.Rename(currentPath, expectedPath); err != nil {
		return ActionResult{
			Description: desc,
			Path:        expectedPath,
			Success:     false,
			Message:     fmt.Sprintf("failed to move: %v", err),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        expectedPath,
		Success:     true,
		Message:     "moved",
	}
}

func executeClone(repo *remote.RemoteRepo, expectedPath string) ActionResult {
	desc := fmt.Sprintf("%s/%s (%s)", repo.Owner, repo.Name, repo.SourceLabel())

	// Ensure parent directory exists
	if err := os.MkdirAll(filepath.Dir(expectedPath), 0o755); err != nil {
		return ActionResult{
			Description: desc,
			Path:        expectedPath,
			Success:     false,
			Message:     fmt.Sprintf("failed to create parent directory: %v", err),
		}
	}

	cmd := exec.Command("git", "clone", repo.CloneURL, expectedPath)
	if output, err := cmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        expectedPath,
			Success:     false,
			Message:     fmt.Sprintf("git clone failed: %s", strings.TrimSpace(string(output))),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        expectedPath,
		Success:     true,
		Message:     "cloned",
	}
}
