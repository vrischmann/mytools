package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestInitCreatesPrivateConfigAndPreservesExistingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "config", "config.yaml")
	original := initConfigPath
	t.Cleanup(func() { initConfigPath = original })
	initConfigPath = path
	require.NoError(t, runInit(initCmd, nil))
	info, err := os.Stat(path)
	require.NoError(t, err)
	require.Equal(t, os.FileMode(0o600), info.Mode().Perm())

	require.NoError(t, os.WriteFile(path, []byte("existing settings"), 0o600))
	require.ErrorContains(t, runInit(initCmd, nil), "already exists")
	data, err := os.ReadFile(path)
	require.NoError(t, err)
	require.Equal(t, "existing settings", string(data))
}
