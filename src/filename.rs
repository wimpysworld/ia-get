//! Sanitizing server-controlled file names for cross-platform filesystem
//! compatibility: replacing characters Windows or Unix reject, trimming
//! edge whitespace and dots, marking reserved device names, and dropping
//! `.`/`..` components that could escape the working directory.

/// Windows reserved device names (case-insensitive, after the superscript
/// digit normalization of `device_name_base`). A path component whose base
/// name matches one of these gets an underscore appended: unmarked, the
/// open could reach a console or serial/parallel device instead of a file.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
    "LPT8", "LPT9",
];

/// Normalizes a base name for the device-name comparison: Windows'
/// device-name matching folds Unicode superscript digits to their ASCII
/// form, so "COM¹" opens the COM1 device.
fn device_name_base(base: &str) -> String {
    base.chars()
        .map(|c| match c {
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            '⁴' => '4',
            '⁵' => '5',
            '⁶' => '6',
            '⁷' => '7',
            '⁸' => '8',
            '⁹' => '9',
            _ => c,
        })
        .collect()
}

/// Characters that are invalid in file names on Windows or Unix and are
/// replaced with underscores
fn is_invalid_filename_char(ch: char) -> bool {
    matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\')
        || ('\x00'..='\x1F').contains(&ch)
        || ch == '\x7F'
}

/// Replaces every invalid character in a path component with an underscore.
///
/// Returns the replacement and whether anything was changed.
fn replace_invalid_chars(component: &str) -> (String, bool) {
    let mut sanitized = String::with_capacity(component.len());
    let mut was_modified = false;

    for ch in component.chars() {
        if is_invalid_filename_char(ch) {
            sanitized.push('_');
            was_modified = true;
        } else {
            sanitized.push(ch);
        }
    }

    (sanitized, was_modified)
}

/// Appends an underscore to a component whose base name (the part before the
/// first dot) is a Windows reserved name, keeping the extension intact
/// (`CON.txt` → `CON_.txt`).
///
/// Returns whether anything was changed.
fn mark_reserved_name(component: &mut String) -> bool {
    let dot_pos = component.find('.');
    let base_name = dot_pos.map_or(component.as_str(), |pos| &component[..pos]);

    let is_reserved = WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| device_name_base(base_name).eq_ignore_ascii_case(reserved));

    if is_reserved {
        match dot_pos {
            Some(pos) => component.insert(pos, '_'),
            None => component.push('_'),
        }
    }

    is_reserved
}

/// Sanitizes a single non-empty path component: replaces invalid characters,
/// trims surrounding spaces and trailing dots (Windows rejects both),
/// substitutes a placeholder for components that end up empty, and marks
/// Windows reserved names.
///
/// Returns the sanitized component and whether anything was changed.
fn sanitize_component(component: &str) -> (String, bool) {
    let (mut sanitized, mut was_modified) = replace_invalid_chars(component);

    // Trailing dots and the whitespace they uncover are stripped together
    // ("file ." must not leave a trailing space, which Windows rejects)
    let trimmed = sanitized
        .trim()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.len() != sanitized.len() {
        sanitized = trimmed.to_string();
        was_modified = true;
    }

    if sanitized.is_empty() {
        sanitized = "_".to_string();
        was_modified = true;
    }

    was_modified |= mark_reserved_name(&mut sanitized);

    (sanitized, was_modified)
}

/// Sanitizes a filename for cross-platform filesystem compatibility
///
/// Replaces characters that are invalid on Windows or Unix filesystems
/// with underscores, while preserving path separators.
///
/// Invalid characters replaced with underscores:
/// - Windows: `< > : " | ? *` and control characters (0-31)
/// - Unix: null character (\0)
/// - Both: leading/trailing spaces, trailing dots in path components
///
/// Also handles Windows reserved names (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
/// by appending an underscore, and drops `.`/`..` path components: kept,
/// they would let a server-controlled name escape the working directory.
///
/// # Arguments
/// * `filename` - The original filename (may include path components separated by `/`)
///
/// # Returns
/// * `(sanitized_filename, was_modified)` - Tuple of cleaned filename and whether it was changed
///
/// # Examples
/// ```
/// use ia_get::filename::sanitize_filename;
///
/// let (sanitized, modified) = sanitize_filename("normal_file.txt");
/// assert_eq!(sanitized, "normal_file.txt");
/// assert!(!modified);
///
/// let (sanitized, modified) = sanitize_filename("file?name.txt");
/// assert_eq!(sanitized, "file_name.txt");
/// assert!(modified);
///
/// let (sanitized, modified) = sanitize_filename("Season 1/Episode?.mp4");
/// assert_eq!(sanitized, "Season 1/Episode_.mp4");
/// assert!(modified);
/// ```
#[must_use]
pub fn sanitize_filename(filename: &str) -> (String, bool) {
    // Process each path component separately to preserve directory structure
    let components: Vec<&str> = filename.split('/').collect();
    // "." and ".." are dropped like empty components: kept, they would let
    // a name escape the working directory
    let kept_count = components
        .iter()
        .filter(|c| !c.is_empty() && **c != "." && **c != "..")
        .count();

    // Dropped components (empty, "." or "..") count as a modification,
    // except for an empty input
    let mut was_modified = !filename.is_empty() && kept_count != components.len();

    let mut result = String::with_capacity(filename.len());
    let mut emitted = 0;

    for component in &components {
        if component.is_empty() || *component == "." || *component == ".." {
            continue;
        }

        if emitted > 0 {
            result.push('/');
        }
        emitted += 1;

        let (sanitized, modified) = sanitize_component(component);
        was_modified |= modified;
        result.push_str(&sanitized);
    }

    (result, was_modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_valid_filename() {
        let (result, modified) = sanitize_filename("normal_file-name.txt");
        assert_eq!(result, "normal_file-name.txt");
        assert!(!modified);
    }

    #[test]
    fn test_sanitize_valid_filename_with_path() {
        let (result, modified) = sanitize_filename("folder/subfolder/file.txt");
        assert_eq!(result, "folder/subfolder/file.txt");
        assert!(!modified);
    }

    #[test]
    fn test_sanitize_invalid_characters() {
        let (result, modified) = sanitize_filename("file?name:test<>.txt");
        assert_eq!(result, "file_name_test__.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_question_mark() {
        let (result, modified) = sanitize_filename("Episode?.mp4");
        assert_eq!(result, "Episode_.mp4");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_with_path() {
        let (result, modified) = sanitize_filename("Season 1/Episode?.mp4");
        assert_eq!(result, "Season 1/Episode_.mp4");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_multiple_invalid_in_path() {
        let (result, modified) = sanitize_filename("Folder:Name/File*Name?.txt");
        assert_eq!(result, "Folder_Name/File_Name_.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_windows_reserved_names() {
        let (result, modified) = sanitize_filename("CON.txt");
        assert_eq!(result, "CON_.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("con.txt");
        assert_eq!(result, "con_.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("PRN");
        assert_eq!(result, "PRN_");
        assert!(modified);

        let (result, modified) = sanitize_filename("aux.log");
        assert_eq!(result, "aux_.log");
        assert!(modified);

        let (result, modified) = sanitize_filename("COM1.dat");
        assert_eq!(result, "COM1_.dat");
        assert!(modified);

        let (result, modified) = sanitize_filename("LPT9.txt");
        assert_eq!(result, "LPT9_.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_console_device_aliases() {
        // The console's extra device aliases are reserved names too
        let (result, modified) = sanitize_filename("CONIN$");
        assert_eq!(result, "CONIN$_");
        assert!(modified);

        let (result, modified) = sanitize_filename("CONOUT$.txt");
        assert_eq!(result, "CONOUT$_.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("clock$");
        assert_eq!(result, "clock$_");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_superscript_device_names() {
        // Windows folds Unicode superscript digits in device-name matching:
        // "COM¹" would open the COM1 device unmarked
        let (result, modified) = sanitize_filename("COM¹.txt");
        assert_eq!(result, "COM¹_.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("lpt²");
        assert_eq!(result, "lpt²_");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_reserved_in_path() {
        let (result, modified) = sanitize_filename("folder/CON.txt");
        assert_eq!(result, "folder/CON_.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_control_characters() {
        let (result, modified) = sanitize_filename("file\x00\x1fname.txt");
        assert_eq!(result, "file__name.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("test\x7Ffile.txt");
        assert_eq!(result, "test_file.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_backslash() {
        let (result, modified) = sanitize_filename("folder\\file.txt");
        assert_eq!(result, "folder_file.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_whitespace_edge_cases() {
        let (result, modified) = sanitize_filename(" leading.txt ");
        assert_eq!(result, "leading.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("folder/ spaces /file.txt");
        assert_eq!(result, "folder/spaces/file.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_drops_dot_segments() {
        // "." and ".." must not survive: kept, a server-controlled name
        // could escape the working directory.
        let (result, modified) = sanitize_filename("../../etc/passwd");
        assert_eq!(result, "etc/passwd");
        assert!(modified);

        let (result, modified) = sanitize_filename("a/./b/c.txt");
        assert_eq!(result, "a/b/c.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("a/b.txt");
        assert_eq!(result, "a/b.txt");
        assert!(!modified);
    }

    #[test]
    fn test_sanitize_trailing_dots() {
        let (result, modified) = sanitize_filename("file...");
        assert_eq!(result, "file");
        assert!(modified);

        let (result, modified) = sanitize_filename("folder./file.txt");
        assert_eq!(result, "folder/file.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_trailing_dots_and_spaces() {
        // A space before a trailing dot must not survive the dot removal
        // (Windows rejects names ending in a space)
        let (result, modified) = sanitize_filename("file .");
        assert_eq!(result, "file");
        assert!(modified);

        let (result, modified) = sanitize_filename("a/ b . ./c.txt");
        assert_eq!(result, "a/b/c.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_empty_components() {
        let (result, modified) = sanitize_filename("folder//file.txt");
        assert_eq!(result, "folder/file.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("/folder/file.txt");
        assert_eq!(result, "folder/file.txt");
        assert!(modified);

        let (result, modified) = sanitize_filename("folder/file.txt/");
        assert_eq!(result, "folder/file.txt");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_all_invalid() {
        let (result, modified) = sanitize_filename("???");
        assert_eq!(result, "___");
        assert!(modified);
    }

    #[test]
    fn test_sanitize_unicode() {
        let (result, modified) = sanitize_filename("файл.txt");
        assert_eq!(result, "файл.txt");
        assert!(!modified);

        let (result, modified) = sanitize_filename("文件.txt");
        assert_eq!(result, "文件.txt");
        assert!(!modified);

        let (result, modified) = sanitize_filename("emoji😀.txt");
        assert_eq!(result, "emoji😀.txt");
        assert!(!modified);
    }

    #[test]
    fn test_sanitize_mixed_valid_invalid() {
        let (result, modified) =
            sanitize_filename("Red vs. Blue - Season 1/Episode 1: Why Are We Here?.mp4");
        assert_eq!(
            result,
            "Red vs. Blue - Season 1/Episode 1_ Why Are We Here_.mp4"
        );
        assert!(modified);
    }

    #[test]
    fn test_sanitize_preserves_extension() {
        let (result, modified) = sanitize_filename("file:name.tar.gz");
        assert_eq!(result, "file_name.tar.gz");
        assert!(modified);
    }
}
