package discover

import (
	"fmt"
	"io/fs"
	"iter"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// LocalRepo represents a locally discovered git repository.
type LocalRepo struct {
	Path       string
	RemoteURLs map[string]string // remote name → URL
	// IsWorktree marks a linked worktree of another clone. Worktrees share
	// their origin with the main clone, so they are excluded from the URL
	// index and must never be treated as an independent repo.
	IsWorktree bool
	// MainPath is the path of the main clone a worktree belongs to.
	// Only set when IsWorktree is true.
	MainPath string
}

// LocalRepos holds all discovered local repos with lookup indices.
type LocalRepos struct {
	repos     []LocalRepo
	worktrees []*LocalRepo
	byURL     map[string]string     // normalized URL → local path (main clones only)
	byPath    map[string]*LocalRepo // absolute path → LocalRepo (main clones only)
	byName    map[string][]string   // dir name → list of local paths (main clones only)
}

// NewLocalRepos builds a LocalRepos from a flat list of repos,
// constructing all lookup indices.
func NewLocalRepos(repos []LocalRepo) *LocalRepos {
	byURL := make(map[string]string, len(repos))
	byPath := make(map[string]*LocalRepo, len(repos))
	byName := make(map[string][]string)
	var worktrees []*LocalRepo

	for i := range repos {
		r := &repos[i]
		if r.IsWorktree {
			// Worktrees share their origin URL with the main clone. Indexing
			// them would let the last-discovered worktree shadow the main
			// clone in the URL lookup, so keep them out of all indexes.
			worktrees = append(worktrees, r)
			continue
		}
		for _, url := range r.RemoteURLs {
			normalized := NormalizeURL(url)
			byURL[normalized] = r.Path
		}
		byPath[r.Path] = r
		name := filepath.Base(r.Path)
		byName[name] = append(byName[name], r.Path)
	}

	return &LocalRepos{
		repos:     repos,
		worktrees: worktrees,
		byURL:     byURL,
		byPath:    byPath,
		byName:    byName,
	}
}

// Iter returns an iterator over all discovered local repos,
// including worktrees.
func (lr *LocalRepos) Iter() iter.Seq2[int, *LocalRepo] {
	return func(yield func(int, *LocalRepo) bool) {
		for i := range lr.repos {
			if !yield(i, &lr.repos[i]) {
				return
			}
		}
	}
}

// Worktrees returns the linked worktrees found during discovery.
func (lr *LocalRepos) Worktrees() []*LocalRepo {
	return lr.worktrees
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
// .git entries. A .git directory is a repo (or the main clone of worktrees);
// a .git file is either a linked worktree or a submodule, told apart by
// where their gitdir points. For each entry it reads the origin remote URL.
func Discover(root string) (*LocalRepos, error) {
	var repos []LocalRepo

	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil // skip errors, continue walking
		}

		// We only care about entries named ".git" (directory or file)
		base := filepath.Base(path)
		if base != ".git" {
			return nil
		}

		// The repo path is the parent of .git
		repoPath := filepath.Dir(path)

		var repo LocalRepo

		if d.IsDir() {
			// Regular repo or the main clone of linked worktrees.
			originURL, err := getRemoteURL(repoPath, "origin")
			if err != nil {
				// Skip repos without an origin remote (e.g. bare init, no remotes configured)
				return nil
			}
			repo = LocalRepo{
				Path:       repoPath,
				RemoteURLs: map[string]string{"origin": originURL},
			}
		} else {
			// A .git file is a linked worktree or a submodule checkout.
			mainPath, ok := worktreeMainPath(path, repoPath)
			if !ok {
				return nil // submodule or unrecognized; not a workspace repo
			}
			originURL, err := getRemoteURL(repoPath, "origin")
			if err != nil {
				return nil
			}
			repo = LocalRepo{
				Path:       repoPath,
				RemoteURLs: map[string]string{"origin": originURL},
				IsWorktree: true,
				MainPath:   mainPath,
			}
		}

		repos = append(repos, repo)

		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("walking %s: %w", root, err)
	}

	return NewLocalRepos(repos), nil
}

// worktreeMainPath inspects the .git file at gitFilePath (whose parent
// directory is worktreePath) and reports the path of the main clone when the
// file points at that clone's worktree metadata (<main>/.git/worktrees/<name>).
// It returns false for anything else, notably submodule checkouts whose
// gitdir lives under <parent>/.git/modules/.
func worktreeMainPath(gitFilePath, worktreePath string) (string, bool) {
	data, err := os.ReadFile(gitFilePath)
	if err != nil {
		return "", false
	}
	content := strings.TrimSpace(string(data))
	gitdir, ok := strings.CutPrefix(content, "gitdir:")
	if !ok {
		return "", false
	}
	gitdir = strings.TrimSpace(gitdir)
	if !filepath.IsAbs(gitdir) {
		gitdir = filepath.Join(worktreePath, gitdir)
	}
	gitdir = filepath.Clean(gitdir)

	// Shape check: <main>/.git/worktrees/<name>
	nameDir := filepath.Dir(gitdir)
	if filepath.Base(nameDir) != "worktrees" {
		return "", false
	}
	dotGitDir := filepath.Dir(nameDir)
	if filepath.Base(dotGitDir) != ".git" {
		return "", false
	}
	return filepath.Dir(dotGitDir), true
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
