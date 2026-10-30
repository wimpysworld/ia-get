# ia-get - Just Commands

# List commands
default:
    @just --list

# Build ia-get
build:
    cargo build

# Build release binary
release-build:
    cargo build --release

# Show current version from Cargo.toml
version:
    @grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2

# List recent releases
releases:
    @git tag --sort=-creatordate | head -10

# Show what would be in the next release changelog
changelog:
    #!/usr/bin/env bash
    LATEST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
    if [ -n "$LATEST_TAG" ]; then
        echo "Changes since $LATEST_TAG:"
        git log $LATEST_TAG..HEAD --pretty=format:"* %s (%h)"
    else
        echo "No previous tags found. All commits:"
        git log --pretty=format:"* %s (%h)"
    fi

# Create a new release tag (requires VERSION=x.y.z)
release VERSION:
    #!/usr/bin/env bash
    set -e

    # Validate version format
    if ! echo "{{VERSION}}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "Error: VERSION must be in format x.y.z (e.g., 0.1.0)"
        exit 1
    fi

    # Check Cargo.toml version matches the release tag
    cargo_version=$(grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)
    if [ "$cargo_version" != "{{VERSION}}" ]; then
        echo "Error: Cargo.toml version ($cargo_version) does not match {{VERSION}}"
        exit 1
    fi

    # Check for uncommitted changes
    if ! git diff-index --quiet HEAD --; then
        echo "Error: Working directory has uncommitted changes"
        exit 1
    fi

    # Check if tag already exists locally or remotely
    if git rev-parse "{{VERSION}}" >/dev/null 2>&1; then
        echo "Error: Tag {{VERSION}} already exists locally"
        exit 1
    fi
    if git ls-remote --tags origin "refs/tags/{{VERSION}}" | grep -q "refs/tags/{{VERSION}}"; then
        echo "Error: Tag {{VERSION}} already exists on origin"
        exit 1
    fi

    echo "Creating release {{VERSION}}..."
    git tag -a "{{VERSION}}" -m "Release {{VERSION}}"
    echo "✓ Tag {{VERSION}} created"
    echo ""
    echo "To publish the release:"
    echo "  git push origin {{VERSION}}"
    echo ""
    echo "This will trigger the GitHub Actions release workflow which will:"
    echo "  - Run the sentinel-gated Builder workflow"
    echo "  - Build binaries for all platforms"
    echo "  - Build Linux .deb packages"
    echo "  - Create GitHub release with downloadable assets"

# Run tests
test:
    cargo test

# Run clippy lints
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt -- --check

# Run all checks (format, lint, test)
check: fmt-check lint test

# Clean build artifacts
clean:
    cargo clean

# Run ia-get with arguments
run *ARGS:
    cargo run -- {{ARGS}}

# Update dependencies
update:
    nix flake update
    cargo update
