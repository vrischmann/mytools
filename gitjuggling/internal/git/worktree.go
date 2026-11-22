package git

import (
	"crypto/sha256"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// Upstream identifies a branch on a configured remote.
type Upstream struct {
	Remote string
	Branch string
}

// WorktreeState records the checkout and tracking information that a prune
// was approved for. The registration ID survives moves of the main clone.
type WorktreeState struct {
	BranchState
	WorktreeID string `json:",omitempty"`
}

// BranchState identifies the local tip and the remote ref it tracks.
type BranchState struct {
	Branch   string
	Commit   string   `json:",omitempty"`
	Upstream Upstream `json:",omitzero"`
	// Bind the plan to the same fetch URL without persisting credentials.
	RemoteURLHash string `json:",omitempty"`
}

// ReadWorktreeState requires a registered linked checkout with a configured
// remote branch. Git can report the upstream even after fetch --prune has
// removed its local remote-tracking ref.
func ReadWorktreeState(mainPath, worktreePath string) (WorktreeState, error) {
	id, err := worktreeID(mainPath, worktreePath)
	if err != nil {
		return WorktreeState{}, err
	}
	branch, err := CurrentBranch(worktreePath)
	if err != nil {
		return WorktreeState{}, err
	}
	state, err := ReadBranchState(worktreePath, branch)
	if err != nil {
		return WorktreeState{}, err
	}
	if state.Upstream.Remote == "" {
		return WorktreeState{}, fmt.Errorf("branch %s has no remote upstream", branch)
	}
	return WorktreeState{BranchState: state, WorktreeID: id}, nil
}

// ReadBranchState reads a local branch independently of its checkout. It
// leaves Upstream empty for branches without a remote tracking branch and
// reports a missing local branch as os.ErrNotExist.
func ReadBranchState(path, branch string) (BranchState, error) {
	ref := "refs/heads/" + branch
	cmd := exec.Command("git", "for-each-ref",
		"--format=%(refname)%00%(objectname)%00%(upstream:remotename)%00%(upstream:remoteref)", "--", ref)
	cmd.Dir = path
	out, err := cmd.Output()
	if err != nil {
		return BranchState{}, fmt.Errorf("reading upstream of %s: %w", branch, err)
	}
	fields := strings.Split(strings.TrimSuffix(string(out), "\n"), "\x00")
	if fields[0] != ref {
		return BranchState{}, fmt.Errorf("local branch %s: %w", branch, os.ErrNotExist)
	}
	if len(fields) != 4 || fields[1] == "" {
		return BranchState{}, fmt.Errorf("incomplete ref information for branch %s", branch)
	}
	state := BranchState{Branch: branch, Commit: fields[1]}
	upstreamBranch, ok := strings.CutPrefix(fields[3], "refs/heads/")
	if !ok || upstreamBranch == "" || fields[2] == "" || fields[2] == "." {
		return state, nil
	}
	url, err := RemoteURL(path, fields[2])
	if err != nil {
		return BranchState{}, err
	}
	state.Upstream = Upstream{Remote: fields[2], Branch: upstreamBranch}
	state.RemoteURLHash = fmt.Sprintf("%x", sha256.Sum256([]byte(url)))
	return state, nil
}

func worktreeID(mainPath, worktreePath string) (string, error) {
	link, err := ReadGitfile(worktreePath)
	if err != nil {
		return "", err
	}
	adminDir := ResolvePath(worktreePath, link)
	mainWorktrees, err := os.Stat(filepath.Join(mainPath, ".git", "worktrees"))
	parent, statErr := os.Stat(filepath.Dir(adminDir))
	if err != nil || statErr != nil || !os.SameFile(parent, mainWorktrees) {
		return "", fmt.Errorf("worktree does not belong to %s", mainPath)
	}
	// Check the reverse link too. Git can discover a parent repository when
	// the directory at the planned path is no longer a checkout.
	data, err := os.ReadFile(filepath.Join(adminDir, "gitdir"))
	if err != nil {
		return "", fmt.Errorf("reading worktree back-link: %w", err)
	}
	registered, err := os.Stat(ResolvePath(adminDir, strings.TrimRight(string(data), "\r\n")))
	if err != nil {
		return "", fmt.Errorf("reading registered checkout: %w", err)
	}
	checkout, err := os.Stat(filepath.Join(worktreePath, ".git"))
	if err != nil || !os.SameFile(registered, checkout) {
		return "", fmt.Errorf("worktree is no longer registered at %s", worktreePath)
	}
	return filepath.Base(adminDir), nil
}

// ReadGitfile reads the gitdir recorded in a checkout's .git file. Relative
// links are returned as written so they can be resolved across a move.
func ReadGitfile(checkout string) (string, error) {
	data, err := os.ReadFile(filepath.Join(checkout, ".git"))
	if err != nil {
		return "", err
	}
	link, ok := strings.CutPrefix(strings.TrimRight(string(data), "\r\n"), "gitdir: ")
	if !ok || link == "" {
		return "", fmt.Errorf("invalid .git file in %s", checkout)
	}
	return link, nil
}

// ResolvePath resolves an absolute or relative Git link at its original location.
func ResolvePath(dir, link string) string {
	if filepath.IsAbs(link) {
		return filepath.Clean(link)
	}
	return filepath.Join(dir, link)
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
