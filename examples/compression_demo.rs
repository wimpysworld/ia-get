//! Example demonstrating compression functionality
//!
//! This example shows how the compression features work without requiring
//! a full working main.rs binary.

use ia_get::compression::{CompressionFormat, should_decompress};
use ia_get::metadata_storage::ArchiveFile;

fn main() {
    println!("=== ia-get Compression Features Demo ===\n");

    // 1. Demonstrate compression format detection
    println!("1. Compression Format Detection:");
    let test_files = vec![
        "archive.tar.gz",
        "data.bz2",
        "document.xz",
        "backup.zip",
        "readme.txt",
    ];

    for filename in test_files {
        let format = CompressionFormat::from_filename(filename);
        println!("  {} -> {:?}", filename, format);
    }

    // 2. Demonstrate Archive.org file compression detection
    println!("\n2. Archive.org File Compression Detection:");

    let archive_files = vec![
        ArchiveFile {
            name: "zzap64_issue_001.zip".to_string(),
            source: "original".to_string(),
            format: Some("ZIP".to_string()),
            mtime: Some(1234567890),
            size: Some(50_000_000),
            md5: Some("abcd1234".to_string()),
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
            rotation: None,
        },
        ArchiveFile {
            name: "magazine_scans.tar.gz".to_string(),
            source: "derivative".to_string(),
            format: Some("gzip".to_string()),
            mtime: Some(1234567890),
            size: Some(150_000_000),
            md5: Some("efgh5678".to_string()),
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
            rotation: None,
        },
        ArchiveFile {
            name: "metadata.xml".to_string(),
            source: "metadata".to_string(),
            format: Some("Metadata".to_string()),
            mtime: Some(1234567890),
            size: Some(1024),
            md5: Some("ijkl9012".to_string()),
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
            rotation: None,
        },
    ];

    for file in &archive_files {
        println!("  File: {}", file.name);
        println!("    Compressed: {}", file.is_compressed());
        if file.is_compressed() {
            println!("    Format: {:?}", file.get_compression_format());
            println!("    Decompressed name: {}", file.get_decompressed_name());
        }
        println!();
    }

    // 3. Demonstrate compression configuration
    println!("3. Compression Configuration:");

    let compression_configs = [
        vec![], // Default configuration
        vec!["gzip".to_string()],
        vec!["zip".to_string(), "bzip2".to_string()],
        vec!["all".to_string()],
    ];

    let test_formats = vec![
        CompressionFormat::Gzip,
        CompressionFormat::Zip,
        CompressionFormat::Bzip2,
        CompressionFormat::TarGz,
    ];

    for (i, config) in compression_configs.iter().enumerate() {
        println!("  Configuration {}: {:?}", i + 1, config);
        for format in &test_formats {
            let should_decomp = should_decompress(format, config);
            println!("    {:?} -> {}", format, should_decomp);
        }
        println!();
    }

    // 4. Show the typical Archive.org use case
    println!("4. Typical Archive.org Use Cases:");
    println!("  - Magazine scans (zzap64_issue_001.zip): Large ZIP archives with many images");
    println!("  - Compressed backups (data.tar.gz): Multi-file archives with gzip compression");
    println!("  - Individual compressed files (document.bz2): Single files with bzip2 compression");
    println!("  - Metadata files (usually uncompressed XML/JSON)");

    println!("\n=== Features Summary ===");
    println!("✓ HTTP compression headers (Accept-Encoding: gzip, deflate, br)");
    println!("✓ Automatic compression detection from Archive.org metadata");
    println!("✓ Support for gzip, bzip2, xz, tar, and combined formats");
    println!("✓ Configurable auto-decompression with format filters");
    println!("✓ Transparent decompression after download verification");
    println!("✓ Preservation of original compressed files alongside decompressed versions");

    println!("\n=== Command Line Usage ===");
    println!("ia-get --compress --decompress <archive_url>");
    println!("ia-get --compress --decompress --decompress-formats gzip,bzip2 <archive_url>");
}
