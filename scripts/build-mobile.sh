#!/bin/bash
# Flutter mobile app build script (Pure Dart - No native dependencies)
# Builds Android APK or App Bundle using standard Flutter toolchain

set -e

# Parse command line arguments
BUILD_TYPE="apk"
STORE_READY=false
ENVIRONMENT="production"
FLAVOR="production"

while [[ $# -gt 0 ]]; do
    case $1 in
        --appbundle)
            BUILD_TYPE="appbundle"
            shift
            ;;
        --store-ready)
            STORE_READY=true
            shift
            ;;
        --environment=*)
            ENVIRONMENT="${1#*=}"
            FLAVOR="$ENVIRONMENT"
            shift
            ;;
        --dev|--development)
            ENVIRONMENT="development"
            FLAVOR="development"
            shift
            ;;
        --staging)
            ENVIRONMENT="staging" 
            FLAVOR="staging"
            shift
            ;;
        --production|--prod)
            ENVIRONMENT="production"
            FLAVOR="production"
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Build Flutter mobile app (Pure Dart implementation - No native dependencies)"
            echo ""
            echo "OPTIONS:"
            echo "  --appbundle              Build App Bundle instead of APK"
            echo "  --store-ready            Build with store-ready optimizations"
            echo "  --dev, --development     Build development variant"
            echo "  --staging                Build staging variant"
            echo "  --production, --prod     Build production variant (default)"
            echo "  --environment=ENV        Set environment (development|staging|production)"
            echo ""
            echo "EXAMPLES:"
            echo "  $0                       # Build production APK"
            echo "  $0 --appbundle          # Build production App Bundle"
            echo "  $0 --dev                # Build development APK"
            echo "  $0 --store-ready        # Build store-ready production App Bundle"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

success() {
    echo -e "${GREEN}✓${NC} $1"
}

error() {
    echo -e "${RED}✗${NC} $1"
}

# Configuration
MOBILE_DIR="mobile"
FLUTTER_DIR="$MOBILE_DIR/flutter"

echo ""
echo -e "${GREEN}╔═══════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   Internet Archive Helper - Flutter Build        ║${NC}"
echo -e "${GREEN}║   Pure Dart Implementation (No Native Code)      ║${NC}"
echo -e "${GREEN}╚═══════════════════════════════════════════════════╝${NC}"
echo ""

info "Build Type: $BUILD_TYPE"
info "Environment: $ENVIRONMENT"
info "Flavor: $FLAVOR"
info "Store Ready: $STORE_READY"

# Check if Flutter is available
echo ""
echo -e "${YELLOW}Step 1: Checking Flutter installation...${NC}"
if ! command -v flutter &> /dev/null; then
    error "Flutter is not installed or not in PATH"
    echo -e "${BLUE}Please install Flutter from https://flutter.dev/docs/get-started/install${NC}"
    exit 1
fi

# Display Flutter version
FLUTTER_VERSION=$(flutter --version | head -1)
success "Flutter found: $FLUTTER_VERSION"

# Navigate to Flutter directory
cd "$FLUTTER_DIR" || {
    error "Flutter directory not found: $FLUTTER_DIR"
    exit 1
}

# Get Flutter dependencies
echo ""
echo -e "${YELLOW}Step 2: Getting Flutter dependencies...${NC}"
flutter pub get
success "Dependencies retrieved"

# Run Flutter analyzer
echo ""
echo -e "${YELLOW}Step 3: Running Flutter analyzer...${NC}"
if flutter analyze; then
    success "Code analysis passed"
else
    error "Code analysis failed - please fix issues before building"
    exit 1
fi

# Build the app
echo ""
echo -e "${YELLOW}Step 4: Building Flutter app...${NC}"

BUILD_ARGS=()

# Add build number and version
BUILD_ARGS+=(--build-number=$(date +%s))

# Add flavor if not production
if [[ "$FLAVOR" != "production" ]]; then
    BUILD_ARGS+=(--flavor "$FLAVOR")
fi

# Store-ready optimizations
if [[ "$STORE_READY" == "true" ]]; then
    BUILD_ARGS+=(--obfuscate --split-debug-info=build/app/outputs/symbols)
fi

# Build based on type
if [[ "$BUILD_TYPE" == "appbundle" ]]; then
    info "Building App Bundle..."
    flutter build appbundle "${BUILD_ARGS[@]}"
    
    OUTPUT_PATH="build/app/outputs/bundle/${FLAVOR}Release/app-${FLAVOR}-release.aab"
    if [[ -f "$OUTPUT_PATH" ]]; then
        success "App Bundle built successfully"
        echo ""
        echo -e "${GREEN}Output: $OUTPUT_PATH${NC}"
    else
        error "Build completed but output file not found"
        exit 1
    fi
else
    info "Building APK..."
    flutter build apk "${BUILD_ARGS[@]}"
    
    OUTPUT_PATH="build/app/outputs/flutter-apk/app-${FLAVOR}-release.apk"
    if [[ -f "$OUTPUT_PATH" ]]; then
        success "APK built successfully"
        echo ""
        echo -e "${GREEN}Output: $OUTPUT_PATH${NC}"
        
        # Display APK size
        SIZE=$(du -h "$OUTPUT_PATH" | cut -f1)
        info "Size: $SIZE"
    else
        error "Build completed but output file not found"
        exit 1
    fi
fi

# Success message
echo ""
echo -e "${GREEN}╔═══════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          Build Completed Successfully!           ║${NC}"
echo -e "${GREEN}╚═══════════════════════════════════════════════════╝${NC}"
echo ""

if [[ "$STORE_READY" == "true" ]]; then
    echo -e "${YELLOW}Note: This is a store-ready build with:${NC}"
    echo -e "  - Code obfuscation enabled"
    echo -e "  - Debug symbols separated"
    echo -e "  - Optimized for production"
fi

echo ""
info "Next steps:"
if [[ "$BUILD_TYPE" == "appbundle" ]]; then
    echo "  1. Test the App Bundle on a device"
    echo "  2. Upload to Google Play Console"
    echo "  3. Configure release track and rollout"
else
    echo "  1. Install on device: adb install $OUTPUT_PATH"
    echo "  2. Or transfer and install manually"
fi

exit 0
