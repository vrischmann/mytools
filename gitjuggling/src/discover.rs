use anyhow::Result;
use jwalk::WalkDir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A locally discovered git repository.
#[derive(Debug, Clone)]
pub struct LocalRepo {
    pub path: PathBuf,
    pub remote_urls: HashMap<String, String>, // remote name → URL
}

/// Discovery result with lookup maps.
#[derive(Debug)]
pub struct LocalRepos {
    pub repos: Vec<LocalRepo>,
    /// Map: remote URL (normalized) → local path
    pub by_url: HashMap<String, PathBuf>,
    /// Map: repo directory name → list of local paths
    #[allow(dead_code)]
    pub by_name: HashMap<String, Vec<PathBuf>>,
}

impl LocalRepos {
    /// Discover all git repos under the given root by scanning for `.git` directories.
    pub fn discover(root: &Path) -> Result<Self> {
        let mut repos = Vec::new();
        let mut by_url: HashMap<String, PathBuf> = HashMap::new();
        let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();

        let walker = WalkDir::new(root).skip_hidden(false);

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();

            // Look for .git directories
            if !path.file_name().map(|n| n == ".git").unwrap_or(false) {
                continue;
            }

            // The repo path is the parent of .git/
            let repo_path = match path.parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            // Read the origin remote URL
            let origin_url = get_remote_url(&repo_path, "origin");

            let mut remote_urls = HashMap::new();
            if let Some(url) = &origin_url {
                let normalized = normalize_url(url);
                by_url.insert(normalized, repo_path.clone());
                remote_urls.insert("origin".to_string(), url.clone());
            }

            // Index by directory name
            let name = repo_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            by_name.entry(name).or_default().push(repo_path.clone());

            repos.push(LocalRepo {
                path: repo_path,
                remote_urls,
            });
        }

        Ok(Self {
            repos,
            by_url,
            by_name,
        })
    }

    /// Find a local repo by matching against any of the provided clone URLs.
    /// Tries both the URL as-is and normalized forms.
    pub fn find_by_url(&self, url: &str) -> Option<&PathBuf> {
        let normalized = normalize_url(url);
        if let Some(p) = self.by_url.get(&normalized) {
            return Some(p);
        }
        // Also try the original URL
        if let Some(p) = self.by_url.get(url) {
            return Some(p);
        }
        None
    }
}

/// Get the URL of a git remote by name.
fn get_remote_url(repo_path: &Path, remote: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", remote])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

/// Normalize a git URL for comparison.
/// Handles SSH vs HTTPS differences for the same repo.
///
/// Examples:
///   git@github.com:owner/repo.git → github.com/owner/repo
///   https://github.com/owner/repo.git → github.com/owner/repo
///   https://git.example.com/owner/repo → git.example.com/owner/repo
fn normalize_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');

    // SSH form: git@host:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        // rest = "github.com:owner/repo.git"
        let normalized = rest.replacen(':', "/", 1);
        return normalized.trim_end_matches(".git").to_string();
    }

    // HTTPS form: https://host/owner/repo.git
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        return rest.trim_end_matches(".git").to_string();
    }

    // Fallback: return as-is
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ssh_url() {
        assert_eq!(
            normalize_url("git@github.com:owner/repo.git"),
            "github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_https_url() {
        assert_eq!(
            normalize_url("https://github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_url("https://github.com/owner/repo"),
            "github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_forgejo_url() {
        assert_eq!(
            normalize_url("https://git.example.com/user/project.git"),
            "git.example.com/user/project"
        );
    }

    #[test]
    fn test_normalize_trailing_slash() {
        assert_eq!(
            normalize_url("https://github.com/owner/repo/"),
            "github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_idempotent() {
        let url = "github.com/owner/repo";
        assert_eq!(normalize_url(url), url);
    }
}
