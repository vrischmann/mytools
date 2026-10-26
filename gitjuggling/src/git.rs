//! Read-only git helpers (port of `internal/git/git.go`).
//!
//! Decision G (RUST_REWRITE_PLAN.md §7): every git operation is a `git`
//! subprocess via [`std::process::Command`] — no git2 — so stderr wording and
//! the §4.10 substring contracts carry over for free. The port mirrors Go's
//! `cmd.Output()` semantics: stdout is captured, stderr is discarded, and a
//! non-zero exit becomes `<prefix>: exit status N` (Go's `exec.ExitError`
//! wording, which the `… failed: …` error prefixes below rely on).

use std::path::Path;
use std::process::{Command, ExitStatus};

use anyhow::{Result, anyhow, bail};

/// Runs `git` with `args` in `local_path` and returns its raw stdout.
///
/// Mirror of Go's `cmd.Output()` (stdout captured, stderr discarded, no env
/// tricks): a non-zero exit becomes the error `<fail_prefix>: exit status N`,
/// with `fail_prefix` the exact Go wording, e.g. `git rev-parse failed`.
fn git_output(local_path: &Path, fail_prefix: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(local_path)
        .output()
        .map_err(|err| anyhow!("{fail_prefix}: {err}"))?;
    if !output.status.success() {
        bail!("{fail_prefix}: {}", exit_status_string(&output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Formats an exit status the way Go's `exec.ExitError` does, so errors read
/// `git rev-parse failed: exit status 128` exactly as in Go. Killed-by-signal
/// wording differs (`signal: 9 (SIGKILL)` vs Go's `signal: killed`); git does
/// not die by signal in any code path we run.
fn exit_status_string(status: &ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => format!("signal: {status}"),
    }
}

/// Returns the current branch name for a git repo.
///
/// Port of `git.CurrentBranch` (git.go:11): `git rev-parse --abbrev-ref HEAD`
/// in `local_path`; the literal output `HEAD` means detached HEAD and is
/// reported as an error, not a branch name.
pub fn current_branch(local_path: &Path) -> Result<String> {
    let output = git_output(
        local_path,
        "git rev-parse failed",
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )?;
    let branch = output.trim();
    if branch == "HEAD" {
        bail!("repository is in detached HEAD state");
    }
    Ok(branch.to_owned())
}

/// Reports whether the working tree at `local_path` has uncommitted changes:
/// staged, unstaged, or untracked files. This matches the set of changes that
/// `git stash -u` would capture, so it flags repos whose uncommitted work an
/// update would displace.
///
/// Port of `git.IsDirty` (git.go:29).
pub fn is_dirty(local_path: &Path) -> Result<bool> {
    let output = git_output(local_path, "git status failed", &["status", "--porcelain"])?;
    Ok(!output.trim().is_empty())
}

/// Returns the default branch of the remote (e.g. "main").
///
/// Port of `git.DefaultBranch` (git.go:40).
pub fn default_branch(local_path: &Path, remote_name: &str) -> Result<String> {
    let ref_name = format!("refs/remotes/{remote_name}/HEAD");
    let output = git_output(
        local_path,
        "git symbolic-ref failed",
        &["symbolic-ref", "--short", &ref_name],
    )?;
    let short = output.trim();
    let prefix = format!("{remote_name}/");
    if !short.starts_with(&prefix) {
        bail!("unexpected default ref {short:?}");
    }
    Ok(short
        .strip_prefix(&prefix)
        .expect("prefix checked above")
        .to_owned())
}

/// Reports whether `branch` has been merged into `base`.
///
/// Port of `git.IsBranchMerged` (git.go:57).
pub fn is_branch_merged(local_path: &Path, branch: &str, base: &str) -> Result<bool> {
    let output = git_output(
        local_path,
        "git branch --merged failed",
        &["branch", "--merged", base, "--format=%(refname:short)"],
    )?;
    Ok(output.split('\n').any(|line| line.trim() == branch))
}

/// Checks whether a branch exists on the given remote. This shells out to
/// ls-remote, so it performs a network round trip.
///
/// Port of `git.RemoteBranchExists` (git.go:74).
pub fn remote_branch_exists(local_path: &Path, remote_name: &str, branch: &str) -> Result<bool> {
    let output = git_output(
        local_path,
        "git ls-remote failed",
        &["ls-remote", "--heads", remote_name],
    )?;
    let target = format!("refs/heads/{branch}");
    Ok(output
        .split('\n')
        .any(|line| line.trim().ends_with(&target)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    // go test-run: internal/execute/execute_test.go TestIsDirty
    #[test]
    fn test_is_dirty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();

        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git run");
            assert!(status.success(), "git {args:?} failed");
        };
        let write_file = |name: &str, content: &str| {
            std::fs::write(dir.join(name), content).expect("write file");
        };

        git(&["init"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);

        // Fresh empty repo: not dirty.
        assert!(!is_dirty(dir).expect("is_dirty"));

        // Untracked file: dirty.
        write_file("untracked.txt", "hello");
        assert!(is_dirty(dir).expect("is_dirty"));

        // Once committed: clean again.
        git(&["add", "untracked.txt"]);
        git(&["commit", "-m", "initial"]);
        assert!(!is_dirty(dir).expect("is_dirty"));

        // Modifying a tracked file: dirty.
        write_file("untracked.txt", "changed");
        assert!(is_dirty(dir).expect("is_dirty"));
    }

    // New (no direct Go counterpart): pins current_branch, default_branch and
    // is_branch_merged against the gittest fixture harness, the same setup
    // M3/M4 tests build on. remote_branch_exists is deliberately not covered:
    // it shells out to ls-remote, a network round trip.
    #[test]
    fn test_read_ops_against_fixture_repo() {
        let root = tempfile::tempdir().expect("tempdir");
        let main_path = root.path().join("demo");
        let wt_path = root.path().join("demo-wt");

        fixtures::repo(&main_path, "ssh://git@example.com/vincent/demo.git");
        fixtures::worktree(&main_path, &wt_path, "feature", false);

        assert_eq!(
            current_branch(&main_path).expect("current_branch"),
            "master"
        );
        assert_eq!(current_branch(&wt_path).expect("current_branch"), "feature");

        // Works offline thanks to the fixture's origin/HEAD symbolic ref.
        assert_eq!(
            default_branch(&main_path, "origin").expect("default_branch"),
            "master"
        );

        // A non-diverged worktree branch is merged into master.
        assert!(
            is_branch_merged(&main_path, "feature", "master").expect("is_branch_merged"),
            "same-commit branch must count as merged"
        );

        // Detached HEAD is an error, not the literal branch name "HEAD".
        fixtures::run(&main_path, &["checkout", "--detach"]);
        let err = current_branch(&main_path).expect_err("detached HEAD must fail");
        assert!(
            err.to_string()
                .contains("repository is in detached HEAD state"),
            "got: {err}"
        );

        // A diverged worktree branch is NOT merged into master.
        let root2 = tempfile::tempdir().expect("tempdir");
        let main2 = root2.path().join("demo");
        let wt2 = root2.path().join("demo-wt");
        fixtures::repo(&main2, "ssh://git@example.com/vincent/demo.git");
        fixtures::worktree(&main2, &wt2, "feature", true);
        assert!(
            !is_branch_merged(&main2, "feature", "master").expect("is_branch_merged"),
            "diverged branch must not count as merged"
        );
    }
}
