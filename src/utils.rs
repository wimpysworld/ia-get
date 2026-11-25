//! Utility functions for ia-get.

use crate::constants::URL_PATTERN;
use crate::{IaGetError, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use std::sync::LazyLock;

/// Spinner tick interval in milliseconds
pub const SPINNER_TICK_INTERVAL: u64 = 100;

/// Size constants for formatting
const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;

/// Compiled regex for URL validation (initialized once)
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(URL_PATTERN).expect("Invalid URL regex pattern"));

/// Validates an archive.org details URL format
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
/// assert!(validate_archive_url("https://example.com/invalid").is_err());
/// ```
pub fn validate_archive_url(url: &str) -> Result<()> {
    if URL_REGEX.is_match(url) {
        // Further check: ensure there's an identifier after "details/"
        // and that the identifier is not empty.
        if let Some(path_segment) = url.split("/details/").nth(1) {
            if !path_segment.trim_end_matches('/').is_empty() {
                return Ok(());
            }
        }
    }
    Err(IaGetError::UrlFormat(url.to_string()))
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
}
