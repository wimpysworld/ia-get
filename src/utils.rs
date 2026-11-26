//! Utility functions for ia-get.

use crate::constants::IDENTIFIER_PATTERN;
use crate::{IaGetError, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use regex::Regex;
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, LazyLock, Mutex};
use url::Url;

/// Spinner tick interval in milliseconds
const SPINNER_TICK_INTERVAL: u64 = 100;

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

/// Returns the shared draw target used by all `indicatif` progress indicators.
///
/// Routing progress bars and spinners to `stdout` keeps `stderr` clean for
/// `--verbose` diagnostic logs so `2> ia-get.log` captures diagnostics without
/// terminal control codes or progress noise.
pub(crate) fn progress_draw_target() -> ProgressDrawTarget {
    ProgressDrawTarget::stdout()
}

/// Sanitises a URL for diagnostic logging by redacting any embedded userinfo
/// (`username:password@`) while keeping scheme, host, port, path, query, and
/// fragment intact.
pub fn sanitize_url_for_log(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.as_str().to_string();
    }

    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    let serialized = sanitized.as_str();
    let prefix = format!("{}://", sanitized.scheme());
    if let Some(rest) = serialized.strip_prefix(&prefix) {
        format!("{prefix}<redacted>@{rest}")
    } else {
        serialized.to_string()
    }
}

#[derive(Clone)]
enum LogSink {
    Disabled,
    Stderr,
    Writer(Arc<Mutex<Box<dyn Write + Send>>>),
}

impl fmt::Debug for LogSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::Stderr => f.write_str("Stderr"),
            Self::Writer(_) => f.write_str("Writer(..)"),
        }
    }
}

/// Structured single-line diagnostic logger for `--verbose` output.
///
/// Emits plain text `[verbose] <category>: <message>` lines to `stderr` or an
/// injected writer, with no ANSI escape codes. Write errors (such as broken
/// pipes) are ignored so diagnostic output never aborts a transfer.
#[derive(Clone, Debug)]
pub struct VerboseLogger {
    sink: LogSink,
}

impl Default for VerboseLogger {
    fn default() -> Self {
        Self::disabled()
    }
}

impl VerboseLogger {
    /// Creates a disabled logger that drops all messages without formatting.
    pub fn disabled() -> Self {
        Self {
            sink: LogSink::Disabled,
        }
    }

    /// Creates a logger that writes diagnostic lines to `stderr`.
    pub fn stderr() -> Self {
        Self {
            sink: LogSink::Stderr,
        }
    }

    /// Creates a `stderr` logger when `enabled` is `true`, or a disabled logger otherwise.
    pub fn from_enabled(enabled: bool) -> Self {
        if enabled {
            Self::stderr()
        } else {
            Self::disabled()
        }
    }

    /// Creates a logger that writes diagnostic lines to an arbitrary writer.
    pub fn with_writer<W: Write + Send + 'static>(writer: W) -> Self {
        Self {
            sink: LogSink::Writer(Arc::new(Mutex::new(Box::new(writer)))),
        }
    }

    /// Returns `true` when diagnostic logging is enabled.
    pub fn is_enabled(&self) -> bool {
        !matches!(self.sink, LogSink::Disabled)
    }

    /// Emits a single plain-text diagnostic line when enabled.
    ///
    /// Embedded carriage returns and newlines in `message` are normalised to
    /// spaces so every log record stays on a single line.
    pub fn log(&self, category: &str, message: impl fmt::Display) {
        if !self.is_enabled() {
            return;
        }

        let raw = message.to_string();
        let clean = if raw.contains(['\r', '\n']) {
            raw.replace(['\r', '\n'], " ")
        } else {
            raw
        };
        let line = format!("[verbose] {category}: {clean}\n");

        match &self.sink {
            LogSink::Disabled => {}
            LogSink::Stderr => {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(line.as_bytes());
                let _ = stderr.flush();
            }
            LogSink::Writer(writer) => {
                let mut guard = writer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let _ = guard.write_all(line.as_bytes());
                let _ = guard.flush();
            }
        }
    }
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
    let pb = ProgressBar::with_draw_target(Some(total), progress_draw_target());
    let color_str = color.unwrap_or("green/green");

    let styled_action = if action.contains("├╼") || action.contains("╰╼") {
        action
            .replace("├╼", &"├╼".cyan().dimmed().to_string())
            .replace("╰╼", &"╰╼".cyan().dimmed().to_string())
    } else {
        action.to_string()
    };

    let eta_suffix = if with_eta { " (ETA: {eta})" } else { "" };
    let template = format!(
        "{styled_action}{{elapsed_precise}} {{bar:40.{color_str}}} {{bytes}}/{{total_bytes}}{eta_suffix}"
    );

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
    let spinner = ProgressBar::with_draw_target(None, progress_draw_target());
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
        format!("{size}B")
    } else {
        let (value, unit) = format_transfer_rate(size as f64);
        format!("{value:.2}{unit}")
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

/// Formats a plain-text end-of-run summary, including any failed files.
pub(crate) fn format_run_summary<'a, D>(
    heading: &str,
    ok: usize,
    skipped: Option<usize>,
    failed: impl IntoIterator<Item = (&'a str, D)>,
    interrupted: bool,
) -> String
where
    D: fmt::Display,
{
    let failed: Vec<(&str, D)> = failed.into_iter().collect();
    let mut text = match skipped {
        Some(skipped) => format!(
            "{heading} {ok} ok, {skipped} skipped, {} failed",
            failed.len()
        ),
        None => format!("{heading} {ok} ok, {} failed", failed.len()),
    };
    if interrupted {
        text.push_str(", interrupted");
    }

    if !failed.is_empty() {
        text.push_str("\nFailed files:");
        for (file_path, detail) in failed {
            text.push_str(&format!("\n  - {file_path}: {detail}"));
        }
    }

    text
}

/// Prints a coloured end-of-run summary to stdout.
pub(crate) fn print_run_summary<'a, D>(
    heading: &str,
    ok: usize,
    skipped: Option<usize>,
    failed: impl IntoIterator<Item = (&'a str, D)>,
    interrupted: bool,
) where
    D: fmt::Display,
{
    let failed: Vec<(&str, D)> = failed.into_iter().collect();
    let status_icon = if interrupted || !failed.is_empty() {
        "✘".red().bold()
    } else {
        "✔".green().bold()
    };

    let failed_count = if failed.is_empty() {
        failed.len().to_string().bold()
    } else {
        failed.len().to_string().red().bold()
    };

    let interrupted_suffix = if interrupted {
        format!(", {}", "interrupted".yellow().bold())
    } else {
        String::new()
    };

    let skipped_segment = match skipped {
        Some(skipped) => format!(" {} skipped,", skipped.to_string().cyan().bold()),
        None => String::new(),
    };

    println!(
        "\n{} {} {} ok,{} {} failed{}",
        status_icon,
        heading.bold(),
        ok.to_string().green().bold(),
        skipped_segment,
        failed_count,
        interrupted_suffix
    );

    if !failed.is_empty() {
        println!("{}", "Failed files:".red().bold());
        for (file_path, detail) in failed {
            println!("  {} {}: {}", "✘".red().bold(), file_path.bold(), detail);
        }
    }
}

/// Checks whether a filename stem matches a Windows reserved device name.
pub(crate) fn is_windows_reserved_name(name: &str) -> bool {
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

        // Replace invalid characters: Windows invalid chars, backslash, control chars (0-31), and DEL (127)
        for ch in component.chars() {
            match ch {
                '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '\x00'..='\x1F' | '\x7F' => {
                    sanitized_component.push('_');
                    was_modified = true;
                }
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
    use crate::test_support::CapturedLogs;

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
    fn test_sanitize_filename_unmodified_inputs() {
        let cases = [
            "normal_file-name.txt",
            "folder/subfolder/file.txt",
            "файл.txt",
            "文件.txt",
            "emoji😀.txt",
            // Non-reserved names with spaces before '.' remain intact
            "file .txt",
            // Reserved-name lookalikes remain intact
            "COM10.txt",
            "LPT10.txt",
            "CONSOLE.txt",
            "CLOCK.txt",
        ];

        for input in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, input, "input: {input:?}");
            assert!(!modified, "expected modified=false for {input:?}");
            assert_only_normal_components(&result);
        }
    }

    #[test]
    fn test_sanitize_filename_modified_inputs() {
        let cases = [
            ("file?name:test<>.txt", "file_name_test__.txt"),
            ("Season 1/Episode?.mp4", "Season 1/Episode_.mp4"),
            ("Folder:Name/File*Name?.txt", "Folder_Name/File_Name_.txt"),
            ("???", "___"),
            ("file:name.tar.gz", "file_name.tar.gz"),
            (
                "Red vs. Blue - Season 1/Episode 1: Why Are We Here?.mp4",
                "Red vs. Blue - Season 1/Episode 1_ Why Are We Here_.mp4",
            ),
            ("file\x00\x1fname.txt", "file__name.txt"),
            ("test\x7Ffile.txt", "test_file.txt"),
            ("folder\\file.txt", "folder_file.txt"),
            (" leading.txt ", "leading.txt"),
            ("folder/ spaces /file.txt", "folder/spaces/file.txt"),
            ("file...", "file"),
            ("folder./file.txt", "folder/file.txt"),
            ("file . .", "file"),
            ("folder . /file . .", "folder/file"),
            ("folder//file.txt", "folder/file.txt"),
            ("/folder/file.txt", "folder/file.txt"),
            ("folder/file.txt/", "folder/file.txt"),
        ];

        for (input, expected) in cases {
            let (result, modified) = sanitize_filename(input);
            assert_eq!(result, expected, "input: {input:?}");
            assert!(modified, "expected modified=true for {input:?}");
            assert_only_normal_components(&result);
        }
    }

    #[test]
    fn test_sanitize_filename_windows_reserved_and_dot_segments() {
        let cases = [
            // Classic Windows device names
            ("CON.txt", "CON_.txt"),
            ("con.txt", "con_.txt"),
            ("PRN", "PRN_"),
            ("aux.log", "aux_.log"),
            ("COM1.dat", "COM1_.dat"),
            ("LPT9.txt", "LPT9_.txt"),
            ("folder/CON.txt", "folder/CON_.txt"),
            // Extended Windows device names, trailing spaces/dots, superscripts
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
            // Dot segments and empty paths
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
            assert_eq!(result, expected, "input: {input:?}");
            assert!(modified, "expected modified=true for {input:?}");
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
        }
    }

    #[test]
    fn test_progress_bar_and_spinner_target_stdout() {
        let target = progress_draw_target();
        let expected_stdout = ProgressDrawTarget::stdout();
        let stderr_target = ProgressDrawTarget::stderr();

        fn strip_rate_limiter(debug: &str) -> &str {
            debug
                .split_once("rate_limiter:")
                .map_or(debug, |(prefix, _)| prefix)
        }

        let actual_debug = format!("{target:?}");
        let stdout_debug = format!("{expected_stdout:?}");
        let stderr_debug = format!("{stderr_target:?}");

        assert_eq!(
            strip_rate_limiter(&actual_debug),
            strip_rate_limiter(&stdout_debug)
        );
        assert_eq!(target.is_hidden(), expected_stdout.is_hidden());
        assert!(
            !actual_debug.contains("Stderr"),
            "progress draw target must never target stderr: {actual_debug}"
        );
        if !expected_stdout.is_hidden() {
            assert!(
                actual_debug.contains("Stdout"),
                "when stdout is a terminal, progress draw target must target stdout: {actual_debug}"
            );
        }
        if !expected_stdout.is_hidden() || !stderr_target.is_hidden() {
            assert_ne!(
                strip_rate_limiter(&actual_debug),
                strip_rate_limiter(&stderr_debug),
                "when stdout or stderr is a terminal, progress_draw_target() must differ from stderr"
            );
        } else {
            assert!(
                target.is_hidden(),
                "when neither stdout nor stderr is a terminal, progress target must be hidden"
            );
        }

        let pb_with_eta = create_progress_bar(100, "╰╼ Downloading  ", None, true);
        assert_eq!(pb_with_eta.is_hidden(), expected_stdout.is_hidden());
        pb_with_eta.inc(50);
        pb_with_eta.finish_and_clear();

        let pb_no_eta = create_progress_bar(100, "├╼ Verifying    ", Some("cyan/blue"), false);
        assert_eq!(pb_no_eta.is_hidden(), expected_stdout.is_hidden());
        pb_no_eta.inc(100);
        pb_no_eta.finish_and_clear();

        let spinner = create_spinner("Testing spinner lifecycle");
        assert_eq!(spinner.is_hidden(), expected_stdout.is_hidden());
        spinner.set_message("Updated spinner message");
        spinner.finish_and_clear();
    }

    #[test]
    fn test_sanitize_url_for_log_redacts_userinfo_and_preserves_plain_urls() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "https://alice:s3cr3t@archive.org/download/item/file.bin?token=1#top",
                "https://<redacted>@archive.org/download/item/file.bin?token=1#top",
                &["alice", "s3cr3t"],
            ),
            (
                "https://bob@archive.org/details/item",
                "https://<redacted>@archive.org/details/item",
                &["bob"],
            ),
            (
                "http://:onlypass@127.0.0.1:8080/protected/file.bin?a=1&b=2",
                "http://<redacted>@127.0.0.1:8080/protected/file.bin?a=1&b=2",
                &["onlypass"],
            ),
            (
                "https://archive.org/details/deftributetozzap64",
                "https://archive.org/details/deftributetozzap64",
                &[],
            ),
            (
                "http://127.0.0.1:9000/download/item/my%20file.pdf?download=1&foo=bar#frag",
                "http://127.0.0.1:9000/download/item/my%20file.pdf?download=1&foo=bar#frag",
                &[],
            ),
        ];

        for (raw, expected, forbidden) in cases {
            let parsed = Url::parse(raw).unwrap();
            let sanitized = sanitize_url_for_log(&parsed);
            assert_eq!(sanitized, *expected, "raw: {raw}");
            for token in *forbidden {
                assert!(
                    !sanitized.contains(token),
                    "forbidden token leaked in sanitized URL"
                );
            }
        }
    }

    #[test]
    fn test_verbose_logger_disabled_is_silent_and_enabled_has_no_ansi_codes() {
        let disabled = VerboseLogger::disabled();
        assert!(!disabled.is_enabled());
        disabled.log("http", "this must be dropped silently");

        let from_false = VerboseLogger::from_enabled(false);
        assert!(!from_false.is_enabled());
        from_false.log("download", "this must also be dropped silently");

        let default_logger = VerboseLogger::default();
        assert!(!default_logger.is_enabled());

        assert!(VerboseLogger::stderr().is_enabled());
        assert!(VerboseLogger::from_enabled(true).is_enabled());

        let captured = CapturedLogs::new();
        let enabled = VerboseLogger::with_writer(captured.clone());
        assert!(enabled.is_enabled());

        enabled.log(
            "http",
            "-> GET https://archive.org/details/item (cookies=no)",
        );
        enabled.log("download", "line1\nline2\r\nline3\rline4");

        let raw_bytes = captured.as_bytes();
        assert!(
            !raw_bytes.contains(&0x1b),
            "verbose logger must not emit ANSI escape bytes: {raw_bytes:?}"
        );

        assert_eq!(
            captured.lines(),
            vec![
                "[verbose] http: -> GET https://archive.org/details/item (cookies=no)",
                "[verbose] download: line1 line2  line3 line4",
            ]
        );
    }
}
