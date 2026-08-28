package discover

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"github.com/stretchr/testify/require"
)

func TestNormalizeSSHURL(t *testing.T) {
	got := NormalizeURL("git@github.com:owner/repo.git")
	want := "github.com/owner/repo"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "git@github.com:owner/repo.git", got, want)
	}
}

func TestNormalizeHTTPSURL(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{
			input: "https://github.com/owner/repo.git",
			want:  "github.com/owner/repo",
		},
		{
			input: "https://github.com/owner/repo",
			want:  "github.com/owner/repo",
		},
	}
	for _, tt := range tests {
		got := NormalizeURL(tt.input)
		if got != tt.want {
			t.Errorf("NormalizeURL(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}

func TestNormalizeForgejoURL(t *testing.T) {
	got := NormalizeURL("https://git.example.com/user/project.git")
	want := "git.example.com/user/project"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "https://git.example.com/user/project.git", got, want)
	}
}

func TestNormalizeTrailingSlash(t *testing.T) {
	got := NormalizeURL("https://github.com/owner/repo/")
	want := "github.com/owner/repo"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "https://github.com/owner/repo/", got, want)
	}
}

func TestNormalizeIdempotent(t *testing.T) {
	url := "github.com/owner/repo"
	got := NormalizeURL(url)
	if got != url {
		t.Errorf("NormalizeURL(%q) = %q, want %q", url, got, url)
	}
}

func TestDiscoverClassifiesWorktree(t *testing.T) {
	require := require.New(t)
	root := t.TempDir()

	mainPath := filepath.Join(root, "ppst")
	gittest.Repo(t, mainPath, "ssh://git@git.rischmann.fr/vincent/ppst.git")

	wtPath := filepath.Join(root, "ppst-iter-seq")
	gittest.Worktree(t, mainPath, wtPath, "iter-seq", false)

	local, err := Discover(root)
	require.NoError(err)

	// The main clone must own the URL index; the worktree must not shadow it.
	path, found := local.FindByURL("ssh://git@git.rischmann.fr/vincent/ppst.git")
	require.True(found)
	require.Equal(mainPath, path)

	worktrees := local.Worktrees()
	require.Len(worktrees, 1)
	require.Equal(wtPath, worktrees[0].Path)
	require.True(worktrees[0].IsWorktree)
	require.Equal(mainPath, worktrees[0].MainPath)
	require.Equal("git.rischmann.fr/vincent/ppst",
		NormalizeURL(worktrees[0].RemoteURLs["origin"]))
}

func TestDiscoverSkipsSubmoduleCheckout(t *testing.T) {
	require := require.New(t)
	root := t.TempDir()

	mainPath := filepath.Join(root, "super")
	gittest.Repo(t, mainPath, "ssh://git@git.rischmann.fr/vincent/super.git")

	// Fake a submodule checkout: a .git file whose gitdir lives under
	// .git/modules, not under .git/worktrees.
	subPath := filepath.Join(root, "submodule-checkout")
	require.NoError(os.MkdirAll(subPath, 0o755))
	gitFile := fmt.Sprintf("gitdir: %s\n", filepath.Join(mainPath, ".git", "modules", "sub"))
	require.NoError(os.WriteFile(filepath.Join(subPath, ".git"), []byte(gitFile), 0o644))

	local, err := Discover(root)
	require.NoError(err)

	_, found := local.FindByURL("ssh://git@git.rischmann.fr/vincent/super.git")
	require.True(found)

	worktrees := local.Worktrees()
	require.Len(worktrees, 0)

	_, found = local.FindByPath(subPath)
	require.False(found)
}
