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

// IsStaleUpstreamError returns true if the error message indicates that the
// configured upstream ref no longer exists on the remote.
func IsStaleUpstreamError(msg string) bool {
	return strings.Contains(msg, "no such ref was fetched")
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

// IsDirty reports whether the working tree at localPath has uncommitted
// changes: staged, unstaged, or untracked files. This matches the set of
// changes that `git stash -u` would capture, so it flags repos whose
// uncommitted work an update would displace.
func IsDirty(localPath string) (bool, error) {
	cmd := exec.Command("git", "status", "--porcelain")
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return false, fmt.Errorf("git status failed: %w", err)
	}
	return len(strings.TrimSpace(string(output))) > 0, nil
}

// GetDefaultBranch returns the default branch of the remote (e.g. "main").
func GetDefaultBranch(localPath, remoteName string) (string, error) {
	ref := fmt.Sprintf("refs/remotes/%s/HEAD", remoteName)
	cmd := exec.Command("git", "symbolic-ref", "--short", ref)
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("git symbolic-ref failed: %w", err)
	}
	short := strings.TrimSpace(string(output))
	prefix := remoteName + "/"
	if !strings.HasPrefix(short, prefix) {
		return "", fmt.Errorf("unexpected default ref %q", short)
	}
	return strings.TrimPrefix(short, prefix), nil
}

// IsBranchMerged reports whether branch has been merged into base.
func IsBranchMerged(localPath, branch, base string) (bool, error) {
	cmd := exec.Command("git", "branch", "--merged", base, "--format=%(refname:short)")
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return false, fmt.Errorf("git branch --merged failed: %w", err)
	}
	for _, line := range strings.Split(string(output), "\n") {
		if strings.TrimSpace(line) == branch {
			return true, nil
		}
	}
	return false, nil
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

// ResolveStaleUpstream handles the case where the local branch tracks a
// remote ref that no longer exists. It checks out the default branch,
// pulls, and deletes the stale branch if it has been merged.
func ResolveStaleUpstream(localPath, remoteName, staleBranch string) ActionResult {
	desc := filepath.Base(filepath.Dir(localPath)) + "/" + filepath.Base(localPath)

	defaultBranch, err := GetDefaultBranch(localPath, remoteName)
	if err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("could not determine default branch: %v", err),
		}
	}

	if staleBranch == defaultBranch {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("stale upstream on default branch %q — refusing to delete", defaultBranch),
		}
	}

	checkoutCmd := exec.Command("git", "checkout", defaultBranch)
	checkoutCmd.Dir = localPath
	if output, err := checkoutCmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git checkout %s failed: %s", defaultBranch, strings.TrimSpace(string(output))),
		}
	}

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

	merged, err := IsBranchMerged(localPath, staleBranch, defaultBranch)
	if err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("could not check merge status of %s: %v", staleBranch, err),
		}
	}

	if !merged {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     true,
			Message:     fmt.Sprintf("switched to %s and pulled (kept %s — not merged)", defaultBranch, staleBranch),
		}
	}

	deleteCmd := exec.Command("git", "branch", "-d", staleBranch)
	deleteCmd.Dir = localPath
	if output, err := deleteCmd.CombinedOutput(); err != nil {
		return ActionResult{
			Description: desc,
			Path:        localPath,
			Success:     false,
			Message:     fmt.Sprintf("git branch -d %s failed: %s", staleBranch, strings.TrimSpace(string(output))),
		}
	}

	return ActionResult{
		Description: desc,
		Path:        localPath,
		Success:     true,
		Message:     fmt.Sprintf("switched to %s, pulled, deleted merged branch %s", defaultBranch, staleBranch),
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
func ExecuteActions(ctx context.Context, actions []syncplan.Action, dryRun bool, confirmFn ConfirmFunc, concurrency int, configuredGitHubToken string) <-chan ActionResult {
	out := make(chan ActionResult, len(actions))
	githubToken, githubTokenErr := githubCloneToken(actions, configuredGitHubToken)

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
					result = executeAction(a, confirmFn, githubToken, githubTokenErr)
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

func executeAction(action syncplan.Action, confirmFn ConfirmFunc, githubToken string, githubTokenErr error) ActionResult {
	switch action.Type {
	case syncplan.ActionUpdate:
		return executeUpdate(action.Repo, action.LocalPath)
	case syncplan.ActionMove:
		return executeMove(action.Repo, action.CurrentPath, action.ExpectedPath, confirmFn)
	case syncplan.ActionClone:
		if action.Repo.Source == remote.SourceGitHub && githubTokenErr != nil {
			return ActionResult{
				Description: fmt.Sprintf("%s/%s (%s)", action.Repo.Owner, action.Repo.Name, action.Repo.SourceLabel()),
				Path:        action.ExpectedPath,
				Success:     false,
				Message:     fmt.Sprintf("resolving GitHub token: %v", githubTokenErr),
			}
		}
		return executeClone(action.Repo, action.ExpectedPath, githubToken)
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

func executeClone(repo *remote.RemoteRepo, expectedPath, githubToken string) ActionResult {
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

	args := []string{"clone", repo.CloneURL, expectedPath}
	var env []string
	var removeAskpass func()
	if repo.Source == remote.SourceGitHub {
		askpassPath, cleanup, err := githubAskpass(githubToken)
		if err != nil {
			return ActionResult{
				Description: desc,
				Path:        expectedPath,
				Success:     false,
				Message:     fmt.Sprintf("creating GitHub authentication helper: %v", err),
			}
		}
		removeAskpass = cleanup
		defer removeAskpass()
		env = append(os.Environ(),
			"GIT_ASKPASS="+askpassPath,
			"GIT_TERMINAL_PROMPT=0",
			"GITJUGGLING_GITHUB_TOKEN="+githubToken,
		)
		args = append([]string{"-c", "credential.helper="}, args...)
	}
	cmd := exec.Command("git", args...)
	if env != nil {
		cmd.Env = env
	}
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

// githubCloneToken resolves a token only when a GitHub clone will run. This
// preserves the existing `gh auth token` fallback while avoiding a needless
// credential lookup for Forgejo-only plans.
func githubCloneToken(actions []syncplan.Action, configuredToken string) (string, error) {
	for _, action := range actions {
		if action.Type == syncplan.ActionClone && action.Repo.Source == remote.SourceGitHub {
			return remote.GitHubToken(configuredToken)
		}
	}
	return "", nil
}

// githubAskpass returns a short-lived helper that gives Git the conventional
// username/password pair for GitHub HTTPS token authentication. The token is
// kept out of command arguments and clone URLs.
func githubAskpass(token string) (string, func(), error) {
	helper, err := os.CreateTemp("", "gitjuggling-github-askpass-*")
	if err != nil {
		return "", nil, err
	}
	path := helper.Name()
	cleanup := func() { _ = os.Remove(path) }

	const script = `#!/bin/sh
case "$1" in
  *Username*|*username*) printf '%s\n' x-access-token ;;
  *Password*|*password*) printf '%s\n' "$GITJUGGLING_GITHUB_TOKEN" ;;
  *) exit 1 ;;
esac
`
	if _, err := helper.WriteString(script); err != nil {
		helper.Close()
		cleanup()
		return "", nil, err
	}
	if err := helper.Close(); err != nil {
		cleanup()
		return "", nil, err
	}
	if err := os.Chmod(path, 0o700); err != nil {
		cleanup()
		return "", nil, err
	}
	return path, cleanup, nil
}
