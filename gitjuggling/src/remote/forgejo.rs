//! Forgejo HTTP client.
//!
//! Go oracle: `internal/remote/forgejo.go` (`FetchForgejoRepos`). The HTTP
//! details are load-bearing (RUST_REWRITE_PLAN.md §4.12):
//!
//! - endpoint `<base>/api/v1/users/<user>/repos` with `page=N&limit=50`;
//! - auth header `Authorization: token <token>` (note: not `Bearer`);
//! - no `User-Agent` header is set (unlike the GitHub client);
//! - pagination stops on the first **empty page**, never on `Link` headers;
//! - mirror repos are dropped (they duplicate GitHub repos) and every kept
//!   repo's `clone_url` is rewritten to SSH via `urlnorm::https_to_ssh`.

use anyhow::{Result, anyhow};
use serde::Deserialize;

use super::secrets::forgejo_token;
use super::{RemoteRepo, Source, Transport};
use crate::urlnorm::https_to_ssh;

/// Forgejo API repository object (Go: `forgejoRepo`, forgejo.go:16).
///
/// Every field carries `#[serde(default)]` to match Go's `encoding/json`,
/// which leaves absent members at their zero value instead of failing.
#[derive(Deserialize, Default)]
struct ForgejoRepo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    mirror: bool,
    #[serde(default)]
    clone_url: String,
    #[serde(default)]
    owner: ForgejoOwner,
}

/// The `owner` object of a Forgejo repo (Go: anonymous struct, forgejo.go:22).
#[derive(Deserialize, Default)]
struct ForgejoOwner {
    #[serde(default)]
    login: String,
}

/// Fetches all repos for the given Forgejo user, filtering out mirrors.
///
/// `token_ref` is either a literal token or an `op://` 1Password reference
/// (resolved by [`forgejo_token`]).
///
/// Go: `FetchForgejoRepos` (forgejo.go:62).
pub fn fetch_forgejo_repos(
    transport: &dyn Transport,
    base_url: &str,
    user: &str,
    token_ref: &str,
) -> Result<Vec<RemoteRepo>> {
    // Go's TUI command skips the fetch when Forgejo is unconfigured (any of
    // url/user/token empty) — before token resolution or HTTP. Mirror that
    // so an unconfigured workspace loads offline.
    if base_url.is_empty() || user.is_empty() || token_ref.is_empty() {
        return Ok(Vec::new());
    }

    let token = forgejo_token(token_ref)?;

    let mut repos = Vec::new();
    let mut page = 1;

    let base_url = base_url.trim_end_matches('/');

    loop {
        let url = format!("{base_url}/api/v1/users/{user}/repos?page={page}&limit=50");
        let headers = vec![("Authorization".to_owned(), format!("token {token}"))];

        let response = transport.get(&url, &headers).map_err(|e| {
            anyhow!("request to Forgejo API failed for user {user:?} at {base_url}: {e}")
        })?;

        if response.status != 200 {
            return Err(anyhow!(
                "forgejo API returned {} for user {user:?}",
                response.status
            ));
        }

        let page_repos: Vec<ForgejoRepo> = serde_json::from_str(&response.body)
            .map_err(|e| anyhow!("decoding Forgejo response: {e}"))?;

        if page_repos.is_empty() {
            break;
        }

        for r in page_repos {
            // Skip mirrors — they are duplicates of GitHub repos
            if r.mirror {
                continue;
            }

            repos.push(RemoteRepo {
                name: r.name,
                owner: r.owner.login,
                is_fork: r.fork,
                is_archived: r.archived,
                is_mirror: r.mirror,
                clone_url: https_to_ssh(&r.clone_url),
                source: Source::Forgejo,
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

    fn repo_json(name: &str, owner: &str, mirror: bool, clone_url: &str) -> String {
        serde_json::json!({
            "name": name,
            "fork": false,
            "archived": false,
            "mirror": mirror,
            "clone_url": clone_url,
            "owner": { "login": owner },
        })
        .to_string()
    }

    // §4.12: exact endpoint and page params, `Authorization: token …` header,
    // no User-Agent header, trailing slash trimmed off the base URL, and
    // pagination stopping on the first EMPTY page.
    #[test]
    fn request_shape_and_empty_page_termination() {
        let transport = CannedTransport::new(vec![
            response(
                200,
                &format!(
                    "[{}]",
                    repo_json(
                        "ppst",
                        "Vincent",
                        false,
                        "https://git.example.com/vincent/ppst.git",
                    )
                ),
            ),
            response(200, "[]"),
        ]);

        let repos =
            fetch_forgejo_repos(&transport, "https://git.example.com/", "vincent", "tok").unwrap();

        assert_eq!(repos.len(), 1);
        let requests = transport.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].url,
            "https://git.example.com/api/v1/users/vincent/repos?page=1&limit=50"
        );
        assert_eq!(
            requests[1].url,
            "https://git.example.com/api/v1/users/vincent/repos?page=2&limit=50"
        );
        assert_eq!(
            requests[0].headers,
            vec![("Authorization".to_owned(), "token tok".to_owned())]
        );
    }

    // Mirrors are dropped entirely; kept repos get their clone_url rewritten
    // to SSH (via the shared https_to_ssh port) and source tag forgejo. The
    // page was non-empty, so the loop pages on — and stops at the next empty
    // page, exactly like the Go original.
    #[test]
    fn mirrors_are_dropped_and_clone_urls_are_rewritten_to_ssh() {
        let transport = CannedTransport::new(vec![
            response(
                200,
                &format!(
                    "[{}, {}, {}]",
                    repo_json(
                        "mirror1",
                        "vincent",
                        true,
                        "https://git.example.com/vincent/mirror1.git",
                    ),
                    repo_json(
                        "plain",
                        "vincent",
                        false,
                        "https://git.example.com/vincent/plain"
                    ),
                    repo_json(
                        "slash",
                        "vincent",
                        false,
                        "https://git.example.com/vincent/slash/"
                    ),
                ),
            ),
            response(200, "[]"),
        ]);

        let repos =
            fetch_forgejo_repos(&transport, "https://git.example.com", "vincent", "tok").unwrap();

        assert_eq!(
            repos,
            vec![
                RemoteRepo {
                    name: "plain".to_owned(),
                    owner: "vincent".to_owned(),
                    is_fork: false,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "git@git.example.com:vincent/plain.git".to_owned(),
                    source: Source::Forgejo,
                },
                RemoteRepo {
                    name: "slash".to_owned(),
                    owner: "vincent".to_owned(),
                    is_fork: false,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "git@git.example.com:vincent/slash.git".to_owned(),
                    source: Source::Forgejo,
                },
            ]
        );
        // The loop paged past the mirror-dropping page and stopped on the
        // empty second page.
        assert_eq!(transport.requests().len(), 2);
    }

    // An empty FIRST page ends the loop immediately: exactly one request.
    #[test]
    fn empty_first_page_stops_after_one_request() {
        let transport = CannedTransport::new(vec![response(200, "[]")]);

        let repos =
            fetch_forgejo_repos(&transport, "https://git.example.com", "vincent", "tok").unwrap();

        assert!(repos.is_empty());
        assert_eq!(transport.requests().len(), 1);
    }

    // Go: "forgejo API returned %d for user %q" — the user appears quoted in
    // the parity error, and a non-200 first page aborts (no second request).
    #[test]
    fn non_200_status_is_an_error_naming_user_and_status() {
        let transport = CannedTransport::new(vec![response(500, "[]")]);

        let err = fetch_forgejo_repos(&transport, "https://git.example.com", "vincent", "tok")
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "forgejo API returned 500 for user \"vincent\""
        );
        assert_eq!(transport.requests().len(), 1);
    }

    // Go: "decoding Forgejo response: %w".
    #[test]
    fn undecodable_body_is_a_decode_error() {
        let transport = CannedTransport::new(vec![response(200, "not json")]);

        let err = fetch_forgejo_repos(&transport, "https://git.example.com", "vincent", "tok")
            .unwrap_err();

        assert!(err.to_string().contains("decoding Forgejo response"));
    }
}
