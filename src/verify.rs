//! Strictly read-only local file verification against archive.org `_files.xml` metadata.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use colored::*;

use crate::archive_metadata::XmlFile;
use crate::downloader::{self, ShutdownSignal};
use crate::error::IaGetError;
use crate::utils::{self, VerboseLogger};
use crate::Result;

/// A single issue detected while verifying a local file against archive metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationIssue {
    /// Expected file does not exist on disk.
    Missing,
    /// A leftover `.part` staging file exists on disk.
    LeftoverPart {
        /// Sanitised relative path to the leftover `.part` file.
        part_path: String,
        /// Byte size of the leftover `.part` file.
        size: u64,
    },
    /// Local file size differs from the expected metadata size.
    SizeMismatch {
        /// Expected byte length from `_files.xml`.
        expected: u64,
        /// Actual byte length found on disk.
        actual: u64,
    },
    /// Local file MD5 digest differs from the expected metadata MD5.
    Md5Mismatch {
        /// Expected MD5 digest from `_files.xml`.
        expected: String,
        /// Actual computed MD5 digest.
        actual: String,
    },
    /// Target path is unsafe (symlink, non-regular file, path escape) or unreadable.
    UnreadableTarget {
        /// Reason the target could not be safely read or inspected.
        reason: String,
    },
}

impl fmt::Display for VerificationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing"),
            Self::LeftoverPart { part_path, size } => {
                write!(f, "leftover partial file '{part_path}' ({size} bytes)")
            }
            Self::SizeMismatch { expected, actual } => {
                write!(
                    f,
                    "size mismatch (expected {expected} bytes, actual {actual} bytes)"
                )
            }
            Self::Md5Mismatch { expected, actual } => {
                write!(f, "MD5 mismatch (expected {expected}, actual {actual})")
            }
            Self::UnreadableTarget { reason } => write!(f, "unreadable target: {reason}"),
        }
    }
}

/// Details of a single file that failed one or more verification checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedVerification {
    /// Sanitised local path of the file that failed verification.
    pub file_path: String,
    /// Ordered list of verification issues detected for this file.
    pub issues: Vec<VerificationIssue>,
}

/// Outcome summary of a verification run.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct VerificationSummary {
    /// Number of files that passed all verification checks.
    pub ok: usize,
    /// Files that failed one or more verification checks.
    pub failed: Vec<FailedVerification>,
    /// Whether verification stopped early due to a user interrupt signal.
    pub interrupted: bool,
}

/// Validates that `path` is an existing, accessible directory without creating or modifying it,
/// returning its canonical path.
pub fn validate_check_dir(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(
            "Check directory path must not be empty".to_string(),
        ));
    }

    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let canonical = downloader::resolve_destination_root(path)?;
            // Confirm search access into the directory itself without modifying anything.
            fs::metadata(canonical.join(".")).map_err(|err| {
                IaGetError::FileSystem(format!(
                    "Failed to access check directory '{}': {}",
                    path.display(),
                    err
                ))
            })?;
            Ok(canonical)
        }
        Ok(_) => Err(IaGetError::FileSystem(format!(
            "Check path '{}' exists and is not a directory",
            path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(IaGetError::FileSystem(format!(
                    "Check path '{}' is a broken symlink",
                    path.display()
                )));
            }

            Err(IaGetError::FileSystem(format!(
                "Check directory '{}' does not exist",
                path.display()
            )))
        }
        Err(err) => Err(IaGetError::FileSystem(format!(
            "Failed to access check directory '{}': {}",
            path.display(),
            err
        ))),
    }
}

fn error_reason(err: IaGetError) -> String {
    match err {
        IaGetError::FileSystem(msg) => msg,
        other => other.to_string(),
    }
}

fn push_unique_issue(issues: &mut Vec<VerificationIssue>, issue: VerificationIssue) {
    if !issues.contains(&issue) {
        issues.push(issue);
    }
}

fn format_issues(issues: &[VerificationIssue]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn print_interrupted_notice() {
    println!("\n{} Verification interrupted by user.", "✘".red().bold());
}

/// Formats a plain-text summary of a verification run, including any failed files.
pub fn format_verification_summary(summary: &VerificationSummary) -> String {
    let mut text = format!(
        "Verification summary: {} ok, {} failed",
        summary.ok,
        summary.failed.len()
    );
    if summary.interrupted {
        text.push_str(", interrupted");
    }

    if !summary.failed.is_empty() {
        text.push_str("\nFailed files:");
        for failed in &summary.failed {
            text.push_str(&format!(
                "\n  - {}: {}",
                failed.file_path,
                format_issues(&failed.issues)
            ));
        }
    }

    text
}

/// Prints a coloured end-of-verification summary to stdout.
fn print_verification_summary(summary: &VerificationSummary) {
    let status_icon = if summary.interrupted || !summary.failed.is_empty() {
        "✘".red().bold()
    } else {
        "✔".green().bold()
    };

    let failed_count = if summary.failed.is_empty() {
        summary.failed.len().to_string().bold()
    } else {
        summary.failed.len().to_string().red().bold()
    };

    let interrupted_suffix = if summary.interrupted {
        format!(", {}", "interrupted".yellow().bold())
    } else {
        String::new()
    };

    println!(
        "\n{} {} {} ok, {} failed{}",
        status_icon,
        "Verification summary:".bold(),
        summary.ok.to_string().green().bold(),
        failed_count,
        interrupted_suffix
    );

    if !summary.failed.is_empty() {
        println!("{}", "Failed files:".red().bold());
        for failed in &summary.failed {
            println!(
                "  {} {}: {}",
                "✘".red().bold(),
                failed.file_path.bold(),
                format_issues(&failed.issues)
            );
        }
    }
}

/// Verifies local files inside `root` against `files` without creating, modifying, or deleting
/// any files or directories.
pub fn verify_files(
    root: &Path,
    files: &[XmlFile],
    signal: &ShutdownSignal,
) -> Result<VerificationSummary> {
    verify_files_with_logger(root, files, signal, &VerboseLogger::disabled())
}

/// Verifies local files inside `root` against `files` without creating, modifying, or deleting
/// any files or directories, emitting diagnostic logs to `logger`.
pub fn verify_files_with_logger(
    root: &Path,
    files: &[XmlFile],
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<VerificationSummary> {
    let canonical_root = validate_check_dir(root)?;
    let mut summary = VerificationSummary::default();

    for file in files {
        if !signal.is_running() {
            summary.interrupted = true;
            logger.log("verify", "interrupted by user");
            print_interrupted_notice();
            break;
        }

        let (sanitized_name, _) = utils::sanitize_filename(&file.name);
        let mut issues = Vec::new();

        // Check for leftover `.part` file regardless of whether the final file exists.
        let part_rel = downloader::part_file_path(&sanitized_name);
        match downloader::validate_contained_path(&canonical_root, &part_rel, true) {
            Ok(part_full) => match fs::symlink_metadata(&part_full) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        push_unique_issue(
                            &mut issues,
                            VerificationIssue::UnreadableTarget {
                                reason: format!(
                                    "Refusing symlinked partial file '{}'",
                                    part_full.display()
                                ),
                            },
                        );
                    } else if !meta.file_type().is_file() {
                        push_unique_issue(
                            &mut issues,
                            VerificationIssue::UnreadableTarget {
                                reason: format!(
                                    "Refusing non-regular partial file '{}'",
                                    part_full.display()
                                ),
                            },
                        );
                    } else {
                        issues.push(VerificationIssue::LeftoverPart {
                            part_path: part_rel.to_string_lossy().into_owned(),
                            size: meta.len(),
                        });
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    push_unique_issue(
                        &mut issues,
                        VerificationIssue::UnreadableTarget {
                            reason: err.to_string(),
                        },
                    );
                }
            },
            Err(err) => {
                push_unique_issue(
                    &mut issues,
                    VerificationIssue::UnreadableTarget {
                        reason: error_reason(err),
                    },
                );
            }
        }

        // Check final target file presence, safety, readability, byte length, and MD5 digest.
        let rel_path = Path::new(&sanitized_name);
        match downloader::validate_contained_path(&canonical_root, rel_path, true) {
            Ok(target_full) => match fs::symlink_metadata(&target_full) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        push_unique_issue(
                            &mut issues,
                            VerificationIssue::UnreadableTarget {
                                reason: format!(
                                    "Refusing symlink target '{}'",
                                    target_full.display()
                                ),
                            },
                        );
                    } else if !meta.file_type().is_file() {
                        push_unique_issue(
                            &mut issues,
                            VerificationIssue::UnreadableTarget {
                                reason: format!(
                                    "Refusing non-regular file target '{}'",
                                    target_full.display()
                                ),
                            },
                        );
                    } else {
                        match downloader::open_regular_file_for_read(&canonical_root, rel_path) {
                            Ok(opened) => match opened.metadata() {
                                Ok(opened_meta) => {
                                    let actual_size = opened_meta.len();
                                    drop(opened);

                                    let size_mismatch = match file.size {
                                        Some(expected) if actual_size != expected => {
                                            issues.push(VerificationIssue::SizeMismatch {
                                                expected,
                                                actual: actual_size,
                                            });
                                            true
                                        }
                                        _ => false,
                                    };

                                    if !size_mismatch {
                                        if let Some(expected_md5) = file.md5.as_deref() {
                                            if !signal.is_running() {
                                                summary.interrupted = true;
                                                logger.log(
                                                    "verify",
                                                    format_args!(
                                                        "interrupted while verifying '{sanitized_name}'"
                                                    ),
                                                );
                                                print_interrupted_notice();
                                                break;
                                            }
                                            match downloader::calculate_md5(
                                                &canonical_root,
                                                rel_path,
                                                signal.running_flag(),
                                            ) {
                                                Ok(actual_md5) => {
                                                    if !actual_md5
                                                        .eq_ignore_ascii_case(expected_md5)
                                                    {
                                                        issues.push(
                                                            VerificationIssue::Md5Mismatch {
                                                                expected: expected_md5.to_string(),
                                                                actual: actual_md5,
                                                            },
                                                        );
                                                    }
                                                }
                                                Err(err) => {
                                                    if !signal.is_running()
                                                        || err
                                                            .to_string()
                                                            .contains("interrupted by signal")
                                                    {
                                                        summary.interrupted = true;
                                                        logger.log(
                                                            "verify",
                                                            format_args!(
                                                                "interrupted while verifying '{sanitized_name}'"
                                                            ),
                                                        );
                                                        print_interrupted_notice();
                                                        break;
                                                    }
                                                    push_unique_issue(
                                                        &mut issues,
                                                        VerificationIssue::UnreadableTarget {
                                                            reason: error_reason(err),
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    push_unique_issue(
                                        &mut issues,
                                        VerificationIssue::UnreadableTarget {
                                            reason: err.to_string(),
                                        },
                                    );
                                }
                            },
                            Err(err) => {
                                push_unique_issue(
                                    &mut issues,
                                    VerificationIssue::UnreadableTarget {
                                        reason: error_reason(err),
                                    },
                                );
                            }
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    issues.push(VerificationIssue::Missing);
                }
                Err(err) => {
                    push_unique_issue(
                        &mut issues,
                        VerificationIssue::UnreadableTarget {
                            reason: err.to_string(),
                        },
                    );
                }
            },
            Err(err) => {
                push_unique_issue(
                    &mut issues,
                    VerificationIssue::UnreadableTarget {
                        reason: error_reason(err),
                    },
                );
            }
        }

        if issues.is_empty() {
            summary.ok += 1;
            logger.log("verify", format_args!("ok '{sanitized_name}'"));
        } else {
            let formatted_issues = format_issues(&issues);
            logger.log(
                "verify",
                format_args!("failed '{sanitized_name}': {formatted_issues}"),
            );
            println!(
                "{} {}: {}",
                "✘".red().bold(),
                sanitized_name.bold(),
                formatted_issues
            );
            summary.failed.push(FailedVerification {
                file_path: sanitized_name,
                issues,
            });
        }
    }

    logger.log(
        "summary",
        format_args!(
            "ok={} failed={} interrupted={}",
            summary.ok,
            summary.failed.len(),
            summary.interrupted
        ),
    );
    print_verification_summary(&summary);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{md5_hex, snapshot_dir, xml_file, CapturedLogs, TempTestDir};

    #[test]
    fn validate_check_dir_and_verify_files_reject_invalid_dirs_without_creating() {
        let sandbox = TempTestDir::new("verify-validate");
        let files = vec![xml_file("a.bin", Some(1), None)];
        let signal = ShutdownSignal::new();

        let missing = sandbox.join("does-not-exist/nested");
        let file_path = sandbox.write("regular.txt", b"hello");

        for invalid in [Path::new(""), missing.as_path(), file_path.as_path()] {
            assert!(
                matches!(validate_check_dir(invalid), Err(IaGetError::FileSystem(_))),
                "validate_check_dir must reject {invalid:?}"
            );
            assert!(
                matches!(
                    verify_files(invalid, &files, &signal),
                    Err(IaGetError::FileSystem(_))
                ),
                "verify_files must reject {invalid:?}"
            );
        }

        assert!(!missing.exists());
        assert!(!sandbox.join("does-not-exist").exists());
        assert_eq!(fs::read(&file_path).unwrap(), b"hello");

        let canonical = validate_check_dir(sandbox.path()).unwrap();
        assert_eq!(canonical, fs::canonicalize(sandbox.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn validate_check_dir_rejects_broken_symlink() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("verify-broken-symlink");
        let missing_target = sandbox.join("missing-target");
        let broken = sandbox.join("broken-link");
        symlink(&missing_target, &broken).unwrap();

        let err = validate_check_dir(&broken).unwrap_err();
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(err.to_string().contains("broken symlink"));
        assert!(!missing_target.exists());
    }

    #[test]
    fn verify_files_clean_pass_variants_and_read_only() {
        let sandbox = TempTestDir::new("verify-clean-pass");

        let top_bytes = b"top-level payload";
        let top_md5 = md5_hex(top_bytes);
        sandbox.write("readme.txt", top_bytes);

        let nested_bytes = b"hello archive verification";
        let nested_md5_upper = format!("{:X}", md5::compute(nested_bytes));
        sandbox.write("docs/sub/manual.pdf", nested_bytes);

        let empty_md5 = md5_hex(b"");
        assert_eq!(empty_md5, "d41d8cd98f00b204e9800998ecf8427e");
        sandbox.write("empty.dat", b"");

        let size_only_bytes = b"only size checked";
        sandbox.write("size_only.bin", size_only_bytes);

        let md5_only_bytes = b"only md5 checked";
        let md5_only_digest = md5_hex(md5_only_bytes);
        sandbox.write("md5_only.bin", md5_only_bytes);

        sandbox.write("no-meta.txt", b"anything");

        let sanitized_bytes = b"episode video bytes";
        let sanitized_md5 = md5_hex(sanitized_bytes);
        sandbox.write("Season 1/Episode_.mp4", sanitized_bytes);

        let files = vec![
            xml_file("readme.txt", Some(top_bytes.len() as u64), Some(&top_md5)),
            xml_file(
                "docs/sub/manual.pdf",
                Some(nested_bytes.len() as u64),
                Some(&nested_md5_upper),
            ),
            xml_file("empty.dat", Some(0), Some(&empty_md5)),
            xml_file("size_only.bin", Some(size_only_bytes.len() as u64), None),
            xml_file("md5_only.bin", None, Some(&md5_only_digest)),
            xml_file("no-meta.txt", None, None),
            xml_file(
                "Season 1/Episode?.mp4",
                Some(sanitized_bytes.len() as u64),
                Some(&sanitized_md5),
            ),
        ];

        let before = snapshot_dir(sandbox.path());
        let signal = ShutdownSignal::new();
        let summary = verify_files(sandbox.path(), &files, &signal).unwrap();
        let after = snapshot_dir(sandbox.path());

        assert_eq!(summary.ok, 7);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_eq!(
            format_verification_summary(&summary),
            "Verification summary: 7 ok, 0 failed"
        );
        assert_eq!(before, after, "verify_files must be strictly read-only");
    }

    #[test]
    fn verify_files_failure_matrix_is_read_only() {
        let sandbox = TempTestDir::new("verify-failures");

        let ok_bytes = b"exact-bytes";
        let ok_md5 = md5_hex(ok_bytes);
        sandbox.write("with_part.bin", ok_bytes);
        sandbox.write("with_part.bin.part", b"partial");

        sandbox.write("wrong_size.bin", b"short");
        sandbox.write("oversized.bin", b"123456789012");

        let bad_bytes = b"wrong-hash!";
        sandbox.write("wrong_md5.bin", bad_bytes);

        let corrupt_bytes = b"corrupt-payload";
        sandbox.write("md5_only_bad.bin", corrupt_bytes);

        sandbox.write("orphan.bin.part", b"123456789");
        fs::create_dir_all(sandbox.join("dir_as_file.bin")).unwrap();

        let valid_bytes = b"valid";
        let valid_md5 = md5_hex(valid_bytes);
        sandbox.write("dir_part.bin", valid_bytes);
        fs::create_dir_all(sandbox.join("dir_part.bin.part")).unwrap();

        let files = vec![
            xml_file(
                "missing.bin",
                Some(10),
                Some("00000000000000000000000000000000"),
            ),
            xml_file("absent_dir/sub/missing.bin", Some(16), None),
            xml_file("with_part.bin", Some(ok_bytes.len() as u64), Some(&ok_md5)),
            xml_file(
                "wrong_size.bin",
                Some(100),
                Some("11111111111111111111111111111111"),
            ),
            xml_file(
                "oversized.bin",
                Some(4),
                Some("00000000000000000000000000000000"),
            ),
            xml_file(
                "wrong_md5.bin",
                Some(bad_bytes.len() as u64),
                Some("22222222222222222222222222222222"),
            ),
            xml_file(
                "md5_only_bad.bin",
                None,
                Some("ffffffffffffffffffffffffffffffff"),
            ),
            xml_file("orphan.bin", Some(20), None),
            xml_file("dir_as_file.bin", Some(5), None),
            xml_file(
                "dir_part.bin",
                Some(valid_bytes.len() as u64),
                Some(&valid_md5),
            ),
        ];

        let before = snapshot_dir(sandbox.path());
        let signal = ShutdownSignal::new();
        let summary = verify_files(sandbox.path(), &files, &signal).unwrap();
        let after = snapshot_dir(sandbox.path());

        assert_eq!(summary.ok, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 10);
        assert!(
            !sandbox.join("absent_dir").exists(),
            "missing nested subdirectory must not be created"
        );
        assert_eq!(
            before, after,
            "verify_files must leave directory completely untouched on failures"
        );

        let expected_exact: [(&str, Vec<VerificationIssue>); 8] = [
            ("missing.bin", vec![VerificationIssue::Missing]),
            (
                "absent_dir/sub/missing.bin",
                vec![VerificationIssue::Missing],
            ),
            (
                "with_part.bin",
                vec![VerificationIssue::LeftoverPart {
                    part_path: "with_part.bin.part".to_string(),
                    size: 7,
                }],
            ),
            (
                "wrong_size.bin",
                vec![VerificationIssue::SizeMismatch {
                    expected: 100,
                    actual: 5,
                }],
            ),
            (
                "oversized.bin",
                vec![VerificationIssue::SizeMismatch {
                    expected: 4,
                    actual: 12,
                }],
            ),
            (
                "wrong_md5.bin",
                vec![VerificationIssue::Md5Mismatch {
                    expected: "22222222222222222222222222222222".to_string(),
                    actual: md5_hex(bad_bytes),
                }],
            ),
            (
                "md5_only_bad.bin",
                vec![VerificationIssue::Md5Mismatch {
                    expected: "ffffffffffffffffffffffffffffffff".to_string(),
                    actual: md5_hex(corrupt_bytes),
                }],
            ),
            (
                "orphan.bin",
                vec![
                    VerificationIssue::LeftoverPart {
                        part_path: "orphan.bin.part".to_string(),
                        size: 9,
                    },
                    VerificationIssue::Missing,
                ],
            ),
        ];
        for (idx, (path, issues)) in expected_exact.iter().enumerate() {
            assert_eq!(summary.failed[idx].file_path, *path);
            assert_eq!(summary.failed[idx].issues, *issues, "case {path}");
        }

        assert_eq!(summary.failed[8].file_path, "dir_as_file.bin");
        assert!(matches!(
            &summary.failed[8].issues[..],
            [VerificationIssue::UnreadableTarget { reason }]
                if reason.contains("non-regular file target")
        ));

        assert_eq!(summary.failed[9].file_path, "dir_part.bin");
        assert!(matches!(
            &summary.failed[9].issues[..],
            [VerificationIssue::UnreadableTarget { reason }]
                if reason.contains("non-regular") && reason.contains("dir_part.bin.part")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn verify_files_rejects_symlinks_for_targets_parts_and_parents() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("verify-symlinks");
        let outside = TempTestDir::new("verify-outside");

        let payload = b"outside-data";
        let payload_md5 = md5_hex(payload);
        let outside_file = outside.write("victim.bin", payload);
        outside.write("secret.bin", b"secret");

        symlink(&outside_file, sandbox.join("live_target.bin")).unwrap();
        symlink(
            outside.join("does-not-exist.bin"),
            sandbox.join("dangling_target.bin"),
        )
        .unwrap();

        sandbox.write("part_live.bin", payload);
        symlink(&outside_file, sandbox.join("part_live.bin.part")).unwrap();
        symlink(
            outside.join("missing-part.bin"),
            sandbox.join("part_dangling.bin.part"),
        )
        .unwrap();

        symlink(outside.path(), sandbox.join("linked")).unwrap();

        let files = vec![
            xml_file(
                "live_target.bin",
                Some(payload.len() as u64),
                Some(&payload_md5),
            ),
            xml_file("dangling_target.bin", Some(12), None),
            xml_file(
                "part_live.bin",
                Some(payload.len() as u64),
                Some(&payload_md5),
            ),
            xml_file("part_dangling.bin", Some(12), None),
            xml_file("linked/secret.bin", Some(6), None),
        ];

        let before = snapshot_dir(sandbox.path());
        let signal = ShutdownSignal::new();
        let summary = verify_files(sandbox.path(), &files, &signal).unwrap();
        let after = snapshot_dir(sandbox.path());

        assert_eq!(summary.ok, 0);
        assert_eq!(summary.failed.len(), 5);
        assert_eq!(before, after);

        let single_unreadable: [(usize, &str, &[&str]); 4] = [
            (0, "live_target.bin", &["symlink target"]),
            (1, "dangling_target.bin", &["symlink target"]),
            (2, "part_live.bin", &["symlink", "part_live.bin.part"]),
            (4, "linked/secret.bin", &[]),
        ];
        for (idx, path, expected_needles) in single_unreadable {
            assert_eq!(summary.failed[idx].file_path, path);
            assert_eq!(
                summary.failed[idx].issues.len(),
                1,
                "expected single deduplicated issue for {path}"
            );
            match &summary.failed[idx].issues[0] {
                VerificationIssue::UnreadableTarget { reason } => {
                    for needle in expected_needles {
                        assert!(
                            reason.contains(needle),
                            "{path}: expected {needle:?} in {reason:?}"
                        );
                    }
                }
                other => panic!("{path}: expected UnreadableTarget, got {other:?}"),
            }
        }

        assert_eq!(summary.failed[3].file_path, "part_dangling.bin");
        assert_eq!(summary.failed[3].issues.len(), 2);
        assert!(matches!(
            &summary.failed[3].issues[0],
            VerificationIssue::UnreadableTarget { reason }
                if reason.contains("symlink") && reason.contains("part_dangling.bin.part")
        ));
        assert_eq!(summary.failed[3].issues[1], VerificationIssue::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn verify_files_reports_unreadable_file_permissions_as_unreadable_target() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        struct RestoreModeGuard {
            path: PathBuf,
            mode: u32,
        }

        impl Drop for RestoreModeGuard {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
            }
        }

        let sandbox = TempTestDir::new("verify-unreadable");
        let file_path = sandbox.write("secret.bin", b"cannot read this");

        // Guard drops before `sandbox`, guaranteeing permissions are restored prior to cleanup.
        let restore_guard = RestoreModeGuard {
            path: file_path.clone(),
            mode: 0o644,
        };
        fs::set_permissions(&file_path, fs::Permissions::from_mode(0o000)).unwrap();

        // Skip when running as root (UID 0) or with CAP_DAC_OVERRIDE where 0o000 remains readable.
        let meta = fs::metadata(&file_path).unwrap();
        if meta.uid() == 0 || fs::File::open(&file_path).is_ok() {
            drop(restore_guard);
            return;
        }

        let files = vec![xml_file("secret.bin", Some(16), None)];
        let signal = ShutdownSignal::new();
        let summary = verify_files(sandbox.path(), &files, &signal).unwrap();

        drop(restore_guard);

        assert_eq!(summary.ok, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "secret.bin");
        assert!(matches!(
            &summary.failed[0].issues[..],
            [VerificationIssue::UnreadableTarget { .. }]
        ));
    }

    #[test]
    fn verify_files_stops_early_when_shutdown_signal_is_pre_triggered() {
        let sandbox = TempTestDir::new("verify-interrupt");
        let files = vec![
            xml_file("missing1.bin", Some(10), None),
            xml_file("missing2.bin", Some(20), None),
        ];

        let signal = ShutdownSignal::new();
        signal.trigger();

        let summary = verify_files(sandbox.path(), &files, &signal).unwrap();
        assert!(summary.interrupted);
        assert_eq!(summary.ok, 0);
        assert!(summary.failed.is_empty());
        assert!(format_verification_summary(&summary).contains("interrupted"));
    }

    #[test]
    fn format_verification_summary_formats_pass_failures_and_interrupted_states() {
        let pass = VerificationSummary {
            ok: 3,
            failed: Vec::new(),
            interrupted: false,
        };
        assert_eq!(
            format_verification_summary(&pass),
            "Verification summary: 3 ok, 0 failed"
        );

        let failures = VerificationSummary {
            ok: 1,
            failed: vec![
                FailedVerification {
                    file_path: "a.bin".to_string(),
                    issues: vec![
                        VerificationIssue::LeftoverPart {
                            part_path: "a.bin.part".to_string(),
                            size: 12,
                        },
                        VerificationIssue::Missing,
                    ],
                },
                FailedVerification {
                    file_path: "b.bin".to_string(),
                    issues: vec![VerificationIssue::SizeMismatch {
                        expected: 100,
                        actual: 50,
                    }],
                },
                FailedVerification {
                    file_path: "c.bin".to_string(),
                    issues: vec![VerificationIssue::Md5Mismatch {
                        expected: "aaaa".to_string(),
                        actual: "bbbb".to_string(),
                    }],
                },
                FailedVerification {
                    file_path: "d.bin".to_string(),
                    issues: vec![VerificationIssue::UnreadableTarget {
                        reason: "permission denied".to_string(),
                    }],
                },
            ],
            interrupted: false,
        };
        assert_eq!(
            format_verification_summary(&failures),
            concat!(
                "Verification summary: 1 ok, 4 failed\n",
                "Failed files:\n",
                "  - a.bin: leftover partial file 'a.bin.part' (12 bytes); missing\n",
                "  - b.bin: size mismatch (expected 100 bytes, actual 50 bytes)\n",
                "  - c.bin: MD5 mismatch (expected aaaa, actual bbbb)\n",
                "  - d.bin: unreadable target: permission denied"
            )
        );

        let interrupted_clean = VerificationSummary {
            ok: 2,
            failed: Vec::new(),
            interrupted: true,
        };
        assert_eq!(
            format_verification_summary(&interrupted_clean),
            "Verification summary: 2 ok, 0 failed, interrupted"
        );

        let interrupted_with_failure = VerificationSummary {
            ok: 0,
            failed: vec![FailedVerification {
                file_path: "missing.bin".to_string(),
                issues: vec![VerificationIssue::Missing],
            }],
            interrupted: true,
        };
        assert_eq!(
            format_verification_summary(&interrupted_with_failure),
            "Verification summary: 0 ok, 1 failed, interrupted\nFailed files:\n  - missing.bin: missing"
        );
    }

    #[test]
    fn verify_files_with_logger_emits_plain_diagnostics_and_stays_silent_when_disabled() {
        let sandbox = TempTestDir::new("verify-verbose");

        let good_payload = b"verified archive content";
        let good_md5 = md5_hex(good_payload);
        sandbox.write("good.bin", good_payload);

        let corrupt_payload = b"corrupted content";
        sandbox.write("corrupt.bin", corrupt_payload);

        let files = vec![
            xml_file("good.bin", Some(good_payload.len() as u64), Some(&good_md5)),
            xml_file(
                "corrupt.bin",
                Some(corrupt_payload.len() as u64),
                Some("00000000000000000000000000000000"),
            ),
        ];

        let signal = ShutdownSignal::new();
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone());

        let summary = verify_files_with_logger(sandbox.path(), &files, &signal, &logger).unwrap();
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "corrupt.bin");
        assert!(!summary.interrupted);

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "verification verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.text();
        assert!(
            logs.contains("[verbose] verify: ok 'good.bin'"),
            "missing ok log line:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] verify: failed 'corrupt.bin': MD5 mismatch"),
            "missing failure reason log line:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] summary: ok=1 failed=1 interrupted=false"),
            "missing verification summary line:\n{logs}"
        );

        let disabled = VerboseLogger::disabled();
        assert!(!disabled.is_enabled());
        let silent_summary =
            verify_files_with_logger(sandbox.path(), &files, &signal, &disabled).unwrap();
        assert_eq!(silent_summary, summary);
    }
}
