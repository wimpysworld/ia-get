<h1 align="center">
  <img src="assets/ia-get.png" width="256" height="256" alt="ia-get">
  <br />
  ia-get
</h1>

<p align="center"><b>High-performance file downloader for archive.org</b></p>
<p align="center">
<img alt="GitHub Downloads" src="https://img.shields.io/github/downloads/Gameaday/ia-get-cli/total?logo=github&label=Downloads">
<img alt="CI Status" src="https://img.shields.io/github/actions/workflow/status/Gameaday/ia-get-cli/ci.yml?branch=main&logo=github&label=CI">
</p>

<p align="center">Made with 💝 by 🤖</p>

## 📥 Quick Download

### 🚀 Production Releases
- **🐧 Linux**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (x86_64, ARM, musl)
- **🪟 Windows**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (x86_64)
- **🍎 macOS**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (Intel + Apple Silicon)
- **🤖 Android**: [Latest Release](https://github.com/Gameaday/ia-get-cli/releases/latest) (Native libraries for app development)

### 🔧 Development Builds
- **Latest Changes**: [Development Release](https://github.com/Gameaday/ia-get-cli/releases/tag/development) (All platforms + Android)

### 📱 Android Integration
The Android package contains optimized native libraries for:
- **ARM64** (arm64-v8a) - Modern devices
- **ARMv7** (armeabi-v7a) - Older devices  
- **x86_64** - Intel emulators
- **x86** - Legacy emulators

Perfect for embedding into Flutter, React Native, or native Android apps. See [Mobile Development Guide](docs/MOBILE_DEVELOPMENT_GUIDE.md) for integration instructions.

> **🍴 Fork Notice**: This is a heavily modified fork of the original [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) project. Due to extensive architectural changes and different development directions, changes from this fork will not be pushed back to the upstream repository.

## ⚡ Quick Start

ia-get provides both CLI and GUI interfaces with smart auto-detection:

```shell
# Auto-detect best mode (GUI if available, menu otherwise)
ia-get

# Download directly from command line
ia-get https://archive.org/details/<identifier>

# Show help
ia-get --help
```

**Smart Mode Detection**: Automatically chooses GUI when display is available, falls back to interactive menu or CLI mode.

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
- **📱 Mobile**: Native Android libraries for app integration
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
## 🍴 Fork Information

This is a **heavily modified fork** of [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) with extensive architectural changes:

- **🔄 Complete API Migration**: Modern JSON APIs replace legacy XML
- **⚡ Enhanced Architecture**: Rebuilt concurrent downloader with session tracking  
- **🧪 Extensive Testing**: Full test suite with 81+ tests (all passing)
- **📦 CI/CD Pipeline**: Automated builds for all platforms including Android
- **🗜️ Compression Support**: HTTP compression and automatic decompression

**~80-90% of the codebase has been rewritten** for modern performance and reliability.

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
- **Android**: Native libraries for all major architectures

## 🤖 A.I. Driven Development

This fork represents a major evolution in AI-assisted development, building upon [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get):

- **🗑️ Complete Architectural Rewrite**: Migration from XML to JSON-first APIs
- **⚡ Enhanced Performance**: Rebuilt concurrent downloading with session management
- **🧹 Modern Codebase**: Comprehensive cleanup with extensive documentation
- **📦 Professional CI/CD**: Multi-platform builds including Android support

**~80-90% of the codebase has been rewritten** for modern performance standards.

**Featured on [Linux Matters Podcast](https://linuxmatters.sh)** 🎙️ discussing AI development processes!
