# ia-get - Just Commands

# List commands
default:
    @just --list

# Build ia-get
build:
    cargo build

# Build release binary
release:
    cargo build --release

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
