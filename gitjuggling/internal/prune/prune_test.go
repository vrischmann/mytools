package prune

import (
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"github.com/stretchr/testify/require"
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

func TestPruneKeepsAncestorOfManagedCheckout(t *testing.T) {
	root := t.TempDir()
	parent := filepath.Join(root, "old")
	child := filepath.Join(parent, "src", "active")
	sibling := filepath.Join(root, "old-copy")
	const childURL = "https://example.com/owner/active.git"
	gittest.Repo(t, parent, "https://example.com/owner/old.git")
	gittest.Repo(t, child, childURL)
	gittest.Repo(t, sibling, "https://example.com/owner/old-copy.git")

	local, err := discover.Discover(root)
	require.NoError(t, err)
	orphans := FindOrphans(local, []*remote.RemoteRepo{{CloneURL: childURL}}, &config.Workspace{})
	require.Equal(t, []*OrphanRepo{{Name: "old-copy", Path: sibling}}, orphans)

	results := PruneOrphans(orphans, false, nil)
	require.Len(t, results, 1)
	require.True(t, results[0].Success, results[0].Message)
	require.NoDirExists(t, sibling)
	require.DirExists(t, filepath.Join(child, ".git"))
}

func TestPruneRechecksForNestedCheckoutAfterConfirmation(t *testing.T) {
	parent := filepath.Join(t.TempDir(), "old")
	mainClone := filepath.Join(t.TempDir(), "active")
	gittest.Repo(t, parent, "https://example.com/owner/old.git")
	gittest.Repo(t, mainClone, "https://example.com/owner/active.git")
	local, err := discover.Discover(parent)
	require.NoError(t, err)
	orphans := FindOrphans(local, nil, &config.Workspace{})
	require.Len(t, orphans, 1)

	child := filepath.Join(parent, "checkout")
	results := PruneOrphans(orphans, false, func(string) bool {
		gittest.Worktree(t, mainClone, child, "topic", false)
		return true
	})
	require.Len(t, results, 1)
	require.False(t, results[0].Success)
	require.Contains(t, results[0].Message, child)
	require.DirExists(t, filepath.Join(parent, ".git"))
	require.FileExists(t, filepath.Join(child, ".git"))

	preview := PruneOrphans(orphans, true, nil)
	require.Len(t, preview, 1)
	require.Contains(t, preview[0].Message, child)
}
