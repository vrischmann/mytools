package syncplan

import (
	"path/filepath"

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
	baseDir := rulesBaseDir(repo, ws)

	if clash {
		return filepath.Join(baseDir, repo.Owner+"-"+repo.Name)
	}
	return filepath.Join(baseDir, repo.Name)
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
	// Detect name clashes: repos with the same Name+rules category
	// share the same target directory name.
	type nameKey struct {
		name    string
		baseDir string
	}
	nameCount := make(map[nameKey]int)
	for _, repo := range remoteRepos {
		baseDir := rulesBaseDir(repo, ws)
		nameCount[nameKey{repo.Name, baseDir}]++
	}

	var actions []Action

	for _, repo := range remoteRepos {
		baseDir := rulesBaseDir(repo, ws)
		clash := nameCount[nameKey{repo.Name, baseDir}] > 1
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
