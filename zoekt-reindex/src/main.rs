use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;
use std::path::PathBuf;
use std::process::Command;
use jwalk::WalkDir;

#[derive(Parser)]
#[command(about = "Reindex git repositories for zoekt source code search")]
struct Args {
    /// Path to zoekt-git-index binary
    #[arg(long, default_value = "~/go/bin/zoekt-git-index")]
    zoekt_bin: String,

    /// Directory where zoekt stores indexes
    #[arg(long, default_value = "~/.zoekt")]
    index_dir: String,

    /// Root directory to scan for git repositories
    #[arg(long, default_value = "~/dev/Batch")]
    codebase: String,

    /// Max depth to search for .git directories
    #[arg(long, default_value = "3")]
    depth: usize,

    /// Number of concurrent indexing processes
    #[arg(long, short = 'c', default_value = "2")]
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

    let zoekt_bin = expand_tilde(&args.zoekt_bin);
    let index_dir = expand_tilde(&args.index_dir);
    let codebase = expand_tilde(&args.codebase);

    println!("Indexing repos under: {}", codebase.display());

    // Setup rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(args.concurrency)
        .build_global()
        .context("Failed to build rayon thread pool")?;

    // Collect all repo paths
    let repos = get_repositories_paths(&codebase, args.depth);
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
