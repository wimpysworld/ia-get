#!/bin/bash
# Build validation script - catches issues early in the development process
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

info "🔍 Build Validation Script"
echo "=========================="

# Function to print status (using common utilities when possible)
print_status() {
    local status=$1
    local message=$2
    if [ "$status" = "success" ]; then
        success "✅ $message"
    elif [ "$status" = "warning" ]; then
        warning "⚠️  $message"
    else
        echo -e "${RED}❌ $message${NC}"
    fi
}

echo "📋 Checking prerequisites..."

# Check required tools
if ! command_exists cargo; then
    print_status "error" "cargo not found. Please install Rust."
    exit 1
fi

if ! command_exists rustc; then
    print_status "error" "rustc not found. Please install Rust."
    exit 1
fi

print_status "success" "Prerequisites check passed"

echo ""
echo "📋 Step 1: Checking code formatting..."
if cargo fmt --check --all; then
    print_status "success" "Code formatting is correct"
else
    print_status "error" "Code formatting issues found. Run 'cargo fmt' to fix."
    echo "💡 Tip: Run 'cargo fmt' to automatically fix formatting issues"
    exit 1
fi

echo ""
echo "📋 Step 2: Running clippy (CLI features)..."
if cargo clippy --no-default-features --features cli --all-targets -- -D warnings; then
    print_status "success" "Clippy (CLI) passed with no warnings"
else
    print_status "error" "Clippy (CLI) found issues"
    exit 1
fi

echo ""
echo "📋 Step 3: Running clippy (GUI features)..."
if cargo clippy --features gui --all-targets -- -D warnings; then
    print_status "success" "Clippy (GUI) passed with no warnings"
else
    print_status "error" "Clippy (GUI) found issues"
    exit 1
fi

echo ""
echo "📋 Step 4: Checking compilation (CLI)..."
if cargo check --no-default-features --features cli; then
    print_status "success" "CLI compilation successful"
else
    print_status "error" "CLI compilation failed"
    exit 1
fi

echo ""
echo "📋 Step 5: Checking compilation (GUI)..."
if cargo check --features gui; then
    print_status "success" "GUI compilation successful"
else
    print_status "error" "GUI compilation failed"
    exit 1
fi

echo ""
echo "📋 Step 6: Running tests (CLI)..."
if cargo test --no-default-features --features cli --quiet; then
    print_status "success" "CLI tests passed"
else
    print_status "error" "CLI tests failed"
    exit 1
fi

echo ""
echo "📋 Step 7: Running tests (GUI)..."
if cargo test --features gui --quiet; then
    print_status "success" "GUI tests passed"
else
    print_status "error" "GUI tests failed"
    exit 1
fi

echo ""
echo "📋 Step 8: Checking for security vulnerabilities..."
# Install cargo-audit if not present (but don't fail if it can't be installed)
if ! command_exists cargo-audit; then
    print_status "info" "Installing cargo-audit for security scanning..."
    if cargo install cargo-audit --quiet; then
        print_status "success" "cargo-audit installed successfully"
    else
        print_status "warning" "Failed to install cargo-audit. Consider running: cargo install cargo-audit"
    fi
fi

if command_exists cargo-audit; then
    if cargo audit --quiet; then
        print_status "success" "Security audit passed"
    else
        print_status "warning" "Security vulnerabilities found (non-blocking)"
    fi
else
    print_status "warning" "cargo-audit not available. Install with: cargo install cargo-audit"
fi

echo ""
echo "📋 Step 9: Checking for outdated dependencies..."
# Install cargo-outdated if not present (but don't fail if it can't be installed)
if ! command_exists cargo-outdated; then
    print_status "info" "Installing cargo-outdated for dependency checking..."
    if cargo install cargo-outdated --quiet; then
        print_status "success" "cargo-outdated installed successfully"
    else
        print_status "warning" "Failed to install cargo-outdated. Consider running: cargo install cargo-outdated"
    fi
fi

if command_exists cargo-outdated; then
    if cargo outdated --quiet; then
        print_status "success" "No major dependency updates available"
    else
        print_status "warning" "Outdated dependencies found (non-blocking)"
    fi
else
    print_status "warning" "cargo-outdated not installed. Install with: cargo install cargo-outdated"
fi

echo ""
echo "🎉 Build validation completed successfully!"
echo ""
echo "📊 Summary:"
echo "   - Code formatting: ✅"
echo "   - Clippy (CLI): ✅"
echo "   - Clippy (GUI): ✅"
echo "   - Compilation (CLI): ✅"
echo "   - Compilation (GUI): ✅"
echo "   - Tests (CLI): ✅"
echo "   - Tests (GUI): ✅"
echo ""
echo "💡 Your code is ready for commit and CI/CD pipeline!"
