package git

import (
	"net/url"
	"os/exec"
	"slices"
	"strings"
	"sync"

	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
)

// GitHubAuth supplies an optional credential for GitHub HTTPS transports.
// Its zero value uses Git's own authentication.
type GitHubAuth struct {
	token func() string
}

func NewGitHubAuth(configuredToken string) GitHubAuth {
	return GitHubAuth{token: sync.OnceValue(func() string {
		// Without a workspace or gh token, Git can still use its own helpers.
		token, _ := remote.GitHubToken(configuredToken)
		return token
	})}
}

// RemoteCommand constructs a Git command for the given remote name or URL.
// An empty remote selects the current branch's fetch remote, as git pull does.
func (auth GitHubAuth) RemoteCommand(dir, remote, command string, args ...string) *exec.Cmd {
	cmd := exec.Command("git", command)
	cmd.Args = append(cmd.Args, args...)
	cmd.Dir = dir
	if auth.token == nil {
		return cmd
	}

	// Resolve insteadOf rewrites and the actual tracking or push destination.
	// ls-remote --get-url only inspects local config and never connects.
	resolveArgs := []string{"ls-remote", "--get-url"}
	if command == "push" {
		resolveArgs = []string{"remote", "get-url", "--push", "--all"}
	}
	if remote != "" {
		resolveArgs = append(resolveArgs, "--", remote)
	}
	resolve := exec.Command("git", resolveArgs...)
	resolve.Dir = dir
	urls, err := resolve.Output()
	if err != nil {
		return cmd
	}
	for raw := range strings.SplitSeq(strings.TrimSuffix(string(urls), "\n"), "\n") {
		u, err := url.Parse(raw)
		if err != nil || u.Scheme != "https" || !strings.EqualFold(u.Hostname(), "github.com") || (u.Port() != "" && u.Port() != "443") {
			return cmd
		}
	}
	token := auth.token()
	if token == "" {
		return cmd
	}

	// Reset helpers for this host so Git cannot store the supplied token.
	// Git also matches the credential URL when redirects or submodules ask
	// for authentication. The helper only responds to credential reads.
	const scope = "credential.https://github.com.helper="
	const helper = `!f() { if test "$1" = get; then printf 'username=x-access-token\npassword=%s\n' "$GITJUGGLING_GITHUB_TOKEN"; fi; }; f`
	cmd.Args = slices.Insert(cmd.Args, 1, "-c", scope, "-c", scope+helper)
	cmd.Env = append(cmd.Environ(), "GITJUGGLING_GITHUB_TOKEN="+token, "GIT_TERMINAL_PROMPT=0")
	return cmd
}
