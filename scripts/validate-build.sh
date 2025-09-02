#!/bin/bash
# Build validation script - catches issues early in the development process
set -e

echo "🔍 Build Validation Script"
echo "=========================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print status
print_status() {
    local status=$1
    local message=$2
    if [ "$status" = "success" ]; then
        echo -e "${GREEN}✅ $message${NC}"
    elif [ "$status" = "warning" ]; then
        echo -e "${YELLOW}⚠️  $message${NC}"
    else
        echo -e "${RED}❌ $message${NC}"
    fi
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
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
if command_exists cargo-audit; then
    if cargo audit --quiet; then
        print_status "success" "Security audit passed"
    else
        print_status "warning" "Security vulnerabilities found (non-blocking)"
    fi
else
    print_status "warning" "cargo-audit not installed. Install with: cargo install cargo-audit"
fi

echo ""
echo "📋 Step 9: Checking for outdated dependencies..."
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
