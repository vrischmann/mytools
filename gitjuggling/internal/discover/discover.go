package discover

import (
	"fmt"
	"io/fs"
	"os/exec"
	"path/filepath"
	"strings"
)

// LocalRepo represents a locally discovered git repository.
type LocalRepo struct {
	Path       string
	RemoteURLs map[string]string // remote name → URL
}

// LocalRepos holds all discovered local repos with lookup indices.
type LocalRepos struct {
	Repos  []*LocalRepo
	ByURL  map[string]string     // normalized URL → local path
	ByPath map[string]*LocalRepo // absolute path → LocalRepo
	ByName map[string][]string   // dir name → list of local paths
}

// Discover walks the filesystem under root, finding git repos by looking for
// .git directories. For each repo it reads the origin remote URL.
func Discover(root string) (*LocalRepos, error) {
	var repos []*LocalRepo
	byURL := make(map[string]string)
	byName := make(map[string][]string)

	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil // skip errors, continue walking
		}

		// We only care about directories named ".git"
		base := filepath.Base(path)
		if base != ".git" {
			return nil
		}

		// The repo path is the parent of .git/
		repoPath := filepath.Dir(path)

		// Read origin remote URL
		originURL, err := getRemoteURL(repoPath, "origin")
		if err != nil {
			// Skip repos without an origin remote (e.g. bare init, no remotes configured)
			return nil
		}

		normalized := NormalizeURL(originURL)
		byURL[normalized] = repoPath

		remoteURLs := map[string]string{"origin": originURL}

		// Index by directory name
		name := filepath.Base(repoPath)
		byName[name] = append(byName[name], repoPath)

		repos = append(repos, &LocalRepo{
			Path:       repoPath,
			RemoteURLs: remoteURLs,
		})

		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("walking %s: %w", root, err)
	}

	byPath := make(map[string]*LocalRepo, len(repos))
	for _, r := range repos {
		byPath[r.Path] = r
	}

	return &LocalRepos{
		Repos:  repos,
		ByURL:  byURL,
		ByPath: byPath,
		ByName: byName,
	}, nil
}

// FindByURL looks up a local repo by matching against clone URLs.
// It tries both the normalized URL and the raw URL.
func (lr *LocalRepos) FindByURL(url string) (string, bool) {
	normalized := NormalizeURL(url)
	if p, ok := lr.ByURL[normalized]; ok {
		return p, true
	}
	if p, ok := lr.ByURL[url]; ok {
		return p, true
	}
	return "", false
}

// getRemoteURL returns the URL of a git remote by running git remote get-url.
func getRemoteURL(repoPath, remote string) (string, error) {
	cmd := exec.Command("git", "remote", "get-url", remote)
	cmd.Dir = repoPath
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// NormalizeURL normalizes a git URL for comparison.
// It handles SSH vs HTTPS differences for the same repo.
//
// Examples:
//
//	git@github.com:owner/repo.git       → github.com/owner/repo
//	ssh://git@github.com/owner/repo.git → github.com/owner/repo
//	https://github.com/owner/repo.git   → github.com/owner/repo
//	https://github.com/owner/repo       → github.com/owner/repo
func NormalizeURL(url string) string {
	url = strings.TrimSpace(url)
	url = strings.TrimRight(url, "/")

	// SSH URL form: ssh://[user@]host/path.git
	if after, ok := strings.CutPrefix(url, "ssh://"); ok {
		if at := strings.Index(after, "@"); at != -1 {
			after = after[at+1:]
		}
		return strings.TrimSuffix(after, ".git")
	}

	// SCP-style SSH form: git@host:owner/repo.git
	if after, ok := strings.CutPrefix(url, "git@"); ok {
		normalized := strings.Replace(after, ":", "/", 1)
		return strings.TrimSuffix(normalized, ".git")
	}

	// HTTPS form
	if after, ok := strings.CutPrefix(url, "https://"); ok {
		return strings.TrimSuffix(after, ".git")
	}
	if after, ok := strings.CutPrefix(url, "http://"); ok {
		return strings.TrimSuffix(after, ".git")
	}

	// Fallback: return as-is
	return url
}
