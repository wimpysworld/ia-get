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

> **🍴 Fork Notice**: This is a heavily modified fork of the original [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) project. Due to extensive architectural changes and different development directions, changes from this fork will not be pushed back to the upstream repository.

# Usage 📖

ia-get provides both a command-line interface (CLI) and a graphical user interface (GUI) for downloading files from [archive.org](https://archive.org).

## Command Line Interface

Simply pass the URL of an [archive.org](https://archive.org) details page you want to download and `ia-get` will automatically fetch the JSON metadata and download all files with blazing speed.

```shell
ia-get https://archive.org/details/<identifier>
```

## Graphical User Interface 🖼️

For users who prefer a visual interface, ia-get includes a cross-platform GUI:

```shell
ia-get-gui
```

The GUI provides:
- **Easy archive input**: Enter URLs or identifiers with validation
- **Visual file filtering**: Configure downloads with intuitive controls  
- **Real-time progress**: See download status with progress bars and statistics
- **Settings management**: Configure all options through a user-friendly interface
- **Download history**: Quick access to previously downloaded archives

See [GUI_README.md](GUI_README.md) for detailed GUI documentation.

## 📥 Download ia-get

Get the latest version for your platform:

<div align="center">

[![Download for Linux](https://img.shields.io/badge/Download-Linux-0078d4?style=for-the-badge&logo=linux&logoColor=white)](https://github.com/Gameaday/ia-get-cli/releases/latest)
[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078d4?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/Gameaday/ia-get-cli/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-0078d4?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/Gameaday/ia-get-cli/releases/latest)

</div>

### 🔧 Development Builds

For the latest features and fixes, try our [development builds](https://github.com/Gameaday/ia-get-cli/releases/tag/development) which are automatically updated with every commit to the main branch.

### 📦 All Platforms & Packages

Visit our [releases page](https://github.com/Gameaday/ia-get-cli/releases) for:
- **All supported platforms**: Linux (x86_64, ARM, musl), Windows, macOS (Intel + Apple Silicon)
- **Package formats**: Archives (.tar.gz, .zip) and Debian packages (.deb)
- **Checksums**: SHA256 hashes for integrity verification
- **Changelog**: Full release notes with all changes

### 🛡️ Integrity Verification

All releases include SHA256 checksums. To verify your download:

```bash
# Download both the archive and checksum file from the releases page
# Example for Linux x86_64:
curl -LO https://github.com/Gameaday/ia-get-cli/releases/latest/download/ia-get-v1.3.0-x86_64-unknown-linux-gnu.tar.gz
curl -LO https://github.com/Gameaday/ia-get-cli/releases/latest/download/ia-get-v1.3.0-x86_64-unknown-linux-gnu.tar.gz.sha256

# Verify the download
sha256sum -c ia-get-v1.3.0-x86_64-unknown-linux-gnu.tar.gz.sha256
```

## Advanced Usage 🚀

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

## Fork Information 🍴

This repository is a **heavily modified fork** of the original [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) project, developed by Martin Wimpress. Due to extensive architectural changes and different development directions, this fork has diverged significantly from the upstream project.

### Key Differences from Upstream

- **🔄 Complete API Migration**: Migrated entirely from legacy XML to modern JSON APIs
- **⚡ Enhanced Architecture**: Rebuilt concurrent downloader with session tracking and progress reporting  
- **🧹 Modern Codebase**: Comprehensive refactoring with extensive documentation and modern Rust practices
- **🧪 Extensive Testing**: Full test suite with 27+ unit tests (all passing)
- **📦 CI/CD Pipeline**: Automated builds and artifact generation for multiple platforms
- **🎯 Advanced Filtering**: Smart file filtering system with size and extension-based controls
- **🗜️ Compression Support**: HTTP compression and automatic decompression capabilities

### Code Comparison

Based on the extensive refactoring documented in the changelog, **approximately 80-90% of the codebase has been rewritten** from the original. The core concept and some utility functions remain, but the architecture, API usage, error handling, and feature set have been fundamentally rebuilt.

## Why? 🤔💭

Archive.org provides comprehensive JSON APIs for every collection that indexes every file available. This tool automates the download process with maximum efficiency and reliability, making it ideal for downloading large archives like high-quality magazine scans, software collections, and other digital preservation content.

## ✨ Features

### Core Functionality
- 🔽 **Fast Concurrent Downloads**: Parallel downloading with configurable concurrency limits
- 🌳 **Directory Structure**: Preserves the original archive directory structure
- 🔄 **Smart Resume**: Automatically resumes partial or failed downloads
- 🔏 **Integrity Verification**: MD5 hash checks to confirm file integrity
- 🌱 **Incremental Updates**: Can be run multiple times to update existing downloads
- 📊 **Complete Metadata**: Fetches all metadata for the archive using JSON API

### Advanced Features
- 🗜️ **Compression Support**: HTTP compression + automatic decompression of archives
- 🎯 **Smart Filtering**: Filter by file extension, size, or custom patterns  
- 📈 **Progress Tracking**: Real-time progress bars and download statistics
- � **Session Management**: Persistent download sessions for large archives
- 🛡️ **Error Recovery**: Robust retry logic for network failures
- �📦 **Cross-Platform**: Available for **Linux** 🐧 **macOS** 🍏 and **Windows** 🪟

### Technical Improvements
- ⚡ **JSON-First**: Uses Internet Archive's modern JSON API (no legacy XML)
- 🧹 **Clean Codebase**: Comprehensive refactoring with extensive documentation
- 🧪 **Well-Tested**: Full test suite with 27+ unit tests
- 📚 **Rich Documentation**: In-depth API documentation and examples

## 🗜️ Compression Support

ia-get includes comprehensive compression features powered by modern JSON APIs:

- **HTTP Compression**: Automatically reduces bandwidth usage during downloads
- **Auto-Detection**: Detects compressed files from Internet Archive metadata
- **Multiple Formats**: Support for gzip, bzip2, xz, tar, and combined formats (tar.gz, tar.bz2, tar.xz)
- **Transparent Decompression**: Automatically decompresses files after download
- **Configurable**: Choose which formats to decompress automatically

```bash
# Enable compression and auto-decompression
ia-get --compress --decompress https://archive.org/details/your_archive

# Decompress specific formats only
ia-get --decompress --decompress-formats gzip,bzip2 https://archive.org/details/your_archive
```

See [docs/COMPRESSION.md](docs/COMPRESSION.md) for detailed compression documentation.

## 🏗️ Architecture

The codebase has been completely refactored for performance and maintainability:

- **JSON-First Design**: Uses Internet Archive's modern JSON API exclusively
- **Concurrent Engine**: Enhanced parallel downloader with session tracking
- **Session Management**: Persistent download state for large archives
- **Error Recovery**: Comprehensive retry logic for network failures
- **Clean Abstractions**: Well-documented modules with clear responsibilities

### Key Modules

- `metadata`: JSON metadata fetching with retry logic
- `enhanced_downloader`: Main download engine with session support  
- `concurrent_simple`: Enhanced concurrent downloader
- `compression`: Automatic decompression utilities
- `filters`: File filtering and formatting utilities

### Sharing is caring 🤝

You can use `ia-get` to download files from archive.org, including all the metadata and the `.torrent` file, if there is one.
You can the start seeding the torrent using a pristine copy of the archive, and a complete file set.

# Demo 🧑‍💻

<div align="center"><img alt="ia-get demo" src="assets/ia-get.gif" width="1024" /></div>

# Development 🏗️

ia-get is built with modern Rust practices and comprehensive testing.

```shell
# Build CLI only (fastest - 60% faster builds)
cargo build --no-default-features --features cli

# Build with GUI support
cargo build --features gui

# Fast development builds
cargo build --profile fast-dev --no-default-features --features cli

# Build optimized release
cargo build --release --no-default-features --features cli

# Run with CLI features
cargo run --no-default-features --features cli -- https://archive.org/details/your_archive
```

## ⚡ Build Performance

The project now includes significant build time optimizations:

- **Feature Gates**: CLI and GUI components are separated for faster compilation
- **Development Profiles**: Multiple build profiles optimized for different use cases
- **Dependency Optimization**: GUI dependencies only compile when needed

**Build Time Improvements:**
- CLI-only builds: ~60-70% faster than full builds
- Test compilation: ~50% faster with CLI-only features
- Development iteration: Additional 10-20% improvement with fast-dev profile

```shell
# Measure your build performance
./scripts/build-benchmark.sh

# See full development guide
cat docs/DEVELOPMENT.md
```

## Code Quality 🧪

The codebase maintains high quality standards:

```shell
# Run tests (CLI only - fastest)
cargo test --no-default-features --features cli

# Run all tests including GUI
cargo test --all-features

# Check code formatting
cargo fmt --check

# Run linting (CLI only)
cargo clippy --no-default-features --features cli -- -D warnings

# Check compilation
cargo check
```

## Recent Improvements 🚀

The codebase has undergone major improvements:

- ✅ **Removed Legacy XML Support**: Migrated entirely to JSON API
- ✅ **Enhanced Concurrent Downloader**: Added session tracking and progress reporting
- ✅ **Comprehensive Documentation**: Added extensive inline documentation and examples
- ✅ **Code Cleanup**: Removed orphaned files and improved error handling
- ✅ **Test Coverage**: Updated tests to work with new JSON-only architecture
- ✅ **CI/CD Pipeline**: Automated builds and artifact generation for every commit

## CI/CD & Quality Assurance 🔄

The project now includes comprehensive CI/CD processes:

### Continuous Integration
- **Multi-platform builds**: Automatic builds for Linux, Windows, and macOS
- **Code quality checks**: Formatting (`cargo fmt`), linting (`cargo clippy`), and compilation verification
- **Standard toolchain**: Uses Rust's standard toolchain for reliable, reproducible builds

### Automated Artifacts
- **Every commit**: Binary artifacts automatically published as [development releases](https://github.com/Gameaday/ia-get-cli/releases/tag/development)
- **Tagged releases**: Production-quality binaries with comprehensive packaging (archives, .deb packages)
  - Supports both `v1.2.3` and `1.2.3` tag formats
- **Permanent retention**: All artifacts available permanently via GitHub releases
- **Integrity verification**: SHA256 checksums for all downloads ensuring reproducible builds
- **Commit traceability**: Development builds tagged with commit SHA for easy identification
- **Multi-platform**: Automated builds for all supported architectures and operating systems

### Supported Platforms
- **Linux**: x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl, i686-unknown-linux-gnu, arm-unknown-linux-gnueabihf, aarch64-unknown-linux-gnu
- **Windows**: x86_64-pc-windows-msvc  
- **macOS**: x86_64-apple-darwin, aarch64-apple-darwin

The CI runs on every push and pull request, ensuring code quality and platform compatibility.

### Testing CI Locally
You can simulate the CI process locally using the provided script:

```bash
./scripts/test-ci.sh
```

This script runs all the same checks as the CI workflow:
- Code formatting verification
- Clippy linting 
- Build verification
- Release binary creation
- Artifact packaging

The script creates test artifacts in `artifacts/` directory which is gitignored.

## Manual Tests 🤞

You can test `ia-get` with various archive types:

```shell
# Test with magazine archives
ia-get https://archive.org/details/your_magazine_archive

# Test with software collections  
ia-get https://archive.org/details/your_software_archive

# Test with document collections
ia-get https://archive.org/details/your_document_archive
```

# A.I. Driven Development 🤖

This program represents an ongoing experiment 🧪 in AI-assisted development, building upon the foundation of the original [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) project. This fork has evolved through multiple phases of development:

## Development History

**Original Project (2023)**: The upstream [`wimpysworld/ia-get`](https://github.com/wimpysworld/ia-get) was initially co-authored by Martin Wimpress using [Chatty Jeeps](https://ubuntu.social/@popey/111527182881051626) (ChatGPT-4), with subsequent improvements through [Unfold.ai](https://unfoldai.io/) and community input.

**Fork Development (2025)**: This fork represents a major departure from the original, featuring:
- **🗑️ Complete Architectural Rewrite**: Migration from XML to JSON-first APIs
- **⚡ Enhanced Performance**: Rebuilt concurrent downloading with session management
- **🧹 Modern Codebase**: Comprehensive cleanup with extensive documentation
- **🧪 Robust Testing**: Full test suite with 27+ unit tests
- **📦 Professional CI/CD**: Multi-platform builds and automated artifact generation

## Divergence from Upstream

Due to the extensive changes in architecture, coding style, and project direction, **this fork will not contribute changes back to the upstream repository**. The projects have evolved to serve different use cases and development philosophies.

## Featured Coverage

**As featured on [Linux Matters](https://linuxmatters.sh) podcast!** 🎙️ The [original version](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was discussed in [Episode 16 - Blogging to the Fediverse](https://linuxmatters.sh/16/), covering the AI development process, successes, and challenges.

<div align="center">
  <a href="https://linuxmatters.sh" target="_blank"><img src="https://raw.githubusercontent.com/wimpysworld/nix-config/main/.github/screenshots/linuxmatters.png" alt="Linux Matters Podcast"/></a>
  <br />
  <em>Linux Matters Podcast</em>
</div>

This fork continues the AI-assisted development journey, demonstrating how modern tools can be used to completely transform and modernize an existing codebase while maintaining respect for the original work.
