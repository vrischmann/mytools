# cargo-target-clean

A tool to find and clean Rust Cargo target directories to free up disk space.

## Description

Scans a directory tree for Rust Cargo projects and presents their `target` directories in an interactive fzf interface, showing the disk space used by each. You can then select one or more target directories to remove using multi-select.

## Usage

```bash
cargo run --release -p cargo-target-clean -- [OPTIONS]
```

### Options

- `-b, --base-dir <DIR>` - Base directory to search for cargo projects (default: `$HOME/dev`)

### Examples

Scan the default directory (`$HOME/dev`):
```bash
cargo run --release -p cargo-target-clean
```

Scan a custom directory:
```bash
cargo run --release -p cargo-target-clean -- --base-dir /path/to/projects
```

## How It Works

1. Uses `jwalk` to parallel walk the directory tree
2. Finds `target` directories that belong to Cargo projects (verified by presence of `Cargo.toml`)
3. Uses `rayon` to process results in parallel
4. Calculates the disk size of each target directory
5. Presents results in `fzf` with the following columns:
   - Path to the target directory
   - Disk size (human-readable)
   - Project name
6. Use **TAB** to multi-select directories
7. After selection, shows total size and prompts for confirmation before deletion

## Building

```bash
cargo build --release -p cargo-target-clean
```

The binary will be available at `target/release/cargo-target-clean`.

## Dependencies

- `anyhow` - Error handling
- `clap` - Command-line argument parsing
- `jwalk` - Parallel directory walking
- `rayon` - Parallel processing
- `remove_dir_all` - Directory removal
- `num_cpus` - CPU count detection
