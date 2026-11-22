package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"github.com/stretchr/testify/require"
)

func TestExecFindsSiblingRepositoriesWithTheSameNameAsSubmodules(t *testing.T) {
	root := t.TempDir()
	t.Chdir(root)
	for _, path := range []string{"project", "project/deps/lib", "project/tools/lib", "project/tools/standalone",
		"project/tools/standalone/inner", "z/lib", "z/inner"} {
		gittest.Run(t, root, "init", "-q", path)
	}
	modules := "# submodule names can differ from their paths\n[submodule \"library\"]\n path = \"deps/lib\"\n"
	require.NoError(t, os.WriteFile(filepath.Join("project", ".gitmodules"), []byte(modules), 0o600))
	require.NoError(t, os.WriteFile(filepath.Join("project", "tools", "standalone", ".gitmodules"),
		[]byte("[submodule \"nested\"]\n path = inner\n"), 0o600))
	paths, err := getRepositoryPaths(5)
	require.NoError(t, err)
	require.ElementsMatch(t, []string{"project", "project/tools/lib", "project/tools/standalone", "z/lib", "z/inner"}, paths)
	paths, err = getRepositoryPaths(1)
	require.NoError(t, err)
	require.Equal(t, []string{"project"}, paths)
}

func TestExecRejectsInvalidConcurrencyBeforeDiscovery(t *testing.T) {
	t.Chdir(t.TempDir())
	original := execConcurrency
	t.Cleanup(func() { execConcurrency = original })
	for _, concurrency := range []int{0, -1} {
		execConcurrency = concurrency
		require.ErrorContains(t, runExec(execCmd, []string{"status"}), "--concurrency must be at least 1")
	}
}
