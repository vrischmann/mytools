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
    @cargo install --path ansible-password-agent

# Install Go project
go-install-all:
    cd gitjuggling && go install .
# Install all workspace binaries
install-all: cargo-install-all go-install-all
# Bump ansible-password-agent version in Cargo.toml, commit, and tag for release.
# Usage: just bump-apa 2.0.0
bump-apa version:
    @sed -i 's/^version = ".*"/version = "{{version}}"/' ansible-password-agent/Cargo.toml
    @git add ansible-password-agent/Cargo.toml
    @git commit -m "chore(ansible-password-agent): bump to {{version}}"
    @git tag ansible-password-agent/v{{version}}
    @echo "=> Bumped and tagged ansible-password-agent/v{{version}}"
    @echo "=> Push with: git push --follow-tags origin main"

# Show available modules
list-modules:
    @echo "Available modules:"
    @echo "  - gitjuggling"
    @echo "  - git-stacked"
    @echo "  - cargo-target-clean"
    @echo "  - git-journal"
    @echo "  - ansible-password-agent"
