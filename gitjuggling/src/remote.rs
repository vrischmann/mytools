use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

// ---------------------------------------------------------------------------
// RemoteRepo — common representation of a remote repository
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RemoteRepo {
    pub name: String,
    pub owner: String,
    pub is_fork: bool,
    pub is_archived: bool,
    #[allow(dead_code)]
    pub is_mirror: bool,
    #[allow(dead_code)]
    pub clone_url: String,
    #[allow(dead_code)]
    pub source: RemoteSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSource {
    GitHub,
    Forgejo,
}

impl RemoteRepo {
    /// Unique key for deduplication: (owner, name) in lowercase.
    pub fn dedup_key(&self) -> (String, String) {
        (self.owner.to_lowercase(), self.name.to_lowercase())
    }
}

// ---------------------------------------------------------------------------
// GitHub client
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    clone_url: String,
    owner: GitHubOwner,
}

#[derive(Debug, Deserialize)]
struct GitHubOwner {
    login: String,
}

/// Fetch a GitHub token using `gh auth token`.
fn github_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("failed to run `gh auth token`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`gh auth token` failed: {}", stderr.trim());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Fetch all repos for the given GitHub owners (users or orgs).
pub fn fetch_github_repos(owners: &[String]) -> Result<Vec<RemoteRepo>> {
    let token = github_token()?;
    let mut all_repos = Vec::new();

    for owner in owners {
        let repos = fetch_github_owner_repos(&token, owner)?;
        all_repos.extend(repos);
    }

    Ok(all_repos)
}

fn fetch_github_owner_repos(token: &str, owner: &str) -> Result<Vec<RemoteRepo>> {
    let client = reqwest::blocking::Client::new();
    let mut repos = Vec::new();
    let mut page = 1u32;

    loop {
        // Try org endpoint first, fall back to user endpoint
        let url = format!(
            "https://api.github.com/orgs/{}/repos?page={}&per_page=100&type=all",
            owner, page
        );

        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "gitjuggling")
            .send()
            .context(format!(
                "failed to fetch GitHub repos for owner '{}'",
                owner
            ))?;

        // If 404, try user endpoint
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            let user_url = format!(
                "https://api.github.com/users/{}/repos?page={}&per_page=100&type=all",
                owner, page
            );

            let user_resp = client
                .get(&user_url)
                .header("Authorization", format!("Bearer {}", token))
                .header("User-Agent", "gitjuggling")
                .send()
                .context(format!("failed to fetch GitHub repos for user '{}'", owner))?;

            let page_repos: Vec<GitHubRepo> = user_resp.json()?;
            if page_repos.is_empty() {
                break;
            }

            for r in page_repos {
                repos.push(RemoteRepo {
                    name: r.name,
                    owner: r.owner.login,
                    is_fork: r.fork,
                    is_archived: r.archived,
                    is_mirror: false,
                    clone_url: r.clone_url,
                    source: RemoteSource::GitHub,
                });
            }

            page += 1;
            continue;
        }

        let page_repos: Vec<GitHubRepo> = resp.json()?;
        if page_repos.is_empty() {
            break;
        }

        for r in page_repos {
            repos.push(RemoteRepo {
                name: r.name,
                owner: r.owner.login,
                is_fork: r.fork,
                is_archived: r.archived,
                is_mirror: false,
                clone_url: r.clone_url,
                source: RemoteSource::GitHub,
            });
        }

        page += 1;
    }

    Ok(repos)
}

// ---------------------------------------------------------------------------
// Forgejo / Gitea client
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ForgejoRepo {
    name: String,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    mirror: bool,
    clone_url: String,
    owner: ForgejoOwner,
}

#[derive(Debug, Deserialize)]
struct ForgejoOwner {
    login: String,
}

/// Fetch a Forgejo token by executing the configured command.
fn forgejo_token(cmd: &str) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .context(format!("failed to execute forgejo token command: {}", cmd))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("forgejo token command failed ({}): {}", cmd, stderr.trim());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Fetch all repos for the given Forgejo user, filtering out mirrors.
pub fn fetch_forgejo_repos(base_url: &str, user: &str, token_cmd: &str) -> Result<Vec<RemoteRepo>> {
    let token = forgejo_token(token_cmd)?;
    let client = reqwest::blocking::Client::new();
    let mut repos = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "{}/api/v1/users/{}/repos?page={}&limit=50",
            base_url.trim_end_matches('/'),
            user,
            page
        );

        let resp = client
            .get(&url)
            .header("Authorization", format!("token {}", token))
            .send()
            .context(format!(
                "failed to fetch Forgejo repos for user '{}' at {}",
                user, base_url
            ))?;

        if !resp.status().is_success() {
            anyhow::bail!("Forgejo API returned {} for user '{}'", resp.status(), user);
        }

        let page_repos: Vec<ForgejoRepo> = resp.json()?;
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
                clone_url: r.clone_url,
                source: RemoteSource::Forgejo,
            });
        }

        page += 1;
    }

    Ok(repos)
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Remove Forgejo repos that duplicate GitHub repos (same owner/name).
#[allow(clippy::ptr_arg)]
pub fn dedup_repos(github: &mut Vec<RemoteRepo>, forgejo: &mut Vec<RemoteRepo>) {
    let github_keys: std::collections::HashSet<(String, String)> =
        github.iter().map(|r| r.dedup_key()).collect();

    forgejo.retain(|r| !github_keys.contains(&r.dedup_key()));
}
