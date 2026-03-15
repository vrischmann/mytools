use anyhow::{anyhow, Context, Result};
use clap::Parser;
use jwalk::WalkDir;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Tool to find and clean Rust Cargo target directories
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Base directory to search for cargo projects (default: $HOME/dev)
    #[arg(short, long)]
    base_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct TargetDir {
    path: PathBuf,
    size: u64,
    project_name: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let base_dir = match args.base_dir {
        Some(dir) => dir,
        None => {
            let home = std::env::var("HOME")
                .context("HOME environment variable not set")?;
            PathBuf::from(home).join("dev")
        }
    };

    if !base_dir.exists() {
        return Err(anyhow!(
            "Base directory '{}' does not exist",
            base_dir.display()
        ));
    }

    println!("Scanning '{}' for cargo target directories...", base_dir.display());

    let target_dirs = find_target_dirs(&base_dir)?;

    if target_dirs.is_empty() {
        println!("No cargo target directories found.");
        return Ok(());
    }

    // Format for fzf: path | size | project_name
    let fzf_input: String = target_dirs
        .iter()
        .map(|t| {
            let size_str = format_size(t.size);
            format!("{}\t{}\t{}", t.path.display(), size_str, t.project_name)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Run fzf with input piped via stdin
    let mut child = Command::new("fzf")
        .args([
            "-m",
            "--delimiter", "\t",
            "--with-nth", "1,2,3",
            "--tabstop", "1",
            "--header", "Select target directories to remove (TAB to multi-select, ESC to cancel)",
            "--prompt", "> ",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    // Write input to fzf's stdin
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(fzf_input.as_bytes())?;
        stdin.write_all(b"\n")?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        // User cancelled or fzf error
        if output.status.code() == Some(130) {
            println!("Cancelled.");
            return Ok(());
        }
        return Err(anyhow!("fzf exited with error: {:?}", output.status));
    }

    let selected = String::from_utf8_lossy(&output.stdout);

    // Parse all selected lines
    let selected_paths: Vec<String> = selected
        .lines()
        .filter_map(|line| line.split('\t').next())
        .map(|s| s.trim().to_string())
        .collect();

    if selected_paths.is_empty() {
        println!("No selection from fzf.");
        return Ok(());
    }

    // Find all target dirs with the selected paths
    let selected_dirs: Vec<&TargetDir> = selected_paths
        .iter()
        .filter_map(|path| {
            target_dirs
                .iter()
                .find(|t| t.path.display().to_string() == *path)
        })
        .collect();

    if selected_dirs.is_empty() {
        return Err(anyhow!("No matching target directories found"));
    }

    // Calculate total size
    let total_size: u64 = selected_dirs.iter().map(|t| t.size).sum();

    println!("\nSelected {} directories ({}):", selected_dirs.len(), format_size(total_size));
    for dir in &selected_dirs {
        println!("  - {} ({} - {})", dir.path.display(), dir.project_name, format_size(dir.size));
    }

    // Confirm deletion
    print!("Delete these directories? [y/N] ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut confirmation = String::new();
    std::io::stdin().read_line(&mut confirmation)?;

    if !confirmation.trim().to_lowercase().starts_with('y') {
        println!("Cancelled.");
        return Ok(());
    }

    // Remove all selected directories
    for dir in &selected_dirs {
        remove_dir_all::remove_dir_all(&dir.path).with_context(|| {
            format!(
                "Failed to remove directory '{}'",
                dir.path.display()
            )
        })?;
        println!("Deleted '{}' ({})", dir.path.display(), format_size(dir.size));
    }

    println!("\nFreed up {}", format_size(total_size));

    Ok(())
}

fn find_target_dirs(base_dir: &Path) -> Result<Vec<TargetDir>> {
    let base_dir = Arc::new(base_dir.to_path_buf());

    // Collect all target directories using jwalk
    let target_paths: Vec<PathBuf> = WalkDir::new(base_dir.as_ref())
        .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()))
        .skip_hidden(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name() == "target" && e.file_type().is_dir()
        })
        .map(|e| e.path())
        .collect();

    // Process in parallel with rayon
    let target_dirs: Vec<TargetDir> = target_paths
        .into_par_iter()
        .filter_map(|target_path| {
            // Check if this target directory belongs to a cargo project
            // by looking for Cargo.toml in the parent directory
            let parent = target_path.parent()?;

            if !parent.join("Cargo.toml").exists() {
                return None;
            }

            // Get project name from parent directory
            let project_name = parent
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Calculate directory size
            let size = calculate_dir_size(&target_path).ok()?;

            Some(TargetDir {
                path: target_path,
                size,
                project_name,
            })
        })
        .collect();

    Ok(target_dirs)
}

fn calculate_dir_size(path: &Path) -> Result<u64> {
    let mut total_size = 0u64;

    for entry in WalkDir::new(path)
        .skip_hidden(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(metadata) = std::fs::metadata(entry.path()) {
            total_size += metadata.len();
        }
    }

    Ok(total_size)
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_index])
}
