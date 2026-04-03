use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use clap::Parser;
use git2::{Repository, Sort};
use jwalk::WalkDir;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "git-journal")]
#[command(about = "Summarize git commits for journal entries")]
struct Args {
    /// Target date (YYYY-MM-DD). Defaults to today.
    /// Can also accept a date range in format "YYYY-MM-DD..YYYY-MM-DD".
    #[arg(long)]
    date: Option<String>,

    /// Start date for range (YYYY-MM-DD). Use with --until.
    #[arg(long)]
    since: Option<String>,

    /// End date for range (YYYY-MM-DD). Use with --since.
    #[arg(long)]
    until: Option<String>,

    /// Author email to filter commits.
    #[arg(long, default_value = "vincent@rischmann.fr")]
    author: String,

    /// Output format.
    #[arg(long, value_parser = ["journal", "plain"], default_value = "journal")]
    format: String,
}

#[derive(Debug)]
struct Commit {
    repo: String,
    #[allow(dead_code)]
    sha: String,
    message: String,
    date: NaiveDate,
}

struct Config {
    work_path: PathBuf,
    personal_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").expect("HOME not set");
        Config {
            work_path: PathBuf::from(&home).join("dev").join("Batch"),
            personal_path: PathBuf::from(&home).join("dev").join("perso"),
        }
    }
}

fn parse_date(date_str: &str) -> NaiveDate {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").expect("Invalid date format")
}

/// Parse a date range string in format "YYYY-MM-DD..YYYY-MM-DD"
fn parse_date_range(range_str: &str) -> Option<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = range_str.split("..").collect();
    if parts.len() == 2 {
        let start = parse_date(parts[0]);
        let end = parse_date(parts[1]);
        Some((start, end))
    } else {
        None
    }
}

fn get_date_range(date: NaiveDate) -> (DateTime<Local>, DateTime<Local>) {
    let since = Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
        .expect("Invalid date");
    let until = since + chrono::Duration::days(1);
    (since, until)
}

fn get_date_range_from_bounds(
    start: NaiveDate,
    end: NaiveDate,
) -> (DateTime<Local>, DateTime<Local>) {
    let since = Local
        .with_ymd_and_hms(start.year(), start.month(), start.day(), 0, 0, 0)
        .single()
        .expect("Invalid start date");
    // Add 1 day to end to make it inclusive (git uses exclusive end boundary)
    let until = Local
        .with_ymd_and_hms(end.year(), end.month(), end.day(), 0, 0, 0)
        .single()
        .expect("Invalid end date")
        + chrono::Duration::days(1);
    (since, until)
}

fn find_git_repos(base_path: &Path) -> Vec<PathBuf> {
    if !base_path.exists() {
        return Vec::new();
    }

    WalkDir::new(base_path)
        .skip_hidden(false) // Include .git directories
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir() && e.file_name().to_string_lossy() == ".git")
        .filter_map(|e| e.path().parent().map(|p| p.to_path_buf()))
        .collect()
}

fn get_commits(
    repo_path: &Path,
    author_email: &str,
    since: &DateTime<Local>,
    until: &DateTime<Local>,
) -> Vec<Commit> {
    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return Vec::new(),
    };

    if revwalk.push_head().is_err() {
        return Vec::new();
    }

    revwalk.set_sorting(Sort::TIME).ok();

    let since_ts = since.timestamp();
    let until_ts = until.timestamp();
    let repo_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut commits = Vec::new();

    for oid_result in revwalk {
        let oid = match oid_result {
            Ok(o) => o,
            Err(_) => continue,
        };

        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Skip merge commits
        if commit.parent_count() > 1 {
            continue;
        }

        // Filter by author email
        let author = commit.author();
        if author.email() != Some(author_email) {
            continue;
        }

        // Filter by time
        let commit_time = commit.time().seconds();
        if commit_time < since_ts || commit_time >= until_ts {
            continue;
        }

        // Extract the commit date
        let commit_date = Local
            .timestamp_opt(commit_time, 0)
            .single()
            .map(|dt| dt.date_naive())
            .unwrap_or_else(|| Local::now().date_naive());

        let sha = oid.to_string();
        let short_sha = if sha.len() >= 7 { &sha[..7] } else { &sha };

        let message = commit.summary().unwrap_or("").to_string();

        commits.push(Commit {
            repo: repo_name.clone(),
            sha: short_sha.to_string(),
            message,
            date: commit_date,
        });
    }

    commits
}

/// Group commits by date and format for journal output
fn format_journal_by_date(
    work_commits: &[Commit],
    personal_commits: &[Commit],
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> String {
    let mut lines = Vec::new();

    // Group work commits by date
    let mut work_by_date: BTreeMap<NaiveDate, Vec<&Commit>> = BTreeMap::new();
    for c in work_commits {
        work_by_date.entry(c.date).or_default().push(c);
    }

    // Group personal commits by date
    let mut personal_by_date: BTreeMap<NaiveDate, Vec<&Commit>> = BTreeMap::new();
    for c in personal_commits {
        personal_by_date.entry(c.date).or_default().push(c);
    }

    // Get all unique dates in range
    let mut all_dates: Vec<NaiveDate> = Vec::new();
    let mut current = start_date;
    while current <= end_date {
        all_dates.push(current);
        current += chrono::Duration::days(1);
    }

    // Output commits grouped by day (most recent first)
    for date in all_dates.iter().rev() {
        let work_for_date = work_by_date.get(date);
        let personal_for_date = personal_by_date.get(date);

        if work_for_date.is_none() && personal_for_date.is_none() {
            continue;
        }

        lines.push(format!("### {}", date.format("%Y-%m-%d")));
        lines.push(String::new());

        if let Some(work) = work_for_date {
            lines.push("#### Work".to_string());
            for c in work {
                lines.push(format!("- {}: {}", c.repo, c.message));
            }
            lines.push(String::new());
        }

        if let Some(personal) = personal_for_date {
            lines.push("#### Personal".to_string());
            for c in personal {
                lines.push(format!("- {}: {}", c.repo, c.message));
            }
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

fn format_plain(work_commits: &[Commit], personal_commits: &[Commit]) -> String {
    let mut lines = Vec::new();

    // Group by date for plain format too
    let mut by_date: BTreeMap<NaiveDate, Vec<(String, &Commit)>> = BTreeMap::new();

    for c in work_commits {
        by_date
            .entry(c.date)
            .or_default()
            .push(("work".to_string(), c));
    }
    for c in personal_commits {
        by_date
            .entry(c.date)
            .or_default()
            .push(("personal".to_string(), c));
    }

    for (date, commits) in by_date.iter().rev() {
        lines.push(format!("{}:", date.format("%Y-%m-%d")));
        for (category, c) in commits {
            lines.push(format!("  [{}] {}: {}", category, c.repo, c.message));
        }
    }

    lines.join("\n")
}

fn main() {
    let args = Args::parse();
    let config = Config::default();

    // Determine the date range to use
    let (since, until, start_date, end_date, date_label): (
        DateTime<Local>,
        DateTime<Local>,
        NaiveDate,
        NaiveDate,
        String,
    ) = if let Some(since_str) = args.since {
        // Use explicit since/until range
        let start_date = parse_date(&since_str);
        let end_date = args
            .until
            .map(|u| parse_date(&u))
            .unwrap_or_else(|| Local::now().date_naive());
        let (since_dt, until_dt) = get_date_range_from_bounds(start_date, end_date);
        let label = if start_date == end_date {
            format!("{}", start_date.format("%Y-%m-%d"))
        } else {
            format!(
                "{} to {}",
                start_date.format("%Y-%m-%d"),
                end_date.format("%Y-%m-%d")
            )
        };
        (since_dt, until_dt, start_date, end_date, label)
    } else if let Some(date_str) = args.date {
        // Check if it's a date range (contains "..")
        if let Some((start, end)) = parse_date_range(&date_str) {
            let (since_dt, until_dt) = get_date_range_from_bounds(start, end);
            let label = if start == end {
                format!("{}", start.format("%Y-%m-%d"))
            } else {
                format!("{} to {}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d"))
            };
            (since_dt, until_dt, start, end, label)
        } else {
            // Single date
            let target_date = parse_date(&date_str);
            let (since_dt, until_dt) = get_date_range(target_date);
            (
                since_dt,
                until_dt,
                target_date,
                target_date,
                format!("{}", target_date.format("%Y-%m-%d")),
            )
        }
    } else {
        // Default to today
        let target_date = Local::now().date_naive();
        let (since_dt, until_dt) = get_date_range(target_date);
        (
            since_dt,
            until_dt,
            target_date,
            target_date,
            format!("{}", target_date.format("%Y-%m-%d")),
        )
    };

    let mut work_commits = Vec::new();
    let mut personal_commits = Vec::new();

    // Find and process work repos
    for repo_path in find_git_repos(&config.work_path) {
        let commits = get_commits(&repo_path, &args.author, &since, &until);
        work_commits.extend(commits);
    }

    // Find and process personal repos
    for repo_path in find_git_repos(&config.personal_path) {
        let commits = get_commits(&repo_path, &args.author, &since, &until);
        personal_commits.extend(commits);
    }

    if work_commits.is_empty() && personal_commits.is_empty() {
        println!("No commits found for {} on {}", args.author, date_label);
        return;
    }

    let output = if args.format == "plain" {
        format_plain(&work_commits, &personal_commits)
    } else {
        format_journal_by_date(&work_commits, &personal_commits, start_date, end_date)
    };

    println!("{}", output);
}
