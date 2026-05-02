package prune

import (
	"fmt"
	"os"
	"path/filepath"

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
func FindOrphans(local *discover.LocalRepos, remoteRepos []*remote.RemoteRepo) []*OrphanRepo {
	// Build a set of normalized remote URLs for fast lookup
	remoteURLs := make(map[string]struct{}, len(remoteRepos))
	for _, r := range remoteRepos {
		remoteURLs[discover.NormalizeURL(r.CloneURL)] = struct{}{}
	}

	var orphans []*OrphanRepo

	for _, repo := range local.Repos {
		hasMatch := false
		for _, url := range repo.RemoteURLs {
			if _, ok := remoteURLs[discover.NormalizeURL(url)]; ok {
				hasMatch = true
				break
			}
		}

		if !hasMatch {
			name := filepath.Base(repo.Path)
			orphans = append(orphans, &OrphanRepo{
				Path: repo.Path,
				Name: name,
			})
		}
	}

	return orphans
}

// PruneOrphans removes orphan repos. If dryRun is true, it only reports what
// would be done. If confirmFn is non-nil, it prompts for each orphan.
func PruneOrphans(orphans []*OrphanRepo, dryRun bool, confirmFn ConfirmFunc) []*PruneResult {
	var results []*PruneResult

	for _, orphan := range orphans {
		if dryRun {
			results = append(results, &PruneResult{
				Name:    orphan.Name,
				Path:    orphan.Path,
				Success: true,
				Message: "would remove",
			})
			continue
		}

		if confirmFn != nil {
			prompt := fmt.Sprintf("Remove orphan repo %s (%s)?", orphan.Name, orphan.Path)
			if !confirmFn(prompt) {
				results = append(results, &PruneResult{
					Name:    orphan.Name,
					Path:    orphan.Path,
					Success: true,
					Message: "skipped (user declined)",
				})
				continue
			}
		}

		if err := os.RemoveAll(orphan.Path); err != nil {
			results = append(results, &PruneResult{
				Name:    orphan.Name,
				Path:    orphan.Path,
				Success: false,
				Message: fmt.Sprintf("failed to remove: %v", err),
			})
		} else {
			results = append(results, &PruneResult{
				Name:    orphan.Name,
				Path:    orphan.Path,
				Success: true,
				Message: "removed",
			})
		}
	}

	return results
}
