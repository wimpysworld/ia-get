//! Stateless file validation operations
//!
//! Pure functions for checksum validation.

use crate::{error::IaGetError, Result};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Validate file checksum
///
/// # Arguments
///
/// * `file_path` - Path to file to validate
/// * `expected_hash` - Expected hash value (hex string)
/// * `hash_type` - Hash algorithm: "md5", "sha1", or "sha256"
///
/// # Returns
///
/// * `Ok(true)` - Hash matches
/// * `Ok(false)` - Hash doesn't match
/// * `Err(IaGetError)` - Validation failed (file not found, etc.)
pub fn validate_checksum<P>(file_path: P, expected_hash: &str, hash_type: &str) -> Result<bool>
where
    P: AsRef<Path>,
{
    let mut file = File::open(file_path.as_ref())
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open file: {}", e)))?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to read file: {}", e)))?;

    let computed_hash = match hash_type.to_lowercase().as_str() {
        "md5" => {
            let digest = md5::compute(&buffer);
            format!("{:x}", digest)
        }
        "sha1" => {
            let mut hasher = Sha1::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            format!("{:x}", result)
        }
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            format!("{:x}", result)
        }
        _ => {
            return Err(IaGetError::FileSystem(format!(
                "Unsupported hash type: {}. Supported: md5, sha1, sha256",
                hash_type
            )));
        }
    };

    Ok(computed_hash.eq_ignore_ascii_case(expected_hash))
}

/// Async version of validate_checksum for CLI use
///
/// This version uses async file I/O for better performance in CLI applications.
///
/// # Arguments
///
/// * `file_path` - Path to file to validate
/// * `expected_hash` - Expected hash value (hex string)
/// * `hash_type` - Hash algorithm: "md5", "sha1", or "sha256"
///
/// # Returns
///
/// * `Ok(true)` - Hash matches
/// * `Ok(false)` - Hash doesn't match
/// * `Err(IaGetError)` - Validation failed (file not found, etc.)
pub async fn validate_checksum_async<P>(
    file_path: P,
    expected_hash: &str,
    hash_type: &str,
) -> Result<bool>
where
    P: AsRef<Path>,
{
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(file_path.as_ref())
        .await
        .map_err(|e| IaGetError::FileSystem(format!("Failed to open file: {}", e)))?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .await
        .map_err(|e| IaGetError::FileSystem(format!("Failed to read file: {}", e)))?;

    let computed_hash = match hash_type.to_lowercase().as_str() {
        "md5" => {
            let digest = md5::compute(&buffer);
            format!("{:x}", digest)
        }
        "sha1" => {
            let mut hasher = Sha1::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            format!("{:x}", result)
        }
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            format!("{:x}", result)
        }
        _ => {
            return Err(IaGetError::FileSystem(format!(
                "Unsupported hash type: {}. Supported: md5, sha1, sha256",
                hash_type
            )));
        }
    };

    Ok(computed_hash.eq_ignore_ascii_case(expected_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_validate_checksum_md5() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        // MD5 of "Hello, world!" is 6cd3556deb0da54bca060b4c39479839
        let result = validate_checksum(temp_file.path(), "6cd3556deb0da54bca060b4c39479839", "md5");

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_validate_checksum_mismatch() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        let result = validate_checksum(temp_file.path(), "wronghash", "md5");

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_validate_checksum_sha1() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        // SHA1 of "Hello, world!" is 943a702d06f34599aee1f8da8ef9f7296031d699
        let result = validate_checksum(
            temp_file.path(),
            "943a702d06f34599aee1f8da8ef9f7296031d699",
            "sha1",
        );

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_validate_checksum_sha256() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        // SHA256 of "Hello, world!" is 315f5bdb76d078c43b8ac0064e4a0164612b1fce77c869345bfc94c75894edd3
        let result = validate_checksum(
            temp_file.path(),
            "315f5bdb76d078c43b8ac0064e4a0164612b1fce77c869345bfc94c75894edd3",
            "sha256",
        );

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_validate_checksum_unsupported() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        let result = validate_checksum(temp_file.path(), "somehash", "unsupported");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));
    }

    #[tokio::test]
    async fn test_validate_checksum_async_md5() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        let result =
            validate_checksum_async(temp_file.path(), "6cd3556deb0da54bca060b4c39479839", "md5")
                .await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_validate_checksum_async_sha256() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello, world!").unwrap();
        temp_file.flush().unwrap();

        let result = validate_checksum_async(
            temp_file.path(),
            "315f5bdb76d078c43b8ac0064e4a0164612b1fce77c869345bfc94c75894edd3",
            "sha256",
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}
