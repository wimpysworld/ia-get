# ia-get - Just Commands

import 'just/loader.just'

# List commands
default:
    @just --list

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
