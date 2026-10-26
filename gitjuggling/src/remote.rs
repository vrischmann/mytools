//! Remote-side data types shared across the rewrite.
//!
//! Go oracle: `internal/remote/remote.go` ([`RemoteRepo`], [`Source`],
//! `SourceLabel`, `DedupKey`) and `internal/remote/forgejo.go`
//! (`forgejoToken`). Only the shared types and token resolution live here for
//! now — the per-source HTTP clients (`github.rs`, `forgejo.rs`,
//! `secrets.rs`) and `DedupRepos` land with M3 (RUST_REWRITE_PLAN.md §5).

use std::process::Command;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

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
    /// Go: `RemoteRepo.DedupKey`. `DedupRepos` itself lands with the M3
    /// clients; the key type is fixed here so they can build on it.
    pub fn dedup_key(&self) -> (String, String) {
        (self.owner.to_lowercase(), self.name.to_lowercase())
    }
}

/// Resolves the Forgejo API token.
///
/// Go: `forgejo.forgejoToken` (forgejo.go:47). A literal token is returned
/// trimmed; a value starting with `op://` is a 1Password secret *reference*
/// and is resolved by shelling out to
/// `op read --account my.1password.eu <ref>`.
///
/// Hygiene (§4.9), matching the Go original: only the `op://` reference —
/// never the resolved secret — appears in argv or in error messages. The
/// secret itself is only ever present in the returned value.
pub fn forgejo_token(token_or_secret_ref: &str) -> Result<String> {
    let token_or_secret_ref = token_or_secret_ref.trim();
    if !token_or_secret_ref.starts_with("op://") {
        return Ok(token_or_secret_ref.to_owned());
    }

    let output = Command::new("op")
        .args(["read", "--account", "my.1password.eu", token_or_secret_ref])
        .output()
        .map_err(|e| anyhow!("op read failed for {token_or_secret_ref:?}: {e}"))?;
    if !output.status.success() {
        // Report only the exit status: stdout holds the secret and must not
        // be copied into the error (Go wraps *exec.ExitError, whose message
        // is likewise just "exit status N").
        return Err(anyhow!(
            "op read failed for {token_or_secret_ref:?}: {}",
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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

    // Go: forgejo_test.go TestForgejoTokenReturnsPlaintextToken — a literal
    // token is only trimmed, never sent through `op`.
    #[test]
    fn forgejo_token_returns_plaintext_token() {
        let token = forgejo_token("  forgejo_token_here  ").unwrap();
        assert_eq!(token, "forgejo_token_here");
    }

    // `op://` only marks a secret reference at the very start of the value;
    // anything else is a literal and must not spawn `op` (the assertion is
    // that this returns without touching any process).
    #[test]
    fn forgejo_token_requires_op_prefix_at_start() {
        let token = forgejo_token("  token op://vault/item/field ").unwrap();
        assert_eq!(token, "token op://vault/item/field");
    }
}
