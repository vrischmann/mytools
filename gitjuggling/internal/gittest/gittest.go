// Package gittest builds small real git repositories for tests.
// It requires the git binary.
package gittest

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
)

func run(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(),
		"GIT_AUTHOR_NAME=test", "GIT_AUTHOR_EMAIL=test@example.com",
		"GIT_COMMITTER_NAME=test", "GIT_COMMITTER_EMAIL=test@example.com",
	)
	out, err := cmd.CombinedOutput()
	require.NoError(t, err, "git %v: %s", args, out)
}

// Repo creates a git repo at path with a single commit on master and origin
// pointing at originURL. refs/remotes/origin/HEAD is set to origin/master so
// default-branch detection works without a network.
func Repo(t *testing.T, path, originURL string) {
	t.Helper()
	require.NoError(t, os.MkdirAll(path, 0o755))
	run(t, path, "init", "-b", "master", ".")
	run(t, path, "remote", "add", "origin", originURL)
	require.NoError(t, os.WriteFile(filepath.Join(path, "README.md"), []byte("test"), 0o644))
	run(t, path, "add", ".")
	run(t, path, "commit", "-m", "init")
	run(t, path, "update-ref", "refs/remotes/origin/master", "HEAD")
	run(t, path, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/master")
}

// Worktree creates a linked worktree with a new branch at path of the repo
// at repoPath. When diverged is true, an extra commit is added on the
// worktree branch so it is no longer an ancestor of master (as with
// squash-merged or genuinely unmerged branches).
func Worktree(t *testing.T, repoPath, path, branch string, diverged bool) {
	t.Helper()
	run(t, repoPath, "worktree", "add", "-b", branch, path, "HEAD")

	if diverged {
		require.NoError(t, os.WriteFile(filepath.Join(path, "wip.txt"), []byte("wip"), 0o644))
		run(t, path, "add", ".")
		run(t, path, "commit", "-m", "wip")
	}
}
