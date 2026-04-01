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

# Install all workspace binaries
install-all:
    @echo "Installing gitjuggling..."
    @cargo install --path gitjuggling
    @echo "Installing git-stacked..."
    @cargo install --path git-stacked
    @echo "Installing cargo-target-clean..."
    @cargo install --path cargo-target-clean
    @echo "Installing git-journal..."
    @cargo install --path git-journal
    @echo "Installing zoekt-reindex..."
    @cargo install --path zoekt-reindex
    @echo "Installing ansible-password-agent..."
    @cargo install --path ansible-password-agent

# Show available modules
list-modules:
    @echo "Available modules:"
    @echo "  - gitjuggling"
    @echo "  - git-stacked"
    @echo "  - cargo-target-clean"
    @echo "  - git-journal"
    @echo "  - zoekt-reindex"
    @echo "  - ansible-password-agent"
