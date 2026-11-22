package execute

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"github.com/stretchr/testify/require"
)

func TestMoveRepairsLinkedWorktrees(t *testing.T) {
	for _, relative := range []bool{false, true} {
		for _, stage := range []string{"planned", "renamed", "partly repaired"} {
			t.Run("relative="+strconv.FormatBool(relative)+"/"+stage, func(t *testing.T) {
				root := t.TempDir()
				oldMain := filepath.Join(root, "workspace", "old")
				newMain := filepath.Join(root, "workspace", "personal", "repos", "new")
				external := filepath.Join(root, "outside workspace")
				nested := filepath.Join(oldMain, "linked", "inside")
				repo := &remote.RemoteRepo{Name: "new", CloneURL: gittest.BareRemote(t)}
				gittest.Repo(t, oldMain, repo.CloneURL)
				gittest.Run(t, oldMain, "config", "worktree.useRelativePaths", strconv.FormatBool(relative))
				gittest.Worktree(t, oldMain, external, "outside", true)
				gittest.Worktree(t, oldMain, nested, "inside", true)
				externalTip := gittest.Run(t, external, "rev-parse", "HEAD")
				nestedTip := gittest.Run(t, nested, "rev-parse", "HEAD")
				require.NoError(t, os.WriteFile(filepath.Join(external, "uncommitted"), []byte("draft"), 0o600))
				if stage != "planned" {
					require.NoError(t, os.MkdirAll(filepath.Dir(newMain), 0o755))
					require.NoError(t, os.Rename(oldMain, newMain))
				}
				if stage == "partly repaired" {
					gitfile := filepath.Join(external, ".git")
					oldLink, err := os.ReadFile(gitfile)
					require.NoError(t, err)
					gittest.Run(t, newMain, "worktree", "repair", "--", external, filepath.Join(newMain, "linked", "inside"))
					require.NoError(t, os.WriteFile(gitfile, oldLink, 0o644))
				}
				result := executeMove(repo, oldMain, newMain, nil)
				require.True(t, result.Success, result.Message)
				require.Equal(t, oldMain, result.MovedFrom)
				require.NoDirExists(t, oldMain)
				newNested := filepath.Join(newMain, "linked", "inside")
				require.Equal(t, externalTip, gittest.Run(t, external, "rev-parse", "HEAD"))
				require.Equal(t, nestedTip, gittest.Run(t, newNested, "rev-parse", "HEAD"))
				require.Contains(t, gittest.Run(t, external, "status", "--porcelain"), "?? uncommitted")
				checkouts := gittest.Run(t, newMain, "worktree", "list", "--porcelain")
				require.Contains(t, checkouts, "worktree "+newNested)
				require.Contains(t, checkouts, "worktree "+external)
				require.NotContains(t, checkouts, oldMain)
				// A second resume after repair also completes the pending move.
				retry := executeMove(repo, oldMain, newMain, nil)
				require.True(t, retry.Success, retry.Message)
			})
		}
	}
}

func TestMoveAndPruneUseSameLocations(t *testing.T) {
	for _, decision := range []string{"move", "decline", "occupied"} {
		t.Run(decision, func(t *testing.T) {
			root := t.TempDir()
			oldMain := filepath.Join(root, "old")
			newMain := filepath.Join(root, "repos", "demo")
			prune := planStaleWorktree(t, oldMain, filepath.Join(oldMain, "trees", "stale"), true)
			keep := filepath.Join(root, "old-unrelated")
			gittest.Worktree(t, oldMain, keep, "keep", true)
			keepTip := gittest.Run(t, keep, "rev-parse", "HEAD")
			move := syncplan.Action{Type: syncplan.ActionMove, Repo: prune.Repo, CurrentPath: oldMain, ExpectedPath: newMain}
			if decision == "occupied" {
				require.NoError(t, os.MkdirAll(newMain, 0o755))
			}
			actions := []syncplan.Action{prune, move}
			results := ExecuteActions(t.Context(), actions, false, func(string) bool { return decision != "decline" }, 4, git.GitHubAuth{})
			relocation := <-results
			require.Equal(t, decision != "occupied", relocation.Success, relocation.Message)
			removal := <-results
			require.True(t, removal.Success, removal.Message)
			mainPath := oldMain
			if decision == "move" {
				mainPath = newMain
				require.Equal(t, oldMain, relocation.MovedFrom)
			} else {
				require.Empty(t, relocation.MovedFrom)
			}
			require.Equal(t, filepath.Join(mainPath, "trees", "stale"), removal.Path)
			require.NoDirExists(t, removal.Path)
			require.Equal(t, prune.Commit, gittest.Run(t, mainPath, "rev-parse", "refs/heads/feature"))
			require.Equal(t, keepTip, gittest.Run(t, keep, "rev-parse", "HEAD"))
			require.Equal(t, oldMain, actions[0].MainRepoPath, "the caller owns the original plan")
			_, ok := <-results
			require.False(t, ok)
		})
	}
}

func TestMoveRefusesUnavailableWorktree(t *testing.T) {
	root := t.TempDir()
	oldMain, newMain := filepath.Join(root, "old"), filepath.Join(root, "new")
	wt := filepath.Join(root, "worktree")
	repo := &remote.RemoteRepo{Name: "demo", CloneURL: gittest.BareRemote(t)}
	gittest.Repo(t, oldMain, repo.CloneURL)
	gittest.Worktree(t, oldMain, wt, "feature", true)
	require.NoError(t, os.Rename(wt, filepath.Join(root, "offline")))
	result := executeMove(repo, oldMain, newMain, nil)
	require.False(t, result.Success)
	require.Contains(t, result.Message, "linked worktree")
	require.DirExists(t, oldMain)
	require.NoDirExists(t, newMain)
}

func TestMoveRelocatesCanonicalMainPath(t *testing.T) {
	root := t.TempDir()
	alias := filepath.Join(t.TempDir(), "workspace")
	require.NoError(t, os.Symlink(root, alias))
	mainPath := filepath.Join(root, "old")
	prune := planStaleWorktree(t, mainPath, filepath.Join(mainPath, "stale"), true)
	prune.WorktreePath = filepath.Join(alias, "old", "stale")
	move := syncplan.Action{Type: syncplan.ActionMove, Repo: prune.Repo,
		CurrentPath: filepath.Join(alias, "old"), ExpectedPath: filepath.Join(alias, "new")}
	results := ExecuteActions(t.Context(), []syncplan.Action{move, prune}, false, nil, 1, git.GitHubAuth{})
	for result := range results {
		require.True(t, result.Success, result.Message)
	}
	require.NoDirExists(t, filepath.Join(alias, "new", "stale"))
	require.Equal(t, prune.Commit, gittest.Run(t, move.ExpectedPath, "rev-parse", "refs/heads/feature"))
}

func TestDryRunPreviewsPruneAtMovedPath(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "old")
	prune := planStaleWorktree(t, mainPath, filepath.Join(mainPath, "stale"), false)
	move := syncplan.Action{Type: syncplan.ActionMove, Repo: prune.Repo,
		CurrentPath: mainPath, ExpectedPath: filepath.Join(root, "new")}
	results := ExecuteActions(t.Context(), []syncplan.Action{move, prune}, true, nil, 2, git.GitHubAuth{})
	require.Contains(t, (<-results).Message, "would move")
	require.Equal(t, filepath.Join(move.ExpectedPath, "stale"), (<-results).Path)
	require.DirExists(t, prune.WorktreePath)
	require.NoDirExists(t, move.ExpectedPath)
}
