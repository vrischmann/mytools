package git

import (
	"os"
	"strings"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"github.com/stretchr/testify/require"
)

func TestGitHubCredentialIsScopedToHTTPS(t *testing.T) {
	t.Setenv("GIT_CONFIG_GLOBAL", os.DevNull)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	dir := t.TempDir()
	gittest.Repo(t, dir, "https://github.com/owner/repo.git")
	gittest.Run(t, dir, "config", "credential.helper", `!f() { printf 'username=local\npassword=native-credential\n'; }; f`)
	const token = "workspace-test-token"
	auth := NewGitHubAuth(token)
	for _, tc := range []struct{ url, password string }{
		{"https://github.com/owner/repo.git", token},
		{"https://github.com:443/owner/repo.git", token},
		{"https://forgejo.example/owner/repo.git", "native-credential"},
		{"https://github.com.example/owner/repo.git", "native-credential"},
		{"http://github.com/owner/repo.git", "native-credential"},
		{"https://github.com:8443/owner/repo.git", "native-credential"},
	} {
		t.Run(tc.url, func(t *testing.T) {
			cmd := auth.RemoteCommand(dir, "origin", "credential", "fill")
			require.NotContains(t, strings.Join(cmd.Args, " "), token)
			cmd.Stdin = strings.NewReader("url=" + tc.url + "\n\n")
			out, err := cmd.CombinedOutput()
			require.NoError(t, err, "%s", out)
			require.Contains(t, string(out), "password="+tc.password+"\n")
		})
	}
	require.NotContains(t, gittest.Run(t, dir, "config", "--local", "--list"), token)
}

func TestGitHubAuthChecksActualTransportBeforeLookingUpToken(t *testing.T) {
	t.Setenv("GIT_CONFIG_GLOBAL", os.DevNull)
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	dir := t.TempDir()
	gittest.Repo(t, dir, "https://github.com/owner/repo.git")
	gittest.Run(t, dir, "remote", "add", "fork", "https://forgejo.example/owner/repo.git")
	gittest.Run(t, dir, "config", "branch.master.remote", "fork")
	auth := GitHubAuth{token: func() string {
		t.Fatal("looked up a GitHub token for another transport")
		return ""
	}}
	require.Nil(t, auth.RemoteCommand(dir, "", "pull", "--rebase").Env)
	gittest.Run(t, dir, "config", "branch.master.remote", "origin")
	gittest.Run(t, dir, "config", "url.git@github.com:.insteadOf", "https://github.com/")
	require.Nil(t, auth.RemoteCommand(dir, "", "pull", "--rebase").Env)
	require.Nil(t, auth.RemoteCommand(dir, "https://github.com/owner/repo.git", "clone", "--", "https://github.com/owner/repo.git", "clone").Env)
	gittest.Run(t, dir, "config", "--unset", "url.git@github.com:.insteadOf")
	gittest.Run(t, dir, "remote", "set-url", "--push", "origin", "https://github.com/owner/repo.git")
	gittest.Run(t, dir, "remote", "set-url", "--push", "--add", "origin", "https://forgejo.example/owner/repo.git")
	require.Nil(t, auth.RemoteCommand(dir, "origin", "push", "origin", "master").Env)
}
