# Run a specific module
# Build all modules
build-all:
    @cargo build --release --workspace

# Check all modules
check-all:
    @cargo check --workspace
    @cargo clippy --workspace -- -D warnings

# Test all modules
test-all:
    @cargo test --workspace

# Clean all build artifacts
clean:
    @cargo clean

# Install all Rust workspace binaries
cargo-install-all:
    @cargo install --path git-stacked
    @cargo install --path cargo-target-clean
    @cargo install --path git-journal
    @cargo install --path gitjuggling
    @cargo install --path hf-cache
    @cargo install --path humanize

# Install all workspace binaries
install-all: cargo-install-all

# Build the Go oracle used by the parity harness (gitjuggling/tests/parity.rs).
# The Go tree is removed at cutover; until then it stays as the port's oracle.
go-build-oracle:
    go build -o /tmp/gitjuggling-go ./gitjuggling
# Show available modules
list-modules:
    @echo "Available modules:"
    @echo "  - gitjuggling"
    @echo "  - git-stacked"
    @echo "  - cargo-target-clean"
    @echo "  - git-journal"
    @echo "  - hf-cache"
    @echo "  - humanize"

