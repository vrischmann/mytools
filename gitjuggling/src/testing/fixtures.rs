//! Real-git fixture builder (port of `internal/gittest/gittest.go`).
//!
//! Builds small real git repositories for tests; requires the `git` binary.

use std::path::Path;
use std::process::{Command, Output};

/// Runs a git command in `dir`, panicking on failure with the combined output.
///
/// Port of `gittest.run` (gittest.go:14): the author/committer env vars are
/// set on top of the inherited environment, exactly as Go's
/// `cmd.Env = append(os.Environ(), …)` does (later entries win there; explicit
/// `.env()` calls override here).
pub fn run(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git subprocess");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        combined(&output)
    );
}

/// Creates a git repo at `path` with a single commit on master and origin
/// pointing at `origin_url`. `refs/remotes/origin/HEAD` is set to
/// `origin/master` so default-branch detection works without a network.
///
/// Port of `gittest.Repo` (gittest.go:27).
pub fn repo(path: &Path, origin_url: &str) {
    std::fs::create_dir_all(path).expect("create repo dir");
    run(path, &["init", "-b", "master", "."]);
    run(path, &["remote", "add", "origin", origin_url]);
    std::fs::write(path.join("README.md"), "test").expect("write README.md");
    run(path, &["add", "."]);
    run(path, &["commit", "-m", "init"]);
    run(path, &["update-ref", "refs/remotes/origin/master", "HEAD"]);
    run(
        path,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/master",
        ],
    );
}

/// Creates a linked worktree with a new branch at `path` of the repo at
/// `repo_path`. When `diverged` is true, an extra commit is added on the
/// worktree branch so it is no longer an ancestor of master (as with
/// squash-merged or genuinely unmerged branches).
///
/// Port of `gittest.Worktree` (gittest.go:42).
pub fn worktree(repo_path: &Path, path: &Path, branch: &str, diverged: bool) {
    run(repo_path, &["worktree", "add", "-b", branch, &path.to_string_lossy(), "HEAD"]);

    if diverged {
        std::fs::write(path.join("wip.txt"), "wip").expect("write wip.txt");
        run(path, &["add", "."]);
        run(path, &["commit", "-m", "wip"]);
    }
}

/// Go's `CombinedOutput` equivalent: stdout then stderr, lossily decoded
/// (Rust has no interleaving pipe; order is irrelevant for these fixtures).
fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}
