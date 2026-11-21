// Package gittest builds small real git repositories for tests.
// It requires the git binary.
package gittest

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
)

// Run executes Git with a test identity and returns its trimmed output.
func Run(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(),
		"GIT_AUTHOR_NAME=test", "GIT_AUTHOR_EMAIL=test@example.com",
		"GIT_COMMITTER_NAME=test", "GIT_COMMITTER_EMAIL=test@example.com",
	)
	out, err := cmd.CombinedOutput()
	require.NoError(t, err, "git %v: %s", args, out)
	return strings.TrimSpace(string(out))
}

// Repo creates a git repo at path with a single commit on master and origin
// pointing at originURL. refs/remotes/origin/HEAD is set to origin/master so
// default-branch detection works without a network.
func Repo(t *testing.T, path, originURL string) {
	t.Helper()
	require.NoError(t, os.MkdirAll(path, 0o755))
	Run(t, path, "init", "-b", "master", ".")
	Run(t, path, "remote", "add", "origin", originURL)
	require.NoError(t, os.WriteFile(filepath.Join(path, "README.md"), []byte("test"), 0o644))
	Run(t, path, "add", ".")
	Run(t, path, "commit", "-m", "init")
	Run(t, path, "update-ref", "refs/remotes/origin/master", "HEAD")
	Run(t, path, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/master")
}

// Worktree creates a linked worktree with a new branch at path of the repo
// at repoPath. When diverged is true, an extra commit is added on the
// worktree branch so it is no longer an ancestor of master (as with
// squash-merged or genuinely unmerged branches).
func Worktree(t *testing.T, repoPath, path, branch string, diverged bool) {
	t.Helper()
	Run(t, repoPath, "worktree", "add", "-b", branch, path, "HEAD")

	if diverged {
		require.NoError(t, os.WriteFile(filepath.Join(path, "wip.txt"), []byte("wip"), 0o644))
		Run(t, path, "add", ".")
		Run(t, path, "commit", "-m", "wip")
	}
}

// BareRemote creates a local remote for tests that exercise ls-remote or push.
func BareRemote(t *testing.T) string {
	t.Helper()
	path := t.TempDir()
	Run(t, path, "init", "--bare", "-b", "master")
	return path
}
