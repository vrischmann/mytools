//! URL normalization for repo matching.
//!
//! Go oracles: `internal/discover.NormalizeURL` (discover.go:222) and
//! `internal/remote.httpsToSSH` (forgejo.go:31). Both are exact ports —
//! including the quirks (`.git` is only stripped for the recognized schemes;
//! anything else is returned as-is) — and the tests below pin the Go cases.

/// Normalizes a git URL for comparison.
///
/// It handles SSH vs HTTPS differences for the same repo. Port of
/// `discover.NormalizeURL` (discover.go:222); examples from its doc comment:
///
/// ```text
/// git@github.com:owner/repo.git       → github.com/owner/repo
/// ssh://git@github.com/owner/repo.git → github.com/owner/repo
/// https://github.com/owner/repo.git   → github.com/owner/repo
/// https://github.com/owner/repo       → github.com/owner/repo
/// ```
///
/// Note what it deliberately does *not* do: there is no case folding and no
/// `.git` stripping on the fallback (scheme-less) form — see the tests.
pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let url = url.trim_end_matches('/');

    // SSH URL form: ssh://[user@]host/path.git — drop the user part and the
    // `.git` suffix. The colon replacement of the SCP-style branch below does
    // NOT apply here.
    if let Some(after) = url.strip_prefix("ssh://") {
        let after = match after.find('@') {
            Some(at) => &after[at + 1..],
            None => after,
        };
        return strip_git_suffix(after);
    }

    // SCP-style SSH form: git@host:owner/repo.git — the first `:` becomes `/`.
    if let Some(after) = url.strip_prefix("git@") {
        return strip_git_suffix(&after.replacen(':', "/", 1));
    }

    // HTTPS / HTTP form: only the `.git` suffix is stripped.
    if let Some(after) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        return strip_git_suffix(after);
    }

    // Fallback: return as-is.
    url.to_owned()
}

/// Converts an HTTPS git URL to SSH format.
///
/// Port of `forgejo.httpsToSSH` (forgejo.go:31); used when building
/// [`crate::remote::RemoteRepo::clone_url`] for Forgejo repos. Examples from
/// its doc comment:
///
/// ```text
/// https://git.example.com/user/project.git → git@git.example.com:user/project.git
/// https://git.example.com/user/project      → git@git.example.com:user/project.git
/// ```
///
/// Non-HTTPS URLs and HTTPS URLs without a `/` after the host are returned
/// as-is (modulo the trailing-slash trim, as in Go).
pub fn https_to_ssh(url: &str) -> String {
    let url = url.trim_end_matches('/');

    let Some(after) = url.strip_prefix("https://") else {
        return url.to_owned(); // not HTTPS, return as-is
    };
    let after = after.strip_suffix(".git").unwrap_or(after);
    let Some((host, path)) = after.split_once('/') else {
        return url.to_owned(); // can't parse, return as-is
    };
    format!("git@{host}:{path}.git")
}

/// Go `strings.TrimSuffix(s, ".git")` equivalent.
fn strip_git_suffix(url: &str) -> String {
    match url.strip_suffix(".git") {
        Some(stripped) => stripped.to_owned(),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go: discover_test.go TestNormalizeSSHURL.
    #[test]
    fn normalize_scp_style_ssh_url() {
        assert_eq!(
            normalize_url("git@github.com:owner/repo.git"),
            "github.com/owner/repo"
        );
    }

    // Go: discover_test.go TestNormalizeHTTPSURL.
    #[test]
    fn normalize_https_url() {
        assert_eq!(
            normalize_url("https://github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_url("https://github.com/owner/repo"),
            "github.com/owner/repo"
        );
    }

    // Go: discover_test.go TestNormalizeForgejoURL.
    #[test]
    fn normalize_forgejo_url() {
        assert_eq!(
            normalize_url("https://git.example.com/user/project.git"),
            "git.example.com/user/project"
        );
    }

    // Go: discover_test.go TestNormalizeTrailingSlash.
    #[test]
    fn normalize_trailing_slash() {
        assert_eq!(
            normalize_url("https://github.com/owner/repo/"),
            "github.com/owner/repo"
        );
    }

    // Go: discover_test.go TestNormalizeIdempotent.
    #[test]
    fn normalize_is_idempotent() {
        let url = "github.com/owner/repo";
        assert_eq!(normalize_url(url), url);
    }

    // The `ssh://` scheme form, with and without a user part (doc-comment
    // examples of discover.go's NormalizeURL).
    #[test]
    fn normalize_ssh_scheme_url() {
        assert_eq!(
            normalize_url("ssh://git@github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_url("ssh://github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
        // No `.git` suffix either.
        assert_eq!(
            normalize_url("ssh://git@github.com/owner/repo"),
            "github.com/owner/repo"
        );
    }

    // Plain http:// is handled like https://.
    #[test]
    fn normalize_http_url() {
        assert_eq!(
            normalize_url("http://github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
    }

    // The fallback returns scheme-less URLs exactly as-is — in particular the
    // `.git` suffix survives there (only the recognized schemes strip it).
    #[test]
    fn normalize_fallback_returns_url_as_is() {
        assert_eq!(
            normalize_url("github.com/owner/repo.git"),
            "github.com/owner/repo.git"
        );
    }

    // Leading/trailing whitespace is trimmed before anything else (Go
    // `strings.TrimSpace`).
    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(
            normalize_url("  https://github.com/owner/repo.git\n"),
            "github.com/owner/repo"
        );
    }

    // Go: remote_test.go TestHttpsToSSH (all five table cases).
    #[test]
    fn https_to_ssh_table() {
        let cases = [
            (
                "https://git.example.com/user/project.git",
                "git@git.example.com:user/project.git",
            ),
            (
                "https://git.example.com/user/project",
                "git@git.example.com:user/project.git",
            ),
            (
                "https://git.example.com/org/deep/nested/repo.git",
                "git@git.example.com:org/deep/nested/repo.git",
            ),
            // Already SSH, unchanged.
            (
                "git@git.example.com:user/project.git",
                "git@git.example.com:user/project.git",
            ),
            (
                "https://git.example.com/user/project/",
                "git@git.example.com:user/project.git",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(https_to_ssh(input), want, "https_to_ssh({input:?})");
        }
    }

    // forgejo.go's can't-parse branch: an HTTPS URL with no path component is
    // returned as-is.
    #[test]
    fn https_to_ssh_without_path_is_returned_as_is() {
        assert_eq!(
            https_to_ssh("https://git.example.com"),
            "https://git.example.com"
        );
    }
}
