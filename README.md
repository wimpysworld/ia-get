<h1 align="center">
  <img src="assets/ia-get.png" width="256" height="256" alt="Internet Archive Helper">
  <br />
  Internet Archive Helper
</h1>

<p align="center"><b>Your comprehensive companion for accessing Internet Archive content</b></p>
<p align="center">
<img alt="GitHub Downloads" src="https://img.shields.io/github/downloads/Gameaday/ia-get-cli/total?logo=github&label=Downloads">
<img alt="CI Status" src="https://img.shields.io/github/actions/workflow/status/Gameaday/ia-get-cli/ci.yml?branch=main&logo=github&label=CI">
</p>

<p align="center">Built with ❤️ for the Internet Archive community</p>

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
The Internet Archive Helper mobile app provides a premium Android experience:

**Android App Features:**
- **Material 3 Design** - Modern, accessible interface aligned with Android standards
- **Intuitive Touch Interface** - Optimized for mobile browsing and downloading
- **Background Downloads** - Continue downloads even when the app is closed
- **Deep Link Support** - Open archive.org links directly in the app

**For Developers:**
The project also provides native libraries for embedding into other applications:
- **ARM64** (arm64-v8a) - Modern Android devices
- **ARMv7** (armeabi-v7a) - Older Android devices  
- **x86_64** - Intel emulators
- **x86** - Legacy emulators

See [Mobile Development Guide](docs/MOBILE_DEVELOPMENT_GUIDE.md) for integration instructions.

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
- **📱 Mobile**: Android APK app + Native libraries for development
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

# Android builds (requires Android NDK)
./scripts/build-mobile.sh          # Complete APK build  
./scripts/build-android-libs-only.sh  # Native libraries only
```

### 📱 Android Development

```shell
# Prerequisites for Android builds
export ANDROID_NDK_HOME=/path/to/android/ndk
export ANDROID_HOME=/path/to/android/sdk

# Install Flutter (for APK builds)
# See https://flutter.dev/docs/get-started/install

# Build Android APK + Native Libraries
./scripts/build-mobile.sh

# Build only native libraries (faster, no Flutter required)  
./scripts/build-android-libs-only.sh
```

For complete Android deployment instructions including APK generation and Play Store submission, see **[ANDROID_DEPLOYMENT_GUIDE.md](ANDROID_DEPLOYMENT_GUIDE.md)**.

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
- **Multi-platform builds**: Automatic builds for Linux, Windows, macOS, and Android
- **Code quality checks**: Formatting, linting, and compilation verification
- **Standard toolchain**: Uses Rust's standard toolchain for reliable builds

### Automated Artifacts
- **Every commit**: Development releases with all platforms including Android
- **Tagged releases**: Production-quality binaries with comprehensive packaging
- **Integrity verification**: SHA256 checksums for all downloads
- **Multi-platform**: All supported architectures and operating systems

### Supported Platforms
- **Linux**: x86_64, ARM, musl variants
- **Windows**: x86_64
- **macOS**: Intel + Apple Silicon  
- **Android**: APK apps + Native libraries for all major architectures

## 🏗️ Modern Architecture & Development

Internet Archive Helper is built with modern software development principles:

- **🔄 Modern JSON APIs**: Clean, efficient communication with Internet Archive services
- **⚡ High-Performance Architecture**: Concurrent downloading with intelligent session management
- **🧪 Comprehensive Testing**: Robust test suite ensuring reliability and stability
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
