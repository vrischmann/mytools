package execute

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
)

func moveClone(from, to, cloneURL string) error {
	_, err := os.Lstat(from)
	alreadyMoved := errors.Is(err, os.ErrNotExist)
	if err != nil && !alreadyMoved {
		return err
	}
	mainPath := from
	if alreadyMoved {
		// A previous run may have stopped after rename and before saving the
		// result. Repair the matching clone at its destination on resume.
		mainPath = to
	} else if _, err := os.Lstat(to); !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("move destination is occupied or inaccessible: %s", to)
	}
	if err := verifyMainClone(mainPath, cloneURL); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(to), 0o755); err != nil {
		return err
	}
	// Git records canonical paths even if discovery walked a symlinked parent.
	from, err = syncplan.CanonicalLocation(from)
	if err != nil {
		return err
	}
	to, err = syncplan.CanonicalLocation(to)
	if err != nil {
		return err
	}
	paths, err := movedWorktreePaths(from, to, alreadyMoved)
	if err != nil {
		return err
	}
	if alreadyMoved {
		return repairWorktrees(to, paths)
	}
	if err := os.Rename(from, to); err != nil {
		return err
	}
	if err := repairWorktrees(to, paths); err != nil {
		if undo := os.Rename(to, from); undo != nil {
			return fmt.Errorf("%w (clone is at %s, undoing the move failed: %v)", err, to, undo)
		}
		for i, path := range paths {
			paths[i] = syncplan.MovedPath(path, to, from)
		}
		return errors.Join(err, repairWorktrees(from, paths))
	}
	return nil
}

func verifyMainClone(path, cloneURL string) error {
	dir, err := os.Lstat(path)
	if err != nil || !dir.IsDir() {
		return fmt.Errorf("expected a clone directory at %s", path)
	}
	dotGit, err := os.Lstat(filepath.Join(path, ".git"))
	if err != nil || !dotGit.IsDir() {
		return fmt.Errorf("expected a main clone at %s", path)
	}
	origin, err := git.RemoteURL(path, "origin")
	if err != nil || discover.NormalizeURL(origin) != discover.NormalizeURL(cloneURL) {
		return fmt.Errorf("could not verify the planned clone at %s", path)
	}
	return nil
}

// Use Git's registrations, which also cover checkouts outside the workspace.
// A relative gitdir back-link must be resolved at its pre-move location until
// Git repairs it. A resumed move can contain both old and repaired links.
func movedWorktreePaths(from, to string, alreadyMoved bool) ([]string, error) {
	mainPath := from
	if alreadyMoved {
		mainPath = to
	}
	adminRoot := filepath.Join(mainPath, ".git", "worktrees")
	entries, err := os.ReadDir(adminRoot)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var paths []string
	for _, entry := range entries {
		admin := filepath.Join(adminRoot, entry.Name())
		data, err := os.ReadFile(filepath.Join(admin, "gitdir"))
		if err != nil {
			return nil, err
		}
		backlink := strings.TrimRight(string(data), "\r\n")
		oldAdmin := filepath.Join(from, ".git", "worktrees", entry.Name())
		oldCheckout := filepath.Dir(git.ResolvePath(oldAdmin, backlink))
		target := syncplan.MovedPath(oldCheckout, from, to)
		checkout := oldCheckout
		if alreadyMoved {
			current := filepath.Dir(git.ResolvePath(admin, backlink))
			if link, err := git.ReadGitfile(current); err == nil {
				if git.ResolvePath(current, link) == admin {
					continue // both links have already been repaired
				}
				// The back-link can reach the new checkout even if repair
				// stopped before updating that checkout's .git file.
				before := syncplan.MovedPath(current, to, from)
				if git.ResolvePath(before, link) == oldAdmin {
					paths = append(paths, current)
					continue
				}
			}
			checkout = target
		}
		link, err := git.ReadGitfile(checkout)
		if err != nil {
			return nil, fmt.Errorf("checking linked worktree at %s: %w", checkout, err)
		}
		if git.ResolvePath(oldCheckout, link) != oldAdmin && git.ResolvePath(checkout, link) != admin {
			return nil, fmt.Errorf("worktree registration no longer matches %s", checkout)
		}
		paths = append(paths, target)
	}
	return paths, nil
}

func repairWorktrees(mainPath string, paths []string) error {
	if len(paths) == 0 {
		return nil
	}
	cmd := exec.Command("git", append([]string{"worktree", "repair", "--"}, paths...)...)
	cmd.Dir = mainPath
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("git worktree repair in %s: %w: %s", mainPath, err, strings.TrimSpace(string(output)))
	}
	return nil
}
