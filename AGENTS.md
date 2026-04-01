# Agent Guidelines

## Project Overview

This is a Cargo workspace containing personal utility tools. Each tool is a standalone binary in its own crate.

Workspace members: `gitjuggling`, `git-stacked`, `cargo-target-clean`, `git-journal`, `zoekt-reindex`, `ansible-password-agent`.

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

# Format check
cargo fmt --check --all
```

## Running Individual Tools

```bash
cargo run --release -p <package> -- <args>
```

Where `<package>` is one of: `gitjuggling`, `git-stacked`, `cargo-target-clean`, `git-journal`, `zoekt-reindex`, `ansible-password-agent`.

## Justfile Recipes

A `justfile` exists with the following recipes:
- `build-all` — Build all workspace crates in release mode
- `check-all` — Run `cargo check` and `cargo clippy` with `-D warnings`
- `test-all` — Run all workspace tests
- `clean` — Run `cargo clean`
- `install-all` — Install all workspace binaries via `cargo install --path`
- `list-modules` — List available workspace members

## Code Style

- Rust editions vary by crate: `gitjuggling`, `cargo-target-clean`, `zoekt-reindex`, `ansible-password-agent` use edition 2021; `git-stacked`, `git-journal` use edition 2024
- Follow standard Rust formatting (`cargo fmt`)
- Address all clippy warnings

## CI / Release

A GitHub Actions workflow (`.github/workflows/release.yml`) builds and releases `zoekt-reindex` binaries on tag push (`v*`). Targets: `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.

## Dependencies

Each crate has its own `Cargo.toml` with independent dependencies. Common dependencies across crates:

| Dependency | Used by | Purpose |
|-----------|---------|---------|
| `clap` | gitjuggling, cargo-target-clean, git-journal, zoekt-reindex, ansible-password-agent | CLI argument parsing (derive or builder API) |
| `anyhow` | gitjuggling, cargo-target-clean, zoekt-reindex, ansible-password-agent | Error handling |
| `rayon` | gitjuggling, cargo-target-clean, zoekt-reindex | Parallel processing |
| `jwalk` | gitjuggling, cargo-target-clean, git-journal, zoekt-reindex | Parallel directory walking |
| `git2` | git-stacked, git-journal | Git repository operations |
| `onlyerror` | gitjuggling, git-stacked | Error derive macros |
| `chrono` | git-journal | Date/time handling |
| `serde` + `toml` | zoekt-reindex | Config file deserialization |
| `rpassword` | ansible-password-agent | Terminal password input via /dev/tty |
| `linux-keyutils` | ansible-password-agent (Linux only) | Kernel keyring access |
| `security-framework` | ansible-password-agent (macOS only) | Keychain Services access |

## Tool-Specific Notes

### gitjuggling (v1.4.0, edition 2021)
- Uses builder-style `clap` (not derive)
- Uses `rayon` for parallel git command execution
- Default concurrency is 2 (limited by SSH multiplexing)
- Default search depth is 3
- Supports submodules detection via custom `.gitmodules` parser (`src/gitmodules.rs`)
- Uses `indicatif` for progress bar, `colored` for terminal output
- Has unit tests for gitmodules parser
- Has a COPR repository for Fedora installation

### git-stacked (v0.1.0, edition 2024)
- No CLI arguments — runs in the current git repository
- Uses `git2` for repository inspection
- Determines branch hierarchy via merge-base calculations
- Outputs ASCII tree visualization with colored detached branches
- Recognizes mainline branch names: `main`, `master`, `develop`, `dev`, `local-dev`

### cargo-target-clean (v0.1.0, edition 2021)
- Requires `fzf` to be installed for interactive selection
- Supports `--dry-run` flag to scan without prompting for deletion
- Uses parallel directory scanning with `jwalk` and `rayon`
- Default search path is `$HOME/dev`
- Verifies `target` directories belong to Cargo projects (checks for `Cargo.toml` in parent)

### git-journal (v0.1.0, edition 2024)
- Hardcoded paths: `~/dev/Batch` (work), `~/dev/perso` (personal)
- Default author filter: `vincent@rischmann.fr`
- Supports date selection: `--date YYYY-MM-DD`, `--date "YYYY-MM-DD..YYYY-MM-DD"`, `--since`/`--until`
- Two output formats: `journal` (markdown, default) and `plain`
- Groups commits by date, then by category (work/personal)
- Skips merge commits

### zoekt-reindex (v0.1.0, edition 2021)
- Requires `zoekt-git-index` binary (from zoekt sourcegraph project)
- Config file: `~/.config/zoekt-reindex/config.toml` (also supports `--config` flag for custom path)
- Uses `jwalk` and `rayon` for parallel repository discovery and indexing
- CLI args override config file settings
- Default codebase path is `~/dev/Batch`

### ansible-password-agent (v0.1.0, edition 2021)
- Secure credential provider for Ansible vault and become passwords
- Never writes cleartext to disk
- Multi-file source: `main.rs`, `tty.rs`, `backend/mod.rs`, `backend/linux.rs`, `backend/macos.rs`
- Linux: uses kernel keyring via `linux-keyutils` (process session keyring `@s`, 600s timeout, unswappable memory)
- macOS: uses Keychain via `security-framework` with biometric access control (USER_PRESENCE), iCloud sync disabled
- CLI: `--type vault|become` (default: vault)
- Reads passwords from `/dev/tty` via `rpassword` (immune to stdin redirects)
- Empty password input is treated as user cancellation
