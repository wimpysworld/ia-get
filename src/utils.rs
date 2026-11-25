//! Utility functions for ia-get.

use crate::constants::IDENTIFIER_PATTERN;
use crate::{IaGetError, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use std::sync::LazyLock;
use url::Url;

/// Spinner tick interval in milliseconds
pub const SPINNER_TICK_INTERVAL: u64 = 100;

/// Size constants for formatting
const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;

/// Compiled regex for archive.org item identifier validation (initialized once)
static IDENTIFIER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(IDENTIFIER_PATTERN).expect("Invalid identifier regex pattern"));

/// Parsed representation of a supported archive.org URL
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveUrl {
    /// An item details URL (`https://archive.org/details/<identifier>`)
    Details {
        /// Archive.org item identifier
        identifier: String,
    },
    /// A direct file download URL (`https://archive.org/download/<identifier>/<path>`)
    Download {
        /// Archive.org item identifier
        identifier: String,
        /// Decoded relative file path within the archive item
        file_path: String,
    },
}

impl ArchiveUrl {
    /// Returns the archive.org item identifier.
    pub fn identifier(&self) -> &str {
        match self {
            Self::Details { identifier } | Self::Download { identifier, .. } => identifier,
        }
    }

    /// Returns the decoded relative file path for download URLs, or `None` for details URLs.
    pub fn file_path(&self) -> Option<&str> {
        match self {
            Self::Details { .. } => None,
            Self::Download { file_path, .. } => Some(file_path),
        }
    }

    /// Returns the canonical details page URL for this item.
    pub fn details_url(&self) -> String {
        format!("https://archive.org/details/{}", self.identifier())
    }

    /// Returns the XML metadata URL for this item.
    pub fn xml_url(&self) -> String {
        let identifier = self.identifier();
        format!("https://archive.org/download/{identifier}/{identifier}_files.xml")
    }
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes a URL path string into valid UTF-8 without NUL bytes.
///
/// Preserves `+` literally according to URL path rules (not form query rules).
/// Rejects incomplete `%` escapes, non-hex escapes, NUL bytes (`\0` or `%00`),
/// and invalid UTF-8 byte sequences with [`IaGetError::UrlFormat`].
///
/// # Examples
/// ```
/// use ia_get::utils::percent_decode_utf8;
///
/// assert_eq!(percent_decode_utf8("folder/my%20file+1.pdf").unwrap(), "folder/my file+1.pdf");
/// assert!(percent_decode_utf8("%2G").is_err());
/// assert!(percent_decode_utf8("%00").is_err());
/// ```
pub fn percent_decode_utf8(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%' {
            if i + 2 >= bytes.len() {
                return Err(IaGetError::UrlFormat(input.to_string()));
            }
            let (Some(hi), Some(lo)) = (
                decode_hex_nibble(bytes[i + 1]),
                decode_hex_nibble(bytes[i + 2]),
            ) else {
                return Err(IaGetError::UrlFormat(input.to_string()));
            };
            let value = (hi << 4) | lo;
            if value == 0 {
                return Err(IaGetError::UrlFormat(input.to_string()));
            }
            decoded.push(value);
            i += 3;
        } else {
            if byte == 0 {
                return Err(IaGetError::UrlFormat(input.to_string()));
            }
            decoded.push(byte);
            i += 1;
        }
    }

    String::from_utf8(decoded).map_err(|_| IaGetError::UrlFormat(input.to_string()))
}

fn is_valid_identifier(identifier: &str) -> bool {
    identifier != "." && identifier != ".." && IDENTIFIER_REGEX.is_match(identifier)
}

fn has_invalid_path_slashes(path: &str) -> bool {
    path.is_empty() || path.starts_with('/') || path.ends_with('/') || path.contains("//")
}

/// Parses and validates an archive.org details or direct download URL.
///
/// Supports:
/// - `https://archive.org/details/<identifier>[/]`
/// - `https://archive.org/download/<identifier>/<path>`
///
/// # Arguments
/// * `url` - The URL to parse
///
/// # Returns
/// * `Ok(ArchiveUrl)` if the URL is valid
/// * `Err(IaGetError::UrlFormat)` if the URL format is invalid
///
/// # Examples
/// ```
/// use ia_get::utils::{parse_archive_url, ArchiveUrl};
///
/// assert_eq!(
///     parse_archive_url("https://archive.org/details/valid-item/").unwrap(),
///     ArchiveUrl::Details {
///         identifier: "valid-item".to_string(),
///     }
/// );
/// assert_eq!(
///     parse_archive_url("https://archive.org/download/valid-item/dir/my%20file.pdf").unwrap(),
///     ArchiveUrl::Download {
///         identifier: "valid-item".to_string(),
///         file_path: "dir/my file.pdf".to_string(),
///     }
/// );
/// ```
pub fn parse_archive_url(url: &str) -> Result<ArchiveUrl> {
    let invalid = || IaGetError::UrlFormat(url.to_string());
    let parsed = Url::parse(url).map_err(|_| invalid())?;

    if parsed.scheme() != "https"
        || parsed.host_str() != Some("archive.org")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(invalid());
    }

    let path = parsed.path();

    if let Some(rest) = path.strip_prefix("/details/") {
        let identifier = rest.strip_suffix('/').unwrap_or(rest);
        if !is_valid_identifier(identifier) {
            return Err(invalid());
        }
        return Ok(ArchiveUrl::Details {
            identifier: identifier.to_string(),
        });
    }

    if let Some(rest) = path.strip_prefix("/download/") {
        let Some((identifier, raw_file_path)) = rest.split_once('/') else {
            return Err(invalid());
        };
        if !is_valid_identifier(identifier) || has_invalid_path_slashes(raw_file_path) {
            return Err(invalid());
        }
        let file_path = percent_decode_utf8(raw_file_path).map_err(|_| invalid())?;
        if has_invalid_path_slashes(&file_path) {
            return Err(invalid());
        }
        return Ok(ArchiveUrl::Download {
            identifier: identifier.to_string(),
            file_path,
        });
    }

    Err(invalid())
}

/// Validates an archive.org details or direct download URL format
///
/// # Arguments
/// * `url` - The URL to validate
///
/// # Returns
/// * `Ok(())` if the URL is valid
/// * `Err(IaGetError::UrlFormat)` if the URL format is invalid
///
/// # Examples
/// ```
/// use ia_get::utils::validate_archive_url;
///
/// assert!(validate_archive_url("https://archive.org/details/valid-item").is_ok());
/// assert!(validate_archive_url("https://archive.org/details/valid-item/").is_ok());
/// assert!(validate_archive_url("https://archive.org/download/valid-item/dir/file.pdf").is_ok());
/// assert!(validate_archive_url("https://example.com/invalid").is_err());
/// ```
pub fn validate_archive_url(url: &str) -> Result<()> {
    parse_archive_url(url).map(|_| ())
}

/// Create a progress bar with consistent styling
///
/// # Arguments
/// * `total` - Total value for the progress bar
/// * `action` - Action text to show at the beginning (e.g., "╰╼ Downloading  ")
/// * `color` - Optional color style (defaults to "green/green")
/// * `with_eta` - Whether to include ETA in the template
///
/// # Returns
/// A configured progress bar
pub fn create_progress_bar(
    total: u64,
    action: &str,
    color: Option<&str>,
    with_eta: bool,
) -> ProgressBar {
    let pb = ProgressBar::new(total);
    let color_str = color.unwrap_or("green/green");

    let styled_action = if action.contains("├╼") || action.contains("╰╼") {
        action
            .replace("├╼", &"├╼".cyan().dimmed().to_string())
            .replace("╰╼", &"╰╼".cyan().dimmed().to_string())
    } else {
        action.to_string()
    };

    let template = if with_eta {
        format!(
            "{}{{elapsed_precise}} {{bar:40.{}}} {{bytes}}/{{total_bytes}} (ETA: {{eta}})",
            styled_action, color_str
        )
    } else {
        format!(
            "{}{{elapsed_precise}} {{bar:40.{}}} {{bytes}}/{{total_bytes}}",
            styled_action, color_str
        )
    };

    pb.set_style(
        ProgressStyle::default_bar()
            .template(&template)
            .expect("Failed to set progress bar style")
            .progress_chars("▓▒░"),
    );

    pb
}

/// Create a spinner with braille animation
///
/// # Arguments
/// * `message` - Message to display next to the spinner
///
/// # Returns
/// A configured spinner
pub fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template(&format!("{} {}", "{spinner}".yellow().bold(), message))
            .expect("Failed to set spinner style"),
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(SPINNER_TICK_INTERVAL));
    spinner
}

/// Format a duration into a human-readable string
pub fn format_duration(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 {
        return format!("{}.{:02}s", total_secs, duration.subsec_millis() / 10);
    }

    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else {
        format!("{}m {}s", mins, secs)
    }
}

/// Format a size in bytes to a human-readable string
pub fn format_size(size: u64) -> String {
    if size < KB {
        format!("{}B", size)
    } else if size < MB {
        format!("{:.2}KB", size as f64 / KB as f64)
    } else if size < GB {
        format!("{:.2}MB", size as f64 / MB as f64)
    } else {
        format!("{:.2}GB", size as f64 / GB as f64)
    }
}

/// Format transfer rate to appropriate units
pub fn format_transfer_rate(bytes_per_sec: f64) -> (f64, &'static str) {
    let kb = KB as f64;
    let mb = MB as f64;
    let gb = GB as f64;

    if bytes_per_sec < kb {
        (bytes_per_sec, "B")
    } else if bytes_per_sec < mb {
        (bytes_per_sec / kb, "KB")
    } else if bytes_per_sec < gb {
        (bytes_per_sec / mb, "MB")
    } else {
        (bytes_per_sec / gb, "GB")
    }
}

/// Checks whether a filename stem matches a Windows reserved device name.
fn is_windows_reserved_name(name: &str) -> bool {
    const RESERVED_NAMES: &[&str] = &[
        "CON",
        "PRN",
        "AUX",
        "NUL",
        "CONIN$",
        "CONOUT$",
        "CLOCK$",
        "COM0",
        "COM1",
        "COM2",
        "COM3",
        "COM4",
        "COM5",
        "COM6",
        "COM7",
        "COM8",
        "COM9",
        "COM\u{00B9}",
        "COM\u{00B2}",
        "COM\u{00B3}",
        "LPT0",
        "LPT1",
        "LPT2",
        "LPT3",
        "LPT4",
        "LPT5",
        "LPT6",
        "LPT7",
        "LPT8",
        "LPT9",
        "LPT\u{00B9}",
        "LPT\u{00B2}",
        "LPT\u{00B3}",
    ];

    RESERVED_NAMES
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// Sanitizes a filename for cross-platform filesystem compatibility
///
/// Replaces characters that are invalid on Windows or Unix filesystems
/// with underscores, while preserving path separators and ensuring the
/// output is always a non-empty relative path of normal components.
///
/// Invalid characters and path hazards handled:
/// - Windows: `< > : " | ? * \` and control characters (0-31, 127)
/// - Unix: null character (\0)
/// - Both: leading whitespace, trailing whitespace/dots in path components,
///   dot-segment components (`.` and `..`), empty components, and empty paths
///
/// Also handles Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`,
/// `CONIN$`, `CONOUT$`, `CLOCK$`, `COM0`-`COM9`, `COM¹`-`COM³`, `LPT0`-`LPT9`,
/// `LPT¹`-`LPT³`), matched case-insensitively against the trimmed base name
/// before the first `.`, by appending an underscore before any extension.
///
/// # Arguments
/// * `filename` - The original filename (may include path components separated by `/`)
///
/// # Returns
/// * `(sanitized_filename, was_modified)` - Tuple of cleaned filename and whether it was changed
///
/// # Examples
/// ```
/// use ia_get::utils::sanitize_filename;
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
pub fn sanitize_filename(filename: &str) -> (String, bool) {
    let mut was_modified = false;
    let mut result = String::with_capacity(filename.len());

    // Process each path component separately to preserve directory structure
    let components: Vec<&str> = filename.split('/').collect();
    let mut first_component = true;

    for component in components.iter() {
        // Skip empty components (e.g., from leading/trailing slashes or "//" sequences)
        if component.is_empty() {
            if !filename.is_empty() {
                was_modified = true;
            }
            continue;
        }

        // Add separator before non-first components
        if !first_component {
            result.push('/');
        }
        first_component = false;

        let mut sanitized_component = String::with_capacity(component.len());

        // Replace invalid characters
        for ch in component.chars() {
            match ch {
                // Windows invalid characters
                '<' | '>' | ':' | '"' | '|' | '?' | '*' => {
                    sanitized_component.push('_');
                    was_modified = true;
                }
                // Backslash (path separator on Windows, invalid in filenames on Unix)
                '\\' => {
                    sanitized_component.push('_');
                    was_modified = true;
                }
                // Control characters (0-31) and DEL (127)
                '\x00'..='\x1F' | '\x7F' => {
                    sanitized_component.push('_');
                    was_modified = true;
                }
                // Valid character
                _ => sanitized_component.push(ch),
            }
        }

        // Trim leading whitespace and any trailing whitespace or dots in one pass
        let trimmed = sanitized_component
            .trim_start()
            .trim_end_matches(|c: char| c.is_whitespace() || c == '.');
        if trimmed.len() != sanitized_component.len() {
            was_modified = true;
            sanitized_component = trimmed.to_string();
        }

        // Rewrite dot-segment and empty post-trim components so no path traversal survives
        if sanitized_component.is_empty()
            || sanitized_component == "."
            || sanitized_component == ".."
        {
            sanitized_component = "_".to_string();
            was_modified = true;
        }

        // Check for Windows reserved names against the base name before the first '.'
        // after stripping trailing whitespace and dots from that base name
        let dot_pos = sanitized_component.find('.');
        let base_name = if let Some(pos) = dot_pos {
            &sanitized_component[..pos]
        } else {
            &sanitized_component
        };
        let trimmed_stem = base_name.trim_end_matches(|c: char| c.is_whitespace() || c == '.');

        if is_windows_reserved_name(trimmed_stem) {
            sanitized_component = if let Some(pos) = dot_pos {
                format!("{trimmed_stem}_{}", &sanitized_component[pos..])
            } else {
                format!("{trimmed_stem}_")
            };
            was_modified = true;
        }

        result.push_str(&sanitized_component);
    }

    if result.is_empty() {
        return ("_".to_string(), true);
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
    fn test_sanitize_trailing_dots() {
        let (result, modified) = sanitize_filename("file...");
        assert_eq!(result, "file");
        assert!(modified);

        let (result, modified) = sanitize_filename("folder./file.txt");
        assert_eq!(result, "folder/file.txt");
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

    fn assert_only_normal_components(path: &str) {
        let components: Vec<_> = std::path::Path::new(path).components().collect();
        assert!(
            !components.is_empty(),
            "expected non-empty path, got {:?}",
            path
        );
        assert!(
            components
                .iter()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "expected only Component::Normal in {:?}, got {:?}",
            path,
            components
        );
    }

    #[test]
    fn test_sanitize_windows_reserved_names_extended() {
        let cases = [
            ("CON .txt", "CON_.txt"),
            ("con  .tar.gz", "con_.tar.gz"),
            ("CON. ", "CON_"),
            ("COM0", "COM0_"),
            ("LPT0", "LPT0_"),
            ("COM\u{00B9}", "COM\u{00B9}_"),
            ("com\u{00B9} .bin", "com\u{00B9}_.bin"),
            ("COM\u{00B2}.txt", "COM\u{00B2}_.txt"),
            ("COM\u{00B3}.txt", "COM\u{00B3}_.txt"),
            ("LPT\u{00B9}", "LPT\u{00B9}_"),
            ("LPT\u{00B2}.dat", "LPT\u{00B2}_.dat"),
            ("lpt\u{00B3} .log", "lpt\u{00B3}_.log"),
            ("CONIN$", "CONIN$_"),
            ("CONIN$.log", "CONIN$_.log"),
            ("CONOUT$", "CONOUT$_"),
            ("conout$ .txt", "conout$_.txt"),
            ("CLOCK$", "CLOCK$_"),
            ("clock$.sys", "clock$_.sys"),
        ];

        for (input, expected) in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, expected, "input: {:?}", input);
            assert!(modified, "expected modified=true for {:?}", input);
            assert_only_normal_components(&result);
        }
    }

    #[test]
    fn test_sanitize_trailing_spaces_and_dots() {
        let cases = [
            ("file . .", "file"),
            ("CON. ", "CON_"),
            ("folder . /file . .", "folder/file"),
        ];

        for (input, expected) in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, expected, "input: {:?}", input);
            assert!(modified, "expected modified=true for {:?}", input);
            assert_only_normal_components(&result);
        }

        // Non-reserved names with spaces before '.' remain intact
        let (result, modified) = sanitize_filename("file .txt");
        assert_eq!(result, "file .txt");
        assert!(!modified);
        assert_only_normal_components(&result);
    }

    #[test]
    fn test_sanitize_dot_segments_and_empty_paths() {
        let cases = [
            (".", "_"),
            ("..", "_"),
            ("../evil.txt", "_/evil.txt"),
            ("a/../b.txt", "a/_/b.txt"),
            ("a/./b.txt", "a/_/b.txt"),
            ("a/.. /b.txt", "a/_/b.txt"),
            ("a/. ./b.txt", "a/_/b.txt"),
            ("/", "_"),
            ("///", "_"),
            ("", "_"),
        ];

        for (input, expected) in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, expected, "input: {:?}", input);
            assert!(modified, "expected modified=true for {:?}", input);
            assert_only_normal_components(&result);
        }
    }

    #[test]
    fn test_sanitize_reserved_lookalikes_unmodified() {
        let cases = ["COM10.txt", "LPT10.txt", "CONSOLE.txt", "CLOCK.txt"];

        for input in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, input, "input: {:?}", input);
            assert!(!modified, "expected modified=false for {:?}", input);
            assert_only_normal_components(&result);
        }
    }

    #[test]
    fn test_percent_decode_utf8_valid_inputs() {
        let cases = [
            ("", ""),
            ("plain_file-1.pdf", "plain_file-1.pdf"),
            ("sub/dir/file.pdf", "sub/dir/file.pdf"),
            ("file%20name.pdf", "file name.pdf"),
            ("part%231.pdf", "part#1.pdf"),
            ("what%3F.txt", "what?.txt"),
            ("100%25.txt", "100%.txt"),
            ("a%20b%23c%3Fd%25e", "a b#c?d%e"),
            ("a+b/file+name%201.pdf", "a+b/file+name 1.pdf"),
            ("%2f%2F%3a%3A%3f%3F", "//::??"),
            ("caf%c3%a9.pdf", "café.pdf"),
            ("caf%C3%A9.pdf", "café.pdf"),
            ("%D1%84%D0%B0%D0%B9%D0%BB.txt", "файл.txt"),
            ("%E6%96%87%E4%BB%B6.txt", "文件.txt"),
            ("emoji%F0%9F%98%80.txt", "emoji😀.txt"),
        ];

        for (input, expected) in cases {
            let decoded = percent_decode_utf8(input)
                .unwrap_or_else(|err| panic!("expected Ok for {input:?}, got {err:?}"));
            assert_eq!(decoded, expected, "input: {input:?}");
        }
    }

    #[test]
    fn test_percent_decode_utf8_rejects_invalid_inputs() {
        let cases = [
            "%", "file%", "%2", "file%2", "%2G", "%G2", "%ZZ", "%FF", "%C3%28", "%00", "a%00b",
            "\0", "a\0b",
        ];

        for input in cases {
            let err = percent_decode_utf8(input).unwrap_err();
            assert!(
                matches!(err, IaGetError::UrlFormat(_)),
                "expected UrlFormat for {input:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn test_parse_archive_url_valid_details_and_derived_urls() {
        let cases = [
            (
                "https://archive.org/details/my-item_1.0@org",
                "my-item_1.0@org",
            ),
            ("https://archive.org/details/my-item/", "my-item"),
            ("https://archive.org/details/my-item?tab=files", "my-item"),
            ("https://archive.org/details/my-item#description", "my-item"),
            (
                "https://archive.org/details/my-item/?view=theater#page/n1",
                "my-item",
            ),
        ];

        for (url, expected_id) in cases {
            let parsed = parse_archive_url(url)
                .unwrap_or_else(|err| panic!("expected Ok for {url:?}, got {err:?}"));
            assert_eq!(
                parsed,
                ArchiveUrl::Details {
                    identifier: expected_id.to_string(),
                },
                "url: {url:?}"
            );
            assert_eq!(parsed.identifier(), expected_id, "url: {url:?}");
            assert_eq!(parsed.file_path(), None, "url: {url:?}");
            assert_eq!(
                parsed.details_url(),
                format!("https://archive.org/details/{expected_id}"),
                "url: {url:?}"
            );
            assert_eq!(
                parsed.xml_url(),
                format!("https://archive.org/download/{expected_id}/{expected_id}_files.xml"),
                "url: {url:?}"
            );
            assert!(
                validate_archive_url(url).is_ok(),
                "validate_archive_url should accept {url:?}"
            );
        }
    }

    #[test]
    fn test_parse_archive_url_valid_download_and_derived_urls() {
        let cases = [
            (
                "https://archive.org/download/item1/file.pdf",
                "item1",
                "file.pdf",
            ),
            (
                "https://archive.org/download/item1/sub/dir/file.pdf",
                "item1",
                "sub/dir/file.pdf",
            ),
            (
                "https://archive.org/download/item1/My%20Folder/file%20name+1.pdf",
                "item1",
                "My Folder/file name+1.pdf",
            ),
            (
                "https://archive.org/download/item1/sub/caf%C3%A9.pdf",
                "item1",
                "sub/café.pdf",
            ),
            (
                "https://archive.org/download/item1/%D1%84%D0%B0%D0%B9%D0%BB.txt",
                "item1",
                "файл.txt",
            ),
            (
                "https://archive.org/download/item1/notes%3Fv=1%23top%2520.txt",
                "item1",
                "notes?v=1#top%20.txt",
            ),
            (
                "https://archive.org/download/item1/sub/file%20name.pdf?download=1#page=2",
                "item1",
                "sub/file name.pdf",
            ),
            (
                "https://archive.org/download/item1/book.epub?foo=bar&baz=1",
                "item1",
                "book.epub",
            ),
            (
                "https://archive.org/download/item1/book.epub#chapter-3",
                "item1",
                "book.epub",
            ),
        ];

        for (url, expected_id, expected_path) in cases {
            let parsed = parse_archive_url(url)
                .unwrap_or_else(|err| panic!("expected Ok for {url:?}, got {err:?}"));
            assert_eq!(
                parsed,
                ArchiveUrl::Download {
                    identifier: expected_id.to_string(),
                    file_path: expected_path.to_string(),
                },
                "url: {url:?}"
            );
            assert_eq!(parsed.identifier(), expected_id, "url: {url:?}");
            assert_eq!(parsed.file_path(), Some(expected_path), "url: {url:?}");
            assert_eq!(
                parsed.details_url(),
                format!("https://archive.org/details/{expected_id}"),
                "url: {url:?}"
            );
            assert_eq!(
                parsed.xml_url(),
                format!("https://archive.org/download/{expected_id}/{expected_id}_files.xml"),
                "url: {url:?}"
            );
            assert!(
                validate_archive_url(url).is_ok(),
                "validate_archive_url should accept {url:?}"
            );
        }
    }

    #[test]
    fn test_parse_archive_url_rejects_invalid_urls() {
        assert!(!is_valid_identifier("."));
        assert!(!is_valid_identifier(".."));

        let cases = [
            // Non-HTTPS schemes
            "http://archive.org/details/item",
            "http://archive.org/download/item/file.pdf",
            "ftp://archive.org/details/item",
            // Non-archive.org hosts
            "https://example.com/details/item",
            "https://www.archive.org/details/item",
            "https://www.archive.org/download/item/file.pdf",
            "https://archive.org.evil.com/download/item/file.pdf",
            // Explicit non-default port
            "https://archive.org:8443/details/item",
            "https://archive.org:8443/download/item/file.pdf",
            // Userinfo
            "https://user@archive.org/details/item",
            "https://user:pass@archive.org/download/item/file.pdf",
            "https://:pass@archive.org/details/item",
            // Unknown path prefixes
            "https://archive.org/",
            "https://archive.org/metadata/item",
            "https://archive.org/stream/item/file.pdf",
            // Dot identifiers
            "https://archive.org/details/.",
            "https://archive.org/details/..",
            "https://archive.org/download/./file.pdf",
            "https://archive.org/download/../file.pdf",
            // Malformed /details/ paths
            "https://archive.org/details/",
            "https://archive.org/details/item/extra",
            "https://archive.org/details/item//",
            "https://archive.org/details/bad*item",
            // Malformed /download/ paths
            "https://archive.org/download/item",
            "https://archive.org/download/item/",
            "https://archive.org/download/item/?q=1",
            "https://archive.org/download/item/sub/",
            "https://archive.org/download/item//file.pdf",
            "https://archive.org/download/item/sub//file.pdf",
            "https://archive.org/download/bad*item/file.pdf",
            // Malformed percent escapes and decoded slash hazards
            "https://archive.org/download/item/file%2G.pdf",
            "https://archive.org/download/item/file%2",
            "https://archive.org/download/item/file%FF.pdf",
            "https://archive.org/download/item/file%00.pdf",
            "https://archive.org/download/item/%2Ffile.pdf",
            "https://archive.org/download/item/sub%2F%2Ffile.pdf",
            "https://archive.org/download/item/file.pdf%2F",
            // Unparseable URLs
            "",
            "not-a-url",
        ];

        for url in cases {
            let parse_err = parse_archive_url(url).unwrap_err();
            assert!(
                matches!(parse_err, IaGetError::UrlFormat(_)),
                "expected UrlFormat from parse_archive_url({url:?}), got {parse_err:?}"
            );
            let validate_err = validate_archive_url(url).unwrap_err();
            assert!(
                matches!(validate_err, IaGetError::UrlFormat(_)),
                "expected UrlFormat from validate_archive_url({url:?}), got {validate_err:?}"
            );
        }
    }
}
