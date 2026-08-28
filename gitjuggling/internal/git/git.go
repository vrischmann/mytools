// Package git wraps the git command-line calls shared across packages.
package git

import (
	"fmt"
	"os/exec"
	"strings"
)

// CurrentBranch returns the current branch name for a git repo.
func CurrentBranch(localPath string) (string, error) {
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

// DefaultBranch returns the default branch of the remote (e.g. "main").
func DefaultBranch(localPath, remoteName string) (string, error) {
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
// This shells out to ls-remote, so it performs a network round trip.
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
