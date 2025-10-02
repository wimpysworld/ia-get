#!/bin/bash
# Quick Fix Script for Flutter Dependency Issues
# This script helps resolve common Flutter/Dart SDK version conflicts

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}  Flutter Dependency Quick Fix Script${NC}"
echo -e "${BLUE}============================================${NC}"
echo ""

# Function to print colored output
print_status() {
    local status=$1
    local message=$2
    case $status in
        "success")
            echo -e "${GREEN}✓ $message${NC}"
            ;;
        "warning")
            echo -e "${YELLOW}⚠ $message${NC}"
            ;;
        "error")
            echo -e "${RED}✗ $message${NC}"
            ;;
        "info")
            echo -e "${BLUE}ℹ $message${NC}"
            ;;
    esac
}

# Check if Flutter is installed
if ! command -v flutter &> /dev/null; then
    print_status "error" "Flutter is not installed!"
    echo ""
    echo "Please install Flutter from: https://flutter.dev/docs/get-started/install"
    echo ""
    echo "For quick installation:"
    echo "  1. Download Flutter SDK from https://flutter.dev/docs/get-started/install"
    echo "  2. Extract to a location like ~/flutter"
    echo "  3. Add to PATH: export PATH=\"\$PATH:\$HOME/flutter/bin\""
    echo "  4. Run: flutter doctor"
    exit 1
fi

print_status "success" "Flutter is installed"

# Check Flutter version
echo ""
print_status "info" "Checking Flutter version..."
FLUTTER_VERSION=$(flutter --version | grep "Flutter" | awk '{print $2}')
DART_VERSION=$(flutter --version | grep "Dart" | awk '{print $4}')

echo "  Current Flutter version: $FLUTTER_VERSION"
echo "  Current Dart version: $DART_VERSION"
echo ""

# Required versions
REQUIRED_FLUTTER="3.27.1"
REQUIRED_DART="3.8.0"

# Function to compare versions
version_ge() {
    [ "$(printf '%s\n' "$1" "$2" | sort -V | head -n1)" == "$2" ]
}

# Extract major.minor.patch from version strings
extract_version() {
    echo "$1" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1
}

FLUTTER_VERSION_CLEAN=$(extract_version "$FLUTTER_VERSION")
DART_VERSION_CLEAN=$(extract_version "$DART_VERSION")

# Check if versions meet requirements
FLUTTER_OK=false
DART_OK=false

if version_ge "$FLUTTER_VERSION_CLEAN" "$REQUIRED_FLUTTER"; then
    FLUTTER_OK=true
fi

if version_ge "$DART_VERSION_CLEAN" "$REQUIRED_DART"; then
    DART_OK=true
fi

# If versions are insufficient, upgrade Flutter
if [ "$FLUTTER_OK" = false ] || [ "$DART_OK" = false ]; then
    print_status "warning" "Flutter or Dart version is below requirements"
    echo "  Required: Flutter >= $REQUIRED_FLUTTER, Dart >= $REQUIRED_DART"
    echo ""
    print_status "info" "Upgrading Flutter to latest stable version..."
    echo ""
    
    if flutter upgrade; then
        print_status "success" "Flutter upgraded successfully"
    else
        print_status "error" "Flutter upgrade failed"
        echo ""
        echo "Please manually upgrade Flutter:"
        echo "  1. Run: flutter upgrade"
        echo "  2. Run: flutter doctor"
        echo "  3. Run this script again"
        exit 1
    fi
    
    # Re-check versions after upgrade
    FLUTTER_VERSION=$(flutter --version | grep "Flutter" | awk '{print $2}')
    DART_VERSION=$(flutter --version | grep "Dart" | awk '{print $4}')
    echo ""
    echo "  Updated Flutter version: $FLUTTER_VERSION"
    echo "  Updated Dart version: $DART_VERSION"
    echo ""
else
    print_status "success" "Flutter and Dart versions meet requirements"
fi

# Navigate to Flutter project directory
FLUTTER_DIR="mobile/flutter"
if [ ! -d "$FLUTTER_DIR" ]; then
    print_status "error" "Flutter project directory not found: $FLUTTER_DIR"
    echo "Please run this script from the root of the ia-get-cli repository"
    exit 1
fi

cd "$FLUTTER_DIR"
print_status "info" "Working in: $(pwd)"
echo ""

# Clean Flutter build artifacts
print_status "info" "Cleaning Flutter build artifacts..."
if flutter clean; then
    print_status "success" "Flutter build artifacts cleaned"
else
    print_status "warning" "Flutter clean command had issues (non-critical)"
fi

# Remove pubspec.lock to force fresh dependency resolution
if [ -f "pubspec.lock" ]; then
    print_status "info" "Removing pubspec.lock for fresh dependency resolution..."
    rm -f pubspec.lock
    print_status "success" "pubspec.lock removed"
fi

# Get Flutter dependencies
echo ""
print_status "info" "Resolving Flutter dependencies..."
echo ""
if flutter pub get; then
    echo ""
    print_status "success" "Flutter dependencies resolved successfully!"
else
    echo ""
    print_status "error" "Failed to resolve Flutter dependencies"
    echo ""
    echo "This might indicate:"
    echo "  1. Network connectivity issues"
    echo "  2. Incompatible dependency versions"
    echo "  3. Corrupted Flutter cache"
    echo ""
    echo "Try these manual steps:"
    echo "  1. Run: flutter clean"
    echo "  2. Run: flutter pub cache repair"
    echo "  3. Run: flutter pub get"
    exit 1
fi

# Run Flutter doctor to check for any issues
echo ""
print_status "info" "Running Flutter doctor to check for issues..."
echo ""
flutter doctor
echo ""

# Verify the fix with analyzer
print_status "info" "Running Flutter analyzer to verify the setup..."
echo ""
if flutter analyze --no-pub; then
    echo ""
    print_status "success" "Flutter analyzer passed - no issues found!"
else
    echo ""
    print_status "warning" "Flutter analyzer found some issues (review above)"
    echo "These may not be critical, but should be reviewed."
fi

# Return to original directory
cd - > /dev/null

echo ""
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  ✓ Flutter Dependency Fix Complete!${NC}"
echo -e "${GREEN}============================================${NC}"
echo ""
echo "Next steps:"
echo "  1. Build the Flutter app: cd mobile/flutter && flutter build apk"
echo "  2. Or use the mobile build script: ./scripts/build-mobile.sh --development"
echo ""
echo "For more help, see: TROUBLESHOOTING.md"
echo ""
