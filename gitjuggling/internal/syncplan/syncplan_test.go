package syncplan

import (
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
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

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws)
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

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws)
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

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws)
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

	actions := BuildPlan([]*remote.RemoteRepo{repo}, local, ws)
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

	actions := BuildPlan([]*remote.RemoteRepo{repo1, repo2}, local, ws)
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
