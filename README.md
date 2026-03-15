# mytools

A multi-binary Rust workspace containing git-related utilities.

## Structure

This is a Cargo workspace with the following members:

- **gitjuggling** - Run a git command in all repositories under the current working directory
- **git-stacked** - Tool for managing stacked git branches

## Building

To build all binaries:

```bash
cargo build --release
```

The binaries will be available in `target/release/`:
- `target/release/gitjuggling`
- `target/release/git-stacked`

## Development

To build in debug mode:

```bash
cargo build --workspace
```

To run tests:

```bash
cargo test --workspace
```

To run a specific binary:

```bash
cargo run --release -p gitjuggling -- <args>
cargo run --release -p git-stacked -- <args>
```

## Individual Projects

### gitjuggling

See [gitjuggling/README.md](gitjuggling/README.md) for more details.

### git-stacked

See [git-stacked/README.md](git-stacked/README.md) for more details.
