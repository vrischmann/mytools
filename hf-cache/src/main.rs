use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use clap::Parser;
use colored::Colorize;
use regex::Regex;
use serde::Serialize;

// ── Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
enum FileRole {
    Weights,
    Mmproj,
    Metadata,
    Other,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(PartialEq))]
struct CachedFile {
    name: String,
    path: PathBuf,
    size: u64,
    role: FileRole,
    quant: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(PartialEq))]
struct CachedRepo {
    id: String,
    kind: String,
    refs: Vec<String>,
    files: Vec<CachedFile>,
    weights_bytes: u64,
    total_bytes: u64,
    incomplete: bool,
}

// ── CLI ─────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "hf-cache", about = "Inspect the local Hugging Face hub cache")]
struct Cli {
    /// Override the cache directory path
    #[arg(long = "cache")]
    cache_dir: Option<String>,

    /// Case-insensitive substring filter applied to repo ids
    query: Option<String>,

    /// Emit machine-readable JSON instead of the text view
    #[arg(long)]
    json: bool,
}

// ── Quant parsing ───────────────────────────────────────────────────────

fn quant_regex() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[^A-Za-z0-9])((?:(?:UD|I1)-)?(?:BF16|FP16|F16|FP32|F32|IQ[1-4](?:_[A-Z0-9]+)+|Q[2-8](?:_[A-Z0-9]+)+|TQ[12]_0))(?:[^A-Za-z0-9]|$)",
        )
        .expect("quant regex pattern should be valid")
    })
}

fn extract_quant_from_name(name: &str) -> Option<String> {
    quant_regex()
        .captures(name)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

const FORMAT_TOKENS: &[&str] = &[
    "SVDQuant", "NVFP4", "FP8", "GPTQ", "AWQ", "EXL2", "BNB", "MLC",
];

fn format_tokens_regex() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let alts: Vec<String> = FORMAT_TOKENS.iter().map(|t| regex::escape(t)).collect();
        let pattern = format!(
            r"(?i)(?:^|[^A-Za-z0-9])({})(?:[^A-Za-z0-9]|$)",
            alts.join("|")
        );
        Regex::new(&pattern).expect("format tokens regex pattern should be valid")
    })
}

fn extract_format_from_repo_id(repo_id: &str) -> Option<String> {
    let tail = repo_id.rsplit('/').next().unwrap_or(repo_id);
    format_tokens_regex()
        .captures(tail)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

// ── File / repo classification ──────────────────────────────────────────

const WEIGHT_EXTS: &[&str] = &[".gguf", ".safetensors", ".bin", ".pt", ".ckpt", ".ot"];
const METADATA_EXTS: &[&str] = &[
    ".json",
    ".jinja",
    ".txt",
    ".md",
    ".csv",
    ".gitattributes",
    ".yaml",
    ".yml",
    ".tiktoken",
    ".model",
];

fn classify_file_role(name: &str) -> FileRole {
    let lower = name.to_lowercase();
    if lower.contains("mmproj") {
        return FileRole::Mmproj;
    }
    if WEIGHT_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return FileRole::Weights;
    }
    if METADATA_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return FileRole::Metadata;
    }
    FileRole::Other
}

fn parse_repo_dir_name(dir_name: &str) -> Option<(String, String)> {
    let kind_prefixes: &[(&str, &str)] = &[
        ("models--", "model"),
        ("datasets--", "dataset"),
        ("spaces--", "space"),
    ];
    for (prefix, kind) in kind_prefixes {
        if let Some(rest) = dir_name.strip_prefix(prefix) {
            let sep = rest.find("--")?;
            let org = &rest[..sep];
            let name = &rest[sep + 2..];
            if org.is_empty() || name.is_empty() {
                return None;
            }
            return Some((kind.to_string(), format!("{org}/{name}")));
        }
    }
    None
}

// ── Formatting helpers ──────────────────────────────────────────────────

const SIZE_UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 1; // KB
    while value >= 1024.0 && unit < SIZE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if value.fract() == 0.0 || value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{:.prec$} {}", value, SIZE_UNITS[unit], prec = decimals)
}

fn file_label(repo: &CachedRepo, file: &CachedFile) -> String {
    if file.role == FileRole::Mmproj {
        return "mmproj".to_string();
    }
    if let Some(ref q) = file.quant {
        return q.clone();
    }
    if file.role == FileRole::Weights {
        return extract_format_from_repo_id(&repo.id).unwrap_or_else(|| "weights".to_string());
    }
    format!("{:?}", file.role).to_lowercase()
}

// ── Cache root resolution ───────────────────────────────────────────────

fn resolve_cache_root(override_path: Option<&str>) -> PathBuf {
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    if let Ok(p) = env::var("HF_HUB_CACHE")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(p) = env::var("HUGGINGFACE_HUB_CACHE")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(p) = env::var("HF_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p).join("hub");
    }
    if let Ok(p) = env::var("HUGGINGFACE_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p).join("hub");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".cache")
        .join("huggingface")
        .join("hub")
}

fn has_model_dirs(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let n = name.to_string_lossy();
        n.starts_with("models--") || n.starts_with("datasets--") || n.starts_with("spaces--")
    })
}

fn find_hub_root(candidate: &Path) -> PathBuf {
    if has_model_dirs(candidate) {
        return candidate.to_path_buf();
    }
    let hub_child = candidate.join("hub");
    if has_model_dirs(&hub_child) {
        return hub_child;
    }
    candidate.to_path_buf()
}

fn list_dir(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

// ── Discovery ───────────────────────────────────────────────────────────

fn discover_repos(cache_root: &Path) -> Vec<CachedRepo> {
    let root = find_hub_root(cache_root);
    let entries = list_dir(&root);
    let mut repos = Vec::new();

    for entry in entries {
        let Some((kind, repo_id)) = parse_repo_dir_name(&entry) else {
            continue;
        };
        let repo_dir = root.join(&entry);
        let refs_dir = repo_dir.join("refs");
        let snapshots_dir = repo_dir.join("snapshots");

        let ref_names = list_dir(&refs_dir);
        let mut ref_map: HashMap<String, String> = HashMap::new();
        for ref_name in &ref_names {
            if let Ok(hash) = fs::read_to_string(refs_dir.join(ref_name)) {
                ref_map.insert(ref_name.clone(), hash.trim().to_string());
            }
        }

        // Pick the snapshot: main ref, else any ref, else first on disk.
        let snapshot_hash = ref_map
            .get("main")
            .cloned()
            .or_else(|| ref_map.values().next().cloned())
            .or_else(|| list_dir(&snapshots_dir).into_iter().next());

        let mut snapshot_dir = snapshot_hash
            .as_deref()
            .map(|h| snapshots_dir.join(h))
            .filter(|p| p.exists());

        if snapshot_dir.is_none() {
            let snaps = list_dir(&snapshots_dir);
            if let Some(first) = snaps.into_iter().next() {
                snapshot_dir = Some(snapshots_dir.join(first));
            }
        }

        let mut files = Vec::new();
        if let Some(ref snap_dir) = snapshot_dir {
            for name in list_dir(snap_dir) {
                let path = snap_dir.join(&name);
                let size = fs::metadata(&path).map(|m| m.size()).unwrap_or(0);
                let role = classify_file_role(&name);
                let quant = extract_quant_from_name(&name);
                files.push(CachedFile {
                    name,
                    path,
                    size,
                    role,
                    quant,
                });
            }
        }

        let weights_bytes = files
            .iter()
            .filter(|f| f.role == FileRole::Weights)
            .map(|f| f.size)
            .sum();
        let total_bytes = files.iter().map(|f| f.size).sum();
        let incomplete = kind == "model" && !files.iter().any(|f| f.role == FileRole::Weights);

        repos.push(CachedRepo {
            id: repo_id,
            kind,
            refs: {
                let mut r = ref_names;
                r.sort();
                r
            },
            files: {
                files.sort_by(|a, b| a.role.cmp(&b.role).then_with(|| a.name.cmp(&b.name)));
                files
            },
            weights_bytes,
            total_bytes,
            incomplete,
        });
    }

    repos.sort_by(|a, b| a.id.cmp(&b.id));
    repos
}

// ── Rendering ───────────────────────────────────────────────────────────

fn render_text(repos: &[CachedRepo], root: &Path) -> String {
    let mut lines = Vec::new();
    let with_weights: Vec<&CachedRepo> = repos.iter().filter(|r| !r.incomplete).collect();
    let incomplete: Vec<&CachedRepo> = repos.iter().filter(|r| r.incomplete).collect();
    let total_bytes: u64 = repos.iter().map(|r| r.total_bytes).sum();
    let weights_bytes: u64 = repos.iter().map(|r| r.weights_bytes).sum();

    lines.push(format!(
        "{} {}",
        "Hugging Face cache".bold(),
        root.display().to_string().dimmed()
    ));
    lines.push(format!(
        "{}",
        format!(
            "{} repos · {} on disk · {} in weights",
            repos.len(),
            format_bytes(total_bytes),
            format_bytes(weights_bytes)
        )
        .dimmed()
    ));

    if repos.is_empty() {
        return lines.join("\n");
    }
    lines.push(String::new());

    // Compute column widths for alignment across all rendered files.
    let mut label_w = "quant".len();
    let mut size_w = "size".len();
    for r in &with_weights {
        for f in &r.files {
            if f.role == FileRole::Weights || f.role == FileRole::Mmproj {
                let label = file_label(r, f);
                label_w = label_w.max(label.len());
                let s = format_bytes(f.size);
                size_w = size_w.max(s.len());
            }
        }
    }

    for r in &with_weights {
        let refs_str = if r.refs.is_empty() {
            "no refs".to_string()
        } else {
            r.refs.join(", ")
        };
        lines.push(format!(
            "{} {} {} {}",
            r.id.bold().cyan(),
            format!("[{refs_str}]").dimmed(),
            "·".dimmed(),
            format_bytes(r.total_bytes).dimmed()
        ));

        for f in &r.files {
            if f.role != FileRole::Weights && f.role != FileRole::Mmproj {
                continue;
            }
            let label = file_label(r, f);
            let padded_label = format!("{:<width$}", label, width = label_w);
            let size_str = format!("{:>width$}", format_bytes(f.size), width = size_w);

            let label_col = if f.role == FileRole::Mmproj {
                padded_label.dimmed().to_string()
            } else {
                padded_label.green().to_string()
            };
            lines.push(format!(
                "  {}  {}  {}",
                label_col,
                size_str.dimmed(),
                f.name.dimmed()
            ));
        }
        lines.push(String::new());
    }

    if !incomplete.is_empty() {
        lines.push(
            "Incomplete (metadata only, no weights)"
                .yellow()
                .bold()
                .to_string(),
        );
        for r in &incomplete {
            let refs_str = if r.refs.is_empty() {
                "?".to_string()
            } else {
                r.refs.join(", ")
            };
            lines.push(format!(
                "  {} {}",
                r.id.yellow(),
                format!("[{refs_str}]").dimmed()
            ));
        }
    }

    lines.join("\n")
}

// ── Entry point ─────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    let requested = resolve_cache_root(cli.cache_dir.as_deref());
    if !requested.exists() {
        eprintln!(
            "{} Hugging Face cache not found at {}. {}",
            "error:".red().bold(),
            requested.display(),
            "Set HF_HUB_CACHE (or HF_HOME), or pass --cache <path>.".dimmed()
        );
        std::process::exit(1);
    }

    let all = discover_repos(&requested);
    if all.is_empty() {
        eprintln!(
            "{} No cached models found in {}. {}",
            "error:".red().bold(),
            requested.display(),
            "Expected directories named like models--<org>--<repo>.".dimmed()
        );
        std::process::exit(1);
    }

    let filtered: Vec<CachedRepo> = if let Some(ref query) = cli.query {
        let q = query.to_lowercase();
        all.into_iter()
            .filter(|r| r.id.to_lowercase().contains(&q))
            .collect()
    } else {
        all
    };

    if cli.json {
        let root = find_hub_root(&requested);
        let output = serde_json::json!({
            "cacheRoot": root,
            "repos": filtered,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("JSON serialization should not fail")
        );
        return;
    }

    if filtered.is_empty() {
        println!(
            "{}",
            format!("No repos match \"{}\".", cli.query.unwrap_or_default()).yellow()
        );
        return;
    }

    let root = find_hub_root(&requested);
    println!("{}", render_text(&filtered, &root));
}
