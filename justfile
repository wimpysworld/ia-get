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

# Record pages/demo.webp and pages/demo-still.png from ia-get.tape using ntrecord
record:
    #!/usr/bin/env bash
    set -euo pipefail

    for tool in ntrecord ffmpeg; do
        command -v "$tool" >/dev/null || {
            printf "just record: %s not found; enter the Nix shell with 'nix develop'\n" "$tool" >&2
            exit 1
        }
    done

    cargo build --release --locked

    repo_root="$PWD"
    tmp_dir="$(mktemp -d)"

    cleanup() {
        rm -f -- "$repo_root/.demo.mp4"
        if [[ -n "${tmp_dir:-}" && -d "$tmp_dir" ]]; then
            rm -rf -- "$tmp_dir"
        fi
    }
    trap cleanup EXIT

    mkdir -p pages

    font_env=()
    if [[ -z "${NTRECORD_FONT_PATH:-}" && -d /run/current-system/sw/share/X11/fonts ]]; then
        font_env=(NTRECORD_FONT_PATH=/run/current-system/sw/share/X11/fonts)
    fi

    env -u DIRENV_DIR -u DIRENV_FILE -u DIRENV_DIFF -u DIRENV_WATCHES \
        -u PROMPT_COMMAND -u BASH_ENV \
        PATH="$repo_root/target/release:$PATH" \
        IA_GET_DEMO_DIR="$tmp_dir" \
        "${font_env[@]}" \
        ntrecord ia-get.tape

    ffmpeg -y -v error -i "$repo_root/.demo.mp4" -c:v libwebp_anim -lossless 1 -loop 0 pages/demo.webp
