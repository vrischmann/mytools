package syncstate

import (
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"github.com/stretchr/testify/require"
)

func TestSaveLoadAndDelete(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	plan := Plan{Actions: []syncplan.Action{{
		Type:         syncplan.ActionClone,
		Repo:         &remote.RemoteRepo{Owner: "owner", Name: "repo", CloneURL: "https://example.test/repo.git", Source: remote.SourceForgejo},
		ExpectedPath: filepath.Join("/tmp", "repo"),
	}}}

	require.NoError(t, Save("personal", plan))
	loaded, err := Load("personal")
	require.NoError(t, err)
	require.Equal(t, plan, *loaded)

	require.NoError(t, Delete("personal"))
	loaded, err = Load("personal")
	require.NoError(t, err)
	require.Nil(t, loaded)
}
