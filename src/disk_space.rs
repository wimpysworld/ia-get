//! Pre-download disk-space estimation and verification for destination volumes.

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
    let Ok(Some(_)) = downloader::inspect_contained_regular_file(canonical_root, rel_path) else {
        return Ok(false);
    };

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
    match downloader::inspect_contained_regular_file(canonical_root, &part_rel) {
        Ok(Some((_, meta))) => meta.len(),
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

fn ensure_sufficient_space(root: &Path, required: u64, available: u64) -> Result<()> {
    if required > available {
        return Err(IaGetError::InsufficientDiskSpace {
            required,
            available,
            path: root.to_path_buf(),
        });
    }
    Ok(())
}

/// Checks whether `available_bytes` is sufficient for `items` in `root`, honouring `signal`.
pub fn check_disk_space_with_available_and_signal(
    root: &Path,
    items: &[PlannedDownload],
    available_bytes: u64,
    signal: &ShutdownSignal,
) -> Result<()> {
    let canonical_root = downloader::resolve_destination_root(root)?;
    let required = calculate_required_bytes_in_canonical_root(&canonical_root, items, signal)?;
    if required == 0 {
        return Ok(());
    }

    ensure_sufficient_space(root, required, available_bytes)
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

    ensure_sufficient_space(root, required, available)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::{md5_hex, TempTestDir};

    fn planned(file_path: &str, expected_size: Option<u64>, md5: Option<&str>) -> PlannedDownload {
        PlannedDownload {
            url: format!("https://archive.org/download/item/{file_path}"),
            file_path: file_path.to_string(),
            expected_md5: md5.map(str::to_string),
            expected_size,
        }
    }

    fn required_bytes(
        root: &Path,
        items: &[PlannedDownload],
        signal: &ShutdownSignal,
    ) -> Result<u64> {
        let canonical = downloader::resolve_destination_root(root)?;
        calculate_required_bytes_in_canonical_root(&canonical, items, signal)
    }

    #[test]
    fn calculate_required_bytes_sums_missing_skips_unknown_and_zero_and_saturates() {
        let sandbox = TempTestDir::new("disk-space-basic");
        let signal = ShutdownSignal::new();

        let items = vec![
            planned("a.bin", Some(1_000), None),
            planned("unknown.bin", None, None),
            planned("empty.bin", Some(0), None),
            planned("nested/b.bin", Some(2_500), None),
        ];

        assert_eq!(
            required_bytes(sandbox.path(), &items, &signal).unwrap(),
            3_500
        );

        let overflowing = vec![
            planned("huge1.bin", Some(u64::MAX - 10), None),
            planned("huge2.bin", Some(100), None),
        ];
        assert_eq!(
            required_bytes(sandbox.path(), &overflowing, &signal).unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn calculate_required_bytes_accounts_for_verified_corrupt_and_part_files() {
        let sandbox = TempTestDir::new("disk-space-existing-and-part");
        let signal = ShutdownSignal::new();

        // Verified final file with matching MD5 -> 0 remaining
        let good_bytes = b"complete-file-content";
        let good_md5 = md5_hex(good_bytes);
        sandbox.write("verified.bin", good_bytes);

        // Final file without expected MD5 -> trusted as complete -> 0 remaining
        sandbox.write("no_md5.bin", b"present");

        // Corrupt final files (short, equal-length, truncated) with and without `.part`
        sandbox.write("corrupt_no_part.bin", b"bad");
        sandbox.write("corrupt_with_part.bin", b"bad");
        sandbox.write("corrupt_with_part.bin.part", vec![0u8; 40]);

        let expected_payload = b"expected-equal-len-bytes";
        let corrupt_equal_len = b"corrupt!-equal-len-bytes";
        assert_eq!(expected_payload.len(), corrupt_equal_len.len());
        let expected_md5 = md5_hex(expected_payload);
        let payload_len = expected_payload.len() as u64;
        sandbox.write("equal_corrupt.bin", corrupt_equal_len);
        sandbox.write("equal_corrupt_part.bin", corrupt_equal_len);
        sandbox.write("equal_corrupt_part.bin.part", vec![0u8; 10]);
        sandbox.write("truncated.bin", &expected_payload[..8]);

        // Missing final files with partial, exact-size, and oversized `.part` (top-level & nested)
        sandbox.write("resuming.bin.part", vec![0u8; 30]);
        sandbox.write("complete_part.bin.part", vec![0u8; 50]);
        sandbox.write("oversized_part.bin.part", vec![0u8; 120]);
        sandbox.write("sub/dir/file.bin.part", vec![0u8; 35]);
        sandbox.write("sub/dir/exact.bin.part", vec![0u8; 50]);
        sandbox.write("sub/dir/oversized.bin.part", vec![0u8; 90]);

        let cases = [
            (
                planned(
                    "verified.bin",
                    Some(good_bytes.len() as u64),
                    Some(&good_md5),
                ),
                0,
            ),
            (planned("no_md5.bin", Some(500), None), 0),
            (
                planned(
                    "corrupt_no_part.bin",
                    Some(200),
                    Some("00000000000000000000000000000000"),
                ),
                200,
            ),
            (
                planned(
                    "corrupt_with_part.bin",
                    Some(100),
                    Some("11111111111111111111111111111111"),
                ),
                60,
            ),
            (
                planned("equal_corrupt.bin", Some(payload_len), Some(&expected_md5)),
                payload_len,
            ),
            (
                planned(
                    "equal_corrupt_part.bin",
                    Some(payload_len),
                    Some(&expected_md5),
                ),
                payload_len - 10,
            ),
            (
                planned("truncated.bin", Some(payload_len), Some(&expected_md5)),
                payload_len,
            ),
            (planned("resuming.bin", Some(100), None), 70),
            (planned("complete_part.bin", Some(50), None), 0),
            (planned("oversized_part.bin", Some(80), None), 80),
            (planned("sub/dir/file.bin", Some(100), None), 65),
            (planned("sub/dir/exact.bin", Some(50), None), 0),
            (planned("sub/dir/oversized.bin", Some(60), None), 60),
        ];

        for (item, expected) in &cases {
            assert_eq!(
                required_bytes(sandbox.path(), std::slice::from_ref(item), &signal).unwrap(),
                *expected,
                "case {}",
                item.file_path
            );
        }

        let all_items: Vec<_> = cases.iter().map(|(item, _)| item.clone()).collect();
        let total_expected: u64 = cases.iter().map(|(_, expected)| *expected).sum();
        assert_eq!(
            required_bytes(sandbox.path(), &all_items, &signal).unwrap(),
            total_expected
        );
    }

    #[test]
    fn calculate_required_bytes_ignores_non_regular_files() {
        let sandbox = TempTestDir::new("disk-space-non-regular");
        let signal = ShutdownSignal::new();

        // Directory at final path and directory at .part path must not reduce required bytes
        fs::create_dir_all(sandbox.join("as_dir.bin")).unwrap();
        fs::create_dir_all(sandbox.join("part_dir.bin.part")).unwrap();

        let items = vec![
            planned("as_dir.bin", Some(128), None),
            planned("part_dir.bin", Some(256), None),
        ];

        assert_eq!(
            required_bytes(sandbox.path(), &items, &signal).unwrap(),
            384
        );
    }

    #[cfg(unix)]
    #[test]
    fn calculate_required_bytes_ignores_external_and_internal_symlinks() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("disk-space-symlinks");
        let outside = TempTestDir::new("disk-space-outside");
        let signal = ShutdownSignal::new();

        let ext_data = b"external-payload";
        let ext_digest = md5_hex(ext_data);
        let ext_target = outside.write("target.bin", ext_data);
        let ext_partial = outside.write("partial.bin", vec![0u8; 90]);

        symlink(&ext_target, sandbox.join("linked_final.bin")).unwrap();
        symlink(&ext_partial, sandbox.join("linked_part.bin.part")).unwrap();
        symlink(outside.path(), sandbox.join("linked_dir")).unwrap();

        let int_data = b"internal-verified-payload";
        let int_digest = md5_hex(int_data);
        let int_good = sandbox.write("real/good.bin", int_data);
        let int_no_md5 = sandbox.write("real/no_md5.bin", b"present");
        let int_part = sandbox.write("real/resuming.bin.part", vec![0u8; 45]);

        symlink(&int_good, sandbox.join("sym_good.bin")).unwrap();
        symlink(&int_no_md5, sandbox.join("sym_no_md5.bin")).unwrap();
        symlink(&int_part, sandbox.join("sym_part.bin.part")).unwrap();
        symlink(sandbox.join("real"), sandbox.join("sym_dir")).unwrap();

        let items = vec![
            planned("linked_final.bin", Some(100), Some(&ext_digest)),
            planned("linked_part.bin", Some(100), None),
            planned("linked_dir/target.bin", Some(100), Some(&ext_digest)),
            planned("sym_good.bin", Some(100), Some(&int_digest)),
            planned("sym_no_md5.bin", Some(100), None),
            planned("sym_part.bin", Some(100), None),
            planned("sym_dir/good.bin", Some(100), Some(&int_digest)),
            planned("sym_dir/no_md5.bin", Some(100), None),
            planned("sym_dir/resuming.bin", Some(100), None),
        ];

        assert_eq!(
            required_bytes(sandbox.path(), &items, &signal).unwrap(),
            900
        );
    }

    #[test]
    fn disk_space_operations_propagate_pre_triggered_shutdown() {
        let sandbox = TempTestDir::new("disk-space-interrupt");
        let payload = b"existing-file-needing-md5";
        let digest = md5_hex(payload);
        sandbox.write("existing.bin", payload);

        let stopped = ShutdownSignal::new();
        stopped.trigger();

        let items = vec![
            planned("existing.bin", Some(payload.len() as u64), Some(&digest)),
            planned("missing.bin", Some(100), None),
        ];

        assert!(matches!(
            required_bytes(sandbox.path(), &items, &stopped),
            Err(IaGetError::Interrupted)
        ));
        assert!(matches!(
            check_disk_space_with_available_and_signal(sandbox.path(), &items, 1_024, &stopped),
            Err(IaGetError::Interrupted)
        ));
        assert!(matches!(
            check_disk_space_with_signal(sandbox.path(), &items, &stopped),
            Err(IaGetError::Interrupted)
        ));
    }

    #[test]
    fn check_disk_space_with_available_enforces_boundaries_formatting_and_saturation() {
        let sandbox = TempTestDir::new("disk-space-boundaries");
        let signal = ShutdownSignal::new();

        // Single-file exact fit and 1 MiB shortfall formatted via format_size
        let big = vec![planned("archive.iso", Some(2 * 1024 * 1024), None)];
        check_disk_space_with_available_and_signal(sandbox.path(), &big, 2 * 1024 * 1024, &signal)
            .unwrap();
        let err =
            check_disk_space_with_available_and_signal(sandbox.path(), &big, 1024 * 1024, &signal)
                .unwrap_err();
        assert!(matches!(
            &err,
            IaGetError::InsufficientDiskSpace {
                required,
                available,
                path,
            } if *required == 2 * 1024 * 1024 && *available == 1024 * 1024 && path == sandbox.path()
        ));
        let msg = err.to_string();
        assert!(msg.contains("Insufficient disk space"));
        assert!(msg.contains("2.00MB"));
        assert!(msg.contains("1.00MB"));
        assert!(msg.contains(&sandbox.path().display().to_string()));

        // Multi-file exact fit and exact 1-byte shortfall
        let multi = vec![
            planned("part1.bin", Some(1_024), None),
            planned("part2.bin", Some(512), None),
        ];
        let required = 1_536;
        let available = required - 1;
        check_disk_space_with_available_and_signal(sandbox.path(), &multi, required, &signal)
            .unwrap();
        let err =
            check_disk_space_with_available_and_signal(sandbox.path(), &multi, available, &signal)
                .unwrap_err();
        assert!(matches!(
            &err,
            IaGetError::InsufficientDiskSpace {
                required: err_req,
                available: err_avail,
                path,
            } if *err_req == required && *err_avail == available && path == sandbox.path()
        ));
        let formatted = err.to_string();
        assert!(formatted.contains(&crate::utils::format_size(required)));
        assert!(formatted.contains(&crate::utils::format_size(available)));
        assert!(formatted.contains(&sandbox.path().display().to_string()));

        // u64::MAX + 100 saturates without overflow
        let saturated = vec![
            planned("max.bin", Some(u64::MAX), None),
            planned("extra.bin", Some(100), None),
        ];
        assert_eq!(
            required_bytes(sandbox.path(), &saturated, &signal).unwrap(),
            u64::MAX
        );
        check_disk_space_with_available_and_signal(sandbox.path(), &saturated, u64::MAX, &signal)
            .unwrap();
        let sat_err = check_disk_space_with_available_and_signal(
            sandbox.path(),
            &saturated,
            u64::MAX - 1,
            &signal,
        )
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
    fn check_disk_space_handles_unknown_zero_and_mixed_entries() {
        let sandbox = TempTestDir::new("disk-space-unknown-mixed");
        let signal = ShutdownSignal::new();

        for items in [
            vec![
                planned("unknown1.bin", None, None),
                planned("zero.bin", Some(0), None),
            ],
            vec![
                planned("unknown_a.bin", None, None),
                planned("sub/unknown_b.bin", None, None),
            ],
        ] {
            check_disk_space_with_available_and_signal(sandbox.path(), &items, 0, &signal).unwrap();
            check_disk_space_with_signal(sandbox.path(), &items, &signal).unwrap();
        }

        // Mixed known + None counts only known bytes
        let mixed = vec![
            planned("known_1.bin", Some(250), None),
            planned("unknown.bin", None, None),
            planned("known_2.bin", Some(750), None),
        ];
        check_disk_space_with_available_and_signal(sandbox.path(), &mixed, 1_000, &signal).unwrap();
        let err = check_disk_space_with_available_and_signal(sandbox.path(), &mixed, 999, &signal)
            .unwrap_err();
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
    fn check_disk_space_queries_real_volume_for_small_and_impossibly_large_requirements() {
        let sandbox = TempTestDir::new("disk-space-real-fs");
        let signal = ShutdownSignal::new();

        let tiny = vec![planned("tiny.txt", Some(1), None)];
        check_disk_space_with_signal(sandbox.path(), &tiny, &signal).unwrap();

        let impossible = vec![planned("impossible.bin", Some(u64::MAX), None)];
        let err = check_disk_space_with_signal(sandbox.path(), &impossible, &signal).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InsufficientDiskSpace {
                required: u64::MAX,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_rejects_missing_and_non_directory_roots() {
        let sandbox = TempTestDir::new("disk-space-invalid-roots");
        let signal = ShutdownSignal::new();
        let items = vec![planned("file.bin", Some(64), None)];

        let missing = sandbox.join("does-not-exist");
        let regular_file = sandbox.write("not-a-dir.txt", b"plain file");

        for invalid in [&missing, &regular_file] {
            assert!(matches!(
                required_bytes(invalid, &items, &signal),
                Err(IaGetError::FileSystem(_))
            ));
            assert!(matches!(
                check_disk_space_with_available_and_signal(invalid, &items, 1_024, &signal),
                Err(IaGetError::FileSystem(_))
            ));
            assert!(matches!(
                check_disk_space_with_signal(invalid, &items, &signal),
                Err(IaGetError::FileSystem(_))
            ));
        }
    }
}
