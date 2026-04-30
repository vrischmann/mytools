use crate::sync_plan::Action;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

/// Result of executing a single action.
#[derive(Debug)]
pub struct ActionResult {
    pub description: String,
    pub success: bool,
    pub message: String,
}

/// Options for action execution.
pub struct ExecuteOptions {
    pub dry_run: bool,
    pub interactive: bool,
    pub concurrency: usize,
}

/// Execute a list of actions.
pub fn execute_actions(actions: &[Action], opts: &ExecuteOptions) -> Vec<ActionResult> {
    if actions.is_empty() {
        return vec![];
    }

    // Build rayon pool with concurrency limit
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.concurrency)
        .build()
        .unwrap();

    let pb = Arc::new(ProgressBar::new(actions.len() as u64));
    pb.set_style(
        ProgressStyle::with_template("  [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("█░"),
    );

    let results: Vec<ActionResult> = pool.install(|| {
        actions
            .par_iter()
            .map(|action| {
                let pb = Arc::clone(&pb);
                let desc = action_description(action);
                pb.set_message(desc.clone());

                let result = if opts.dry_run {
                    dry_run_action(action)
                } else {
                    execute_action(action, opts.interactive)
                };

                pb.inc(1);
                result
            })
            .collect()
    });

    pb.finish_and_clear();
    results
}

fn action_description(action: &Action) -> String {
    match action {
        Action::Update {
            repo,
            local_path: _,
        } => {
            format!("{} {}/{}", "update".blue(), repo.owner, repo.name)
        }
        Action::Move {
            repo,
            current_path: _,
            expected_path,
        } => {
            format!(
                "{} {}/{} → {}",
                "move".yellow(),
                repo.owner,
                repo.name,
                expected_path.display()
            )
        }
        Action::Clone {
            repo,
            expected_path: _,
        } => {
            format!("{} {}/{}", "clone".green(), repo.owner, repo.name)
        }
    }
}

fn dry_run_action(action: &Action) -> ActionResult {
    match action {
        Action::Update { repo, local_path } => ActionResult {
            description: format!("{}/{}", repo.owner, repo.name),
            success: true,
            message: format!(
                "would update: {} (stash + pull --rebase)",
                local_path.display()
            ),
        },
        Action::Move {
            repo,
            current_path,
            expected_path,
        } => ActionResult {
            description: format!("{}/{}", repo.owner, repo.name),
            success: true,
            message: format!(
                "would move: {} → {}",
                current_path.display(),
                expected_path.display()
            ),
        },
        Action::Clone {
            repo,
            expected_path,
        } => ActionResult {
            description: format!("{}/{}", repo.owner, repo.name),
            success: true,
            message: format!(
                "would clone: {} → {}",
                repo.clone_url,
                expected_path.display()
            ),
        },
    }
}

fn execute_action(action: &Action, interactive: bool) -> ActionResult {
    match action {
        Action::Update { repo, local_path } => execute_update(repo, local_path),
        Action::Move {
            repo,
            current_path,
            expected_path,
        } => execute_move(repo, current_path, expected_path, interactive),
        Action::Clone {
            repo,
            expected_path,
        } => execute_clone(repo, expected_path),
    }
}

fn execute_update(repo: &crate::remote::RemoteRepo, local_path: &Path) -> ActionResult {
    let desc = format!("{}/{}", repo.owner, repo.name);

    // git stash -u
    let stash = Command::new("git")
        .args(["stash", "-u"])
        .current_dir(local_path)
        .output();

    match stash {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return ActionResult {
                    description: desc,
                    success: false,
                    message: format!("git stash failed: {}", stderr.trim()),
                };
            }
        }
        Err(err) => {
            return ActionResult {
                description: desc,
                success: false,
                message: format!("failed to run git stash: {}", err),
            };
        }
    }

    // git pull --rebase
    let pull = Command::new("git")
        .args(["pull", "--rebase"])
        .current_dir(local_path)
        .output();

    match pull {
        Ok(output) => {
            if output.status.success() {
                ActionResult {
                    description: desc,
                    success: true,
                    message: "updated".to_string(),
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ActionResult {
                    description: desc,
                    success: false,
                    message: format!("git pull --rebase failed: {}", stderr.trim()),
                }
            }
        }
        Err(err) => ActionResult {
            description: desc,
            success: false,
            message: format!("failed to run git pull: {}", err),
        },
    }
}

fn execute_move(
    repo: &crate::remote::RemoteRepo,
    current_path: &Path,
    expected_path: &Path,
    interactive: bool,
) -> ActionResult {
    let desc = format!("{}/{}", repo.owner, repo.name);

    if interactive {
        let prompt = format!(
            "Move {} from {} to {}?",
            desc,
            current_path.display(),
            expected_path.display()
        );
        let confirm = dialoguer::Confirm::new()
            .with_prompt(&prompt)
            .default(true)
            .interact();

        match confirm {
            Ok(false) => {
                return ActionResult {
                    description: desc,
                    success: true,
                    message: "skipped (user declined)".to_string(),
                };
            }
            Err(err) => {
                return ActionResult {
                    description: desc,
                    success: false,
                    message: format!("interactive prompt failed: {}", err),
                };
            }
            Ok(true) => {}
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = expected_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return ActionResult {
                description: desc,
                success: false,
                message: format!(
                    "failed to create parent directory {}: {}",
                    parent.display(),
                    err
                ),
            };
        }
    }

    match std::fs::rename(current_path, expected_path) {
        Ok(()) => ActionResult {
            description: desc,
            success: true,
            message: "moved".to_string(),
        },
        Err(err) => ActionResult {
            description: desc,
            success: false,
            message: format!("failed to move: {}", err),
        },
    }
}

fn execute_clone(repo: &crate::remote::RemoteRepo, expected_path: &Path) -> ActionResult {
    let desc = format!("{}/{}", repo.owner, repo.name);

    // Ensure parent directory exists
    if let Some(parent) = expected_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return ActionResult {
                description: desc,
                success: false,
                message: format!(
                    "failed to create parent directory {}: {}",
                    parent.display(),
                    err
                ),
            };
        }
    }

    let output = Command::new("git")
        .args(["clone", &repo.clone_url])
        .arg(expected_path)
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                ActionResult {
                    description: desc,
                    success: true,
                    message: "cloned".to_string(),
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ActionResult {
                    description: desc,
                    success: false,
                    message: format!("git clone failed: {}", stderr.trim()),
                }
            }
        }
        Err(err) => ActionResult {
            description: desc,
            success: false,
            message: format!("failed to run git clone: {}", err),
        },
    }
}

/// Print a summary of action results.
pub fn print_summary(results: &[ActionResult]) {
    let (succeeded, failed): (Vec<_>, Vec<_>) = results.iter().partition(|r| r.success);

    println!(
        "\n{}{}{}",
        "=== ".bright_white(),
        "Sync Summary".bright_cyan(),
        " ===".bright_white()
    );

    if !succeeded.is_empty() {
        println!("\n{}", "Succeeded:".green());
        for r in &succeeded {
            println!("  {} {}", "✓".green(), r.description);
            if !r.message.is_empty()
                && r.message != "updated"
                && r.message != "cloned"
                && r.message != "moved"
            {
                println!("    {}", r.message.dimmed());
            }
        }
    }

    if !failed.is_empty() {
        println!("\n{}", "Failed:".red());
        for r in &failed {
            println!("  {} {}", "✗".red(), r.description);
            println!("    {}", r.message.red());
        }
    }

    println!(
        "\n{} {} | {} {}",
        "Succeeded:".blue(),
        succeeded.len().to_string().bright_green(),
        "Failed:".blue(),
        failed.len().to_string().bright_red(),
    );
}
