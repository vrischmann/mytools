//! Remote-side data types and the GitHub/Forgejo HTTP clients.
//!
//! Go oracles: `internal/remote/remote.go` ([`RemoteRepo`], [`Source`],
//! `source_label`, `dedup_key`, `DedupRepos`), `internal/remote/github.go`
//! (the GitHub client + [`github_token`]) and `internal/remote/forgejo.go`
//! (the Forgejo client + [`forgejo_token`], which lives in [`secrets`]).
//!
//! Both listing functions sit behind the injectable [`Transport`] trait —
//! blocking, no async runtime. Tests replay canned JSON through
//! [`test_support::CannedTransport`] and assert the exact HTTP contract of
//! RUST_REWRITE_PLAN.md §4.12 (endpoints, auth headers, page params,
//! empty-page pagination stop, owner/mirror filters); [`UreqTransport`] is
//! the production implementation and carries the mandatory 30s timeout
//! (ureq 3 has no default — an unconfigured agent would hang where Go's
//! `http.Client{Timeout: 30s}` errors).

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

mod forgejo;
mod github;
mod secrets;

pub use forgejo::fetch_forgejo_repos;
pub use github::fetch_github_repos;
pub use secrets::{forgejo_token, github_token};

/// Where a remote repository originates from.
///
/// Go: `remote.RemoteSource` (an int with `SourceGitHub` = 0). The new
/// plan-file format serializes it as a lowercase string tag
/// (RUST_REWRITE_PLAN.md §7 decision F); the legacy int-tagged encoding
/// (`0`/`1`) is handled by the M4 fallback parser in `state.rs`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// github.com (Go: `SourceGitHub`).
    ///
    /// The explicit rename is required: `rename_all = "snake_case"` alone
    /// would produce `"git_hub"`, but the contract (§7 decision F) is the
    /// lowercase source name `"github"` / `"forgejo"`.
    #[serde(rename = "github")]
    GitHub,
    /// A self-hosted Forgejo instance (Go: `SourceForgejo`).
    #[serde(rename = "forgejo")]
    Forgejo,
}

/// A repository as seen on a remote source (GitHub or Forgejo).
///
/// Go: `remote.RemoteRepo`. Field names serialize snake_case per the new
/// plan-file contract (§7 decision F), with every field always present — the
/// Go original has no `omitempty` and the new format keeps that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RemoteRepo {
    pub name: String,
    pub owner: String,
    pub is_fork: bool,
    pub is_archived: bool,
    pub is_mirror: bool,
    pub clone_url: String,
    pub source: Source,
}

impl RemoteRepo {
    /// Returns a short human-readable label for the repo's source.
    ///
    /// Go: `RemoteRepo.SourceLabel`.
    pub fn source_label(&self) -> &'static str {
        match self.source {
            Source::GitHub => "github",
            Source::Forgejo => "forgejo",
        }
    }

    /// Returns the lowercase `(owner, name)` tuple used for deduplication.
    ///
    /// Go: `RemoteRepo.DedupKey`.
    pub fn dedup_key(&self) -> (String, String) {
        (self.owner.to_lowercase(), self.name.to_lowercase())
    }
}

/// Removes Forgejo repos that duplicate GitHub repos (same owner/name).
///
/// Returns the GitHub list unchanged and the Forgejo list minus every repo
/// whose lowercase `(owner, name)` key matches a GitHub repo.
/// Go: `remote.DedupRepos` (remote.go:43).
pub fn dedup_repos(
    github: Vec<RemoteRepo>,
    forgejo: Vec<RemoteRepo>,
) -> (Vec<RemoteRepo>, Vec<RemoteRepo>) {
    let github_keys: HashSet<(String, String)> = github.iter().map(RemoteRepo::dedup_key).collect();

    let filtered: Vec<RemoteRepo> = forgejo
        .into_iter()
        .filter(|r| !github_keys.contains(&r.dedup_key()))
        .collect();

    (github, filtered)
}

/// A blocking HTTP response: status code plus the full body.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Injectable blocking transport for the remote HTTP clients (§4.12).
///
/// Non-2xx statuses are *returned*, not surfaced as errors: the fetchers
/// inspect the status themselves to reproduce Go's exact
/// `… API returned %d` error strings (Go's `net/http` client does the same).
pub trait Transport: Send + Sync {
    /// Issues a blocking GET, returning the status and the full body.
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResponse>;
}

/// Production [`Transport`] backed by `ureq`.
///
/// ureq 3 has **no default timeout**; the 30s `timeout_global` here covers
/// DNS lookup through reading the full response body — the same envelope as
/// Go's client-level `http.Client{Timeout: 30 * time.Second}` (§4.12).
/// `http_status_as_error` is disabled so non-200 statuses reach the fetchers
/// (which own the parity error strings) instead of ureq's own error type.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResponse> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let mut response = request
            .call()
            .map_err(|e| anyhow!("GET {url} failed: {e}"))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| anyhow!("reading response body of {url} failed: {e}"))?;

        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Canned [`Transport`] used by the remote-client tests: replays
    //! recorded status/body pairs in request order and records every request
    //! so tests can assert the exact URL, headers and query params of the
    //! §4.12 contract. Living here (not in `src/testing`) keeps it beside the
    //! HTTP code, like an in-package Go test double.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use anyhow::{Result, anyhow};

    use super::{HttpResponse, Transport};

    /// One recorded request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedRequest {
        pub url: String,
        pub headers: Vec<(String, String)>,
    }

    /// A [`Transport`] replaying canned responses in order.
    pub(crate) struct CannedTransport {
        responses: Mutex<VecDeque<HttpResponse>>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    impl CannedTransport {
        /// Builds a transport serving `responses` in request order. A request
        /// past the last canned response is an error — which is how tests pin
        /// pagination termination.
        pub(crate) fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        /// Snapshot of every request made so far, in order.
        pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Transport for CannedTransport {
        fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResponse> {
            self.requests.lock().unwrap().push(RecordedRequest {
                url: url.to_owned(),
                headers: headers.to_vec(),
            });
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow!("unexpected request: no canned response for {url}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn forgejo_repo() -> RemoteRepo {
        RemoteRepo {
            name: "ppst".to_owned(),
            owner: "Vincent".to_owned(),
            is_fork: false,
            is_archived: true,
            is_mirror: false,
            clone_url: "git@git.example.com:vincent/ppst.git".to_owned(),
            source: Source::Forgejo,
        }
    }

    fn github_repo() -> RemoteRepo {
        RemoteRepo {
            name: "ppst".to_owned(),
            owner: "Alice".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: "https://github.com/Alice/ppst.git".to_owned(),
            source: Source::GitHub,
        }
    }

    // Go: RemoteRepo.SourceLabel.
    #[test]
    fn source_label() {
        let mut repo = forgejo_repo();
        assert_eq!(repo.source_label(), "forgejo");
        repo.source = Source::GitHub;
        assert_eq!(repo.source_label(), "github");
    }

    // Go: RemoteRepo.DedupKey — lowercase (owner, name).
    #[test]
    fn dedup_key_is_lowercase_owner_name() {
        let repo = forgejo_repo();
        assert_eq!(repo.dedup_key(), ("vincent".to_owned(), "ppst".to_owned()));
    }

    // §7 decision F: the NEW format is snake_case fields with a string-tagged
    // `source`, and every field always present. (The legacy PascalCase /
    // int-tag fallback parser is M4's state.rs.)
    #[test]
    fn serde_new_format_snake_case_and_string_tags() {
        let repo = forgejo_repo();
        assert_eq!(
            serde_json::to_value(&repo).unwrap(),
            json!({
                "name": "ppst",
                "owner": "Vincent",
                "is_fork": false,
                "is_archived": true,
                "is_mirror": false,
                "clone_url": "git@git.example.com:vincent/ppst.git",
                "source": "forgejo",
            })
        );
        let back: RemoteRepo =
            serde_json::from_value(serde_json::to_value(&repo).unwrap()).unwrap();
        assert_eq!(back, repo);
    }

    #[test]
    fn source_string_tags_round_trip() {
        for (source, tag) in [(Source::GitHub, "github"), (Source::Forgejo, "forgejo")] {
            assert_eq!(serde_json::to_value(source).unwrap(), json!(tag));
            assert_eq!(
                serde_json::from_value::<Source>(json!(tag)).unwrap(),
                source
            );
        }
    }

    // Go: remote.DedupRepos — Forgejo repos whose lowercase (owner, name) key
    // matches a GitHub repo are dropped; the GitHub list passes through
    // unchanged, and non-duplicates survive.
    #[test]
    fn dedup_repos_drops_forgejo_duplicates_case_insensitively() {
        let mut duplicate = forgejo_repo();
        duplicate.owner = "alice".to_owned(); // matches github_repo's "Alice"
        let distinct = RemoteRepo {
            name: "other".to_owned(),
            owner: "Vincent".to_owned(),
            ..forgejo_repo()
        };

        let (github, forgejo) = dedup_repos(vec![github_repo()], vec![duplicate, distinct.clone()]);

        assert_eq!(github, vec![github_repo()]);
        assert_eq!(forgejo, vec![distinct]);
    }

    // Go: remote.DedupRepos — no GitHub input means nothing is deduped.
    #[test]
    fn dedup_repos_with_no_github_repos_keeps_everything() {
        let forgejo = vec![forgejo_repo()];
        let (github, remaining) = dedup_repos(Vec::new(), forgejo.clone());
        assert!(github.is_empty());
        assert_eq!(remaining, forgejo);
    }
}
