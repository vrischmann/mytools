package execute

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestIsNoTrackingError(t *testing.T) {
	tests := []struct {
		name string
		msg  string
		want bool
	}{
		{
			name: "git pull no tracking information",
			msg:  "git pull --rebase failed: There is no tracking information for the current branch.\nPlease specify which branch you want to rebase against.",
			want: true,
		},
		{
			name: "lowercase variant",
			msg:  "git pull --rebase failed: fatal: no tracking information for the current branch",
			want: true,
		},
		{
			name: "unrelated git error",
			msg:  "git pull --rebase failed: error: could not apply abc1234..def5678",
			want: false,
		},
		{
			name: "stash error",
			msg:  "git stash failed: error: could not stash",
			want: false,
		},
		{
			name: "empty message",
			msg:  "",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsNoTrackingError(tt.msg)
			if got != tt.want {
				t.Errorf("IsNoTrackingError(%q) = %v, want %v", tt.msg, got, tt.want)
			}
		})
	}
}

func TestIsDirty(t *testing.T) {
	dir := t.TempDir()
	git := func(args ...string) {
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		require.NoError(t, cmd.Run())
	}
	writeFile := func(name, content string) {
		require.NoError(t, os.WriteFile(filepath.Join(dir, name), []byte(content), 0o600))
	}

	git("init")
	git("config", "user.email", "test@example.com")
	git("config", "user.name", "test")

	// Fresh empty repo: not dirty.
	clean, err := IsDirty(dir)
	require.NoError(t, err)
	require.False(t, clean)

	// Untracked file: dirty.
	writeFile("untracked.txt", "hello")
	dirty, err := IsDirty(dir)
	require.NoError(t, err)
	require.True(t, dirty)

	// Once committed: clean again.
	git("add", "untracked.txt")
	git("commit", "-m", "initial")
	clean, err = IsDirty(dir)
	require.NoError(t, err)
	require.False(t, clean)

	// Modifying a tracked file: dirty.
	writeFile("untracked.txt", "changed")
	dirty, err = IsDirty(dir)
	require.NoError(t, err)
	require.True(t, dirty)
}
