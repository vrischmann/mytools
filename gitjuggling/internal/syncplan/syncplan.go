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
	Type         ActionType
	Repo         *remote.RemoteRepo
	LocalPath    string // set for ActionUpdate
	CurrentPath  string // set for ActionMove
	ExpectedPath string // set for ActionMove, ActionClone
}

// ClassifyRepo determines the expected local directory for a remote repo
// based on workspace rules.
//
// Priority:
//  1. is_fork → rules.Forks (if configured)
//  2. is_archived → rules.Archived (if configured)
//  3. default → rules.Base
//
// Final path: <basedir>/<owner>/<name>
func ClassifyRepo(repo *remote.RemoteRepo, ws *config.Workspace) string {
	var baseDir string
	switch {
	case repo.IsFork && ws.Rules.Forks != "":
		baseDir = ws.Rules.Forks
	case repo.IsArchived && ws.Rules.Archived != "":
		baseDir = ws.Rules.Archived
	default:
		baseDir = ws.Rules.Base
	}

	return filepath.Join(baseDir, repo.Owner, repo.Name)
}

// BuildPlan determines the action for each remote repo by matching against
// locally discovered repos.
func BuildPlan(remoteRepos []*remote.RemoteRepo, local *discover.LocalRepos, ws *config.Workspace) []Action {
	var actions []Action

	for _, repo := range remoteRepos {
		expectedPath := ClassifyRepo(repo, ws)

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
				Type:      ActionUpdate,
				Repo:      repo,
				LocalPath: localPath,
			})
		default:
			actions = append(actions, Action{
				Type:         ActionMove,
				Repo:         repo,
				CurrentPath:  localPath,
				ExpectedPath: expectedPath,
			})
		}
	}

	return actions
}
