//! Filesystem containment, symlink-safe opens, output directory preparation, and atomic `.part` promotion.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::error::IaGetError;
use crate::Result;

/// Canonicalises the destination root directory and verifies that it is a directory.
pub(crate) fn resolve_destination_root(root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(root)?;
    if !fs::metadata(&canonical)?.is_dir() {
        return Err(IaGetError::FileSystem(format!(
            "Destination root '{}' is not a directory",
            root.display()
        )));
    }
    Ok(canonical)
}

/// Validates a target directory (optionally creating missing directories when
/// `create_if_missing` is `true`) and returns its canonical path.
///
/// When `create_if_missing` is `false`, this helper is strictly read-only: it never creates or
/// modifies directories and verifies search access via `canonical.join(".")`.
pub(crate) fn validate_or_prepare_dir(
    path: &Path,
    label_cap: &str,
    label_lower: &str,
    create_if_missing: bool,
) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(format!(
            "{label_cap} directory path must not be empty"
        )));
    }

    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let canonical = resolve_destination_root(path)?;
            if !create_if_missing {
                // Confirm search access into the directory itself without modifying anything.
                fs::metadata(canonical.join(".")).map_err(|err| {
                    IaGetError::FileSystem(format!(
                        "Failed to access {label_lower} directory '{}': {}",
                        path.display(),
                        err
                    ))
                })?;
            }
            Ok(canonical)
        }
        Ok(_) => Err(IaGetError::FileSystem(format!(
            "{label_cap} path '{}' exists and is not a directory",
            path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(IaGetError::FileSystem(format!(
                    "{label_cap} path '{}' is a broken symlink",
                    path.display()
                )));
            }

            if !create_if_missing {
                return Err(IaGetError::FileSystem(format!(
                    "{label_cap} directory '{}' does not exist",
                    path.display()
                )));
            }

            fs::create_dir_all(path).map_err(|create_err| {
                IaGetError::FileSystem(format!(
                    "Failed to create {label_lower} directory '{}': {}",
                    path.display(),
                    create_err
                ))
            })?;

            resolve_destination_root(path)
        }
        Err(err) => Err(IaGetError::FileSystem(format!(
            "Failed to access {label_lower} directory '{}': {}",
            path.display(),
            err
        ))),
    }
}

/// Validates and prepares the destination output directory, creating missing directories
/// when needed and returning its canonical path.
pub fn prepare_output_dir(path: &Path) -> Result<PathBuf> {
    validate_or_prepare_dir(path, "Output", "output", true)
}

/// Validates that a destination path is a non-empty relative path made solely of normal segments.
pub(super) fn validate_relative_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(
            "Destination path must not be empty".to_string(),
        ));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => clean.push(segment),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing unsafe destination path '{}'",
                    path.display()
                )));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(format!(
            "Destination path '{}' has no filename component",
            path.display()
        )));
    }

    Ok(clean)
}

/// Typed internal failure classification for contained-file inspection, opening, and hashing.
pub(super) enum ContainedFileError {
    /// Path containment or safety refusal (symlink, non-regular file, escape, unsafe path).
    Containment(IaGetError),
    /// Operation was interrupted by a shutdown signal.
    Interrupted(IaGetError),
    /// Ordinary filesystem I/O failure.
    Io(IaGetError),
}

impl ContainedFileError {
    pub(super) fn into_ia_get_error(self) -> IaGetError {
        match self {
            Self::Containment(err) | Self::Interrupted(err) | Self::Io(err) => err,
        }
    }
}

fn validate_contained_path_inner(
    canonical_root: &Path,
    rel_path: &Path,
    allow_missing_leaf: bool,
) -> std::result::Result<(PathBuf, Option<fs::Metadata>), ContainedFileError> {
    let clean_rel = validate_relative_path(rel_path).map_err(ContainedFileError::Containment)?;
    let segments: Vec<&OsStr> = clean_rel
        .components()
        .map(|component| match component {
            Component::Normal(segment) => segment,
            _ => unreachable!("validate_relative_path permits only normal components"),
        })
        .collect();

    let (parents, leaf) = segments.split_at(segments.len() - 1);
    let leaf = leaf[0];

    let mut current = canonical_root.to_path_buf();
    for (index, segment) in parents.iter().enumerate() {
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Refusing symlinked directory component '{}'",
                            current.display()
                        ),
                    )));
                }
                if !meta.file_type().is_dir() {
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Expected directory at '{}', found non-directory",
                            current.display()
                        ),
                    )));
                }
                let canonical_current =
                    fs::canonicalize(&current).map_err(|e| ContainedFileError::Io(e.into()))?;
                if !canonical_current.starts_with(canonical_root) {
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Path '{}' escapes destination root '{}'",
                            current.display(),
                            canonical_root.display()
                        ),
                    )));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if allow_missing_leaf {
                    for remaining in &parents[index + 1..] {
                        current.push(remaining);
                    }
                    current.push(leaf);
                    return Ok((current, None));
                }
                return Err(ContainedFileError::Io(err.into()));
            }
            Err(err) => return Err(ContainedFileError::Io(err.into())),
        }
    }

    current.push(leaf);
    match fs::symlink_metadata(&current) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!("Refusing symlink target '{}'", current.display()),
                )));
            }
            if !meta.file_type().is_file() {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!("Refusing non-regular file target '{}'", current.display()),
                )));
            }
            let canonical_leaf =
                fs::canonicalize(&current).map_err(|e| ContainedFileError::Io(e.into()))?;
            if !canonical_leaf.starts_with(canonical_root) {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!(
                        "Target '{}' escapes destination root '{}'",
                        current.display(),
                        canonical_root.display()
                    ),
                )));
            }
            Ok((current, Some(meta)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if allow_missing_leaf {
                Ok((current, None))
            } else {
                Err(ContainedFileError::Io(err.into()))
            }
        }
        Err(err) => Err(ContainedFileError::Io(err.into())),
    }
}

/// Validates that `rel_path` stays inside `canonical_root` with no symlinks or non-regular leaves.
pub(super) fn validate_contained_path(
    canonical_root: &Path,
    rel_path: &Path,
    allow_missing_leaf: bool,
) -> Result<PathBuf> {
    validate_contained_path_inner(canonical_root, rel_path, allow_missing_leaf)
        .map(|(path, _)| path)
        .map_err(ContainedFileError::into_ia_get_error)
}

/// Validates that `rel_path` stays inside `canonical_root` with no symlinks or non-regular leaves,
/// returning `Ok(Some((path, metadata)))` for an existing regular file or `Ok(None)` when absent.
pub(crate) fn inspect_contained_regular_file(
    canonical_root: &Path,
    rel_path: &Path,
) -> Result<Option<(PathBuf, fs::Metadata)>> {
    match validate_contained_path_inner(canonical_root, rel_path, true)
        .map_err(ContainedFileError::into_ia_get_error)?
    {
        (path, Some(meta)) => Ok(Some((path, meta))),
        (_, None) => Ok(None),
    }
}

/// Applies `O_NOFOLLOW` to `OpenOptions` on Unix targets.
pub(super) fn configure_no_follow(options: &mut fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let _ = options;
    }
}

pub(super) fn open_regular_file_for_read_in_canonical_root_typed(
    canonical_root: &Path,
    rel_path: &Path,
) -> std::result::Result<File, ContainedFileError> {
    let (target_path, _) = validate_contained_path_inner(canonical_root, rel_path, false)?;

    let mut options = fs::OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);

    let file = options
        .open(&target_path)
        .map_err(|e| ContainedFileError::Io(e.into()))?;
    let meta = file
        .metadata()
        .map_err(|e| ContainedFileError::Io(e.into()))?;
    if !meta.file_type().is_file() {
        return Err(ContainedFileError::Containment(IaGetError::FileSystem(
            format!("Refusing non-regular file '{}'", target_path.display()),
        )));
    }

    Ok(file)
}

/// Opens a verified regular file inside `root` for reading without following symlinks.
pub(crate) fn open_regular_file_for_read(root: &Path, rel_path: &Path) -> Result<File> {
    let canonical_root = resolve_destination_root(root)?;
    open_regular_file_for_read_in_canonical_root_typed(&canonical_root, rel_path)
        .map_err(ContainedFileError::into_ia_get_error)
}

/// Safely removes a contained regular `.part` file inside `root`, ignoring `NotFound`.
pub(super) fn remove_part_file_safely(root: &Path, rel_part_path: &Path) -> Result<()> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, true)?;
    match fs::remove_file(&part_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Builds the temporary `.part` path for a target file without replacing existing extensions.
pub(crate) fn part_file_path(file_path: &str) -> PathBuf {
    let mut part_path = OsString::from(file_path);
    part_path.push(".part");
    PathBuf::from(part_path)
}

/// Returns the current byte length of a `.part` file inside `root`, or `0` if it does not exist.
pub(super) fn existing_part_size(root: &Path, rel_part_path: &Path) -> Result<u64> {
    let canonical_root = resolve_destination_root(root)?;
    Ok(
        inspect_contained_regular_file(&canonical_root, rel_part_path)?
            .map_or(0, |(_, meta)| meta.len()),
    )
}

/// Atomically promote a verified `.part` file to its final destination path inside `root`.
pub(super) fn promote_part_file(
    root: &Path,
    rel_part_path: &Path,
    rel_final_path: &Path,
) -> Result<()> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, false)?;
    let final_path = validate_contained_path(&canonical_root, rel_final_path, true)?;
    fs::rename(&part_path, &final_path)?;
    Ok(())
}

/// Verifies that `path` is a real directory and not a symlink or non-directory entry.
fn verify_non_symlink_dir(path: &Path, meta: &fs::Metadata) -> Result<()> {
    if meta.file_type().is_symlink() {
        return Err(IaGetError::FileSystem(format!(
            "Refusing symlinked directory '{}'",
            path.display()
        )));
    }
    if !meta.file_type().is_dir() {
        return Err(IaGetError::FileSystem(format!(
            "Cannot create directory '{}': existing path is not a directory",
            path.display()
        )));
    }
    Ok(())
}

/// Ensure parent directories exist inside `root` for a relative destination path.
pub(super) fn ensure_parent_directories(root: &Path, rel_path: &Path) -> Result<PathBuf> {
    let canonical_root = resolve_destination_root(root)?;
    let clean_rel = validate_relative_path(rel_path)?;

    let mut current = canonical_root.clone();
    if let Some(parent) = clean_rel.parent() {
        for component in parent.components() {
            let Component::Normal(segment) = component else {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing unsafe parent path '{}'",
                    rel_path.display()
                )));
            };
            current.push(segment);

            match fs::symlink_metadata(&current) {
                Ok(meta) => verify_non_symlink_dir(&current, &meta)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(create_err)
                            if create_err.kind() == std::io::ErrorKind::AlreadyExists =>
                        {
                            let meta = fs::symlink_metadata(&current)?;
                            verify_non_symlink_dir(&current, &meta)?;
                        }
                        Err(create_err) => return Err(create_err.into()),
                    }
                }
                Err(e) => return Err(e.into()),
            }

            let canonical_current = fs::canonicalize(&current)?;
            if !canonical_current.starts_with(&canonical_root) {
                return Err(IaGetError::FileSystem(format!(
                    "Directory '{}' escapes destination root '{}'",
                    current.display(),
                    canonical_root.display()
                )));
            }
        }
    }

    Ok(current)
}

/// Prepare a `.part` file inside `root` for download by opening it safely for truncation or append.
pub(super) fn prepare_file_for_download(
    root: &Path,
    rel_part_path: &Path,
    truncate: bool,
) -> Result<File> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, true)?;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true);
    configure_no_follow(&mut options);

    let mut file = options.open(&part_path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(IaGetError::FileSystem(format!(
            "Refusing non-regular partial file '{}'",
            part_path.display()
        )));
    }

    if truncate {
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
    } else {
        file.seek(SeekFrom::End(0))?;
    }

    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::downloader::tests::{
        assert_downloaded_file, download_single_file, run_batch_in_dir, DownloadFixture,
    };
    use crate::downloader::{http_client_builder, RetryPolicy, ShutdownSignal};
    use crate::test_support::{md5_hex, MockResponse, MockServer, TempTestDir};
    use tokio::sync::Mutex;

    #[test]
    fn test_part_file_path() {
        for (input, expected) in [
            ("file.zip", "file.zip.part"),
            ("archive.tar.gz", "archive.tar.gz.part"),
            ("README", "README.part"),
            (
                "dir.v1/sub.dir/data.2026.tar.xz",
                "dir.v1/sub.dir/data.2026.tar.xz.part",
            ),
            (".hidden", ".hidden.part"),
            ("file name (1).bin", "file name (1).bin.part"),
        ] {
            assert_eq!(part_file_path(input), PathBuf::from(expected));
        }
    }

    #[test]
    fn test_validate_relative_path_rejects_traversal_absolute_empty_and_curdir() {
        for unsafe_path in [
            "",
            ".",
            "./item.bin",
            "../escape.bin",
            "sub/../../escape.bin",
            "/abs/path.bin",
        ] {
            let err = validate_relative_path(Path::new(unsafe_path)).unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "expected FileSystem error for {unsafe_path:?}, got {err:?}"
            );
        }

        assert_eq!(
            validate_relative_path(Path::new("item.bin")).unwrap(),
            PathBuf::from("item.bin")
        );
        assert_eq!(
            validate_relative_path(Path::new("nested/dir/item.bin")).unwrap(),
            PathBuf::from("nested/dir/item.bin")
        );
    }

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed for {}", path.display());
    }

    #[tokio::test]
    async fn test_download_refuses_unsafe_paths_and_special_targets_with_zero_requests() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // 1. Traversal, absolute, empty, and current-dir paths
        let dir = TempTestDir::new("unsafe-paths");
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();
        let server = MockServer::start(vec![MockResponse::ok(b"should never be fetched")]).await;
        for unsafe_path in [
            "../escape.bin",
            "sub/../../escape.bin",
            "/abs/path.bin",
            "",
            ".",
        ] {
            let err = download_single_file(
                &client,
                &root,
                &server.url("/item.bin"),
                unsafe_path,
                None,
                &signal,
                None,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "expected FileSystem error for {unsafe_path:?}, got {err:?}"
            );
        }
        assert!(server.abort().await.is_empty());
        assert!(!dir.join("escape.bin").exists());

        // 2. Directory at target (with/without MD5), directory at .part, regular file blocking dir
        type SetupFn = fn(&Path);
        let cross_cases: [(&str, &str, Option<&str>, SetupFn); 4] = [
            ("dir-target-nomd5", "item.bin", None, |r| {
                fs::create_dir(r.join("item.bin")).unwrap();
            }),
            (
                "dir-target-md5",
                "item.bin",
                Some("d41d8cd98f00b204e9800998ecf8427e"),
                |r| {
                    fs::create_dir(r.join("item.bin")).unwrap();
                },
            ),
            ("dir-part", "item.bin", None, |r| {
                fs::create_dir(r.join("item.bin.part")).unwrap();
            }),
            ("file-blocks-dir", "blocker/item.bin", None, |r| {
                fs::write(r.join("blocker"), b"existing regular file").unwrap();
            }),
        ];
        for (label, rel_path, expected_md5, setup) in cross_cases {
            let fx = DownloadFixture::new(label);
            setup(fx.path());
            let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
            let err = fx
                .download(&server.url("/item.bin"), rel_path, expected_md5, None)
                .await
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)), "case {label}");
            assert!(server.abort().await.is_empty(), "case {label}");
            if label == "file-blocks-dir" {
                assert_eq!(
                    fs::read(fx.join("blocker")).unwrap(),
                    b"existing regular file"
                );
            } else if label == "dir-part" {
                assert!(fx.part_path("item.bin").is_dir());
                assert!(!fx.join("item.bin").exists());
            } else {
                assert!(fx.join("item.bin").is_dir());
                assert!(!fx.part_path("item.bin").exists());
            }
        }

        // 3. Unix symlinks (intermediate dir, live/dangling .part, live/dangling target) and FIFOs
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            enum UnixSpecial {
                SymlinkParent,
                SymlinkPartLive,
                SymlinkPartDangling,
                SymlinkTargetLive,
                SymlinkTargetDangling,
                FifoTarget,
                FifoPart,
            }

            let md5_some = Some("d41d8cd98f00b204e9800998ecf8427e");
            let unix_cases: [(&str, UnixSpecial, &str, Option<&str>); 10] = [
                (
                    "symlink-parent",
                    UnixSpecial::SymlinkParent,
                    "sub/victim.bin",
                    None,
                ),
                (
                    "symlink-part-live",
                    UnixSpecial::SymlinkPartLive,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-part-dangling",
                    UnixSpecial::SymlinkPartDangling,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-live-nomd5",
                    UnixSpecial::SymlinkTargetLive,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-live-md5",
                    UnixSpecial::SymlinkTargetLive,
                    "item.bin",
                    md5_some,
                ),
                (
                    "symlink-target-dangling-nomd5",
                    UnixSpecial::SymlinkTargetDangling,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-dangling-md5",
                    UnixSpecial::SymlinkTargetDangling,
                    "item.bin",
                    md5_some,
                ),
                (
                    "fifo-target-nomd5",
                    UnixSpecial::FifoTarget,
                    "item.bin",
                    None,
                ),
                (
                    "fifo-target-md5",
                    UnixSpecial::FifoTarget,
                    "item.bin",
                    md5_some,
                ),
                ("fifo-part", UnixSpecial::FifoPart, "item.bin", None),
            ];

            for (label, kind, rel_path, expected_md5) in unix_cases {
                let sandbox = TempTestDir::new(label);
                let root = sandbox.join("root");
                let outside = sandbox.join("outside");
                fs::create_dir_all(&root).unwrap();
                fs::create_dir_all(&outside).unwrap();
                let victim = outside.join("victim.bin");
                let missing = outside.join("missing.bin");

                match kind {
                    UnixSpecial::SymlinkParent => {
                        fs::write(&victim, b"original outside data").unwrap();
                        symlink(&outside, root.join("sub")).unwrap();
                    }
                    UnixSpecial::SymlinkPartLive => {
                        fs::write(&victim, b"do not overwrite").unwrap();
                        symlink(&victim, root.join("item.bin.part")).unwrap();
                    }
                    UnixSpecial::SymlinkPartDangling => {
                        symlink(&missing, root.join("item.bin.part")).unwrap();
                    }
                    UnixSpecial::SymlinkTargetLive => {
                        fs::write(&victim, b"protected target").unwrap();
                        symlink(&victim, root.join("item.bin")).unwrap();
                    }
                    UnixSpecial::SymlinkTargetDangling => {
                        symlink(&missing, root.join("item.bin")).unwrap();
                    }
                    UnixSpecial::FifoTarget => create_fifo(&root.join("item.bin")),
                    UnixSpecial::FifoPart => create_fifo(&root.join("item.bin.part")),
                }

                let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    download_single_file(
                        &client,
                        &root,
                        &server.url("/item.bin"),
                        rel_path,
                        expected_md5,
                        &signal,
                        None,
                    ),
                )
                .await
                .unwrap_or_else(|_| panic!("case {label}: check must not hang"));

                assert!(
                    matches!(result.unwrap_err(), IaGetError::FileSystem(_)),
                    "case {label}"
                );
                assert!(server.abort().await.is_empty(), "case {label}");
                match kind {
                    UnixSpecial::SymlinkParent => {
                        assert_eq!(fs::read(&victim).unwrap(), b"original outside data");
                        assert!(!outside.join("victim.bin.part").exists());
                    }
                    UnixSpecial::SymlinkPartLive => {
                        assert_eq!(fs::read(&victim).unwrap(), b"do not overwrite");
                        assert!(!root.join("item.bin").exists());
                    }
                    UnixSpecial::SymlinkPartDangling => {
                        assert!(!missing.exists());
                        assert!(!root.join("item.bin").exists());
                    }
                    UnixSpecial::SymlinkTargetLive => {
                        assert_eq!(fs::read(&victim).unwrap(), b"protected target");
                    }
                    UnixSpecial::SymlinkTargetDangling => assert!(!missing.exists()),
                    UnixSpecial::FifoTarget | UnixSpecial::FifoPart => {}
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_open_nofollow_flag_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let dir = TempTestDir::new("nofollow-flag");
        let real_file = dir.write("real.bin", b"hello");
        let link_file = dir.join("link.bin");
        symlink(&real_file, &link_file).unwrap();

        let mut regular_opts = fs::OpenOptions::new();
        regular_opts.read(true);
        configure_no_follow(&mut regular_opts);
        let _regular = regular_opts
            .open(&real_file)
            .expect("O_NOFOLLOW should allow opening regular files");

        let mut link_opts = fs::OpenOptions::new();
        link_opts.read(true);
        configure_no_follow(&mut link_opts);
        let err = link_opts.open(&link_file).unwrap_err();
        assert!(
            err.raw_os_error().is_some(),
            "expected OS error (ELOOP) when opening symlink with O_NOFOLLOW, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_promote_part_file_refuses_symlink_fifo_and_directory_targets() {
        use std::os::unix::fs::symlink;

        enum PromoteTarget {
            LiveSymlink,
            DanglingSymlink,
            Fifo,
            Directory,
        }

        for (label, kind) in [
            ("promote-symlink-live", PromoteTarget::LiveSymlink),
            ("promote-symlink-dangling", PromoteTarget::DanglingSymlink),
            ("promote-fifo", PromoteTarget::Fifo),
            ("promote-dir", PromoteTarget::Directory),
        ] {
            let dir = TempTestDir::new(label);
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();
            fs::write(root.join("item.bin.part"), b"staged").unwrap();

            let outside_file = outside.join("victim.bin");
            match kind {
                PromoteTarget::LiveSymlink => {
                    fs::write(&outside_file, b"original").unwrap();
                    symlink(&outside_file, root.join("item.bin")).unwrap();
                }
                PromoteTarget::DanglingSymlink => {
                    symlink(&outside_file, root.join("item.bin")).unwrap();
                }
                PromoteTarget::Fifo => create_fifo(&root.join("item.bin")),
                PromoteTarget::Directory => fs::create_dir(root.join("item.bin")).unwrap(),
            }

            let err = promote_part_file(&root, Path::new("item.bin.part"), Path::new("item.bin"))
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)), "case {label}");
            assert_eq!(fs::read(root.join("item.bin.part")).unwrap(), b"staged");
            match kind {
                PromoteTarget::LiveSymlink => {
                    assert_eq!(fs::read(&outside_file).unwrap(), b"original");
                }
                PromoteTarget::DanglingSymlink => assert!(!outside_file.exists()),
                PromoteTarget::Directory => assert!(root.join("item.bin").is_dir()),
                PromoteTarget::Fifo => {}
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlinked_output_root_allows_contained_downloads_and_blocks_inner_escapes() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("symlinked-output-root");
        let real_root = sandbox.join("real_root");
        let link_root = sandbox.join("link_root");
        let outside = sandbox.join("outside");
        fs::create_dir_all(&real_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&real_root, &link_root).unwrap();
        symlink(&outside, real_root.join("escape")).unwrap();

        let canonical = prepare_output_dir(&link_root)
            .expect("symlinked --output-dir pointing to an existing directory should succeed");
        assert_eq!(canonical, fs::canonicalize(&real_root).unwrap());
        assert!(matches!(
            validate_contained_path(&canonical, Path::new("escape/bad.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Single-file download into symlinked root succeeds; inner symlink escape fails with 0 requests
        let single_payload = b"downloaded through symlinked root";
        let server_single = MockServer::start(vec![MockResponse::ok(single_payload)]).await;
        download_single_file(
            &client,
            &link_root,
            &server_single.url("/single.bin"),
            "nested/single.bin",
            Some(&md5_hex(single_payload)),
            &signal,
            None,
        )
        .await
        .expect("single download into symlinked root should succeed");
        assert_eq!(server_single.finish().await.len(), 1);
        assert_downloaded_file(&real_root, "nested/single.bin", single_payload);

        let server_single_bad = MockServer::start(vec![MockResponse::ok(b"nope")]).await;
        let err = download_single_file(
            &client,
            &link_root,
            &server_single_bad.url("/bad.bin"),
            "escape/single_bad.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(server_single_bad.abort().await.is_empty());

        // Batch download into symlinked root skips inner escape (0 requests) and downloads sibling
        let batch_payload = b"batch downloaded via symlinked output-dir";
        let server_batch = MockServer::start(vec![MockResponse::ok(batch_payload)]).await;
        let files = vec![
            (
                server_batch.url("/batch_bad.bin"),
                "escape/batch_bad.bin".to_string(),
                None,
            ),
            (
                server_batch.url("/batch_good.bin"),
                "nested/batch_good.bin".to_string(),
                Some(md5_hex(batch_payload)),
            ),
        ];
        let summary = run_batch_in_dir(
            &client,
            &link_root,
            files,
            2,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("batch into symlinked output-dir should return summary");

        let batch_requests = server_batch.finish().await;
        assert_eq!(batch_requests.len(), 1);
        assert!(batch_requests[0]
            .request_line
            .starts_with("GET /batch_good.bin "));
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "escape/batch_bad.bin");
        assert!(summary.failed[0]
            .error
            .starts_with("File operation failed:"));
        assert!(!outside.join("single_bad.bin").exists());
        assert!(!outside.join("batch_bad.bin").exists());
        assert_downloaded_file(&real_root, "nested/batch_good.bin", batch_payload);
    }

    static CURRENT_DIR_LOCK: Mutex<()> = Mutex::const_new(());

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(target: &Path) -> Self {
            let original = std::env::current_dir().expect("read current working directory");
            std::env::set_current_dir(target).expect("switch current working directory");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[tokio::test]
    async fn test_prepare_output_dir_dot_returns_canonical_current_working_dir() {
        let _cwd_lock = CURRENT_DIR_LOCK.lock().await;
        let dir = TempTestDir::new("prepare-dot");
        let _cwd_guard = CurrentDirGuard::enter(dir.path());

        let canonical = prepare_output_dir(Path::new("."))
            .expect("prepare_output_dir('.') should resolve current working directory");
        assert_eq!(canonical, fs::canonicalize(dir.path()).unwrap());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_writes_to_default_working_dir() {
        let _cwd_lock = CURRENT_DIR_LOCK.lock().await;
        let fx = DownloadFixture::new("default-cwd-download");
        let _cwd_guard = CurrentDirGuard::enter(fx.path());

        let payload_top = b"default working directory top-level payload";
        let payload_nested = b"default working directory nested payload";
        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_nested),
        ])
        .await;

        let files = vec![
            (
                server.url("/top.bin"),
                "top.bin".to_string(),
                Some(md5_hex(payload_top)),
            ),
            (
                server.url("/nested.bin"),
                "sub/nested.bin".to_string(),
                Some(md5_hex(payload_nested)),
            ),
        ];

        let summary = run_batch_in_dir(
            &fx.client,
            Path::new("."),
            files,
            2,
            &RetryPolicy::default(),
            &fx.signal,
        )
        .await
        .expect("default working directory download should succeed");

        assert_eq!(server.finish().await.len(), 2);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        fx.assert_downloaded("top.bin", payload_top);
        fx.assert_downloaded("sub/nested.bin", payload_nested);
    }

    #[test]
    fn test_prepare_output_dir_creates_missing_nested_directories() {
        let sandbox = TempTestDir::new("prepare-nested");
        let target = sandbox.join("level1/level2/output");
        assert!(!target.exists());

        let canonical = prepare_output_dir(&target)
            .expect("prepare_output_dir should create missing nested directories");
        assert!(target.is_dir());
        assert_eq!(canonical, fs::canonicalize(&target).unwrap());

        let canonical_again = prepare_output_dir(&target)
            .expect("prepare_output_dir should accept already-created directory");
        assert_eq!(canonical_again, canonical);
    }

    #[tokio::test]
    async fn test_download_files_in_dir_creates_nested_output_dir_and_preserves_archive_paths() {
        let sandbox = TempTestDir::new("nested-output-dir");
        let output_dir = sandbox.join("downloads/archive/item");
        assert!(!output_dir.exists());

        let payload_top = b"top-level archive file";
        let payload_deep = b"deeply nested archive binary payload";
        let payload_sibling = b"second nested archive file";
        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_deep),
            MockResponse::ok(payload_sibling),
        ])
        .await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (
                server.url("/top.txt"),
                "top.txt".to_string(),
                Some(md5_hex(payload_top)),
            ),
            (
                server.url("/dir1/sub/file.bin"),
                "dir1/sub/file.bin".to_string(),
                Some(md5_hex(payload_deep)),
            ),
            (
                server.url("/dir2/data.bin"),
                "dir2/data.bin".to_string(),
                Some(md5_hex(payload_sibling)),
            ),
        ];

        let summary = run_batch_in_dir(
            &client,
            &output_dir,
            files,
            3,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("download_files_in_dir_with_logger should create missing nested output dir");

        assert_eq!(server.finish().await.len(), 3);
        assert!(output_dir.is_dir());
        assert_eq!(summary.ok, 3);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_downloaded_file(&output_dir, "top.txt", payload_top);
        assert_downloaded_file(&output_dir, "dir1/sub/file.bin", payload_deep);
        assert_downloaded_file(&output_dir, "dir2/data.bin", payload_sibling);

        let sandbox_entries: Vec<OsString> = fs::read_dir(sandbox.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            sandbox_entries,
            vec![OsString::from("downloads")],
            "nothing should be written outside the target output directory"
        );
    }

    #[tokio::test]
    async fn test_prepare_output_dir_and_download_reject_invalid_paths_with_zero_requests() {
        let sandbox = TempTestDir::new("invalid-output-paths");
        let existing_file = sandbox.write("existing.txt", b"untouched-regular-file");
        let file_ancestor = existing_file.join("sub");
        let empty_path = PathBuf::from("");

        let mut cases: Vec<(&str, PathBuf, Option<&str>)> = vec![
            ("existing-file", existing_file.clone(), None),
            ("file-ancestor", file_ancestor, None),
            ("empty-path", empty_path, None),
        ];

        #[cfg(unix)]
        let missing_symlink_target = {
            use std::os::unix::fs::symlink;
            let missing_target = sandbox.join("missing-target");
            let broken_link = sandbox.join("broken-out");
            symlink(&missing_target, &broken_link).unwrap();
            cases.push(("broken-symlink", broken_link, Some("broken symlink")));
            missing_target
        };

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for (label, bad_path, expected_substr) in cases {
            let prep_err = prepare_output_dir(&bad_path).unwrap_err();
            assert!(
                matches!(prep_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from prepare_output_dir, got {prep_err:?}"
            );
            if let Some(substr) = expected_substr {
                assert!(
                    prep_err.to_string().contains(substr),
                    "case {label}: expected '{substr}' in '{prep_err}'"
                );
            }

            let server =
                MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
            let dl_err = run_batch_in_dir(
                &client,
                &bad_path,
                vec![(server.url("/file.bin"), "file.bin".to_string(), None)],
                1,
                &RetryPolicy::default(),
                &signal,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(dl_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from download_files_in_dir_with_logger, got {dl_err:?}"
            );
            assert!(
                server.abort().await.is_empty(),
                "case {label}: expected 0 HTTP requests for invalid output path"
            );
        }

        assert_eq!(fs::read(&existing_file).unwrap(), b"untouched-regular-file");
        #[cfg(unix)]
        assert!(!missing_symlink_target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlink_escape_relative_to_output_dir_rejects_escapes_and_downloads_siblings(
    ) {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("output-dir-symlink-escape");
        let output_dir = sandbox.join("out");
        let outside_dir = sandbox.join("outside");
        fs::create_dir_all(&output_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let outside_victim = outside_dir.join("victim.bin");
        fs::write(&outside_victim, b"untouched-outside-victim").unwrap();
        symlink(&outside_dir, output_dir.join("escape_dir")).unwrap();
        symlink(&outside_victim, output_dir.join("escape_file.bin")).unwrap();

        let canonical_out = prepare_output_dir(&output_dir).unwrap();
        assert!(matches!(
            validate_contained_path(&canonical_out, Path::new("escape_dir/payload.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            validate_contained_path(&canonical_out, Path::new("escape_file.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));

        let safe_payload = b"safe sibling downloaded inside output-dir";
        let server = MockServer::start(vec![MockResponse::ok(safe_payload)]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (
                server.url("/escape-dir.bin"),
                "escape_dir/payload.bin".to_string(),
                None,
            ),
            (
                server.url("/escape-file.bin"),
                "escape_file.bin".to_string(),
                None,
            ),
            (
                server.url("/safe.bin"),
                "safe/sibling.bin".to_string(),
                Some(md5_hex(safe_payload)),
            ),
        ];

        let summary = run_batch_in_dir(
            &client,
            &output_dir,
            files,
            3,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("batch with escaped entries should return summary");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /safe.bin "));
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 2);
        for (idx, path) in ["escape_dir/payload.bin", "escape_file.bin"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(summary.failed[idx].file_path, path);
            assert!(summary.failed[idx]
                .error
                .starts_with("File operation failed:"));
        }

        assert!(!outside_dir.join("payload.bin").exists());
        assert!(!outside_dir.join("payload.bin.part").exists());
        assert_eq!(
            fs::read(&outside_victim).unwrap(),
            b"untouched-outside-victim"
        );
        assert_downloaded_file(&output_dir, "safe/sibling.bin", safe_payload);
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_root_path_prepare_and_contained_validation() {
        let root = prepare_output_dir(Path::new("/"))
            .expect("Unix root path '/' should pass prepare_output_dir");
        assert_eq!(root, PathBuf::from("/"));

        let validated =
            validate_contained_path(&root, Path::new("ia-get-nonexistent/file.bin"), true)
                .expect("missing relative path under '/' should be accepted");
        assert_eq!(validated, PathBuf::from("/ia-get-nonexistent/file.bin"));

        let escape_err =
            validate_contained_path(&root, Path::new("../outside.bin"), true).unwrap_err();
        assert!(matches!(escape_err, IaGetError::FileSystem(_)));
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_drive_root_prepare_and_escape_rejection() {
        let temp = std::env::temp_dir();
        let mut drive_root = PathBuf::new();
        for component in temp.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => drive_root.push(component.as_os_str()),
                _ => break,
            }
        }
        assert!(
            drive_root.has_root(),
            "expected drive root from temp_dir '{}', got '{}'",
            temp.display(),
            drive_root.display()
        );

        let canonical_root = prepare_output_dir(&drive_root)
            .expect("Windows drive root should pass prepare_output_dir");
        assert_eq!(canonical_root, fs::canonicalize(&drive_root).unwrap());

        let validated = validate_contained_path(
            &canonical_root,
            Path::new("ia-get-nonexistent/file.bin"),
            true,
        )
        .expect("missing relative path under Windows drive root should succeed");
        assert_eq!(
            validated,
            canonical_root.join("ia-get-nonexistent").join("file.bin")
        );

        for unsafe_path in [
            Path::new("../escape.bin"),
            Path::new("nested/../../escape.bin"),
            Path::new(r"\absolute.bin"),
            Path::new(r"C:\escape.bin"),
        ] {
            let err = validate_contained_path(&canonical_root, unsafe_path, true).unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "path '{}' should be rejected as FileSystem error, got {err:?}",
                unsafe_path.display()
            );
        }
    }
}
