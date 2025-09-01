<h1 align="center">
  <img src="assets/ia-get.png" width="256" height="256" alt="ia-get">
  <br />
  ia-get
</h1>

<p align="center"><b>High-performance file downloader for archive.org</b></p>
<p align="center">
<img alt="GitHub all releases" src="https://img.shields.io/github/downloads/wimpysworld/ia-get/total?logo=github&label=Downloads">
<img alt="CI Status" src="https://img.shields.io/github/actions/workflow/status/Gameaday/ia-get-cli/ci.yml?branch=main&logo=github&label=CI">
</p>

<p align="center">Made with 💝 by 🤖</p>

# Usage 📖

Simply pass the URL of an [archive.org](https://archive.org) details page you want to download and `ia-get` will automatically fetch the JSON metadata and download all files with blazing speed.

```shell
ia-get https://archive.org/details/<identifier>
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

## Why? 🤔💭

I wanted to download high-quality scans of [ZZap!64 magazine](https://en.wikipedia.org/wiki/Zzap!64) and some read-only memory from archive.org.
Archives of this type often include many large files, torrents are not always provided and when they are available they do not index all the available files in the archive.

Archive.org provides comprehensive JSON APIs for every collection that indexes every file available.
So I co-authored `ia-get` to automate the download process with maximum efficiency and reliability.

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
# Build the project
cargo build

# Build optimized release
cargo build --release

# Run with development features
cargo run -- https://archive.org/details/your_archive
```

## Code Quality 🧪

The codebase maintains high quality standards:

```shell
# Run all tests (27+ unit tests)
cargo test

# Check code formatting
cargo fmt --check

# Run linting
cargo clippy

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
- **Every commit**: Binary artifacts uploaded for all supported platforms
- **Release builds**: Optimized binaries with full packaging (archives, .deb packages)
- **30-day retention**: Development artifacts available for testing and debugging
- **Commit traceability**: Artifacts tagged with commit SHA for easy identification

### Supported Platforms
- x86_64-unknown-linux-gnu
- x86_64-pc-windows-msvc
- x86_64-apple-darwin

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

I used these commands to test `ia-get` during development.

```shell
ia-get https://archive.org/details/deftributetozzap64
ia-get https://archive.org/details/zzapp_64_issue_001_600dpi
```

# A.I. Driven Development 🤖

This program is an ongoing experiment 🧪 in AI-assisted development. The project has evolved through multiple phases:

## Development History

**Phase 1 (Late 2023)**: Initially co-authored using [Chatty Jeeps](https://ubuntu.social/@popey/111527182881051626) (ChatGPT-4). I had no Rust experience and wanted to see if AI could help me build a program in an unfamiliar language.

**Phase 2 (Oct-Dec 2023)**: Used [Unfold.ai](https://unfoldai.io/) to add features and improve the code. All AI-assisted commits from this period have full details in the commit messages.

**Phase 3 (Jan 2024)**: Community input from Linux Matters listener [Daniel Dewberry](https://github.com/DanielDewberry) who submitted a [comprehensive peer review](https://github.com/wimpysworld/ia-get/issues/7).

**Phase 4 (May 2025)**: Major refactoring and modernization using GitHub Copilot with Claude Sonnet 3.5, including:
- Complete migration from XML to JSON APIs
- Enhanced concurrent downloading with session management
- Comprehensive code cleanup and documentation
- Modern Rust practices and error handling

## Featured Coverage

**As featured on [Linux Matters](https://linuxmatters.sh) podcast!** 🎙️ The [initial version](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was discussed in [Episode 16 - Blogging to the Fediverse](https://linuxmatters.sh/16/), covering the AI development process, successes, and challenges.

<div align="center">
  <a href="https://linuxmatters.sh" target="_blank"><img src="https://raw.githubusercontent.com/wimpysworld/nix-config/main/.github/screenshots/linuxmatters.png" alt="Linux Matters Podcast"/></a>
  <br />
  <em>Linux Matters Podcast</em>
</div>

Through this journey, I've learned Rust fundamentals and modern development practices, with each phase building on the previous work to create a robust, well-documented tool.
