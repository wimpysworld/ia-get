# Compression Support in ia-get

This document describes the compression features implemented in ia-get following Archive.org guidelines.

## Overview

ia-get now supports comprehensive compression handling for downloads from Internet Archive, including:

1. **HTTP Compression**: Enable compression during download transfers
2. **Automatic Detection**: Detect compressed files from Archive.org metadata
3. **Transparent Decompression**: Automatically decompress downloaded files
4. **Format Support**: Support for common compression formats used by Archive.org

## Supported Formats

### Compression Formats
- **gzip** (.gz files)
- **bzip2** (.bz2 files) 
- **xz** (.xz files)
- **tar** (.tar archives)
- **zip** (.zip archives) *[framework in place]*

### Combined Formats
- **tar.gz** (gzipped tar archives)
- **tar.bz2** (bzip2 compressed tar archives)
- **tar.xz** (xz compressed tar archives)

## Command Line Usage

### Basic Compression Options

```bash
# Enable HTTP compression during downloads
ia-get --compress <archive_url>

# Enable automatic decompression of downloaded files
ia-get --decompress <archive_url>

# Enable both compression and decompression
ia-get --compress --decompress <archive_url>
```

### Advanced Configuration

```bash
# Specify which formats to auto-decompress
ia-get --decompress --decompress-formats gzip,bzip2,xz <archive_url>

# Only decompress specific formats
ia-get --decompress --decompress-formats zip <archive_url>
```

### Subcommand Usage

```bash
# Using the download subcommand with compression
ia-get download --compress <archive_url>
```

## How It Works

### 1. HTTP Compression

When `--compress` is enabled, ia-get adds compression headers to HTTP requests:

```
Accept-Encoding: gzip, deflate, br
```

This allows Archive.org servers to compress the data during transfer, reducing bandwidth usage and download times.

### 2. Compression Detection

ia-get automatically detects compressed files using two methods:

1. **Metadata Format Field**: Checks the `format` field in Archive.org metadata
2. **File Extension**: Analyzes file extensions as a fallback

Example metadata-based detection:
```json
{
  "name": "magazine_scans.tar.gz",
  "format": "gzip",
  "size": "150000000"
}
```

### 3. Automatic Decompression

After successful download and verification, compressed files are automatically decompressed if:

- `--decompress` flag is enabled
- The file format is in the configured decompress formats list
- The decompression succeeds without errors

### 4. File Preservation

By default, both the original compressed file and decompressed version are preserved:

```
downloads/
├── archive.tar.gz          # Original compressed file
└── archive.tar             # Decompressed file
```

## Configuration Options

### DownloadConfig Fields

The compression behavior is controlled by these configuration fields:

```rust
pub struct DownloadConfig {
    // ... other fields
    
    /// Enable HTTP compression during downloads
    pub enable_compression: bool,
    
    /// Automatically decompress downloaded files
    pub auto_decompress: bool,
    
    /// List of formats to decompress automatically
    pub decompress_formats: Vec<String>,
}
```

### Default Behavior

When `decompress_formats` is empty, the following formats are enabled by default:
- `gzip`
- `bzip2` 
- `xz`

Archive formats (zip, tar) require explicit configuration due to their multi-file nature.

## Archive.org Integration

### Following Archive.org Guidelines

The compression implementation follows Archive.org best practices:

1. **Respectful Downloads**: HTTP compression reduces server load
2. **Metadata Compliance**: Uses official metadata format indicators
3. **Preservation**: Maintains original file integrity alongside decompressed versions
4. **Format Support**: Supports formats commonly found in Archive.org collections

### Common Use Cases

#### Magazine Archives (e.g., ZZap!64)
```bash
# Download and decompress magazine scans
ia-get --compress --decompress https://archive.org/details/zzap64_issue_001
```

Typically contains:
- Large ZIP files with scanned images
- Compressed PDF versions
- Metadata files

#### Software Archives
```bash
# Download compressed software distributions
ia-get --decompress --decompress-formats gzip,bzip2,xz https://archive.org/details/software_archive
```

Typically contains:
- tar.gz source packages
- bz2 compressed binaries
- xz compressed documentation

#### Document Collections
```bash
# Download and auto-extract document archives
ia-get --compress --decompress https://archive.org/details/document_collection
```

## Technical Implementation

### Architecture

The compression system consists of several components:

1. **compression.rs**: Core decompression algorithms
2. **metadata_storage.rs**: Compression detection methods
3. **enhanced_downloader.rs**: Integration with download pipeline
4. **cli.rs**: Command-line interface options

### Dependencies

Compression support requires these Rust crates:
- `flate2`: gzip compression/decompression
- `bzip2`: bzip2 compression/decompression  
- `xz2`: xz compression/decompression
- `tar`: TAR archive handling

### Error Handling

Compression errors are handled gracefully:
- Failed decompression doesn't fail the entire download
- Original compressed files are preserved if decompression fails
- Detailed error messages help diagnose issues

## Examples

See `examples/compression_demo.rs` for a complete demonstration of the compression features.

## Testing

Run the compression tests:

```bash
# Run library tests (includes compression tests)
cargo test --lib

# Run specific compression module tests
cargo test compression::tests

# Run the compression demo
cargo run --example compression_demo
```

## Future Enhancements

Potential future improvements:
- ZIP file extraction support (requires `zip` crate)
- 7z format support
- Parallel decompression for large files
- Configurable compression levels for uploads
- Resume support for interrupted decompression