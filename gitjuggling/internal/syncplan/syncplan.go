package syncplan

import (
	"path/filepath"
	"regexp"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
)

// ActionType classifies the kind of sync action needed.
type ActionType int

const (
	ActionUpdate ActionType = iota
	ActionMove
	ActionClone
	// ActionPruneWorktree removes a linked worktree whose checked-out branch
	// no longer exists on the remote, then deletes the merged local branch.
	ActionPruneWorktree
)

// Action represents a planned action for a single remote repo.
type Action struct {
	Type           ActionType
	Repo           *remote.RemoteRepo
	LocalPath      string // set for ActionUpdate
	CurrentPath    string // set for ActionMove
	ExpectedPath   string // set for ActionMove, ActionClone
	AlreadyInPlace bool   // set for ActionUpdate when LocalPath == ExpectedPath

	// Worktree fields, set for ActionPruneWorktree.
	WorktreePath string // directory of the stale worktree
	MainRepoPath string // main clone the worktree belongs to
	Branch       string // checked-out branch whose upstream is gone
	// Merged reports whether Branch is merged into the default branch by
	// ancestry (git branch --merged). When false the branch is kept after
	// removing the worktree: its commits may be squash-merged or unmerged,
	// and git branch -d would refuse anyway.
	Merged bool
}

// BranchExistsFunc reports whether branch exists on remoteName for the
// repository at localPath. Implementations typically shell out to
// git ls-remote, so this performs a network round trip.
type BranchExistsFunc func(localPath, remoteName, branch string) (bool, error)

// ClassifyRepo determines the expected local directory for a remote repo
// based on workspace rules.
//
// Priority:
//  1. is_fork → rules.Forks (if configured)
//  2. is_archived → rules.Archived (if configured)
//  3. default → rules.Base
//
// If clash is true, the final path is <basedir>/<owner>-<name>.
// Otherwise, the final path is <basedir>/<name>.
func ClassifyRepo(repo *remote.RemoteRepo, ws *config.Workspace, clash bool) string {
	path := rawExpectedPath(repo, ws)
	if !clash {
		return path
	}
	// Disambiguate colliding repos by prefixing the owner onto the final
	// path component.
	dir, name := filepath.Split(path)
	return filepath.Join(dir, repo.Owner+"-"+name)
}

// rawExpectedPath returns the target directory for a repo before clash
// disambiguation. Pattern rules take precedence over the fork/archived/base
// category rules: the first matching pattern's expanded template wins, and if
// no pattern matches the repo is placed under its category base directory
// using its name.
func rawExpectedPath(repo *remote.RemoteRepo, ws *config.Workspace) string {
	if dir, ok := matchPattern(ws, repo.Name); ok {
		return dir
	}
	return filepath.Join(rulesBaseDir(repo, ws), repo.Name)
}

// matchPattern evaluates the workspace pattern rules in order against name
// and returns the expanded target directory of the first match. The To
// template is expanded with regexp.ExpandString, so $1, ${name}, and $0 all
// work.
func matchPattern(ws *config.Workspace, name string) (string, bool) {
	for _, rule := range ws.Rules.Patterns {
		re, err := regexp.Compile(rule.Pattern)
		if err != nil {
			continue // config validation rejects this; stay defensive.
		}
		match := re.FindStringSubmatchIndex(name)
		if match == nil {
			continue
		}
		expanded := re.ExpandString(nil, rule.To, name, match)
		return string(expanded), true
	}
	return "", false
}

// rulesBaseDir returns the base directory for a repo based on workspace rules.
func rulesBaseDir(repo *remote.RemoteRepo, ws *config.Workspace) string {
	switch {
	case repo.IsFork && ws.Rules.Forks != "":
		return ws.Rules.Forks
	case repo.IsArchived && ws.Rules.Archived != "":
		return ws.Rules.Archived
	default:
		return ws.Rules.Base
	}
}

// BuildPlan determines the action for each remote repo by matching against
// locally discovered repos. branchExists performs the live upstream check for
// worktree pruning; when it is nil, no prune actions are proposed.
func BuildPlan(remoteRepos []*remote.RemoteRepo, local *discover.LocalRepos, ws *config.Workspace, branchExists BranchExistsFunc) []Action {
	// Filter out ignored repos.
	var filtered []*remote.RemoteRepo
	for _, repo := range remoteRepos {
		if !ws.IsIgnored(repo.Name) {
			filtered = append(filtered, repo)
		}
	}
	remoteRepos = filtered

	// Detect collisions: repos that map to the same target directory.
	pathCount := make(map[string]int)
	for _, repo := range remoteRepos {
		pathCount[rawExpectedPath(repo, ws)]++
	}

	var actions []Action

	for _, repo := range remoteRepos {
		clash := pathCount[rawExpectedPath(repo, ws)] > 1
		expectedPath := ClassifyRepo(repo, ws, clash)

		localPath, found := local.FindByURL(repo.CloneURL)
		switch {
		case !found:
			actions = append(actions, Action{
				Type:         ActionClone,
				Repo:         repo,
				ExpectedPath: expectedPath,
			})
		case localPath == expectedPath:
			actions = append(actions, Action{
				Type:           ActionUpdate,
				Repo:           repo,
				LocalPath:      localPath,
				ExpectedPath:   expectedPath,
				AlreadyInPlace: true,
			})
		default:
			// If the expected path is already occupied by a different
			// local repo, just update in place instead of moving.
			if _, occupied := local.FindByPath(expectedPath); occupied {
				actions = append(actions, Action{
					Type:         ActionUpdate,
					Repo:         repo,
					LocalPath:    localPath,
					ExpectedPath: expectedPath,
				})
			} else {
				actions = append(actions, Action{
					Type:         ActionMove,
					Repo:         repo,
					CurrentPath:  localPath,
					ExpectedPath: expectedPath,
				})
			}
		}
	}

	actions = append(actions, worktreePruneActions(remoteRepos, local, branchExists)...)

	return actions
}

// worktreePruneActions proposes removing linked worktrees whose checked-out
// branch no longer exists upstream. Removing the worktree only deletes the
// checkout — the branch and its commits stay in the main clone — so the
// proposal does not depend on merge state. The branch itself is deleted only
// when it is merged into the default branch; otherwise it is kept (its
// commits may be squash-merged or genuinely unmerged). Only worktrees of
// repos known to the workspace are considered.
func worktreePruneActions(remoteRepos []*remote.RemoteRepo, local *discover.LocalRepos, branchExists BranchExistsFunc) []Action {
	if branchExists == nil {
		return nil
	}

	var actions []Action
	for _, wt := range local.Worktrees() {
		repo := matchRemoteRepo(remoteRepos, wt)
		if repo == nil {
			continue
		}

		branch, err := git.CurrentBranch(wt.Path)
		if err != nil {
			continue // detached HEAD or unreadable repo; leave it alone
		}

		exists, err := branchExists(wt.Path, "origin", branch)
		if err != nil || exists {
			continue // only prune when we can confirm the upstream is gone
		}

		actions = append(actions, Action{
			Type:         ActionPruneWorktree,
			Repo:         repo,
			WorktreePath: wt.Path,
			MainRepoPath: wt.MainPath,
			Branch:       branch,
			Merged:       worktreeBranchMerged(wt, branch),
		})
	}
	return actions
}

// matchRemoteRepo finds the remote repo whose clone URL matches the origin
// of the given discovered repo.
func matchRemoteRepo(remoteRepos []*remote.RemoteRepo, repo *discover.LocalRepo) *remote.RemoteRepo {
	for _, r := range remoteRepos {
		for _, url := range repo.RemoteURLs {
			if discover.NormalizeURL(url) == discover.NormalizeURL(r.CloneURL) {
				return r
			}
		}
	}
	return nil
}

// worktreeBranchMerged reports whether the worktree's branch has been merged
// into the default branch of its origin. Any error (missing origin/HEAD,
// missing remote-tracking ref, git failure) counts as not merged so the
// branch is kept on the safe side.
func worktreeBranchMerged(wt *discover.LocalRepo, branch string) bool {
	defaultBranch, err := git.DefaultBranch(wt.Path, "origin")
	if err != nil {
		return false
	}
	merged, err := git.IsBranchMerged(wt.Path, branch, defaultBranch)
	if err != nil {
		return false
	}
	return merged
}
