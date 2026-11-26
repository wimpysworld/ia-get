//! Pre-write destination path collision detection and per-directory filesystem equivalence probing.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::fs_safety::{configure_no_follow, part_file_path, validate_relative_path};
use crate::error::IaGetError;
use crate::Result;

/// Counter for generating unique temporary probe filenames during case-sensitivity detection.
static CASE_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum directory entries inspected during read-only case-sensitivity probing.
const MAX_READ_ONLY_CASE_PROBE_ENTRIES: usize = 32;

/// Maximum attempts to create a unique temporary probe file before falling back.
const MAX_TEMP_CASE_PROBE_ATTEMPTS: usize = 16;

/// Case-sensitivity rule used when checking planned download paths for collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCaseSensitivity {
    /// Treat paths differing only in letter case as distinct (for example ext4, tmpfs, or
    /// case-sensitive APFS).
    CaseSensitive,
    /// Treat paths differing only in letter case as colliding (for example default APFS, NTFS,
    /// FAT32, or exFAT).
    CaseInsensitive,
}

/// Resolves `dir` (or its nearest existing ancestor directory) for filesystem probing.
fn resolve_existing_probe_dir(dir: &Path) -> Option<PathBuf> {
    if dir.as_os_str().is_empty() {
        return None;
    }

    let candidate = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(dir)
    };

    for ancestor in candidate.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if fs::metadata(ancestor)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            return Some(ancestor.to_path_buf());
        }
    }

    None
}

/// Flips the ASCII letter case of `name`, returning `None` if `name` contains no ASCII letters.
fn toggle_ascii_case(name: &str) -> Option<String> {
    if !name.bytes().any(|b| b.is_ascii_alphabetic()) {
        return None;
    }

    let toggled: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect();
    Some(toggled)
}

/// Probes case sensitivity inside `probe_dir` without writing by inspecting existing entries.
fn probe_existing_child_case_sensitivity(probe_dir: &Path) -> Option<PathCaseSensitivity> {
    let entries = fs::read_dir(probe_dir).ok()?;

    for entry in entries.take(MAX_READ_ONLY_CASE_PROBE_ENTRIES) {
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        let Some(toggled) = toggle_ascii_case(name_str) else {
            continue;
        };

        let toggled_path = probe_dir.join(&toggled);
        match fs::symlink_metadata(&toggled_path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // Re-verify the original entry still exists so a concurrent deletion in a shared
                // directory cannot be misclassified as case-sensitive.
                if fs::symlink_metadata(probe_dir.join(&file_name)).is_ok() {
                    return Some(PathCaseSensitivity::CaseSensitive);
                }
            }
            Ok(toggled_meta) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(orig_meta) = entry.metadata() {
                        if orig_meta.dev() == toggled_meta.dev()
                            && orig_meta.ino() == toggled_meta.ino()
                            && (orig_meta.is_dir() || orig_meta.nlink() == 1)
                        {
                            return Some(PathCaseSensitivity::CaseInsensitive);
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = toggled_meta;
                }
            }
            Err(_) => continue,
        }
    }

    None
}

/// Normalises a relative destination path into a collision-detection key.
fn collision_path_key(path: &Path) -> PathBuf {
    validate_relative_path(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Formats a normalised relative collision path using `/` separators for consistent diagnostics.
fn format_collision_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => parts.push(segment.to_string_lossy()),
            _ => return path.display().to_string(),
        }
    }
    if parts.is_empty() {
        path.display().to_string()
    } else {
        parts.join("/")
    }
}

/// Checks planned `(entry_label, local_path)` pairs for exact final and `.part` staging collisions.
fn validate_exact_collisions(entries: &[(&str, &str)]) -> Result<()> {
    let mut final_paths: HashMap<PathBuf, String> = HashMap::new();
    let mut staging_paths: HashMap<PathBuf, String> = HashMap::new();

    for &(entry_label, local_path) in entries {
        let clean_final = collision_path_key(Path::new(local_path));
        let part_path = part_file_path(local_path);
        let clean_staging = collision_path_key(&part_path);

        if let Some(first_entry) = final_paths.get(&clean_final) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_final),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: "both entries map to the same destination path".to_string(),
            });
        }

        if let Some(first_entry) = staging_paths.get(&clean_final) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_final),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: "destination path collides with staging '.part' path".to_string(),
            });
        }

        if let Some(first_entry) = final_paths.get(&clean_staging) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_staging),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: "staging '.part' path collides with destination path".to_string(),
            });
        }

        if let Some(first_entry) = staging_paths.get(&clean_staging) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_staging),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: "both entries map to the same staging '.part' path".to_string(),
            });
        }

        final_paths.insert(clean_final, entry_label.to_string());
        staging_paths.insert(clean_staging, entry_label.to_string());
    }

    Ok(())
}

/// Guard that removes a temporary filesystem probe directory and its contents when dropped.
struct ProbeDirGuard(PathBuf);

impl Drop for ProbeDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Resolved destination root state used for per-parent-directory filesystem lookup checks.
#[derive(Debug, Clone)]
struct ProbeRoot {
    /// Existing directory whose filesystem lookup rules apply to the output root.
    probe_dir: Option<PathBuf>,
    /// Canonical path of the output root when it already exists as a directory on disk.
    /// Remains `None` when the output root does not exist yet so missing children are never
    /// probed against unrelated sibling directories in an existing ancestor.
    exact_on_disk_dir: Option<PathBuf>,
}

/// Result of resolving filename equivalence for distinct component names inside one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirEquivalenceResult {
    /// For each component index `i`, the smallest index `groups[i] <= i` whose name resolves to
    /// the same directory entry under that directory's filesystem lookup rules.
    groups: Vec<usize>,
    /// Reason suffix describing the lookup rule that detected a non-exact collision.
    reason_suffix: &'static str,
    /// When `Some`, `groups` came from a partial read-only case-folding check that could not
    /// verify every candidate name. If `check_level_groups` does not report a
    /// [`IaGetError::PathCollision`], validation must fail closed with this error message.
    unverified_error: Option<String>,
}

/// Abstraction over per-directory filesystem lookup rules so [`validate_download_paths_for_dir`]
/// probes real host directories while unit tests deterministically exercise mixed-mount and
/// Unicode-normalisation directory trees.
trait DirectoryLookup {
    /// Resolves the output root for probing without creating missing directories.
    fn resolve_root(&self, dir: &Path) -> ProbeRoot;

    /// Resolves an existing, non-symlink child directory inside `parent_on_disk` for `segment`,
    /// returning `None` if it does not exist, is a symlink, or escapes `canonical_root`.
    fn resolve_child_dir(
        &self,
        canonical_root: Option<&Path>,
        parent_on_disk: &Path,
        segment: &str,
    ) -> Option<PathBuf>;

    /// Partitions `names` (distinct component names at a single directory level) into equivalence
    /// classes under `probe_dir`'s filesystem lookup rules, failing closed when distinctness
    /// cannot be verified.
    fn resolve_equivalences(
        &self,
        probe_dir: Option<&Path>,
        names: &[String],
        candidates: &[usize],
    ) -> Result<DirEquivalenceResult>;
}

/// Production [`DirectoryLookup`] backed by the host OS filesystem.
struct OsDirectoryLookup;

/// Resolves `dir` into its existing probe directory and, when `dir` itself exists as a directory,
/// its canonical on-disk path.
fn resolve_probe_root(dir: &Path) -> ProbeRoot {
    if dir.as_os_str().is_empty() {
        return ProbeRoot {
            probe_dir: None,
            exact_on_disk_dir: None,
        };
    }

    if fs::metadata(dir).map(|meta| meta.is_dir()).unwrap_or(false) {
        let resolved = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        return ProbeRoot {
            probe_dir: Some(resolved.clone()),
            exact_on_disk_dir: Some(resolved),
        };
    }

    let probe_dir = resolve_existing_probe_dir(dir)
        .map(|ancestor| fs::canonicalize(&ancestor).unwrap_or(ancestor));
    ProbeRoot {
        probe_dir,
        exact_on_disk_dir: None,
    }
}

/// Resolves `parent_dir.join(segment)` only when it is an existing, non-symlink directory
/// contained within `canonical_root`.
fn resolve_contained_child_dir(
    canonical_root: Option<&Path>,
    parent_dir: &Path,
    segment: &str,
) -> Option<PathBuf> {
    let child = parent_dir.join(segment);
    let meta = fs::symlink_metadata(&child).ok()?;
    if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
        return None;
    }
    if let Some(root) = canonical_root {
        let canonical_child = fs::canonicalize(&child).ok()?;
        if !canonical_child.starts_with(root) {
            return None;
        }
    }
    Some(child)
}

/// Creates a private empty temporary probe directory inside `parent` with RAII cleanup.
fn create_temp_probe_dir(parent: &Path) -> std::io::Result<ProbeDirGuard> {
    let pid = std::process::id();
    for _ in 0..MAX_TEMP_CASE_PROBE_ATTEMPTS {
        let seq = CASE_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".ia-get-fs-probe-{pid}-{seq}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(ProbeDirGuard(candidate)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "exhausted temporary probe directory attempts",
    ))
}

/// Selects indices in `names` that could collide under non-exact filesystem lookup rules.
///
/// Returns an empty vector when all component names are pure ASCII and have pairwise distinct
/// ASCII-lowercased forms (preserving a zero-I/O fast path).
fn select_probe_candidates(names: &[String]) -> Vec<usize> {
    if names.len() < 2 {
        return Vec::new();
    }

    if names.iter().any(|name| !name.is_ascii()) {
        return (0..names.len()).collect();
    }

    let mut counts: HashMap<String, usize> = HashMap::with_capacity(names.len());
    let keys: Vec<String> = names
        .iter()
        .map(|name| {
            let key = name.trim_end_matches(['.', ' ']).to_ascii_lowercase();
            *counts.entry(key.clone()).or_insert(0) += 1;
            key
        })
        .collect();

    keys.into_iter()
        .enumerate()
        .filter_map(|(idx, key)| (counts.get(&key).copied().unwrap_or(0) > 1).then_some(idx))
        .collect()
}

/// Returns `true` when `name` is a single normal path component safe to probe on the host OS.
fn is_probable_component_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        let base = name.split('.').next().unwrap_or(name);
        let trimmed_stem = base.trim_end_matches(|c: char| c.is_whitespace() || c == '.');
        if crate::utils::is_windows_reserved_name(trimmed_stem) {
            return false;
        }
    }

    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// Returns `true` when `name` is pure ASCII without trailing dots or spaces, so ASCII case
/// sensitivity alone completely determines filename equivalence on standard filesystems.
fn is_plain_ascii_probe_name(name: &str) -> bool {
    name.is_ascii() && !name.ends_with(['.', ' '])
}

/// Builds a fail-closed filesystem error message when filename distinctness cannot be verified.
fn unverified_probe_error(probe_dir: Option<&Path>) -> String {
    match probe_dir {
        Some(dir) => format!(
            "Destination path collision check could not verify whether candidate paths are distinct in '{}'",
            dir.display()
        ),
        None => "Destination path collision check could not verify whether candidate paths are distinct in the destination directory".to_string(),
    }
}

/// Outcome of probing candidate names inside a private temporary subdirectory.
#[derive(Debug, PartialEq, Eq)]
enum TempDirProbeOutcome {
    /// Successfully determined equivalence groups for all candidates.
    Resolved(Vec<usize>),
    /// `probe_dir` denied directory creation because it is unwritable or read-only.
    DirUnwritable,
    /// Candidate names could not be probed conclusively.
    Inconclusive,
}

/// Probes actual filename equivalence inside a private temporary directory created in `probe_dir`.
fn probe_names_in_dir(
    probe_dir: &Path,
    names: &[String],
    candidates: &[usize],
) -> TempDirProbeOutcome {
    if !candidates
        .iter()
        .all(|&idx| is_probable_component_name(&names[idx]))
    {
        return TempDirProbeOutcome::Inconclusive;
    }

    let temp_dir = match create_temp_probe_dir(probe_dir) {
        Ok(dir) => dir,
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return TempDirProbeOutcome::DirUnwritable;
        }
        Err(_) => return TempDirProbeOutcome::Inconclusive,
    };
    let mut groups: Vec<usize> = (0..names.len()).collect();

    for &idx in candidates {
        let name = &names[idx];
        let probe_path = temp_dir.0.join(name);
        let mut create_opts = fs::OpenOptions::new();
        create_opts.write(true).create_new(true);
        configure_no_follow(&mut create_opts);

        match create_opts.open(&probe_path) {
            Ok(mut file) => {
                let write_res = file.write_all(idx.to_string().as_bytes());
                drop(file);
                if write_res.is_err() {
                    return TempDirProbeOutcome::Inconclusive;
                }
                groups[idx] = idx;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut read_opts = fs::OpenOptions::new();
                read_opts.read(true);
                configure_no_follow(&mut read_opts);
                let Ok(mut file) = read_opts.open(&probe_path) else {
                    return TempDirProbeOutcome::Inconclusive;
                };
                if !file.metadata().ok().map(|m| m.is_file()).unwrap_or(false) {
                    return TempDirProbeOutcome::Inconclusive;
                }
                let mut bytes = Vec::new();
                let read_res = (&mut file).take(64).read_to_end(&mut bytes);
                drop(file);
                if read_res.is_err() || bytes.len() == 64 {
                    return TempDirProbeOutcome::Inconclusive;
                }
                let Ok(earlier_idx) = std::str::from_utf8(&bytes)
                    .ok()
                    .map(str::trim)
                    .unwrap_or("")
                    .parse::<usize>()
                else {
                    return TempDirProbeOutcome::Inconclusive;
                };
                if earlier_idx >= idx || earlier_idx >= names.len() {
                    return TempDirProbeOutcome::Inconclusive;
                }
                groups[idx] = groups[earlier_idx];
            }
            Err(_) => return TempDirProbeOutcome::Inconclusive,
        }
    }

    drop(temp_dir);
    TempDirProbeOutcome::Resolved(groups)
}

/// Fail-closed fallback filename equivalence check when live probing cannot run.
///
/// Uses read-only existing-child probing only when it can definitively decide the candidates:
/// - If read-only probing confirms `CaseInsensitive` and case-folding already detects a collision,
///   returns those equivalence groups (while keeping `unverified_error` set whenever non-ASCII or
///   unprobable names remain).
/// - If `allow_read_only_distinct` is `true`, all candidate names are pure ASCII without trailing
///   dots or spaces, and read-only probing confirms `CaseSensitive`, returns distinct groups.
/// - Otherwise fails closed with [`IaGetError::FileSystem`].
fn fallback_dir_equivalence(
    probe_dir: Option<&Path>,
    names: &[String],
    candidates: &[usize],
    allow_read_only_distinct: bool,
) -> Result<DirEquivalenceResult> {
    let all_plain_ascii = candidates
        .iter()
        .all(|&idx| is_plain_ascii_probe_name(&names[idx]));
    let read_only_sensitivity = probe_dir.and_then(probe_existing_child_case_sensitivity);

    match read_only_sensitivity {
        Some(PathCaseSensitivity::CaseInsensitive) => {
            let mut seen: HashMap<String, usize> = HashMap::with_capacity(candidates.len());
            let mut groups: Vec<usize> = (0..names.len()).collect();
            let mut has_equivalence = false;

            for &idx in candidates {
                let folded = names[idx].to_lowercase();
                let group = *seen.entry(folded).or_insert(idx);
                if group != idx {
                    has_equivalence = true;
                }
                groups[idx] = group;
            }

            if allow_read_only_distinct && all_plain_ascii {
                return Ok(DirEquivalenceResult {
                    groups,
                    reason_suffix: " on a case-insensitive filesystem",
                    unverified_error: None,
                });
            }

            if has_equivalence {
                return Ok(DirEquivalenceResult {
                    groups,
                    reason_suffix: " on a case-insensitive filesystem",
                    unverified_error: Some(unverified_probe_error(probe_dir)),
                });
            }
        }
        Some(PathCaseSensitivity::CaseSensitive) if allow_read_only_distinct && all_plain_ascii => {
            return Ok(DirEquivalenceResult {
                groups: (0..names.len()).collect(),
                reason_suffix: "",
                unverified_error: None,
            });
        }
        _ => {}
    }

    Err(IaGetError::FileSystem(unverified_probe_error(probe_dir)))
}

impl DirectoryLookup for OsDirectoryLookup {
    fn resolve_root(&self, dir: &Path) -> ProbeRoot {
        resolve_probe_root(dir)
    }

    fn resolve_child_dir(
        &self,
        canonical_root: Option<&Path>,
        parent_on_disk: &Path,
        segment: &str,
    ) -> Option<PathBuf> {
        resolve_contained_child_dir(canonical_root, parent_on_disk, segment)
    }

    fn resolve_equivalences(
        &self,
        probe_dir: Option<&Path>,
        names: &[String],
        candidates: &[usize],
    ) -> Result<DirEquivalenceResult> {
        let mut allow_read_only_distinct = false;

        if let Some(dir) = probe_dir {
            match probe_names_in_dir(dir, names, candidates) {
                TempDirProbeOutcome::Resolved(groups) => {
                    return Ok(DirEquivalenceResult {
                        groups,
                        reason_suffix: " under the destination directory's filesystem lookup rules",
                        unverified_error: None,
                    });
                }
                TempDirProbeOutcome::DirUnwritable => {
                    allow_read_only_distinct = true;
                }
                TempDirProbeOutcome::Inconclusive => {}
            }
        }

        fallback_dir_equivalence(probe_dir, names, candidates, allow_read_only_distinct)
    }
}

/// Distinguishes whether a planned path placement represents a final file or a `.part` staging
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedPathKind {
    Final,
    Staging,
}

/// Role played by a planned path item at a specific directory depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollisionRole {
    Final,
    Staging,
    Subdir,
}

impl CollisionRole {
    const fn label(self) -> &'static str {
        match self {
            Self::Final => "destination path",
            Self::Staging => "staging '.part' path",
            Self::Subdir => "directory path",
        }
    }
}

/// Normalised planned path placement (either final destination or `.part` staging file).
#[derive(Debug)]
struct PlannedPathItem<'a> {
    entry_idx: usize,
    entry_label: &'a str,
    kind: PlannedPathKind,
    segments: Vec<String>,
    display_path: String,
}

/// Per-equivalence-group occupancy state while checking a single directory level.
#[derive(Default)]
struct GroupOccupancy {
    final_item: Option<usize>,
    staging_item: Option<usize>,
    subdir_first_item: Option<usize>,
    subdir_items: Vec<usize>,
    subdir_name_ids: Vec<usize>,
}

/// Work item representing a single planned directory level to validate.
struct DirLevelWork {
    depth: usize,
    item_indices: Vec<usize>,
    probe_dir: Option<PathBuf>,
    exact_on_disk_dir: Option<PathBuf>,
    inherited_reason_suffix: Option<&'static str>,
}

/// Extracts strictly normal UTF-8 path segments from a normalised relative path.
fn normal_path_segments(path: &Path) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_str()?.to_string()),
            _ => return None,
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

/// Selects the appropriate collision reason suffix depending on whether the conflict occurred on
/// exact identical segments or via non-exact lookup equivalence at this level or an ancestor.
fn collision_reason_suffix(
    first: &PlannedPathItem<'_>,
    second: &PlannedPathItem<'_>,
    depth: usize,
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> &'static str {
    if first.segments[depth] != second.segments[depth] {
        level_reason_suffix
    } else if first.segments[..depth] != second.segments[..depth] {
        inherited_reason_suffix.unwrap_or(level_reason_suffix)
    } else {
        ""
    }
}

/// Builds a [`IaGetError::PathCollision`] for two conflicting items at `depth`.
fn build_level_collision_error(
    first: &PlannedPathItem<'_>,
    second: &PlannedPathItem<'_>,
    depth: usize,
    first_role: CollisionRole,
    second_role: CollisionRole,
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> IaGetError {
    let suffix = collision_reason_suffix(
        first,
        second,
        depth,
        level_reason_suffix,
        inherited_reason_suffix,
    );
    let local_path = match second_role {
        CollisionRole::Final | CollisionRole::Staging => second.display_path.clone(),
        CollisionRole::Subdir => second.segments[..=depth].join("/"),
    };
    let reason = match (first_role, second_role) {
        (CollisionRole::Subdir, CollisionRole::Subdir) => {
            unreachable!("matching subdirectories merge instead of colliding")
        }
        (a, b) if a == b => format!("both entries map to the same {}{suffix}", b.label()),
        (a, b) => format!("{} collides with {}{suffix}", b.label(), a.label()),
    };

    IaGetError::PathCollision {
        local_path,
        first_entry: first.entry_label.to_string(),
        second_entry: second.entry_label.to_string(),
        reason,
    }
}

/// Checks all items at a single directory level against equivalence `groups` and returns the
/// per-group occupancy (including merged subdirectory children) on success.
fn check_level_groups(
    all_items: &[PlannedPathItem<'_>],
    item_indices: &[usize],
    item_name_ids: &[usize],
    depth: usize,
    groups: &[usize],
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> Result<Vec<GroupOccupancy>> {
    let mut occupancy: Vec<GroupOccupancy> = (0..groups.len())
        .map(|_| GroupOccupancy::default())
        .collect();

    for (pos, &item_idx) in item_indices.iter().enumerate() {
        let name_id = item_name_ids[pos];
        let group_id = groups[name_id];
        let item = &all_items[item_idx];
        let occ = &mut occupancy[group_id];
        let role = if depth + 1 < item.segments.len() {
            CollisionRole::Subdir
        } else {
            match item.kind {
                PlannedPathKind::Final => CollisionRole::Final,
                PlannedPathKind::Staging => CollisionRole::Staging,
            }
        };

        for (existing_role, existing_idx) in [
            (CollisionRole::Final, occ.final_item),
            (CollisionRole::Staging, occ.staging_item),
            (CollisionRole::Subdir, occ.subdir_first_item),
        ] {
            if role == CollisionRole::Subdir && existing_role == CollisionRole::Subdir {
                continue;
            }
            if let Some(first_idx) = existing_idx {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        existing_role,
                        role,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
        }

        match role {
            CollisionRole::Subdir => {
                occ.subdir_first_item.get_or_insert(item_idx);
                occ.subdir_items.push(item_idx);
                if !occ.subdir_name_ids.contains(&name_id) {
                    occ.subdir_name_ids.push(name_id);
                }
            }
            CollisionRole::Final => {
                occ.final_item.get_or_insert(item_idx);
            }
            CollisionRole::Staging => {
                occ.staging_item.get_or_insert(item_idx);
            }
        }
    }

    Ok(occupancy)
}

/// Validates planned `(entry_label, local_path)` pairs using per-parent-directory filesystem
/// lookup rules supplied by `lookup`.
fn validate_download_paths_with_lookup(
    dir: &Path,
    entries: &[(&str, &str)],
    lookup: &impl DirectoryLookup,
) -> Result<()> {
    // Fast exact pass catches byte-identical final and `.part` collisions with zero I/O.
    validate_exact_collisions(entries)?;

    if entries.len() < 2 {
        return Ok(());
    }

    let mut all_items: Vec<PlannedPathItem<'_>> = Vec::with_capacity(entries.len() * 2);
    for (entry_idx, &(entry_label, local_path)) in entries.iter().enumerate() {
        let clean_final = collision_path_key(Path::new(local_path));
        if let Some(segments) = normal_path_segments(&clean_final) {
            all_items.push(PlannedPathItem {
                entry_idx,
                entry_label,
                kind: PlannedPathKind::Final,
                segments,
                display_path: format_collision_path(&clean_final),
            });
        }

        let part_path = part_file_path(local_path);
        let clean_staging = collision_path_key(&part_path);
        if let Some(segments) = normal_path_segments(&clean_staging) {
            all_items.push(PlannedPathItem {
                entry_idx,
                entry_label,
                kind: PlannedPathKind::Staging,
                segments,
                display_path: format_collision_path(&clean_staging),
            });
        }
    }

    if all_items.len() < 2 {
        return Ok(());
    }

    let root = lookup.resolve_root(dir);
    let canonical_root = root.exact_on_disk_dir.clone();

    let mut work_stack = vec![DirLevelWork {
        depth: 0,
        item_indices: (0..all_items.len()).collect(),
        probe_dir: root.probe_dir,
        exact_on_disk_dir: root.exact_on_disk_dir,
        inherited_reason_suffix: None,
    }];

    while let Some(level) = work_stack.pop() {
        let (Some(&first_item_idx), Some(&last_item_idx)) =
            (level.item_indices.first(), level.item_indices.last())
        else {
            continue;
        };
        if all_items[first_item_idx].entry_idx == all_items[last_item_idx].entry_idx {
            continue;
        }

        let mut name_to_id: HashMap<&str, usize> = HashMap::new();
        let mut names: Vec<String> = Vec::new();
        let mut item_name_ids: Vec<usize> = Vec::with_capacity(level.item_indices.len());

        for &item_idx in &level.item_indices {
            let seg = all_items[item_idx].segments[level.depth].as_str();
            let name_id = match name_to_id.get(seg) {
                Some(&id) => id,
                None => {
                    let id = names.len();
                    name_to_id.insert(seg, id);
                    names.push(seg.to_string());
                    id
                }
            };
            item_name_ids.push(name_id);
        }

        // First validate exact component groups (catches exact file-vs-directory and merged-parent
        // exact leaf collisions without filesystem probing).
        let exact_groups: Vec<usize> = (0..names.len()).collect();
        let mut occupancy = check_level_groups(
            &all_items,
            &level.item_indices,
            &item_name_ids,
            level.depth,
            &exact_groups,
            "",
            level.inherited_reason_suffix,
        )?;
        let mut level_reason_suffix = "";

        let candidates = select_probe_candidates(&names);
        if !candidates.is_empty() {
            let eq =
                lookup.resolve_equivalences(level.probe_dir.as_deref(), &names, &candidates)?;
            if eq
                .groups
                .iter()
                .enumerate()
                .any(|(idx, &group)| group != idx)
            {
                occupancy = check_level_groups(
                    &all_items,
                    &level.item_indices,
                    &item_name_ids,
                    level.depth,
                    &eq.groups,
                    eq.reason_suffix,
                    level.inherited_reason_suffix,
                )?;
                level_reason_suffix = eq.reason_suffix;
            }
            if let Some(err_msg) = eq.unverified_error {
                return Err(IaGetError::FileSystem(err_msg));
            }
        }

        let mut next_levels = Vec::new();
        for occ in occupancy {
            let (Some(&first_child), Some(&last_child)) =
                (occ.subdir_items.first(), occ.subdir_items.last())
            else {
                continue;
            };
            if all_items[first_child].entry_idx == all_items[last_child].entry_idx {
                continue;
            }

            let child_exact_on_disk = level.exact_on_disk_dir.as_deref().and_then(|parent_dir| {
                occ.subdir_name_ids.iter().find_map(|&name_id| {
                    lookup.resolve_child_dir(canonical_root.as_deref(), parent_dir, &names[name_id])
                })
            });
            let child_probe_dir = child_exact_on_disk
                .clone()
                .or_else(|| level.probe_dir.clone());
            let child_inherited_reason_suffix = if occ.subdir_name_ids.len() > 1 {
                Some(level_reason_suffix)
            } else {
                level.inherited_reason_suffix
            };

            next_levels.push(DirLevelWork {
                depth: level.depth + 1,
                item_indices: occ.subdir_items,
                probe_dir: child_probe_dir,
                exact_on_disk_dir: child_exact_on_disk,
                inherited_reason_suffix: child_inherited_reason_suffix,
            });
        }

        // Push in reverse order so child directories are visited in first-appearance order.
        for next in next_levels.into_iter().rev() {
            work_stack.push(next);
        }
    }

    Ok(())
}

/// Validates that no two planned entries collide on either a final destination path,
/// a `.part` staging path, or a planned directory path according to each parent directory's
/// actual filesystem lookup rules.
///
/// Probes only existing parent directories (or the nearest existing ancestor when directories do
/// not exist yet, without creating missing directories or following symlinks inside `dir`) and
/// avoids filesystem I/O when all names at a directory level are pure ASCII without case
/// conflicts.
pub fn validate_download_paths_for_dir<'a, I>(dir: &Path, entries: I) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let entries: Vec<(&'a str, &'a str)> = entries.into_iter().collect();
    validate_download_paths_with_lookup(dir, &entries, &OsDirectoryLookup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::tests::{assert_downloaded_file, run_batch_in_dir};
    use crate::downloader::{http_client_builder, RetryPolicy, ShutdownSignal};
    use crate::test_support::{md5_hex, MockResponse, MockServer, TempTestDir};

    fn assert_path_collision(
        err: IaGetError,
        expected_local: &str,
        expected_first: Option<&str>,
        expected_second: Option<&str>,
        reason_contains: Option<&str>,
        context: &str,
    ) {
        match err {
            IaGetError::PathCollision {
                local_path,
                first_entry,
                second_entry,
                reason,
            } => {
                assert_eq!(
                    local_path, expected_local,
                    "{context}: unexpected local_path"
                );
                if let Some(first) = expected_first {
                    assert_eq!(first_entry, first, "{context}: unexpected first_entry");
                }
                if let Some(second) = expected_second {
                    assert_eq!(second_entry, second, "{context}: unexpected second_entry");
                }
                if let Some(needle) = reason_contains {
                    assert!(
                        reason.contains(needle),
                        "{context}: expected reason to contain '{needle}', got '{reason}'"
                    );
                }
            }
            other => panic!("{context}: expected PathCollision, got {other:?}"),
        }
    }

    fn same_paths<'a>(paths: &[&'a str]) -> Vec<(&'a str, &'a str)> {
        paths.iter().map(|&p| (p, p)).collect()
    }

    /// Independently observes whether `dir` treats `first` and `second` as equivalent names.
    fn probe_host_dir_equivalent(dir: &Path, first: &str, second: &str) -> bool {
        let first_path = dir.join(first);
        let second_path = dir.join(second);
        fs::write(&first_path, b"probe").expect("write host equivalence probe");
        let equivalent = match fs::symlink_metadata(&second_path) {
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => panic!("unexpected error probing host equivalence: {err}"),
        };
        let _ = fs::remove_file(&first_path);
        equivalent
    }

    fn measure_host_dir_case_sensitivity(dir: &Path) -> PathCaseSensitivity {
        if probe_host_dir_equivalent(dir, "host-case-probe-file.txt", "HOST-CASE-PROBE-FILE.TXT") {
            PathCaseSensitivity::CaseInsensitive
        } else {
            PathCaseSensitivity::CaseSensitive
        }
    }

    #[cfg(unix)]
    fn with_readonly_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreWritable<'a>(&'a Path);
        impl Drop for RestoreWritable<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }

        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        let _restore = RestoreWritable(dir);
        f()
    }

    #[test]
    fn test_validate_download_paths_exact_and_case_sensitivity_modes() {
        let lookup = MockDirectoryLookup::new([
            ("/cs", MockDirRule::Exact),
            ("/ci", MockDirRule::CaseAndNormalizationInsensitive),
        ]);
        let cs = Path::new("/cs");
        let ci = Path::new("/ci");

        // Exact mode: final duplicates, normalised slash duplicates, staging collisions in both orders
        for (label, first, second, expected_local, check_entries) in [
            (
                "dup-sanitised",
                ("foo?.txt", "foo_.txt"),
                ("foo*.txt", "foo_.txt"),
                "foo_.txt",
                true,
            ),
            (
                "normalised-slash",
                ("a/file.txt", "a/file.txt"),
                ("a//file.txt", "a//file.txt"),
                "a/file.txt",
                false,
            ),
            (
                "staging-after",
                ("foo.txt", "foo.txt"),
                ("foo.txt.part", "foo.txt.part"),
                "foo.txt.part",
                true,
            ),
            (
                "staging-before",
                ("foo.txt.part", "foo.txt.part"),
                ("foo.txt", "foo.txt"),
                "foo.txt.part",
                true,
            ),
        ] {
            let err =
                validate_download_paths_with_lookup(cs, &[first, second], &lookup).unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                check_entries.then_some(first.0),
                check_entries.then_some(second.0),
                None,
                label,
            );
        }

        assert!(validate_download_paths_with_lookup(
            cs,
            &same_paths(&[
                "foo.txt",
                "foo.txt.bak",
                "a/foo.txt",
                "b/foo.txt",
                "README.md",
                "readme.md",
            ]),
            &lookup,
        )
        .is_ok());

        // Case-insensitive mode: filename/dir/staging/Unicode/sanitised case collisions
        let ci_cases = [
            (
                "filename-case",
                ("Foo.txt", "Foo.txt"),
                ("foo.txt", "foo.txt"),
                "foo.txt",
            ),
            (
                "subdir-and-filename-case",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.txt", "dir/A.txt"),
                "dir/A.txt",
            ),
            (
                "staging-then-final-upper-base",
                ("a.txt", "a.txt"),
                ("A.TXT.part", "A.TXT.part"),
                "A.TXT.part",
            ),
            (
                "staging-then-final-upper-part",
                ("a.txt", "a.txt"),
                ("a.txt.PART", "a.txt.PART"),
                "a.txt.PART",
            ),
            (
                "final-then-staging-upper-base",
                ("A.TXT.part", "A.TXT.part"),
                ("a.txt", "a.txt"),
                "a.txt.part",
            ),
            (
                "final-then-staging-upper-part",
                ("a.txt.PART", "a.txt.PART"),
                ("a.txt", "a.txt"),
                "a.txt.part",
            ),
            (
                "nested-staging-case",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.TXT.PART", "dir/A.TXT.PART"),
                "dir/A.TXT.PART",
            ),
            (
                "unicode-case",
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("CAF\u{00c9}.txt", "CAF\u{00c9}.txt"),
                "CAF\u{00c9}.txt",
            ),
            (
                "sanitised-case",
                ("foo?.txt", "foo_.txt"),
                ("FOO*.TXT", "FOO_.TXT"),
                "FOO_.TXT",
            ),
        ];

        for (label, first, second, expected_local) in ci_cases {
            let err =
                validate_download_paths_with_lookup(ci, &[first, second], &lookup).unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first.0),
                Some(second.0),
                Some("filesystem lookup rules"),
                label,
            );
        }

        // Exact duplicates still report the exact reason even on a case-insensitive filesystem.
        let exact_err = validate_download_paths_with_lookup(
            ci,
            &[("foo?.txt", "foo_.txt"), ("foo*.txt", "foo_.txt")],
            &lookup,
        )
        .unwrap_err();
        assert_path_collision(
            exact_err,
            "foo_.txt",
            None,
            None,
            Some("both entries map to the same destination path"),
            "exact-in-ci",
        );

        // Case-sensitive mode allows all case-only differences.
        assert!(validate_download_paths_with_lookup(
            cs,
            &same_paths(&[
                "Foo.txt",
                "foo.txt",
                "Dir/a.txt",
                "dir/A.txt",
                "a.txt",
                "A.TXT.part",
                "a.txt.PART",
                "Caf\u{00e9}.txt",
                "CAF\u{00c9}.txt",
            ]),
            &lookup,
        )
        .is_ok());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_pre_write_validation_aborts_before_http_requests() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // 1. Colliding final/staging paths abort before creating output dir or sending HTTP.
        let cases: &[(&str, Vec<(&str, &str)>)] = &[
            (
                "duplicate-final",
                vec![("/a", "foo_.txt"), ("/b", "foo_.txt")],
            ),
            (
                "staging-then-final",
                vec![("/a", "foo.txt"), ("/b", "foo.txt.part")],
            ),
            (
                "final-then-staging",
                vec![("/a", "foo.txt.part"), ("/b", "foo.txt")],
            ),
        ];

        for (label, entries) in cases {
            let server =
                MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
            let sandbox = TempTestDir::new(&format!("collision-{label}"));
            let target_dir = sandbox.join("not-created-yet");

            let files: Vec<(String, String, Option<String>)> = entries
                .iter()
                .map(|(url_path, local_path)| {
                    (server.url(url_path), (*local_path).to_string(), None)
                })
                .collect();
            let total = files.len();

            let err = run_batch_in_dir(
                &client,
                &target_dir,
                files,
                total,
                &RetryPolicy::default(),
                &signal,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, IaGetError::PathCollision { .. }),
                "case {label}: expected PathCollision, got {err:?}"
            );
            assert!(
                server.abort().await.is_empty(),
                "case {label}: expected 0 HTTP requests on collision"
            );
            assert!(
                !target_dir.exists(),
                "case {label}: colliding batch must fail before creating output directory"
            );
        }

        // 2. Fail-closed unprobable overlong Unicode candidates abort before any HTTP transfer.
        let sandbox = TempTestDir::new("failclosed-aborts-transfers");
        let server = MockServer::start(vec![
            MockResponse::ok(b"must never be requested"),
            MockResponse::ok(b"must never be requested"),
        ])
        .await;
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        let err = run_batch_in_dir(
            &client,
            sandbox.path(),
            vec![
                (server.url("/nfc"), long_nfc, None),
                (server.url("/nfd"), long_nfd, None),
            ],
            2,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, IaGetError::FileSystem(ref msg) if msg.contains("could not verify")),
            "expected fail-closed FileSystem error, got {err:?}"
        );
        assert!(
            server.abort().await.is_empty(),
            "fail-closed validation error must abort before any HTTP transfer requests"
        );
        assert_no_probe_dirs_remaining(sandbox.path());
    }

    #[test]
    fn test_probe_existing_child_and_validate_for_dir_match_host_filesystem() {
        let oracle_dir = TempTestDir::new("case-oracle");
        let expected = measure_host_dir_case_sensitivity(oracle_dir.path());

        // 1. Empty existing directory yields None for read-only child probe.
        let empty_dir = TempTestDir::new("case-empty");
        assert_eq!(
            probe_existing_child_case_sensitivity(empty_dir.path()),
            None
        );
        assert_eq!(fs::read_dir(empty_dir.path()).unwrap().count(), 0);

        // 2. Non-empty existing directory detects host sensitivity without writes.
        let populated_dir = TempTestDir::new("case-populated");
        populated_dir.write("Existing.txt", b"data");
        assert_eq!(
            probe_existing_child_case_sensitivity(populated_dir.path()),
            Some(expected)
        );
        assert_eq!(fs::read_dir(populated_dir.path()).unwrap().count(), 1);

        // 3. Not-yet-created nested directory resolves nearest existing ancestor without creating it.
        let missing_dir = empty_dir.join("not-created-yet/nested");
        assert_eq!(
            resolve_existing_probe_dir(&missing_dir).as_deref(),
            Some(empty_dir.path())
        );
        assert!(!empty_dir.join("not-created-yet").exists());

        // 4. Read-only directory with an existing child still probes cleanly on Unix.
        #[cfg(unix)]
        {
            let ro_dir = TempTestDir::new("case-readonly");
            ro_dir.write("Sample.txt", b"ro");
            let detected = with_readonly_dir(ro_dir.path(), || {
                probe_existing_child_case_sensitivity(ro_dir.path())
            });
            assert_eq!(detected, Some(expected));
        }

        // 5. validate_download_paths_for_dir matches host sensitivity for case-only pairs.
        for pair in [["Foo.txt", "foo.txt"], ["a.txt", "A.TXT.part"]] {
            let result = validate_download_paths_for_dir(&missing_dir, same_paths(&pair));
            match expected {
                PathCaseSensitivity::CaseSensitive => assert!(
                    result.is_ok(),
                    "case-sensitive host should allow {pair:?}, got {result:?}"
                ),
                PathCaseSensitivity::CaseInsensitive => assert!(
                    matches!(result, Err(IaGetError::PathCollision { .. })),
                    "case-insensitive host should reject {pair:?}, got {result:?}"
                ),
            }
        }
        assert!(!empty_dir.join("not-created-yet").exists());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_case_only_entries_follow_host_filesystem() {
        let sandbox = TempTestDir::new("case-download-host");
        let expected = measure_host_dir_case_sensitivity(sandbox.path());
        let target_dir = sandbox.join("not-created-yet");

        let payload_upper = b"upper-file-contents";
        let payload_lower = b"lower-file-contents";
        let server = MockServer::start(vec![
            MockResponse::ok(payload_upper),
            MockResponse::ok(payload_lower),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let files = vec![
            (
                server.url("/upper"),
                "Foo.txt".to_string(),
                Some(md5_hex(payload_upper)),
            ),
            (
                server.url("/lower"),
                "foo.txt".to_string(),
                Some(md5_hex(payload_lower)),
            ),
        ];

        let result = run_batch_in_dir(
            &client,
            &target_dir,
            files,
            2,
            &RetryPolicy::default(),
            &signal,
        )
        .await;

        match expected {
            PathCaseSensitivity::CaseInsensitive => {
                assert!(matches!(
                    result.unwrap_err(),
                    IaGetError::PathCollision { .. }
                ));
                assert!(server.abort().await.is_empty());
                assert!(!target_dir.exists());
            }
            PathCaseSensitivity::CaseSensitive => {
                let summary = result.expect("case-sensitive volume should download both files");
                assert_eq!(summary.ok, 2);
                assert!(summary.failed.is_empty());
                assert_downloaded_file(&target_dir, "Foo.txt", payload_upper);
                assert_downloaded_file(&target_dir, "foo.txt", payload_lower);
                assert_eq!(server.finish().await.len(), 2);
            }
        }
    }

    /// Lookup rule simulated by [`MockDirectoryLookup`] for deterministic per-directory tests.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockDirRule {
        /// Case-sensitive and Unicode-normalisation-sensitive (for example Linux ext4).
        Exact,
        /// ASCII case-insensitive (for example a FAT32/SMB mount).
        CaseInsensitiveAscii,
        /// Case-sensitive, Unicode NFC/NFD normalisation-insensitive (for example case-sensitive
        /// APFS/HFS+).
        NormalizationInsensitive,
        /// Both Unicode case-folding (including `ß` -> `ss`) and NFC/NFD normalisation-insensitive
        /// (for example default APFS or ext4 `+F`).
        CaseAndNormalizationInsensitive,
        /// Simulates an inconclusive filesystem probe that cannot verify distinctness and must
        /// fail closed.
        InconclusiveProbe,
    }

    impl MockDirRule {
        fn canonical_key(self, name: &str) -> String {
            let normalize_nfc = |s: &str| {
                s.replace("e\u{0301}", "\u{00e9}")
                    .replace("E\u{0301}", "\u{00c9}")
                    .replace("a\u{030a}", "\u{00e5}")
                    .replace("A\u{030a}", "\u{00c5}")
            };

            match self {
                MockDirRule::Exact | MockDirRule::InconclusiveProbe => name.to_string(),
                MockDirRule::CaseInsensitiveAscii => name.to_ascii_lowercase(),
                MockDirRule::NormalizationInsensitive => normalize_nfc(name),
                MockDirRule::CaseAndNormalizationInsensitive => {
                    normalize_nfc(name).replace('\u{00df}', "ss").to_lowercase()
                }
            }
        }
    }

    /// Deterministic in-memory [`DirectoryLookup`] simulating mixed-mount directory trees and
    /// recording every probed directory path.
    struct MockDirectoryLookup {
        dirs: HashMap<PathBuf, MockDirRule>,
        probed_dirs: std::cell::RefCell<Vec<PathBuf>>,
    }

    impl MockDirectoryLookup {
        fn new(entries: impl IntoIterator<Item = (&'static str, MockDirRule)>) -> Self {
            Self {
                dirs: entries
                    .into_iter()
                    .map(|(path, rule)| (PathBuf::from(path), rule))
                    .collect(),
                probed_dirs: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn probed(&self) -> Vec<PathBuf> {
            self.probed_dirs.borrow().clone()
        }

        fn clear_probed(&self) {
            self.probed_dirs.borrow_mut().clear();
        }
    }

    impl DirectoryLookup for MockDirectoryLookup {
        fn resolve_root(&self, dir: &Path) -> ProbeRoot {
            if self.dirs.contains_key(dir) {
                return ProbeRoot {
                    probe_dir: Some(dir.to_path_buf()),
                    exact_on_disk_dir: Some(dir.to_path_buf()),
                };
            }

            let ancestor = dir
                .ancestors()
                .find(|candidate| self.dirs.contains_key(*candidate))
                .map(Path::to_path_buf);
            ProbeRoot {
                probe_dir: ancestor,
                exact_on_disk_dir: None,
            }
        }

        fn resolve_child_dir(
            &self,
            _canonical_root: Option<&Path>,
            parent_on_disk: &Path,
            segment: &str,
        ) -> Option<PathBuf> {
            let exact = parent_on_disk.join(segment);
            if self.dirs.contains_key(&exact) {
                return Some(exact);
            }

            let parent_rule = self
                .dirs
                .get(parent_on_disk)
                .copied()
                .unwrap_or(MockDirRule::Exact);
            let target_key = parent_rule.canonical_key(segment);
            self.dirs.keys().find_map(|candidate| {
                if candidate.parent() == Some(parent_on_disk) {
                    let child_name = candidate.file_name()?.to_str()?;
                    if parent_rule.canonical_key(child_name) == target_key {
                        return Some(candidate.clone());
                    }
                }
                None
            })
        }

        fn resolve_equivalences(
            &self,
            probe_dir: Option<&Path>,
            names: &[String],
            candidates: &[usize],
        ) -> Result<DirEquivalenceResult> {
            let dir = probe_dir.expect("mock lookup expects a resolved probe_dir");
            self.probed_dirs.borrow_mut().push(dir.to_path_buf());
            let rule = self.dirs.get(dir).copied().unwrap_or(MockDirRule::Exact);
            if rule == MockDirRule::InconclusiveProbe {
                return Err(IaGetError::FileSystem(unverified_probe_error(Some(dir))));
            }

            let mut groups: Vec<usize> = (0..names.len()).collect();
            let mut seen: HashMap<String, usize> = HashMap::new();
            for &idx in candidates {
                let key = rule.canonical_key(&names[idx]);
                let group = *seen.entry(key).or_insert(idx);
                groups[idx] = group;
            }

            Ok(DirEquivalenceResult {
                groups,
                reason_suffix: " under the destination directory's filesystem lookup rules",
                unverified_error: None,
            })
        }
    }

    #[test]
    fn test_per_parent_lookup_mixed_mounts_and_merged_parents() {
        // 1. Case-insensitive subdir under case-sensitive root
        let cs_root = MockDirectoryLookup::new([
            ("/root", MockDirRule::Exact),
            ("/root/sub", MockDirRule::CaseInsensitiveAscii),
        ]);
        let root = Path::new("/root");

        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["Foo.txt", "foo.txt"]),
            &cs_root
        )
        .is_ok());
        assert_eq!(cs_root.probed(), vec![PathBuf::from("/root")]);

        for (label, first, second, expected_local) in [
            (
                "final-vs-final",
                "sub/Foo.txt",
                "sub/foo.txt",
                "sub/foo.txt",
            ),
            (
                "staging-vs-final",
                "sub/a.txt",
                "sub/A.TXT.part",
                "sub/A.TXT.part",
            ),
            (
                "final-vs-staging",
                "sub/A.TXT.part",
                "sub/a.txt",
                "sub/a.txt.part",
            ),
            (
                "missing-nested-under-submount",
                "sub/newdir/Foo.txt",
                "sub/newdir/foo.txt",
                "sub/newdir/foo.txt",
            ),
        ] {
            cs_root.clear_probed();
            let err =
                validate_download_paths_with_lookup(root, &same_paths(&[first, second]), &cs_root)
                    .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                Some("filesystem lookup rules"),
                label,
            );
            assert_eq!(
                cs_root.probed(),
                vec![PathBuf::from("/root/sub")],
                "case {label}: expected probe inside /root/sub"
            );
        }

        // 2. Case-sensitive subdir under case-insensitive root (including merged parent spellings)
        let ci_root = MockDirectoryLookup::new([
            ("/root", MockDirRule::CaseInsensitiveAscii),
            ("/root/sub", MockDirRule::Exact),
        ]);

        let root_err = validate_download_paths_with_lookup(
            root,
            &same_paths(&["Foo.txt", "foo.txt"]),
            &ci_root,
        )
        .unwrap_err();
        assert_path_collision(root_err, "foo.txt", None, None, None, "ci-root");

        ci_root.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["sub/Foo.txt", "sub/foo.txt"]),
            &ci_root,
        )
        .is_ok());
        assert_eq!(ci_root.probed(), vec![PathBuf::from("/root/sub")]);

        // Equivalent parent spellings (`Sub` and `sub`) merge onto the existing case-sensitive `/root/sub`.
        ci_root.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["Sub/Foo.txt", "sub/foo.txt"]),
            &ci_root,
        )
        .is_ok());
        assert_eq!(
            ci_root.probed(),
            vec![PathBuf::from("/root"), PathBuf::from("/root/sub")]
        );

        let merged_leaf_err = validate_download_paths_with_lookup(
            root,
            &same_paths(&["Sub/Foo.txt", "sub/Foo.txt"]),
            &ci_root,
        )
        .unwrap_err();
        assert_path_collision(
            merged_leaf_err,
            "sub/Foo.txt",
            Some("Sub/Foo.txt"),
            Some("sub/Foo.txt"),
            Some("filesystem lookup rules"),
            "merged-parent-identical-leaf",
        );

        // 3. Merging case-equivalent parent subdirectories on a CI volume
        let ci = MockDirectoryLookup::new([("/ci", MockDirRule::CaseInsensitiveAscii)]);
        for (label, first, second, expected_local) in [
            (
                "dir-case-and-file-case",
                "Dir/a.txt",
                "dir/A.txt",
                "dir/A.txt",
            ),
            (
                "dir-case-and-staging-upper-part",
                "Dir/a.txt",
                "dir/a.txt.PART",
                "dir/a.txt.PART",
            ),
            (
                "dir-case-and-staging-before-final",
                "Dir/a.txt.PART",
                "dir/a.txt",
                "dir/a.txt.part",
            ),
        ] {
            let err = validate_download_paths_with_lookup(
                Path::new("/ci"),
                &same_paths(&[first, second]),
                &ci,
            )
            .unwrap_err();
            assert_path_collision(err, expected_local, Some(first), Some(second), None, label);
        }
        assert!(validate_download_paths_with_lookup(
            Path::new("/ci"),
            &same_paths(&["Dir/a.txt", "dir/b.txt"]),
            &ci,
        )
        .is_ok());
    }

    #[test]
    fn test_per_parent_lookup_unicode_nfc_nfd_and_folding_equivalence() {
        let lookup = MockDirectoryLookup::new([
            ("/apfs-cs", MockDirRule::NormalizationInsensitive),
            ("/ext4", MockDirRule::Exact),
            ("/apfs-ci", MockDirRule::CaseAndNormalizationInsensitive),
        ]);

        // Case-sensitive but normalisation-insensitive directory allows ASCII case differences.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &same_paths(&["Foo.txt", "foo.txt"]),
            &lookup,
        )
        .is_ok());

        for (label, first, second, expected_local) in [
            (
                "nfc-vs-nfd-final",
                "Caf\u{00e9}.txt",
                "Cafe\u{0301}.txt",
                "Cafe\u{0301}.txt",
            ),
            (
                "nfc-vs-nfd-staging-after",
                "Caf\u{00e9}.txt",
                "Cafe\u{0301}.txt.part",
                "Cafe\u{0301}.txt.part",
            ),
            (
                "nfd-staging-before-nfc",
                "Cafe\u{0301}.txt.part",
                "Caf\u{00e9}.txt",
                "Caf\u{00e9}.txt.part",
            ),
            (
                "nfc-vs-nfd-parent-dir",
                "Dir_Caf\u{00e9}/a.txt",
                "Dir_Cafe\u{0301}/a.txt",
                "Dir_Cafe\u{0301}/a.txt",
            ),
        ] {
            let pair = same_paths(&[first, second]);
            let err = validate_download_paths_with_lookup(Path::new("/apfs-cs"), &pair, &lookup)
                .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                Some("filesystem lookup rules"),
                label,
            );
            assert!(
                validate_download_paths_with_lookup(Path::new("/ext4"), &pair, &lookup).is_ok(),
                "case {label}: normalisation-sensitive directory should allow {pair:?}"
            );
        }

        // Equivalent NFC/NFD parent directories with distinct child filenames are allowed.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &same_paths(&["Dir_Caf\u{00e9}/a.txt", "Dir_Cafe\u{0301}/b.txt"]),
            &lookup,
        )
        .is_ok());

        // Non-ASCII entry colliding with a pure-ASCII entry under Unicode full case folding.
        let folding_err = validate_download_paths_with_lookup(
            Path::new("/apfs-ci"),
            &same_paths(&["Stra\u{00df}e.txt", "STRASSE.TXT"]),
            &lookup,
        )
        .unwrap_err();
        assert_path_collision(folding_err, "STRASSE.TXT", None, None, None, "eszett-fold");
    }

    #[test]
    fn test_per_parent_lookup_file_vs_directory_collisions() {
        let lookup = MockDirectoryLookup::new([
            ("/exact", MockDirRule::Exact),
            ("/ci", MockDirRule::CaseInsensitiveAscii),
        ]);

        // Exact file-vs-directory and staging-vs-directory collisions fail with zero probing.
        for (first, second, expected_local) in [
            ("foo", "foo/bar.txt", "foo"),
            ("foo/bar.txt", "foo", "foo"),
            ("foo", "foo.part/bar.txt", "foo.part"),
            ("foo.part/bar.txt", "foo", "foo.part"),
        ] {
            lookup.clear_probed();
            let err = validate_download_paths_with_lookup(
                Path::new("/exact"),
                &same_paths(&[first, second]),
                &lookup,
            )
            .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                None,
                "exact-fvd",
            );
            assert!(
                lookup.probed().is_empty(),
                "exact file-vs-dir collision must not require filesystem probing"
            );
        }

        // Case-only file-vs-directory collision is allowed on exact dir and rejected on CI dir.
        assert!(validate_download_paths_with_lookup(
            Path::new("/exact"),
            &same_paths(&["foo", "FOO/bar.txt"]),
            &lookup,
        )
        .is_ok());

        for (first, second, expected_local) in [
            ("foo", "FOO/bar.txt", "FOO"),
            ("FOO/bar.txt", "foo", "foo"),
            ("foo", "FOO.PART/bar.txt", "FOO.PART"),
        ] {
            let err = validate_download_paths_with_lookup(
                Path::new("/ci"),
                &same_paths(&[first, second]),
                &lookup,
            )
            .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                None,
                "ci-fvd",
            );
        }
    }

    #[test]
    fn test_per_parent_lookup_zero_io_fast_path_and_missing_root_isolation() {
        let lookup = MockDirectoryLookup::new([
            ("/ancestor", MockDirRule::Exact),
            ("/ancestor/sub", MockDirRule::CaseInsensitiveAscii),
        ]);

        // Pure-ASCII batch with no ASCII case duplicates performs zero probing.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor"),
            &same_paths(&["a.txt", "b.txt", "sub/c.txt", "sub/d.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Single non-ASCII entry performs zero probing.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor"),
            &same_paths(&["Caf\u{00e9}.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Missing output root (`/ancestor/missing`) must inherit `/ancestor`, not sibling `/ancestor/sub`.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor/missing"),
            &same_paths(&["sub/Foo.txt", "sub/foo.txt"]),
            &lookup,
        )
        .is_ok());
        assert_eq!(lookup.probed(), vec![PathBuf::from("/ancestor")]);
    }

    /// Asserts that `dir` contains no leftover `.ia-get-fs-probe-*` entries.
    fn assert_no_probe_dirs_remaining(dir: &Path) {
        for entry in fs::read_dir(dir).expect("read directory for probe cleanup check") {
            let name = entry.expect("dir entry").file_name();
            let name_str = name.to_string_lossy();
            assert!(
                !name_str.starts_with(".ia-get-fs-probe-"),
                "leftover temporary probe directory found in {}: {}",
                dir.display(),
                name_str
            );
        }
    }

    fn assert_matches_host_case(result: &Result<()>, host_case: PathCaseSensitivity, ctx: &str) {
        match host_case {
            PathCaseSensitivity::CaseSensitive => {
                assert!(
                    result.is_ok(),
                    "{ctx}: CS host should allow, got {result:?}"
                )
            }
            PathCaseSensitivity::CaseInsensitive => assert!(
                matches!(result, Err(IaGetError::PathCollision { .. })),
                "{ctx}: CI host should reject, got {result:?}"
            ),
        }
    }

    fn assert_matches_host_nfc(result: &Result<()>, host_nfc_nfd: bool, ctx: &str) {
        if host_nfc_nfd {
            assert!(
                matches!(result, Err(IaGetError::PathCollision { .. })),
                "{ctx}: normalisation-insensitive host should reject, got {result:?}"
            );
        } else {
            assert!(
                result.is_ok(),
                "{ctx}: normalisation-sensitive host should allow, got {result:?}"
            );
        }
    }

    #[test]
    fn test_real_host_probe_unicode_cleanup_symlink_safety_readonly_and_overlong_fallback() {
        let sandbox = TempTestDir::new("real-host-per-parent-probe");
        let host_case = measure_host_dir_case_sensitivity(sandbox.path());
        let host_nfc_nfd = probe_host_dir_equivalent(
            sandbox.path(),
            "host-nfc-probe-caf\u{00e9}.txt",
            "host-nfc-probe-cafe\u{0301}.txt",
        );

        // 1. NFC vs NFD final and `.part` pairs match host semantics and leave no probe dirs.
        for pair in [
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"],
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt.part"],
        ] {
            let result = validate_download_paths_for_dir(sandbox.path(), same_paths(&pair));
            assert_matches_host_nfc(&result, host_nfc_nfd, "real-host-nfc");
            assert_no_probe_dirs_remaining(sandbox.path());
        }

        // 2. Missing output dir is never created and leaves no probe dirs in its parent.
        let missing_dir = sandbox.join("never-created/nested");
        let case_res =
            validate_download_paths_for_dir(&missing_dir, same_paths(&["Foo.txt", "foo.txt"]));
        assert_matches_host_case(&case_res, host_case, "missing-dir");
        assert!(!sandbox.join("never-created").exists());
        assert_no_probe_dirs_remaining(sandbox.path());

        // 3. Overlong ASCII & Unicode names fail closed in both empty and populated dirs, and via prefixed probe.
        let long_lower = "a".repeat(300);
        let long_upper = "A".repeat(300);
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        assert!(matches!(
            validate_download_paths_for_dir(
                sandbox.path(),
                same_paths(&[long_lower.as_str(), long_upper.as_str()])
            ),
            Err(IaGetError::FileSystem(_))
        ));
        assert_no_probe_dirs_remaining(sandbox.path());

        let populated = TempTestDir::new("overlong-populated-failclosed");
        populated.write("Sample.txt", b"existing");
        let ascii_pop = validate_download_paths_for_dir(
            populated.path(),
            same_paths(&[long_lower.as_str(), long_upper.as_str()]),
        );
        match host_case {
            PathCaseSensitivity::CaseSensitive => {
                assert!(matches!(ascii_pop, Err(IaGetError::FileSystem(_))));
            }
            PathCaseSensitivity::CaseInsensitive => {
                assert!(matches!(
                    ascii_pop,
                    Err(IaGetError::PathCollision { .. } | IaGetError::FileSystem(_))
                ));
            }
        }
        assert!(matches!(
            validate_download_paths_for_dir(
                populated.path(),
                same_paths(&[long_nfc.as_str(), long_nfd.as_str()])
            ),
            Err(IaGetError::FileSystem(_))
        ));
        assert_no_probe_dirs_remaining(populated.path());

        // 4. Symlinks inside output_dir are never followed for probing; read-only dirs preserve
        // zero-I/O fast path and fail closed when candidates cannot be verified.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let external_target = TempTestDir::new("symlink-probe-external");
            symlink(external_target.path(), sandbox.join("linked_sub")).unwrap();
            let link_res = with_readonly_dir(external_target.path(), || {
                validate_download_paths_for_dir(
                    sandbox.path(),
                    same_paths(&["linked_sub/Foo.txt", "linked_sub/foo.txt"]),
                )
            });
            assert_matches_host_case(&link_res, host_case, "symlink-external");
            assert_eq!(fs::read_dir(external_target.path()).unwrap().count(), 0);
            assert_no_probe_dirs_remaining(sandbox.path());

            // Populated read-only dir: zero-I/O ASCII ok, case-only via child probe, Unicode & trailing-dot fail closed.
            let ro_populated = TempTestDir::new("ro-probe-populated");
            ro_populated.write("Sample.txt", b"ro");
            let (pop_distinct, pop_case, pop_unicode, pop_dot) =
                with_readonly_dir(ro_populated.path(), || {
                    (
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["alpha.txt", "beta.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["Foo.txt", "foo.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["foo.", "foo"]),
                        ),
                    )
                });
            assert!(pop_distinct.is_ok());
            assert_matches_host_case(&pop_case, host_case, "ro-populated-case");
            assert!(matches!(pop_unicode, Err(IaGetError::FileSystem(_))));
            assert!(matches!(pop_dot, Err(IaGetError::FileSystem(_))));
            assert_no_probe_dirs_remaining(ro_populated.path());

            // Empty read-only dir: zero-I/O ASCII ok; case-only and Unicode fail closed.
            let ro_empty = TempTestDir::new("ro-probe-empty");
            let (empty_distinct, empty_case, empty_unicode) =
                with_readonly_dir(ro_empty.path(), || {
                    (
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["alpha.txt", "beta.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["Foo.txt", "foo.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"]),
                        ),
                    )
                });
            assert!(empty_distinct.is_ok());
            assert!(matches!(empty_case, Err(IaGetError::FileSystem(_))));
            assert!(matches!(empty_unicode, Err(IaGetError::FileSystem(_))));
            assert_no_probe_dirs_remaining(ro_empty.path());
        }
    }

    #[test]
    fn test_temp_dir_probe_and_fallback_dir_equivalence() {
        let sandbox = TempTestDir::new("temp-dir-probe-and-fallback");
        let host_case = measure_host_dir_case_sensitivity(sandbox.path());

        // 1. Direct temp-subdir probe detects collisions and distinctness with clean teardown.
        let colliding_names = vec!["same.txt".to_string(), "same.txt".to_string()];
        assert_eq!(
            probe_names_in_dir(sandbox.path(), &colliding_names, &[0, 1]),
            TempDirProbeOutcome::Resolved(vec![0, 0])
        );
        assert_no_probe_dirs_remaining(sandbox.path());

        let distinct_names = vec!["alpha.txt".to_string(), "beta.txt".to_string()];
        assert_eq!(
            probe_names_in_dir(sandbox.path(), &distinct_names, &[0, 1]),
            TempDirProbeOutcome::Resolved(vec![0, 1])
        );
        assert_no_probe_dirs_remaining(sandbox.path());

        // 2. Read-only directory routes mkdir PermissionDenied to DirUnwritable.
        #[cfg(unix)]
        {
            let ro_dir = TempTestDir::new("temp-dir-probe-ro");
            let case_names = vec!["Foo.txt".to_string(), "foo.txt".to_string()];
            let outcome = with_readonly_dir(ro_dir.path(), || {
                probe_names_in_dir(ro_dir.path(), &case_names, &[0, 1])
            });
            assert_eq!(outcome, TempDirProbeOutcome::DirUnwritable);
            assert_no_probe_dirs_remaining(ro_dir.path());
        }

        // 3. fallback_dir_equivalence fails closed when dir is absent, empty, non-ASCII, or
        // allow_read_only_distinct is false on a case-sensitive volume.
        let case_names = vec!["Foo.txt".to_string(), "foo.txt".to_string()];
        assert!(matches!(
            fallback_dir_equivalence(None, &case_names, &[0, 1], true),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            fallback_dir_equivalence(Some(sandbox.path()), &case_names, &[0, 1], true),
            Err(IaGetError::FileSystem(_))
        ));

        sandbox.write("Sample.txt", b"existing");
        let strict = fallback_dir_equivalence(Some(sandbox.path()), &case_names, &[0, 1], false);
        let relaxed = fallback_dir_equivalence(Some(sandbox.path()), &case_names, &[0, 1], true);
        match host_case {
            PathCaseSensitivity::CaseSensitive => {
                assert!(matches!(strict, Err(IaGetError::FileSystem(_))));
                let eq = relaxed.expect("read-only CS fallback allows plain ASCII");
                assert_eq!(eq.groups, vec![0, 1]);
                assert_eq!(eq.unverified_error, None);
            }
            PathCaseSensitivity::CaseInsensitive => {
                let eq = strict.expect("CI fallback detects ASCII collision");
                assert_eq!(eq.groups, vec![0, 0]);
                assert!(eq.unverified_error.is_some());
                let relaxed_eq = relaxed.expect("read-only CI fallback detects plain ASCII");
                assert_eq!(relaxed_eq.groups, vec![0, 0]);
                assert_eq!(relaxed_eq.unverified_error, None);
            }
        }
    }

    #[test]
    fn test_per_parent_lookup_inconclusive_probe_fails_closed() {
        let lookup = MockDirectoryLookup::new([
            ("/exact", MockDirRule::Exact),
            ("/exact/unverifiable", MockDirRule::InconclusiveProbe),
            ("/unverifiable", MockDirRule::InconclusiveProbe),
        ]);

        // Zero-I/O fast path still succeeds on an unverifiable directory for non-colliding ASCII.
        assert!(validate_download_paths_with_lookup(
            Path::new("/unverifiable"),
            &same_paths(&["alpha.txt", "beta.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Unicode and case-only candidates fail closed when the probe is inconclusive.
        for entries in [
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"],
            ["Foo.txt", "foo.txt"],
        ] {
            lookup.clear_probed();
            let err = validate_download_paths_with_lookup(
                Path::new("/unverifiable"),
                &same_paths(&entries),
                &lookup,
            )
            .unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(ref msg) if msg.contains("could not verify")),
                "inconclusive probe must fail closed with IaGetError::FileSystem, got {err:?}"
            );
            assert_eq!(lookup.probed(), vec![PathBuf::from("/unverifiable")]);
        }

        // Inconclusive subdirectory under an exact root fails closed for candidates in that subdir.
        lookup.clear_probed();
        let subdir_err = validate_download_paths_with_lookup(
            Path::new("/exact"),
            &same_paths(&[
                "unverifiable/Caf\u{00e9}.txt",
                "unverifiable/Cafe\u{0301}.txt",
            ]),
            &lookup,
        )
        .unwrap_err();
        assert!(
            matches!(subdir_err, IaGetError::FileSystem(ref msg) if msg.contains("/exact/unverifiable")),
            "inconclusive child directory probe must fail closed, got {subdir_err:?}"
        );
        assert_eq!(lookup.probed(), vec![PathBuf::from("/exact/unverifiable")]);
    }
}
