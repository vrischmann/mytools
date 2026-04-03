use anyhow::{Context, Result};
use clap::Parser;
use directories::ProjectDirs;
use jwalk::WalkDir;
use rayon::prelude::*;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(about = "Reindex git repositories for zoekt source code search")]
struct Args {
    /// Path to zoekt-git-index binary
    #[arg(long)]
    zoekt_bin: Option<String>,

    /// Directory where zoekt stores indexes
    #[arg(long)]
    index_dir: Option<String>,

    /// Root directory to scan for git repositories
    #[arg(long)]
    codebase: Option<String>,

    /// Max depth to search for .git directories
    #[arg(long)]
    depth: Option<usize>,

    /// Number of concurrent indexing processes
    #[arg(long, short = 'c')]
    concurrency: Option<usize>,

    /// Path to config file (default: ~/.config/zoekt-reindex/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct Config {
    /// Path to zoekt-git-index binary
    zoekt_bin: Option<String>,

    /// Directory where zoekt stores indexes
    index_dir: Option<String>,

    /// Root directory to scan for git repositories
    codebase: Option<String>,

    /// Max depth to search for .git directories
    depth: Option<usize>,

    /// Number of concurrent indexing processes
    concurrency: Option<usize>,
}

impl Config {
    /// Load config from a specific file path
    fn from_file(path: &PathBuf) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))
    }

    /// Load config from default locations
    fn load_default() -> Self {
        // Try XDG config directory first
        if let Some(proj_dirs) = ProjectDirs::from("", "", "zoekt-reindex") {
            let config_path = proj_dirs.config_dir().join("config.toml");
            if config_path.exists() {
                match Self::from_file(&config_path) {
                    Ok(config) => {
                        eprintln!("Loaded config from: {}", config_path.display());
                        return config;
                    }
                    Err(e) => {
                        eprintln!("Warning: {}", e);
                    }
                }
            }
        }

        // Try local directory
        let local_config = PathBuf::from(".zoekt-reindex.toml");
        if local_config.exists() {
            match Self::from_file(&local_config) {
                Ok(config) => {
                    eprintln!("Loaded config from: {}", local_config.display());
                    return config;
                }
                Err(e) => {
                    eprintln!("Warning: {}", e);
                }
            }
        }

        Self::default()
    }

    /// Merge CLI args into config (CLI takes precedence)
    fn merge_with_args(self, args: Args) -> MergedConfig {
        MergedConfig {
            zoekt_bin: args
                .zoekt_bin
                .or(self.zoekt_bin)
                .unwrap_or_else(|| "~/go/bin/zoekt-git-index".to_string()),
            index_dir: args
                .index_dir
                .or(self.index_dir)
                .unwrap_or_else(|| "~/.zoekt".to_string()),
            codebase: args
                .codebase
                .or(self.codebase)
                .unwrap_or_else(|| "~/dev/Batch".to_string()),
            depth: args.depth.or(self.depth).unwrap_or(3),
            concurrency: args.concurrency.or(self.concurrency).unwrap_or(2),
        }
    }
}

struct MergedConfig {
    zoekt_bin: String,
    index_dir: String,
    codebase: String,
    depth: usize,
    concurrency: usize,
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(path)
}

fn get_repositories_paths(codebase: &PathBuf, depth: usize) -> Vec<PathBuf> {
    WalkDir::new(codebase)
        .max_depth(depth)
        .skip_hidden(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str() == Some(".git") && e.file_type().is_dir())
        .filter_map(|e| e.path().parent().map(|p| p.to_path_buf()))
        .collect()
}

struct IndexResult {
    repo: PathBuf,
    success: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Load config
    let config = if let Some(config_path) = &args.config {
        Config::from_file(config_path)?
    } else {
        Config::load_default()
    };

    // Merge config with CLI args
    let merged = config.merge_with_args(args);

    let zoekt_bin = expand_tilde(&merged.zoekt_bin);
    let index_dir = expand_tilde(&merged.index_dir);
    let codebase = expand_tilde(&merged.codebase);

    println!("Indexing repos under: {}", codebase.display());

    // Setup rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(merged.concurrency)
        .build_global()
        .context("Failed to build rayon thread pool")?;

    // Collect all repo paths
    let repos = get_repositories_paths(&codebase, merged.depth);
    println!("Found {} repositories", repos.len());

    // Parallel indexing
    let results: Vec<IndexResult> = repos
        .into_par_iter()
        .map(|repo| {
            println!("Indexing: {}", repo.display());
            let status = Command::new(&zoekt_bin)
                .arg("-index")
                .arg(&index_dir)
                .arg(&repo)
                .status();

            IndexResult {
                repo,
                success: status.map(|s| s.success()).unwrap_or(false),
            }
        })
        .collect();

    let succeeded = results.iter().filter(|r| r.success).count();
    let failed = results.len() - succeeded;

    for result in results.iter().filter(|r| !r.success) {
        eprintln!("Failed to index: {}", result.repo.display());
    }

    println!(
        "Done. Indexed {} repositories ({} failed).",
        succeeded, failed
    );
    Ok(())
}
