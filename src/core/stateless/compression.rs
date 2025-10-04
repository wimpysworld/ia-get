//! Stateless compression and decompression operations
//!
//! Pure functions for handling archive files.

use crate::{error::IaGetError, utilities::compression::CompressionFormat, Result};
use std::fs;
use std::path::Path;

/// Decompress an archive file
///
/// Supports: zip, gzip, bzip2, xz, tar, tar.gz, tar.bz2, tar.xz
///
/// # Arguments
///
/// * `archive_path` - Path to archive file
/// * `output_dir` - Directory to extract to
///
/// # Returns
///
/// * `Ok(Vec<String>)` - List of extracted file paths
/// * `Err(IaGetError)` - Extraction failed
pub fn decompress_archive<P1, P2>(archive_path: P1, output_dir: P2) -> Result<Vec<String>>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let archive_path = archive_path.as_ref();
    let output_dir = output_dir.as_ref();

    // Detect format from filename
    let format = CompressionFormat::from_filename(
        archive_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| IaGetError::FileSystem("Invalid archive filename".to_string()))?,
    )
    .ok_or_else(|| {
        IaGetError::FileSystem(format!(
            "Unsupported archive format: {}",
            archive_path.display()
        ))
    })?;

    // Ensure output directory exists
    fs::create_dir_all(output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    // For single-file compression (gzip, bzip2, xz), output is a file
    // For archives (zip, tar, etc.), output is the directory
    let extracted_files = match format {
        CompressionFormat::Gzip | CompressionFormat::Bzip2 | CompressionFormat::Xz => {
            let output_name = format.get_decompressed_name(
                archive_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("output"),
            );
            let output_path = output_dir.join(&output_name);

            // Use existing decompression utilities
            crate::utilities::compression::decompress_file(
                archive_path,
                &output_path,
                format,
                None, // No progress bar for stateless function
            )?;

            vec![output_path.to_string_lossy().to_string()]
        }
        _ => {
            // For archives (tar, zip, etc.), decompress to directory
            crate::utilities::compression::decompress_file(archive_path, output_dir, format, None)?;

            // Collect all extracted files
            collect_files_recursively(output_dir)?
        }
    };

    Ok(extracted_files)
}

/// Recursively collect all files in a directory
fn collect_files_recursively<P: AsRef<Path>>(dir: P) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let dir = dir.as_ref();

    if dir.is_dir() {
        for entry in fs::read_dir(dir)
            .map_err(|e| IaGetError::FileSystem(format!("Failed to read directory: {}", e)))?
        {
            let entry = entry
                .map_err(|e| IaGetError::FileSystem(format!("Failed to read entry: {}", e)))?;
            let path = entry.path();

            if path.is_file() {
                files.push(path.to_string_lossy().to_string());
            } else if path.is_dir() {
                files.extend(collect_files_recursively(&path)?);
            }
        }
    }

    Ok(files)
}

/// Decompress archive and return JSON array of extracted files
///
/// # Arguments
///
/// * `archive_path` - Path to archive file
/// * `output_dir` - Directory to extract to
///
/// # Returns
///
/// * `Ok(String)` - JSON array of extracted file paths
/// * `Err(IaGetError)` - Extraction failed
pub fn decompress_archive_json<P1, P2>(archive_path: P1, output_dir: P2) -> Result<String>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let files = decompress_archive(archive_path, output_dir)?;

    serde_json::to_string(&files)
        .map_err(|e| IaGetError::Parse(format!("Failed to serialize file list: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_decompress_archive_unsupported() {
        let dir = tempdir().unwrap();
        let archive_path = dir.path().join("test.unknown");
        File::create(&archive_path).unwrap();

        let output_dir = dir.path().join("output");

        let result = decompress_archive(&archive_path, &output_dir);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported archive format"));
    }

    #[test]
    fn test_collect_files_recursively() {
        let dir = tempdir().unwrap();

        // Create some test files
        File::create(dir.path().join("file1.txt")).unwrap();
        File::create(dir.path().join("file2.txt")).unwrap();

        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        File::create(subdir.join("file3.txt")).unwrap();

        let files = collect_files_recursively(dir.path()).unwrap();

        assert_eq!(files.len(), 3);
    }
}
