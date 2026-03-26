# zoekt-reindex

Reindex git repositories for [zoekt](https://github.com/sourcegraph/zoekt) source code search.

## Description

Scans a directory tree for git repositories and runs `zoekt-git-index` on each one in parallel. Useful for keeping your zoekt index up to date after pulling changes across many repositories.

## Usage

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

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--zoekt-bin` | `~/go/bin/zoekt-git-index` | Path to zoekt-git-index binary |
| `--index-dir` | `~/.zoekt` | Directory where zoekt stores indexes |
| `--codebase` | `~/dev/Batch` | Root directory to scan for git repositories |
| `--depth` | `3` | Max depth to search for `.git` directories |
| `--concurrency, -c` | `2` | Number of concurrent indexing processes |
| `--config` | `~/.config/zoekt-reindex/config.toml` | Path to config file |

## Configuration

Create a config file at `~/.config/zoekt-reindex/config.toml`:

```toml
zoekt_bin = "~/go/bin/zoekt-git-index"
index_dir = "~/.zoekt"
codebase = "~/dev/Batch"
depth = 3
concurrency = 2
```

CLI arguments take precedence over config file settings.

## How It Works

1. Uses `jwalk` to parallel walk the directory tree
2. Finds `.git` directories up to the configured depth
3. Uses `rayon` to run `zoekt-git-index` on each repository in parallel
4. Reports success/failure for each repository

## Building

```bash
cargo build --release -p zoekt-reindex
```

The binary will be available at `target/release/zoekt-reindex`.

## Dependencies

- `anyhow` - Error handling
- `clap` - Command-line argument parsing
- `directories` - XDG config directory resolution
- `jwalk` - Parallel directory walking
- `rayon` - Parallel processing
- `serde` - Config file deserialization
- `toml` - TOML parsing
