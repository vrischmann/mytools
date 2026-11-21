package execute

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"sync"

	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
)

// ActionResult holds the outcome of executing a single action.
type ActionResult struct {
	Description string
	Path        string
	MovedFrom   string // previous path of a completed move
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
	return git.CurrentBranch(localPath)
}

// IsDirty reports whether the working tree at localPath has uncommitted
// changes.
func IsDirty(localPath string) (bool, error) {
	return git.IsDirty(localPath)
}

// GetDefaultBranch returns the default branch of the remote (e.g. "main").
func GetDefaultBranch(localPath, remoteName string) (string, error) {
	return git.DefaultBranch(localPath, remoteName)
}

// IsBranchMerged reports whether branch has been merged into base.
func IsBranchMerged(localPath, branch, base string) (bool, error) {
	return git.IsBranchMerged(localPath, branch, base)
}

// RemoteBranchExists checks whether a branch exists on the given remote.
func RemoteBranchExists(localPath, remoteName, branch string) (bool, error) {
	return git.RemoteBranchExists(localPath, remoteName, branch)
}

// RemoveWorktreeAndBranch checks that the planned checkout still has the
// same missing upstream before removing it. Git also refuses dirty or locked
// worktrees. A saved plan from before these checks needs to be rebuilt.
func RemoveWorktreeAndBranch(action syncplan.Action) ActionResult {
	path := action.WorktreePath
	result := ActionResult{Description: filepath.Base(filepath.Dir(path)) + "/" + filepath.Base(path), Path: path}
	if action.Repo == nil || action.WorktreeID == "" || action.Upstream.Remote == "" {
		result.Message = "rebuild the sync plan to record the worktree details"
		return result
	}
	if err := verifyMainClone(action.MainRepoPath, action.Repo.CloneURL); err != nil {
		result.Message = err.Error()
		return result
	}
	// If removal finished before the plan could be saved, retire the action.
	// An offline checkout still has a registration and must be left pending.
	if _, err := os.Lstat(path); errors.Is(err, os.ErrNotExist) {
		_, err = os.Lstat(filepath.Join(action.MainRepoPath, ".git", "worktrees", action.WorktreeID))
		if errors.Is(err, os.ErrNotExist) {
			result.Success = true
			result.Message = "worktree already removed"
			return result
		}
	}
	// Verify the checkout both before contacting its remote and after the
	// network round trip, during which the user may have switched branches.
	if err := verifyPlannedWorktree(action); err != nil {
		result.Message = err.Error()
		return result
	}
	status, err := git.LookupRemoteBranch(path, action.Upstream.Remote, action.Upstream.Branch)
	if err != nil {
		result.Message = fmt.Sprintf("could not check upstream %s/%s: %v", action.Upstream.Remote, action.Upstream.Branch, err)
		return result
	}
	if status.Exists {
		result.Success = true
		result.Message = fmt.Sprintf("skipped (upstream %s/%s exists again)", action.Upstream.Remote, action.Upstream.Branch)
		return result
	}
	if err := verifyPlannedWorktree(action); err != nil {
		result.Message = err.Error()
		return result
	}

	absPath, err := filepath.Abs(path)
	if err != nil {
		result.Message = fmt.Sprintf("resolving worktree path: %v", err)
		return result
	}
	removeCmd := exec.Command("git", "worktree", "remove", "--", absPath)
	removeCmd.Dir = action.MainRepoPath
	if output, err := removeCmd.CombinedOutput(); err != nil {
		result.Message = fmt.Sprintf("git worktree remove failed: %s", strings.TrimSpace(string(output)))
		return result
	}

	if !action.Merged || !git.IsAncestor(action.MainRepoPath, "refs/heads/"+action.Branch, status.DefaultTip) {
		result.Success = true
		result.Message = fmt.Sprintf("removed stale worktree (kept branch %s because its merge could not be confirmed)", action.Branch)
		return result
	}

	deleteCmd := exec.Command("git", "branch", "-d", "--", action.Branch)
	deleteCmd.Dir = action.MainRepoPath
	if output, err := deleteCmd.CombinedOutput(); err != nil {
		result.Message = fmt.Sprintf("removed worktree, but git branch -d %s failed: %s", action.Branch, strings.TrimSpace(string(output)))
		return result
	}

	result.Success = true
	result.Message = fmt.Sprintf("removed stale worktree and merged branch %s", action.Branch)
	return result
}

func verifyPlannedWorktree(action syncplan.Action) error {
	current, err := git.ReadWorktreeState(action.MainRepoPath, action.WorktreePath)
	if err != nil {
		return fmt.Errorf("could not verify the planned worktree: %w", err)
	}
	if current != action.WorktreeState {
		return errors.New("rebuild the sync plan because the worktree, branch, commit or upstream changed")
	}
	return nil
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
	pending := slices.Clone(actions)
	run := func(action syncplan.Action) ActionResult {
		if dryRun {
			return dryRunAction(action)
		}
		return executeAction(action, confirmFn, githubToken, githubTokenErr)
	}

	go func() {
		defer close(out)
		// A rename changes the paths of dependent actions. Finish the moves
		// and send their results before any worker starts using those paths.
		for _, action := range pending {
			if ctx.Err() != nil {
				return
			}
			if action.Type == syncplan.ActionMove {
				result := run(action)
				if dryRun || result.MovedFrom != "" {
					syncplan.Relocate(pending, action.CurrentPath, action.ExpectedPath)
				}
				out <- result
			}
		}
		var workers sync.WaitGroup
		defer workers.Wait()
		sem := make(chan struct{}, max(1, concurrency))
		for _, action := range pending {
			if action.Type == syncplan.ActionMove {
				continue
			}
			select {
			case <-ctx.Done():
				return
			case sem <- struct{}{}:
			}

			workers.Go(func() {
				defer func() { <-sem }()
				out <- run(action)
			})
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
	case syncplan.ActionPruneWorktree:
		message := fmt.Sprintf("would remove stale worktree and merged branch %s", action.Branch)
		if !action.Merged {
			message = fmt.Sprintf("would remove stale worktree (branch %s kept — not fully merged)", action.Branch)
		}
		return ActionResult{
			Description: desc,
			Path:        action.WorktreePath,
			Success:     true,
			Message:     message,
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
	case syncplan.ActionPruneWorktree:
		return RemoveWorktreeAndBranch(action)
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

	if err := moveClone(currentPath, expectedPath, repo.CloneURL); err != nil {
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
		MovedFrom:   currentPath,
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
