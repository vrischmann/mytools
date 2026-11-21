package execute

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"github.com/stretchr/testify/require"
)

func TestIsNoTrackingError(t *testing.T) {
	tests := []struct {
		name string
		msg  string
		want bool
	}{
		{
			name: "git pull no tracking information",
			msg:  "git pull --rebase failed: There is no tracking information for the current branch.\nPlease specify which branch you want to rebase against.",
			want: true,
		},
		{
			name: "lowercase variant",
			msg:  "git pull --rebase failed: fatal: no tracking information for the current branch",
			want: true,
		},
		{
			name: "unrelated git error",
			msg:  "git pull --rebase failed: error: could not apply abc1234..def5678",
			want: false,
		},
		{
			name: "stash error",
			msg:  "git stash failed: error: could not stash",
			want: false,
		},
		{
			name: "empty message",
			msg:  "",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsNoTrackingError(tt.msg)
			if got != tt.want {
				t.Errorf("IsNoTrackingError(%q) = %v, want %v", tt.msg, got, tt.want)
			}
		})
	}
}

func TestIsDirty(t *testing.T) {
	dir := t.TempDir()
	git := func(args ...string) {
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		require.NoError(t, cmd.Run())
	}
	writeFile := func(name, content string) {
		require.NoError(t, os.WriteFile(filepath.Join(dir, name), []byte(content), 0o600))
	}

	git("init")
	git("config", "user.email", "test@example.com")
	git("config", "user.name", "test")

	// Fresh empty repo: not dirty.
	clean, err := IsDirty(dir)
	require.NoError(t, err)
	require.False(t, clean)

	// Untracked file: dirty.
	writeFile("untracked.txt", "hello")
	dirty, err := IsDirty(dir)
	require.NoError(t, err)
	require.True(t, dirty)

	// Once committed: clean again.
	git("add", "untracked.txt")
	git("commit", "-m", "initial")
	clean, err = IsDirty(dir)
	require.NoError(t, err)
	require.False(t, clean)

	// Modifying a tracked file: dirty.
	writeFile("untracked.txt", "changed")
	dirty, err = IsDirty(dir)
	require.NoError(t, err)
	require.True(t, dirty)
}

func TestRemoveWorktreeAndBranchMergedDeletesBranch(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "demo")
	wtPath := filepath.Join(root, "demo-wt")

	action := planStaleWorktree(t, mainPath, wtPath, false)
	result := RemoveWorktreeAndBranch(action)
	require.True(t, result.Success, "result: %s", result.Message)
	require.NoDirExists(t, wtPath)

	out, err := exec.Command("git", "-C", mainPath, "branch", "--list", "feature").Output()
	require.NoError(t, err)
	require.Empty(t, string(out), "merged branch should be deleted")
}

func TestRemoveWorktreeAndBranchKeepsUnmergedBranch(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "demo")
	wtPath := filepath.Join(root, "demo-wt")

	action := planStaleWorktree(t, mainPath, wtPath, true)
	result := RemoveWorktreeAndBranch(action)
	require.True(t, result.Success, "result: %s", result.Message)
	require.Contains(t, result.Message, "kept branch feature")
	require.NoDirExists(t, wtPath)

	out, err := exec.Command("git", "-C", mainPath, "branch", "--list", "feature").Output()
	require.NoError(t, err)
	require.Contains(t, string(out), "feature", "unmerged branch must be kept")
}

func TestRemoveWorktreeAndBranchWithoutDelete(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "demo")
	wtPath := filepath.Join(root, "demo-wt")

	action := planStaleWorktree(t, mainPath, wtPath, false)
	action.Merged = false
	result := RemoveWorktreeAndBranch(action)
	require.True(t, result.Success, "result: %s", result.Message)
	require.NoDirExists(t, wtPath)

	out, err := exec.Command("git", "-C", mainPath, "branch", "--list", "feature").Output()
	require.NoError(t, err)
	require.Contains(t, string(out), "feature", "branch should be kept when deleteBranch is false")
}

func planStaleWorktree(t *testing.T, mainPath, wtPath string, diverged bool) syncplan.Action {
	t.Helper()
	origin := gittest.BareRemote(t)
	gittest.Repo(t, mainPath, origin)
	gittest.Run(t, mainPath, "push", "-u", "origin", "master")
	gittest.Worktree(t, mainPath, wtPath, "feature", diverged)
	gittest.Run(t, wtPath, "push", "-u", "origin", "HEAD:refs/heads/published")
	gittest.Run(t, origin, "update-ref", "-d", "refs/heads/published")
	gittest.Run(t, mainPath, "fetch", "--prune", "origin")
	repo := &remote.RemoteRepo{Name: filepath.Base(mainPath), CloneURL: origin, Source: remote.SourceForgejo}
	root := filepath.Dir(mainPath)
	local, err := discover.Discover(root)
	require.NoError(t, err)
	ws := &config.Workspace{Root: root, Rules: config.Rules{Base: root}}
	actions := syncplan.BuildPlan([]*remote.RemoteRepo{repo}, local, ws, git.RemoteBranchExists)
	require.Len(t, actions, 2)
	require.Equal(t, syncplan.ActionPruneWorktree, actions[1].Type)
	return actions[1]
}

func TestResumedPruneRevalidatesCheckout(t *testing.T) {
	tests := []struct {
		name   string
		change func(*testing.T, *syncplan.Action)
		want   string
	}{
		{"different branch", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.WorktreePath, "switch", "-c", "active")
			gittest.Run(t, a.WorktreePath, "push", "-u", "origin", "active")
		}, "changed"},
		{"new commit", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.WorktreePath, "commit", "--allow-empty", "-m", "new work")
		}, "changed"},
		{"changed tracking branch", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.WorktreePath, "config", "branch.feature.merge", "refs/heads/other")
		}, "changed"},
		{"removed tracking", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.WorktreePath, "branch", "--unset-upstream")
		}, "no remote upstream"},
		{"recreated upstream", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.WorktreePath, "push", "origin", "HEAD:refs/heads/published")
		}, "exists again"},
		{"unreachable upstream", func(t *testing.T, a *syncplan.Action) {
			require.NoError(t, os.Rename(a.Repo.CloneURL, filepath.Join(t.TempDir(), "offline")))
		}, "could not check upstream"},
		{"different checkout at path", func(t *testing.T, a *syncplan.Action) {
			gittest.Run(t, a.MainRepoPath, "worktree", "move", a.WorktreePath, a.WorktreePath+"-moved")
			gittest.Repo(t, a.WorktreePath, a.Repo.CloneURL)
		}, "could not verify"},
		{"dirty worktree", func(t *testing.T, a *syncplan.Action) {
			require.NoError(t, os.WriteFile(filepath.Join(a.WorktreePath, "work.txt"), []byte("unfinished"), 0o600))
		}, "git worktree remove failed"},
		{"legacy saved plan", func(_ *testing.T, a *syncplan.Action) {
			a.WorktreeState = git.WorktreeState{Branch: a.Branch}
		}, "rebuild"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			action := planStaleWorktree(t, filepath.Join(root, "demo"), filepath.Join(root, "checkout"), false)
			tc.change(t, &action)
			saved, err := json.Marshal(action)
			require.NoError(t, err)
			var resumed syncplan.Action
			require.NoError(t, json.Unmarshal(saved, &resumed))
			result := <-ExecuteActions(t.Context(), []syncplan.Action{resumed}, false, nil, 2, "")
			require.Contains(t, result.Message, tc.want)
			require.DirExists(t, action.WorktreePath)
			require.Equal(t, "feature", gittest.Run(t, action.MainRepoPath, "for-each-ref", "--format=%(refname:short)", "refs/heads/feature"))
		})
	}
}

func TestPruneRechecksMergeBeforeDeletingBranch(t *testing.T) {
	root := t.TempDir()
	action := planStaleWorktree(t, filepath.Join(root, "demo"), filepath.Join(root, "checkout"), false)
	require.True(t, action.Merged)
	// Replace the default branch on the bare remote without updating the
	// local tracking ref, which still contains the worktree's commit.
	replacement := gittest.Run(t, action.MainRepoPath, "commit-tree", "HEAD^{tree}", "-m", "new root")
	gittest.Run(t, action.MainRepoPath, "push", "--force", action.Repo.CloneURL, replacement+":refs/heads/master")
	result := RemoveWorktreeAndBranch(action)
	require.True(t, result.Success, result.Message)
	require.NoDirExists(t, action.WorktreePath)
	require.Equal(t, action.Commit, gittest.Run(t, action.MainRepoPath, "rev-parse", "refs/heads/feature"))
}

func TestResumedPruneKeepsBranchAfterInterruptedRemoval(t *testing.T) {
	root := t.TempDir()
	action := planStaleWorktree(t, filepath.Join(root, "demo"), filepath.Join(root, "checkout"), false)
	gittest.Run(t, action.MainRepoPath, "worktree", "remove", action.WorktreePath)
	result := RemoveWorktreeAndBranch(action)
	require.True(t, result.Success, result.Message)
	require.Equal(t, action.Commit, gittest.Run(t, action.MainRepoPath, "rev-parse", "refs/heads/feature"))
}
