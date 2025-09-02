//! Compression and decompression utilities for ia-get
//!
//! Handles automatic decompression of common archive formats
//! downloaded from Internet Archive following their compression guidelines.

use crate::{IaGetError, Result};
use indicatif::ProgressBar;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Supported compression formats for automatic decompression
#[derive(Debug, Clone, PartialEq)]
pub enum CompressionFormat {
    Gzip,
    Bzip2,
    Xz,
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
}

impl CompressionFormat {
    /// Detect compression format from file extension
    pub fn from_filename(filename: &str) -> Option<Self> {
        let name_lower = filename.to_lowercase();

        if name_lower.ends_with(".tar.gz") {
            Some(CompressionFormat::TarGz)
        } else if name_lower.ends_with(".tar.bz2") {
            Some(CompressionFormat::TarBz2)
        } else if name_lower.ends_with(".tar.xz") {
            Some(CompressionFormat::TarXz)
        } else if name_lower.ends_with(".gz") {
            Some(CompressionFormat::Gzip)
        } else if name_lower.ends_with(".bz2") {
            Some(CompressionFormat::Bzip2)
        } else if name_lower.ends_with(".xz") {
            Some(CompressionFormat::Xz)
        } else if name_lower.ends_with(".zip") {
            Some(CompressionFormat::Zip)
        } else if name_lower.ends_with(".tar") {
            Some(CompressionFormat::Tar)
        } else {
            None
        }
    }

    /// Get the expected output filename after decompression
    pub fn get_decompressed_name(&self, original_name: &str) -> String {
        match self {
            CompressionFormat::Gzip => {
                if original_name.ends_with(".gz") {
                    original_name.trim_end_matches(".gz").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::Bzip2 => {
                if original_name.ends_with(".bz2") {
                    original_name.trim_end_matches(".bz2").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::Xz => {
                if original_name.ends_with(".xz") {
                    original_name.trim_end_matches(".xz").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::TarGz => {
                if original_name.ends_with(".tar.gz") {
                    original_name.trim_end_matches(".gz").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::TarBz2 => {
                if original_name.ends_with(".tar.bz2") {
                    original_name.trim_end_matches(".bz2").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::TarXz => {
                if original_name.ends_with(".tar.xz") {
                    original_name.trim_end_matches(".xz").to_string()
                } else {
                    original_name.to_string()
                }
            }
            CompressionFormat::Zip | CompressionFormat::Tar => {
                // These extract to directories
                Path::new(original_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(original_name)
                    .to_string()
            }
        }
    }
}

/// Decompress a file to the specified output path
pub fn decompress_file<P: AsRef<Path>>(
    input_path: P,
    output_path: P,
    format: CompressionFormat,
    progress_bar: Option<&ProgressBar>,
) -> Result<()> {
    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();

    if let Some(pb) = progress_bar {
        pb.set_message(format!("Decompressing {:?} file...", format));
    }

    match format {
        CompressionFormat::Gzip => decompress_gzip(input_path, output_path)?,
        CompressionFormat::Bzip2 => decompress_bzip2(input_path, output_path)?,
        CompressionFormat::Xz => decompress_xz(input_path, output_path)?,
        CompressionFormat::TarGz => decompress_tar_gz(input_path, output_path)?,
        CompressionFormat::TarBz2 => decompress_tar_bz2(input_path, output_path)?,
        CompressionFormat::TarXz => decompress_tar_xz(input_path, output_path)?,
        CompressionFormat::Zip => decompress_zip(input_path, output_path)?,
        CompressionFormat::Tar => decompress_tar(input_path, output_path)?,
    }

    if let Some(pb) = progress_bar {
        pb.set_message("Decompression completed");
    }

    Ok(())
}

/// Decompress a gzip file
fn decompress_gzip<P: AsRef<Path>>(input_path: P, output_path: P) -> Result<()> {
    use flate2::read::GzDecoder;
    use std::io::copy;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let mut decoder = GzDecoder::new(BufReader::new(input_file));
    let mut output_file = File::create(output_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output file: {}", e)))?;

    copy(&mut decoder, &mut output_file)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to decompress gzip: {}", e)))?;

    Ok(())
}

/// Decompress a bzip2 file
fn decompress_bzip2<P: AsRef<Path>>(input_path: P, output_path: P) -> Result<()> {
    use bzip2::read::BzDecoder;
    use std::io::copy;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let mut decoder = BzDecoder::new(BufReader::new(input_file));
    let mut output_file = File::create(output_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output file: {}", e)))?;

    copy(&mut decoder, &mut output_file)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to decompress bzip2: {}", e)))?;

    Ok(())
}

/// Decompress an xz file
fn decompress_xz<P: AsRef<Path>>(input_path: P, output_path: P) -> Result<()> {
    use std::io::copy;
    use xz2::read::XzDecoder;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let mut decoder = XzDecoder::new(BufReader::new(input_file));
    let mut output_file = File::create(output_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output file: {}", e)))?;

    copy(&mut decoder, &mut output_file)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to decompress xz: {}", e)))?;

    Ok(())
}

/// Decompress a tar.gz file
fn decompress_tar_gz<P: AsRef<Path>>(input_path: P, output_dir: P) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let decoder = GzDecoder::new(BufReader::new(input_file));
    let mut archive = Archive::new(decoder);

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    archive
        .unpack(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to extract tar.gz: {}", e)))?;

    Ok(())
}

/// Decompress a tar.bz2 file
fn decompress_tar_bz2<P: AsRef<Path>>(input_path: P, output_dir: P) -> Result<()> {
    use bzip2::read::BzDecoder;
    use tar::Archive;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let decoder = BzDecoder::new(BufReader::new(input_file));
    let mut archive = Archive::new(decoder);

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    archive
        .unpack(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to extract tar.bz2: {}", e)))?;

    Ok(())
}

/// Decompress a tar.xz file
fn decompress_tar_xz<P: AsRef<Path>>(input_path: P, output_dir: P) -> Result<()> {
    use tar::Archive;
    use xz2::read::XzDecoder;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open compressed file: {}", e)))?;

    let decoder = XzDecoder::new(BufReader::new(input_file));
    let mut archive = Archive::new(decoder);

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    archive
        .unpack(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to extract tar.xz: {}", e)))?;

    Ok(())
}

/// Decompress a ZIP file
fn decompress_zip<P: AsRef<Path>>(input_path: P, output_dir: P) -> Result<()> {
    use zip::ZipArchive;

    let input_file = File::open(input_path.as_ref())
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open ZIP file: {}", e)))?;

    let mut archive = ZipArchive::new(BufReader::new(input_file))
        .map_err(|e| IaGetError::Parse(format!("Failed to read ZIP archive: {}", e)))?;

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| IaGetError::Parse(format!("Failed to access ZIP entry {}: {}", i, e)))?;

        let outpath = match file.enclosed_name() {
            Some(path) => output_dir.as_ref().join(path),
            None => continue, // Skip invalid paths
        };

        if file.name().ends_with('/') {
            // Directory
            std::fs::create_dir_all(&outpath)
                .map_err(|e| IaGetError::FileSystem(format!("Failed to create directory: {}", e)))?;
        } else {
            // File
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)
                        .map_err(|e| IaGetError::FileSystem(format!("Failed to create parent directory: {}", e)))?;
                }
            }
            let mut outfile = File::create(&outpath)
                .map_err(|e| IaGetError::FileSystem(format!("Failed to create output file: {}", e)))?;
            
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| IaGetError::FileSystem(format!("Failed to extract file: {}", e)))?;
        }

        // Get and set permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| IaGetError::FileSystem(format!("Failed to set file permissions: {}", e)))?;
            }
        }
    }

    Ok(())
}

/// Extract a TAR file (uncompressed)
fn decompress_tar<P: AsRef<Path>>(input_path: P, output_dir: P) -> Result<()> {
    use tar::Archive;

    let input_file = File::open(input_path)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open tar file: {}", e)))?;

    let mut archive = Archive::new(BufReader::new(input_file));

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;

    archive
        .unpack(&output_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to extract tar: {}", e)))?;

    Ok(())
}

/// Check if automatic decompression is enabled for this format
pub fn should_decompress(format: &CompressionFormat, enabled_formats: &[String]) -> bool {
    if enabled_formats.is_empty() {
        // If no specific formats are configured, enable for common text-based compressions
        matches!(
            format,
            CompressionFormat::Gzip
                | CompressionFormat::Bzip2
                | CompressionFormat::Xz
                | CompressionFormat::TarGz // Include tar.gz in default decompression
        )
    } else {
        let format_str = match format {
            CompressionFormat::Gzip => "gzip",
            CompressionFormat::Bzip2 => "bzip2",
            CompressionFormat::Xz => "xz",
            CompressionFormat::Zip => "zip",
            CompressionFormat::Tar => "tar",
            CompressionFormat::TarGz => "tar.gz",
            CompressionFormat::TarBz2 => "tar.bz2",
            CompressionFormat::TarXz => "tar.xz",
        };

        enabled_formats
            .iter()
            .any(|f| f.to_lowercase() == format_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_format_detection() {
        assert_eq!(
            CompressionFormat::from_filename("test.tar.gz"),
            Some(CompressionFormat::TarGz)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.tar.bz2"),
            Some(CompressionFormat::TarBz2)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.tar.xz"),
            Some(CompressionFormat::TarXz)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.gz"),
            Some(CompressionFormat::Gzip)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.bz2"),
            Some(CompressionFormat::Bzip2)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.xz"),
            Some(CompressionFormat::Xz)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.zip"),
            Some(CompressionFormat::Zip)
        );
        assert_eq!(
            CompressionFormat::from_filename("test.tar"),
            Some(CompressionFormat::Tar)
        );
        assert_eq!(CompressionFormat::from_filename("test.txt"), None);
    }

    #[test]
    fn test_decompressed_name_generation() {
        let gzip = CompressionFormat::Gzip;
        assert_eq!(gzip.get_decompressed_name("test.txt.gz"), "test.txt");

        let tar_gz = CompressionFormat::TarGz;
        assert_eq!(
            tar_gz.get_decompressed_name("archive.tar.gz"),
            "archive.tar"
        );

        let zip = CompressionFormat::Zip;
        assert_eq!(zip.get_decompressed_name("archive.zip"), "archive");
    }

    #[test]
    fn test_should_decompress() {
        let gzip = CompressionFormat::Gzip;
        let zip = CompressionFormat::Zip;

        // Empty config enables common formats
        assert!(should_decompress(&gzip, &[]));
        assert!(!should_decompress(&zip, &[]));

        // Explicit config
        let formats = vec!["zip".to_string()];
        assert!(!should_decompress(&gzip, &formats));
        assert!(should_decompress(&zip, &formats));
    }

    #[test]
    fn test_zip_decompression_error_handling() {
        // Test that the ZIP decompression function handles invalid input correctly
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a temporary file that's not a valid ZIP
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"not a zip file").unwrap();
        
        let temp_dir = tempfile::tempdir().unwrap();
        
        // This should fail gracefully with an error
        let result = super::decompress_zip(temp_file.path(), temp_dir.path());
        assert!(result.is_err());
        
        // Verify the error message indicates it's a ZIP parsing issue
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Failed to read ZIP archive"));
    }
}
