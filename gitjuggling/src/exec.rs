//! Mutating git operations — the full `internal/execute` port (M4b).
//!
//! M2 landed the worktree-prune action and the tracking-error classifiers;
//! this milestone adds the rest of execute.go: the bounded-concurrency
//! runner ([`execute_actions`], Go's `ExecuteActions`), per-type execution
//! ([`execute_action`]: update / move / clone / prune worktree), dry-run
//! handling ([`dry_run_action`]), the tracking-fix helpers
//! ([`push_and_set_upstream`], [`set_upstream_and_pull`],
//! [`resolve_stale_upstream`]) and the GitHub askpass clone authentication.
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
//! * **Update = `git stash -u` then `git pull --rebase`, and the stash is
//!   never popped** — do not "improve" this into a pop (it changes what the
//!   user finds in their working tree; the TUI copy even says the stash is
//!   "recoverable via `git stash pop`"). A test pins the surviving stash
//!   entry.
//! * A declined move is reported as **success** (`"skipped (user declined)"`)
//!   so the durable plan drops it and resume does not re-offer it.
//!   (The occupied-destination fallback — a move that would clobber another
//!   local repo becoming an in-place update — is a *plan-time* decision and
//!   lives in [`crate::plan::build_plan`].)
//! * [`resolve_stale_upstream`] refuses to act when the stale branch **is**
//!   the default branch, and deletes it only when it is merged into the
//!   default branch.
//!
//! §4.9 secrets + subprocess hygiene: the GitHub token reaches the clone
//! subprocess **only through its environment** (a 0700 askpass helper script
//! plus `GITJUGGLING_GITHUB_TOKEN`), never through argv and never through
//! the clone URL; `credential.helper` is cleared for the clone. The helper
//! file is removed by a drop guard, so cleanup survives a panicking worker.
//!
//! Decision B (§7): no async — one orchestrator thread + `std::thread::scope`
//! workers bounded by a counting semaphore, results on a `std::sync::mpsc`
//! channel. Go's contract carries over: results stream as actions complete,
//! the channel closes when everything is done, and cancellation stops
//! *scheduling* new actions while in-flight git subprocesses run to
//! completion (no kill-on-drop — §5 M5b).

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Condvar, Mutex};
use std::thread;

use anyhow::Result;

use crate::git;
use crate::plan::{Action, ActionType};
use crate::remote::{self, RemoteRepo, Source};

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
    let desc = path_desc(worktree_path);
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

// ---------------------------------------------------------------------------
// Bounded-concurrency runner (Go: ExecuteActions)
// ---------------------------------------------------------------------------

/// Cooperative cancellation for [`execute_actions`] — the `context.Context`
/// of the Go port. Setting the flag stops the *scheduling* of new actions;
/// in-flight git subprocesses run to completion (no kill-on-drop).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken(Arc::new(AtomicBool::new(false)))
    }

    /// Requests cancellation; safe to call from any thread.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Interactive confirmation callback (Go: `execute.ConfirmFunc`). It should
/// display the prompt and return true if the user confirms. `None` — what
/// the TUI passes — never prompts.
pub type ConfirmFn = dyn Fn(&str) -> bool + Send + Sync;

/// Runs a list of actions with bounded concurrency.
///
/// Port of `execute.ExecuteActions` (execute.go:324). Results are streamed on
/// the returned receiver **as they complete** (completion order, not plan
/// order); the channel closes when every scheduled action is done. The
/// runner itself returns immediately — the work proceeds on its own
/// orchestrator thread (decision B: no async, threads + mpsc).
///
/// Cancellation (§5 M5b): once [`CancelToken::cancel`] is observed, no new
/// action is scheduled; actions already in flight run their git subprocesses
/// to completion and their results still stream out. (Go closes the channel
/// the instant cancellation is observed mid-scheduling, which can race with
/// in-flight workers sending results; the port closes it after the in-flight
/// work finishes instead — same documented contract, no send-on-closed race.)
///
/// Like Go, the GitHub token is resolved **once, up front** and only when a
/// GitHub clone will actually run ([`github_clone_token`]), and a
/// `concurrency` below 1 is clamped to 1.
pub fn execute_actions(
    actions: &[Action],
    dry_run: bool,
    confirm_fn: Option<Arc<ConfirmFn>>,
    concurrency: usize,
    configured_github_token: &str,
    cancel: &CancelToken,
) -> Receiver<ActionResult> {
    let (tx, rx) = mpsc::channel();
    let orchestrator_tx = tx.clone();

    // Go resolves the token synchronously in ExecuteActions, before any
    // worker starts — keep that (it also keeps `gh auth token` off the
    // worker threads).
    let (github_token, github_token_err) =
        match github_clone_token(actions, configured_github_token) {
            Ok(token) => (token, None),
            Err(err) => (String::new(), Some(err.to_string())),
        };

    let concurrency = concurrency.max(1);
    let actions = actions.to_vec();
    let cancel = cancel.clone();

    let spawned = thread::Builder::new()
        .name("execute-actions".into())
        .spawn(move || {
            let tx = orchestrator_tx;
            let semaphore = Arc::new(Semaphore::new(concurrency));

            thread::scope(|scope| {
                for action in actions {
                    if cancel.is_cancelled() {
                        break;
                    }
                    // Blocks until a worker slot frees up — Go's
                    // `case sem <- struct{}{}`.
                    let permit = semaphore.acquire();
                    // Re-check after the wait: a cancel observed while this
                    // thread sat in `acquire` must not schedule the action.
                    if cancel.is_cancelled() {
                        drop(permit);
                        break;
                    }

                    let tx = tx.clone();
                    let semaphore = Arc::clone(&semaphore);
                    let confirm_fn = confirm_fn.clone();
                    let github_token = github_token.clone();
                    let github_token_err = github_token_err.clone();

                    scope.spawn(move || {
                        let result = if dry_run {
                            dry_run_action(&action)
                        } else {
                            execute_action(
                                &action,
                                confirm_fn.as_deref(),
                                &github_token,
                                github_token_err.as_deref(),
                            )
                        };
                        let _ = tx.send(result);
                        drop(permit); // Go's `defer func() { <-sem }()`
                        drop(semaphore);
                    });
                }
                // Leaving the scope joins every worker: all in-flight
                // subprocesses run to completion — Go's "wait for all
                // goroutines by filling the semaphore".
            });
            // `tx` and every worker clone drop here → the channel closes.
        });

    // The outer sender is not needed: the orchestrator holds its own clone,
    // worker clones keep the channel open until all results are in.
    drop(tx);
    if let Err(err) = spawned {
        panic!("spawn execute-actions thread: {err}");
    }

    rx
}

/// Counting semaphore behind [`execute_actions`] — Go's
/// `sem := make(chan struct{}, concurrency)`.
struct Semaphore {
    available: Mutex<usize>,
    released: Condvar,
}

impl Semaphore {
    fn new(permits: usize) -> Semaphore {
        Semaphore {
            available: Mutex::new(permits),
            released: Condvar::new(),
        }
    }

    /// Blocks until a permit is free and returns it; the permit releases
    /// itself on drop (so a panicking worker cannot leak it — the §4.9
    /// guard-type pattern).
    fn acquire(&self) -> SemaphorePermit<'_> {
        let mut available = self.available.lock().expect("semaphore mutex");
        while *available == 0 {
            available = self.released.wait(available).expect("semaphore condvar");
        }
        *available -= 1;
        SemaphorePermit { semaphore: self }
    }

    fn release(&self) {
        let mut available = self.available.lock().expect("semaphore mutex");
        *available += 1;
        drop(available);
        self.released.notify_one();
    }
}

struct SemaphorePermit<'a> {
    semaphore: &'a Semaphore,
}

impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        self.semaphore.release();
    }
}

// ---------------------------------------------------------------------------
// Action execution (Go: dryRunAction / executeAction / execute*)
// ---------------------------------------------------------------------------

/// Builds the no-op result for `--dry-run` / `--interactive=false` plans
/// (Go: `dryRunAction`). Nothing touches the filesystem; every message is a
/// "would …" promise.
pub fn dry_run_action(action: &Action) -> ActionResult {
    let desc = action_desc(&action.repo);
    match action.action_type {
        ActionType::Update => ActionResult {
            description: desc,
            path: path_string(&action.local_path),
            success: true,
            message: "would update (stash + pull --rebase)".to_owned(),
        },
        ActionType::Move => ActionResult {
            description: desc,
            path: path_string(&action.expected_path),
            success: true,
            message: format!(
                "would move: {} → {}",
                path_string(&action.current_path),
                path_string(&action.expected_path)
            ),
        },
        ActionType::Clone => ActionResult {
            description: desc,
            path: path_string(&action.expected_path),
            success: true,
            message: "would clone".to_owned(),
        },
        ActionType::PruneWorktree => {
            let message = if action.merged {
                format!(
                    "would remove stale worktree and merged branch {}",
                    action.branch
                )
            } else {
                format!(
                    "would remove stale worktree (branch {} kept — not fully merged)",
                    action.branch
                )
            };
            ActionResult {
                description: desc,
                path: path_string(&action.worktree_path),
                success: true,
                message,
            }
        }
    }
}

/// Executes a single planned action for real (Go: `executeAction`).
///
/// `github_token` / `github_token_err` are the values resolved up front by
/// [`github_clone_token`]; only GitHub clones consult them, and a resolution
/// failure is reported as a failed clone *before* any subprocess runs.
pub fn execute_action(
    action: &Action,
    confirm_fn: Option<&ConfirmFn>,
    github_token: &str,
    github_token_err: Option<&str>,
) -> ActionResult {
    match action.action_type {
        ActionType::Update => execute_update(&action.repo, &action.local_path),
        ActionType::Move => execute_move(
            &action.repo,
            &action.current_path,
            &action.expected_path,
            confirm_fn,
        ),
        ActionType::PruneWorktree => remove_worktree_and_branch(
            &action.worktree_path,
            &action.main_repo_path,
            &action.branch,
            action.merged,
        ),
        ActionType::Clone => {
            if action.repo.source == Source::GitHub
                && let Some(err) = github_token_err
            {
                return ActionResult {
                    description: action_desc(&action.repo),
                    path: path_string(&action.expected_path),
                    success: false,
                    message: format!("resolving GitHub token: {err}"),
                };
            }
            execute_clone(&action.repo, &action.expected_path, github_token)
        } // Go's `default: "unknown action type"` arms are unreachable here:
          // the enum is exhaustive.
    }
}

/// `git stash -u` then `git pull --rebase` — and the stash is **never**
/// popped (Go: `executeUpdate`, execute.go:383; §4.10).
///
/// A clean tree stashes fine ("No local changes to save", exit 0), so every
/// update goes through both commands exactly like Go.
fn execute_update(repo: &RemoteRepo, local_path: &Path) -> ActionResult {
    let desc = action_desc(repo);
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: path_string(local_path),
        success,
        message,
    };

    let output = match run_git_combined(local_path, &["stash", "-u"], &[]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git stash failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git stash failed: {}", combined_trimmed(&output)),
        );
    }

    let output = match run_git_combined(local_path, &["pull", "--rebase"], &[]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git pull --rebase failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git pull --rebase failed: {}", combined_trimmed(&output)),
        );
    }

    result(true, "updated".to_owned())
}

/// Moves the repo from `current_path` to `expected_path` after an optional
/// confirmation (Go: `executeMove`). A decline is a **success** result with
/// the literal message `"skipped (user declined)"` — §4.10 couples the
/// durable-plan cleanup to that string, so it must stay verbatim.
fn execute_move(
    repo: &RemoteRepo,
    current_path: &Path,
    expected_path: &Path,
    confirm_fn: Option<&ConfirmFn>,
) -> ActionResult {
    let desc = action_desc(repo);

    if let Some(confirm_fn) = confirm_fn {
        let prompt = format!(
            "Move {desc} from {} to {}?",
            path_string(current_path),
            path_string(expected_path)
        );
        if !confirm_fn(&prompt) {
            return ActionResult {
                description: desc,
                path: path_string(current_path),
                success: true,
                message: "skipped (user declined)".to_owned(),
            };
        }
    }

    // Ensure parent directory exists
    let parent = parent_dir(expected_path);
    if let Err(err) = fs::create_dir_all(&parent) {
        return ActionResult {
            description: desc,
            path: path_string(expected_path),
            success: false,
            message: format!("failed to create parent directory: {err}"),
        };
    }

    if let Err(err) = fs::rename(current_path, expected_path) {
        return ActionResult {
            description: desc,
            path: path_string(expected_path),
            success: false,
            message: format!("failed to move: {err}"),
        };
    }

    ActionResult {
        description: desc,
        path: path_string(expected_path),
        success: true,
        message: "moved".to_owned(),
    }
}

/// `git clone` with GitHub HTTPS authentication through a short-lived
/// askpass helper (Go: `executeClone`, execute.go:459).
///
/// §4.9 hygiene: for GitHub sources the token is injected via the clone
/// subprocess's environment (`GIT_ASKPASS` script answering the Username /
/// Password prompts from `GITJUGGLING_GITHUB_TOKEN`) — never argv, never the
/// clone URL — and `credential.helper` is cleared with `-c` so no cached
/// helper can intercept the exchange. Forgejo clones run plain.
fn execute_clone(repo: &RemoteRepo, expected_path: &Path, github_token: &str) -> ActionResult {
    let desc = action_desc(repo);
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: path_string(expected_path),
        success,
        message,
    };

    // Ensure parent directory exists
    let parent = parent_dir(expected_path);
    if let Err(err) = fs::create_dir_all(&parent) {
        return result(false, format!("failed to create parent directory: {err}"));
    }

    let mut command = Command::new("git");
    let mut args: Vec<OsString> = Vec::new();

    let mut _askpass = None; // keeps the helper alive until the clone ends
    if repo.source == Source::GitHub {
        match AskpassGuard::create() {
            Ok(guard) => {
                command
                    .env("GIT_ASKPASS", guard.path())
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .env("GITJUGGLING_GITHUB_TOKEN", github_token);
                _askpass = Some(guard);
                args.push(OsString::from("-c"));
                args.push(OsString::from("credential.helper="));
            }
            Err(err) => {
                return result(
                    false,
                    format!("creating GitHub authentication helper: {err}"),
                );
            }
        }
    }
    args.push(OsString::from("clone"));
    args.push(OsString::from(&repo.clone_url));
    args.push(expected_path.as_os_str().to_owned());
    command.args(&args);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) => return result(false, format!("git clone failed: {err}")),
    };
    drop(_askpass);

    if !output.status.success() {
        return result(
            false,
            format!("git clone failed: {}", combined_trimmed(&output)),
        );
    }

    result(true, "cloned".to_owned())
}

// ---------------------------------------------------------------------------
// Tracking-fix helpers (Go: PushAndSetUpstream / ResolveStaleUpstream /
// SetUpstreamAndPull) — wired into the TUI's fix phase in M5c.
// ---------------------------------------------------------------------------

/// Pushes the current branch to the remote with `-u`, creating the remote
/// branch and setting the upstream tracking branch (Go:
/// `execute.PushAndSetUpstream`, execute.go:110).
pub fn push_and_set_upstream(local_path: &Path, remote_name: &str, branch: &str) -> ActionResult {
    let desc = path_desc(local_path);
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: path_string(local_path),
        success,
        message,
    };

    let output = match run_git_combined(
        local_path,
        &["push", "-u"],
        &[OsStr::new(remote_name), OsStr::new(branch)],
    ) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git push -u failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git push -u failed: {}", combined_trimmed(&output)),
        );
    }

    result(
        true,
        format!("pushed (tracking set to {remote_name}/{branch})"),
    )
}

/// Handles the case where the local branch tracks a remote ref that no
/// longer exists: checks out the default branch, pulls, and deletes the
/// stale branch **only if it has been merged** (Go:
/// `execute.ResolveStaleUpstream`, execute.go:127). Refuses outright when
/// the stale branch *is* the default branch (§4.10).
pub fn resolve_stale_upstream(
    local_path: &Path,
    remote_name: &str,
    stale_branch: &str,
) -> ActionResult {
    let desc = path_desc(local_path);
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: path_string(local_path),
        success,
        message,
    };

    let default = match git::default_branch(local_path, remote_name) {
        Ok(branch) => branch,
        Err(err) => {
            return result(false, format!("could not determine default branch: {err}"));
        }
    };

    if stale_branch == default {
        return result(
            false,
            format!("stale upstream on default branch {default:?} — refusing to delete"),
        );
    }

    let output = match run_git_combined(local_path, &["checkout"], &[OsStr::new(&default)]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git checkout {default} failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!(
                "git checkout {default} failed: {}",
                combined_trimmed(&output)
            ),
        );
    }

    let output = match run_git_combined(local_path, &["pull", "--rebase"], &[]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git pull --rebase failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git pull --rebase failed: {}", combined_trimmed(&output)),
        );
    }

    let merged = match git::is_branch_merged(local_path, stale_branch, &default) {
        Ok(merged) => merged,
        Err(err) => {
            return result(
                false,
                format!("could not check merge status of {stale_branch}: {err}"),
            );
        }
    };

    if !merged {
        return result(
            true,
            format!("switched to {default} and pulled (kept {stale_branch} — not merged)"),
        );
    }

    let output = match run_git_combined(local_path, &["branch", "-d"], &[OsStr::new(stale_branch)])
    {
        Ok(output) => output,
        Err(err) => return result(false, format!("git branch -d {stale_branch} failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!(
                "git branch -d {stale_branch} failed: {}",
                combined_trimmed(&output)
            ),
        );
    }

    result(
        true,
        format!("switched to {default}, pulled, deleted merged branch {stale_branch}"),
    )
}

/// Sets the upstream tracking branch and runs `git pull --rebase` (Go:
/// `execute.SetUpstreamAndPull`, execute.go:200).
pub fn set_upstream_and_pull(local_path: &Path, remote_name: &str, branch: &str) -> ActionResult {
    let desc = path_desc(local_path);
    let result = |success: bool, message: String| ActionResult {
        description: desc.clone(),
        path: path_string(local_path),
        success,
        message,
    };

    // git branch --set-upstream-to=origin/<branch> <branch>
    let upstream = format!("{remote_name}/{branch}");
    let output = match run_git_combined(
        local_path,
        &["branch", "--set-upstream-to"],
        &[OsStr::new(&upstream), OsStr::new(branch)],
    ) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git branch --set-upstream-to failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!(
                "git branch --set-upstream-to failed: {}",
                combined_trimmed(&output)
            ),
        );
    }

    // git pull --rebase
    let output = match run_git_combined(local_path, &["pull", "--rebase"], &[]) {
        Ok(output) => output,
        Err(err) => return result(false, format!("git pull --rebase failed: {err}")),
    };
    if !output.status.success() {
        return result(
            false,
            format!("git pull --rebase failed: {}", combined_trimmed(&output)),
        );
    }

    result(true, format!("updated (tracking set to {upstream})"))
}

// ---------------------------------------------------------------------------
// GitHub clone-token plumbing (Go: githubCloneToken / githubAskpass)
// ---------------------------------------------------------------------------

/// Resolves a token only when a GitHub clone will run (Go:
/// `execute.githubCloneToken`, execute.go:505). This preserves the
/// `gh auth token` fallback while avoiding a needless credential lookup for
/// Forgejo-only plans.
fn github_clone_token(actions: &[Action], configured_token: &str) -> Result<String> {
    for action in actions {
        if action.action_type == ActionType::Clone && action.repo.source == Source::GitHub {
            return remote::github_token(configured_token);
        }
    }
    Ok(String::new())
}

/// Short-lived askpass helper for GitHub HTTPS clones (Go:
/// `execute.githubAskpass`, execute.go:526).
///
/// The script answers git's Username / Password prompts with the
/// conventional `x-access-token` / `$GITJUGGLING_GITHUB_TOKEN` pair, so the
/// token itself is only ever present in the clone subprocess's environment —
/// never argv, never the clone URL (§4.9). The file is created 0700 and
/// removed on drop; a drop guard so cleanup survives a panicking worker.
struct AskpassGuard {
    path: PathBuf,
}

impl AskpassGuard {
    fn create() -> io::Result<AskpassGuard> {
        const SCRIPT: &str = r#"#!/bin/sh
case "$1" in
  *Username*|*username*) printf '%s\n' x-access-token ;;
  *Password*|*password*) printf '%s\n' "$GITJUGGLING_GITHUB_TOKEN" ;;
  *) exit 1 ;;
esac
"#;

        let dir = std::env::temp_dir();
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        for _ in 0..64 {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!(
                "gitjuggling-github-askpass-{}-{n}",
                std::process::id()
            ));
            // os.CreateTemp equivalent: O_EXCL creation gives a unique file.
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            };
            let write_result = file.write_all(SCRIPT.as_bytes()).and_then(|()| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                }
                #[cfg(not(unix))]
                {
                    Ok(())
                }
            });
            return match write_result {
                Ok(()) => Ok(AskpassGuard { path }),
                Err(err) => {
                    let _ = fs::remove_file(&path);
                    Err(err)
                }
            };
        }
        Err(io::Error::other("could not create askpass helper file"))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AskpassGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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

/// `base(dir(path)) + "/" + base(path)` — the description used by
/// [`remove_worktree_and_branch`], [`push_and_set_upstream`],
/// [`resolve_stale_upstream`] and [`set_upstream_and_pull`] (Go:
/// `filepath.Base(filepath.Dir(path)) + "/" + filepath.Base(path)`).
fn path_desc(path: &Path) -> String {
    format!(
        "{}/{}",
        path_base(path.parent().unwrap_or(path)),
        path_base(path),
    )
}

/// `owner/name (source)` — the description used by [`dry_run_action`],
/// [`execute_action`] and the update/move/clone executors (Go:
/// `fmt.Sprintf("%s/%s (%s)", repo.Owner, repo.Name, repo.SourceLabel())`).
fn action_desc(repo: &RemoteRepo) -> String {
    format!("{}/{} ({})", repo.owner, repo.name, repo.source_label())
}

/// Go passes paths around as strings; this is the lossy stringification
/// every `ActionResult.path` and prompt uses.
fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Go's `filepath.Dir` for the create-parent-directory calls: never empty
/// (Go's `filepath.Dir` returns at least `"."`, and `MkdirAll(".")` is a
/// no-op success).
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_owned(),
        _ => PathBuf::from("."),
    }
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

    // ------------------------------------------------------------------
    // M4 layer: end-to-end against a local bare "origin", the §4.10
    // stash pin, the execute_actions runner contract, and dry-run
    // immutability (RUST_REWRITE_PLAN.md §5 phase M4).
    // ------------------------------------------------------------------

    /// A Forgejo-sourced remote repo (its clones run plain, no askpass)
    /// whose clone URL points at a local bare origin.
    fn e2e_repo(name: &str, clone_url: &Path) -> RemoteRepo {
        RemoteRepo {
            name: name.to_owned(),
            owner: "vincent".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: clone_url.to_string_lossy().into_owned(),
            source: Source::Forgejo,
        }
    }

    /// An `Action` with every field spelled out (Go's zero value equivalent;
    /// no `Default` derive on the struct).
    fn e2e_action(action_type: ActionType, repo: RemoteRepo) -> Action {
        Action {
            action_type,
            repo,
            local_path: PathBuf::new(),
            current_path: PathBuf::new(),
            expected_path: PathBuf::new(),
            already_in_place: false,
            worktree_path: PathBuf::new(),
            main_repo_path: PathBuf::new(),
            branch: String::new(),
            merged: false,
        }
    }

    /// `git <args>` in `dir`, asserting success and returning combined
    /// output (mirrors the Go tests' inline exec.Command probes).
    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} in {} failed: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    /// The M4 temp-repo end-to-end: clone from a local bare "origin", then
    /// update after the origin advances (stash + pull, never popped — §4.10),
    /// then move to a rules-derived path, then prune a stale worktree of the
    /// moved repo.
    #[test]
    fn test_execute_action_lifecycle_against_bare_origin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let origin = root.join("origin.git");
        fixtures::bare_repo(&origin);
        let origin_url = origin.to_string_lossy().into_owned();
        let repo = e2e_repo("demo", &origin);

        // A second clone, used later to advance the origin.
        let other = root.join("other");
        fixtures::clone(&origin_url, &other);

        // 1. Clone into the workspace.
        let ws = root.join("ws");
        let demo = ws.join("demo");
        let mut action = e2e_action(ActionType::Clone, repo.clone());
        action.expected_path = demo.clone();
        let result = execute_action(&action, None, "", None);
        assert!(result.success, "clone failed: {}", result.message);
        assert_eq!(result.description, "vincent/demo (forgejo)");
        assert_eq!(result.path, path_string(&demo));
        assert_eq!(result.message, "cloned");
        assert!(demo.join(".git").exists());

        // 2. Update: advance the origin, dirty the clone, then stash+pull.
        std::fs::write(other.join("second.txt"), "second").expect("write second.txt");
        git_ok(&other, &["add", "."]);
        git_ok(&other, &["commit", "-m", "second"]);
        git_ok(&other, &["push", "origin", "master"]);

        std::fs::write(demo.join("dirty.txt"), "dirty").expect("write dirty.txt");

        let mut action = e2e_action(ActionType::Update, repo.clone());
        action.local_path = demo.clone();
        let result = execute_action(&action, None, "", None);
        assert!(result.success, "update failed: {}", result.message);
        assert_eq!(result.message, "updated");
        assert_eq!(result.path, path_string(&demo));
        // The pull brought the second commit's file in.
        assert!(demo.join("second.txt").exists());
        // §4.10 pin: the stash is never popped — the uncommitted work lives
        // in the stash, not the tree.
        assert!(
            !demo.join("dirty.txt").exists(),
            "stashed file must be gone from the tree"
        );
        let stash_list = git_ok(&demo, &["stash", "list"]);
        assert!(
            !stash_list.trim().is_empty(),
            "stash entry must survive the update"
        );

        // 3. Move to the rules-derived path.
        let moved = ws.join("repos").join("demo");
        let mut action = e2e_action(ActionType::Move, repo.clone());
        action.current_path = demo.clone();
        action.expected_path = moved.clone();
        let result = execute_action(&action, None, "", None);
        assert!(result.success, "move failed: {}", result.message);
        assert_eq!(result.message, "moved");
        assert_eq!(result.path, path_string(&moved));
        assert!(moved.join(".git").exists());
        assert!(!demo.exists());

        // 4. Prune a stale worktree of the moved repo (merged branch).
        let wt = ws.join("demo-wt");
        fixtures::worktree(&moved, &wt, "wt", false);
        let mut action = e2e_action(ActionType::PruneWorktree, repo);
        action.worktree_path = wt.clone();
        action.main_repo_path = moved.clone();
        action.branch = "wt".to_owned();
        action.merged = true;
        let result = execute_action(&action, None, "", None);
        assert!(result.success, "prune failed: {}", result.message);
        assert_eq!(
            result.message,
            "removed stale worktree and merged branch wt"
        );
        assert_eq!(result.path, path_string(&wt));
        assert!(!wt.exists());
        let branches = git_ok(&moved, &["branch", "--list", "wt"]);
        assert!(
            branches.trim().is_empty(),
            "branch wt should be deleted, got {branches:?}"
        );
    }

    /// The runner streams results in completion order until the channel
    /// closes (Go: ExecuteActions closes `out` when all actions finish);
    /// three clones of distinct bare origins under concurrency 2.
    #[test]
    fn test_execute_actions_streams_until_channel_closes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let mut actions = Vec::new();
        let mut want_paths = std::collections::BTreeSet::new();
        for i in 0..3 {
            let origin = root.join(format!("origin{i}.git"));
            fixtures::bare_repo(&origin);
            let expected = root.join("ws").join(format!("demo{i}"));
            let mut action = e2e_action(ActionType::Clone, e2e_repo(&format!("demo{i}"), &origin));
            action.expected_path = expected.clone();
            want_paths.insert(path_string(&expected));
            actions.push(action);
        }

        let cancel = CancelToken::new();
        let rx = execute_actions(&actions, false, None, 2, "", &cancel);
        let results: Vec<ActionResult> = rx.into_iter().collect();

        assert_eq!(results.len(), 3, "one result per action, then close");
        for result in &results {
            assert!(result.success, "clone failed: {}", result.message);
            assert_eq!(result.message, "cloned");
        }
        let got_paths: std::collections::BTreeSet<String> =
            results.iter().map(|r| r.path.clone()).collect();
        assert_eq!(
            got_paths, want_paths,
            "completion order is free; the set is not"
        );
        for path in &want_paths {
            assert!(
                PathBuf::from(path).join(".git").exists(),
                "clone landed: {path}"
            );
        }
    }

    /// A cancellation observed while an action is in flight lets that
    /// action finish while nothing new is scheduled — the runner's re-check
    /// after the semaphore acquire is what makes this deterministic at
    /// concurrency 1 (Go: ctx cancel stops scheduling, in-flight runs
    /// complete).
    #[test]
    fn test_execute_actions_cancel_stops_scheduling_inflight_completes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let repo = e2e_repo("demo", &root.join("unused.git"));

        let a_path = root.join("a");
        let b_path = root.join("b");
        fixtures::repo(&a_path, "https://example.com/unused/a.git");
        fixtures::repo(&b_path, "https://example.com/unused/b.git");

        let moved_a = root.join("ws").join("a");
        let moved_b = root.join("ws").join("b");

        let mut action_a = e2e_action(ActionType::Move, repo.clone());
        action_a.current_path = a_path.clone();
        action_a.expected_path = moved_a.clone();
        let mut action_b = e2e_action(ActionType::Move, repo);
        action_b.current_path = b_path.clone();
        action_b.expected_path = moved_b.clone();

        // The first action's confirmation cancels the run while it is in
        // flight; with concurrency 1 the orchestrator blocks acquiring the
        // second slot, then re-checks and must not schedule action B.
        let cancel = CancelToken::new();
        let cancel_for_confirm = cancel.clone();
        let confirm: Arc<ConfirmFn> = Arc::new(move |_prompt| {
            cancel_for_confirm.cancel();
            true
        });

        let rx = execute_actions(&[action_a, action_b], false, Some(confirm), 1, "", &cancel);
        let results: Vec<ActionResult> = rx.into_iter().collect();

        assert_eq!(results.len(), 1, "only the in-flight action may finish");
        let result = &results[0];
        assert!(result.success, "move failed: {}", result.message);
        assert_eq!(result.message, "moved");
        assert_eq!(result.path, path_string(&moved_a));
        assert!(moved_a.exists(), "in-flight action completed on disk");
        assert!(!moved_b.exists(), "second action must not be scheduled");
        assert!(b_path.exists(), "second repo untouched");
    }

    /// dry_run_action reports every action type's message without touching
    /// the filesystem, and execute_actions(dry_run=true) routes through it.
    #[test]
    fn test_dry_run_action_reports_without_mutating() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let origin = root.join("origin.git");
        fixtures::bare_repo(&origin);
        let repo = e2e_repo("demo", &origin);

        // A real repo to point update/move/prune at, with a linked worktree.
        let demo = root.join("demo");
        fixtures::repo(&demo, "https://example.com/unused/demo.git");
        std::fs::write(demo.join("dirty.txt"), "dirty").expect("write dirty.txt");
        let wt = root.join("demo-wt");
        fixtures::worktree(&demo, &wt, "wt", false);

        let moved = root.join("ws").join("demo");
        let cloned = root.join("ws").join("clone");
        let mut update = e2e_action(ActionType::Update, repo.clone());
        update.local_path = demo.clone();
        let mut mv = e2e_action(ActionType::Move, repo.clone());
        mv.current_path = demo.clone();
        mv.expected_path = moved.clone();
        let mut clone = e2e_action(ActionType::Clone, repo.clone());
        clone.expected_path = cloned.clone();
        let mut prune = e2e_action(ActionType::PruneWorktree, repo);
        prune.worktree_path = wt.clone();
        prune.main_repo_path = demo.clone();
        prune.branch = "wt".to_owned();
        prune.merged = false;

        // dry_run_action's path/message per type, verbatim from the Go
        // wording (execute.go dryRunAction).
        let expected = [
            (
                path_string(&demo),
                "would update (stash + pull --rebase)".to_owned(),
            ),
            (
                path_string(&moved),
                format!(
                    "would move: {} → {}",
                    path_string(&demo),
                    path_string(&moved)
                ),
            ),
            (path_string(&cloned), "would clone".to_owned()),
            (
                path_string(&wt),
                "would remove stale worktree (branch wt kept — not fully merged)".to_owned(),
            ),
        ];

        let actions = [update, mv, clone, prune];
        for (action, (path, message)) in actions.iter().zip(&expected) {
            let result = dry_run_action(action);
            assert!(result.success, "dry run reports success: {result:?}");
            assert_eq!(result.path, *path);
            assert_eq!(result.message, *message);
            assert_eq!(result.description, "vincent/demo (forgejo)");
        }

        // No mutations: the dirty file survives, nothing moved or cloned,
        // the worktree survives.
        assert!(demo.join("dirty.txt").exists());
        assert!(!root.join("ws").exists());
        assert!(wt.join(".git").exists());

        // The runner's dry-run path routes every action through it.
        let cancel = CancelToken::new();
        let rx = execute_actions(&actions, true, None, 2, "", &cancel);
        let results: Vec<ActionResult> = rx.into_iter().collect();
        assert_eq!(results.len(), 4);
        let by_path: std::collections::HashMap<String, &ActionResult> =
            results.iter().map(|r| (r.path.clone(), r)).collect();
        for (path, message) in &expected {
            let result = by_path
                .get(path)
                .unwrap_or_else(|| panic!("dry-run result for {path} missing"));
            assert!(result.success);
            assert_eq!(result.message, *message);
        }

        // Still no mutations after the runner pass.
        assert!(demo.join("dirty.txt").exists());
        assert!(!root.join("ws").exists());
    }
}
