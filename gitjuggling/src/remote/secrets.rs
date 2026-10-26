//! Credential resolution for the remote clients.
//!
//! Go oracles: `internal/remote/forgejo.go` (`forgejoToken`, forgejo.go:47)
//! and `internal/remote/github.go` (`GitHubToken`, github.go:26).
//!
//! Subprocess hygiene (RUST_REWRITE_PLAN.md §4.9), matching the Go
//! originals: the `op://` *reference* (never the resolved secret) and the
//! invoked command's arguments are the only things that reach argv or error
//! messages. Resolved tokens exist only in the returned value — never in
//! argv, never in a clone URL, never echoed into an error.

use std::process::Command;

use anyhow::{Result, anyhow};

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

/// Retrieves a GitHub token, first trying the provided token, then falling
/// back to `gh auth token`.
///
/// Go: `github.GitHubToken` (github.go:26). A non-blank direct token wins
/// (trimmed, and no process is spawned); otherwise the `gh` CLI is asked for
/// the token of the logged-in account.
///
/// Hygiene (§4.9), matching the Go original: `gh auth token` takes no secret
/// on argv, and on failure only the exit status is reported — the token
/// printed on stdout is never copied into the error.
pub fn github_token(direct_token: &str) -> Result<String> {
    let direct_token = direct_token.trim();
    if !direct_token.is_empty() {
        return Ok(direct_token.to_owned());
    }

    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| anyhow!("`gh auth token` failed: {e}"))?;
    if !output.status.success() {
        // Go wraps *exec.ExitError, whose message is just "exit status N";
        // stdout holds the token and must not leak into the error.
        return Err(anyhow!("`gh auth token` failed: {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Go: github.GitHubToken — a non-blank direct token is returned trimmed
    // and `gh` is never spawned (this test passing is that assertion).
    #[test]
    fn github_token_uses_direct_token_without_spawning_gh() {
        let token = github_token("  ghp_direct_token  ").unwrap();
        assert_eq!(token, "ghp_direct_token");
    }

    // The `gh auth token` fallback itself (blank direct token) is exercised
    // on the host rather than here: it depends on the `gh` binary being
    // installed and logged in, exactly like the Go original's unstested
    // branch. Whitespace alone is not a token — the fetcher must not send an
    // empty bearer credential.
}
