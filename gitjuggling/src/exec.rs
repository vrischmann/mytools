//! Mutating git operations — first slice of the `internal/execute` port.
//!
//! M2 carries only the worktree-prune action and the tracking-error
//! classifiers; the bounded-concurrency runner, dry-run handling and the
//! GitHub askpass helper land with M4 (RUST_REWRITE_PLAN.md §5).
//!
//! §4.10 semantics preserved verbatim:
//!
//! * [`is_no_tracking_error`] / [`is_stale_upstream_error`] are **substring**
//!   matches on git's combined output, with the exact Go strings — they drive
//!   the tracking-fix phase.
//! * [`remove_worktree_and_branch`] deletes the branch only when the plan
//!   verified it as merged **at plan time** (the `Merged` flag → the
//!   `delete_branch` argument); an unmerged branch is kept and the action is
//!   still reported as success. Removal is `git worktree remove` **then**
//!   `git branch -d`, both run in the main repo.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

/// Holds the outcome of executing a single action (Go: `execute.ActionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResult {
    pub description: String,
    pub path: String,
    pub success: bool,
    pub message: String,
}

/// Returns true if the error message indicates a missing upstream/tracking
/// branch.
///
/// Port of `execute.IsNoTrackingError` (execute.go:26); both Go conditions are
/// kept verbatim even though the first subsumes the second — §4.10 pins the
/// exact substring set ("There is no tracking information" contains "no
/// tracking information", so the second check is redundant).
pub fn is_no_tracking_error(msg: &str) -> bool {
    msg.contains("no tracking information") || msg.contains("There is no tracking information")
}

/// Returns true if the error message indicates that the configured upstream
/// ref no longer exists on the remote.
///
/// Port of `execute.IsStaleUpstreamError` (execute.go:33).
pub fn is_stale_upstream_error(msg: &str) -> bool {
    msg.contains("no such ref was fetched")
}

/// Removes a linked worktree whose branch is gone from the remote. When
/// `delete_branch` is true it also deletes the local branch. Callers pass
/// `delete_branch` only when the branch was verified as merged at plan time,
/// so a `branch -d` failure here is a genuine error and is reported as such.
/// git also refuses to remove a dirty worktree.
///
/// Port of `execute.RemoveWorktreeAndBranch` (execute.go:68).
pub fn remove_worktree_and_branch(
    worktree_path: &Path,
    main_repo_path: &Path,
    branch: &str,
    delete_branch: bool,
) -> ActionResult {
    let desc = format!(
        "{}/{}",
        path_base(worktree_path.parent().unwrap_or(worktree_path)),
        path_base(worktree_path),
    );
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: worktree_path.to_string_lossy().into_owned(),
        success,
        message,
    };

    let output = match run_git_combined(
        main_repo_path,
        &["worktree", "remove"],
        &[worktree_path.as_os_str()],
    ) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git worktree remove failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git worktree remove failed: {}", combined_trimmed(&output)),
        );
    }

    if !delete_branch {
        return result(
            true,
            format!("removed stale worktree (kept branch {branch} — not fully merged)"),
        );
    }

    let output = match run_git_combined(main_repo_path, &["branch", "-d"], &[OsStr::new(branch)]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git branch -d {branch} failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!(
                "git branch -d {branch} failed: {}",
                combined_trimmed(&output)
            ),
        );
    }

    result(
        true,
        format!("removed stale worktree and merged branch {branch}"),
    )
}

/// Runs `git` with fixed `args` plus extra `operands` in `dir`, collecting the
/// equivalent of Go's `CombinedOutput`. Rust has no interleaving pipe, so
/// stdout and stderr are concatenated (stdout first); every git message the
/// §4.10 contracts match lives wholly in one stream, so substring matching and
/// the trimmed failure messages are unaffected.
fn run_git_combined(dir: &Path, args: &[&str], operands: &[&OsStr]) -> std::io::Result<Output> {
    Command::new("git")
        .args(args)
        .args(operands)
        .current_dir(dir)
        .output()
}

/// Go's `CombinedOutput` equivalent: stdout then stderr, lossily decoded.
fn combined_trimmed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
    .trim()
    .to_owned()
}

/// Go's `filepath.Base` for display strings (falls back to the whole path).
fn path_base(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    // go test-run: internal/execute/execute_test.go TestIsNoTrackingError
    #[test]
    fn test_is_no_tracking_error() {
        let tests: &[(&str, &str, bool)] = &[
            (
                "git pull no tracking information",
                "git pull --rebase failed: There is no tracking information for the current branch.\nPlease specify which branch you want to rebase against.",
                true,
            ),
            (
                "lowercase variant",
                "git pull --rebase failed: fatal: no tracking information for the current branch",
                true,
            ),
            (
                "unrelated git error",
                "git pull --rebase failed: error: could not apply abc1234..def5678",
                false,
            ),
            (
                "stash error",
                "git stash failed: error: could not stash",
                false,
            ),
            ("empty message", "", false),
        ];
        for &(name, msg, want) in tests {
            let got = is_no_tracking_error(msg);
            assert_eq!(
                got, want,
                "subtest {name:?}: IsNoTrackingError({msg:?}) = {got}, want {want}"
            );
        }
    }

    // go test-run: internal/execute/execute_test.go TestRemoveWorktreeAndBranchMergedDeletesBranch
    #[test]
    fn test_remove_worktree_and_branch_merged_deletes_branch() {
        let root = tempfile::tempdir().expect("tempdir");
        let main_path = root.path().join("demo");
        let wt_path = root.path().join("demo-wt");

        fixtures::repo(&main_path, "ssh://git@example.com/vincent/demo.git");
        fixtures::worktree(&main_path, &wt_path, "feature", false);

        let result = remove_worktree_and_branch(&wt_path, &main_path, "feature", true);
        assert!(result.success, "result: {}", result.message);
        assert!(!wt_path.exists(), "worktree dir should be gone");

        let out = branch_list(&main_path, "feature");
        assert!(
            out.trim().is_empty(),
            "merged branch should be deleted, got {out:?}"
        );
    }

    // go test-run: internal/execute/execute_test.go TestRemoveWorktreeAndBranchKeepsUnmergedBranch
    #[test]
    fn test_remove_worktree_and_branch_keeps_unmerged_branch() {
        let root = tempfile::tempdir().expect("tempdir");
        let main_path = root.path().join("demo");
        let wt_path = root.path().join("demo-wt");

        fixtures::repo(&main_path, "ssh://git@example.com/vincent/demo.git");
        fixtures::worktree(&main_path, &wt_path, "feature", true); // diverged branch

        // The plan flags a diverged branch as not merged, so delete_branch is
        // false: the worktree goes away, the branch stays for manual inspection.
        let result = remove_worktree_and_branch(&wt_path, &main_path, "feature", false);
        assert!(result.success, "result: {}", result.message);
        assert!(result.message.contains("kept branch feature"));
        assert!(!wt_path.exists(), "worktree dir should be gone");

        let out = branch_list(&main_path, "feature");
        assert!(
            out.contains("feature"),
            "unmerged branch must be kept, got {out:?}"
        );
    }

    // go test-run: internal/execute/execute_test.go TestRemoveWorktreeAndBranchWithoutDelete
    #[test]
    fn test_remove_worktree_and_branch_without_delete() {
        let root = tempfile::tempdir().expect("tempdir");
        let main_path = root.path().join("demo");
        let wt_path = root.path().join("demo-wt");

        fixtures::repo(&main_path, "ssh://git@example.com/vincent/demo.git");
        fixtures::worktree(&main_path, &wt_path, "feature", false);

        let result = remove_worktree_and_branch(&wt_path, &main_path, "feature", false);
        assert!(result.success, "result: {}", result.message);
        assert!(!wt_path.exists(), "worktree dir should be gone");

        let out = branch_list(&main_path, "feature");
        assert!(
            out.contains("feature"),
            "branch should be kept when delete_branch is false, got {out:?}"
        );
    }

    /// Mirror of the Go tests' probe
    /// `exec.Command("git", "-C", mainPath, "branch", "--list", name).Output()`.
    fn branch_list(main_path: &Path, branch: &str) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(main_path)
            .args(["branch", "--list", branch])
            .output()
            .expect("git branch --list");
        assert!(output.status.success(), "git branch --list failed");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}
