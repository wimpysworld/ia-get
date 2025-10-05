<h1 align="center">
  <img src="assets/ia-helper.png" width="256" height="256" alt="Internet Archive Helper">
  <br />
  Internet Archive Helper
</h1>

<p align="center"><b>Your comprehensive companion for accessing Internet Archive content</b></p>
<p align="center">
<img alt="GitHub Downloads" src="https://img.shields.io/github/downloads/Gameaday/ia-get-cli/total?logo=github&label=Downloads">
<img alt="CI Status" src="https://img.shields.io/github/actions/workflow/status/Gameaday/ia-get-cli/ci.yml?branch=main&logo=github&label=CI">
</p>

<p align="center">Built with ❤️ for the Internet Archive community</p>

> **Note**: This is an unofficial, community-developed project and is not affiliated with or endorsed by the Internet Archive.

## 📥 Quick Download

### 🚀 Production Releases
- **🐧 Linux**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (x86_64, ARM, musl)
- **🪟 Windows**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (x86_64, code-signed)
- **🍎 macOS**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (Intel + Apple Silicon)
- **🤖 Android**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (APK + Native libraries)

### 🔐 Security & Trust
- **Windows binaries are code-signed** using Azure Trusted Signing to prevent SmartScreen warnings
- **SHA256 checksums** provided for all releases to verify file integrity
- **Automated security audits** run on every commit to detect vulnerabilities

### 🔧 Development Builds
- **Latest Changes**: [Development Release](https://github.com/Gameaday/ia-get-cli/releases/tag/development) (All platforms + Android)

### 📱 Mobile App
The Internet Archive Helper mobile app provides a premium Android experience with a **pure Flutter/Dart implementation**:

**Android App Features:**
- **Material 3 Design** - Modern, accessible interface aligned with Android standards
- **Intuitive Touch Interface** - Optimized for mobile browsing and downloading
- **Background Downloads** - Continue downloads even when the app is closed
- **Deep Link Support** - Open archive.org links directly in the app
- **Pure Dart Implementation** - Fast, platform-independent code without native dependencies
- **Easy to Build** - Standard Flutter build process, no complex native compilation

**Platform Support:**
The Flutter app now supports multiple platforms:
- **Android** - Full-featured mobile experience
- **iOS** - Coming soon (easy to add with pure Dart implementation)
- **Web** - Access via browser (experimental)
- **Desktop** - Windows, macOS, Linux support via Flutter

See [Flutter App Documentation](mobile/flutter/README.md) for building and development instructions.

## 🌟 Project Vision

Internet Archive Helper is designed to be your comprehensive companion for accessing the vast resources of the Internet Archive. Whether you're a researcher, student, archivist, or simply someone who loves exploring historical digital content, this tool provides both powerful command-line capabilities and an intuitive mobile experience.

**Our Mission**: Make Internet Archive content more accessible, discoverable, and useful for everyone.

## ⚡ Quick Start

Internet Archive Helper provides both CLI and GUI interfaces with smart auto-detection:

```shell
# Auto-detect best mode (GUI if available, menu otherwise)
ia-get

# Download directly from command line
ia-get https://archive.org/details/<identifier>

# Show help and available options
ia-get --help
```

**Smart Interface Detection**: Automatically chooses the best interface - GUI when display is available, falls back to interactive menu or CLI mode based on your environment.

## 🎯 Features

### Core Functionality
- 🔽 **Fast Concurrent Downloads**: Parallel downloading with configurable concurrency limits
- 🌳 **Directory Structure**: Preserves the original archive directory structure  
- 🔄 **Smart Resume**: Automatically resumes partial or failed downloads
- 🎯 **Advanced Filtering**: Filter by file type, size, and custom patterns
- 🗜️ **Compression Support**: HTTP compression and automatic archive extraction
- 📊 **Progress Tracking**: Real-time progress with speed and ETA information

### Cross-Platform Support
- **🖥️ Desktop**: CLI and GUI modes for Linux, Windows, macOS
- **📱 Mobile**: Android APK app (pure Flutter/Dart implementation)
- **🔧 Development**: Hot-reload support and comprehensive debugging tools

### User Interfaces
- **🖼️ GUI Mode**: Intuitive graphical interface with visual filtering and settings
- **⌨️ CLI Mode**: Powerful command-line interface for automation
- **📱 Interactive Menu**: Fallback text interface when GUI isn't available

## 🚀 Advanced Usage

```shell
# Concurrent downloads with compression
ia-get --compress --decompress https://archive.org/details/your_archive

# Filter by file types
ia-get --include-ext pdf,epub https://archive.org/details/books_archive

# Limit file sizes  
ia-get --max-file-size 100MB https://archive.org/details/data_archive

# Specify output directory
ia-get --output ./downloads https://archive.org/details/software_archive
```

### GUI Features
The GUI provides smart detection, easy archive input with validation, visual file filtering, real-time progress tracking, settings management, and download history. See [GUI_README.md](GUI_README.md) for detailed documentation.

## 🛡️ Integrity Verification

All releases include SHA256 checksums for security verification:

```bash
# Download and verify (example for Linux x86_64)
curl -LO https://github.com/Gameaday/ia-get-cli/releases/latest/download/RELEASE_HASHES.txt
sha256sum -c RELEASE_HASHES.txt
```

## 🗜️ Compression & Decompression

```bash
# Enable compression and auto-decompression
ia-get --compress --decompress https://archive.org/details/your_archive

# Decompress specific formats only
ia-get --decompress --decompress-formats gzip,bzip2 https://archive.org/details/your_archive
```

Supports gzip, bzip2, xz, tar, and combined formats. See [docs/COMPRESSION.md](docs/COMPRESSION.md) for details.

## 🏗️ Development

```shell
# Build CLI only (fastest - 60% faster builds)
cargo build --no-default-features --features cli

# Build with GUI support  
cargo build --features gui

# Fast development builds
cargo build --profile fast-dev --no-default-features --features cli

# Optimized release
cargo build --release --no-default-features --features cli
```

### 📱 Flutter/Android Development

The Flutter mobile app now uses a **pure Dart implementation** - no native Rust compilation required!

```shell
# Install Flutter (required)
# See https://flutter.dev/docs/get-started/install

# Navigate to Flutter project
cd mobile/flutter

# Get dependencies
flutter pub get

# Run on connected device/emulator
flutter run

# Build Android APK
flutter build apk

# Build for other platforms
flutter build ios          # iOS (coming soon)
flutter build web          # Web version
flutter build windows      # Windows desktop
flutter build macos        # macOS desktop
flutter build linux        # Linux desktop
```

**Benefits of Pure Dart:**
- ✅ No Android NDK required
- ✅ No Rust toolchain needed for Flutter development
- ✅ Standard Flutter build process
- ✅ Faster builds (no native compilation)
- ✅ Easy debugging with Flutter DevTools
- ✅ Works on all Flutter platforms

For Android deployment and Play Store submission, see **[ANDROID_DEPLOYMENT_GUIDE.md](ANDROID_DEPLOYMENT_GUIDE.md)**.

### 🔧 Troubleshooting Build Issues

**Having Flutter/Dart SDK version conflicts?** Run our quick-fix script:

```shell
./scripts/fix-flutter-deps.sh
```

**Common Issues:**
- **Flutter version errors**: Ensure Flutter 3.35.0+ is installed (includes Dart 3.8.0+)
- **Dependency conflicts**: Run `flutter clean` and `flutter pub get` in `mobile/flutter/`
- **Build failures**: See **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** for comprehensive solutions

**Required Versions:**
- Flutter: 3.35.0 or higher
- Dart: 3.8.0 or higher (included with Flutter 3.35.0+)
- Rust: Latest stable (1.75.0+)

### Build Profiles
- **`dev`**: Fast compilation for development
- **`fast-dev`**: Minimal optimization for quick iteration
- **`release`**: Maximum optimization for production

### 📊 Build Optimization

The project includes significant build time optimizations:

- **Feature Gates**: CLI and GUI components separated for faster compilation
- **Build Profiles**: Multiple profiles optimized for different use cases  
- **CLI-only builds**: ~60-70% faster than full builds
- **Development iteration**: Additional 10-20% improvement with fast-dev profile

```shell
# Measure build performance
./scripts/build-benchmark.sh

# See full development guide  
cat docs/DEVELOPMENT.md
```

## 🧪 Quality Assurance

```shell
# Run tests (CLI only - fastest)
cargo test --no-default-features --features cli

# Check formatting and linting
cargo fmt --check
cargo clippy --no-default-features --features cli -- -D warnings

# Test CI locally
./scripts/test-ci.sh
```

**Test Coverage**: 81+ tests passing across unit, integration, and API validation.

## CI/CD & Quality Assurance 🔄

The project now includes comprehensive CI/CD processes:

### Continuous Integration
- **Multi-platform builds**: Automatic builds for Linux, Windows, macOS
- **Flutter builds**: Separate workflow for Android APK building
- **Code quality checks**: Formatting, linting, and compilation verification
- **Standard toolchains**: Uses Rust's cargo and Flutter's build system

### Automated Artifacts
- **Every commit**: Development releases with all desktop platforms + Android APK
- **Tagged releases**: Production-quality binaries with comprehensive packaging
- **Integrity verification**: SHA256 checksums for all downloads
- **Multi-platform**: All supported architectures and operating systems

### Supported Platforms

**Rust CLI/GUI:**
- **Linux**: x86_64, ARM, musl variants
- **Windows**: x86_64
- **macOS**: Intel + Apple Silicon

**Flutter Mobile App:**
- **Android**: APK for all architectures (arm64-v8a, armeabi-v7a, x86_64)
- **iOS**: Coming soon (pure Dart makes this easy)
- **Web**: Experimental support
- **Desktop**: Windows, macOS, Linux via Flutter

## 🏗️ Modern Architecture & Development

Internet Archive Helper uses **two independent, optimized implementations**:

### Rust CLI/GUI (Desktop)
- **🔄 Modern JSON APIs**: Clean, efficient communication with Internet Archive services
- **⚡ High-Performance Architecture**: Concurrent downloading with intelligent session management
- **🧪 Comprehensive Testing**: Robust test suite ensuring reliability and stability
- **🔧 Zero Unsafe Code**: Safe, reliable Rust code with minimal unsafe blocks
- **📦 Standard Toolchain**: Uses standard Cargo for building - no special setup required

### Flutter Mobile App  
- **🎨 Pure Dart Implementation**: No native dependencies or FFI complexity
- **📱 Cross-Platform Ready**: Android, iOS, Web, and Desktop from single codebase
- **🚀 Fast Development**: Hot-reload and Flutter DevTools for rapid iteration
- **🔄 Modern HTTP Client**: Direct API communication using Dart's http package
- **✅ Easy Testing**: Standard Flutter testing framework with no native complications

**Architecture Benefits:**
- ✅ Clear separation of concerns
- ✅ Independent optimization for each platform
- ✅ Simplified development and maintenance
- ✅ No build complexity from FFI bridges
- ✅ Both implementations maintain full feature parity

See [FFI Removal Migration Guide](docs/FFI_REMOVAL_MIGRATION_GUIDE.md) for details on the architectural change.
- **📦 Professional CI/CD**: Automated builds and testing across all supported platforms
- **🎯 Cross-Platform Excellence**: Native performance on desktop, mobile, and embedded systems

**Built for the future** with forward-compatible design and modern development practices.

## 🌐 Community & Contributions

We welcome contributions from developers, researchers, and Internet Archive enthusiasts! Whether you want to:

- **🐛 Report bugs** or suggest improvements
- **💻 Contribute code** or documentation
- **🎨 Improve the user interface**
- **📚 Help with translations**

Check out our [Contributing Guidelines](CONTRIBUTING.md) to get started. Every contribution helps make Internet Archive content more accessible to everyone.
