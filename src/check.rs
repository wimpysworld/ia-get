//! Verifies a local directory against an archive's `_files.xml` metadata.
//!
//! `ia-get --check` compares the files a download would have produced
//! (sanitized names, under the `-o` directory) with what is actually on
//! disk: presence, size, last-modified time, and — on demand — the MD5
//! hash. It never downloads anything.
//!
//! The archive's self-referencing `<id>_files.xml` entry is excluded from the
//! size/date/hash comparison (its own metadata is unreliable) but is still
//! recognized, so it is not reported as an unexpected file. Leftover
//! `<name>.part` files are accounted for: a `.part` whose final file is
//! missing means the download is incomplete, while a `.part` next to a
//! complete file is a stale leftover.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::UNIX_EPOCH;

use colored::*;

use crate::Result;
use crate::display::format_size;
use crate::downloader::{calculate_md5, setup_signal_handler};
use crate::plan::DownloadPlan;

/// The findings of a directory check, grouped by kind.
///
/// The hard categories (missing, incomplete, size, md5, type, read) always
/// fail the run; the soft ones (date, extra) fail it only in `--strict`
/// mode. `notes` are informational and never fail the run.
#[derive(Debug, Default)]
pub struct CheckReport {
    /// Files that matched the metadata: present, and size/hash (when checked)
    /// correct.
    pub ok: Vec<String>,
    /// Listed in the metadata but absent from the directory.
    pub missing: Vec<String>,
    /// Listed in the metadata, final file absent, but a `<name>.part` holds a
    /// prefix: `(relative path, bytes present, expected bytes)`.
    pub incomplete: Vec<(String, u64, u64)>,
    /// Present but the wrong size: `(relative path, on-disk, expected)`.
    pub size_mismatch: Vec<(String, u64, u64)>,
    /// Present, a metadata MD5 exists, and the on-disk hash differs:
    /// `(relative path, on-disk hash, expected hash)`.
    pub md5_mismatch: Vec<(String, String, String)>,
    /// A directory occupies a path the metadata lists as a file.
    pub type_mismatch: Vec<String>,
    /// Present, but unreadable: the size could not be read, or the `--md5`
    /// pass could not hash the file. The download would have verified it, so
    /// an unverifiable copy fails the check.
    pub read_failed: Vec<String>,
    /// Present, and the on-disk mtime differs from the metadata's `<mtime>`.
    /// The download prefers the server's `Last-Modified`, so this is a warning
    /// by default, not a failure.
    pub date_mismatch: Vec<String>,
    /// Files in the directory that no metadata entry (or its `.part`)
    /// accounts for.
    pub extra: Vec<String>,
    /// Informational lines (no MD5 in the metadata, an unreadable file, ...).
    pub notes: Vec<String>,
}

impl CheckReport {
    /// The number of findings that fail the run: the hard categories always,
    /// the soft ones (date, extra) only when `strict`.
    pub fn failing_count(&self, strict: bool) -> usize {
        let hard = self.missing.len()
            + self.incomplete.len()
            + self.size_mismatch.len()
            + self.md5_mismatch.len()
            + self.type_mismatch.len()
            + self.read_failed.len();
        let warn = self.date_mismatch.len() + self.extra.len();
        hard + if strict { warn } else { 0 }
    }

    /// Whether the directory conforms for the mode: no failing findings.
    pub fn is_clean(&self, strict: bool) -> bool {
        self.failing_count(strict) == 0
    }

    /// Prints the findings grouped by status, then a one-line summary.
    pub fn print(&self) {
        for rel in &self.ok {
            println!("{} {}", "✔".green().bold(), rel);
        }
        for (rel, have, expected) in &self.incomplete {
            println!(
                "{} {}  {} ({}/{})",
                "…".yellow().bold(),
                rel,
                "incomplete".yellow(),
                format_size(*have).dimmed(),
                format_size(*expected).dimmed()
            );
        }
        for rel in &self.missing {
            println!("{} {}  {}", "✘".red().bold(), rel, "missing".red());
        }
        for (rel, disk, expected) in &self.size_mismatch {
            println!(
                "{} {}  {} {} (expected {})",
                "✘".red().bold(),
                rel,
                "size".red(),
                format_size(*disk).red(),
                format_size(*expected).dimmed()
            );
        }
        for (rel, disk, expected) in &self.md5_mismatch {
            println!(
                "{} {}  {} {} (expected {})",
                "✘".red().bold(),
                rel,
                "md5".red(),
                disk.red(),
                expected.dimmed()
            );
        }
        for rel in &self.type_mismatch {
            println!(
                "{} {}  {}",
                "✘".red().bold(),
                rel,
                "a directory occupies this file path".red()
            );
        }
        for rel in &self.read_failed {
            println!(
                "{} {}  {}",
                "✘".red().bold(),
                rel,
                "unreadable; could not be verified".red()
            );
        }
        for rel in &self.date_mismatch {
            println!(
                "{} {}  {}",
                "▲".yellow().bold(),
                rel,
                "last-modified time differs".yellow()
            );
        }
        for rel in &self.extra {
            println!(
                "{} {}",
                "⚠".yellow().bold(),
                format!("extra {rel}").yellow()
            );
        }
        for note in &self.notes {
            println!("{} {}", "ℹ".blue().bold(), note.dimmed());
        }

        // The exclusive presence/content outcomes; date and extra are
        // additive warnings and are reported on their own.
        let checked = self.ok.len()
            + self.missing.len()
            + self.incomplete.len()
            + self.size_mismatch.len()
            + self.md5_mismatch.len()
            + self.type_mismatch.len()
            + self.read_failed.len();
        println!();
        println!(
            "{} checked {} file{}: {} ok, {} missing, {} incomplete, {} size, {} md5, {} type, {} read, {} date⚠, {} extra⚠",
            "Σ".bold(),
            checked,
            if checked == 1 { "" } else { "s" },
            self.ok.len(),
            self.missing.len(),
            self.incomplete.len(),
            self.size_mismatch.len(),
            self.md5_mismatch.len(),
            self.type_mismatch.len(),
            self.read_failed.len(),
            self.date_mismatch.len(),
            self.extra.len()
        );
    }
}

/// Verifies the directory that `output_dir` names (the current directory when
/// `output_dir` is empty) against the files `plan` will download.
///
/// Each planned file is checked for presence, size, and — when `verify_md5` —
/// the MD5 hash; last-modified time is always compared to the metadata's
/// `<mtime>` and reported. Everything on disk that no planned file (or its
/// `.part`) explains is collected as unexpected.
///
/// Registers the Ctrl+C handler (via `setup_signal_handler`) so a long MD5
/// pass can be aborted; a stop surfaces as `IaGetError::Interrupted`, which
/// the caller (`main::run`) announces before `main` maps it to an exit code.
pub fn check_directory(
    plan: &DownloadPlan,
    xml_file_name: &str,
    whole_item: bool,
    output_dir: &str,
    verify_md5: bool,
) -> Result<CheckReport> {
    check_directory_with_signal(
        plan,
        xml_file_name,
        whole_item,
        output_dir,
        verify_md5,
        &setup_signal_handler(),
    )
}

/// The directory check with an externally provided stop flag, mirroring the
/// download side's `download_files_with_signal`: tests drive the
/// Ctrl+C-during-MD5 path with a pre-stopped flag instead of registering a
/// second (and panicking) signal handler. A stop propagates as
/// `IaGetError::Interrupted` at the first file whose hash it reaches.
fn check_directory_with_signal(
    plan: &DownloadPlan,
    xml_file_name: &str,
    whole_item: bool,
    output_dir: &str,
    verify_md5: bool,
    running: &Arc<AtomicBool>,
) -> Result<CheckReport> {
    let root = if output_dir.is_empty() {
        Path::new(".")
    } else {
        Path::new(output_dir)
    };
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    walk_into(root, "", &mut files, &mut dirs, &mut others);
    // Key disk names and metadata names the way the volume treats them:
    // lowercased on a case-insensitive volume (a case difference is one
    // path, not "missing + extra"), identity on a case-sensitive one
    // (collapsing them there would verify the wrong file or pass a check
    // for a file that does not exist under its expected spelling)
    let ci = volume_case_insensitive(root, &files);
    let key = |name: &str| -> String {
        if ci {
            name.to_lowercase()
        } else {
            name.to_string()
        }
    };

    let file_set: HashSet<String> = files.iter().map(|file| key(file)).collect();
    let dir_set: HashSet<String> = dirs.iter().map(|dir| key(dir)).collect();

    let expected_rel: Vec<String> = plan
        .tasks
        .iter()
        .map(|task| expected_rel(&task.file_path, output_dir))
        .collect();
    let expected_set: HashSet<String> = expected_rel.iter().map(|rel| key(rel)).collect();

    // The `.part` suffix obeys the volume's case rules too, so a leftover
    // spelled `A.BIN.PART` pairs with its `a.bin` exactly once instead of
    // being classified both "incomplete" and "extra" (see part_base).

    let mut report = CheckReport::default();

    // A whole-item run always saves the metadata file: its absence means
    // the directory is not what a download would have produced (it stays
    // excluded from the size/date/hash comparison, whose self-metadata is
    // unreliable)
    if whole_item && !file_set.contains(&key(xml_file_name)) {
        report.missing.push(xml_file_name.to_string());
    }

    // Classify each planned file against what is on disk.
    for (task, rel) in plan.tasks.iter().zip(expected_rel.iter()) {
        if dir_set.contains(&key(rel)) {
            // A directory where a file is expected can never be right.
            report.type_mismatch.push(rel.clone());
            continue;
        }
        if !file_set.contains(&key(rel)) {
            let part = format!("{rel}.part");
            if file_set.contains(&key(&part)) {
                let have = fs::metadata(root.join(&part))
                    .ok()
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                report
                    .incomplete
                    .push((rel.clone(), have, task.expected_size.unwrap_or(0)));
            } else {
                report.missing.push(rel.clone());
            }
            continue;
        }

        // Present as a regular file.
        let abs = root.join(rel);
        let meta = match fs::metadata(&abs) {
            Ok(meta) => meta,
            // Present on the walk but not stat-able now (a permission that
            // changed, a vanishing file): the download would have verified
            // it, so an unverifiable copy fails the check rather than
            // staying a note
            Err(_) => {
                report.read_failed.push(rel.clone());
                continue;
            }
        };

        // A .part next to a complete file is a stale leftover.
        let part = format!("{rel}.part");
        if file_set.contains(&key(&part)) {
            report.extra.push(part);
        }

        let size_ok = task
            .expected_size
            .is_none_or(|expected| meta.len() == expected);
        if !size_ok {
            report
                .size_mismatch
                .push((rel.clone(), meta.len(), task.expected_size.unwrap_or(0)));
            continue;
        }

        if verify_md5 {
            match &task.expected_md5 {
                Some(expected) => {
                    let Some(path_str) = abs.to_str() else {
                        report.read_failed.push(rel.clone());
                        continue;
                    };
                    match calculate_md5(path_str, running) {
                        Ok(hash) if hash.eq_ignore_ascii_case(expected) => {}
                        Ok(hash) => {
                            report
                                .md5_mismatch
                                .push((rel.clone(), hash, expected.clone()));
                            continue;
                        }
                        Err(e) if matches!(e, crate::IaGetError::Interrupted) => return Err(e),
                        // --md5 was requested and the hash could not be
                        // computed: the copy is unverifiable, a hard finding
                        Err(_) => {
                            report.read_failed.push(rel.clone());
                            continue;
                        }
                    }
                }
                None => report
                    .notes
                    .push(format!("no MD5 in metadata for {rel}; hash not verified")),
            }
        }

        if let Some(expected_mtime) = task.expected_mtime
            && let Some(disk_mtime) = mtime_secs(&abs)
            && disk_mtime != expected_mtime
        {
            report.date_mismatch.push(rel.clone());
        }

        report.ok.push(rel.clone());
    }

    // Classify the directory's files: anything a planned entry (or its .part)
    // does not account for is unexpected. The metadata file is recognized for
    // whole-item runs (it is always saved and reported above when absent),
    // so it is not flagged as extra.
    for rel in &files {
        if expected_set.contains(&key(rel)) || (whole_item && key(rel) == key(xml_file_name)) {
            continue;
        }
        if let Some(base) = part_base(ci, rel)
            && expected_set.contains(&key(base))
        {
            // A .part for a planned file is in progress (final still missing)
            // or stale (already reported above) — either way not "extra".
            continue;
        }
        report.extra.push(rel.clone());
    }

    // Non-regular entries (symlinks, FIFOs, ...) nothing explains: extras.
    // A symlink whose name matches a planned file was already ruled
    // "missing" above (it is not in the regular-file sets), so it is not
    // double-reported here.
    for rel in &others {
        if expected_set.contains(&key(rel)) {
            continue;
        }
        report.extra.push(rel.clone());
    }

    // Empty unexpected directories: a walked directory is legitimate only if
    // it holds a regular file somewhere beneath it (an expected file, or one
    // already reported as extra) or it is the ancestor of a planned file
    // (whose absence is reported separately). An otherwise-empty directory
    // has no file to make it visible, so it is collected here.
    for dir in &dirs {
        let prefix = format!("{dir}/");
        let holds_a_file = files.iter().any(|file| file.starts_with(&prefix));
        let ancestor_of_expected = expected_rel
            .iter()
            .any(|expected| expected.starts_with(&prefix));
        if !holds_a_file && !ancestor_of_expected {
            report.extra.push(dir.clone());
        }
    }

    Ok(report)
}

/// The archive-relative local path a planned file lands at, i.e. its
/// `file_path` with the `output_dir` prefix removed: that is the path
/// relative to the scanned root.
fn expected_rel(file_path: &str, output_dir: &str) -> String {
    if output_dir.is_empty() {
        file_path.to_string()
    } else {
        let prefix = format!("{output_dir}/");
        file_path
            .strip_prefix(&prefix)
            .unwrap_or(file_path)
            .to_string()
    }
}

/// Recursively collects the regular files, directories, and every other
/// entry (symlinks, FIFOs, ...) under `dir` as relative paths (always with
/// `/` separators), appending to `files`, `dirs` and `others`. Non-regular
/// entries are recorded but never followed: a symlink where a file is
/// expected still surfaces as "missing" (the safe call), and an unexpected
/// one is reported as extra instead of staying invisible. `rel_prefix` is
/// the path of `dir` relative to the scan root.
fn walk_into(
    dir: &Path,
    rel_prefix: &str,
    files: &mut Vec<String>,
    dirs: &mut Vec<String>,
    others: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            dirs.push(rel.clone());
            walk_into(&entry.path(), &rel, files, dirs, others);
        } else if file_type.is_file() {
            files.push(rel);
        } else {
            others.push(rel);
        }
    }
}

/// Whether `root`'s volume treats names differing only by case as one path.
///
/// Probed read-only (a check never writes): a walked file that carries an
/// ASCII letter is stat-ed under a spelling with that letter's case
/// flipped. A miss settles the question (case-sensitive); a hit is only
/// case-insensitivity if the flipped spelling is NOT also present as its
/// own entry — a case-sensitive volume that holds both spellings (twins)
/// would otherwise probe as insensitive and collapse the two distinct
/// paths. A root without such a file cannot be probed: it stays
/// case-sensitively keyed, the conservative direction (a case difference
/// then reports as missing + extra instead of silently verifying the other
/// spelling).
fn volume_case_insensitive(root: &Path, files: &[String]) -> bool {
    files
        .iter()
        .find_map(|rel| {
            let flipped = flip_first_ascii_letter(rel)?;
            if fs::metadata(root.join(&flipped)).is_ok() {
                Some(!files.iter().any(|other| other == &flipped))
            } else {
                Some(false)
            }
        })
        .unwrap_or(false)
}

/// `rel` with its first ASCII letter's case flipped, or `None` when it
/// holds no ASCII letter to flip.
fn flip_first_ascii_letter(rel: &str) -> Option<String> {
    let mut flipped = false;
    let out = rel
        .chars()
        .map(|c| {
            if !flipped && c.is_ascii_alphabetic() {
                flipped = true;
                if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                }
            } else {
                c
            }
        })
        .collect::<String>();
    if flipped { Some(out) } else { None }
}

/// The planned name a `.part` leftover stands for: the `.part` suffix is
/// matched the way the volume treats case — lowercased on a
/// case-insensitive one (so `A.BIN.PART` pairs with `a.bin`), exactly on a
/// case-sensitive one.
fn part_base(ci: bool, rel: &str) -> Option<&str> {
    if ci {
        rel.to_lowercase().strip_suffix(".part")?;
        Some(&rel[..rel.len() - 5])
    } else {
        rel.strip_suffix(".part")
    }
}

/// A file's last-modified time as Unix seconds, or `None` when it cannot be
/// read.
fn mtime_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::DownloadTask;
    use crate::test_support::{TempDir, md5_hex, task};

    /// The on-disk path string for `root/name`, in the `file_path` spelling
    /// (a native root with `/` separators).
    fn path_str(root: &Path, name: &str) -> String {
        format!("{}/{}", root.to_string_lossy(), name)
    }

    /// Creates `root/name` (and any missing parents) holding `bytes` zero
    /// bytes, returning its `file_path` string.
    fn make_file(root: &Path, name: &str, bytes: usize) -> String {
        let abs = root.join(name);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, vec![0u8; bytes]).unwrap();
        path_str(root, name)
    }

    fn plan(tasks: Vec<DownloadTask>) -> DownloadPlan {
        DownloadPlan {
            tasks,
            sanitized_count: 0,
            warnings: Vec::new(),
        }
    }

    /// A fresh temp dir plus its `output_dir` string (the directory the
    /// check scans, in the same spelling as the plan's file paths).
    fn harness(name: &str) -> (TempDir, String) {
        let dir = TempDir::new(name);
        let output_dir = dir.to_string_lossy().to_string();
        (dir, output_dir)
    }

    #[test]
    fn present_matching_file_is_ok() {
        let (dir, output_dir) = harness("check_ok");
        let file_path = make_file(&dir, "a.bin", 10);
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.ok, vec!["a.bin".to_string()]);
        assert!(report.is_clean(false));
        assert_eq!(report.failing_count(false), 0);
    }

    #[test]
    fn absent_file_is_missing() {
        let (dir, output_dir) = harness("check_missing");
        let file_path = path_str(&dir, "a.bin");
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.missing, vec!["a.bin".to_string()]);
        assert_eq!(report.failing_count(false), 1);
        assert!(!report.is_clean(false));
    }

    #[test]
    fn part_only_is_incomplete_not_extra() {
        let (dir, output_dir) = harness("check_incomplete");
        fs::write(dir.join("a.bin.part"), vec![0u8; 40]).unwrap();
        let file_path = path_str(&dir, "a.bin");
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                None,
                Some(100),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.incomplete, vec![("a.bin".to_string(), 40, 100)]);
        assert!(
            report.extra.is_empty(),
            "an in-progress .part is not an unexpected file: {:?}",
            report.extra
        );
        assert_eq!(report.failing_count(false), 1);
    }

    #[test]
    fn wrong_size_is_a_mismatch() {
        let (dir, output_dir) = harness("check_size");
        let file_path = make_file(&dir, "a.bin", 20);
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.size_mismatch, vec![("a.bin".to_string(), 20, 10)]);
        assert_eq!(report.failing_count(false), 1);
    }

    #[test]
    fn extra_file_is_a_warning_not_a_failure() {
        let (dir, output_dir) = harness("check_extra");
        let a = make_file(&dir, "a.bin", 10);
        fs::write(dir.join("stray.txt"), b"hi").unwrap();
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.extra, vec!["stray.txt".to_string()]);
        assert!(
            report.is_clean(false),
            "extra files do not fail a non-strict check"
        );
        assert!(!report.is_clean(true), "extra files fail a strict check");
        assert_eq!(report.failing_count(true), 1);
    }

    #[test]
    fn stale_part_next_to_complete_file_is_extra() {
        let (dir, output_dir) = harness("check_stale_part");
        let a = make_file(&dir, "a.bin", 10);
        fs::write(dir.join("a.bin.part"), vec![0u8; 3]).unwrap();
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.ok, vec!["a.bin".to_string()]);
        assert_eq!(report.extra, vec!["a.bin.part".to_string()]);
    }

    #[test]
    fn directory_where_a_file_is_expected_is_a_type_mismatch() {
        let (dir, output_dir) = harness("check_type");
        fs::create_dir(dir.join("a.bin")).unwrap();
        let file_path = path_str(&dir, "a.bin");
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.type_mismatch, vec!["a.bin".to_string()]);
        assert_eq!(report.failing_count(false), 1);
    }

    #[test]
    fn metadata_file_is_not_extra_for_whole_items() {
        let (dir, output_dir) = harness("check_xml_whole");
        let a = make_file(&dir, "a.bin", 10);
        fs::write(dir.join("item_files.xml"), b"<xml/>").unwrap();

        let whole = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                a.clone(),
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            true,
            &output_dir,
            false,
        )
        .unwrap();
        assert!(
            whole.extra.is_empty(),
            "the saved _files.xml is expected for a whole-item download: {:?}",
            whole.extra
        );

        let single = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();
        assert_eq!(
            single.extra,
            vec!["item_files.xml".to_string()],
            "a single-file run never saves the metadata, so it is unexpected"
        );
    }

    #[test]
    fn differing_case_file_matches_the_metadata() {
        // The disk holds "A.BIN" while the metadata names "a.bin": on
        // case-insensitive filesystems (Windows, default macOS) that is one
        // path, not "missing + extra"
        let (dir, output_dir) = harness("check_case");
        fs::write(dir.join("A.BIN"), vec![0u8; 10]).unwrap();
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                path_str(&dir, "a.bin"),
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        #[cfg(target_os = "linux")]
        {
            // Linux: genuinely two different paths — missing + extra
            assert_eq!(report.missing, vec!["a.bin".to_string()]);
            assert_eq!(report.extra, vec!["A.BIN".to_string()]);
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(report.ok, vec!["a.bin".to_string()], "{report:?}");
            assert!(report.missing.is_empty(), "{:?}", report.missing);
            assert!(report.extra.is_empty(), "{:?}", report.extra);
        }
    }

    #[test]
    fn differing_case_part_is_classified_once() {
        // A leftover spelled "A.BIN.PART" for the expected "a.bin": on a
        // case-insensitive volume it is the same .part — exactly one finding
        // (incomplete), not incomplete + extra
        let (dir, output_dir) = harness("check_case_part");
        fs::write(dir.join("A.BIN.PART"), vec![0u8; 40]).unwrap();
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                path_str(&dir, "a.bin"),
                None,
                Some(100),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        #[cfg(target_os = "linux")]
        {
            // Linux: "A.BIN.PART" is a different name — missing + extra
            assert_eq!(report.missing, vec!["a.bin".to_string()]);
            assert_eq!(report.extra, vec!["A.BIN.PART".to_string()]);
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(report.incomplete, vec![("a.bin".to_string(), 40, 100)]);
            assert!(report.extra.is_empty(), "{:?}", report.extra);
            assert!(report.missing.is_empty(), "{:?}", report.missing);
        }
    }

    #[test]
    fn flip_first_ascii_letter_flips_only_the_first() {
        assert_eq!(flip_first_ascii_letter("A.BIN"), Some("a.BIN".to_string()));
        assert_eq!(flip_first_ascii_letter("a.bin"), Some("A.bin".to_string()));
        // A non-ASCII prefix is skipped, the first ASCII letter flips
        assert_eq!(
            flip_first_ascii_letter("файл.bin"),
            Some("файл.Bin".to_string())
        );
        assert_eq!(flip_first_ascii_letter("123"), None);
    }

    #[test]
    fn missing_metadata_file_fails_a_whole_item_check() {
        // A whole-item download always saves <id>_files.xml: its absence
        // must fail the check even though its size/date/hash are exempt
        let (dir, output_dir) = harness("check_xml_missing");
        let a = make_file(&dir, "a.bin", 10);
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            true,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.missing, vec!["item_files.xml".to_string()]);
        assert!(!report.is_clean(false), "the metadata file is required");

        // A single-file run never saves it, so its absence is no finding
        let b = make_file(&dir, "b.bin", 5);
        let single = check_directory(
            &plan(vec![task("https://x/b.bin", b, None, Some(5), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();
        assert!(single.missing.is_empty(), "{:?}", single.missing);
    }

    #[cfg(unix)]
    #[test]
    fn unexpected_symlink_is_reported_as_extra() {
        // A non-regular entry the metadata does not explain: extra, not
        // invisible (the walk records it without following it)
        let (dir, output_dir) = harness("check_symlink_extra");
        let a = make_file(&dir, "a.bin", 10);
        let link = dir.join("stray-link");
        std::os::unix::fs::symlink(dir.join("absent-target"), &link).unwrap();

        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.extra, vec!["stray-link".to_string()]);
        assert!(report.is_clean(false), "a symlink extra stays a warning");
        assert!(
            !report.is_clean(true),
            "a strict check must see the symlink"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn twin_spellings_stay_distinct_findings() {
        // A case-sensitive volume holding both spellings of one name: the
        // probe must not mistake the flipped twin for case-insensitivity —
        // the expected file verifies, the twin stays extra
        let (dir, output_dir) = harness("check_twins");
        fs::write(dir.join("a.bin"), vec![0u8; 10]).unwrap();
        fs::write(dir.join("A.bin"), vec![0u8; 10]).unwrap();

        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                path_str(&dir, "a.bin"),
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.ok, vec!["a.bin".to_string()]);
        assert_eq!(report.extra, vec!["A.bin".to_string()]);
        assert!(report.missing.is_empty(), "{:?}", report.missing);
    }

    #[cfg(unix)]
    #[test]
    fn md5_pass_on_an_unreadable_file_is_a_hard_finding() {
        // --md5 requested, the hash cannot be computed (no read permission):
        // an unverifiable copy fails the check, it does not stay a note
        let (dir, output_dir) = harness("check_md5_unreadable");
        let path = dir.join("a.bin");
        fs::write(&path, b"hello").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0)).unwrap();
        // Root (and other privilege) ignores mode bits: if the mode-000 file
        // still opens, the unreadable path cannot be exercised on this host,
        // so restore the permissions and skip rather than fail spuriously.
        if fs::File::open(&path).is_ok() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            eprintln!("skipping: mode 000 does not block reads here (root?)");
            return;
        }

        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                path.to_str().unwrap().to_string(),
                Some(md5_hex(b"hello")),
                Some(5),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            true,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(report.read_failed, vec!["a.bin".to_string()]);
        assert!(
            !report.is_clean(false),
            "an unreadable file fails the check"
        );
    }

    #[test]
    fn check_directory_interrupt_during_md5_is_an_error() {
        // A Ctrl+C during the --md5 pass must surface as an error, not a
        // (wrong) digest: the pre-stopped flag short-circuits the hash. The
        // public check_directory registers a process-wide handler, so the
        // test drives the flag-injected variant directly.
        let (dir, output_dir) = harness("check_interrupt_md5");
        fs::write(dir.join("a.bin"), b"hello").unwrap();
        // path_str (the forward-slash spelling) so expected_rel strips the
        // output_dir prefix — a native (backslash) path would not
        let file_path = path_str(&dir, "a.bin");
        let stopped = Arc::new(AtomicBool::new(false));
        let err = check_directory_with_signal(
            &plan(vec![task(
                "https://x/a.bin",
                file_path,
                Some(md5_hex(b"hello")),
                Some(5),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            true,
            &stopped,
        )
        .unwrap_err();
        assert!(
            matches!(err, crate::IaGetError::Interrupted),
            "an interrupt during the --md5 hash must propagate, got {err:?}"
        );
    }

    #[test]
    fn differing_mtime_is_a_warning_not_a_failure() {
        let (dir, output_dir) = harness("check_date");
        let a = make_file(&dir, "a.bin", 10);
        // A far-past mtime can never match the file's actual (now) mtime.
        let report = check_directory(
            &plan(vec![task(
                "https://x/a.bin",
                a,
                None,
                Some(10),
                Some(1_000_000_000),
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.date_mismatch, vec!["a.bin".to_string()]);
        assert!(report.is_clean(false));
        assert!(!report.is_clean(true));
    }

    #[test]
    fn md5_is_checked_only_when_requested() {
        let (dir, output_dir) = harness("check_md5");
        fs::write(dir.join("good.bin"), b"hello").unwrap();
        fs::write(dir.join("bad.bin"), b"world").unwrap();
        let good = path_str(&dir, "good.bin");
        let bad = path_str(&dir, "bad.bin");
        let good_md5 = md5_hex(b"hello");

        // Two fresh task lists (DownloadTask is not Clone), one per run.
        let make_tasks = || {
            vec![
                task(
                    "https://x/good.bin",
                    good.clone(),
                    Some(good_md5.clone()),
                    Some(5),
                    None,
                ),
                task(
                    "https://x/bad.bin",
                    bad.clone(),
                    Some("00000000000000000000000000000000".to_string()),
                    Some(5),
                    None,
                ),
            ]
        };

        // The report holds archive-relative names, not full paths.
        // Without --md5 the hash is never computed: both are ok.
        let off = check_directory(
            &plan(make_tasks()),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();
        assert_eq!(off.ok, vec!["good.bin".to_string(), "bad.bin".to_string()]);
        assert!(off.md5_mismatch.is_empty());

        // With --md5 the wrong hash is caught, the right one passes.
        let on = check_directory(
            &plan(make_tasks()),
            "item_files.xml",
            false,
            &output_dir,
            true,
        )
        .unwrap();
        assert_eq!(on.ok, vec!["good.bin".to_string()]);
        assert_eq!(on.md5_mismatch.len(), 1);
        assert_eq!(on.md5_mismatch[0].0, "bad.bin");
        assert_eq!(on.failing_count(false), 1);
    }

    #[test]
    fn empty_unexpected_directory_is_extra() {
        let (dir, output_dir) = harness("check_empty_dir");
        let a = make_file(&dir, "a.bin", 10);
        fs::create_dir(dir.join("stray")).unwrap(); // an empty, unexplained dir
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert!(
            report.extra.iter().any(|e| e == "stray"),
            "an empty unexpected directory must be reported: {:?}",
            report.extra
        );
        assert!(!report.is_clean(true));
    }

    #[test]
    fn nested_empty_directories_are_each_reported() {
        let (dir, output_dir) = harness("check_nested_empty_dir");
        let a = make_file(&dir, "a.bin", 10);
        fs::create_dir_all(dir.join("stray/deeper")).unwrap();
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        let extras: HashSet<&str> = report.extra.iter().map(String::as_str).collect();
        assert!(
            extras.contains("stray"),
            "outer empty dir: {:?}",
            report.extra
        );
        assert!(
            extras.contains("stray/deeper"),
            "inner empty dir: {:?}",
            report.extra
        );
    }

    #[test]
    fn ancestor_dir_of_a_missing_file_is_not_extra() {
        // "sub" holds only the planned (but absent) "sub/file.bin": it is
        // part of the expected structure, not a leftover.
        let (dir, output_dir) = harness("check_ancestor_dir");
        fs::create_dir(dir.join("sub")).unwrap();
        let rel = path_str(&dir, "sub/file.bin");
        let report = check_directory(
            &plan(vec![task(
                "https://x/sub/file.bin",
                rel,
                None,
                Some(10),
                None,
            )]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.missing, vec!["sub/file.bin".to_string()]);
        assert!(
            report.extra.is_empty(),
            "the expected ancestor directory must not be flagged: {:?}",
            report.extra
        );
    }

    #[test]
    fn dir_holding_an_extra_file_is_not_itself_extra() {
        // "stray" is visible through its file; it must not be reported twice.
        let (dir, output_dir) = harness("check_dir_with_file");
        let a = make_file(&dir, "a.bin", 10);
        fs::create_dir(dir.join("stray")).unwrap();
        fs::write(dir.join("stray/junk.txt"), b"x").unwrap();
        let report = check_directory(
            &plan(vec![task("https://x/a.bin", a, None, Some(10), None)]),
            "item_files.xml",
            false,
            &output_dir,
            false,
        )
        .unwrap();

        assert_eq!(report.extra, vec!["stray/junk.txt".to_string()]);
    }
}
