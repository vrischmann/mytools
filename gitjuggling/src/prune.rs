use crate::discover::LocalRepos;
use crate::remote::RemoteRepo;
use std::path::PathBuf;

/// A local repo that has no matching upstream remote.
#[derive(Debug)]
pub struct OrphanRepo {
    pub path: PathBuf,
    pub name: String,
}

/// Find local repos that have no matching upstream remote.
pub fn find_orphans(local: &LocalRepos, remote_repos: &[RemoteRepo]) -> Vec<OrphanRepo> {
    // Build a set of normalized remote URLs for fast lookup
    let remote_urls: std::collections::HashSet<String> = remote_repos
        .iter()
        .map(|r| normalize_for_lookup(&r.clone_url))
        .collect();

    let mut orphans = Vec::new();

    for repo in &local.repos {
        // Check if any of the remote URLs match an upstream
        let has_match = repo
            .remote_urls
            .values()
            .any(|url| remote_urls.contains(&normalize_for_lookup(url)));

        if !has_match {
            let name = repo
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            orphans.push(OrphanRepo {
                path: repo.path.clone(),
                name,
            });
        }
    }

    orphans
}

/// Result of pruning a single orphan.
#[derive(Debug)]
pub struct PruneResult {
    pub name: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub success: bool,
    pub message: String,
}

/// Prune orphan repos (interactive or dry-run).
pub fn prune_orphans(orphans: &[OrphanRepo], dry_run: bool, interactive: bool) -> Vec<PruneResult> {
    let mut results = Vec::new();

    for orphan in orphans {
        if dry_run {
            results.push(PruneResult {
                name: orphan.name.clone(),
                path: orphan.path.clone(),
                success: true,
                message: "would remove".to_string(),
            });
            continue;
        }

        if interactive {
            let prompt = format!(
                "Remove orphan repo {} ({})?",
                orphan.name,
                orphan.path.display()
            );
            let confirm = dialoguer::Confirm::new()
                .with_prompt(&prompt)
                .default(false)
                .interact();

            match confirm {
                Ok(false) => {
                    results.push(PruneResult {
                        name: orphan.name.clone(),
                        path: orphan.path.clone(),
                        success: true,
                        message: "skipped (user declined)".to_string(),
                    });
                    continue;
                }
                Err(err) => {
                    results.push(PruneResult {
                        name: orphan.name.clone(),
                        path: orphan.path.clone(),
                        success: false,
                        message: format!("prompt failed: {}", err),
                    });
                    continue;
                }
                Ok(true) => {}
            }
        }

        match std::fs::remove_dir_all(&orphan.path) {
            Ok(()) => results.push(PruneResult {
                name: orphan.name.clone(),
                path: orphan.path.clone(),
                success: true,
                message: "removed".to_string(),
            }),
            Err(err) => results.push(PruneResult {
                name: orphan.name.clone(),
                path: orphan.path.clone(),
                success: false,
                message: format!("failed to remove: {}", err),
            }),
        }
    }

    results
}

/// Normalize a URL for lookup comparison.
/// Reuses the same logic as discover::normalize_url.
fn normalize_for_lookup(url: &str) -> String {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");

    // SSH form: git@host:owner/repo
    if let Some(rest) = url.strip_prefix("git@") {
        return rest.replacen(':', "/", 1);
    }

    // HTTPS form
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        return rest.to_string();
    }

    url.to_string()
}

/// Print a summary of prune results.
pub fn print_prune_summary(results: &[PruneResult]) {
    if results.is_empty() {
        println!("\n  {}", "No orphan repos found.".dimmed());
        return;
    }

    let (removed, failed): (Vec<_>, Vec<_>) = results.iter().partition(|r| r.success);

    use colored::Colorize;
    println!(
        "\n{}{}{}",
        "=== ".bright_white(),
        "Prune Summary".bright_cyan(),
        " ===".bright_white()
    );

    for r in &removed {
        println!("  {} {} ({})", "✓".green(), r.name, r.message);
    }

    for r in &failed {
        println!("  {} {} — {}", "✗".red(), r.name, r.message.red());
    }

    println!(
        "\n{} {} | {} {}",
        "Removed:".blue(),
        removed.len().to_string().bright_green(),
        "Failed:".blue(),
        failed.len().to_string().bright_red(),
    );
}
