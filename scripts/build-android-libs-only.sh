#!/bin/bash
# Build script for Android native libraries only (without Flutter/APK)
# This is a lightweight alternative to build-mobile.sh for developers who only need the .so files

set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

info "Building ia-get native libraries for Android targets..."

# Configure Android cross-compilation environment
configure_android_environment

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

success "Native library build complete!"
info "Use 'scripts/build-mobile.sh' to build the complete Android APK"