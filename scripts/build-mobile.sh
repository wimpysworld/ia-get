#!/bin/bash
# Complete mobile app build script

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}Building IA Get Mobile App...${NC}"

# Check if we're in the right directory
if [[ ! -f "Cargo.toml" ]]; then
    echo -e "${RED}Error: Must be run from the ia-get project root${NC}"
    exit 1
fi

# Configuration
MOBILE_DIR="mobile"
RUST_FFI_DIR="$MOBILE_DIR/rust-ffi"
FLUTTER_DIR="$MOBILE_DIR/flutter"
OUTPUT_DIR="target/mobile"

# Android targets
ANDROID_TARGETS=(
    "aarch64-linux-android:arm64-v8a"
    "armv7-linux-androideabi:armeabi-v7a"
    "x86_64-linux-android:x86_64"
    "i686-linux-android:x86"
)

echo -e "${YELLOW}Step 1: Building Rust FFI library for Android...${NC}"

# Create output directories
mkdir -p "$OUTPUT_DIR/android"
mkdir -p "$FLUTTER_DIR/android/app/src/main/jniLibs"

# Build Rust library for each Android target
for target_pair in "${ANDROID_TARGETS[@]}"; do
    IFS=':' read -r rust_target android_arch <<< "$target_pair"
    
    echo -e "${BLUE}Building for ${rust_target} (${android_arch})...${NC}"
    
    # Install target if not already installed
    if ! rustup target list --installed | grep -q "$rust_target"; then
        echo -e "${BLUE}Installing target ${rust_target}...${NC}"
        rustup target add "$rust_target"
    fi
    
    # Build the FFI library
    if cargo build --target "$rust_target" --release --features ffi; then
        echo -e "${GREEN}✓ Successfully built for ${rust_target}${NC}"
        
        # Copy to Flutter Android directory
        mkdir -p "$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch"
        mkdir -p "$OUTPUT_DIR/android/$android_arch"
        
        cp "target/${rust_target}/release/libia_get.so" \
           "$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch/"
        cp "target/${rust_target}/release/libia_get.so" \
           "$OUTPUT_DIR/android/$android_arch/"
           
        echo -e "${GREEN}✓ Copied to $android_arch directory${NC}"
    else
        echo -e "${RED}✗ Failed to build for ${rust_target}${NC}"
        exit 1
    fi
done

echo -e "${YELLOW}Step 2: Generating C header file...${NC}"

# Generate header file if cbindgen is available
if command -v cbindgen &> /dev/null; then
    cbindgen --config cbindgen.toml --crate ia-get --output "$OUTPUT_DIR/ia_get.h"
    cp "$OUTPUT_DIR/ia_get.h" "$FLUTTER_DIR/android/app/src/main/cpp/"
    echo -e "${GREEN}✓ Header file generated${NC}"
else
    echo -e "${YELLOW}⚠ cbindgen not found. Install with: cargo install cbindgen${NC}"
fi

echo -e "${YELLOW}Step 3: Building mobile FFI wrapper...${NC}"

# Build the mobile wrapper library
cd "$RUST_FFI_DIR"
for target_pair in "${ANDROID_TARGETS[@]}"; do
    IFS=':' read -r rust_target android_arch <<< "$target_pair"
    
    echo -e "${BLUE}Building mobile wrapper for ${rust_target}...${NC}"
    
    if cargo build --target "$rust_target" --release; then
        # Copy wrapper library
        mkdir -p "../../$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch"
        cp "target/${rust_target}/release/libia_get_mobile.so" \
           "../../$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch/"
        echo -e "${GREEN}✓ Mobile wrapper built for ${android_arch}${NC}"
    else
        echo -e "${RED}✗ Failed to build mobile wrapper for ${rust_target}${NC}"
        exit 1
    fi
done

cd "../.."

echo -e "${YELLOW}Step 4: Preparing Flutter project...${NC}"

# Ensure Flutter directory exists and is set up
cd "$FLUTTER_DIR"

# Check if Flutter is available
if ! command -v flutter &> /dev/null; then
    echo -e "${RED}Error: Flutter is not installed or not in PATH${NC}"
    echo -e "${BLUE}Please install Flutter from https://flutter.dev/docs/get-started/install${NC}"
    exit 1
fi

# Get Flutter dependencies
echo -e "${BLUE}Getting Flutter dependencies...${NC}"
if flutter pub get; then
    echo -e "${GREEN}✓ Flutter dependencies installed${NC}"
else
    echo -e "${RED}✗ Failed to get Flutter dependencies${NC}"
    exit 1
fi

echo -e "${YELLOW}Step 5: Building Flutter APK and App Bundle...${NC}"

# Parse command line arguments for build type
BUILD_TYPE="apk"
STORE_READY=false

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
        --help)
            echo "Usage: $0 [--appbundle] [--store-ready] [--help]"
            echo "  --appbundle    Build App Bundle instead of APK"
            echo "  --store-ready  Build with store-ready optimizations"
            echo "  --help         Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option $1"
            exit 1
            ;;
    esac
done

# Build for different targets
if [[ "$BUILD_TYPE" == "appbundle" ]]; then
    echo -e "${BLUE}Building Android App Bundle for Google Play Store...${NC}"
    
    if flutter build appbundle --release; then
        echo -e "${GREEN}✓ Flutter App Bundle built successfully${NC}"
        
        # Copy App Bundle to output directory
        mkdir -p "../../../$OUTPUT_DIR"
        cp "build/app/outputs/bundle/release/app-release.aab" \
           "../../../$OUTPUT_DIR/ia-get-mobile.aab"
        echo -e "${GREEN}✓ App Bundle copied to $OUTPUT_DIR/ia-get-mobile.aab${NC}"
    else
        echo -e "${RED}✗ Failed to build Flutter App Bundle${NC}"
        exit 1
    fi
else
    echo -e "${BLUE}Building Android APK...${NC}"
    
    # Build different APK variants
    if [[ "$STORE_READY" == true ]]; then
        # Build split APKs for better optimization
        if flutter build apk --release --split-per-abi; then
            echo -e "${GREEN}✓ Split APKs built successfully${NC}"
            
            # Copy all APK variants
            mkdir -p "../../../$OUTPUT_DIR/apk-variants"
            cp build/app/outputs/flutter-apk/*.apk "../../../$OUTPUT_DIR/apk-variants/"
            echo -e "${GREEN}✓ APK variants copied to $OUTPUT_DIR/apk-variants/${NC}"
        else
            echo -e "${RED}✗ Failed to build split APKs${NC}"
            exit 1
        fi
    fi
    
    # Build universal APK
    if flutter build apk --release; then
        echo -e "${GREEN}✓ Universal APK built successfully${NC}"
        
        # Copy APK to output directory
        mkdir -p "../../../$OUTPUT_DIR"
        cp "build/app/outputs/flutter-apk/app-release.apk" \
           "../../../$OUTPUT_DIR/ia-get-mobile.apk"
        echo -e "${GREEN}✓ APK copied to $OUTPUT_DIR/ia-get-mobile.apk${NC}"
    else
        echo -e "${RED}✗ Failed to build Flutter APK${NC}"
        exit 1
    fi
fi

cd "../.."

# Build validation and testing
echo -e "${YELLOW}Step 6: Build Validation...${NC}"

# Calculate build sizes
if [[ -f "$OUTPUT_DIR/ia-get-mobile.apk" ]]; then
    APK_SIZE=$(du -h "$OUTPUT_DIR/ia-get-mobile.apk" | cut -f1)
    echo -e "${BLUE}📦 Universal APK size: $APK_SIZE${NC}"
fi

if [[ -f "$OUTPUT_DIR/ia-get-mobile.aab" ]]; then
    AAB_SIZE=$(du -h "$OUTPUT_DIR/ia-get-mobile.aab" | cut -f1)
    echo -e "${BLUE}📦 App Bundle size: $AAB_SIZE${NC}"
fi

# Validate native libraries
echo -e "${BLUE}Validating native libraries...${NC}"
ARCHS_FOUND=0
for target_pair in "${ANDROID_TARGETS[@]}"; do
    IFS=':' read -r rust_target android_arch <<< "$target_pair"
    
    if [[ -f "$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch/libia_get.so" ]]; then
        LIB_SIZE=$(du -h "$FLUTTER_DIR/android/app/src/main/jniLibs/$android_arch/libia_get.so" | cut -f1)
        echo -e "${GREEN}✓ $android_arch: $LIB_SIZE${NC}"
        ((ARCHS_FOUND++))
    else
        echo -e "${RED}✗ Missing library for $android_arch${NC}"
    fi
done

if [[ $ARCHS_FOUND -eq ${#ANDROID_TARGETS[@]} ]]; then
    echo -e "${GREEN}✓ All ${#ANDROID_TARGETS[@]} architectures present${NC}"
else
    echo -e "${YELLOW}⚠ Only $ARCHS_FOUND/${#ANDROID_TARGETS[@]} architectures found${NC}"
fi

echo -e "${GREEN}✅ Mobile app build completed successfully!${NC}"

# Output summary
echo -e "${BLUE}📱 Build Artifacts:${NC}"
if [[ -f "$OUTPUT_DIR/ia-get-mobile.apk" ]]; then
    echo -e "   APK: $OUTPUT_DIR/ia-get-mobile.apk ($APK_SIZE)"
fi
if [[ -f "$OUTPUT_DIR/ia-get-mobile.aab" ]]; then
    echo -e "   App Bundle: $OUTPUT_DIR/ia-get-mobile.aab ($AAB_SIZE)"
fi
if [[ -d "$OUTPUT_DIR/apk-variants" ]]; then
    echo -e "   Split APKs: $OUTPUT_DIR/apk-variants/"
fi
echo -e "   Native libs: $OUTPUT_DIR/android/"

echo ""
echo -e "${YELLOW}📋 Next Steps:${NC}"
if [[ "$BUILD_TYPE" == "appbundle" ]]; then
    echo -e "1. 🚀 Upload App Bundle to Google Play Console"
    echo -e "2. 📋 Complete store listing metadata"
    echo -e "3. 🧪 Run internal testing track"
    echo -e "4. 📢 Submit for review and publication"
else
    echo -e "1. 📱 Install APK: adb install $OUTPUT_DIR/ia-get-mobile.apk"
    echo -e "2. 🧪 Test on physical device or emulator"
    echo -e "3. 🔄 Run: $0 --appbundle --store-ready (for store submission)"
fi

echo ""
echo -e "${BLUE}🔧 Development Commands:${NC}"
echo -e "   Flutter hot reload: cd $FLUTTER_DIR && flutter run"
echo -e "   Run tests: cd $FLUTTER_DIR && flutter test"
echo -e "   Analyze code: cd $FLUTTER_DIR && flutter analyze"