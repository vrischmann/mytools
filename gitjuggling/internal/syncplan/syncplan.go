package syncplan

import (
	"path/filepath"
	"regexp"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
)

// ActionType classifies the kind of sync action needed.
type ActionType int

const (
	ActionUpdate ActionType = iota
	ActionMove
	ActionClone
)

// Action represents a planned action for a single remote repo.
type Action struct {
	Type           ActionType
	Repo           *remote.RemoteRepo
	LocalPath      string // set for ActionUpdate
	CurrentPath    string // set for ActionMove
	ExpectedPath   string // set for ActionMove, ActionClone
	AlreadyInPlace bool   // set for ActionUpdate when LocalPath == ExpectedPath
}

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
// locally discovered repos.
func BuildPlan(remoteRepos []*remote.RemoteRepo, local *discover.LocalRepos, ws *config.Workspace) []Action {
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

	return actions
}
