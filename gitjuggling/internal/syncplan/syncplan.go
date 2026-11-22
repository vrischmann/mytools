package syncplan

import (
	"cmp"
	"path/filepath"
	"regexp"
	"slices"
	"strconv"

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
	git.WorktreeState
	// Merged records whether the commit was merged into the tracking
	// remote's default branch when the plan was built.
	Merged bool
	// Set after sync removes the worktree and before trying branch deletion.
	BranchCleanupPending bool `json:",omitempty"`
}

// MovedPath returns the new location of a path carried along by a directory move.
func MovedPath(path, from, to string) string {
	if path == "" {
		return path
	}
	rel, err := filepath.Rel(from, path)
	if err != nil || !filepath.IsLocal(rel) {
		return path
	}
	return filepath.Join(to, rel)
}

// Relocate updates the source paths in pending actions after a successful move.
// ExpectedPath is the configured destination and stays independent of the move.
func Relocate(actions []Action, from, to string) {
	// Discovery uses the workspace's spelling of a path. Git can record the
	// same main clone with an absolute path through its physical parent.
	physical, _ := CanonicalLocation(from)
	relocate := func(path string) string {
		target := MovedPath(path, from, to)
		if target == path && physical != "" && physical != from {
			return MovedPath(path, physical, to)
		}
		return target
	}
	for i := range actions {
		a := &actions[i]
		a.CurrentPath = relocate(a.CurrentPath)
		a.LocalPath = relocate(a.LocalPath)
		a.MainRepoPath = relocate(a.MainRepoPath)
		a.WorktreePath = relocate(a.WorktreePath)
	}
}

// CanonicalLocation resolves the parent of a checkout even when the checkout
// itself has already moved or its destination has yet to be created.
func CanonicalLocation(path string) (string, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	parent, err := filepath.EvalSymlinks(filepath.Dir(abs))
	if err != nil {
		return "", err
	}
	return filepath.Join(parent, filepath.Base(abs)), nil
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
	return withOwner(path, repo.Owner)
}

func withOwner(path, owner string) string {
	return filepath.Join(filepath.Dir(path), owner+"-"+filepath.Base(path))
}

// rawExpectedPath returns the target directory for a repo before clash
// disambiguation. Pattern rules take precedence over the fork/archived/base
// category rules: the first matching pattern's expanded template wins, and if
// no pattern matches the repo is placed under its category base directory
// using its name.
func rawExpectedPath(repo *remote.RemoteRepo, ws *config.Workspace) string {
	path, ok := matchPattern(ws, repo.Name)
	if !ok {
		path = filepath.Join(rulesBaseDir(repo, ws), repo.Name)
	}
	if !filepath.IsAbs(path) {
		return filepath.Join(ws.Root, path)
	}
	return filepath.Clean(path)
}

// Reserve the natural destinations first, then allocate owner-prefixed
// names. If those also collide, use the next free numbered suffix.
func destinationPaths(repos []*remote.RemoteRepo, ws *config.Workspace) []string {
	paths := make([]string, len(repos))
	counts := make(map[string]int)
	for i, repo := range repos {
		paths[i] = ClassifyRepo(repo, ws, false)
		counts[paths[i]]++
	}

	claims := make(map[string][]int)
	var collisions []int
	for i, path := range paths {
		if counts[path] == 1 {
			claims[path] = []int{i}
		} else {
			collisions = append(collisions, i)
		}
	}
	// Resolve ties consistently across different inventory orders.
	slices.SortFunc(collisions, func(i, j int) int {
		a, b := repos[i], repos[j]
		return cmp.Or(cmp.Compare(a.Owner, b.Owner), cmp.Compare(a.Name, b.Name), cmp.Compare(a.CloneURL, b.CloneURL))
	})
	for _, i := range collisions {
		paths[i] = withOwner(paths[i], repos[i].Owner)
		claims[paths[i]] = append(claims[paths[i]], i)
	}
	for path, indices := range claims {
		suffix := 2
		for _, i := range indices[1:] {
			for {
				candidate := path + "-" + strconv.Itoa(suffix)
				suffix++
				if _, reserved := claims[candidate]; !reserved {
					paths[i] = candidate
					break
				}
			}
		}
	}
	return paths
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

	var actions []Action
	paths := destinationPaths(remoteRepos, ws)
	for i, repo := range remoteRepos {
		expectedPath := paths[i]

		localPath, found := local.FindByURL(repo.CloneURL, expectedPath)
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

		state, err := git.ReadWorktreeState(wt.MainPath, wt.Path)
		if err != nil {
			continue
		}

		exists, err := branchExists(wt.Path, state.Upstream.Remote, state.Upstream.Branch)
		if err != nil || exists {
			continue // only prune when we can confirm the upstream is gone
		}

		actions = append(actions, Action{
			Type:          ActionPruneWorktree,
			Repo:          repo,
			WorktreePath:  wt.Path,
			MainRepoPath:  wt.MainPath,
			WorktreeState: state,
			Merged:        git.IsAncestor(wt.Path, state.Commit, "refs/remotes/"+state.Upstream.Remote+"/HEAD"),
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
