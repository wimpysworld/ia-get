#!/usr/bin/env bash
set -euo pipefail

VERSION=$(git describe --tags --match 'v*.*.*' --always --dirty 2>/dev/null || echo "dev")
VERSION="${VERSION#v}"
BUILD_VERSION="${VERSION}" cargo build --workspace --locked
