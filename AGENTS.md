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

Where `<package>` is one of: `gitjuggling`, `git-stacked`, `cargo-target-clean`, `git-journal`.

## Code Style

- Rust edition varies by crate (2021 or 2024)
- Follow standard Rust formatting (`cargo fmt`)
- Address all clippy warnings

## Dependencies

Each crate has its own `Cargo.toml` with independent dependencies. Common dependencies across crates:
- `clap` - CLI argument parsing
- `anyhow` - Error handling
- `git2` - Git operations
- `rayon` - Parallel processing
- `jwalk` - Parallel directory walking

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
