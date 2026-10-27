//! §6 parity harness: runs the Go oracle (built from this checkout) and the
//! Rust binary against the same seeded tree, under a fixed-size pty with
//! `NO_COLOR=1`/`TERM=dumb` and an isolated `HOME`, then diffs the
//! canonicalized transcripts and exit codes.
//!
//! Cross-binary diffs are scoped to what is API-free (§6: "API drift is the
//! hard limit"): `exec` (no remote calls at all) and a `sync --dry-run`
//! whose workspace has no owners and no Forgejo — both fetchers
//! short-circuit, so no HTTP happens on either side.
//!
//! The exec comparison runs on a tree WITHOUT linked worktrees by design:
//! the Rust port visits worktrees (decision E, §8 item 4), Go does not, so
//! worktree paths are a known, deliberate divergence — documented, not
//! diffed. The tree also carries exactly one `.gitmodules` (nearest-file and
//! last-file-wins coincide) so the decision-E lookup fix cannot surface
//! here either.
//!
//! Skips (not fails) when the Go oracle cannot be built — no `go`, or no
//! `go.mod` — which is the post-cutover state.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixed pty size for both runs, so wrapping is deterministic.
const ROWS: u32 = 24;
const COLS: u32 = 100;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent")
        .to_path_buf()
}

fn have_go() -> bool {
    Command::new("go").arg("version").output().is_ok()
}

fn build_go_oracle(tmp: &Path) -> Option<PathBuf> {
    if !have_go() || !repo_root().join("go.mod").exists() {
        return None;
    }
    let out = tmp.join("gitjuggling-go");
    let status = Command::new("go")
        .current_dir(repo_root())
        .args(["build", "-o"])
        .arg(&out)
        .arg("./gitjuggling")
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    Some(out)
}

/// Keys fed to the pty for the sync flow: both binaries pause at the plan
/// view for Enter (dry-run only skips the confirmation walk), and Enter or
/// q also dismisses the summary. Enters sent while the TUI is still
/// loading are ignored by both machines, so the feeder simply repeats
/// until one lands on the plan view. Nothing is fed for `exec`, whose
/// TUI runs to completion unattended (an early q could truncate results).
fn key_feeder(stdin: &mut std::process::ChildStdin) {
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if stdin.write_all(b"\r").is_err() {
            return;
        }
        let _ = stdin.flush();
    }
}

/// Runs `bin args...` under a pty with an isolated `HOME`, capturing the
/// transcript and exit code. macOS's BSD `script` provides the pty; the
/// inner `stty` fixes the window size for both runs.
fn run_under_pty(
    cwd: &Path,
    home: &Path,
    bin: &Path,
    args: &[&str],
    feed_keys: bool,
) -> (String, i32) {
    let transcript = home.join(format!(
        "pty-{}.log",
        bin.file_name().unwrap().to_str().unwrap_or("x")
    ));
    let _ = std::fs::remove_file(&transcript);

    let mut quoted = format!("stty rows {ROWS} cols {COLS}; exec '{}' ", bin.display());
    for arg in args {
        quoted.push_str(&format!("'{}' ", arg.replace('\'', "'\\''")));
    }

    let mut child = Command::new("script")
        .arg("-q")
        .arg(&transcript)
        .arg("sh")
        .arg("-c")
        .arg(&quoted)
        .current_dir(cwd)
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("COLUMNS", COLS.to_string())
        .env("LINES", ROWS.to_string())
        .stdin(std::process::Stdio::piped())
        // script copies the session to its own stdout too; under cargo's
        // output capture that pipe is only drained after the test, so
        // script would deadlock on it. The transcript file already has
        // everything — send script's own stdout/stderr to null.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("running script(1) for the pty");
    // script must never see stdin EOF: BSD script tears the pty down the
    // moment its own stdin closes. The handle stays open until the child
    // exits; keys are written only for the sync flow.
    let mut stdin = child.stdin.take().expect("script stdin");
    if feed_keys {
        std::thread::spawn(move || key_feeder(&mut stdin));
    } else {
        std::mem::forget(stdin);
    }
    let status = child.wait().expect("waiting for script(1)");

    let raw = std::fs::read_to_string(&transcript).unwrap_or_default();
    if std::env::var("PARITY_DEBUG").is_ok() {
        eprintln!(
            "DEBUG run: bin={} args={args:?} script_exit={:?} transcript={} bytes",
            bin.display(),
            status.code(),
            raw.len(),
        );
        let tag = bin
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("x")
            .to_owned();
        let _ = std::fs::write(format!("/tmp/parity-raw-{tag}.txt"), &raw);
    }
    (raw, status.code().unwrap_or(-1))
}

/// Strips ANSI escape sequences (CSI, OSC, single-char).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if c2 == '\u{7}' {
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
        } else if c.is_control() && c != '\n' {
            // Transcript junk from the pty wrapper: \r, \b, EOF markers,
            // bell — none of it belongs in the comparison. The line
            // separator itself obviously stays.
            continue;
        } else {
            out.push(c);
        }
    }
    out
}

const SPINNER_FRAMES: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Extracts the final frame of the transcript: everything after the LAST
/// occurrence of any anchor. The two renderers emit different intermediate
/// machinery (bubbletea redraws changed lines; the port's inline renderer
/// reprints whole frames), so only the final frame — the semantic endpoint
/// — is comparable.
fn final_segment<'a>(stripped: &'a str, anchors: &[&str]) -> &'a str {
    let mut pos = 0;
    for anchor in anchors {
        if let Some(i) = stripped.rfind(anchor) {
            pos = pos.max(i);
        }
    }
    &stripped[pos..]
}

/// The canonical form (§6): strip ANSI, one trimmed line each, drop the
/// blank/redraw noise (progress bars, spinner frames, timing lines, the
/// redraw artifacts of inline TUIs), sort, dedup.
fn canonicalize(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in strip_ansi(raw).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Progress bars and TUI redraw noise.
        if line.contains('█') || line.contains('░') {
            continue;
        }
        if SPINNER_FRAMES.iter().any(|f| line.contains(f)) {
            continue;
        }
        if line.contains("Scanning repos") {
            continue;
        }
        // Timing lines differ by definition.
        if line.starts_with("Time:") || line.starts_with("Executing") {
            continue;
        }
        lines.push(line.to_owned());
    }
    lines.sort();
    lines.dedup();
    lines
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", "/nonexistent-isolated")
        .status()
        .expect("git in fixture");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Seeds a repo at `path` with `origin` pointing at a fake URL, optionally
/// dirty.
fn seed_repo(path: &Path, origin: &str, dirty: bool) {
    std::fs::create_dir_all(path).expect("mkdir repo");
    git(path, &["init", "-q", "-b", "master", "."]);
    git(path, &["remote", "add", "origin", origin]);
    std::fs::write(path.join("README.md"), "seed\n").expect("write README");
    git(path, &["add", "."]);
    git(
        path,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "seed",
        ],
    );
    if dirty {
        std::fs::write(path.join("uncommitted.txt"), "dirty\n").expect("write dirty file");
    }
}

/// The parity tree: three repos at varied depth, one submodule-shaped skip,
/// one dirty repo — and deliberately NO linked worktrees (decision E's
/// known divergence is documented, not diffed).
fn seed_tree(root: &Path) {
    seed_repo(
        &root.join("ws").join("demo"),
        "https://example.com/vincent/demo.git",
        false,
    );
    seed_repo(
        &root.join("ws").join("nested").join("inner"),
        "https://example.com/vincent/inner.git",
        true,
    );

    let sub = root.join("ws").join("has-sub");
    seed_repo(&sub, "https://example.com/vincent/has-sub.git", false);
    std::fs::write(
        sub.join(".gitmodules"),
        "[submodule \"vendor\"]\n\tpath = vendor\n\turl = https://example.com/vincent/vendor.git\n",
    )
    .expect("write .gitmodules");
    // A submodule-shaped checkout: a plain directory with a .git file.
    let vendor = sub.join("vendor");
    std::fs::create_dir_all(&vendor).expect("mkdir vendor");
    std::fs::write(vendor.join(".git"), "gitdir: ../.git/modules/vendor\n")
        .expect("fake submodule .git");
    std::fs::write(vendor.join("README.md"), "vendor\n").expect("vendor file");
}

fn parity_home(tag: &str) -> tempfile::TempDir {
    // One temp dir per side: HOME isolation keeps the durable plan dir
    // apart so neither binary sees the other's plans/<ws>.json.
    let _ = tag;
    tempfile::tempdir().expect("tempdir")
}

fn config_with_root(home: &Path, tree: &Path) -> PathBuf {
    let path = home.join("config.yaml");
    std::fs::write(
        &path,
        format!(
            "default_workspace: personal\nworkspace:\n  personal:\n    root: {}\n    github_owners: []\n    rules:\n      base: {}/repos\n",
            tree.display(),
            tree.display()
        ),
    )
    .expect("write config");
    path
}

/// Go oracle + Rust binary, run under their own isolated HOMEs and
/// canonicalized. `None` skips the test (no Go toolchain / no module —
/// the post-cutover state).
fn both_sides(cwd: &Path, args: &[&str]) -> Option<(Vec<String>, Vec<String>, i32, i32)> {
    let go_home = parity_home("go");
    let rust_home = parity_home("rust");
    let go_bin = build_go_oracle(go_home.path())?;

    let (go_raw, go_code) = run_under_pty(cwd, go_home.path(), &go_bin, args, false);
    let (rust_raw, rust_code) = run_under_pty(
        cwd,
        rust_home.path(),
        Path::new(env!("CARGO_BIN_EXE_gitjuggling")),
        args,
        false,
    );

    let go_lines = canonicalize(final_segment(&strip_ansi(&go_raw), &["=== Summary ==="]));
    let rust_lines = canonicalize(final_segment(&strip_ansi(&rust_raw), &["=== Summary ==="]));
    if std::env::var("PARITY_DEBUG").is_ok() {
        eprintln!("DEBUG canonical go={go_lines:#?}");
        eprintln!("DEBUG canonical rust={rust_lines:#?}");
    }
    Some((go_lines, rust_lines, go_code, rust_code))
}

/// `exec -- status --porcelain` over the seeded tree: same repos, same
/// output split, same exit code, verbose and not.
#[test]
fn exec_parity_status_porcelain() {
    let tree = tempfile::tempdir().expect("tempdir");
    seed_tree(tree.path());

    for verbose in [false, true] {
        let mut args: Vec<String> = vec!["exec".to_owned()];
        if verbose {
            args.push("-v".to_owned());
        }
        args.push("--".to_owned());
        args.push("status".to_owned());
        args.push("--porcelain".to_owned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

        let Some((go_lines, rust_lines, go_code, rust_code)) = both_sides(tree.path(), &arg_refs)
        else {
            eprintln!("skipping: no Go oracle available");
            return;
        };

        assert_eq!(go_code, rust_code, "exit codes differ (verbose={verbose})");
        assert_eq!(
            go_lines, rust_lines,
            "canonicalized exec output differs (verbose={verbose})\nGo: {go_lines:#?}\nRust: {rust_lines:#?}"
        );
        // The comparison is meaningful only if a summary was reached.
        assert!(
            go_lines.iter().any(|l| l.contains("Succeeded:")),
            "no summary reached on the Go side: {go_lines:#?}"
        );
    }
}

/// `sync --dry-run --interactive=false` with an owner-less, Forgejo-less
/// workspace: both fetchers short-circuit, so the whole flow is API-free —
/// loading, local scan, plan build, dry execution, summary.
#[test]
fn sync_dry_run_parity_no_remotes() {
    let tree = tempfile::tempdir().expect("tempdir");
    seed_tree(tree.path());

    // Each side needs its own HOME (config + plan dir isolation).
    let go_home = parity_home("go");
    let rust_home = parity_home("rust");
    let go_bin = match build_go_oracle(go_home.path()) {
        Some(bin) => bin,
        None => {
            eprintln!("skipping: no Go oracle available");
            return;
        }
    };

    let go_config = config_with_root(go_home.path(), tree.path());
    let (go_raw, go_code) = run_under_pty(
        tree.path(),
        go_home.path(),
        &go_bin,
        &[
            "sync",
            "--config",
            go_config.to_str().unwrap(),
            "--dry-run",
            "--interactive=false",
        ],
        true,
    );
    let rust_config = config_with_root(rust_home.path(), tree.path());
    let (rust_raw, rust_code) = run_under_pty(
        tree.path(),
        rust_home.path(),
        Path::new(env!("CARGO_BIN_EXE_gitjuggling")),
        &[
            "sync",
            "--config",
            rust_config.to_str().unwrap(),
            "--dry-run",
            "--interactive=false",
        ],
        true,
    );

    assert_eq!(go_code, rust_code, "sync exit codes differ");
    let go_lines = canonicalize(final_segment(&strip_ansi(&go_raw), &["=== Plan ==="]));
    let rust_lines = canonicalize(final_segment(&strip_ansi(&rust_raw), &["=== Plan ==="]));
    assert_eq!(
        go_lines, rust_lines,
        "canonicalized sync dry-run output differs\nGo: {go_lines:#?}\nRust: {rust_lines:#?}"
    );
    // A summary must have been reached for the diff to mean anything.
    assert!(
        go_lines.iter().any(|l| l.contains("Sync Summary")),
        "no summary reached on the Go side: {}",
        strip_ansi(&go_raw)
    );
}
