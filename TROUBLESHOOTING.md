# Troubleshooting Guide

This guide helps resolve common build and dependency issues in the ia-get project.

## Flutter/Dart SDK Version Issues

### Problem: "The current Dart SDK version is X.X.X. Because internet_archive_helper requires SDK version >=3.8.0"

**Root Cause**: Your local Flutter installation has an older Dart SDK version than required.

**Quick Fix**:

```bash
# Update Flutter to the latest stable version
flutter upgrade

# Verify you have Dart 3.8.0 or higher
flutter --version

# Expected output should show:
# Flutter 3.27.1 or higher
# Dart 3.8.0 or higher
```

**Complete Resolution Steps**:

1. **Update Flutter**:
   ```bash
   flutter upgrade
   flutter doctor
   ```

2. **Clean Flutter cache**:
   ```bash
   cd mobile/flutter
   flutter clean
   rm -rf pubspec.lock
   ```

3. **Get dependencies**:
   ```bash
   flutter pub get
   ```

4. **Verify the build**:
   ```bash
   flutter analyze
   flutter test
   ```

### Why This Issue Occurs

The project uses `flutter_lints: ^6.0.0` which requires Dart SDK ^3.8.0. This version comes with:
- Flutter 3.27.1 → Dart 3.8.0+
- Flutter 3.24.5 → Dart 3.5.4 (too old)
- Flutter 3.16.0 → Dart 3.2.x (too old)

## Rust Build Issues

### Problem: "Rust build fails" or "cargo build errors"

**Quick Fix**:

```bash
# Ensure you have the latest Rust toolchain
rustup update stable

# Format the code
cargo fmt

# Run clippy to check for issues
cargo clippy --no-default-features --features cli -- -D warnings

# Build the CLI version
cargo build --no-default-features --features cli
```

### Problem: "Cross-compilation for Android fails"

**Quick Fix**:

1. **Install Android targets**:
   ```bash
   rustup target add aarch64-linux-android
   rustup target add armv7-linux-androideabi
   rustup target add x86_64-linux-android
   rustup target add i686-linux-android
   ```

2. **Install cargo-ndk**:
   ```bash
   cargo install cargo-ndk
   ```

3. **Set environment variables**:
   ```bash
   export ANDROID_HOME=$HOME/Android/Sdk
   export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/26.1.10909125
   ```

4. **Build Android libraries**:
   ```bash
   ./scripts/build-mobile.sh --development
   ```

## Dependency Issues

### Problem: "Package resolution failed" or "Version conflict"

**Quick Fix**:

```bash
# For Rust dependencies
cd /path/to/ia-get-cli
cargo clean
rm -rf Cargo.lock
cargo update
cargo build

# For Flutter dependencies  
cd mobile/flutter
flutter clean
rm -rf pubspec.lock
flutter pub get
flutter pub upgrade
```

## CI/CD Pipeline Issues

### Problem: "GitHub Actions workflow fails"

**Note**: The repository's CI/CD configuration has been updated to handle Dart SDK version requirements correctly.

**Recent Fix Applied**: The CI/CD workflows now use cache keys that include the Dart SDK version requirement (`dart3.8`) to prevent incompatible cached dependencies from being used. This ensures that when the Dart SDK constraint is updated, the CI will use fresh dependencies compatible with the new SDK version.

**Check These Items**:

1. **Verify Flutter version in workflows** (already updated and correct):
   - ✅ `.github/workflows/ci.yml` has `flutter-version: '3.27.1'`
   - ✅ `.github/workflows/release.yml` has `flutter-version: '3.27.1'`

2. **Verify Dart SDK constraint in pubspec.yaml** (already updated and correct):
   ```yaml
   environment:
     sdk: '>=3.8.0 <4.0.0'  ✓
   ```

3. **Cache invalidation** (automatically handled):
   - The cache keys now include `dart3.8` to prevent using old cached dependencies
   - When SDK constraints change, the cache will be automatically invalidated

4. **Clear GitHub Actions cache** (if still experiencing issues):
   - Go to your repository's Actions tab
   - Click on "Caches" in the left sidebar
   - Delete old caches if they exist (caches with keys not containing `dart3.8`)

5. **Re-run the workflow**:
   - After cache updates, workflows should succeed automatically
   - The workflows now verify Flutter and Dart versions before building

**Important**: The configuration files in this repository are correct and include cache-busting mechanisms. If workflows fail, check:
- Are you using a forked repository with outdated caches?
- Are there network issues preventing Flutter/package downloads?
- Check the workflow logs for the specific error message

## Environment Setup

### Recommended Versions

- **Rust**: 1.75.0 or higher (latest stable recommended)
- **Flutter**: 3.27.1 or higher
- **Dart**: 3.8.0 or higher (included with Flutter 3.27.1+)
- **Android SDK**: API 33 or higher
- **Android NDK**: 26.1.10909125 or compatible version
- **Java**: JDK 17 (for Android builds)

### Quick Environment Check

Run this command to check your environment:

```bash
# Check Rust
cargo --version
rustc --version

# Check Flutter (if building mobile)
flutter --version
flutter doctor

# Check Android (if building mobile)
echo $ANDROID_HOME
echo $ANDROID_NDK_HOME
```

## Getting Help

If you're still experiencing issues after following this guide:

1. **Check the documentation**:
   - [Mobile Development Guide](docs/MOBILE_DEVELOPMENT_GUIDE.md)
   - [Flutter Dependency Fix](FLUTTER_DEPENDENCY_FIX.md)
   - [Contributing Guide](CONTRIBUTING.md)

2. **Search existing issues**:
   - [GitHub Issues](https://github.com/Gameaday/ia-get-cli/issues)

3. **Create a new issue**:
   - Include your environment details (OS, Flutter version, Rust version)
   - Include the full error message
   - Include steps to reproduce the issue

## Quick Reference: Common Commands

### Rust Development

```bash
# Format code
cargo fmt

# Check for issues
cargo clippy --no-default-features --features cli

# Build CLI
cargo build --no-default-features --features cli

# Build GUI
cargo build --features gui

# Run tests
cargo test --no-default-features --features cli

# Build release
cargo build --release --no-default-features --features cli
```

### Flutter Development

```bash
# Get dependencies
flutter pub get

# Run analyzer
flutter analyze

# Run tests
flutter test

# Build APK (debug)
flutter build apk --debug

# Build APK (release)
flutter build apk --release

# Build App Bundle
flutter build appbundle --release
```

### Full Mobile Build

```bash
# Development build (APK only)
./scripts/build-mobile.sh --development

# Development build (APK + App Bundle)
./scripts/build-mobile.sh --development --appbundle

# Production build
./scripts/build-mobile.sh --production --appbundle --store-ready
```
