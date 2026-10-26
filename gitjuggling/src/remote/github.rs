//! GitHub HTTP client.
//!
//! Go oracle: `internal/remote/github.go`. The HTTP details are
//! load-bearing (RUST_REWRITE_PLAN.md §4.12):
//!
//! - endpoint `https://api.github.com/user/repos` — the *authenticated-user*
//!   listing, so private repos of the logged-in account appear (the
//!   `/users/{owner}/repos` endpoint would not return them);
//! - auth header `Authorization: Bearer <token>`;
//! - `User-Agent: gitjuggling` — GitHub 403s requests with no User-Agent;
//! - page params `page=N&per_page=100&affiliation=owner,collaborator,organization_member`;
//! - pagination stops on the first **empty page**, never on `Link` headers;
//! - owner filter applied afterwards, case-insensitively.

use std::collections::HashSet;

use anyhow::{Result, anyhow};
use serde::Deserialize;

use super::{RemoteRepo, Source, Transport};

/// GitHub API repository object (Go: `githubRepo`, github.go:16).
///
/// Every field carries `#[serde(default)]` to match Go's `encoding/json`,
/// which leaves absent members at their zero value instead of failing.
#[derive(Deserialize, Default)]
struct GithubRepo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    clone_url: String,
    #[serde(default)]
    owner: GithubOwner,
}

/// The `owner` object of a GitHub repo (Go: anonymous struct, github.go:21).
#[derive(Deserialize, Default)]
struct GithubOwner {
    #[serde(default)]
    login: String,
}

/// Fetches all repos visible to the authenticated GitHub user, then filters
/// by the given owners (case-insensitively). This includes private repos
/// owned by the authenticated user, which the `/users/{owner}/repos`
/// endpoint would not return. The `github_token` parameter is optional; if
/// blank, it falls back to `gh auth token`.
///
/// Go: `FetchGitHubRepos` (github.go:43).
pub fn fetch_github_repos(
    transport: &dyn Transport,
    owners: &[String],
    github_token: &str,
) -> Result<Vec<RemoteRepo>> {
    let token = super::secrets::github_token(github_token)?;
    let all_repos = fetch_authenticated_user_repos(transport, &token)?;

    // Filter by configured owners (Go lowercases both sides; so do we).
    let want_owners: HashSet<String> = owners.iter().map(|o| o.to_lowercase()).collect();

    let filtered: Vec<RemoteRepo> = all_repos
        .into_iter()
        .filter(|r| want_owners.contains(r.owner.to_lowercase().as_str()))
        .collect();

    Ok(filtered)
}

/// Paginates through `/user/repos` to fetch all repos visible to the
/// authenticated user (including private ones).
///
/// Go: `fetchAuthenticatedUserRepos` (github.go:65). The loop terminates on
/// the first EMPTY page — matching Go byte for byte, and deliberately NOT on
/// `Link` headers, which would fetch more or fewer pages than the original.
fn fetch_authenticated_user_repos(
    transport: &dyn Transport,
    token: &str,
) -> Result<Vec<RemoteRepo>> {
    let mut repos = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "https://api.github.com/user/repos?page={page}&per_page=100&affiliation=owner,collaborator,organization_member"
        );
        let headers = vec![
            ("Authorization".to_owned(), format!("Bearer {token}")),
            ("User-Agent".to_owned(), "gitjuggling".to_owned()),
        ];

        let response = transport
            .get(&url, &headers)
            .map_err(|e| anyhow!("request to GitHub API failed: {e}"))?;

        if response.status != 200 {
            return Err(anyhow!("GitHub API returned {}", response.status));
        }

        let page_repos: Vec<GithubRepo> = serde_json::from_str(&response.body)
            .map_err(|e| anyhow!("decoding GitHub response: {e}"))?;

        if page_repos.is_empty() {
            break;
        }

        for r in page_repos {
            repos.push(RemoteRepo {
                name: r.name,
                owner: r.owner.login,
                is_fork: r.fork,
                is_archived: r.archived,
                // GitHub's /user/repos listing carries no mirror flag.
                is_mirror: false,
                clone_url: r.clone_url,
                source: Source::GitHub,
            });
        }

        page += 1;
    }

    Ok(repos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{HttpResponse, test_support::CannedTransport};

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.to_owned(),
        }
    }

    fn repo_json(name: &str, owner: &str, fork: bool, archived: bool, clone_url: &str) -> String {
        serde_json::json!({
            "name": name,
            "fork": fork,
            "archived": archived,
            "clone_url": clone_url,
            "owner": { "login": owner },
            "unknown_field_ignored": true,
        })
        .to_string()
    }

    // §4.12: exact endpoint, page params ascending, auth + User-Agent
    // headers, and pagination stopping on the first EMPTY page (the canned
    // transport errors on any request past the last response).
    #[test]
    fn request_shape_and_empty_page_termination() {
        let transport = CannedTransport::new(vec![
            response(
                200,
                &format!(
                    "[{}]",
                    repo_json("repo1", "alice", false, false, "https://github.com/alice/repo1.git")
                ),
            ),
            response(200, "[]"),
        ]);

        let repos = fetch_authenticated_user_repos(&transport, "tok").unwrap();

        assert_eq!(repos.len(), 1);
        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].url,
            "https://api.github.com/user/repos?page=1&per_page=100&affiliation=owner,collaborator,organization_member"
        );
        assert_eq!(
            requests[1].url,
            "https://api.github.com/user/repos?page=2&per_page=100&affiliation=owner,collaborator,organization_member"
        );
        assert_eq!(
            requests[0].headers,
            vec![
                ("Authorization".to_owned(), "Bearer tok".to_owned()),
                ("User-Agent".to_owned(), "gitjuggling".to_owned()),
            ]
        );
    }

    // An empty FIRST page ends the loop immediately: exactly one request.
    #[test]
    fn empty_first_page_stops_after_one_request() {
        let transport = CannedTransport::new(vec![response(200, "[]")]);

        let repos = fetch_authenticated_user_repos(&transport, "tok").unwrap();

        assert!(repos.is_empty());
        assert_eq!(transport.requests().len(), 1);
    }

    // JSON fields map onto RemoteRepo; is_mirror is always false and the
    // source tag is github. Unknown JSON members are ignored (like Go).
    #[test]
    fn maps_repo_fields() {
        let transport = CannedTransport::new(vec![
            response(
                200,
                &format!(
                    "[{}]",
                    repo_json("myrepo", "Alice", true, true, "https://github.com/Alice/myrepo.git")
                ),
            ),
            response(200, "[]"),
        ]);

        let repos = fetch_authenticated_user_repos(&transport, "tok").unwrap();

        assert_eq!(
            repos,
            vec![RemoteRepo {
                name: "myrepo".to_owned(),
                owner: "Alice".to_owned(),
                is_fork: true,
                is_archived: true,
                is_mirror: false,
                clone_url: "https://github.com/Alice/myrepo.git".to_owned(),
                source: Source::GitHub,
            }]
        );
    }

    // Go: "GitHub API returned %d" — the status check comes before any body
    // decode, and a non-200 first page aborts (no second request).
    #[test]
    fn non_200_status_is_an_error_naming_the_status() {
        let transport = CannedTransport::new(vec![response(403, "[]")]);

        let err = fetch_authenticated_user_repos(&transport, "tok").unwrap_err();

        assert_eq!(err.to_string(), "GitHub API returned 403");
        assert_eq!(transport.requests().len(), 1);
    }

    // Go: "decoding GitHub response: %w".
    #[test]
    fn undecodable_body_is_a_decode_error() {
        let transport = CannedTransport::new(vec![response(200, "not json")]);

        let err = fetch_authenticated_user_repos(&transport, "tok").unwrap_err();

        assert!(err.to_string().contains("decoding GitHub response"));
    }

    // Go: FetchGitHubRepos — the owner filter is case-insensitive on both
    // sides and keeps the API's ordering. The direct token short-circuits
    // `gh auth token`, so no process is spawned.
    #[test]
    fn owner_filter_is_case_insensitive() {
        let transport = CannedTransport::new(vec![
            response(
                200,
                &format!(
                    "[{}, {}, {}]",
                    repo_json("r1", "Alice", false, false, "https://github.com/Alice/r1.git"),
                    repo_json("r2", "bob", false, false, "https://github.com/bob/r2.git"),
                    repo_json("r3", "ALICE", false, false, "https://github.com/ALICE/r3.git"),
                ),
            ),
            response(200, "[]"),
        ]);

        let owners = vec!["aLice".to_owned()];
        let repos = fetch_github_repos(&transport, &owners, "tok").unwrap();

        assert_eq!(
            repos.iter().map(|r| r.owner.as_str()).collect::<Vec<_>>(),
            vec!["Alice", "ALICE"]
        );
    }
}
