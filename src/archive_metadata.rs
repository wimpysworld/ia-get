use crate::cookie::CookieSource;
use crate::display::{format_size, print_mtime_warning};
use crate::downloader::{parse_last_modified, send_following_redirects, sync_file_mtime};
use crate::fs::ensure_regular_or_absent;
use crate::{IaGetError, Result};
use colored::*;
use regex::Regex;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_xml_rs::from_str;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

/// Maximum length for XML content in debug output (characters)
const XML_DEBUG_TRUNCATE_LEN: usize = 1000;

/// Regex pattern for accepted archive.org URLs: a details page (whole
/// item) or a download URL optionally naming a single file inside the
/// item. Group 1 is the identifier; group 2 (optional) is the raw, still
/// percent-encoded file path.
const URL_PATTERN: &str =
    r"^https://archive\.org/(?:details|download)/([a-zA-Z0-9_\-.@]+)(?:/(.+))?/?$";

/// Compiled regex for URL validation (initialized once)
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(URL_PATTERN).expect("Invalid URL regex pattern"));

/// What an accepted archive.org URL points at: a whole item — a details
/// page, or a download URL with no file path — or a single file inside
/// an item, named by a download URL with a path after the identifier.
#[derive(Debug, Clone)]
pub struct ArchiveTarget {
    /// The archive.org item identifier
    pub identifier: String,
    /// The file's path inside the item (percent-decoded) for a
    /// single-file URL; `None` for a whole-item URL.
    pub file_path: Option<String>,
}

/// Root structure for parsing the XML files list from archive.org
/// The actual XML structure has a `files` root element containing multiple `file` elements
#[derive(Deserialize, Debug)]
pub struct XmlFiles {
    #[serde(rename = "file", default)]
    pub files: Vec<XmlFile>,
}

/// Represents a single file entry from the archive.org `_files.xml` metadata.
///
/// Archive.org XML structure has both attributes and nested elements:
/// ```xml
/// <file name="..." source="...">
///   <mtime>...</mtime>
///   <size>...</size>
///   <md5>...</md5>
///   ...
/// </file>
/// ```
///
/// All fields except `name` are optional: archive.org omits elements when
/// the value is not available (e.g. `md5` for very large files, `btih` for
/// non-torrent items).
///
/// Fields like `crc32`, `sha1`, `btih`, `summation`, `original`, `rotation`,
/// and `format` are not yet used by the download logic but are captured so
/// the full `_files.xml` schema is available for future features (alternative
/// integrity checks, deduplication by hash, format filtering, etc.) without
/// needing a second fetch.
#[derive(Deserialize, Debug, Default)]
pub struct XmlFile {
    #[serde(rename = "@name")]
    pub name: String,
    // Optional: a missing @source attribute must not break parsing of the
    // whole document.
    #[serde(rename = "@source", default)]
    pub source: Option<String>,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
    pub format: Option<String>,
    pub rotation: Option<u32>,
    pub md5: Option<String>,
    pub crc32: Option<String>,
    pub sha1: Option<String>,
    pub btih: Option<String>,
    pub summation: Option<String>,
    pub original: Option<String>,
}

/// Extracts the 1-based line number from a `serde-xml-rs` error, whose
/// display carries the position as `Reader: line:column message`.
///
/// The layout is parser-specific: if `serde-xml-rs` ever changes its
/// display format, this yields `None` and the preview falls back to the
/// document head, so a dependency update degrades gracefully.
///
/// Returns `None` when the parser reported no position; the preview then
/// falls back to the document head.
fn parse_error_line(error: &str) -> Option<usize> {
    let rest = error.strip_prefix("Reader: ")?;
    let position = rest.split_once(' ')?.0;
    let line = position.split_once(':')?.0;
    line.parse().ok().filter(|line: &usize| *line >= 1)
}

/// Builds the numbered line preview shown when a parse fails: the few
/// lines around the reported error line (marked with `»`) when the
/// parser names one, otherwise the document head.
///
/// Each line is capped at `XML_DEBUG_TRUNCATE_LEN` characters so a
/// minified multi-megabyte line cannot blow up the error message.
fn error_context(xml_content: &str, line: Option<usize>) -> String {
    // Count the lines without materializing them: the document can be
    // large, and only the handful of lines around the anchor are shown.
    let newlines = xml_content.bytes().filter(|&byte| byte == b'\n').count();
    let line_count = if xml_content.is_empty() {
        0
    } else if xml_content.ends_with('\n') {
        newlines
    } else {
        newlines + 1
    };
    let last_line = line_count.max(1);
    let anchor = line.unwrap_or(1).min(last_line);
    let first = anchor.saturating_sub(3).max(1);
    let last = (first + 5).min(last_line);

    // Advance to the first line to show; every line is read at most once
    let mut lines = xml_content.lines();
    for _ in 0..first.saturating_sub(1) {
        lines.next();
    }

    let mut out = String::new();
    for number in first..=last {
        let raw = lines.next().unwrap_or("").trim_end();
        let mut end = XML_DEBUG_TRUNCATE_LEN.min(raw.len());
        while end < raw.len() && !raw.is_char_boundary(end) {
            end -= 1;
        }
        let marker = if number == anchor { "»" } else { " " };
        out.push_str(&format!("{marker}{number:>6}  {}\n", &raw[..end]));
    }
    out
}

/// Parses XML content into XmlFiles structure with improved error context
///
/// # Arguments
/// * `xml_content` - Raw XML content string from archive.org
///
/// # Returns
/// * `Ok(XmlFiles)` if parsing succeeds
/// * `Err(IaGetError)` naming the failing line and showing the lines
///   around it, so a late entry in a large document is locatable
pub fn parse_xml_files(xml_content: &str) -> Result<XmlFiles> {
    from_str(xml_content).map_err(|e| {
        let message = e.to_string();
        IaGetError::XmlParsing(format!(
            "Failed to parse _files.xml metadata: {}. Context:\n{}",
            message,
            error_context(xml_content, parse_error_line(&message))
        ))
    })
}

/// Saves the raw `_files.xml` document to `path`, overwriting any existing
/// copy, and syncs its last-modified time with the server's
/// `Last-Modified` header when present.
///
/// The time is never taken from the document itself: its self-entry carries
/// unreliable metadata. Failing to set the time is not fatal, mirroring the
/// download batch.
pub fn save_xml_metadata(
    path: &Path,
    content: &str,
    last_modified: Option<SystemTime>,
) -> Result<()> {
    // Refuse to write through a pre-planted symlink named "<id>_files.xml"
    // (fs::write would silently truncate whatever the link points at) or
    // into a pre-planted FIFO (a write would block or reach the special
    // target).
    ensure_regular_or_absent(path)?;
    std::fs::write(path, content).map_err(|e| crate::error::io_error_with_path(path, e))?;

    if let Some(target) = last_modified
        && let Err(e) = sync_file_mtime(path, target)
    {
        // Best-effort: the save succeeded, only the time sync failed
        print_mtime_warning(&e.to_string());
    }

    Ok(())
}

/// Timeout for the HEAD accessibility check
const HEAD_CHECK_TIMEOUT_SECS: u64 = 60;

/// Checks if a URL is accessible by sending a HEAD request.
///
/// Only a definitive 404/410 is fatal: the resource does not exist or is
/// permanently gone, and the GET would fail the same way. Other failures
/// (a proxy that rejects HEAD with 405, a transient 500, a connection
/// error) are not fatal — the GET that follows gives the authoritative
/// answer.
pub async fn is_url_accessible(
    url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
) -> Result<()> {
    let response = match send_following_redirects(
        client,
        reqwest::Method::HEAD,
        url,
        None,
        cookie_source,
        Some(Duration::from_secs(HEAD_CHECK_TIMEOUT_SECS)),
    )
    .await
    {
        Ok(resp) => resp,
        // Connection-level failure (DNS, TLS, timeout): the GET below will
        // produce the same error with a proper message, so just proceed.
        Err(_) => return Ok(()),
    };

    let status = response.status();
    if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
        return Err(IaGetError::Network {
            detail: format!(
                "XML metadata not found (HTTP {status}): the archive identifier may be incorrect"
            ),
            source: None,
        });
    }
    Ok(())
}

/// Parses an accepted archive.org URL into its target: the item
/// identifier plus, for a URL with a file path, the single file it names
/// (percent-decoded, leading and trailing separators trimmed away).
///
/// Accepts `https://archive.org/details/<identifier>` (a whole item) and
/// `https://archive.org/download/<identifier>/<file>` (one file); a
/// details URL with a path after the identifier is read the same way as a
/// download one. A query or fragment is URL structure, never file-name
/// content (a name's `?`/`#` would arrive percent-encoded), so it is
/// dropped before matching.
///
/// # Arguments
/// * `url` - The URL to parse
///
/// # Returns
/// * `Ok(ArchiveTarget)` if the URL is valid
/// * `Err(IaGetError::UrlFormat)` if the URL format is invalid
///
/// # Examples
/// ```
/// use ia_get::archive_metadata::parse_archive_url;
///
/// let target = parse_archive_url("https://archive.org/details/valid-item")
///     .expect("a details URL");
/// assert_eq!(target.identifier, "valid-item");
/// assert!(target.file_path.is_none());
///
/// let target = parse_archive_url("https://archive.org/download/valid-item/scan/01%20.pdf")
///     .expect("a download URL");
/// assert_eq!(target.file_path.as_deref(), Some("scan/01 .pdf"));
///
/// assert!(parse_archive_url("https://example.com/invalid").is_err());
/// ```
pub fn parse_archive_url(url: &str) -> Result<ArchiveTarget> {
    // A literal '?' or '#' delimits the query/fragment (RFC 3986); only
    // the path names the item and the file
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let Some(caps) = URL_REGEX.captures(path) else {
        return Err(IaGetError::UrlFormat(url.to_string()));
    };

    let identifier = caps[1].to_string();
    let file_path = match caps.get(2) {
        None => None,
        Some(raw) => {
            // A path that decodes to nothing readable names no file
            let Some(decoded) = percent_decode(raw.as_str()) else {
                return Err(IaGetError::UrlFormat(url.to_string()));
            };
            // Leading and trailing separators do not belong to a file name
            let path = decoded.trim_matches('/');
            if path.is_empty() {
                return Err(IaGetError::UrlFormat(url.to_string()));
            }
            Some(path.to_string())
        }
    };

    Ok(ArchiveTarget {
        identifier,
        file_path,
    })
}

/// Percent-decodes the raw file path a URL carries. An escape without two
/// hex digits after the '%' (e.g. a lone '%') passes through literally;
/// `None` is returned when the decoded bytes are not a valid UTF-8 name.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && let Ok(byte) = u8::from_str_radix(&raw[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Builds the XML files-list URL for an item identifier:
/// "https://archive.org/download/<identifier>/<identifier>_files.xml"
///
/// # Arguments
/// * `identifier` - The archive.org item identifier (already validated by
///   `parse_archive_url`)
///
/// # Returns
/// The corresponding XML files list URL
pub fn get_xml_url(identifier: &str) -> Result<Url> {
    Url::parse(&format!(
        "https://archive.org/download/{identifier}/{identifier}_files.xml"
    ))
    .map_err(IaGetError::from)
}

/// Percent-encodes every path segment of a file name, preserving internal `/`
/// separators, so that URL-special characters in the name (`?`, `#`, `%`,
/// spaces, non-ASCII, ...) cannot be misread as a query string or fragment
/// by `Url::join`. Leading `/` characters are dropped, so the result always
/// stays a relative reference: a name like `//host/x` would otherwise be
/// joined against a *different host*.
///
/// This is the RFC 3986 "unreserved" set, not form-URL-encoding: a space
/// must become `%20` (never `+`), which is why a small encoder is written
/// here instead of reusing `form_urlencoded::byte_serialize`.
#[must_use]
pub fn encode_download_path(name: &str) -> String {
    let mut out = String::new();
    for segment in name.split('/') {
        // Dot segments would be silently resolved by Url::join (a "../"
        // name would escape the item directory); drop them up front so
        // the encoded reference matches what is actually requested.
        if segment == "." || segment == ".." {
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        for ch in segment.chars() {
            match ch {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
                _ => {
                    let mut buf = [0u8; 4];
                    for b in ch.encode_utf8(&mut buf).as_bytes() {
                        out.push_str(&format!("%{b:02X}"));
                    }
                }
            }
        }
    }
    out
}

/// The `_files.xml` entry name: the last path segment of the XML URL
/// (which always ends in "<identifier>_files.xml", see get_xml_url).
///
/// Total by construction: a `Url`'s path always splits into at least one
/// segment, so the last one is always present (possibly empty for a
/// host-only URL, which this function is never called with).
pub fn xml_file_name_of(url: &Url) -> &str {
    url.path().rsplit('/').next().unwrap_or("")
}

/// XML metadata response: the parsed file list plus the raw data needed to
/// persist the `_files.xml` document locally.
#[derive(Debug)]
pub struct XmlMetadata {
    /// The parsed `_files.xml` file list
    pub files: XmlFiles,
    /// The URL the metadata was fetched from (the download base for files)
    pub base_url: Url,
    /// The raw XML content, to persist locally
    pub content: String,
    /// The server's `Last-Modified` time, if the server sent one
    pub last_modified: Option<SystemTime>,
}

/// Fetches, downloads and parses a `_files.xml` document from an explicit URL.
///
/// The session's cookie source (if any) is scoped against this URL — a raw
/// header applies as is, a parsed cookies.txt contributes exactly the
/// cookies whose path/domain cover it — so the metadata fetch and the file
/// downloads of one run each carry the right header for their URL.
pub async fn fetch_and_parse_xml(
    xml_url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
) -> Result<XmlMetadata> {
    // The accessibility pre-check's failure (a definitive 404/410) is
    // reported by the caller's spinner error path, which carries the full
    // detail (e.g. "the archive identifier may be incorrect").
    is_url_accessible(xml_url, client, cookie_source).await?;

    // The HEAD check above can pass while the GET still fails (throttling,
    // transient edge errors): surface it as a network error instead of
    // feeding an error page into the XML parser. Redirects are followed
    // with the cookie header re-resolved per target.
    let response = send_following_redirects(
        client,
        reqwest::Method::GET,
        xml_url,
        None,
        cookie_source,
        None,
    )
    .await?
    .error_for_status()?;
    let last_modified = parse_last_modified(response.headers());
    let xml_content = response.text().await?;

    // Parse XML content with improved error handling
    let files = parse_xml_files(&xml_content)?;

    Ok(XmlMetadata {
        files,
        base_url: xml_url.clone(),
        content: xml_content,
        last_modified,
    })
}

/// Return formatted file rows for `--list` output.
///
/// The archive's own `_files.xml` entry (matched against `xml_file_name`)
/// gets a dimmed "(metadata)" marker: a download saves it locally as file
/// #1 instead of fetching it as one of the archive's files.
pub fn list_file_rows(files: &XmlFiles, xml_file_name: &str) -> Vec<String> {
    files
        .files
        .iter()
        .map(|file| {
            let size = file
                .size
                .map(format_size)
                .unwrap_or_else(|| "unknown".to_string());
            if file.name == xml_file_name {
                format!("{size:>9} {} {}", file.name, "(metadata)".dimmed())
            } else {
                format!("{size:>9} {}", file.name)
            }
        })
        .collect()
}

/// Return a summary for `--list` output.
pub fn list_summary(files: &XmlFiles) -> String {
    // A hostile or corrupt metadata document could push the sum past
    // u64::MAX: saturate instead of panicking (debug) or wrapping (release)
    let total_known_size = files
        .files
        .iter()
        .filter_map(|file| file.size)
        .fold(0u64, |total, size| total.saturating_add(size));
    let unknown_size_count = files
        .files
        .iter()
        .filter(|file| file.size.is_none())
        .count();
    let file_label = if files.files.len() == 1 {
        "file"
    } else {
        "files"
    };

    if unknown_size_count == 0 {
        format!(
            "{} {file_label}, {} total",
            files.files.len(),
            format_size(total_known_size)
        )
    } else {
        let unknown_label = if unknown_size_count == 1 {
            "unknown size"
        } else {
            "unknown sizes"
        };
        format!(
            "{} {file_label}, {} total known size, {} {unknown_label}",
            files.files.len(),
            format_size(total_known_size),
            unknown_size_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MockBody, MockResponse, MockServer, TempDir, mtime_of, xml_file};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn file_entry_without_source_attribute_parses() {
        // A missing @source must not break parsing of the whole document.
        let files = parse_xml_files("<files><file name=\"a.bin\"><size>3</size></file></files>")
            .expect("source attribute must be optional");
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "a.bin");
        assert!(files.files[0].source.is_none());
    }

    #[test]
    fn file_entry_with_source_attribute_parses() {
        let files = parse_xml_files(
            "<files><file name=\"a.bin\" source=\"original\"><size>3</size></file></files>",
        )
        .expect("valid metadata must parse");
        assert_eq!(files.files[0].source.as_deref(), Some("original"));
    }

    #[test]
    fn parse_error_line_reads_the_parser_position() {
        assert_eq!(
            parse_error_line("Reader: 12:34 Unexpected closing tag: a != b"),
            Some(12)
        );
        assert_eq!(parse_error_line("Reader: 1:44 EOF"), Some(1));
        assert_eq!(parse_error_line("no position here"), None);
    }

    #[test]
    fn xml_parse_error_shows_context_around_failing_line() {
        // 40 filler entries, then a stray closing tag on line 42: the
        // error must name that line and show its neighbourhood, not the
        // document head.
        let filler = (0..40)
            .map(|i| format!("  <file name=\"filler{i}\"/>"))
            .collect::<Vec<_>>()
            .join("\n");
        let xml = format!("<files>\n{filler}\n</strange>\n</files>");
        let msg = parse_xml_files(&xml).unwrap_err().to_string();

        assert!(msg.contains("42:"), "must name the failing line: {msg}");
        assert!(
            msg.contains("»    42"),
            "the failing line must be marked: {msg}"
        );
        assert!(
            msg.contains("strange"),
            "the context must include the failing line: {msg}"
        );
        assert!(
            !msg.contains("filler0"),
            "the context must not start at the document head: {msg}"
        );
    }

    #[test]
    fn error_context_without_position_shows_document_head() {
        let xml = format!("{}\n<files/>", "x".repeat(100));
        let ctx = error_context(&xml, None);
        assert!(ctx.contains(&"x".repeat(100)));
        assert!(ctx.contains("1"), "head context starts at line 1");
    }

    #[test]
    fn error_context_caps_very_long_lines() {
        // A minified document: one line far beyond the cap. The preview
        // must stay bounded and cut on a char boundary.
        let line = "字".repeat(3 * XML_DEBUG_TRUNCATE_LEN);
        let ctx = error_context(&line, Some(1));
        assert!(
            ctx.chars().count() <= XML_DEBUG_TRUNCATE_LEN + 16,
            "the preview must stay bounded, got {} chars",
            ctx.chars().count()
        );
    }

    #[test]
    fn error_context_clamps_anchor_past_the_end() {
        let xml = "<files>\n<file/>\n";
        let ctx = error_context(xml, Some(999));
        assert!(ctx.contains("2"), "the anchor must clamp to the last line");
        assert!(ctx.contains("»"), "the clamped line must be marked");
    }

    #[test]
    fn error_context_counts_lines_with_trailing_newline() {
        // "a\nb\n" is two lines, not three: an anchor past the end must
        // clamp to line 2, and no phantom line 3 may appear.
        let ctx = error_context("a\nb\n", Some(99));
        assert!(
            ctx.contains("»     2"),
            "the anchor must clamp to the last line: {ctx}"
        );
        assert!(!ctx.contains('3'), "no phantom third line: {ctx}");
    }

    #[test]
    fn error_context_on_empty_document_keeps_the_marker() {
        let ctx = error_context("", Some(1));
        assert!(
            ctx.contains("»"),
            "the marker must survive an empty document: {ctx}"
        );
        assert!(
            ctx.contains("     1"),
            "the head line number must survive: {ctx}"
        );
    }

    #[test]
    fn save_xml_metadata_writes_file_and_sets_mtime() {
        let dir = TempDir::new("save_xml_sets_mtime");
        let path = dir.join("item1_files.xml");

        save_xml_metadata(
            &path,
            "<files><file name=\"item1_files.xml\"/></files>",
            Some(UNIX_EPOCH + Duration::from_secs(1_545_586_142)),
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("item1_files.xml"));
        assert_eq!(mtime_of(&path), Some(1_545_586_142));
    }

    #[test]
    fn save_xml_metadata_overwrites_existing_file() {
        let dir = TempDir::new("save_xml_overwrites");
        let path = dir.join("item1_files.xml");
        std::fs::write(&path, "stale content").unwrap();

        save_xml_metadata(&path, "<files/>", None).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<files/>");
    }

    #[cfg(unix)]
    #[test]
    fn save_xml_metadata_refuses_to_write_through_symlink() {
        // A symlink named "<id>_files.xml" must not have its target
        // silently truncated by the metadata save.
        let dir = TempDir::new("save_xml_symlink");
        let target = dir.join("target.txt");
        std::fs::write(&target, "do not touch").unwrap();
        let link = dir.join("item1_files.xml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = save_xml_metadata(&link, "<files/>", None).unwrap_err();
        assert!(err.to_string().contains("symlink"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "do not touch",
            "the link target must be left untouched"
        );
    }

    #[test]
    fn save_xml_metadata_without_last_modified_keeps_current_time() {
        let dir = TempDir::new("save_xml_no_mtime");
        let path = dir.join("item1_files.xml");

        save_xml_metadata(&path, "<files/>", None).unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mtime = mtime_of(&path).expect("mtime should be readable");
        assert!(
            mtime.abs_diff(now) < 60,
            "mtime {mtime} should be within 60s of now {now}"
        );
    }

    #[test]
    fn parse_archive_url_reads_item_urls_without_a_file() {
        for (url, identifier) in [
            ("https://archive.org/details/Valid-Pattern", "Valid-Pattern"),
            (
                "https://archive.org/details/Valid-Pattern/",
                "Valid-Pattern",
            ),
            ("https://archive.org/details/test123", "test123"),
            ("https://archive.org/details/test123/", "test123"),
            (
                "https://archive.org/details/test_file-name.data",
                "test_file-name.data",
            ),
            ("https://archive.org/details/user@domain", "user@domain"),
            ("https://archive.org/download/item1", "item1"),
            ("https://archive.org/download/item1/", "item1"),
        ] {
            let target = parse_archive_url(url).expect("{url} must parse");
            assert_eq!(target.identifier, identifier, "{url}");
            assert!(target.file_path.is_none(), "{url} must name no file");
        }
    }

    #[test]
    fn parse_archive_url_reads_the_file_path_from_urls_with_one() {
        let target = parse_archive_url("https://archive.org/download/item1/scan/01.pdf")
            .expect("a download URL with a file path");
        assert_eq!(target.identifier, "item1");
        assert_eq!(target.file_path.as_deref(), Some("scan/01.pdf"));

        // A details URL with a path is read like a download one; a
        // trailing separator names the same file
        let target = parse_archive_url("https://archive.org/details/item1/scan/01.pdf/").unwrap();
        assert_eq!(target.file_path.as_deref(), Some("scan/01.pdf"));
    }

    #[test]
    fn parse_archive_url_drops_query_and_fragment() {
        // A query or fragment is URL structure: it must not leak into the
        // file path (where it would fail the metadata match)
        let target =
            parse_archive_url("https://archive.org/download/item1/scan/01.pdf?session=abc#top")
                .expect("a URL with query and fragment must parse");
        assert_eq!(target.identifier, "item1");
        assert_eq!(target.file_path.as_deref(), Some("scan/01.pdf"));

        let target = parse_archive_url("https://archive.org/details/item1?x=1").expect("details");
        assert_eq!(target.identifier, "item1");
        assert!(target.file_path.is_none());
    }

    #[test]
    fn parse_archive_url_percent_decodes_file_paths() {
        // Spaces and URL-special characters arrive percent-encoded and
        // must decode back to the archive's original names
        let target =
            parse_archive_url("https://archive.org/download/item1/Season%201/clip%231.mp4")
                .unwrap();
        assert_eq!(target.file_path.as_deref(), Some("Season 1/clip#1.mp4"));

        // A literal '%' in a name is encoded as %25 and decodes to '%'
        let target = parse_archive_url("https://archive.org/download/item1/100%25off.mp4").unwrap();
        assert_eq!(target.file_path.as_deref(), Some("100%off.mp4"));

        // An escape without two hex digits passes through literally
        let target =
            parse_archive_url("https://archive.org/download/item1/weird%name.bin").unwrap();
        assert_eq!(target.file_path.as_deref(), Some("weird%name.bin"));
    }

    #[test]
    fn parse_archive_url_rejects_invalid_urls() {
        for url in [
            "https://archive.org/details/",
            "https://archive.org/details/Invalid-Pattern-*",
            "https://example.com/details/test",
            "http://archive.org/details/test",
            "https://archive.org/details/test//",
            "https://archive.org/download/item1/%ff",
            "archive.org/details/test",
        ] {
            assert!(parse_archive_url(url).is_err(), "{url} must be rejected");
        }
    }

    #[test]
    fn get_xml_url_builds_the_files_list_url_from_the_identifier() {
        assert_eq!(
            get_xml_url("item1").unwrap().as_str(),
            "https://archive.org/download/item1/item1_files.xml"
        );
        assert_eq!(
            get_xml_url("another-item_v2.0").unwrap().as_str(),
            "https://archive.org/download/another-item_v2.0/another-item_v2.0_files.xml"
        );
    }

    #[test]
    fn encode_download_path_escapes_url_specials() {
        // '#' starts a fragment and '?' a query — both must be escaped.
        assert_eq!(encode_download_path("clip#1.mp4"), "clip%231.mp4");
        assert_eq!(encode_download_path("a?b.mp4"), "a%3Fb.mp4");
        // A literal '%' in the name must be escaped so it is not read as an
        // existing escape sequence.
        assert_eq!(encode_download_path("100%25off.mp4"), "100%2525off.mp4");
        // Spaces and non-ASCII are encoded; '/' separators are preserved.
        assert_eq!(
            encode_download_path("sub dir/файл.mp4"),
            "sub%20dir/%D1%84%D0%B0%D0%B9%D0%BB.mp4"
        );
        // Unreserved characters pass through unchanged.
        assert_eq!(
            encode_download_path("plain-file_v2.0.tar.gz"),
            "plain-file_v2.0.tar.gz"
        );
    }

    #[test]
    fn encode_download_path_drops_dot_segments() {
        // A "../" entry must not escape the item directory once Url::join
        // removes dot segments; dropping them up front keeps the reference
        // relative and honest.
        assert_eq!(encode_download_path("../outside.mp4"), "outside.mp4");
        assert_eq!(encode_download_path("a/./b.mp4"), "a/b.mp4");
        assert_eq!(
            encode_download_path(".."),
            "",
            "a dot-segment-only name encodes to nothing and is skipped"
        );
    }

    #[test]
    fn encode_download_path_drops_leading_slashes() {
        // A leading '/' makes the reference path-absolute, and "//host/"
        // would be read as a different host — both must stay relative.
        assert_eq!(encode_download_path("/foo.mp4"), "foo.mp4");
        assert_eq!(
            encode_download_path("//evil.example/x.mp4"),
            "evil.example/x.mp4"
        );
        // Internal separators, including doubles, are preserved.
        assert_eq!(encode_download_path("a//b.mp4"), "a//b.mp4");
    }

    #[test]
    fn leading_slash_name_stays_under_item() {
        let base = Url::parse("https://archive.org/download/item1/item1_files.xml").unwrap();
        let joined = base
            .join(&encode_download_path("//evil.example/x.mp4"))
            .expect("encoded name must join against the base URL");
        assert_eq!(
            joined.as_str(),
            "https://archive.org/download/item1/evil.example/x.mp4"
        );
    }

    #[test]
    fn file_url_join_must_not_silently_fall_back() {
        // Regression: a failed join used to keep the XML metadata URL, which
        // would download _files.xml under the file's name. Encoded names must
        // always join cleanly against the metadata base URL.
        let base = Url::parse("https://archive.org/download/item1/item1_files.xml").unwrap();
        let joined = base
            .join(&encode_download_path("Season 1/clip#1.mp4"))
            .expect("encoded name must join against the base URL");
        assert_eq!(
            joined.as_str(),
            "https://archive.org/download/item1/Season%201/clip%231.mp4"
        );
    }

    #[tokio::test]
    async fn xml_metadata_http_error_is_a_network_error() {
        // The HEAD check passes; the GET must fail with a status error.
        let (_server, url) = MockServer::scripted(
            "/download/item1/item1_files.xml",
            vec![
                MockResponse::new(200, MockBody::Full(vec![])), // HEAD check
                MockResponse::new(
                    500,
                    MockBody::Full(b"<html><body>nginx error page</body></html>".to_vec()),
                ),
            ],
        );
        let client = Client::new();

        let err = fetch_and_parse_xml(&url, &client, None)
            .await
            .expect_err("an HTTP error on the metadata GET must fail the fetch");

        match err {
            IaGetError::Network { detail, .. } => {
                assert!(
                    detail.contains("500"),
                    "expected the status code in the error, got: {detail}"
                );
            }
            other => panic!("expected a Network error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn head_405_does_not_block_the_get() {
        // A proxy that rejects HEAD (405) must not prevent the GET from
        // proceeding; the metadata is still fetched.
        let xml = "<files><file name=\"a.bin\"/></files>";
        let (_server, url) = MockServer::scripted(
            "/download/item1/item1_files.xml",
            vec![
                MockResponse::new(405, MockBody::Full(vec![])), // HEAD rejected
                MockResponse::new(200, MockBody::Full(xml.as_bytes().to_vec())),
            ],
        );
        let client = Client::new();

        let meta = fetch_and_parse_xml(&url, &client, None)
            .await
            .expect("a 405 on HEAD must not block the GET");
        assert_eq!(meta.files.files.len(), 1);
    }

    #[tokio::test]
    async fn head_404_is_fatal() {
        // A 404 on HEAD is definitive: the resource does not exist, so the
        // fetch must fail without attempting the GET.
        let (_server, url) = MockServer::scripted(
            "/download/item1/item1_files.xml",
            vec![MockResponse::new(404, MockBody::Full(vec![]))],
        );
        let client = Client::new();

        let err = fetch_and_parse_xml(&url, &client, None)
            .await
            .expect_err("a 404 on HEAD must fail the fetch");
        match err {
            IaGetError::Network { detail, .. } => {
                assert!(
                    detail.contains("404"),
                    "expected the status code in the error, got: {detail}"
                );
            }
            other => panic!("expected a Network error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn xml_metadata_success_parses_files() {
        let xml = "<files><file name=\"item1_files.xml\" source=\"original\"><size>23</size></file><file name=\"scan.jpg\" source=\"original\"><size>456</size></file></files>";
        let (_server, url) = MockServer::scripted(
            "/download/item1/item1_files.xml",
            vec![
                MockResponse::new(200, MockBody::Full(vec![])), // HEAD check
                MockResponse::new(200, MockBody::Full(xml.as_bytes().to_vec())),
            ],
        );
        let client = Client::new();

        let meta = fetch_and_parse_xml(&url, &client, None)
            .await
            .expect("metadata fetch should succeed");

        assert_eq!(meta.files.files.len(), 2);
        assert_eq!(meta.files.files[1].name, "scan.jpg");
        assert_eq!(meta.content, xml);
    }

    #[tokio::test]
    async fn precomputed_cookie_header_reaches_the_xml_fetch() {
        let xml = "<files><file name=\"item1_files.xml\"/></files>";
        let (server, url) = MockServer::scripted(
            "/download/item1/item1_files.xml",
            vec![
                MockResponse::new(200, MockBody::Full(vec![])), // HEAD check
                MockResponse::new(200, MockBody::Full(xml.as_bytes().to_vec())),
            ],
        );
        let client = Client::new();
        let source = CookieSource::Raw(reqwest::header::HeaderValue::from_static("session=abc123"));

        fetch_and_parse_xml(&url, &client, Some(&source))
            .await
            .expect("metadata fetch should succeed");
        assert_eq!(
            server.cookies(),
            vec![
                Some("session=abc123".to_string()),
                Some("session=abc123".to_string()),
            ],
            "both the HEAD check and the GET must carry the precomputed header"
        );
    }

    #[test]
    fn list_file_rows_format_sizes_and_unknown_entries() {
        let files = XmlFiles {
            files: vec![
                xml_file("cover.jpg", Some(12_345)),
                xml_file("metadata.xml", None),
            ],
        };

        assert_eq!(
            list_file_rows(&files, "item1_files.xml"),
            vec![
                "  12.06KB cover.jpg".to_string(),
                "  unknown metadata.xml".to_string(),
            ]
        );
    }

    #[test]
    fn list_file_rows_marks_the_xml_metadata_entry() {
        let files = XmlFiles {
            files: vec![
                xml_file("cover.jpg", Some(12_345)),
                xml_file("item1_files.xml", Some(23)),
            ],
        };

        // The expected marker is built with the same colored primitive the
        // function under test uses, so the assertion holds whether colored
        // renders ANSI (a TTY, or CLICOLOR_FORCE on a developer's box) or
        // stays plain (captured/pipe output) — and the dimmed styling of
        // the marker stays pinned
        assert_eq!(
            list_file_rows(&files, "item1_files.xml"),
            vec![
                "  12.06KB cover.jpg".to_string(),
                format!("      23B item1_files.xml {}", "(metadata)".dimmed()),
            ]
        );
    }

    #[test]
    fn list_summary_saturates_on_overflow() {
        // Two maximal sizes must not panic (debug) or wrap to a small
        // total (release): the sum saturates at u64::MAX
        let files = XmlFiles {
            files: vec![
                xml_file("a.bin", Some(u64::MAX)),
                xml_file("b.bin", Some(u64::MAX)),
            ],
        };
        let summary = list_summary(&files);
        assert!(
            !summary.contains("0B total"),
            "a wrapped total must not read as zero: {summary}"
        );
    }

    #[test]
    fn list_summary_reports_total_known_size_and_unknown_count() {
        let files = XmlFiles {
            files: vec![
                xml_file("disk1.zip", Some(1_048_576)),
                xml_file("disk2.zip", Some(2_097_152)),
                xml_file("notes.txt", None),
            ],
        };

        assert_eq!(
            list_summary(&files),
            "3 files, 3.00MB total known size, 1 unknown size"
        );
    }
}
