package git

import (
	"fmt"
	"os/exec"
	"strings"
)

// Upstream identifies a branch on a configured remote.
type Upstream struct {
	Remote string
	Branch string
}

// WorktreeState records the checkout and tracking information that a prune
// was planned for.
type WorktreeState struct {
	Branch   string
	Commit   string   `json:",omitempty"`
	Upstream Upstream `json:",omitzero"`
}

// ReadWorktreeState requires a checked-out branch with a configured
// remote branch. Git can report the upstream even after fetch --prune has
// removed its local remote-tracking ref.
func ReadWorktreeState(worktreePath string) (WorktreeState, error) {
	branch, err := CurrentBranch(worktreePath)
	if err != nil {
		return WorktreeState{}, err
	}
	ref := "refs/heads/" + branch
	cmd := exec.Command("git", "for-each-ref",
		"--format=%(refname)%00%(objectname)%00%(upstream:remotename)%00%(upstream:remoteref)", "--", ref)
	cmd.Dir = worktreePath
	out, err := cmd.Output()
	if err != nil {
		return WorktreeState{}, fmt.Errorf("reading upstream of %s: %w", branch, err)
	}
	fields := strings.Split(strings.TrimSuffix(string(out), "\n"), "\x00")
	if len(fields) != 4 || fields[0] != ref || fields[1] == "" || fields[2] == "" || fields[2] == "." {
		return WorktreeState{}, fmt.Errorf("branch %s has no remote upstream", branch)
	}
	upstreamBranch, ok := strings.CutPrefix(fields[3], "refs/heads/")
	if !ok || upstreamBranch == "" {
		return WorktreeState{}, fmt.Errorf("upstream of %s is not a remote branch", branch)
	}
	return WorktreeState{
		Branch:   branch,
		Commit:   fields[1],
		Upstream: Upstream{Remote: fields[2], Branch: upstreamBranch},
	}, nil
}

// IsAncestor reports whether Git can prove that base contains commit.
func IsAncestor(path, commit, base string) bool {
	if base == "" {
		return false
	}
	cmd := exec.Command("git", "merge-base", "--is-ancestor", commit, base)
	cmd.Dir = path
	return cmd.Run() == nil
}
