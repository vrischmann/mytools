package syncstate

import (
	"os"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"github.com/stretchr/testify/require"
)

func TestSaveLoadAndDelete(t *testing.T) {
	dir := t.TempDir()
	store, err := NewStore(filepath.Join(dir, "plans"), "personal", filepath.Join(dir, "config.yaml"), dir)
	require.NoError(t, err)
	plan := Plan{Actions: []syncplan.Action{{
		Type:         syncplan.ActionClone,
		Repo:         &remote.RemoteRepo{Owner: "owner", Name: "repo", CloneURL: "https://example.test/repo.git", Source: remote.SourceForgejo},
		ExpectedPath: filepath.Join("/tmp", "repo"),
	}}}

	require.NoError(t, store.Save(plan))
	loaded, err := store.Load()
	require.NoError(t, err)
	require.Equal(t, plan, *loaded)

	require.NoError(t, store.Delete())
	loaded, err = store.Load()
	require.NoError(t, err)
	require.Nil(t, loaded)
}

func TestPlansAreBoundToConfigAndRoot(t *testing.T) {
	dir, rootA, rootB := t.TempDir(), t.TempDir(), t.TempDir()
	configA, configB := filepath.Join(dir, "a.yaml"), filepath.Join(dir, "b.yaml")
	for _, path := range []string{configA, configB} {
		require.NoError(t, os.WriteFile(path, nil, 0o600))
	}
	plansDir := filepath.Join(dir, "plans")
	store := func(name, path, root string) Store {
		t.Helper()
		s, err := NewStore(plansDir, name, path, root)
		require.NoError(t, err)
		return s
	}
	original := store("personal", configA, rootA)
	first := Plan{Actions: []syncplan.Action{{Type: syncplan.ActionUpdate, LocalPath: rootA}}}
	second := Plan{Actions: []syncplan.Action{{Type: syncplan.ActionUpdate, LocalPath: rootB}}}
	require.NoError(t, original.Save(first))
	for _, other := range []Store{store("personal", configB, rootA), store("personal", configA, rootB), store("work", configA, rootA)} {
		pending, err := other.Load()
		require.NoError(t, err)
		require.Nil(t, pending)
		require.NoError(t, other.Save(second))
		require.NoError(t, other.Delete())
		pending, err = original.Load()
		require.NoError(t, err)
		require.Equal(t, &first, pending)
	}

	alias := filepath.Join(dir, "alias")
	require.NoError(t, os.Symlink(rootA, alias))
	pending, err := store("personal", configA, alias).Load()
	require.NoError(t, err)
	require.Equal(t, &first, pending)

	other := store("personal", configB, rootB)
	data, err := os.ReadFile(original.path)
	require.NoError(t, err)
	require.NoError(t, os.WriteFile(other.path, data, 0o600))
	_, err = other.Load()
	require.ErrorContains(t, err, "does not match")
}

func TestLegacyPlanWithUnknownRootIsPreserved(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "personal.json")
	legacy := []byte(`{"actions":[{"LocalPath":"/old/root/repo"}]}`)
	require.NoError(t, os.WriteFile(path, legacy, 0o600))
	store, err := NewStore(dir, "personal", filepath.Join(dir, "config.yaml"), dir)
	require.NoError(t, err)
	plan, err := store.Load()
	require.NoError(t, err)
	require.Nil(t, plan)
	require.NoError(t, store.Delete())
	data, err := os.ReadFile(path)
	require.NoError(t, err)
	require.Equal(t, legacy, data)
}

func TestLoadRebuildsPlansWithAmbiguousPaths(t *testing.T) {
	dir := t.TempDir()
	store, err := NewStore(dir, "work", filepath.Join(dir, "config"), dir)
	require.NoError(t, err)
	dest := filepath.Join(dir, "alice-foo")
	for _, actions := range [][]syncplan.Action{
		{{Type: syncplan.ActionMove, ExpectedPath: dest}, {Type: syncplan.ActionClone, ExpectedPath: dest}},
	} {
		require.NoError(t, store.Save(Plan{Actions: actions}))
		pending, err := store.Load()
		require.NoError(t, err)
		require.Nil(t, pending)
		require.FileExists(t, store.path)
	}
}
