package remote

import "strings"

// RemoteSource indicates where a remote repo originates from.
type RemoteSource int

const (
	SourceGitHub RemoteSource = iota
	SourceForgejo
)

// RemoteRepo represents a repository from a remote source (GitHub or Forgejo).
type RemoteRepo struct {
	Name       string
	Owner      string
	IsFork     bool
	IsArchived bool
	IsMirror   bool
	CloneURL   string
	Source     RemoteSource
}

// SourceLabel returns a short human-readable label for the repo's source.
func (r *RemoteRepo) SourceLabel() string {
	switch r.Source {
	case SourceGitHub:
		return "github"
	case SourceForgejo:
		return "forgejo"
	default:
		return "unknown"
	}
}

// DedupKey returns a lowercase (owner, name) tuple for deduplication.
func (r *RemoteRepo) DedupKey() [2]string {
	return [2]string{strings.ToLower(r.Owner), strings.ToLower(r.Name)}
}

// DedupRepos removes Forgejo repos that duplicate GitHub repos (same owner/name).
// It returns the modified github and forgejo slices.
func DedupRepos(github, forgejo []*RemoteRepo) ([]*RemoteRepo, []*RemoteRepo) {
	githubKeys := make(map[[2]string]struct{}, len(github))
	for _, r := range github {
		githubKeys[r.DedupKey()] = struct{}{}
	}

	var filtered []*RemoteRepo
	for _, r := range forgejo {
		if _, dup := githubKeys[r.DedupKey()]; !dup {
			filtered = append(filtered, r)
		}
	}

	return github, filtered
}
