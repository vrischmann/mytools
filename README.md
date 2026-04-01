# mytools

A multi-binary Rust workspace containing personal utility tools.

## Tools

| Tool | Description |
|------|-------------|
| **gitjuggling** | Run a git command in all repositories under the current working directory |
| **git-stacked** | Visualize stacked git branches and their relationships |
| **cargo-target-clean** | Interactively find and clean Cargo target directories to free disk space |
| **git-journal** | Summarize git commits for journal entries across work and personal repos |
| **zoekt-reindex** | Reindex git repositories for zoekt source code search |
| **ansible-password-agent** | Secure credential provider for Ansible vault and become passwords |

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
cargo install --path zoekt-reindex
cargo install --path ansible-password-agent
```

Pre-built binaries for `zoekt-reindex` are available from the [GitHub Releases](https://github.com/vrischmann/mytools/releases) page (Linux x86_64 and macOS ARM64).

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

### zoekt-reindex

Reindex git repositories for [zoekt](https://github.com/sourcegraph/zoekt) source code search.

```bash
# Reindex with defaults (reads config from ~/.config/zoekt-reindex/config.toml)
zoekt-reindex

# Specify options on the command line
zoekt-reindex --codebase ~/dev --index-dir ~/.zoekt --depth 3

# Control concurrency
zoekt-reindex --concurrency 4

# Use a custom config file
zoekt-reindex --config /path/to/config.toml
```

Configuration file (`~/.config/zoekt-reindex/config.toml`):
```toml
zoekt_bin = "~/go/bin/zoekt-git-index"
index_dir = "~/.zoekt"
codebase = "~/dev/Batch"
depth = 3
concurrency = 2
```

See [zoekt-reindex/README.md](zoekt-reindex/README.md) for more details.

### ansible-password-agent

Secure credential provider for Ansible vault and become passwords. Never writes cleartext to disk.

```bash
# Get vault password (default)
ansible-password-agent

# Get become (sudo) password
ansible-password-agent --type become
```

**Platform backends:**
- **Linux**: Uses kernel keyring via `linux-keyutils`. Passwords are stored in unswappable kernel memory and expire after 10 minutes.
- **macOS**: Uses Keychain Services with biometric access control (Touch ID / Face ID / device password). iCloud sync is disabled.

Designed to be used as Ansible's `--vault-password-file` or `--become-password-file`. Passwords are read from `/dev/tty` (immune to stdin redirects).
