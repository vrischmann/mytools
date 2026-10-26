//! Orphan detection (port of `internal/prune/prune.go`).
//!
//! M2 carries [`find_orphans`] only; `PruneOrphans` — the actual recursive
//! removal with its confirm callback — lands with the prune flow in M3
//! (RUST_REWRITE_PLAN.md §5).
//!
//! §4.13 parity note: [`find_orphans`] reproduces Go behavior **exactly**,
//! including the quirk that with zero known remote URLs every non-ignored,
//! non-worktree local repo comes back as an orphan. Decision I's guard
//! (refuse when the known-remote set is empty) is deliberately **not**
//! implemented here — it is an M3 deviation, not a Go-parity one.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::Workspace;
use crate::discover::LocalRepos;
use crate::remote::RemoteRepo;
use crate::urlnorm::normalize_url;

/// A local repo that has no matching upstream remote (Go: `prune.OrphanRepo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanRepo {
    pub path: PathBuf,
    pub name: String,
}

/// Finds local repos that have no matching upstream remote.
///
/// Port of `prune.FindOrphans` (prune.go:27). Repos whose directory name
/// matches the workspace ignore list are excluded, and linked worktrees are
/// never orphans: removing one with a recursive delete would corrupt the
/// parent clone's worktree metadata (§4.3). Worktrees are skipped via
/// [`LocalRepos::iter`], which — like Go's `Iter()` — yields them but lets
/// the caller filter, so the coverage contract of §4.7 stays observable.
pub fn find_orphans(
    local: &LocalRepos,
    remote_repos: &[RemoteRepo],
    ws: &Workspace,
) -> Vec<OrphanRepo> {
    // Build a set of normalized remote URLs for fast lookup.
    let remote_urls: HashSet<String> = remote_repos
        .iter()
        .map(|r| normalize_url(&r.clone_url))
        .collect();

    let mut orphans = Vec::new();

    for (_, repo) in local.iter() {
        if repo.is_worktree {
            continue;
        }

        let name = repo
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.path.to_string_lossy().into_owned());

        // Skip ignored repos.
        if ws.is_ignored(&name) {
            continue;
        }

        let has_match = repo
            .remote_urls
            .values()
            .any(|url| remote_urls.contains(&normalize_url(url)));

        if !has_match {
            orphans.push(OrphanRepo {
                path: repo.path.clone(),
                name,
            });
        }
    }

    orphans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::LocalRepo;
    use crate::remote::Source;
    use std::collections::BTreeMap;

    /// `discover.LocalRepo{Path: …, RemoteURLs: {"origin": …}}` from the Go tests.
    fn local_repo(path: &str, origin_url: &str) -> LocalRepo {
        LocalRepo {
            path: PathBuf::from(path),
            remote_urls: BTreeMap::from([("origin".to_owned(), origin_url.to_owned())]),
            ..Default::default()
        }
    }

    /// `&remote.RemoteRepo{Name: …, Owner: …, CloneURL: …}` from the Go tests.
    fn remote_repo(name: &str, owner: &str, clone_url: &str) -> RemoteRepo {
        RemoteRepo {
            name: name.to_owned(),
            owner: owner.to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: clone_url.to_owned(),
            source: Source::GitHub,
        }
    }

    // go test-run: internal/prune/prune_test.go TestFindOrphans
    #[test]
    fn test_find_orphans() {
        let ws = Workspace::default();

        let local = LocalRepos::new(vec![
            local_repo("/home/user/dev/orphan-repo", "https://github.com/someone/orphan-repo.git"),
            local_repo("/home/user/dev/known-repo", "https://github.com/owner/known-repo.git"),
        ]);

        let remote_repos = [remote_repo(
            "known-repo",
            "owner",
            "https://github.com/owner/known-repo.git",
        )];

        let orphans = find_orphans(&local, &remote_repos, &ws);
        assert_eq!(orphans.len(), 1, "expected 1 orphan");
        assert_eq!(orphans[0].name, "orphan-repo");
        assert_eq!(orphans[0].path, PathBuf::from("/home/user/dev/orphan-repo"));
    }

    // go test-run: internal/prune/prune_test.go TestFindOrphansIgnored
    #[test]
    fn test_find_orphans_ignored() {
        let ws = Workspace {
            ignore: vec!["orphan-repo".to_owned()],
            ..Default::default()
        };

        let local = LocalRepos::new(vec![
            local_repo("/home/user/dev/orphan-repo", "https://github.com/someone/orphan-repo.git"),
            local_repo("/home/user/dev/known-repo", "https://github.com/owner/known-repo.git"),
        ]);

        let remote_repos = [remote_repo(
            "known-repo",
            "owner",
            "https://github.com/owner/known-repo.git",
        )];

        let orphans = find_orphans(&local, &remote_repos, &ws);
        assert_eq!(orphans.len(), 0, "expected 0 orphans (ignored)");
    }

    // go test-run: internal/prune/prune_test.go TestFindOrphansIgnoreGlob.
    // `remoteRepos` is empty here, so this also pins the §4.13 quirk: with
    // zero known remote URLs every non-ignored repo is an orphan.
    #[test]
    fn test_find_orphans_ignore_glob() {
        let ws = Workspace {
            ignore: vec!["temp-*".to_owned()],
            ..Default::default()
        };

        let local = LocalRepos::new(vec![
            local_repo(
                "/home/user/dev/temp-experiment",
                "https://github.com/someone/temp-experiment.git",
            ),
            local_repo("/home/user/dev/orphan-repo", "https://github.com/someone/orphan-repo.git"),
        ]);

        let remote_repos: [RemoteRepo; 0] = [];

        let orphans = find_orphans(&local, &remote_repos, &ws);
        assert_eq!(orphans.len(), 1, "expected 1 orphan (temp-experiment ignored)");
        assert_eq!(orphans[0].name, "orphan-repo");
    }
}
