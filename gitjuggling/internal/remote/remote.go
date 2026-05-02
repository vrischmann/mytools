package remote

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"strings"
)

// RemoteSource indicates where a remote repo originates from.
type RemoteSource int

const (
	SourceGitHub RemoteSource = iota
	SourceForgejo
)

// RemoteRepo represents a repository from a remote source (GitHub or Forgejo).
type RemoteRepo struct {
	Name       string
	Owner      string
	IsFork     bool
	IsArchived bool
	IsMirror   bool
	CloneURL   string
	Source     RemoteSource
}

// DedupKey returns a lowercase (owner, name) tuple for deduplication.
func (r *RemoteRepo) DedupKey() [2]string {
	return [2]string{strings.ToLower(r.Owner), strings.ToLower(r.Name)}
}

// ---------------------------------------------------------------------------
// GitHub client
// ---------------------------------------------------------------------------

// githubRepo is the JSON response struct for GitHub API repos.
type githubRepo struct {
	Name     string `json:"name"`
	Fork     bool   `json:"fork"`
	Archived bool   `json:"archived"`
	CloneURL string `json:"clone_url"`
	Owner    struct {
		Login string `json:"login"`
	} `json:"owner"`
}

// GitHubToken retrieves a GitHub token using `gh auth token`.
func GitHubToken() (string, error) {
	cmd := exec.Command("gh", "auth", "token")
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("`gh auth token` failed: %w", err)
	}
	return strings.TrimSpace(string(out)), nil
}

// FetchGitHubRepos fetches all repos for the given GitHub owners (users or orgs).
func FetchGitHubRepos(owners []string) ([]*RemoteRepo, error) {
	token, err := GitHubToken()
	if err != nil {
		return nil, err
	}

	var allRepos []*RemoteRepo
	for _, owner := range owners {
		repos, err := fetchGitHubOwnerRepos(token, owner)
		if err != nil {
			return nil, fmt.Errorf("fetching GitHub repos for %q: %w", owner, err)
		}
		allRepos = append(allRepos, repos...)
	}
	return allRepos, nil
}

// fetchGitHubOwnerRepos paginates through the GitHub API for a single owner.
// It tries the org endpoint first, falling back to the user endpoint on 404.
func fetchGitHubOwnerRepos(token, owner string) ([]*RemoteRepo, error) {
	client := &http.Client{}
	var repos []*RemoteRepo
	page := 1

	for {
		// Try org endpoint first
		url := fmt.Sprintf("https://api.github.com/orgs/%s/repos?page=%d&per_page=100&type=all", owner, page)

		req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, url, nil)
		if err != nil {
			return nil, err
		}
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("User-Agent", "gitjuggling")

		resp, err := client.Do(req)
		if err != nil {
			return nil, fmt.Errorf("request to GitHub API failed: %w", err)
		}

		// If 404, try user endpoint
		if resp.StatusCode == http.StatusNotFound {
			resp.Body.Close()
			url = fmt.Sprintf("https://api.github.com/users/%s/repos?page=%d&per_page=100&type=all", owner, page)

			req, err = http.NewRequestWithContext(context.Background(), http.MethodGet, url, nil)
			if err != nil {
				return nil, err
			}
			req.Header.Set("Authorization", "Bearer "+token)
			req.Header.Set("User-Agent", "gitjuggling")

			resp, err = client.Do(req)
			if err != nil {
				return nil, fmt.Errorf("request to GitHub API failed: %w", err)
			}
		}
		defer resp.Body.Close()

		var pageRepos []githubRepo
		if err := json.NewDecoder(resp.Body).Decode(&pageRepos); err != nil {
			return nil, fmt.Errorf("decoding GitHub response: %w", err)
		}

		if len(pageRepos) == 0 {
			break
		}

		for _, r := range pageRepos {
			repos = append(repos, &RemoteRepo{
				Name:       r.Name,
				Owner:      r.Owner.Login,
				IsFork:     r.Fork,
				IsArchived: r.Archived,
				IsMirror:   false,
				CloneURL:   r.CloneURL,
				Source:     SourceGitHub,
			})
		}

		page++
	}

	return repos, nil
}

// ---------------------------------------------------------------------------
// Forgejo / Gitea client
// ---------------------------------------------------------------------------

// forgejoRepo is the JSON response struct for Forgejo API repos.
type forgejoRepo struct {
	Name     string `json:"name"`
	Fork     bool   `json:"fork"`
	Archived bool   `json:"archived"`
	Mirror   bool   `json:"mirror"`
	CloneURL string `json:"clone_url"`
	Owner    struct {
		Login string `json:"login"`
	} `json:"owner"`
}

// httpsToSSH converts an HTTPS git URL to SSH format.
//
// Examples:
//
//	https://git.example.com/user/project.git → git@git.example.com:user/project.git
//	https://git.example.com/user/project      → git@git.example.com:user/project.git
func httpsToSSH(url string) string {
	url = strings.TrimRight(url, "/")

	if after, ok := strings.CutPrefix(url, "https://"); ok {
		after = strings.TrimSuffix(after, ".git")
		host, path, ok := strings.Cut(after, "/")
		if !ok {
			return url // can't parse, return as-is
		}
		return "git@" + host + ":" + path + ".git"
	}

	return url // not HTTPS, return as-is
}

// ForgejoToken retrieves a Forgejo token by reading the given 1Password secret reference.
// The secretRef should be an "op://..." URI; this function hardcodes "op read" to execute it.
func ForgejoToken(secretRef string) (string, error) {
	out, err := exec.Command("op", "read", secretRef).Output()
	if err != nil {
		return "", fmt.Errorf("op read failed for %q: %w", secretRef, err)
	}
	return strings.TrimSpace(string(out)), nil
}

// FetchForgejoRepos fetches all repos for the given Forgejo user, filtering out mirrors.
func FetchForgejoRepos(baseURL, user, tokenRef string) ([]*RemoteRepo, error) {
	token, err := ForgejoToken(tokenRef)
	if err != nil {
		return nil, err
	}

	client := &http.Client{}
	var repos []*RemoteRepo
	page := 1

	baseURL = strings.TrimRight(baseURL, "/")

	for {
		url := fmt.Sprintf("%s/api/v1/users/%s/repos?page=%d&limit=50", baseURL, user, page)

		req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, url, nil)
		if err != nil {
			return nil, err
		}
		req.Header.Set("Authorization", "token "+token)

		resp, err := client.Do(req)
		if err != nil {
			return nil, fmt.Errorf("request to Forgejo API failed for user %q at %s: %w", user, baseURL, err)
		}
		defer resp.Body.Close()

		if resp.StatusCode != http.StatusOK {
			return nil, fmt.Errorf("Forgejo API returned %d for user %q", resp.StatusCode, user)
		}

		var pageRepos []forgejoRepo
		if err := json.NewDecoder(resp.Body).Decode(&pageRepos); err != nil {
			return nil, fmt.Errorf("decoding Forgejo response: %w", err)
		}

		if len(pageRepos) == 0 {
			break
		}

		for _, r := range pageRepos {
			// Skip mirrors — they are duplicates of GitHub repos
			if r.Mirror {
				continue
			}

			repos = append(repos, &RemoteRepo{
				Name:       r.Name,
				Owner:      r.Owner.Login,
				IsFork:     r.Fork,
				IsArchived: r.Archived,
				IsMirror:   r.Mirror,
				CloneURL:   httpsToSSH(r.CloneURL),
				Source:     SourceForgejo,
			})
		}

		page++
	}

	return repos, nil
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

// DedupRepos removes Forgejo repos that duplicate GitHub repos (same owner/name).
// It returns the modified github and forgejo slices.
func DedupRepos(github, forgejo []*RemoteRepo) ([]*RemoteRepo, []*RemoteRepo) {
	githubKeys := make(map[[2]string]struct{}, len(github))
	for _, r := range github {
		githubKeys[r.DedupKey()] = struct{}{}
	}

	var filtered []*RemoteRepo
	for _, r := range forgejo {
		if _, dup := githubKeys[r.DedupKey()]; !dup {
			filtered = append(filtered, r)
		}
	}

	return github, filtered
}
