use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

mod config;
mod discover;
mod execute;
mod gitmodules;
mod prune;
mod remote;
mod sync_plan;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "gitjuggling",
    disable_version_flag = true,
    about = "Repository sync and git command runner"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync local repos with upstream GitHub/Forgejo remotes
    Sync {
        /// Workspace name (uses default_workspace from config if omitted)
        workspace: Option<String>,

        /// Path to config file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Dry run: show what would be done without making changes
        #[arg(long)]
        dry_run: bool,

        /// Interactive mode: prompt before destructive actions (default true)
        #[arg(long, default_missing_value = "true", default_value = "true", num_args = 0..=1)]
        interactive: bool,

        /// Prune local repos that have no upstream match
        #[arg(long)]
        prune: bool,

        /// Concurrency limit for parallel operations
        #[arg(long, short('c'), default_value = "2")]
        concurrency: usize,
    },

    /// Run a git command in all local repositories
    Exec {
        /// Search depth for repository discovery
        #[arg(long, short('d'), default_value = "3")]
        depth: usize,

        /// Concurrency limit
        #[arg(long, short('c'), default_value = "2")]
        concurrency: usize,

        /// Show output from all repositories, not just failures
        #[arg(long, short('v'))]
        verbose: bool,

        /// Git arguments to run in each repository
        #[arg(trailing_var_arg = true, required = true)]
        git_args: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync {
            workspace,
            config,
            dry_run,
            interactive,
            prune,
            concurrency,
        } => run_sync(
            workspace.as_deref(),
            config.as_deref(),
            dry_run,
            interactive,
            prune,
            concurrency,
        ),
        Commands::Exec {
            depth,
            concurrency,
            verbose,
            git_args,
        } => run_exec(&depth, &concurrency, verbose, &git_args),
    }
}

// ---------------------------------------------------------------------------
// Sync command
// ---------------------------------------------------------------------------

fn run_sync(
    workspace_name: Option<&str>,
    config_path: Option<&Path>,
    dry_run: bool,
    interactive: bool,
    do_prune: bool,
    concurrency: usize,
) -> Result<()> {
    // 1. Load config
    let config = match config_path {
        Some(path) => config::Config::load_from(path)?,
        None => config::Config::load_default()?,
    };

    let workspace = config.get_workspace(workspace_name)?;

    println!(
        "{}{}{}",
        "=== ".bright_white(),
        format!("Syncing workspace: {}", workspace_name.unwrap_or("default")).bright_cyan(),
        " ===".bright_white()
    );

    // 2. Fetch remote repos
    let mut github_repos = Vec::new();
    let mut forgejo_repos = Vec::new();

    if !workspace.github_owners.is_empty() {
        println!("  {}Fetching GitHub repos...", "→ ".blue());
        github_repos = remote::fetch_github_repos(&workspace.github_owners)?;
        println!("  {}Found {} GitHub repos", "✓".green(), github_repos.len());
    }

    if let (Some(url), Some(user), Some(token_cmd)) = (
        &workspace.forgejo_url,
        &workspace.forgejo_user,
        &workspace.forgejo_token_cmd,
    ) {
        println!("  {}Fetching Forgejo repos...", "→ ".blue());
        forgejo_repos = remote::fetch_forgejo_repos(url, user, token_cmd)?;
        println!(
            "  {}Found {} Forgejo repos (excluding mirrors)",
            "✓".green(),
            forgejo_repos.len()
        );
    }

    // 3. Deduplicate
    remote::dedup_repos(&mut github_repos, &mut forgejo_repos);

    let mut all_remote_repos = github_repos;
    all_remote_repos.extend(forgejo_repos);

    println!(
        "  {}Total: {} remote repos after dedup",
        "→ ".blue(),
        all_remote_repos.len()
    );

    // 4. Discover local repos
    println!("  {}Scanning local repos...", "→ ".blue());
    let local = discover::LocalRepos::discover(workspace.local_scan_root())?;
    println!("  {}Found {} local repos", "✓".green(), local.repos.len());

    // 5. Build sync plan
    let actions = sync_plan::build_plan(&all_remote_repos, &local, workspace);

    let updates = actions
        .iter()
        .filter(|a| matches!(a, sync_plan::Action::Update { .. }))
        .count();
    let moves = actions
        .iter()
        .filter(|a| matches!(a, sync_plan::Action::Move { .. }))
        .count();
    let clones = actions
        .iter()
        .filter(|a| matches!(a, sync_plan::Action::Clone { .. }))
        .count();

    println!(
        "\n  {} {} to update, {} to move, {} to clone",
        "Plan:".blue(),
        updates.to_string().bright_green(),
        moves.to_string().yellow(),
        clones.to_string().cyan(),
    );

    // 6. Execute actions
    if !actions.is_empty() {
        let opts = execute::ExecuteOptions {
            dry_run,
            interactive,
            concurrency,
        };
        let results = execute::execute_actions(&actions, &opts);
        execute::print_summary(&results);
    }

    // 7. Prune (optional)
    if do_prune {
        let orphans = prune::find_orphans(&local, &all_remote_repos);
        if !orphans.is_empty() {
            println!(
                "\n  {}Found {} {}",
                "→ ".blue(),
                orphans.len().to_string().bright_yellow(),
                "orphan repos".bright_yellow()
            );
            let results = prune::prune_orphans(&orphans, dry_run, interactive);
            prune::print_prune_summary(&results);
        } else {
            println!("\n  {}", "No orphan repos found.".dimmed());
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Exec command (preserves old behavior)
// ---------------------------------------------------------------------------

struct GitOutput {
    output: std::process::Output,
}

fn do_git_command(path: &Path, args: &[&str]) -> anyhow::Result<GitOutput> {
    match std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
    {
        Ok(output) => Ok(GitOutput { output }),
        Err(err) => Err(anyhow::anyhow!(err)),
    }
}

fn parse_gitmodules(path: &Path) -> anyhow::Result<gitmodules::GitModules> {
    let contents = {
        let mut file = std::fs::File::open(path)?;
        let mut contents = String::new();
        std::io::Read::read_to_string(&mut file, &mut contents)?;
        contents
    };

    let gitmodules = gitmodules::GitModules::parse(&contents)?;
    Ok(gitmodules)
}

fn is_submodule(path: &Path, gitmodules: Option<&gitmodules::GitModules>) -> bool {
    match gitmodules {
        Some(gitmodules) => {
            let parent_path = match path.parent() {
                Some(p) => p,
                None => return false,
            };

            let tmp = parent_path
                .components()
                .next_back()
                .map(|p| PathBuf::from(p.as_os_str()))
                .unwrap_or_default();

            gitmodules.contains(&tmp)
        }
        None => false,
    }
}

fn get_repositories_paths(depth: usize) -> anyhow::Result<Vec<PathBuf>> {
    use jwalk::WalkDir;

    let mut repositories_paths = Vec::<PathBuf>::new();
    let walker = WalkDir::new(".").max_depth(depth).skip_hidden(false);
    let mut gitmodules: Option<gitmodules::GitModules> = None;

    for entry in walker {
        let entry = entry?;
        let entry_path = entry.path();

        let mut path = match entry_path.canonicalize() {
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => continue,
                _ => return Err(anyhow::anyhow!(err)),
            },
            Ok(v) => v,
        };
        let path_string = path.to_string_lossy();

        let gitmodules_path = path.join(".gitmodules");
        if gitmodules_path.exists() {
            if let Ok(tmp) = parse_gitmodules(&gitmodules_path) {
                gitmodules = Some(tmp)
            }
        }

        if !path_string.ends_with(".git") {
            continue;
        }
        if is_submodule(&path, gitmodules.as_ref()) {
            continue;
        }

        path.pop();
        repositories_paths.push(path);
    }

    Ok(repositories_paths)
}

struct Item {
    path: PathBuf,
    success: bool,
    stdout: String,
    stderr: String,
    err: Option<anyhow::Error>,
}

const STDOUT_COLOR: colored::Color = colored::Color::TrueColor {
    r: 176,
    g: 176,
    b: 176,
};

const STDERR_COLOR: colored::Color = colored::Color::TrueColor {
    r: 219,
    g: 154,
    b: 154,
};

fn run_exec(depth: &usize, concurrency: &usize, verbose: bool, git_args: &[String]) -> Result<()> {
    let git_args: Vec<&str> = git_args.iter().map(String::as_str).collect();

    rayon::ThreadPoolBuilder::new()
        .num_threads(*concurrency)
        .build_global()
        .unwrap();

    let repositories_paths = get_repositories_paths(*depth)?;

    let total = repositories_paths.len();
    let pb = Arc::new(ProgressBar::new(total as u64));
    pb.set_style(
        ProgressStyle::with_template("Processing [{bar:40}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    pb.set_message("");

    let start_time = Instant::now();

    let results: Vec<Item> = repositories_paths
        .into_par_iter()
        .map(|path| {
            let pb = Arc::clone(&pb);
            let repo_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            pb.set_message(repo_name);

            match do_git_command(&path, &git_args) {
                Err(err) => {
                    pb.inc(1);
                    Item {
                        path: path.clone(),
                        success: false,
                        stdout: String::new(),
                        stderr: String::new(),
                        err: Some(err),
                    }
                }
                Ok(go) => {
                    let stdout = String::from_utf8_lossy(&go.output.stdout)
                        .trim()
                        .to_string();
                    let stderr = String::from_utf8_lossy(&go.output.stderr)
                        .trim()
                        .to_string();

                    pb.inc(1);

                    Item {
                        path: path.clone(),
                        success: go.output.status.success(),
                        stdout,
                        stderr,
                        err: None,
                    }
                }
            }
        })
        .collect();

    pb.finish_and_clear();

    let elapsed = start_time.elapsed();
    let (succeeded, failed): (Vec<_>, Vec<_>) = results.into_iter().partition(|item| item.success);

    if verbose {
        println!(
            "\n{}{}{}\n",
            "=== ".bright_white(),
            "Output".bright_cyan(),
            " ===".bright_white()
        );

        for item in &succeeded {
            println!("{}", &item.path.to_string_lossy().to_string().green());
            if !item.stdout.is_empty() {
                println!("{}", item.stdout.color(STDOUT_COLOR));
            }
            if !item.stderr.is_empty() {
                println!("{}", item.stderr.color(STDERR_COLOR));
            }
            println!();
        }
    }

    if !failed.is_empty() {
        if !verbose {
            println!();
        }
        println!(
            "{}{}{}\n",
            "=== ".bright_white(),
            "Failed Items".bright_red(),
            " ===".bright_white()
        );

        for item in &failed {
            println!("{}", &item.path.to_string_lossy().to_string().green());

            if !item.stdout.is_empty() {
                println!("{}", item.stdout.color(STDOUT_COLOR));
            }

            if let Some(err) = &item.err {
                println!("error: {}", err);
            } else if !item.stderr.is_empty() {
                println!("{}", item.stderr.color(STDERR_COLOR));
            }
            println!();
        }
    }

    println!(
        "\n{}{}{}\n",
        "=== ".bright_white(),
        "Summary".bright_cyan(),
        " ===".bright_white()
    );

    println!(
        "{} {}",
        "Succeeded:".blue(),
        format!("{}", succeeded.len()).bright_green()
    );
    println!(
        "{} {}",
        "Failed:   ".blue(),
        format!("{}", failed.len()).bright_red()
    );
    println!(
        "{} {}s",
        "Time:     ".blue(),
        format!("{:.2}", elapsed.as_secs_f64()).bright_white()
    );

    if !failed.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}
