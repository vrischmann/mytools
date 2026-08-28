package syncplan

import (
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"github.com/stretchr/testify/require"
)

func testWorkspace() *config.Workspace {
	return &config.Workspace{
		Root:         "/home/user/dev",
		GitHubOwners: []string{"testowner"},
		Rules: config.Rules{
			Base:     "/home/user/dev/repos",
			Forks:    "/home/user/dev/forks",
			Archived: "/home/user/dev/archived",
		},
	}
}

func makeRepo(name, owner string, isFork, isArchived bool) *remote.RemoteRepo {
	return &remote.RemoteRepo{
		Name:       name,
		Owner:      owner,
		IsFork:     isFork,
		IsArchived: isArchived,
		IsMirror:   false,
		CloneURL:   "https://github.com/" + owner + "/" + name + ".git",
		Source:     remote.SourceGitHub,
	}
}

func TestClassifyBaseRepo(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", false, false)
	got := ClassifyRepo(repo, ws, false)
	want := filepath.Join("/home/user/dev/repos", "myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() = %q, want %q", got, want)
	}
}

func TestClassifyFork(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", true, false)
	got := ClassifyRepo(repo, ws, false)
	want := filepath.Join("/home/user/dev/forks", "myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() = %q, want %q", got, want)
	}
}

func TestClassifyArchived(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", false, true)
	got := ClassifyRepo(repo, ws, false)
	want := filepath.Join("/home/user/dev/archived", "myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() = %q, want %q", got, want)
	}
}

func TestClassifyForkTakesPriorityOverArchived(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", true, true)
	got := ClassifyRepo(repo, ws, false)
	want := filepath.Join("/home/user/dev/forks", "myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() = %q, want %q", got, want)
	}
}

func TestClassifyNoForksDirFallsBackToBase(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Forks = ""
	repo := makeRepo("myrepo", "owner", true, false)
	got := ClassifyRepo(repo, ws, false)
	want := filepath.Join("/home/user/dev/repos", "myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() = %q, want %q", got, want)
	}
}

func TestBuildPlanClone(t *testing.T) {
	ws := testWorkspace()
	local := discover.NewLocalRepos(nil)
	repo := makeRepo("newrepo", "owner", false, false)

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}

	if actions[0].Type != ActionClone {
		t.Errorf("expected ActionClone, got %d", actions[0].Type)
	}

	expected := filepath.Join("/home/user/dev/repos", "newrepo")
	if actions[0].ExpectedPath != expected {
		t.Errorf("expected path %q, got %q", expected, actions[0].ExpectedPath)
	}
}

func TestBuildPlanUpdate(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", false, false)
	expected := filepath.Join("/home/user/dev/repos", "myrepo")

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: expected, RemoteURLs: map[string]string{"origin": "https://github.com/owner/myrepo.git"}},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}

	if actions[0].Type != ActionUpdate {
		t.Fatalf("expected ActionUpdate, got %d", actions[0].Type)
	}

	if actions[0].LocalPath != expected {
		t.Errorf("expected LocalPath %q, got %q", expected, actions[0].LocalPath)
	}
	if !actions[0].AlreadyInPlace {
		t.Fatalf("expected AlreadyInPlace to be true")
	}
}

func TestBuildPlanMove(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", false, false)
	wrongPath := "/home/user/dev/some-other-place/myrepo"

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: wrongPath, RemoteURLs: map[string]string{"origin": "https://github.com/owner/myrepo.git"}},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}

	if actions[0].Type != ActionMove {
		t.Fatalf("expected ActionMove, got %d", actions[0].Type)
	}

	if actions[0].CurrentPath != wrongPath {
		t.Errorf("expected CurrentPath %q, got %q", wrongPath, actions[0].CurrentPath)
	}

	expected := filepath.Join("/home/user/dev/repos", "myrepo")
	if actions[0].ExpectedPath != expected {
		t.Errorf("expected ExpectedPath %q, got %q", expected, actions[0].ExpectedPath)
	}
}

func TestBuildPlanMoveDestinationOccupied(t *testing.T) {
	ws := testWorkspace()

	// Fork repo — rules say it goes to /home/user/dev/forks/
	repo := makeRepo("nvim-treesitter", "vrischmann", true, false)

	// Currently lives at a different location
	currentPath := filepath.Join("/home/user/dev/stuff/neovim", "nvim-treesitter")
	expectedPath := filepath.Join("/home/user/dev/forks", "nvim-treesitter")

	// The expected path is already occupied by a different repo
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: currentPath, RemoteURLs: map[string]string{"origin": "git@github.com:vrischmann/nvim-treesitter.git"}},
		{Path: expectedPath, RemoteURLs: map[string]string{"origin": "https://github.com/nvim-treesitter/nvim-treesitter"}},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}

	// Should fall back to update in place, not try to move
	if actions[0].Type != ActionUpdate {
		t.Fatalf("expected ActionUpdate (destination occupied), got %d", actions[0].Type)
	}
	if actions[0].LocalPath != currentPath {
		t.Errorf("expected LocalPath %q, got %q", currentPath, actions[0].LocalPath)
	}
	if actions[0].AlreadyInPlace {
		t.Fatalf("expected AlreadyInPlace to be false")
	}
}

func TestClassifyClash(t *testing.T) {
	ws := testWorkspace()
	repo := makeRepo("myrepo", "owner", false, false)
	got := ClassifyRepo(repo, ws, true)
	want := filepath.Join("/home/user/dev/repos", "owner-myrepo")
	if got != want {
		t.Errorf("ClassifyRepo() clash = %q, want %q", got, want)
	}
}

func TestBuildPlanClash(t *testing.T) {
	ws := testWorkspace()
	repo1 := makeRepo("samename", "owner1", false, false)
	repo2 := makeRepo("samename", "owner2", false, false)

	local := discover.NewLocalRepos(nil)

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws, nil)
	if len(actions) != 2 {
		t.Fatalf("expected 2 actions, got %d", len(actions))
	}

	expected1 := filepath.Join("/home/user/dev/repos", "owner1-samename")
	expected2 := filepath.Join("/home/user/dev/repos", "owner2-samename")

	for _, a := range actions {
		if a.Type != ActionClone {
			t.Errorf("expected ActionClone, got %d", a.Type)
		}
		if a.ExpectedPath != expected1 && a.ExpectedPath != expected2 {
			t.Errorf("unexpected ExpectedPath %q", a.ExpectedPath)
		}
	}
}

func TestBuildPlanIgnoreExact(t *testing.T) {
	ws := testWorkspace()
	ws.Ignore = []string{"dotfiles"}

	repo1 := makeRepo("dotfiles", "owner", false, false)
	repo2 := makeRepo("myproject", "owner", false, false)
	local := discover.NewLocalRepos(nil)

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}
	if actions[0].Repo.Name != "myproject" {
		t.Errorf("expected myproject, got %s", actions[0].Repo.Name)
	}
}

func TestBuildPlanIgnoreGlob(t *testing.T) {
	ws := testWorkspace()
	ws.Ignore = []string{"temp-*"}

	repo1 := makeRepo("temp-experiment", "owner", false, false)
	repo2 := makeRepo("real-project", "owner", false, false)
	local := discover.NewLocalRepos(nil)

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws, nil)
	if len(actions) != 1 {
		t.Fatalf("expected 1 action, got %d", len(actions))
	}
	if actions[0].Repo.Name != "real-project" {
		t.Errorf("expected real-project, got %s", actions[0].Repo.Name)
	}
}

func TestBuildPlanIgnoreAll(t *testing.T) {
	ws := testWorkspace()
	ws.Ignore = []string{"*"}

	repo1 := makeRepo("repo1", "owner", false, false)
	repo2 := makeRepo("repo2", "owner", false, false)
	local := discover.NewLocalRepos(nil)

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws, nil)
	if len(actions) != 0 {
		t.Fatalf("expected 0 actions, got %d", len(actions))
	}
}

func TestClassifyPatternFull(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/workspaces/$1"},
	}
	repo := makeRepo("workspace-foo", "owner", false, false)

	got := ClassifyRepo(repo, ws, false)
	require.Equal(t, "/home/user/dev/workspaces/foo", got)
}

func TestClassifyPatternWholeMatch(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-.+$`, To: "/home/user/dev/workspaces/$0"},
	}
	repo := makeRepo("workspace-foo", "owner", false, false)

	got := ClassifyRepo(repo, ws, false)
	require.Equal(t, "/home/user/dev/workspaces/workspace-foo", got)
}

func TestClassifyPatternBeatsFork(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/workspaces/$1"},
	}
	repo := makeRepo("workspace-foo", "owner", true, false) // isFork

	got := ClassifyRepo(repo, ws, false)
	require.Equal(t, "/home/user/dev/workspaces/foo", got)
}

func TestClassifyPatternFirstMatchWins(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/ws/$1"},
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/other/$1"},
	}
	repo := makeRepo("workspace-foo", "owner", false, false)

	got := ClassifyRepo(repo, ws, false)
	require.Equal(t, "/home/user/dev/ws/foo", got)
}

func TestClassifyPatternNoMatchFallsBackToBase(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/ws/$1"},
	}
	repo := makeRepo("myrepo", "owner", false, false)

	got := ClassifyRepo(repo, ws, false)
	require.Equal(t, filepath.Join("/home/user/dev/repos", "myrepo"), got)
}

func TestBuildPlanClonePattern(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/workspaces/$1"},
	}
	local := discover.NewLocalRepos(nil)
	repo := makeRepo("workspace-foo", "owner", false, false)

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws, nil)
	require.Len(t, actions, 1)
	require.Equal(t, ActionClone, actions[0].Type)
	require.Equal(t, "/home/user/dev/workspaces/foo", actions[0].ExpectedPath)
}

func TestBuildPlanPatternClash(t *testing.T) {
	ws := testWorkspace()
	ws.Rules.Patterns = []config.PatternRule{
		{Pattern: `^workspace-(.+)$`, To: "/home/user/dev/workspaces/$1"},
	}
	repo1 := makeRepo("workspace-foo", "owner1", false, false)
	repo2 := makeRepo("workspace-foo", "owner2", false, false)
	local := discover.NewLocalRepos(nil)

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws, nil)
	require.Len(t, actions, 2)

	paths := map[string]bool{
		actions[0].ExpectedPath: true,
		actions[1].ExpectedPath: true,
	}
	require.True(t, paths["/home/user/dev/workspaces/owner1-foo"], "expected owner1-foo in %v", paths)
	require.True(t, paths["/home/user/dev/workspaces/owner2-foo"], "expected owner2-foo in %v", paths)
}

func branchExistsStub(online bool) BranchExistsFunc {
	return func(localPath, remoteName, branch string) (bool, error) {
		return online, nil
	}
}

func TestBuildPlanPrunesStaleWorktree(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "ppst")
	wtPath := filepath.Join(root, "ppst-iter-seq")

	const url = "ssh://git@git.rischmann.fr/vincent/ppst.git"
	gittest.Repo(t, mainPath, url)
	gittest.Worktree(t, mainPath, wtPath, "iter-seq", false)

	repo := &remote.RemoteRepo{
		Name: "ppst", Owner: "vincent", CloneURL: url, Source: remote.SourceForgejo,
	}
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: mainPath, RemoteURLs: map[string]string{"origin": url}},
		{Path: wtPath, RemoteURLs: map[string]string{"origin": url}, IsWorktree: true, MainPath: mainPath},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, testWorkspace(), branchExistsStub(false))

	require.Len(t, actions, 2) // update of the main clone + worktree prune

	var prune *Action
	for i := range actions {
		if actions[i].Type == ActionPruneWorktree {
			prune = &actions[i]
		}
	}
	require.NotNil(t, prune, "expected a worktree prune action")
	require.Equal(t, wtPath, prune.WorktreePath)
	require.Equal(t, mainPath, prune.MainRepoPath)
	require.Equal(t, "iter-seq", prune.Branch)
	require.Equal(t, repo, prune.Repo)
	require.True(t, prune.Merged, "branch at HEAD of master should be merged")
}

func TestBuildPlanKeepsWorktreeWhenBranchExistsUpstream(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "ppst")
	wtPath := filepath.Join(root, "ppst-iter-seq")

	const url = "ssh://git@git.rischmann.fr/vincent/ppst.git"
	gittest.Repo(t, mainPath, url)
	gittest.Worktree(t, mainPath, wtPath, "iter-seq", false)

	repo := &remote.RemoteRepo{
		Name: "ppst", Owner: "vincent", CloneURL: url, Source: remote.SourceForgejo,
	}
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: mainPath, RemoteURLs: map[string]string{"origin": url}},
		{Path: wtPath, RemoteURLs: map[string]string{"origin": url}, IsWorktree: true, MainPath: mainPath},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, testWorkspace(), branchExistsStub(true))

	for _, a := range actions {
		require.NotEqual(t, ActionPruneWorktree, a.Type)
	}
}

// A diverged branch (e.g. squash-merged on the forge) still yields a prune
// action, but flagged as not merged so the branch is kept.
func TestBuildPlanPrunesUnmergedWorktreeKeepsBranchFlag(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "ppst")
	wtPath := filepath.Join(root, "ppst-iter-seq")

	const url = "ssh://git@git.rischmann.fr/vincent/ppst.git"
	gittest.Repo(t, mainPath, url)
	gittest.Worktree(t, mainPath, wtPath, "iter-seq", true) // branch has unmerged work

	repo := &remote.RemoteRepo{
		Name: "ppst", Owner: "vincent", CloneURL: url, Source: remote.SourceForgejo,
	}
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: mainPath, RemoteURLs: map[string]string{"origin": url}},
		{Path: wtPath, RemoteURLs: map[string]string{"origin": url}, IsWorktree: true, MainPath: mainPath},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, testWorkspace(), branchExistsStub(false))

	var prune *Action
	for i := range actions {
		if actions[i].Type == ActionPruneWorktree {
			prune = &actions[i]
		}
	}
	require.NotNil(t, prune, "expected a worktree prune action even for an unmerged branch")
	require.False(t, prune.Merged, "diverged branch must be flagged as not merged")
}

func TestBuildPlanWorktreeOfUnknownRepoIgnored(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "other")
	wtPath := filepath.Join(root, "other-wt")

	const url = "ssh://git@git.rischmann.fr/vincent/other.git"
	gittest.Repo(t, mainPath, url)
	gittest.Worktree(t, mainPath, wtPath, "gone-branch", false)

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: wtPath, RemoteURLs: map[string]string{"origin": url}, IsWorktree: true, MainPath: mainPath},
	})

	// The remote list only knows a different repo, so nothing about the
	// worktree's repo is in the plan.
	actions := BuildPlan([]*remote.RemoteRepo{makeRepo("unrelated", "vincent", false, false)}, local, testWorkspace(), branchExistsStub(false))

	for _, a := range actions {
		require.NotEqual(t, ActionPruneWorktree, a.Type)
	}
}

func TestBuildPlanNilBranchExistsSkipsWorktrees(t *testing.T) {
	root := t.TempDir()
	mainPath := filepath.Join(root, "ppst")
	wtPath := filepath.Join(root, "ppst-iter-seq")

	const url = "ssh://git@git.rischmann.fr/vincent/ppst.git"
	gittest.Repo(t, mainPath, url)
	gittest.Worktree(t, mainPath, wtPath, "iter-seq", false)

	repo := &remote.RemoteRepo{
		Name: "ppst", Owner: "vincent", CloneURL: url, Source: remote.SourceForgejo,
	}
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: mainPath, RemoteURLs: map[string]string{"origin": url}},
		{Path: wtPath, RemoteURLs: map[string]string{"origin": url}, IsWorktree: true, MainPath: mainPath},
	})

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, testWorkspace(), nil)

	for _, a := range actions {
		require.NotEqual(t, ActionPruneWorktree, a.Type)
	}
}
