// Package git wraps the git command-line calls shared across packages.
package git

import (
	"fmt"
	"os/exec"
	"strings"
)

// CurrentBranch returns the current branch name for a git repo.
func CurrentBranch(localPath string) (string, error) {
	cmd := exec.Command("git", "symbolic-ref", "--quiet", "HEAD")
	cmd.Dir = localPath
	output, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("reading checked-out branch: %w", err)
	}
	branch, ok := strings.CutPrefix(strings.TrimSpace(string(output)), "refs/heads/")
	if !ok {
		return "", fmt.Errorf("HEAD does not point to a local branch")
	}
	return branch, nil
}

// RemoteURL returns the fetch URL that Git resolves for a configured remote.
func RemoteURL(localPath, remoteName string) (string, error) {
	cmd := exec.Command("git", "remote", "get-url", "--", remoteName)
	cmd.Dir = localPath
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("reading URL of remote %s: %w", remoteName, err)
	}
	return strings.TrimSuffix(string(out), "\n"), nil
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
func (auth GitHubAuth) RemoteBranchExists(localPath, remoteName, branch string) (bool, error) {
	status, err := auth.LookupRemoteBranch(localPath, remoteName, branch)
	return status.Exists, err
}

// RemoteBranchStatus records the advertised branch and the tip of the
// remote's default branch from the same ls-remote response.
type RemoteBranchStatus struct {
	Exists     bool
	DefaultTip string
}

// LookupRemoteBranch checks the exact upstream ref and the remote's current HEAD.
func (auth GitHubAuth) LookupRemoteBranch(localPath, remoteName, branch string) (RemoteBranchStatus, error) {
	target := "refs/heads/" + branch
	cmd := auth.RemoteCommand(localPath, remoteName, "ls-remote", "--", remoteName, target, "HEAD")
	output, err := cmd.Output()
	if err != nil {
		return RemoteBranchStatus{}, fmt.Errorf("git ls-remote failed: %w", err)
	}
	var status RemoteBranchStatus
	for line := range strings.SplitSeq(string(output), "\n") {
		oid, ref, _ := strings.Cut(line, "\t")
		switch ref {
		case target:
			status.Exists = true
		case "HEAD":
			status.DefaultTip = oid
		}
	}
	return status, nil
}
