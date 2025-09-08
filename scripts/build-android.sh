#!/bin/bash
# Build script for Android cross-compilation

set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

info "Building ia-get for Android targets..."

# Check for Android NDK
if [[ -z "$ANDROID_NDK_HOME" ]]; then
    error_exit "ANDROID_NDK_HOME environment variable is not set
Please install Android NDK and set ANDROID_NDK_HOME
Example: export ANDROID_NDK_HOME=\$ANDROID_HOME/ndk/27.3.13750724"
fi

if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
    error_exit "Android NDK directory not found: $ANDROID_NDK_HOME"
fi

# Set Android API level (minimum supported version)
ANDROID_API_LEVEL=${ANDROID_API_LEVEL:-21}

# Detect host platform and architecture for NDK prebuilt toolchain
HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"
case "$HOST_OS" in
    Linux)
        if [[ "$HOST_ARCH" == "x86_64" ]]; then
            NDK_HOST="linux-x86_64"
        elif [[ "$HOST_ARCH" == "aarch64" || "$HOST_ARCH" == "arm64" ]]; then
            NDK_HOST="linux-arm64"
        else
            error_exit "Unsupported Linux architecture: $HOST_ARCH"
        fi
        ;;
    Darwin)
        if [[ "$HOST_ARCH" == "x86_64" ]]; then
            NDK_HOST="darwin-x86_64"
        elif [[ "$HOST_ARCH" == "arm64" ]]; then
            NDK_HOST="darwin-arm64"
        else
            error_exit "Unsupported macOS architecture: $HOST_ARCH"
        fi
        ;;
    *)
        error_exit "Unsupported host OS: $HOST_OS"
        ;;
esac
NDK_BIN_DIR="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST/bin"

if [[ ! -d "$NDK_BIN_DIR" ]]; then
    error_exit "NDK toolchain directory not found: $NDK_BIN_DIR"
fi

success "Using Android NDK: $ANDROID_NDK_HOME"
success "Android API level: $ANDROID_API_LEVEL"

# Function to get compiler prefix for each target
get_compiler_prefix() {
    local target="$1"
    case "$target" in
        "aarch64")
            echo "aarch64-linux-android"
            ;;
        "armv7a")
            echo "armv7a-linux-androideabi"
            ;;
        "x86_64")
            echo "x86_64-linux-android"
            ;;
        "i686")
            echo "i686-linux-android"
            ;;
        *)
            echo "Unknown target: $target" >&2
            return 1
            ;;
    esac
}

# Function to get Rust target name from short target name
get_rust_target() {
    local target="$1"
    case "$target" in
        "aarch64")
            echo "aarch64-linux-android"
            ;;
        "armv7a")
            echo "armv7-linux-androideabi"
            ;;
        "x86_64")
            echo "x86_64-linux-android"
            ;;
        "i686")
            echo "i686-linux-android"
            ;;
        *)
            echo "Unknown target: $target" >&2
            return 1
            ;;
    esac
}

# Function to get Android ABI name from target name
get_android_abi() {
    local target="$1"
    case "$target" in
        "aarch64")
            echo "arm64-v8a"
            ;;
        "armv7a")
            echo "armeabi-v7a"
            ;;
        "x86_64")
            echo "x86_64"
            ;;
        "i686")
            echo "x86"
            ;;
        *)
            echo "Unknown target: $target" >&2
            return 1
            ;;
    esac
}

# Configure cross-compilation environment variables
for target in aarch64 armv7a x86_64 i686; do
    compiler_prefix=$(get_compiler_prefix "$target")
    rust_target=$(get_rust_target "$target")
    
    # Set CC, CXX, and AR variables
    export "CC_${rust_target//-/_}"="$NDK_BIN_DIR/${compiler_prefix}${ANDROID_API_LEVEL}-clang"
    export "CXX_${rust_target//-/_}"="$NDK_BIN_DIR/${compiler_prefix}${ANDROID_API_LEVEL}-clang++"
    export "AR_${rust_target//-/_}"="$NDK_BIN_DIR/llvm-ar"
    
    # Set Cargo linker variables
    rust_target_upper=$(echo "$rust_target" | tr '[:lower:]' '[:upper:]' | tr '-' '_')
    export "CARGO_TARGET_${rust_target_upper}_LINKER"="$NDK_BIN_DIR/${compiler_prefix}${ANDROID_API_LEVEL}-clang"
done

# Verify compilers exist
for target in aarch64 armv7a x86_64 i686; do
    compiler_prefix=$(get_compiler_prefix "$target")
    compiler="$NDK_BIN_DIR/${compiler_prefix}${ANDROID_API_LEVEL}-clang"
    
    if [[ ! -f "$compiler" ]]; then
        echo -e "${RED}Error: Compiler not found: $compiler${NC}"
        echo -e "${YELLOW}Available compilers in NDK:${NC}"
        ls -1 "$NDK_BIN_DIR"/*clang | head -10
        exit 1
    fi
done

success "All required NDK compilers found"

# Android targets to build for
TARGET_NAMES=(aarch64 armv7a x86_64 i686)

# Create output directory
mkdir -p target/android

# Build for each target
for target_name in "${TARGET_NAMES[@]}"; do
    target=$(get_rust_target "$target_name")
    info "Building for $target..."
    
    # Check if target is installed
    check_rust_target "$target"
    
    # Build the library
    if cargo build --target "$target" --release --features ffi; then
        success "Successfully built for $target"
        
        # Copy library to organized output directory
        android_abi=$(get_android_abi "$target_name")
        mkdir -p "target/android/$android_abi"
        cp "target/${target}/release/libia_get.so" "target/android/$android_abi/"
    else
        error_exit "Failed to build for $target"
    fi
done

success "All Android targets built successfully!"
info "Libraries available in target/android/"

# Generate header file for FFI
info "Generating C header file..."
if command_exists cbindgen; then
    cbindgen --config cbindgen.toml --crate ia-get --output target/android/ia_get.h
    success "Header file generated: target/android/ia_get.h"
else
    warning "cbindgen not found. Install with: cargo install cbindgen"
fi

success "Build complete!"