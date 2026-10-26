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

use clap::{ArgAction, Parser, Subcommand};

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

    let name = match command {
        Command::Sync { .. } => "sync",
        Command::Exec { .. } => "exec",
        Command::Init { .. } => "init",
    };

    eprintln!("error: {name} is not yet ported (see RUST_REWRITE_PLAN.md)");
    process::exit(1);
}
