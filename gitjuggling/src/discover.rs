//! Filesystem discovery of local git repositories (port of
//! `internal/discover/discover.go`).
//!
//! [`discover`] walks a workspace root looking for `.git` entries: a `.git`
//! **directory** is a plain repo (or the main clone of linked worktrees); a
//! `.git` **file** is either a linked worktree or a submodule checkout, told
//! apart by where its `gitdir:` points (§4.3). Each candidate's `origin` URL
//! is read by shelling out to `git remote get-url` (decision G, §7); a repo
//! without a working `origin` is skipped, exactly as in Go — which repos exist
//! is plan-visible, so this must stay a subprocess, not a git library call.
//!
//! Worktrees are recognized but kept **out of all lookup indices**
//! ([`LocalRepos::find_by_url`] / [`find_by_path`]): they share their origin
//! URL with the main clone, and indexing them would let the last-discovered
//! worktree shadow the main clone (§4.3, fix e96bff2). They are reachable via
//! [`LocalRepos::worktrees`].
//!
//! Walk order: Go's `filepath.WalkDir` visits entries in DFS-lexical order
//! (parents before children, siblings sorted bytewise). The port uses the
//! parallel `jwalk` walker (§4.7, §8 item 2 — accepted deviation), whose yield
//! order is *completion* order, and then re-sorts the discovered entries with
//! [`cmp_walk_order`] so the resulting [`LocalRepos`] iteration order, the
//! `byURL` last-writer-wins tie-breaking, and the orphan ordering all match
//! the Go binary exactly.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use jwalk::WalkDir;

use crate::urlnorm::normalize_url;

/// A locally discovered git repository (Go: `discover.LocalRepo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalRepo {
    /// Path of the directory containing the `.git` entry.
    pub path: PathBuf,
    /// Remote name → URL. Go uses a `map[string]string`; the `BTreeMap` keeps
    /// iteration deterministic (§4.5). Discovery only ever fills `origin`.
    pub remote_urls: BTreeMap<String, String>,
    /// Marks a linked worktree of another clone. Worktrees share their origin
    /// with the main clone, so they are excluded from the URL index and must
    /// never be treated as an independent repo.
    pub is_worktree: bool,
    /// Path of the main clone a worktree belongs to. Only set when
    /// `is_worktree` is true (Go leaves the field empty otherwise).
    pub main_path: Option<PathBuf>,
}

/// All discovered local repos with lookup indices (Go: `discover.LocalRepos`).
///
/// Indices cover **main clones only** (§4.3). The Go original also builds a
/// `byName` index, but nothing ever reads it (discover.go:32,40,58,66 — dead
/// code, RUST_REWRITE_PLAN.md §2), so it is not ported.
pub struct LocalRepos {
    repos: Vec<LocalRepo>,
    worktrees: Vec<usize>,
    by_url: HashMap<String, PathBuf>,
    by_path: HashMap<PathBuf, usize>,
}

impl LocalRepos {
    /// Builds a [`LocalRepos`] from a flat list of repos, constructing all
    /// lookup indices.
    ///
    /// Port of `NewLocalRepos` (discover.go:34). Repos must be in discovery
    /// (walk) order: when several main clones share a normalized URL, the
    /// **last** one indexed wins the `by_url` slot, as in Go.
    pub fn new(repos: Vec<LocalRepo>) -> Self {
        let mut worktrees = Vec::new();
        let mut by_url = HashMap::with_capacity(repos.len());
        let mut by_path = HashMap::with_capacity(repos.len());

        for (i, repo) in repos.iter().enumerate() {
            if repo.is_worktree {
                // Worktrees share their origin URL with the main clone.
                // Indexing them would let the last-discovered worktree shadow
                // the main clone in the URL lookup, so keep them out of all
                // indexes.
                worktrees.push(i);
                continue;
            }
            for url in repo.remote_urls.values() {
                by_url.insert(normalize_url(url), repo.path.clone());
            }
            by_path.insert(repo.path.clone(), i);
        }

        LocalRepos {
            repos,
            worktrees,
            by_url,
            by_path,
        }
    }

    /// Iterates over all discovered local repos, **including worktrees**, as
    /// `(index, repo)` in discovery order.
    ///
    /// Port of `LocalRepos.Iter`. The dirty check (§4.7) and
    /// `prune::find_orphans` both walk this, so worktree coverage is
    /// identical to Go.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &LocalRepo)> {
        self.repos.iter().enumerate()
    }

    /// Returns the linked worktrees found during discovery, in discovery
    /// order.
    ///
    /// Port of `LocalRepos.Worktrees`.
    pub fn worktrees(&self) -> impl Iterator<Item = &LocalRepo> {
        self.worktrees.iter().map(|&i| &self.repos[i])
    }

    /// Looks up a local repo path by matching against clone URLs. Tries both
    /// the normalized URL and the raw URL.
    ///
    /// Port of `LocalRepos.FindByURL`.
    pub fn find_by_url(&self, url: &str) -> Option<&Path> {
        let normalized = normalize_url(url);
        if let Some(path) = self.by_url.get(&normalized) {
            return Some(path.as_path());
        }
        self.by_url.get(url).map(|path| path.as_path())
    }

    /// Looks up a local repo by its path.
    ///
    /// Port of `LocalRepos.FindByPath`.
    pub fn find_by_path(&self, path: &Path) -> Option<&LocalRepo> {
        let i = *self.by_path.get(path)?;
        self.repos.get(i)
    }
}

/// Walks the filesystem under `root`, finding git repos by looking for `.git`
/// entries. A `.git` directory is a repo (or the main clone of worktrees); a
/// `.git` file is either a linked worktree or a submodule, told apart by
/// where their gitdir points. For each entry it reads the origin remote URL.
///
/// Port of `Discover` (discover.go:84). Error handling matches Go: unreadable
/// entries are skipped and the walk continues, and — because Go's walk
/// callback swallows every error — this never fails in practice; the
/// [`Result`] mirrors the Go signature for the callers that check it.
pub fn discover(root: &Path) -> Result<LocalRepos> {
    // `skip_hidden(false)` is load-bearing: `.git` is a hidden entry, and
    // jwalk skips hidden entries *by default*, unlike Go's WalkDir.
    let walker = WalkDir::new(root).skip_hidden(false);

    // Collect candidates first, then sort: jwalk's yield order under its
    // parallel walk is completion order, not Go's DFS-lexical order. The
    // entries are independent (each shells out its own `git remote get-url`),
    // so sorting after collection is behavior-neutral — except that it makes
    // the result byte-for-byte Go-ordered (see the module docs).
    let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
    for entry in walker {
        // Go: `if err != nil { return nil }` — skip errors, keep walking.
        let Ok(entry) = entry else { continue };
        if entry.file_name() != std::ffi::OsStr::new(".git") {
            continue;
        }
        candidates.push((entry.path(), entry.file_type().is_dir()));
    }
    candidates.sort_by(|a, b| cmp_walk_order(&a.0, &b.0));

    let mut repos = Vec::with_capacity(candidates.len());
    for (git_path, is_dir) in candidates {
        // The repo path is the parent of `.git` (Go: `filepath.Dir(path)`).
        let repo_path = git_path.parent().unwrap_or_else(|| Path::new(""));

        let (is_worktree, main_path) = if is_dir {
            // Regular repo or the main clone of linked worktrees.
            (false, None)
        } else {
            // A .git file is a linked worktree or a submodule checkout.
            let Some(main_path) = worktree_main_path(&git_path, repo_path) else {
                continue; // submodule or unrecognized; not a workspace repo
            };
            (true, Some(main_path))
        };

        // Skip repos without an origin remote (e.g. bare init, no remotes
        // configured) — the error itself is deliberately discarded, as in Go.
        let Some(origin_url) = get_remote_url(repo_path, "origin") else {
            continue;
        };

        repos.push(LocalRepo {
            path: repo_path.to_owned(),
            remote_urls: BTreeMap::from([("origin".to_owned(), origin_url)]),
            is_worktree,
            main_path,
        });
    }

    Ok(LocalRepos::new(repos))
}

/// Returns the URL of a git remote by running `git remote get-url`.
///
/// Port of `getRemoteURL` (discover.go:207), minus Go's error value: the only
/// caller skips the repo on failure either way, so `None` carries the same
/// information. stdout is captured, stderr discarded (`cmd.Output`).
fn get_remote_url(repo_path: &Path, remote: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", remote])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Inspects the `.git` file at `git_file_path` (whose parent directory is
/// `worktree_path`) and returns the path of the main clone when the file
/// points at that clone's worktree metadata (`<main>/.git/worktrees/<name>`).
/// Returns `None` for anything else, notably submodule checkouts whose gitdir
/// lives under `<parent>/.git/modules/`.
///
/// Port of `worktreeMainPath` (discover.go:169).
fn worktree_main_path(git_file_path: &Path, worktree_path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(git_file_path).ok()?;
    let gitdir = content.trim().strip_prefix("gitdir:")?.trim();

    // Go: `filepath.Clean(gitdir)` after the join. The shape check below only
    // inspects the last three components, which `Clean` cannot alter for any
    // path git actually writes, so the lexical component walk of
    // `Path::parent` is equivalent here.
    let gitdir = Path::new(gitdir);
    let joined;
    let gitdir: &Path = if gitdir.is_absolute() {
        gitdir
    } else {
        joined = worktree_path.join(gitdir);
        &joined
    };

    // Shape check: <main>/.git/worktrees/<name>
    let name_dir = gitdir.parent()?;
    if name_dir.file_name()? != std::ffi::OsStr::new("worktrees") {
        return None;
    }
    let dot_git_dir = name_dir.parent()?;
    if dot_git_dir.file_name()? != std::ffi::OsStr::new(".git") {
        return None;
    }
    dot_git_dir.parent().map(Path::to_path_buf)
}

/// Orders two walk paths the way Go's `filepath.WalkDir` visits them:
/// depth-first with siblings compared bytewise, parents before children.
///
/// This is *not* plain string order: `"a.txt"` sorts before `"a/b"` as raw
/// strings (`.` < `/`), but `WalkDir` descends into `a` before visiting the
/// `a.txt` entry. Comparing path **components** (with prefix-shorter-first)
/// reproduces the DFS order, which the `byURL` last-writer-wins tie-break and
/// the orphan list order depend on.
fn cmp_walk_order(a: &Path, b: &Path) -> Ordering {
    let mut a_components = a.components();
    let mut b_components = b.components();
    loop {
        match (a_components.next(), b_components.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = x
                    .as_os_str()
                    .as_encoded_bytes()
                    .cmp(y.as_os_str().as_encoded_bytes());
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    // The five TestNormalize* cases of discover_test.go exercise
    // `discover.NormalizeURL`, which was already ported (with those exact
    // cases, annotated per Go test) as `urlnorm::normalize_url` in M1.

    // go test-run: internal/discover/discover_test.go TestDiscoverClassifiesWorktree
    #[test]
    fn test_discover_classifies_worktree() {
        let root = tempfile::tempdir().expect("tempdir");
        let root = root.path();

        let main_path = root.join("ppst");
        fixtures::repo(&main_path, "ssh://git@git.rischmann.fr/vincent/ppst.git");

        let wt_path = root.join("ppst-iter-seq");
        fixtures::worktree(&main_path, &wt_path, "iter-seq", false);

        let local = discover(root).expect("discover");

        // The main clone must own the URL index; the worktree must not shadow it.
        let path = local
            .find_by_url("ssh://git@git.rischmann.fr/vincent/ppst.git")
            .expect("main clone found by URL");
        assert_eq!(path, main_path.as_path());

        let worktrees: Vec<&LocalRepo> = local.worktrees().collect();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, wt_path);
        assert!(worktrees[0].is_worktree);
        // git canonicalizes the worktree's gitdir pointer (on macOS it
        // resolves the /var -> /private/var symlink), so main_path comes
        // back canonicalized while the fixture paths do not. Go's test
        // compares raw paths and therefore only passes on Linux; compare
        // canonicalized paths so the same invariant is tested portably.
        let got_main = worktrees[0].main_path.as_deref().unwrap();
        let got_main = std::fs::canonicalize(got_main).unwrap();
        let expected_main = std::fs::canonicalize(&main_path).unwrap();
        assert_eq!(got_main, expected_main);
        assert_eq!(
            normalize_url(worktrees[0].remote_urls["origin"].as_str()),
            "git.rischmann.fr/vincent/ppst"
        );
    }

    // go test-run: internal/discover/discover_test.go TestDiscoverSkipsSubmoduleCheckout
    #[test]
    fn test_discover_skips_submodule_checkout() {
        let root = tempfile::tempdir().expect("tempdir");
        let root = root.path();

        let main_path = root.join("super");
        fixtures::repo(&main_path, "ssh://git@git.rischmann.fr/vincent/super.git");

        // Fake a submodule checkout: a .git file whose gitdir lives under
        // .git/modules, not under .git/worktrees.
        let sub_path = root.join("submodule-checkout");
        std::fs::create_dir_all(&sub_path).expect("mkdir");
        let git_file = format!(
            "gitdir: {}\n",
            main_path.join(".git").join("modules").join("sub").display()
        );
        std::fs::write(sub_path.join(".git"), git_file).expect("write .git file");

        let local = discover(root).expect("discover");

        assert!(
            local
                .find_by_url("ssh://git@git.rischmann.fr/vincent/super.git")
                .is_some()
        );

        assert_eq!(local.worktrees().count(), 0);

        assert!(local.find_by_path(&sub_path).is_none());
    }

    // New (no Go counterpart): pins the walk-ordering decision behind the
    // accepted §8-item-2 deviation. jwalk yields in completion order, so the
    // port re-sorts into Go's DFS-lexical order; this asserts that order —
    // note `a` must come before `a.txt`, which a plain full-path string sort
    // would get backwards (`.` < `/`) — and Go's byURL last-writer-wins
    // tie-break for main clones sharing one origin URL.
    #[test]
    fn test_discover_walk_order_matches_go_and_by_url_tiebreak() {
        let root = tempfile::tempdir().expect("tempdir");
        let root = root.path();

        for name in ["a", "a.txt", "b"] {
            let path = root.join(name);
            fixtures::repo(&path, "ssh://git@example.com/vincent/shared.git");
        }
        let nested_path = root.join("b").join("nested");
        fixtures::repo(&nested_path, "ssh://git@example.com/vincent/nested.git");

        let local = discover(root).expect("discover");

        let a_path = root.join("a");
        let a_txt_path = root.join("a.txt");
        let b_path = root.join("b");

        let paths: Vec<&Path> = local.iter().map(|(_, repo)| repo.path.as_path()).collect();
        assert_eq!(
            paths,
            vec![
                a_path.as_path(),
                a_txt_path.as_path(),
                b_path.as_path(),
                nested_path.as_path(),
            ],
            "iteration must follow Go's DFS-lexical walk order"
        );

        // `a`, `a.txt` and `b` all share the URL; the last walk entry (b) wins
        // the byURL slot, as in Go.
        assert_eq!(
            local.find_by_url("https://example.com/vincent/shared.git"),
            Some(b_path.as_path())
        );
    }
}
