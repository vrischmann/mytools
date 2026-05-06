package remote

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"strings"
	"time"
)

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

// forgejoToken retrieves a Forgejo token by reading the given 1Password secret reference.
// The secretRef should be an "op://..." URI; this function hardcodes "op read" to execute it.
func forgejoToken(secretRef string) (string, error) {
	out, err := exec.Command("op", "read", "--account", "my.1password.eu", secretRef).Output()
	if err != nil {
		return "", fmt.Errorf("op read failed for %q: %w", secretRef, err)
	}
	return strings.TrimSpace(string(out)), nil
}

// FetchForgejoRepos fetches all repos for the given Forgejo user, filtering out mirrors.
func FetchForgejoRepos(baseURL, user, tokenRef string) ([]*RemoteRepo, error) {
	token, err := forgejoToken(tokenRef)
	if err != nil {
		return nil, err
	}

	client := &http.Client{Timeout: 30 * time.Second}
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
			return nil, fmt.Errorf("forgejo API returned %d for user %q", resp.StatusCode, user)
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
