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

# Install Go project
go-install-all:
    cd gitjuggling && go install .
# Install all workspace binaries
install-all: cargo-install-all go-install-all
# Show available modules
list-modules:
    @echo "Available modules:"
    @echo "  - gitjuggling"
    @echo "  - git-stacked"
    @echo "  - cargo-target-clean"
    @echo "  - git-journal"

