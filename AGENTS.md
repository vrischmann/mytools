# Agent Guidelines

## Project Overview

This is a Cargo workspace containing personal utility tools. Each tool is a standalone binary in its own crate.

## Build Commands

```bash
# Build all (release)
cargo build --release

# Build all (debug)
cargo build --workspace

# Check + clippy
cargo check --workspace
cargo clippy --workspace -- -D warnings

# Run tests
cargo test --workspace
```

## Running Individual Tools

```bash
cargo run --release -p <package> -- <args>
```

Where `<package>` is one of: `gitjuggling`, `git-stacked`, `cargo-target-clean`, `git-journal`, `zoekt-reindex`, `ansible-password-agent`.

## Code Style

- Rust edition varies by crate (2021 or 2024)
- Follow standard Rust formatting (`cargo fmt`)
- Address all clippy warnings

## Dependencies

Each crate has its own `Cargo.toml` with independent dependencies. Common dependencies across crates:
- `clap` - CLI argument parsing
- `anyhow` - Error handling
- `git2` - Git operations (git-stacked, git-journal)
- `rayon` - Parallel processing (gitjuggling, cargo-target-clean, zoekt-reindex)
- `jwalk` - Parallel directory walking (gitjuggling, cargo-target-clean, git-journal, zoekt-reindex)

## Tool-Specific Notes

### gitjuggling
- Uses `rayon` for parallel git command execution
- Default concurrency is 2 (limited by SSH multiplexing)
- Supports submodules detection via `.gitmodules` parsing

### git-stacked
- Uses `git2` for repository inspection
- Determines branch hierarchy via merge-base calculations
- Outputs ASCII tree visualization

### cargo-target-clean
- Requires `fzf` to be installed for interactive selection
- Uses parallel directory scanning with `jwalk` and `rayon`
- Default search path is `$HOME/dev`

### git-journal
- Hardcoded paths: `~/dev/Batch` (work), `~/dev/perso` (personal)
- Default author filter: `vincent@rischmann.fr`
- Uses `chrono` for date handling

### zoekt-reindex
- Requires `zoekt-git-index` binary (from zoekt sourcegraph project)
- Config file: `~/.config/zoekt-reindex/config.toml`
- Uses `jwalk` and `rayon` for parallel repository discovery and indexing
- CLI args override config file settings

### ansible-password-agent
- Secure credential provider for Ansible vault and become passwords
- Never writes cleartext to disk
- Linux: uses kernel keyring via `linux-keyutils` (600s timeout)
- macOS: uses Keychain via `security-framework` with biometric access control
- CLI: `--type vault|become` (default: vault)
- Reads passwords from `/dev/tty` via `rpassword` (immune to stdin redirects)
