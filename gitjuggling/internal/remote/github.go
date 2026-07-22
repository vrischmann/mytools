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

// GitHubToken retrieves a GitHub token, first trying the provided token,
// then falling back to `gh auth token`.
func GitHubToken(directToken string) (string, error) {
	if t := strings.TrimSpace(directToken); t != "" {
		return t, nil
	}

	cmd := exec.Command("gh", "auth", "token")
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("`gh auth token` failed: %w", err)
	}
	return strings.TrimSpace(string(out)), nil
}

// FetchGitHubRepos fetches all repos visible to the authenticated GitHub user,
// then filters by the given owners. This includes private repos owned by the
// authenticated user, which the /users/{owner}/repos endpoint would not return.
// The githubToken parameter is optional; if empty, it will try `gh auth token`.
func FetchGitHubRepos(owners []string, githubToken string) ([]*RemoteRepo, error) {
	token, err := GitHubToken(githubToken)
	if err != nil {
		return nil, err
	}

	allRepos, err := fetchAuthenticatedUserRepos(token)
	if err != nil {
		return nil, err
	}

	// Filter by configured owners
	wantOwners := make(map[string]struct{}, len(owners))
	for _, o := range owners {
		wantOwners[strings.ToLower(o)] = struct{}{}
	}

	var filtered []*RemoteRepo
	for _, r := range allRepos {
		if _, ok := wantOwners[strings.ToLower(r.Owner)]; ok {
			filtered = append(filtered, r)
		}
	}

	return filtered, nil
}

// fetchAuthenticatedUserRepos paginates through /user/repos to fetch all
// repos visible to the authenticated user (including private ones).
func fetchAuthenticatedUserRepos(token string) ([]*RemoteRepo, error) {
	client := &http.Client{Timeout: 30 * time.Second}
	var repos []*RemoteRepo
	page := 1

	for {
		url := fmt.Sprintf("https://api.github.com/user/repos?page=%d&per_page=100&affiliation=owner,collaborator,organization_member", page)

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
		defer resp.Body.Close()

		if resp.StatusCode != http.StatusOK {
			return nil, fmt.Errorf("GitHub API returned %d", resp.StatusCode)
		}

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
