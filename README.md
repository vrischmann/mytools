# mytools

A multi-binary Rust workspace containing personal utility tools.

## Tools

| Tool | Description |
|------|-------------|
| **gitjuggling** | Run a git command in all repositories under the current working directory |
| **git-stacked** | Visualize stacked git branches and their relationships |
| **cargo-target-clean** | Interactively find and clean Cargo target directories to free disk space |
| **git-journal** | Summarize git commits for journal entries across work and personal repos |

## Building

```bash
# Build all binaries (release mode)
cargo build --release

# Build in debug mode
cargo build --workspace
```

Binaries are output to `target/release/`.

## Development

```bash
# Run tests
cargo test --workspace

# Run clippy
cargo clippy --workspace -- -D warnings

# Run a specific binary
cargo run --release -p <package> -- <args>
```

## Installation

```bash
# Install all binaries to ~/.cargo/bin/
just install-all

# Or install individually
cargo install --path gitjuggling
cargo install --path git-stacked
cargo install --path cargo-target-clean
cargo install --path git-journal
```

## Tool Details

### gitjuggling

Run git commands across multiple repositories in parallel.

```bash
# Fetch all repos under current directory
gitjuggling fetch --all -p

# With verbose output
gitjuggling -v pull

# Control depth and concurrency
gitjuggling -d 5 -c 4 status
```

See [gitjuggling/README.md](gitjuggling/README.md) for more details.

### git-stacked

Visualize stacked branch hierarchies to manage feature branch dependencies.

```bash
cd /path/to/repo
git-stacked
```

See [git-stacked/README.md](git-stacked/README.md) for more details.

### cargo-target-clean

Find and remove Cargo build artifacts interactively using fzf.

```bash
# Scan default directory ($HOME/dev)
cargo-target-clean

# Scan a custom directory
cargo-target-clean --base-dir /path/to/projects
```

See [cargo-target-clean/README.md](cargo-target-clean/README.md) for more details.

### git-journal

Generate journal entries from git commits.

```bash
# Today's commits
git-journal

# Specific date
git-journal --date 2026-03-24

# Date range
git-journal --date "2026-03-20..2026-03-24"

# Plain format
git-journal --format plain
```

See [git-journal/README.md](git-journal/README.md) for more details.
