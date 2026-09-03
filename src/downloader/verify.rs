//! Integrity verification of local files: the (cheap) size check that runs
//! first, and the full-file MD5 hash that confirms the contents.

use std::fs;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use colored::*;
use digest::Digest;

use crate::Result;
use crate::display::{create_progress_bar, finish_progress_bar, format_size, last_glyph};
use crate::error::{IaGetError, io_error_with_path};

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (2MB)
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

/// The lowercase hex rendering of a hash digest.
pub(crate) fn digest_hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Calculates the MD5 hash of a file
pub(crate) fn calculate_md5(file_path: &str, running: &Arc<AtomicBool>) -> Result<String> {
    let file = File::open(file_path).map_err(|e| io_error_with_path(file_path, e))?;
    let file_size = file
        .metadata()
        .map_err(|e| io_error_with_path(file_path, e))?
        .len();
    let is_large_file = file_size > LARGE_FILE_THRESHOLD;

    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut context = md5::Md5::new();
    let mut buffer = [0; BUFFER_SIZE];

    let pb = is_large_file.then(|| {
        create_progress_bar(
            file_size,
            &format!("{} {}    ", last_glyph(), "Verifying".white()),
            "blue/blue",
            false,
        )
    });

    let mut bytes_processed: u64 = 0;

    loop {
        // A stop requested before a read — including on an empty file,
        // whose first read is EOF and would otherwise return its digest
        // unchecked — must not let the run report success
        if !running.load(Ordering::SeqCst) {
            finish_progress_bar(&pb);
            return Err(IaGetError::Interrupted);
        }

        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|e| io_error_with_path(file_path, e))?;
        // A stop can land between the check above and this read returning
        // EOF: re-check before breaking, so the final read cannot report a
        // digest for an already-stopped run
        if !running.load(Ordering::SeqCst) {
            finish_progress_bar(&pb);
            return Err(IaGetError::Interrupted);
        }
        if bytes_read == 0 {
            break;
        }

        context.update(&buffer[..bytes_read]);

        if let Some(ref progress_bar) = pb {
            bytes_processed += bytes_read as u64;
            progress_bar.set_position(bytes_processed);
        }
    }

    finish_progress_bar(&pb);

    Ok(digest_hex(context.finalize()))
}

/// Outcome of checking whether an already-downloaded file is still valid
#[derive(Debug)]
pub(crate) enum ExistingFileStatus {
    /// The file does not exist and must be downloaded
    Missing,
    /// The file exists and passed verification; `md5` is the verified hash
    /// when archive.org provided one to compare against
    Verified { md5: Option<String> },
    /// The file exists but failed verification
    Invalid,
    /// The file could not be read (an I/O error): it is left untouched and
    /// the problem is reported for this file only, because a read failure
    /// proves nothing about the file's contents
    Unreadable(String),
}

/// Whether a local hex digest matches the one from the metadata: hex
/// digests compare case-insensitively, as the metadata may carry an
/// uppercase or mixed-case MD5.
fn hash_matches(local: &str, expected: &str) -> bool {
    local.eq_ignore_ascii_case(expected)
}

/// Size guard shared by the pre-download check and the post-download
/// verification.
///
/// Returns `Some((actual, expected))` when the local file's size differs
/// from the metadata, `None` when the sizes match or no size is known.
fn size_mismatch(file_path: &str, expected_size: Option<u64>) -> Result<Option<(u64, u64)>> {
    let Some(expected) = expected_size else {
        return Ok(None);
    };

    let actual = fs::metadata(file_path)
        .map_err(|e| io_error_with_path(file_path, e))?
        .len();
    if actual == expected {
        Ok(None)
    } else {
        Ok(Some((actual, expected)))
    }
}

/// The verdict for a file type that exists at the final path but cannot be
/// verified as a file, or `None` for a regular file (the normal case):
///
/// - a symlink — valid or dangling (`exists()` follows the link and would
///   misreport a dangling one as Missing) — would be followed by every
///   check below (size, hash, and the mtime sync or removal the caller
///   performs);
/// - a directory can neither be verified as a file nor safely removed;
/// - a FIFO (or other special file) would hang the size/hash reads
///   indefinitely.
///
/// All three are reported and left in place.
fn non_regular_file(meta: &fs::Metadata) -> Option<ExistingFileStatus> {
    if meta.file_type().is_symlink() {
        Some(ExistingFileStatus::Unreadable(
            "a symlink occupies the file path".to_string(),
        ))
    } else if meta.file_type().is_dir() {
        Some(ExistingFileStatus::Unreadable(
            "a directory occupies the file path".to_string(),
        ))
    } else if !meta.file_type().is_file() {
        Some(ExistingFileStatus::Unreadable(
            "a non-regular file occupies the file path".to_string(),
        ))
    } else {
        None
    }
}

/// Check if an existing file is still valid: the (cheap) size check runs
/// first so a stale or truncated copy never spends a full-file hash
/// before being rejected
pub(crate) fn check_existing_file(
    file_path: &str,
    expected_md5: Option<&str>,
    expected_size: Option<u64>,
    running: &Arc<AtomicBool>,
) -> Result<ExistingFileStatus> {
    let path = Path::new(file_path);
    match fs::symlink_metadata(path) {
        // Present: a regular file proceeds to the size/hash checks below,
        // everything else (symlink, directory, FIFO, ...) is reported as
        // unreadable and left in place
        Ok(meta) => {
            if let Some(status) = non_regular_file(&meta) {
                return Ok(status);
            }
        }
        // Absent: download from scratch.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingFileStatus::Missing);
        }
        // Unreadable even to stat (e.g. no permission): nothing can be
        // verified, the file is kept and the problem reported.
        Err(e) => {
            return Ok(ExistingFileStatus::Unreadable(format!(
                "could not read file: {e}"
            )));
        }
    }

    let mismatch = match size_mismatch(file_path, expected_size) {
        Ok(mismatch) => mismatch,
        Err(e) => {
            return Ok(ExistingFileStatus::Unreadable(format!(
                "could not read file size: {e}"
            )));
        }
    };
    if mismatch.is_some() {
        return Ok(ExistingFileStatus::Invalid);
    }

    let Some(expected_md5) = expected_md5 else {
        // No hash to compare against and the size matches: verified
        return Ok(ExistingFileStatus::Verified { md5: None });
    };

    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            if matches!(e, IaGetError::Interrupted) {
                return Err(e);
            }
            // "Could not read" is not "corrupted": the file is kept as is
            // and the problem is reported for this file only.
            return Ok(ExistingFileStatus::Unreadable(format!(
                "could not verify existing file: {e}"
            )));
        }
    };

    if hash_matches(&local_md5, expected_md5) {
        Ok(ExistingFileStatus::Verified {
            md5: Some(local_md5),
        })
    } else {
        Ok(ExistingFileStatus::Invalid)
    }
}

/// Print the line shown once a file has been verified, using the same
/// format for freshly downloaded and already-verified files
pub(crate) fn print_verified_hash(md5: Option<&str>) {
    match md5 {
        Some(hash) => println!(
            "{} {}         {} {}",
            last_glyph(),
            "Hash".white(),
            "✔".green().bold(),
            format!("({})", hash).dimmed()
        ),
        None => println!(
            "{} {}",
            "-".dimmed(),
            "No MD5 hash provided for verification.".dimmed()
        ),
    }
}

/// Verify a downloaded file's size and hash against expected values
pub(crate) fn verify_downloaded_file(
    file_path: &str,
    expected_md5: Option<&str>,
    expected_size: Option<u64>,
    running: &Arc<AtomicBool>,
) -> Result<bool> {
    if let Some((actual_size, expected_size)) = size_mismatch(file_path, expected_size)? {
        println!(
            "{} {}         {} {} (expected {})",
            last_glyph(),
            "Size".white(),
            "✘".red().bold(),
            format_size(actual_size).red(),
            format_size(expected_size).dimmed()
        );
        return Ok(false);
    }

    let Some(expected_md5) = expected_md5 else {
        // No hash to check against, consider it verified
        print_verified_hash(None);
        return Ok(true);
    };

    let local_md5 = calculate_md5(file_path, running)?;
    if hash_matches(&local_md5, expected_md5) {
        print_verified_hash(Some(&local_md5));
        Ok(true)
    } else {
        println!(
            "{} {}         {} ({}) Expected ({})",
            last_glyph(),
            "Hash".white(),
            "✘".red().bold(),
            local_md5.red(),
            expected_md5.dimmed()
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TempDir, md5_hex, test_running};

    #[test]
    fn size_mismatch_short_circuits_before_hashing() {
        // A copy whose size does not match the metadata must be rejected
        // as Invalid without spending a full-file hash.
        let dir = TempDir::new("size_short_circuit");
        let path = dir.join("file.bin");
        fs::write(&path, b"short").unwrap(); // 5 bytes

        let expected = md5_hex(b"totally different content");
        let status = check_existing_file(
            path.to_str().unwrap(),
            Some(expected.as_str()),
            Some(100),
            &test_running(),
        )
        .unwrap();

        assert!(
            matches!(status, ExistingFileStatus::Invalid),
            "got {status:?}"
        );
        assert!(
            path.exists(),
            "the stale copy is removed by the caller, not by the check"
        );
    }

    #[test]
    fn unreadable_existing_file_is_reported_not_redownloaded() {
        // A directory at the file path must yield Unreadable (per-file
        // failure, the entry is kept), not Invalid, which would delete it
        // and re-download based on a read failure.
        let dir = TempDir::new("unreadable_existing");
        let path = dir.join("dirfile.bin");
        fs::create_dir(&path).unwrap();

        let expected = md5_hex(b"x");
        let status = check_existing_file(
            path.to_str().unwrap(),
            Some(expected.as_str()),
            None,
            &test_running(),
        )
        .unwrap();

        match status {
            ExistingFileStatus::Unreadable(reason) => {
                assert!(!reason.is_empty(), "the reason must name the problem");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
        assert!(path.exists(), "an unreadable file must not be deleted");
    }

    #[test]
    fn interrupted_before_hashing_an_empty_file() {
        // An empty file hits EOF on the first read: without a pre-read
        // interrupt check its digest would be returned for an already
        // stopped run
        let dir = TempDir::new("md5_interrupt_empty");
        let path = dir.join("empty.bin");
        fs::write(&path, b"").unwrap();
        let stopped = Arc::new(AtomicBool::new(false));

        let err = calculate_md5(path.to_str().unwrap(), &stopped).unwrap_err();
        assert!(
            matches!(err, IaGetError::Interrupted),
            "a stop before the first read must abort the hash, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_at_final_path_is_unreadable() {
        // A symlink at the final name whose target happens to match the
        // metadata must not verify through the link: the checks would
        // follow it into the link target.
        let dir = TempDir::new("symlink_final");
        let target = dir.join("target.bin");
        fs::write(&target, b"matching-content").unwrap();
        let link = dir.join("file.bin");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let status = check_existing_file(
            link.to_str().unwrap(),
            Some(md5_hex(b"matching-content").as_str()),
            Some(16),
            &test_running(),
        )
        .unwrap();

        match status {
            ExistingFileStatus::Unreadable(reason) => {
                assert!(reason.contains("symlink"), "got: {reason}");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
        assert!(link.exists(), "the symlink must be left in place");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_at_final_path_is_unreadable_not_missing() {
        // exists() follows the link and would misreport a dangling one as
        // Missing: a download would then run pointlessly and only fail at
        // the install step. The link itself must be reported.
        let dir = TempDir::new("dangling_symlink");
        let gone = dir.join("gone.bin");
        let link = dir.join("file.bin");
        std::os::unix::fs::symlink(&gone, &link).unwrap();

        let status =
            check_existing_file(link.to_str().unwrap(), None, Some(16), &test_running()).unwrap();

        match status {
            ExistingFileStatus::Unreadable(reason) => {
                assert!(reason.contains("symlink"), "got: {reason}");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn fifo_at_final_path_is_unreadable() {
        // A FIFO (or other special file) where the file is expected must
        // not be opened for reading: a size/hash read on it would block.
        let dir = TempDir::new("fifo_final");
        let fifo = dir.join("file.bin");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            eprintln!("skipping: mkfifo unavailable");
            return;
        }

        let status =
            check_existing_file(fifo.to_str().unwrap(), None, Some(16), &test_running()).unwrap();

        match status {
            ExistingFileStatus::Unreadable(reason) => {
                assert!(reason.contains("non-regular"), "got: {reason}");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }
}
