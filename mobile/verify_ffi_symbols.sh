#!/bin/bash
# Verification Script for FFI Symbol Fix
# This script verifies that all FFI symbols are properly exported
# in both debug and release builds.

set -e  # Exit on error

echo "=================================="
echo "FFI Symbol Export Verification"
echo "=================================="
echo ""

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Build both debug and release
echo "Building mobile FFI library..."
echo "  - Debug build..."
cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib --quiet

echo "  - Release build..."
cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib --release --quiet

echo -e "${GREEN}✓${NC} Build successful"
echo ""

# Expected symbols
EXPECTED_SYMBOLS=(
    "ia_get_fetch_metadata"
    "ia_get_download_file"
    "ia_get_decompress_file"
    "ia_get_validate_checksum"
    "ia_get_last_error"
    "ia_get_free_string"
    "ia_get_mobile_version"
    "ia_get_mobile_supported_archs"
)

# Check symbols in release build
echo "Verifying symbols in release build..."
RELEASE_LIB="mobile/rust-ffi/target/release/libia_get_mobile.so"

if [ ! -f "$RELEASE_LIB" ]; then
    echo -e "${RED}✗${NC} Release library not found: $RELEASE_LIB"
    exit 1
fi

MISSING_SYMBOLS=0
for symbol in "${EXPECTED_SYMBOLS[@]}"; do
    if nm -D "$RELEASE_LIB" 2>/dev/null | grep -q " T $symbol"; then
        echo -e "  ${GREEN}✓${NC} $symbol"
    else
        echo -e "  ${RED}✗${NC} $symbol (MISSING)"
        MISSING_SYMBOLS=$((MISSING_SYMBOLS + 1))
    fi
done

echo ""

if [ $MISSING_SYMBOLS -eq 0 ]; then
    echo -e "${GREEN}✓${NC} All symbols present in release build"
else
    echo -e "${RED}✗${NC} $MISSING_SYMBOLS symbols missing from release build"
    exit 1
fi

# Check symbols in debug build
echo ""
echo "Verifying symbols in debug build..."
DEBUG_LIB="mobile/rust-ffi/target/debug/libia_get_mobile.so"

if [ ! -f "$DEBUG_LIB" ]; then
    echo -e "${RED}✗${NC} Debug library not found: $DEBUG_LIB"
    exit 1
fi

MISSING_SYMBOLS=0
for symbol in "${EXPECTED_SYMBOLS[@]}"; do
    if nm -D "$DEBUG_LIB" 2>/dev/null | grep -q " T $symbol"; then
        echo -e "  ${GREEN}✓${NC} $symbol"
    else
        echo -e "  ${RED}✗${NC} $symbol (MISSING)"
        MISSING_SYMBOLS=$((MISSING_SYMBOLS + 1))
    fi
done

echo ""

if [ $MISSING_SYMBOLS -eq 0 ]; then
    echo -e "${GREEN}✓${NC} All symbols present in debug build"
else
    echo -e "${RED}✗${NC} $MISSING_SYMBOLS symbols missing from debug build"
    exit 1
fi

# Run tests
echo ""
echo "Running FFI tests..."
if cargo test --lib --features ffi --quiet 2>&1 | grep -q "test result: ok"; then
    echo -e "${GREEN}✓${NC} All tests passed"
else
    echo -e "${RED}✗${NC} Some tests failed"
    exit 1
fi

# Check for code quality
echo ""
echo "Running code quality checks..."
if cargo clippy --manifest-path mobile/rust-ffi/Cargo.toml --lib --quiet 2>&1 | grep -q "warning:"; then
    echo -e "${RED}✗${NC} Clippy warnings found"
    cargo clippy --manifest-path mobile/rust-ffi/Cargo.toml --lib 2>&1 | grep "warning:"
    exit 1
else
    echo -e "${GREEN}✓${NC} No clippy warnings"
fi

echo ""
echo "=================================="
echo -e "${GREEN}✓ All verifications passed!${NC}"
echo "=================================="
echo ""
echo "The FFI symbols are properly exported and ready for use."
echo ""
echo "Next steps:"
echo "  1. Build the Flutter app: cd mobile/flutter && flutter build apk"
echo "  2. Test metadata fetching in the app"
echo "  3. Verify search functionality works"
