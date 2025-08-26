# Changelog

## [1.3.0] - 2025-08-26

### Major Refactoring and Modernization
- **🗑️ Complete XML Removal**: Migrated entirely from legacy XML to modern JSON API for better performance and maintainability
- **⚡ Enhanced Concurrent Downloader**: Rebuilt concurrent downloading with session tracking, progress reporting, and improved error handling
- **🧹 Comprehensive Code Cleanup**: Removed all orphaned and legacy files (main_old.rs, metadata_old.rs, archive_metadata_old.rs, etc.)
- **📚 Extensive Documentation**: Added comprehensive module-level and function-level documentation with examples throughout the codebase
- **🏗️ Modern Architecture**: Clean module structure with well-defined responsibilities and clear abstractions

### Technical Improvements
- **JSON-First Design**: Exclusively uses Internet Archive's JSON API (removed serde-xml-rs dependency)
- **Session Management**: Enhanced DownloadSession integration for better resume capability
- **Statistics Tracking**: Comprehensive DownloadStats with speed monitoring and ETA calculations
- **Error Recovery**: Robust retry logic with proper error context and reporting
- **Test Coverage**: Updated all tests to work with JSON-only architecture (27/27 tests passing)

### Code Quality
- **Zero Compilation Warnings**: Clean codebase with no unused imports or variables
- **Modern Rust Practices**: Comprehensive error handling and idiomatic Rust patterns
- **Rich Documentation**: Each module includes usage examples and architectural overview
- **Performance Focus**: Optimized concurrent downloading with configurable limits

### Breaking Changes
- Removed XML metadata support (JSON-only)
- Simplified concurrent downloader API
- Updated project structure and module organization

## [Unreleased] - 2025-08-16

### New Features
- **Single-line Progress Display**: Replaced multiple progress bars with clean, single-line progress showing current file, statistics, and completion status
- **Real-time Download Progress**: Added percentage completion and transfer speed indicators (MB/s, KB/s) for large file downloads
- **Interactive Retry System**: Post-completion menu allowing users to retry failed downloads or exit gracefully
- **JSON Error Logging**: Optional `--hash` flag enables logging of failed downloads to `batchlog.json` for debugging
- **Configurable Output Directory**: New `--output` flag allows specifying custom download directory (defaults to archive identifier)

### Changes
- **Improved Signal Handling**: Implemented `std::sync::Once` pattern to prevent `MultipleHandlers` panic during retry operations
- **Enhanced Error Recovery**: Added comprehensive retry logic with exponential backoff for network errors and rate limiting (HTTP 429)
- **Silent Hash Verification**: Hash calculations and file checks now run without separate progress bars to reduce terminal clutter
- **Optimized Dependencies**: Reduced tokio features from `"full"` to specific required features, added `chrono`, `humantime`, and `serde_json`
- **Smart URL Handling**: Accept either full archive.org URLs or just identifiers (automatically constructs proper URLs)

### Bug Fixes
- **Fixed Statistics Counting**: Corrected mutually exclusive file categorization (downloaded/skipped/failed counts now add up correctly)
- **Eliminated Line Spam**: Removed all sources of excessive terminal output for clean, professional display
- **Resolved Signal Handler Crashes**: Fixed panic when setting up signal handlers multiple times during retry operations
- **Improved Hash Error Handling**: Hash calculation failures no longer create terminal output or interrupt downloads

### Technical Improvements
- Complete rewrite of progress display system in `downloader.rs`
- Enhanced main loop with proper signal management and user interaction
- Added comprehensive network error categorization and retry strategies
- Implemented duplicate prevention for error logging
