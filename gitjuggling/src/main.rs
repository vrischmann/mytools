//! M0 crate skeleton (RUST_REWRITE_PLAN.md §5): the cobra CLI surface, stubbed.
//!
//! Everything below only parses arguments; every subcommand prints
//! `error: <subcommand> is not yet ported (see RUST_REWRITE_PLAN.md)` to
//! stderr and exits 1. The real handlers land in M1+.
//!
//! Cobra-parity notes (Go oracle: gitjuggling/cmd/{root,sync,exec,init}.go):
//!
//! * **Bool flag forms.** cobra/pflag bools accept the bare form
//!   (`--dry-run`) and the `=value` form (`--dry-run=false`,
//!   `--interactive=true`), but *not* a space-separated value
//!   (`--dry-run false` leaves `false` as a positional). clap reproduces
//!   this with `num_args(0..=1)` + `default_missing_value("true")` +
//!   `require_equals(true)` + `ArgAction::Set` on every bool flag.
//! * **Positional limits.** `sync` takes at most one positional
//!   (cobra `MaximumNArgs(1)`); `exec` requires at least one git argument
//!   (cobra `MinimumNArgs(1)`) and passes them through verbatim: flags are
//!   parsed before `--` (an unknown one is an error, as in cobra), and
//!   everything after `--` is positional. A plain `Vec<String>` positional
//!   gives exactly that, so neither `allow_hyphen_values` nor
//!   `trailing_var_arg` is used — those would silently swallow interspersed
//!   flags that cobra rejects.
//! * **`-c` overload.** `-c` is `--concurrency` on `sync`/`exec` but
//!   `--config` on `init`, as in Go.
//! * **Help/completion.** `-h`/`--help` and the `help` subcommand work
//!   (clap builtin, like cobra's); there is no `completion` subcommand
//!   (root.go sets `CompletionOptions.DisableDefaultCmd`; clap adds none by
//!   default) and no `--version` flag (cobra has no Version set).
//! * **Bare invocation.** cobra's root has no `Run`, so `gitjuggling` with
//!   no subcommand prints help and exits 0; replicated below.
//! * **Accepted deviation (§8 item 1).** cobra prints usage text after
//!   runtime errors, clap does not — stderr wording differs. Flag names,
//!   accepted forms, and exit codes do not: usage errors exit 1 (cobra's
//!   `main.go` does `os.Exit(1)`; clap would exit 2, so the code is mapped
//!   back below), help exits 0, and the stubs exit 1.

use std::process;

use anyhow::Context;

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use gitjuggling::{config, discover, git, plan, state, tui};

/// Repository sync and git command runner
#[derive(Parser)]
#[command(name = "gitjuggling")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

// Parsed for CLI parity; the fields stay unread until the real handlers are
// ported (M1+), hence the allow.
#[allow(dead_code)]
#[derive(Subcommand)]
enum Command {
    /// Sync local repos with upstream GitHub/Forgejo remotes
    Sync {
        /// Workspace name (defaults to `default_workspace` from the config)
        workspace: Option<String>,
        /// path to config file
        #[arg(long, default_value = "")]
        config: String,
        /// show what would be done without making changes
        #[arg(
            long,
            require_equals = true,
            num_args(0..=1),
            default_value = "false",
            default_missing_value = "true",
            action = ArgAction::Set
        )]
        dry_run: bool,
        /// prompt before destructive actions
        #[arg(
            long,
            require_equals = true,
            num_args(0..=1),
            default_value = "true",
            default_missing_value = "true",
            action = ArgAction::Set
        )]
        interactive: bool,
        /// prune local repos that have no upstream match
        #[arg(
            long,
            require_equals = true,
            num_args(0..=1),
            default_value = "false",
            default_missing_value = "true",
            action = ArgAction::Set
        )]
        prune: bool,
        /// skip git pull for repos already in the expected location
        #[arg(
            long,
            require_equals = true,
            num_args(0..=1),
            default_value = "false",
            default_missing_value = "true",
            action = ArgAction::Set
        )]
        skip_pull: bool,
        /// concurrency limit for parallel operations
        #[arg(short = 'c', long, default_value_t = 2)]
        concurrency: u32,
    },
    /// Run a git command in all local repositories
    Exec {
        /// search depth for repository discovery
        #[arg(short = 'd', long, default_value_t = 3)]
        depth: u32,
        /// concurrency limit
        #[arg(short = 'c', long, default_value_t = 2)]
        concurrency: u32,
        /// show output from all repositories, not just failures
        #[arg(
            short = 'v',
            long,
            require_equals = true,
            num_args(0..=1),
            default_value = "false",
            default_missing_value = "true",
            action = ArgAction::Set
        )]
        verbose: bool,
        /// git arguments, passed through verbatim (everything after `--`)
        #[arg(value_name = "git args", required = true)]
        args: Vec<String>,
    },
    /// Initialize a configuration file with placeholder values
    Init {
        /// path to config file (default: <UserConfigDir>/gitjuggling/config.yaml)
        #[arg(short = 'c', long, default_value = "")]
        config: String,
    },
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // cobra exits 1 on usage errors (main.go does os.Exit(1)); clap would
        // exit 2. Help output exits 0, everything else exits 1, like cobra.
        Err(err) => {
            let _ = err.print();
            process::exit(if err.use_stderr() { 1 } else { 0 });
        }
    };

    let command = match cli.command {
        Some(command) => command,
        None => {
            // cobra's root has no Run: bare `gitjuggling` prints help, exit 0.
            let _ = Cli::command().print_help();
            return;
        }
    };

    match command {
        Command::Sync {
            workspace,
            config,
            dry_run,
            interactive,
            prune,
            skip_pull,
            concurrency,
        } => match run_sync_command(
            workspace,
            &config,
            dry_run,
            interactive,
            prune,
            skip_pull,
            concurrency as usize,
        ) {
            Ok(code) => process::exit(code),
            // Pre-TUI failures exit 1 (Go: RunE error -> cobra prints,
            // main.go exits 1); wording drift is accepted (§8 item 1).
            Err(err) => {
                eprintln!("error: {err:#}");
                process::exit(1);
            }
        },
        Command::Exec {
            depth,
            concurrency,
            verbose,
            args,
        } => match run_exec_command(depth as usize, concurrency as usize, verbose, args) {
            Ok(code) => process::exit(code),
            Err(err) => {
                eprintln!("error: {err:#}");
                process::exit(1);
            }
        },
        Command::Init { config } => match run_init_command(&config) {
            Ok(()) => process::exit(0),
            Err(err) => {
                eprintln!("error: {err:#}");
                process::exit(1);
            }
        },
    }
}

/// Go: `cmd/sync.go runSync` — load config, resolve the workspace, load the
/// durable plan, run the TUI, then map to the §4.11 exit matrix: any
/// `failed` entry exits 1 (a declined tracking fix counts); a load error
/// raised *after* the TUI started exits 0 (it only reached the summary
/// view); prune failures never touch `failed`, so they exit 0.
fn run_sync_command(
    workspace: Option<String>,
    config_path: &str,
    dry_run: bool,
    interactive: bool,
    do_prune: bool,
    skip_pull: bool,
    concurrency: usize,
) -> anyhow::Result<i32> {
    let cfg = if config_path.is_empty() {
        config::load_default()
    } else {
        config::load_from(std::path::Path::new(config_path))
    }
    .context("loading config")?;

    let workspace = workspace.unwrap_or_else(|| cfg.default_workspace.clone());
    let ws = cfg
        .get_workspace(&workspace)
        .with_context(|| format!("resolving workspace {workspace:?}"))?
        .clone();

    let saved_plan = state::load(&workspace).context("loading saved sync plan")?;

    let mut app = tui::SyncApp::new(
        workspace,
        ws,
        dry_run,
        interactive,
        do_prune,
        skip_pull,
        concurrency,
        saved_plan,
    );
    let branch_exists: Box<plan::BranchExistsFn> = Box::new(git::remote_branch_exists);
    app.with_seams(std::sync::Arc::new(tui::FilePlanStore), Some(branch_exists));

    tui::run_sync(&mut app).context("running TUI")?;
    Ok(if app.has_failures() { 1 } else { 0 })
}

/// Go: `cmd/exec.go runExec` — discover repos, run the command per repo
/// under the progress TUI, exit 1 when any command failed.
fn run_exec_command(
    depth: usize,
    concurrency: usize,
    verbose: bool,
    git_args: Vec<String>,
) -> anyhow::Result<i32> {
    let paths = discover::exec_repository_paths(depth);
    if paths.is_empty() {
        println!("No repositories found.");
        return Ok(0);
    }

    let mut app = tui::ExecApp::new(git_args, verbose);
    let has_failures = tui::run_exec(&mut app, depth, concurrency).context("running TUI")?;
    Ok(if has_failures { 1 } else { 0 })
}

/// Go: `cmd/init.go runInit` with decision D: a commented YAML template
/// (replacing the anonymous-struct marshal), keeping the exists-check, the
/// 0755 directory / 0644 file modes, and the success message.
fn run_init_command(config_path: &str) -> anyhow::Result<()> {
    let path = if config_path.is_empty() {
        let dir = dirs::config_dir().context("failed to determine config directory")?;
        dir.join("gitjuggling").join("config.yaml")
    } else {
        std::path::PathBuf::from(config_path)
    };

    if path.exists() {
        anyhow::bail!("config file already exists at {}", path.display());
    }

    let dir = path
        .parent()
        .context("config path has no parent directory")?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory {}", dir.display()))?;

    let home = std::env::var("HOME").unwrap_or_default();
    let template = INIT_TEMPLATE.replace("{home}", &home);
    std::fs::write(&path, template)
        .with_context(|| format!("failed to write config file {}", path.display()))?;

    println!("Config file created at {}", path.display());
    Ok(())
}

/// The decision-D init template: same fields as Go's anonymous struct, with
/// the optional keys visible as comments.
const INIT_TEMPLATE: &str = r#"# gitjuggling configuration.
#
# default_workspace selects the workspace used by a bare `gitjuggling sync`.
default_workspace: personal

# One entry per workspace; the key is the workspace name.
workspace:
  personal:
    # Local directory scanned for repositories.
    root: {home}/dev
    # GitHub owners whose repositories are synced (case-insensitive).
    github_owners:
      - YOUR_GITHUB_USERNAME
    # Optional: GitHub token, or an op:// reference resolved via `op read`.
    # github_token: op://vault/item/github
    # Optional: Forgejo instance to sync from.
    # forgejo_url: https://git.example.com
    # forgejo_user: youruser
    # forgejo_token: op://vault/item/forgejo
    # Where each category of repository is placed.
    rules:
      base: {home}/dev/repos
      forks: {home}/dev/forks
      archived: {home}/dev/archived
      # Optional: name-pattern routing; `to` must reference a capture.
      # patterns:
      #   - pattern: '^workspace-(.+)$'
      #     to: {home}/dev/workspaces/$1
    # Optional: ignore globs matched against repo names.
    # ignore:
    #   - '.*-scratch'
"#;
