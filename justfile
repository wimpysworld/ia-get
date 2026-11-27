# ia-get - Just Commands

set allow-duplicate-recipes

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

# Bump Cargo.toml and Cargo.lock if needed, commit, and create a local vX.Y.Z release tag
release $VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! [[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Error: VERSION must be in format x.y.z (e.g., 0.3.0)" >&2
        exit 1
    fi

    branch="$(git branch --show-current)"
    if [[ "${branch}" != "main" ]]; then
        echo "Error: releases must be created from main (currently on '${branch:-detached HEAD}')" >&2
        exit 1
    fi

    if [[ -n "$(git status --porcelain)" ]]; then
        echo "Error: Working directory is not clean" >&2
        exit 1
    fi

    if git show-ref --tags --verify --quiet "refs/tags/v${VERSION}"; then
        echo "Error: Tag v${VERSION} already exists" >&2
        exit 1
    fi

    if git show-ref --tags --verify --quiet "refs/tags/${VERSION}"; then
        echo "Error: Tag ${VERSION} already exists" >&2
        exit 1
    fi

    read_cargo_toml_version() {
        awk '
            /^\[package\][[:space:]]*$/ { in_pkg = 1; next }
            /^\[/ { in_pkg = 0 }
            in_pkg && /^version[[:space:]]*=[[:space:]]*"/ {
                sub(/^[^"]*"/, "")
                sub(/".*$/, "")
                print
                exit
            }
        ' Cargo.toml
    }

    read_cargo_lock_version() {
        awk '
            /^\[\[package\]\][[:space:]]*$/ { in_pkg = 1; is_self = 0; next }
            /^\[/ { in_pkg = 0; is_self = 0 }
            in_pkg && /^name[[:space:]]*=[[:space:]]*"ia-get"[[:space:]]*$/ { is_self = 1; next }
            in_pkg && is_self && /^version[[:space:]]*=[[:space:]]*"/ {
                sub(/^[^"]*"/, "")
                sub(/".*$/, "")
                print
                exit
            }
        ' Cargo.lock
    }

    toml_version="$(read_cargo_toml_version)"
    lock_version="$(read_cargo_lock_version)"

    if [[ -z "${toml_version}" || -z "${lock_version}" ]]; then
        echo "Error: Could not read ia-get version from Cargo.toml or Cargo.lock" >&2
        exit 1
    fi

    bumped=false
    if [[ "${toml_version}" != "${VERSION}" || "${lock_version}" != "${VERSION}" ]]; then
        if [[ "${toml_version}" != "${VERSION}" ]]; then
            tmp_toml="$(mktemp Cargo.toml.XXXXXX)"
            trap 'rm -f -- "${tmp_toml}"' EXIT
            awk -v ver="${VERSION}" '
                /^\[package\][[:space:]]*$/ { in_pkg = 1; print; next }
                /^\[/ { in_pkg = 0 }
                in_pkg && !done && /^version[[:space:]]*=[[:space:]]*"[^"]*"/ {
                    sub(/"[^"]*"/, "\"" ver "\"")
                    done = 1
                }
                { print }
            ' Cargo.toml > "${tmp_toml}"
            mv -- "${tmp_toml}" Cargo.toml
            trap - EXIT
        fi

        cargo update --workspace --offline

        toml_after="$(read_cargo_toml_version)"
        lock_after="$(read_cargo_lock_version)"
        if [[ "${toml_after}" != "${VERSION}" || "${lock_after}" != "${VERSION}" ]]; then
            echo "Error: Version verification failed (Cargo.toml=${toml_after}, Cargo.lock=${lock_after}, expected=${VERSION})" >&2
            exit 1
        fi

        git commit --message "chore(release): bump version to ${VERSION}" -- Cargo.toml Cargo.lock
        bumped=true
    fi

    just --justfile just/tailor.just --working-directory . release "${VERSION}"

    if [[ "${bumped}" == "true" ]]; then
        echo "Note: Created version bump commit on main. Push main first and wait for Builder before pushing the tag:"
        echo "  git push origin main"
        echo "  git push origin v${VERSION}"
        echo ""
    fi

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
