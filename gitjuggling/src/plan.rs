//! Sync-plan building (port of `internal/syncplan/syncplan.go`) — the heart
//! of milestone M3 (RUST_REWRITE_PLAN.md §5).
//!
//! [`build_plan`] matches the remote repo list against the locally discovered
//! repos, classifying each as clone / move / update, and — when a live
//! upstream check is supplied — proposes removing linked worktrees whose
//! checked-out branch no longer exists upstream. Per §4.10 the worktree-prune
//! action carries `branch` and `merged` **computed at plan time**
//! ([`crate::git::remote_branch_exists`] / [`crate::git::is_branch_merged`]).
//!
//! Parity notes:
//!
//! * **Pattern rules (§4.1)** are applied during plan building via the
//!   template expander ([`crate::config::expand_template`]): the first
//!   matching rule's expanded template wins over the fork/archived/base
//!   category rules.
//! * **Worktrees must not shadow their main clone (§4.3)**: discovery keeps
//!   worktrees out of the URL/path indices, so [`crate::discover::LocalRepos::find_by_url`]
//!   only ever yields main clones and a worktree can never be classified as a
//!   repo of its own. Prune actions are proposed for worktrees only.
//! * **Plan determinism (§4.5)**: actions follow remote-API order — the
//!   remote-repo slice is walked in order; no map iteration leaks into the
//!   result (the TUI list and the plan JSON depend on it).
//! * **Failure semantics (§4.10)**: move falls back to in-place update when
//!   the expected path is occupied by another local repo; clash → owner
//!   prefix on the **final path component only**.
//! * **Plan-file JSON (§7 decision F)**: the derives here implement the NEW
//!   idiomatic format only — snake_case field names, string enum tags
//!   (`"update"` / `"move"` / `"clone"` / `"prune_worktree"`), every field
//!   always present. The legacy PascalCase/int-tag fallback parser belongs to
//!   M4's `state.rs` and is deliberately not written here.
//!
//! One deliberate deviation (§4.13 / §7 decision I): when the set of known
//! remote URLs is **empty** and orphan candidates exist, [`build_plan`]
//! refuses (returns an error via [`guard_empty_known_remotes`]) instead of
//! reproducing Go, where every local repo would be offered for deletion.
//! With a configured remote set the behavior is exactly Go's; the exit-code
//! wiring of the refusal is M5c.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::Workspace;
use crate::discover::{LocalRepo, LocalRepos};
use crate::git;
use crate::remote::RemoteRepo;
use crate::urlnorm::normalize_url;

/// Classifies the kind of sync action needed (Go: `syncplan.ActionType`).
///
/// Serialized per the new plan-file contract (§7 decision F): lowercase
/// string tags, e.g. `"prune_worktree"` for the worktree prune. The legacy
/// int tags (`0`–`3`) are M4's fallback-parser concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Pull the repo in place (`git stash -u` + `git pull --rebase`; Go:
    /// `ActionUpdate`).
    Update,
    /// `mv` the repo to its expected location (Go: `ActionMove`).
    Move,
    /// First clone of a repo not found locally (Go: `ActionClone`).
    Clone,
    /// Removes a linked worktree whose checked-out branch no longer exists on
    /// the remote, then deletes the merged local branch (Go:
    /// `ActionPruneWorktree`).
    PruneWorktree,
}

/// A planned action for a single remote repo (Go: `syncplan.Action`).
///
/// Every Go field is carried over, serialized snake_case per the new plan
/// file contract (§7 decision F); like the Go original (no `omitempty`)
/// **every field is always present** in the JSON, including empty strings and
/// `false` — an unset path field is the empty path, mirroring Go's `""`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Action {
    /// Go field `Type` (`type` is a Rust keyword, hence the rename).
    #[serde(rename = "type")]
    pub action_type: ActionType,
    /// The remote repo this action is for (Go: `Repo`). Always set by
    /// [`build_plan`].
    pub repo: RemoteRepo,
    /// Set for [`ActionType::Update`] (Go: `LocalPath`).
    #[serde(default)]
    pub local_path: PathBuf,
    /// Set for [`ActionType::Move`] (Go: `CurrentPath`).
    #[serde(default)]
    pub current_path: PathBuf,
    /// Set for [`ActionType::Move`] and [`ActionType::Clone`] (Go:
    /// `ExpectedPath`).
    #[serde(default)]
    pub expected_path: PathBuf,
    /// Set for [`ActionType::Update`] when `local_path == expected_path` (Go:
    /// `AlreadyInPlace`).
    #[serde(default)]
    pub already_in_place: bool,

    /// Worktree fields, set for [`ActionType::PruneWorktree`]: directory of
    /// the stale worktree (Go: `WorktreePath`).
    #[serde(default)]
    pub worktree_path: PathBuf,
    /// Main clone the worktree belongs to (Go: `MainRepoPath`).
    #[serde(default)]
    pub main_repo_path: PathBuf,
    /// Checked-out branch whose upstream is gone (Go: `Branch`).
    #[serde(default)]
    pub branch: String,
    /// Reports whether `branch` is merged into the default branch by ancestry
    /// (`git branch --merged`, computed at plan time). When `false` the
    /// branch is kept after removing the worktree: its commits may be
    /// squash-merged or unmerged, and `git branch -d` would refuse anyway
    /// (Go: `Merged`; §4.10).
    #[serde(default)]
    pub merged: bool,
}

impl Default for Action {
    /// The Go zero value: `ActionUpdate` (iota 0), empty strings, `false`.
    fn default() -> Self {
        Action {
            action_type: ActionType::Update,
            repo: RemoteRepo {
                name: String::new(),
                owner: String::new(),
                is_fork: false,
                is_archived: false,
                is_mirror: false,
                clone_url: String::new(),
                source: crate::remote::Source::GitHub,
            },
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
}

/// Reports whether `branch` exists on `remote_name` for the repository at
/// `local_path` (Go: `syncplan.BranchExistsFunc`). Implementations typically
/// shell out to `git ls-remote`, so this performs a network round trip.
///
/// `build_plan` takes this as `Option<&BranchExistsFn>`: `None` means no
/// prune actions are proposed (Go passes a nil func).
pub type BranchExistsFn = dyn Fn(&Path, &str, &str) -> Result<bool>;

/// Determines the expected local directory for a remote repo based on
/// workspace rules (Go: `syncplan.ClassifyRepo`).
///
/// Priority:
///  1. `is_fork` → `rules.forks` (if configured)
///  2. `is_archived` → `rules.archived` (if configured)
///  3. default → `rules.base`
///
/// A matching pattern rule takes precedence over all of the above (§4.1).
/// If `clash` is true, the final path is `<basedir>/<owner>-<name>`;
/// otherwise it is `<basedir>/<name>`.
pub fn classify_repo(repo: &RemoteRepo, ws: &Workspace, clash: bool) -> PathBuf {
    let path = raw_expected_path(repo, ws);
    if !clash {
        return path;
    }
    // Disambiguate colliding repos by prefixing the owner onto the final
    // path component (§4.10).
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    dir.join(format!("{}-{name}", repo.owner))
}

/// Returns the target directory for a repo before clash disambiguation (Go:
/// `syncplan.rawExpectedPath`). Pattern rules take precedence over the
/// fork/archived/base category rules: the first matching pattern's expanded
/// template wins, and if no pattern matches the repo is placed under its
/// category base directory using its name.
fn raw_expected_path(repo: &RemoteRepo, ws: &Workspace) -> PathBuf {
    if let Some(dir) = match_pattern(ws, &repo.name) {
        return PathBuf::from(dir);
    }
    PathBuf::from(rules_base_dir(repo, ws)).join(&repo.name)
}

/// Evaluates the workspace pattern rules in order against `name` and returns
/// the expanded target directory of the first match (Go:
/// `syncplan.matchPattern`).
///
/// The `to` template is expanded with [`crate::config::expand_template`], so
/// `$1`, `${name}`, and `$0` all work. An uncompilable pattern is skipped —
/// config validation rejects it; stay defensive, exactly as Go.
fn match_pattern(ws: &Workspace, name: &str) -> Option<String> {
    for rule in &ws.rules.patterns {
        let Ok(re) = Regex::new(&rule.pattern) else {
            continue;
        };
        let Some(caps) = re.captures(name) else {
            continue;
        };
        return Some(crate::config::expand_template(&caps, &rule.to));
    }
    None
}

/// Returns the base directory for a repo based on workspace rules (Go:
/// `syncplan.rulesBaseDir`).
fn rules_base_dir<'a>(repo: &RemoteRepo, ws: &'a Workspace) -> &'a str {
    if repo.is_fork && !ws.rules.forks.is_empty() {
        &ws.rules.forks
    } else if repo.is_archived && !ws.rules.archived.is_empty() {
        &ws.rules.archived
    } else {
        &ws.rules.base
    }
}

/// Determines the action for each remote repo by matching against locally
/// discovered repos (Go: `syncplan.BuildPlan`).
///
/// `branch_exists` performs the live upstream check for worktree pruning;
/// `None` means no prune actions are proposed.
///
/// Deviation (§4.13 / §7 decision I): refuses with an error when the set of
/// known remote URLs is empty and orphan candidates exist — see
/// [`guard_empty_known_remotes`]. Otherwise the behavior is exactly Go's.
pub fn build_plan(
    remote_repos: &[RemoteRepo],
    local: &LocalRepos,
    ws: &Workspace,
    branch_exists: Option<&BranchExistsFn>,
) -> Result<Vec<Action>> {
    // Decision I: with an empty known-remote set and orphan candidates, do
    // not produce a plan whose prune phase would offer every local repo for
    // deletion. Checked against the *full* remote list — the known set is
    // what the sources returned, independent of the ignore filter.
    guard_empty_known_remotes(local, remote_repos, ws)?;

    // Filter out ignored repos.
    let remote_repos: Vec<&RemoteRepo> = remote_repos
        .iter()
        .filter(|repo| !ws.is_ignored(&repo.name))
        .collect();

    // Detect collisions: repos that map to the same target directory.
    let mut path_count: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for repo in &remote_repos {
        *path_count.entry(raw_expected_path(repo, ws)).or_insert(0) += 1;
    }

    // Walk the remote-repo slice in API order — action order must stay
    // stable (§4.5): the TUI list and the plan JSON depend on it.
    let mut actions = Vec::new();
    for repo in &remote_repos {
        let clash = path_count
            .get(&raw_expected_path(repo, ws))
            .is_some_and(|&count| count > 1);
        let expected_path = classify_repo(repo, ws, clash);

        match local.find_by_url(&repo.clone_url) {
            // Not found locally: clone it.
            None => actions.push(Action {
                action_type: ActionType::Clone,
                repo: (*repo).clone(),
                expected_path,
                ..Action::default()
            }),
            // Already at the expected location: update in place.
            Some(local_path) if local_path == expected_path => actions.push(Action {
                action_type: ActionType::Update,
                repo: (*repo).clone(),
                local_path: local_path.to_owned(),
                expected_path,
                already_in_place: true,
                ..Action::default()
            }),
            Some(local_path) => {
                // If the expected path is already occupied by a different
                // local repo, just update in place instead of moving (§4.10).
                if local.find_by_path(&expected_path).is_some() {
                    actions.push(Action {
                        action_type: ActionType::Update,
                        repo: (*repo).clone(),
                        local_path: local_path.to_owned(),
                        expected_path,
                        ..Action::default()
                    });
                } else {
                    actions.push(Action {
                        action_type: ActionType::Move,
                        repo: (*repo).clone(),
                        current_path: local_path.to_owned(),
                        expected_path,
                        ..Action::default()
                    });
                }
            }
        }
    }

    actions.extend(worktree_prune_actions(&remote_repos, local, branch_exists));

    Ok(actions)
}

/// §4.13 / §7 decision I guard: refuses to build prune actions when the set
/// of known remote URLs is empty and orphan candidates exist.
///
/// With no `github_owners` and an unconfigured Forgejo, both fetches return
/// empty lists and [`crate::prune::find_orphans`] sees zero known URLs —
/// reproducing Go exactly, it would flag **every** non-ignored, non-worktree
/// local repo for deletion, including a workspace full of repos that were
/// simply never configured. The port refuses instead: the error reports the
/// refusal plus the count of would-be orphans (the run then exits 1 —
/// informational, exit 0 under `--dry-run`; wired in M5c, not here).
///
/// With any configured remote the guard is a no-op and the behavior is
/// exactly Go's.
pub fn guard_empty_known_remotes(
    local: &LocalRepos,
    remote_repos: &[RemoteRepo],
    ws: &Workspace,
) -> Result<()> {
    let known_urls: HashSet<String> = remote_repos
        .iter()
        .map(|repo| normalize_url(&repo.clone_url))
        .collect();
    if !known_urls.is_empty() {
        return Ok(());
    }

    let orphans = crate::prune::find_orphans(local, remote_repos, ws);
    if orphans.is_empty() {
        return Ok(());
    }

    bail!(
        "refusing to prune: no known remote URLs (github_owners empty, forgejo unconfigured) \
         would mark {} local repo(s) for deletion; configure a remote source or inspect with --dry-run",
        orphans.len()
    )
}

/// Proposes removing linked worktrees whose checked-out branch no longer
/// exists upstream (Go: `syncplan.worktreePruneActions`).
///
/// Removing the worktree only deletes the checkout — the branch and its
/// commits stay in the main clone — so the proposal does not depend on merge
/// state. The branch itself is deleted only when it is merged into the
/// default branch; otherwise it is kept (its commits may be squash-merged or
/// genuinely unmerged). Only worktrees of repos known to the workspace are
/// considered, and `branch_exists` being `None` disables the whole pass.
fn worktree_prune_actions(
    remote_repos: &[&RemoteRepo],
    local: &LocalRepos,
    branch_exists: Option<&BranchExistsFn>,
) -> Vec<Action> {
    let Some(branch_exists) = branch_exists else {
        return Vec::new();
    };

    let mut actions = Vec::new();
    for wt in local.worktrees() {
        let Some(repo) = match_remote_repo(remote_repos, wt) else {
            continue;
        };
        let Ok(branch) = git::current_branch(&wt.path) else {
            continue; // detached HEAD or unreadable repo; leave it alone
        };
        // Only prune when we can confirm the upstream is gone: a check error
        // (e.g. network failure) keeps the worktree, as in Go.
        match branch_exists(&wt.path, "origin", &branch) {
            Ok(false) => {}
            Ok(true) | Err(_) => continue,
        }

        actions.push(Action {
            action_type: ActionType::PruneWorktree,
            repo: repo.clone(),
            worktree_path: wt.path.clone(),
            main_repo_path: wt.main_path.clone().unwrap_or_default(),
            branch: branch.clone(),
            merged: worktree_branch_merged(wt, &branch),
            ..Action::default()
        });
    }
    actions
}

/// Finds the remote repo whose clone URL matches the origin of the given
/// discovered repo (Go: `syncplan.matchRemoteRepo`).
fn match_remote_repo<'a>(
    remote_repos: &[&'a RemoteRepo],
    repo: &LocalRepo,
) -> Option<&'a RemoteRepo> {
    for r in remote_repos {
        for url in repo.remote_urls.values() {
            if normalize_url(url) == normalize_url(&r.clone_url) {
                return Some(r);
            }
        }
    }
    None
}

/// Reports whether the worktree's branch has been merged into the default
/// branch of its origin (Go: `syncplan.worktreeBranchMerged`). Any error
/// (missing origin/HEAD, missing remote-tracking ref, git failure) counts as
/// not merged so the branch is kept on the safe side.
fn worktree_branch_merged(wt: &LocalRepo, branch: &str) -> bool {
    let Ok(default_branch) = git::default_branch(&wt.path, "origin") else {
        return false;
    };
    matches!(
        git::is_branch_merged(&wt.path, branch, &default_branch),
        Ok(true)
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anyhow::Result;

    use super::*;
    use crate::config::{PatternRule, Rules};
    use crate::remote::Source;
    use crate::testing::fixtures;

    /// `testWorkspace()` from syncplan_test.go.
    fn test_workspace() -> Workspace {
        Workspace {
            root: "/home/user/dev".to_owned(),
            github_owners: vec!["testowner".to_owned()],
            rules: Rules {
                base: "/home/user/dev/repos".to_owned(),
                forks: "/home/user/dev/forks".to_owned(),
                archived: "/home/user/dev/archived".to_owned(),
                patterns: Vec::new(),
            },
            ..Workspace::default()
        }
    }

    /// `makeRepo(name, owner, isFork, isArchived)` from syncplan_test.go.
    fn make_repo(name: &str, owner: &str, is_fork: bool, is_archived: bool) -> RemoteRepo {
        RemoteRepo {
            name: name.to_owned(),
            owner: owner.to_owned(),
            is_fork,
            is_archived,
            is_mirror: false,
            clone_url: format!("https://github.com/{owner}/{name}.git"),
            source: Source::GitHub,
        }
    }

    /// `discover.LocalRepo{Path: …, RemoteURLs: {"origin": …}}` from the Go tests.
    fn local_repo(path: &Path, origin_url: &str) -> LocalRepo {
        LocalRepo {
            path: path.to_owned(),
            remote_urls: BTreeMap::from([("origin".to_owned(), origin_url.to_owned())]),
            ..LocalRepo::default()
        }
    }

    /// A linked-worktree `discover.LocalRepo` as built by the Go worktree tests.
    fn worktree_repo(path: &Path, main_path: &Path, origin_url: &str) -> LocalRepo {
        LocalRepo {
            path: path.to_owned(),
            remote_urls: BTreeMap::from([("origin".to_owned(), origin_url.to_owned())]),
            is_worktree: true,
            main_path: Some(main_path.to_owned()),
        }
    }

    /// `branchExistsStub(online)` from syncplan_test.go.
    fn branch_exists_stub(online: bool) -> impl Fn(&Path, &str, &str) -> Result<bool> {
        move |_, _, _| Ok(online)
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyBaseRepo
    #[test]
    fn test_classify_base_repo() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let got = classify_repo(&repo, &ws, false);
        let want = PathBuf::from("/home/user/dev/repos").join("myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyFork
    #[test]
    fn test_classify_fork() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", true, false);
        let got = classify_repo(&repo, &ws, false);
        let want = PathBuf::from("/home/user/dev/forks").join("myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyArchived
    #[test]
    fn test_classify_archived() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, true);
        let got = classify_repo(&repo, &ws, false);
        let want = PathBuf::from("/home/user/dev/archived").join("myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyForkTakesPriorityOverArchived
    #[test]
    fn test_classify_fork_takes_priority_over_archived() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", true, true);
        let got = classify_repo(&repo, &ws, false);
        let want = PathBuf::from("/home/user/dev/forks").join("myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyNoForksDirFallsBackToBase
    #[test]
    fn test_classify_no_forks_dir_falls_back_to_base() {
        let mut ws = test_workspace();
        ws.rules.forks = String::new();
        let repo = make_repo("myrepo", "owner", true, false);
        let got = classify_repo(&repo, &ws, false);
        let want = PathBuf::from("/home/user/dev/repos").join("myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanClone
    #[test]
    fn test_build_plan_clone() {
        let ws = test_workspace();
        let local = LocalRepos::new(Vec::new());
        let repo = make_repo("newrepo", "owner", false, false);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());

        assert_eq!(actions[0].action_type, ActionType::Clone);

        let expected = PathBuf::from("/home/user/dev/repos").join("newrepo");
        assert_eq!(actions[0].expected_path, expected);
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanUpdate
    #[test]
    fn test_build_plan_update() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let expected = PathBuf::from("/home/user/dev/repos").join("myrepo");

        let local = LocalRepos::new(vec![local_repo(
            &expected,
            "https://github.com/owner/myrepo.git",
        )]);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());

        assert_eq!(actions[0].action_type, ActionType::Update);
        assert_eq!(actions[0].local_path, expected);
        assert!(
            actions[0].already_in_place,
            "expected AlreadyInPlace to be true"
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanMove
    #[test]
    fn test_build_plan_move() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let wrong_path = PathBuf::from("/home/user/dev/some-other-place/myrepo");

        let local = LocalRepos::new(vec![local_repo(
            &wrong_path,
            "https://github.com/owner/myrepo.git",
        )]);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());

        assert_eq!(actions[0].action_type, ActionType::Move);
        assert_eq!(actions[0].current_path, wrong_path);

        let expected = PathBuf::from("/home/user/dev/repos").join("myrepo");
        assert_eq!(actions[0].expected_path, expected);
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanMoveDestinationOccupied
    #[test]
    fn test_build_plan_move_destination_occupied() {
        let ws = test_workspace();

        // Fork repo — rules say it goes to /home/user/dev/forks/
        let repo = make_repo("nvim-treesitter", "vrischmann", true, false);

        // Currently lives at a different location
        let current_path = PathBuf::from("/home/user/dev/stuff/neovim").join("nvim-treesitter");
        let expected_path = PathBuf::from("/home/user/dev/forks").join("nvim-treesitter");

        // The expected path is already occupied by a different repo
        let local = LocalRepos::new(vec![
            local_repo(
                &current_path,
                "git@github.com:vrischmann/nvim-treesitter.git",
            ),
            local_repo(
                &expected_path,
                "https://github.com/nvim-treesitter/nvim-treesitter",
            ),
        ]);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());

        // Should fall back to update in place, not try to move
        assert_eq!(
            actions[0].action_type,
            ActionType::Update,
            "expected ActionUpdate (destination occupied)"
        );
        assert_eq!(actions[0].local_path, current_path);
        assert!(
            !actions[0].already_in_place,
            "expected AlreadyInPlace to be false"
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyClash
    #[test]
    fn test_classify_clash() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let got = classify_repo(&repo, &ws, true);
        let want = PathBuf::from("/home/user/dev/repos").join("owner-myrepo");
        assert_eq!(got, want);
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanClash
    #[test]
    fn test_build_plan_clash() {
        let ws = test_workspace();
        let repo1 = make_repo("samename", "owner1", false, false);
        let repo2 = make_repo("samename", "owner2", false, false);

        let local = LocalRepos::new(Vec::new());

        let actions = build_plan(&[repo1, repo2], &local, &ws, None).expect("build plan");
        assert_eq!(
            actions.len(),
            2,
            "expected 2 actions, got {}",
            actions.len()
        );

        let expected1 = PathBuf::from("/home/user/dev/repos").join("owner1-samename");
        let expected2 = PathBuf::from("/home/user/dev/repos").join("owner2-samename");

        for a in &actions {
            assert_eq!(a.action_type, ActionType::Clone);
            assert!(
                a.expected_path == expected1 || a.expected_path == expected2,
                "unexpected ExpectedPath {:?}",
                a.expected_path
            );
        }
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanIgnoreExact
    #[test]
    fn test_build_plan_ignore_exact() {
        let mut ws = test_workspace();
        ws.ignore = vec!["dotfiles".to_owned()];

        let repo1 = make_repo("dotfiles", "owner", false, false);
        let repo2 = make_repo("myproject", "owner", false, false);
        let local = LocalRepos::new(Vec::new());

        let actions = build_plan(&[repo1, repo2], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());
        assert_eq!(actions[0].repo.name, "myproject");
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanIgnoreGlob
    #[test]
    fn test_build_plan_ignore_glob() {
        let mut ws = test_workspace();
        ws.ignore = vec!["temp-*".to_owned()];

        let repo1 = make_repo("temp-experiment", "owner", false, false);
        let repo2 = make_repo("real-project", "owner", false, false);
        let local = LocalRepos::new(Vec::new());

        let actions = build_plan(&[repo1, repo2], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1, "expected 1 action, got {}", actions.len());
        assert_eq!(actions[0].repo.name, "real-project");
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanIgnoreAll
    #[test]
    fn test_build_plan_ignore_all() {
        let mut ws = test_workspace();
        ws.ignore = vec!["*".to_owned()];

        let repo1 = make_repo("repo1", "owner", false, false);
        let repo2 = make_repo("repo2", "owner", false, false);
        let local = LocalRepos::new(Vec::new());

        let actions = build_plan(&[repo1, repo2], &local, &ws, None).expect("build plan");
        assert_eq!(
            actions.len(),
            0,
            "expected 0 actions, got {}",
            actions.len()
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyPatternFull
    #[test]
    fn test_classify_pattern_full() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-(.+)$".to_owned(),
            to: "/home/user/dev/workspaces/$1".to_owned(),
        }];
        let repo = make_repo("workspace-foo", "owner", false, false);

        let got = classify_repo(&repo, &ws, false);
        assert_eq!(got, PathBuf::from("/home/user/dev/workspaces/foo"));
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyPatternWholeMatch
    #[test]
    fn test_classify_pattern_whole_match() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-.+$".to_owned(),
            to: "/home/user/dev/workspaces/$0".to_owned(),
        }];
        let repo = make_repo("workspace-foo", "owner", false, false);

        let got = classify_repo(&repo, &ws, false);
        assert_eq!(
            got,
            PathBuf::from("/home/user/dev/workspaces/workspace-foo")
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyPatternBeatsFork
    #[test]
    fn test_classify_pattern_beats_fork() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-(.+)$".to_owned(),
            to: "/home/user/dev/workspaces/$1".to_owned(),
        }];
        let repo = make_repo("workspace-foo", "owner", true, false); // isFork

        let got = classify_repo(&repo, &ws, false);
        assert_eq!(got, PathBuf::from("/home/user/dev/workspaces/foo"));
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyPatternFirstMatchWins
    #[test]
    fn test_classify_pattern_first_match_wins() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![
            PatternRule {
                pattern: r"^workspace-(.+)$".to_owned(),
                to: "/home/user/dev/ws/$1".to_owned(),
            },
            PatternRule {
                pattern: r"^workspace-(.+)$".to_owned(),
                to: "/home/user/dev/other/$1".to_owned(),
            },
        ];
        let repo = make_repo("workspace-foo", "owner", false, false);

        let got = classify_repo(&repo, &ws, false);
        assert_eq!(got, PathBuf::from("/home/user/dev/ws/foo"));
    }

    // Go: internal/syncplan/syncplan_test.go TestClassifyPatternNoMatchFallsBackToBase
    #[test]
    fn test_classify_pattern_no_match_falls_back_to_base() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-(.+)$".to_owned(),
            to: "/home/user/dev/ws/$1".to_owned(),
        }];
        let repo = make_repo("myrepo", "owner", false, false);

        let got = classify_repo(&repo, &ws, false);
        assert_eq!(got, PathBuf::from("/home/user/dev/repos").join("myrepo"));
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanClonePattern
    #[test]
    fn test_build_plan_clone_pattern() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-(.+)$".to_owned(),
            to: "/home/user/dev/workspaces/$1".to_owned(),
        }];
        let local = LocalRepos::new(Vec::new());
        let repo = make_repo("workspace-foo", "owner", false, false);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, ActionType::Clone);
        assert_eq!(
            actions[0].expected_path,
            PathBuf::from("/home/user/dev/workspaces/foo")
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanPatternClash
    #[test]
    fn test_build_plan_pattern_clash() {
        let mut ws = test_workspace();
        ws.rules.patterns = vec![PatternRule {
            pattern: r"^workspace-(.+)$".to_owned(),
            to: "/home/user/dev/workspaces/$1".to_owned(),
        }];
        let repo1 = make_repo("workspace-foo", "owner1", false, false);
        let repo2 = make_repo("workspace-foo", "owner2", false, false);
        let local = LocalRepos::new(Vec::new());

        let actions = build_plan(&[repo1, repo2], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 2);

        let paths: Vec<&PathBuf> = actions.iter().map(|a| &a.expected_path).collect();
        assert!(
            paths.contains(&&PathBuf::from("/home/user/dev/workspaces/owner1-foo")),
            "expected owner1-foo in {paths:?}"
        );
        assert!(
            paths.contains(&&PathBuf::from("/home/user/dev/workspaces/owner2-foo")),
            "expected owner2-foo in {paths:?}"
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanPrunesStaleWorktree
    #[test]
    fn test_build_plan_prunes_stale_worktree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let main_path = root.join("ppst");
        let wt_path = root.join("ppst-iter-seq");

        const URL: &str = "ssh://git@git.rischmann.fr/vincent/ppst.git";
        fixtures::repo(&main_path, URL);
        fixtures::worktree(&main_path, &wt_path, "iter-seq", false);

        let repo = RemoteRepo {
            name: "ppst".to_owned(),
            owner: "vincent".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: URL.to_owned(),
            source: Source::Forgejo,
        };
        let local = LocalRepos::new(vec![
            local_repo(&main_path, URL),
            worktree_repo(&wt_path, &main_path, URL),
        ]);

        let stub = branch_exists_stub(false);
        let actions = build_plan(
            std::slice::from_ref(&repo),
            &local,
            &test_workspace(),
            Some(&stub),
        )
        .expect("build plan");

        assert_eq!(
            actions.len(),
            2,
            "expected classification of the main clone + worktree prune"
        );

        let prune = actions
            .iter()
            .find(|a| a.action_type == ActionType::PruneWorktree)
            .expect("expected a worktree prune action");
        assert_eq!(prune.worktree_path, wt_path);
        assert_eq!(prune.main_repo_path, main_path);
        assert_eq!(prune.branch, "iter-seq");
        assert_eq!(prune.repo, repo);
        assert!(prune.merged, "branch at HEAD of master should be merged");
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanKeepsWorktreeWhenBranchExistsUpstream
    #[test]
    fn test_build_plan_keeps_worktree_when_branch_exists_upstream() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let main_path = root.join("ppst");
        let wt_path = root.join("ppst-iter-seq");

        const URL: &str = "ssh://git@git.rischmann.fr/vincent/ppst.git";
        fixtures::repo(&main_path, URL);
        fixtures::worktree(&main_path, &wt_path, "iter-seq", false);

        let repo = RemoteRepo {
            name: "ppst".to_owned(),
            owner: "vincent".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: URL.to_owned(),
            source: Source::Forgejo,
        };
        let local = LocalRepos::new(vec![
            local_repo(&main_path, URL),
            worktree_repo(&wt_path, &main_path, URL),
        ]);

        let stub = branch_exists_stub(true);
        let actions =
            build_plan(&[repo], &local, &test_workspace(), Some(&stub)).expect("build plan");

        for a in &actions {
            assert_ne!(a.action_type, ActionType::PruneWorktree);
        }
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanPrunesUnmergedWorktreeKeepsBranchFlag
    //
    // A diverged branch (e.g. squash-merged on the forge) still yields a prune
    // action, but flagged as not merged so the branch is kept.
    #[test]
    fn test_build_plan_prunes_unmerged_worktree_keeps_branch_flag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let main_path = root.join("ppst");
        let wt_path = root.join("ppst-iter-seq");

        const URL: &str = "ssh://git@git.rischmann.fr/vincent/ppst.git";
        fixtures::repo(&main_path, URL);
        fixtures::worktree(&main_path, &wt_path, "iter-seq", true); // branch has unmerged work

        let repo = RemoteRepo {
            name: "ppst".to_owned(),
            owner: "vincent".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: URL.to_owned(),
            source: Source::Forgejo,
        };
        let local = LocalRepos::new(vec![
            local_repo(&main_path, URL),
            worktree_repo(&wt_path, &main_path, URL),
        ]);

        let stub = branch_exists_stub(false);
        let actions =
            build_plan(&[repo], &local, &test_workspace(), Some(&stub)).expect("build plan");

        let prune = actions
            .iter()
            .find(|a| a.action_type == ActionType::PruneWorktree)
            .expect("expected a worktree prune action even for an unmerged branch");
        assert!(
            !prune.merged,
            "diverged branch must be flagged as not merged"
        );
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanWorktreeOfUnknownRepoIgnored
    #[test]
    fn test_build_plan_worktree_of_unknown_repo_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let main_path = root.join("other");
        let wt_path = root.join("other-wt");

        const URL: &str = "ssh://git@git.rischmann.fr/vincent/other.git";
        fixtures::repo(&main_path, URL);
        fixtures::worktree(&main_path, &wt_path, "gone-branch", false);

        let local = LocalRepos::new(vec![worktree_repo(&wt_path, &main_path, URL)]);

        // The remote list only knows a different repo, so nothing about the
        // worktree's repo is in the plan.
        let stub = branch_exists_stub(false);
        let actions = build_plan(
            &[make_repo("unrelated", "vincent", false, false)],
            &local,
            &test_workspace(),
            Some(&stub),
        )
        .expect("build plan");

        for a in &actions {
            assert_ne!(a.action_type, ActionType::PruneWorktree);
        }
    }

    // Go: internal/syncplan/syncplan_test.go TestBuildPlanNilBranchExistsSkipsWorktrees
    #[test]
    fn test_build_plan_nil_branch_exists_skips_worktrees() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let main_path = root.join("ppst");
        let wt_path = root.join("ppst-iter-seq");

        const URL: &str = "ssh://git@git.rischmann.fr/vincent/ppst.git";
        fixtures::repo(&main_path, URL);
        fixtures::worktree(&main_path, &wt_path, "iter-seq", false);

        let repo = RemoteRepo {
            name: "ppst".to_owned(),
            owner: "vincent".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: URL.to_owned(),
            source: Source::Forgejo,
        };
        let local = LocalRepos::new(vec![
            local_repo(&main_path, URL),
            worktree_repo(&wt_path, &main_path, URL),
        ]);

        let actions = build_plan(&[repo], &local, &test_workspace(), None).expect("build plan");

        for a in &actions {
            assert_ne!(a.action_type, ActionType::PruneWorktree);
        }
    }

    // ------------------------------------------------------------------
    // §4.13 / §7 decision I guard (deliberate deviation, not in Go).
    // ------------------------------------------------------------------

    // Decision I: empty known-remote set + orphan candidates → refuse, with
    // the would-be orphan count in the message.
    #[test]
    fn test_guard_refuses_with_orphan_count() {
        let ws = test_workspace();
        let local = LocalRepos::new(vec![
            local_repo(
                Path::new("/home/user/dev/repos/a"),
                "https://github.com/someone/a.git",
            ),
            local_repo(
                Path::new("/home/user/dev/repos/b"),
                "https://github.com/someone/b.git",
            ),
        ]);

        let err = guard_empty_known_remotes(&local, &[], &ws).expect_err("guard must refuse");
        let msg = err.to_string();
        assert!(msg.contains("refusing to prune"), "message: {msg}");
        assert!(
            msg.contains('2'),
            "message must carry the orphan count: {msg}"
        );

        // The same refusal comes out of build_plan.
        let err = build_plan(&[], &local, &ws, None).expect_err("build_plan must refuse");
        assert!(err.to_string().contains("refusing to prune"));
    }

    // Decision I: empty known-remote set but no orphan candidates (nothing
    // discovered) → nothing would be pruned, the plan builds as in Go.
    #[test]
    fn test_guard_allows_when_no_orphan_candidates() {
        let ws = test_workspace();
        let local = LocalRepos::new(Vec::new());

        guard_empty_known_remotes(&local, &[], &ws).expect("no orphan candidates");
        let actions = build_plan(&[], &local, &ws, None).expect("build plan");
        assert!(actions.is_empty());
    }

    // Decision I: configured remotes → the guard is a no-op even when
    // unrelated orphans exist (Go behavior: the prune listing plus its
    // interactive prompt handles them).
    #[test]
    fn test_guard_noop_with_known_remotes_despite_orphans() {
        let ws = test_workspace();
        let known = make_repo("known", "owner", false, false);
        let local = LocalRepos::new(vec![local_repo(
            Path::new("/home/user/dev/repos/orphan"),
            "https://github.com/someone/orphan.git",
        )]);

        guard_empty_known_remotes(&local, &[known], &ws).expect("known remotes present");
    }

    // Decision I: with a configured remote set the plan is unchanged Go
    // behavior (the guard never fires on a configured config).
    #[test]
    fn test_build_plan_with_configured_remotes_is_go_behavior() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let expected = PathBuf::from("/home/user/dev/repos").join("myrepo");
        let local = LocalRepos::new(vec![local_repo(
            &expected,
            "https://github.com/owner/myrepo.git",
        )]);

        let actions = build_plan(&[repo], &local, &ws, None).expect("build plan");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, ActionType::Update);
        assert!(actions[0].already_in_place);
    }

    // ------------------------------------------------------------------
    // §7 decision F: the NEW plan-file format only — snake_case fields,
    // string enum tags, every field always present. (The legacy
    // PascalCase/int fallback parser is M4's state.rs.)
    // ------------------------------------------------------------------

    #[test]
    fn test_action_serde_new_format_snake_case_and_string_tags() {
        let action = Action {
            action_type: ActionType::PruneWorktree,
            repo: RemoteRepo {
                name: "ppst".to_owned(),
                owner: "vincent".to_owned(),
                is_fork: false,
                is_archived: false,
                is_mirror: false,
                clone_url: "ssh://git@git.rischmann.fr/vincent/ppst.git".to_owned(),
                source: Source::Forgejo,
            },
            worktree_path: PathBuf::from("/wt"),
            main_repo_path: PathBuf::from("/main"),
            branch: "iter-seq".to_owned(),
            merged: true,
            ..Action::default()
        };

        let json = serde_json::to_value(&action).expect("serialize");
        assert_eq!(json["type"], "prune_worktree");
        assert_eq!(
            json["repo"]["clone_url"],
            "ssh://git@git.rischmann.fr/vincent/ppst.git"
        );
        assert_eq!(json["repo"]["source"], "forgejo");
        assert_eq!(json["local_path"], ""); // always present, like Go's no-omitempty
        assert_eq!(json["expected_path"], "");
        assert_eq!(json["already_in_place"], false);
        assert_eq!(json["worktree_path"], "/wt");
        assert_eq!(json["main_repo_path"], "/main");
        assert_eq!(json["branch"], "iter-seq");
        assert_eq!(json["merged"], true);

        let round: Action = serde_json::from_value(json).expect("deserialize");
        assert_eq!(round, action);
    }
}
