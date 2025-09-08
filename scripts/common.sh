#!/bin/bash
# Common utilities for build scripts
# Source this file in other scripts to use shared functionality

# Color definitions for consistent output formatting
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Common error handling function
error_exit() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

# Common success message function
success() {
    echo -e "${GREEN}✓ $1${NC}"
}

# Common info message function
info() {
    echo -e "${BLUE}$1${NC}"
}

# Common warning message function
warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

# Function to check if we're in the project root
check_project_root() {
    if [[ ! -f "Cargo.toml" ]]; then
        error_exit "Must be run from the ia-get project root"
    fi
}

# Function to check if a command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to check if a Rust target is installed
check_rust_target() {
    local target="$1"
    if ! rustup target list --installed | grep -q "$target"; then
        info "Installing target $target..."
        rustup target add "$target"
    fi
}