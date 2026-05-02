package discover

import (
	"fmt"
	"io/fs"
	"iter"
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
	repos  []LocalRepo
	byURL  map[string]string     // normalized URL → local path
	byPath map[string]*LocalRepo // absolute path → LocalRepo
	byName map[string][]string   // dir name → list of local paths
}

// NewLocalRepos builds a LocalRepos from a flat list of repos,
// constructing all lookup indices.
func NewLocalRepos(repos []LocalRepo) *LocalRepos {
	byURL := make(map[string]string, len(repos))
	byPath := make(map[string]*LocalRepo, len(repos))
	byName := make(map[string][]string)

	for i := range repos {
		r := &repos[i]
		for _, url := range r.RemoteURLs {
			normalized := NormalizeURL(url)
			byURL[normalized] = r.Path
		}
		byPath[r.Path] = r
		name := filepath.Base(r.Path)
		byName[name] = append(byName[name], r.Path)
	}

	return &LocalRepos{
		repos:  repos,
		byURL:  byURL,
		byPath: byPath,
		byName: byName,
	}
}

// Iter returns an iterator over all discovered local repos.
func (lr *LocalRepos) Iter() iter.Seq2[int, *LocalRepo] {
	return func(yield func(int, *LocalRepo) bool) {
		for i := range lr.repos {
			if !yield(i, &lr.repos[i]) {
				return
			}
		}
	}
}

// FindByURL looks up a local repo by matching against clone URLs.
// It tries both the normalized URL and the raw URL.
func (lr *LocalRepos) FindByURL(url string) (string, bool) {
	normalized := NormalizeURL(url)
	if p, ok := lr.byURL[normalized]; ok {
		return p, true
	}
	if p, ok := lr.byURL[url]; ok {
		return p, true
	}
	return "", false
}

// FindByPath looks up a local repo by its absolute path.
func (lr *LocalRepos) FindByPath(path string) (*LocalRepo, bool) {
	r, ok := lr.byPath[path]
	return r, ok
}

// Discover walks the filesystem under root, finding git repos by looking for
// .git directories. For each repo it reads the origin remote URL.
func Discover(root string) (*LocalRepos, error) {
	var repos []LocalRepo

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

		repos = append(repos, LocalRepo{
			Path:       repoPath,
			RemoteURLs: map[string]string{"origin": originURL},
		})

		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("walking %s: %w", root, err)
	}

	return NewLocalRepos(repos), nil
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
