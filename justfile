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

# Show available modules
list-modules:
    @echo "Available modules:"
    @echo "  - gitjuggling"
    @echo "  - git-stacked"
