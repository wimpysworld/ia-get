#!/bin/bash
# Build script for Android cross-compilation

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Building ia-get for Android targets...${NC}"

# Android targets to build for
TARGETS=(
    "aarch64-linux-android"
    "armv7-linux-androideabi" 
    "x86_64-linux-android"
    "i686-linux-android"
)

# Create output directory
mkdir -p target/android

# Build for each target
for target in "${TARGETS[@]}"; do
    echo -e "${BLUE}Building for ${target}...${NC}"
    
    # Check if target is installed
    if ! rustup target list --installed | grep -q "$target"; then
        echo -e "${BLUE}Installing target ${target}...${NC}"
        rustup target add "$target"
    fi
    
    # Build the library
    if cargo build --target "$target" --release --features ffi; then
        echo -e "${GREEN}✓ Successfully built for ${target}${NC}"
        
        # Copy library to organized output directory
        case "$target" in
            "aarch64-linux-android")
                mkdir -p target/android/arm64-v8a
                cp "target/${target}/release/libia_get.so" target/android/arm64-v8a/
                ;;
            "armv7-linux-androideabi")
                mkdir -p target/android/armeabi-v7a
                cp "target/${target}/release/libia_get.so" target/android/armeabi-v7a/
                ;;
            "x86_64-linux-android")
                mkdir -p target/android/x86_64
                cp "target/${target}/release/libia_get.so" target/android/x86_64/
                ;;
            "i686-linux-android")
                mkdir -p target/android/x86
                cp "target/${target}/release/libia_get.so" target/android/x86/
                ;;
        esac
    else
        echo -e "${RED}✗ Failed to build for ${target}${NC}"
        exit 1
    fi
done

echo -e "${GREEN}✓ All Android targets built successfully!${NC}"
echo -e "${BLUE}Libraries available in target/android/${NC}"

# Generate header file for FFI
echo -e "${BLUE}Generating C header file...${NC}"
if command -v cbindgen &> /dev/null; then
    cbindgen --config cbindgen.toml --crate ia-get --output target/android/ia_get.h
    echo -e "${GREEN}✓ Header file generated: target/android/ia_get.h${NC}"
else
    echo -e "${RED}⚠ cbindgen not found. Install with: cargo install cbindgen${NC}"
fi

echo -e "${GREEN}Build complete!${NC}"