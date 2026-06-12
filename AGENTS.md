# Agent Guidelines

## Project Overview

This is a Cargo workspace containing personal utility tools. Each tool is a standalone binary in its own crate.

Workspace members: `git-stacked`, `cargo-target-clean`, `git-journal`, `ansible-password-agent`.

Note: `gitjuggling` was ported to Go and lives in the same directory but is built separately with `go build`.

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

Where `<package>` is one of: `git-stacked`, `cargo-target-clean`, `git-journal`, `ansible-password-agent`.

For `gitjuggling`, use Go instead:

```bash
cd gitjuggling && go run . -- <args>
# or
cd gitjuggling && go build -o gitjuggling . && ./gitjuggling <args>
```

## Justfile Recipes

A `justfile` exists with the following recipes:
- `build-all` — Build all workspace crates in release mode
- `check-all` — Run `cargo check` and `cargo clippy` with `-D warnings`
- `test-all` — Run all workspace tests
- `clean` — Run `cargo clean`
- `install-all` — Install all workspace binaries via `cargo install --path`
- `list-modules` — List available workspace members

## Code Style

- Rust editions vary by crate: `cargo-target-clean`, `ansible-password-agent` use edition 2021; `git-stacked`, `git-journal` use edition 2024
- `gitjuggling` is written in Go — see `gitjuggling/` for its own build/test commands
- Follow standard Rust formatting (`cargo fmt`)
- Address all clippy warnings

## CI / Release

A GitHub Actions workflow (`.github/workflows/release.yml`) builds and releases binaries on tag push (`v*`). Targets: `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.

## Dependencies

Each crate has its own `Cargo.toml` with independent dependencies. Common dependencies across crates:

| Dependency | Used by | Purpose |
|-----------|---------|---------|
| `clap` | cargo-target-clean, git-journal, ansible-password-agent | CLI argument parsing (derive or builder API) |
| `anyhow` | cargo-target-clean, ansible-password-agent | Error handling |
| `rayon` | cargo-target-clean | Parallel processing |
| `jwalk` | cargo-target-clean, git-journal | Parallel directory walking |
| `git2` | git-stacked, git-journal | Git repository operations |
| `onlyerror` | git-stacked | Error derive macros |
| `chrono` | git-journal | Date/time handling |
| `serde` + `toml` | - | Config file deserialization |
| `rpassword` | ansible-password-agent | Terminal password input via /dev/tty |
| `linux-keyutils` | ansible-password-agent (Linux only) | Kernel keyring access |
| `security-framework` | ansible-password-agent (macOS only) | Keychain Services access |

## Tool-Specific Notes

### gitjuggling (Go)

- Ported from Rust to Go with Bubbletea TUI
- Located in `gitjuggling/` but is NOT part of the Cargo workspace
- Build with `cd gitjuggling && go build .`
- Test with `cd gitjuggling && go test ./...`
- Uses YAML config (`<UserConfigDir>/gitjuggling/config.yaml`)
- Dependencies: bubbletea, bubbles, lipgloss, cobra, yaml.v3, x/sync
- Two subcommands: `sync` (multi-phase TUI) and `exec` (progress bar TUI)
- Has its own `justfile` with build/test/check/install recipes

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

### ansible-password-agent (v0.1.0, edition 2021)
- Secure credential provider for Ansible vault and become passwords
- Never writes cleartext to disk
- Multi-file source: `main.rs`, `tty.rs`, `backend/mod.rs`, `backend/linux.rs`, `backend/macos.rs`
- Linux: uses kernel keyring via `linux-keyutils` (process session keyring `@s`, 600s timeout, unswappable memory)
- macOS: uses Keychain via `security-framework` with biometric access control (USER_PRESENCE), iCloud sync disabled
- CLI: `--type vault|become` (default: vault)
- Reads passwords from `/dev/tty` via `rpassword` (immune to stdin redirects)
- Empty password input is treated as user cancellation
