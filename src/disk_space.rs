//! Pre-download disk-space estimation and verification for destination volumes.

use std::fs;
use std::path::Path;

use crate::downloader::{self, ShutdownSignal};
use crate::error::IaGetError;
use crate::Result;

/// A planned file download used to estimate remaining disk space requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDownload {
    /// Absolute download URL for the file.
    pub url: String,
    /// Sanitised relative destination path inside the output directory.
    pub file_path: String,
    /// Optional expected MD5 digest from archive metadata.
    pub expected_md5: Option<String>,
    /// Optional expected file size in bytes from archive metadata.
    pub expected_size: Option<u64>,
}

/// Returns `true` only when a contained regular final file already satisfies the planned download.
fn has_verified_final_file(
    canonical_root: &Path,
    item: &PlannedDownload,
    signal: &ShutdownSignal,
) -> Result<bool> {
    let rel_path = Path::new(&item.file_path);
    let Ok(full_path) = downloader::validate_contained_path(canonical_root, rel_path, true) else {
        return Ok(false);
    };

    let Ok(meta) = fs::symlink_metadata(&full_path) else {
        return Ok(false);
    };
    if !meta.file_type().is_file() {
        return Ok(false);
    }

    let Some(expected_hash) = item.expected_md5.as_deref() else {
        return Ok(true);
    };

    if !signal.is_running() {
        return Err(IaGetError::Interrupted);
    }

    match downloader::calculate_md5_with_progress(
        canonical_root,
        rel_path,
        signal.running_flag(),
        false,
    ) {
        Ok(actual_md5) => {
            if !signal.is_running() {
                return Err(IaGetError::Interrupted);
            }
            Ok(actual_md5 == expected_hash)
        }
        Err(err) => {
            if !signal.is_running()
                || matches!(err, IaGetError::Interrupted)
                || err.to_string().contains("interrupted by signal")
            {
                return Err(IaGetError::Interrupted);
            }
            Ok(false)
        }
    }
}

/// Returns the byte length of a contained regular `.part` file, or `0` if absent or untrusted.
fn trusted_part_file_len(canonical_root: &Path, file_path: &str) -> u64 {
    let part_rel = downloader::part_file_path(file_path);
    let Ok(part_full) = downloader::validate_contained_path(canonical_root, &part_rel, true) else {
        return 0;
    };

    match fs::symlink_metadata(&part_full) {
        Ok(meta) if meta.file_type().is_file() => meta.len(),
        _ => 0,
    }
}

/// Estimates remaining bytes needed for a single planned item inside `canonical_root`.
fn remaining_bytes_for_item(
    canonical_root: &Path,
    item: &PlannedDownload,
    signal: &ShutdownSignal,
) -> Result<u64> {
    let Some(expected) = item.expected_size.filter(|&size| size > 0) else {
        return Ok(0);
    };

    if has_verified_final_file(canonical_root, item, signal)? {
        return Ok(0);
    }

    let part_len = trusted_part_file_len(canonical_root, &item.file_path);
    if part_len > expected {
        // Oversized `.part` files are discarded and re-fetched from scratch on HTTP 416.
        Ok(expected)
    } else {
        Ok(expected.saturating_sub(part_len))
    }
}

fn calculate_required_bytes_in_canonical_root(
    canonical_root: &Path,
    items: &[PlannedDownload],
    signal: &ShutdownSignal,
) -> Result<u64> {
    if !signal.is_running() {
        return Err(IaGetError::Interrupted);
    }

    let mut total: u64 = 0;
    for item in items {
        if !signal.is_running() {
            return Err(IaGetError::Interrupted);
        }
        let remaining = remaining_bytes_for_item(canonical_root, item, signal)?;
        total = total.saturating_add(remaining);
    }

    Ok(total)
}

/// Calculates the remaining byte count required to download `items` into `root`.
///
/// Accounts for already verified regular destination files and contained regular `.part` files,
/// skips entries with unknown or zero expected size, and stops promptly when `signal` triggers.
pub fn calculate_required_bytes(
    root: &Path,
    items: &[PlannedDownload],
    signal: &ShutdownSignal,
) -> Result<u64> {
    let canonical_root = downloader::resolve_destination_root(root)?;
    calculate_required_bytes_in_canonical_root(&canonical_root, items, signal)
}

/// Checks whether `available_bytes` is sufficient for `items` in `root`, honouring `signal`.
pub fn check_disk_space_with_available_and_signal(
    root: &Path,
    items: &[PlannedDownload],
    available_bytes: u64,
    signal: &ShutdownSignal,
) -> Result<()> {
    let required = calculate_required_bytes(root, items, signal)?;
    if required == 0 {
        return Ok(());
    }

    if required > available_bytes {
        return Err(IaGetError::InsufficientDiskSpace {
            required,
            available: available_bytes,
            path: root.to_path_buf(),
        });
    }

    Ok(())
}

/// Checks whether `available_bytes` is sufficient for `items` in `root`.
pub fn check_disk_space_with_available(
    root: &Path,
    items: &[PlannedDownload],
    available_bytes: u64,
) -> Result<()> {
    check_disk_space_with_available_and_signal(root, items, available_bytes, &ShutdownSignal::new())
}

/// Checks available disk space on the destination volume for `root` against `items`,
/// honouring `signal`.
pub fn check_disk_space_with_signal(
    root: &Path,
    items: &[PlannedDownload],
    signal: &ShutdownSignal,
) -> Result<()> {
    let canonical_root = downloader::resolve_destination_root(root)?;
    let required = calculate_required_bytes_in_canonical_root(&canonical_root, items, signal)?;
    if required == 0 {
        return Ok(());
    }

    let available = fs4::available_space(&canonical_root).map_err(|err| {
        IaGetError::FileSystem(format!(
            "Failed to determine available disk space for '{}': {}",
            root.display(),
            err
        ))
    })?;

    if required > available {
        return Err(IaGetError::InsufficientDiskSpace {
            required,
            available,
            path: root.to_path_buf(),
        });
    }

    Ok(())
}

/// Checks available disk space on the destination volume for `root` against `items`.
pub fn check_disk_space(root: &Path, items: &[PlannedDownload]) -> Result<()> {
    check_disk_space_with_signal(root, items, &ShutdownSignal::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempTestDir(PathBuf);

    impl TempTestDir {
        fn new(label: &str) -> Self {
            let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ia-get-disk-space-{label}-{}-{nanos}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temp test dir");
            Self(path)
        }

        fn join(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn planned(file_path: &str, expected_size: Option<u64>, md5: Option<&str>) -> PlannedDownload {
        PlannedDownload {
            url: format!("https://archive.org/download/item/{file_path}"),
            file_path: file_path.to_string(),
            expected_md5: md5.map(str::to_string),
            expected_size,
        }
    }

    #[test]
    fn calculate_required_bytes_sums_missing_skips_unknown_and_zero_and_saturates() {
        let sandbox = TempTestDir::new("basic");
        let signal = ShutdownSignal::new();

        let items = vec![
            planned("a.bin", Some(1_000), None),
            planned("unknown.bin", None, None),
            planned("empty.bin", Some(0), None),
            planned("nested/b.bin", Some(2_500), None),
        ];

        assert_eq!(
            calculate_required_bytes(&sandbox.0, &items, &signal).unwrap(),
            3_500
        );

        let overflowing = vec![
            planned("huge1.bin", Some(u64::MAX - 10), None),
            planned("huge2.bin", Some(100), None),
        ];
        assert_eq!(
            calculate_required_bytes(&sandbox.0, &overflowing, &signal).unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn calculate_required_bytes_accounts_for_verified_final_files_and_part_files() {
        let sandbox = TempTestDir::new("existing-and-part");
        let signal = ShutdownSignal::new();

        // Verified final file with matching MD5 -> 0 remaining
        let good_bytes = b"complete-file-content";
        let good_md5 = format!("{:x}", md5::compute(good_bytes));
        fs::write(sandbox.join("verified.bin"), good_bytes).unwrap();

        // Final file without expected MD5 -> trusted as complete -> 0 remaining
        fs::write(sandbox.join("no_md5.bin"), b"present").unwrap();

        // Corrupt final file with no .part -> full expected size required
        fs::write(sandbox.join("corrupt_no_part.bin"), b"bad").unwrap();

        // Corrupt final file with partial .part -> expected - part_len
        fs::write(sandbox.join("corrupt_with_part.bin"), b"bad").unwrap();
        fs::write(sandbox.join("corrupt_with_part.bin.part"), vec![0u8; 40]).unwrap();

        // Missing final file with valid .part -> expected - part_len
        fs::write(sandbox.join("resuming.bin.part"), vec![0u8; 30]).unwrap();

        // Missing final file with exact-size .part -> 0 remaining
        fs::write(sandbox.join("complete_part.bin.part"), vec![0u8; 50]).unwrap();

        // Missing final file with oversized .part -> full expected size (discarded on HTTP 416)
        fs::write(sandbox.join("oversized_part.bin.part"), vec![0u8; 120]).unwrap();

        let items = vec![
            planned(
                "verified.bin",
                Some(good_bytes.len() as u64),
                Some(&good_md5),
            ),
            planned("no_md5.bin", Some(500), None),
            planned(
                "corrupt_no_part.bin",
                Some(200),
                Some("00000000000000000000000000000000"),
            ),
            planned(
                "corrupt_with_part.bin",
                Some(100),
                Some("11111111111111111111111111111111"),
            ),
            planned("resuming.bin", Some(100), None),
            planned("complete_part.bin", Some(50), None),
            planned("oversized_part.bin", Some(80), None),
        ];

        // 0 + 0 + 200 + (100 - 40) + (100 - 30) + 0 + 80 = 410
        assert_eq!(
            calculate_required_bytes(&sandbox.0, &items, &signal).unwrap(),
            410
        );
    }

    #[test]
    fn calculate_required_bytes_ignores_non_regular_files() {
        let sandbox = TempTestDir::new("non-regular");
        let signal = ShutdownSignal::new();

        // Directory at final path and directory at .part path must not reduce required bytes
        fs::create_dir_all(sandbox.join("as_dir.bin")).unwrap();
        fs::create_dir_all(sandbox.join("part_dir.bin.part")).unwrap();

        let items = vec![
            planned("as_dir.bin", Some(128), None),
            planned("part_dir.bin", Some(256), None),
        ];

        assert_eq!(
            calculate_required_bytes(&sandbox.0, &items, &signal).unwrap(),
            384
        );
    }

    #[cfg(unix)]
    #[test]
    fn calculate_required_bytes_ignores_symlinks_for_final_and_part_files() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("symlinks");
        let outside = TempTestDir::new("outside");
        let signal = ShutdownSignal::new();

        let data = b"external-payload";
        let digest = format!("{:x}", md5::compute(data));
        fs::write(outside.join("target.bin"), data).unwrap();
        fs::write(outside.join("partial.bin"), vec![0u8; 90]).unwrap();

        // Symlinked final file
        symlink(outside.join("target.bin"), sandbox.join("linked_final.bin")).unwrap();
        // Symlinked .part file
        symlink(
            outside.join("partial.bin"),
            sandbox.join("linked_part.bin.part"),
        )
        .unwrap();
        // Symlinked parent directory
        symlink(&outside.0, sandbox.join("linked_dir")).unwrap();

        let items = vec![
            planned("linked_final.bin", Some(100), Some(&digest)),
            planned("linked_part.bin", Some(100), None),
            planned("linked_dir/target.bin", Some(100), Some(&digest)),
        ];

        assert_eq!(
            calculate_required_bytes(&sandbox.0, &items, &signal).unwrap(),
            300
        );
    }

    #[test]
    fn calculate_required_bytes_propagates_shutdown_interrupt() {
        let sandbox = TempTestDir::new("interrupt");
        let stopped = ShutdownSignal::new();
        stopped.trigger();

        let items = vec![planned("file.bin", Some(100), None)];
        let err = calculate_required_bytes(&sandbox.0, &items, &stopped).unwrap_err();
        assert!(matches!(err, IaGetError::Interrupted));
    }

    #[test]
    fn check_disk_space_with_available_enforces_boundary_and_formats_error() {
        let sandbox = TempTestDir::new("available");
        let items = vec![planned("archive.iso", Some(2 * 1024 * 1024), None)];

        // Exact fit succeeds
        check_disk_space_with_available(&sandbox.0, &items, 2 * 1024 * 1024).unwrap();

        // One byte short fails with InsufficientDiskSpace formatted via format_size
        let err = check_disk_space_with_available(&sandbox.0, &items, 1024 * 1024).unwrap_err();
        match &err {
            IaGetError::InsufficientDiskSpace {
                required,
                available,
                path,
            } => {
                assert_eq!(*required, 2 * 1024 * 1024);
                assert_eq!(*available, 1024 * 1024);
                assert_eq!(path, &sandbox.0);
            }
            other => panic!("expected InsufficientDiskSpace, got {other:?}"),
        }

        let msg = err.to_string();
        assert!(msg.contains("Insufficient disk space"));
        assert!(msg.contains("2.00MB"));
        assert!(msg.contains("1.00MB"));
        assert!(msg.contains(&sandbox.0.display().to_string()));
    }

    #[test]
    fn check_disk_space_succeeds_when_all_sizes_unknown_or_zero_required() {
        let sandbox = TempTestDir::new("unknown-sizes");
        let items = vec![
            planned("unknown1.bin", None, None),
            planned("zero.bin", Some(0), None),
        ];

        check_disk_space_with_available(&sandbox.0, &items, 0).unwrap();
        check_disk_space(&sandbox.0, &items).unwrap();
    }

    #[test]
    fn check_disk_space_queries_real_volume_for_small_and_impossibly_large_requirements() {
        let sandbox = TempTestDir::new("real-fs");

        let tiny = vec![planned("tiny.txt", Some(1), None)];
        check_disk_space(&sandbox.0, &tiny).unwrap();

        let impossible = vec![planned("impossible.bin", Some(u64::MAX), None)];
        let err = check_disk_space(&sandbox.0, &impossible).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InsufficientDiskSpace {
                required: u64::MAX,
                ..
            }
        ));
    }

    #[test]
    fn check_disk_space_with_available_one_byte_shortfall_and_u64_max_saturation() {
        let sandbox = TempTestDir::new("one-byte-shortfall");
        let signal = ShutdownSignal::new();

        let items = vec![
            planned("part1.bin", Some(1_024), None),
            planned("part2.bin", Some(512), None),
        ];
        let required = 1_536;

        // Exact fit passes
        check_disk_space_with_available(&sandbox.0, &items, required).unwrap();

        // Exact 1-byte shortfall fails with InsufficientDiskSpace formatted via format_size
        let available = required - 1;
        let err = check_disk_space_with_available(&sandbox.0, &items, available).unwrap_err();
        match &err {
            IaGetError::InsufficientDiskSpace {
                required: err_req,
                available: err_avail,
                path,
            } => {
                assert_eq!(*err_req, required);
                assert_eq!(*err_avail, available);
                assert_eq!(path, &sandbox.0);
            }
            other => panic!("expected InsufficientDiskSpace, got {other:?}"),
        }

        let formatted = err.to_string();
        assert!(formatted.contains(&crate::utils::format_size(required)));
        assert!(formatted.contains(&crate::utils::format_size(available)));
        assert!(formatted.contains(&sandbox.0.display().to_string()));

        // u64::MAX + 100 saturates without overflow
        let saturated_items = vec![
            planned("max.bin", Some(u64::MAX), None),
            planned("extra.bin", Some(100), None),
        ];
        assert_eq!(
            calculate_required_bytes(&sandbox.0, &saturated_items, &signal).unwrap(),
            u64::MAX
        );
        check_disk_space_with_available(&sandbox.0, &saturated_items, u64::MAX).unwrap();
        let sat_err = check_disk_space_with_available(&sandbox.0, &saturated_items, u64::MAX - 1)
            .unwrap_err();
        assert!(matches!(
            sat_err,
            IaGetError::InsufficientDiskSpace {
                required: u64::MAX,
                available,
                ..
            } if available == u64::MAX - 1
        ));
    }

    #[test]
    fn calculate_required_bytes_handles_corrupt_equal_length_truncated_and_nested_part_files() {
        let sandbox = TempTestDir::new("corrupt-and-nested");
        let signal = ShutdownSignal::new();

        // Corrupt equal-length final file (wrong MD5, no .part) -> full expected_size
        let expected_payload = b"expected-equal-len-bytes";
        let corrupt_equal_len = b"corrupt!-equal-len-bytes";
        assert_eq!(expected_payload.len(), corrupt_equal_len.len());
        let expected_md5 = format!("{:x}", md5::compute(expected_payload));
        fs::write(sandbox.join("equal_corrupt.bin"), corrupt_equal_len).unwrap();
        assert_eq!(
            calculate_required_bytes(
                &sandbox.0,
                &[planned(
                    "equal_corrupt.bin",
                    Some(expected_payload.len() as u64),
                    Some(&expected_md5),
                )],
                &signal,
            )
            .unwrap(),
            expected_payload.len() as u64
        );

        // Corrupt equal-length final file with valid .part -> expected_size - part_len
        fs::write(sandbox.join("equal_corrupt_part.bin"), corrupt_equal_len).unwrap();
        fs::write(sandbox.join("equal_corrupt_part.bin.part"), vec![0u8; 10]).unwrap();
        assert_eq!(
            calculate_required_bytes(
                &sandbox.0,
                &[planned(
                    "equal_corrupt_part.bin",
                    Some(expected_payload.len() as u64),
                    Some(&expected_md5),
                )],
                &signal,
            )
            .unwrap(),
            expected_payload.len() as u64 - 10
        );

        // Truncated final file with wrong MD5 -> full expected_size
        fs::write(sandbox.join("truncated.bin"), &expected_payload[..8]).unwrap();
        assert_eq!(
            calculate_required_bytes(
                &sandbox.0,
                &[planned(
                    "truncated.bin",
                    Some(expected_payload.len() as u64),
                    Some(&expected_md5),
                )],
                &signal,
            )
            .unwrap(),
            expected_payload.len() as u64
        );

        // Nested subdir .part files: normal, exact-length, and oversized
        fs::create_dir_all(sandbox.join("sub/dir")).unwrap();
        fs::write(sandbox.join("sub/dir/file.bin.part"), vec![0u8; 35]).unwrap();
        fs::write(sandbox.join("sub/dir/exact.bin.part"), vec![0u8; 50]).unwrap();
        fs::write(sandbox.join("sub/dir/oversized.bin.part"), vec![0u8; 90]).unwrap();

        let nested_items = vec![
            planned("sub/dir/file.bin", Some(100), None),
            planned("sub/dir/exact.bin", Some(50), None),
            planned("sub/dir/oversized.bin", Some(60), None),
        ];
        // (100 - 35) + 0 + 60 = 125
        assert_eq!(
            calculate_required_bytes(&sandbox.0, &nested_items, &signal).unwrap(),
            125
        );
    }

    #[test]
    fn check_disk_space_handles_unknown_only_and_mixed_known_unknown_entries() {
        let sandbox = TempTestDir::new("unknown-mixed");

        // All None entries never fail even with 0 available bytes or live volume query
        let all_unknown = vec![
            planned("unknown_a.bin", None, None),
            planned("sub/unknown_b.bin", None, None),
        ];
        check_disk_space_with_available(&sandbox.0, &all_unknown, 0).unwrap();
        check_disk_space(&sandbox.0, &all_unknown).unwrap();

        // Mixed known + None counts only known bytes
        let mixed = vec![
            planned("known_1.bin", Some(250), None),
            planned("unknown.bin", None, None),
            planned("known_2.bin", Some(750), None),
        ];
        check_disk_space_with_available(&sandbox.0, &mixed, 1_000).unwrap();
        let err = check_disk_space_with_available(&sandbox.0, &mixed, 999).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InsufficientDiskSpace {
                required: 1_000,
                available: 999,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_rejects_missing_and_non_directory_roots() {
        let sandbox = TempTestDir::new("invalid-roots");
        let signal = ShutdownSignal::new();
        let items = vec![planned("file.bin", Some(64), None)];

        let missing = sandbox.join("does-not-exist");
        assert!(matches!(
            calculate_required_bytes(&missing, &items, &signal),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            check_disk_space_with_available(&missing, &items, 1_024),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            check_disk_space(&missing, &items),
            Err(IaGetError::FileSystem(_))
        ));

        let regular_file = sandbox.join("not-a-dir.txt");
        fs::write(&regular_file, b"plain file").unwrap();
        assert!(matches!(
            calculate_required_bytes(&regular_file, &items, &signal),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            check_disk_space_with_available(&regular_file, &items, 1_024),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            check_disk_space(&regular_file, &items),
            Err(IaGetError::FileSystem(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn calculate_required_bytes_ignores_internal_symlinks_and_symlinked_parent_parts() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("internal-symlinks");
        let signal = ShutdownSignal::new();

        let real_dir = sandbox.join("real");
        fs::create_dir_all(&real_dir).unwrap();

        let payload = b"internal-verified-payload";
        let digest = format!("{:x}", md5::compute(payload));
        fs::write(real_dir.join("good.bin"), payload).unwrap();
        fs::write(real_dir.join("no_md5.bin"), b"present").unwrap();
        fs::write(real_dir.join("resuming.bin.part"), vec![0u8; 45]).unwrap();

        // Symlinked final file pointing inside root (with matching MD5)
        symlink(real_dir.join("good.bin"), sandbox.join("sym_good.bin")).unwrap();
        // Symlinked final file pointing inside root (with expected_md5: None)
        symlink(real_dir.join("no_md5.bin"), sandbox.join("sym_no_md5.bin")).unwrap();
        // Symlinked .part file pointing inside root
        symlink(
            real_dir.join("resuming.bin.part"),
            sandbox.join("sym_part.bin.part"),
        )
        .unwrap();
        // Symlinked parent directory pointing inside root
        symlink(&real_dir, sandbox.join("sym_dir")).unwrap();

        let items = vec![
            planned("sym_good.bin", Some(100), Some(&digest)),
            planned("sym_no_md5.bin", Some(100), None),
            planned("sym_part.bin", Some(100), None),
            planned("sym_dir/good.bin", Some(100), Some(&digest)),
            planned("sym_dir/no_md5.bin", Some(100), None),
            planned("sym_dir/resuming.bin", Some(100), None),
        ];

        assert_eq!(
            calculate_required_bytes(&sandbox.0, &items, &signal).unwrap(),
            600
        );
    }

    #[test]
    fn check_disk_space_helpers_propagate_pre_triggered_shutdown() {
        let sandbox = TempTestDir::new("shutdown-helpers");
        let payload = b"existing-file-needing-md5";
        let digest = format!("{:x}", md5::compute(payload));
        fs::write(sandbox.join("existing.bin"), payload).unwrap();

        let stopped = ShutdownSignal::new();
        stopped.trigger();

        let items = vec![planned(
            "existing.bin",
            Some(payload.len() as u64),
            Some(&digest),
        )];

        assert!(matches!(
            check_disk_space_with_available_and_signal(&sandbox.0, &items, 1_024, &stopped),
            Err(IaGetError::Interrupted)
        ));
        assert!(matches!(
            check_disk_space_with_signal(&sandbox.0, &items, &stopped),
            Err(IaGetError::Interrupted)
        ));
    }
}
