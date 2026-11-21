package prune

import (
	"fmt"
	"io/fs"
	"os"
	"path/filepath"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
)

// OrphanRepo represents a local repo that has no matching upstream remote.
type OrphanRepo struct {
	Path string
	Name string
}

// PruneResult holds the outcome of pruning a single orphan repo.
type PruneResult struct {
	Name    string
	Path    string
	Success bool
	Message string
}

// ConfirmFunc is a callback for interactive confirmation prompts.
type ConfirmFunc func(prompt string) bool

// FindOrphans finds local repos that have no matching upstream remote.
// It excludes ignored repos, linked worktrees and ancestors of other checkouts.
func FindOrphans(local *discover.LocalRepos, remoteRepos []*remote.RemoteRepo, ws *config.Workspace) []*OrphanRepo {
	// Build a set of normalized remote URLs for fast lookup
	remoteURLs := make(map[string]struct{}, len(remoteRepos))
	for _, r := range remoteRepos {
		remoteURLs[discover.NormalizeURL(r.CloneURL)] = struct{}{}
	}

	ancestors := repoAncestors(local)
	var orphans []*OrphanRepo

	for _, repo := range local.Iter() {
		if repo.IsWorktree || ancestors[filepath.Clean(repo.Path)] {
			continue
		}

		name := filepath.Base(repo.Path)

		// Skip ignored repos.
		if ws.IsIgnored(name) {
			continue
		}

		hasMatch := false
		for _, url := range repo.RemoteURLs {
			if _, ok := remoteURLs[discover.NormalizeURL(url)]; ok {
				hasMatch = true
				break
			}
		}

		if !hasMatch {
			orphans = append(orphans, &OrphanRepo{
				Path: repo.Path,
				Name: name,
			})
		}
	}

	return orphans
}

func repoAncestors(local *discover.LocalRepos) map[string]bool {
	ancestors := make(map[string]bool)
	for _, repo := range local.Iter() {
		dir := filepath.Clean(repo.Path)
		for {
			parent := filepath.Dir(dir)
			if parent == dir || ancestors[parent] {
				break
			}
			ancestors[parent] = true
			dir = parent
		}
	}
	return ancestors
}

// A checkout may appear under an orphan after discovery, including during sync.
// Look for both .git directories and gitfiles immediately before removal.
func checkForNestedCheckout(root string) error {
	gitDir := filepath.Join(root, ".git")
	return filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if path == gitDir && entry.IsDir() {
			return fs.SkipDir
		}
		if entry.Name() == ".git" {
			return fmt.Errorf("found a checkout at %s", filepath.Dir(path))
		}
		return nil
	})
}

// PruneOrphans removes orphan repos. If dryRun is true, it only reports what
// would be done. If confirmFn is non-nil, it prompts for each orphan.
func PruneOrphans(orphans []*OrphanRepo, dryRun bool, confirmFn ConfirmFunc) []*PruneResult {
	var results []*PruneResult

	for _, orphan := range orphans {
		result := &PruneResult{Name: orphan.Name, Path: orphan.Path}
		results = append(results, result)

		if !dryRun && confirmFn != nil {
			prompt := fmt.Sprintf("Remove orphan repo %s (%s)?", orphan.Name, orphan.Path)
			if !confirmFn(prompt) {
				result.Success = true
				result.Message = "skipped (user declined)"
				continue
			}
		}

		if err := checkForNestedCheckout(orphan.Path); err != nil {
			result.Message = fmt.Sprintf("refusing to remove: %v", err)
			continue
		}
		if dryRun {
			result.Success = true
			result.Message = "would remove"
			continue
		}
		if err := os.RemoveAll(orphan.Path); err != nil {
			result.Message = fmt.Sprintf("failed to remove: %v", err)
			continue
		}
		result.Success = true
		result.Message = "removed"
	}

	return results
}
