//! Stateless file validation operations
//!
//! Pure functions for checksum validation.

use crate::{error::IaGetError, Result};
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
        _ => {
            return Err(IaGetError::FileSystem(format!(
                "Unsupported hash type: {}",
                hash_type
            )));
        }
    };

    Ok(computed_hash.eq_ignore_ascii_case(expected_hash))
}

// TODO: Add SHA1 and SHA256 support
// TODO: Add async version for CLI use

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
}
