package prune

import (
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
)

func TestFindOrphans(t *testing.T) {
	ws := &config.Workspace{}

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: "/home/user/dev/orphan-repo", RemoteURLs: map[string]string{"origin": "https://github.com/someone/orphan-repo.git"}},
		{Path: "/home/user/dev/known-repo", RemoteURLs: map[string]string{"origin": "https://github.com/owner/known-repo.git"}},
	})

	remoteRepos := []*remote.RemoteRepo{
		{Name: "known-repo", Owner: "owner", CloneURL: "https://github.com/owner/known-repo.git"},
	}

	orphans := FindOrphans(local, remoteRepos, ws)
	if len(orphans) != 1 {
		t.Fatalf("expected 1 orphan, got %d", len(orphans))
	}
	if orphans[0].Name != "orphan-repo" {
		t.Errorf("expected orphan-repo, got %s", orphans[0].Name)
	}
}

func TestFindOrphansIgnored(t *testing.T) {
	ws := &config.Workspace{
		Ignore: []string{"orphan-repo"},
	}

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: "/home/user/dev/orphan-repo", RemoteURLs: map[string]string{"origin": "https://github.com/someone/orphan-repo.git"}},
		{Path: "/home/user/dev/known-repo", RemoteURLs: map[string]string{"origin": "https://github.com/owner/known-repo.git"}},
	})

	remoteRepos := []*remote.RemoteRepo{
		{Name: "known-repo", Owner: "owner", CloneURL: "https://github.com/owner/known-repo.git"},
	}

	orphans := FindOrphans(local, remoteRepos, ws)
	if len(orphans) != 0 {
		t.Fatalf("expected 0 orphans (ignored), got %d", len(orphans))
	}
}

func TestFindOrphansIgnoreGlob(t *testing.T) {
	ws := &config.Workspace{
		Ignore: []string{"temp-*"},
	}

	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: "/home/user/dev/temp-experiment", RemoteURLs: map[string]string{"origin": "https://github.com/someone/temp-experiment.git"}},
		{Path: "/home/user/dev/orphan-repo", RemoteURLs: map[string]string{"origin": "https://github.com/someone/orphan-repo.git"}},
	})

	remoteRepos := []*remote.RemoteRepo{}

	orphans := FindOrphans(local, remoteRepos, ws)
	if len(orphans) != 1 {
		t.Fatalf("expected 1 orphan (temp-experiment ignored), got %d", len(orphans))
	}
	if orphans[0].Name != "orphan-repo" {
		t.Errorf("expected orphan-repo, got %s", orphans[0].Name)
	}
}
