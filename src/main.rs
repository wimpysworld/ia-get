//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//!
//! This tool takes an archive.org details URL or direct file download URL and downloads
//! the associated files, with support for resumable downloads and MD5 hash verification.

use std::path::{Path, PathBuf};

use clap::Parser;
use colored::*;
use ia_get::archive_metadata::{parse_xml_files, XmlFiles};
use ia_get::constants::USER_AGENT;
use ia_get::downloader;
use ia_get::utils::{
    create_spinner, format_size, parse_archive_url, percent_decode_utf8, sanitize_filename,
    ArchiveUrl,
};
use ia_get::{CookieSource, IaGetError, Result};
use indicatif::ProgressStyle;
use reqwest::header::{HeaderMap, COOKIE};
use reqwest::{Client, Url};

/// Extended timeout for large file downloads (10 minutes for connection, no read timeout)
const CONNECTION_TIMEOUT_SECS: u64 = 600;

/// Checks if a URL is accessible by sending a HEAD request with an explicit retry policy.
async fn is_url_accessible_with_policy(
    url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
) -> Result<()> {
    let _response = downloader::send_with_retry(
        url,
        policy,
        signal,
        |target_url| {
            let mut request = client
                .head(target_url.clone())
                .timeout(std::time::Duration::from_secs(60));
            if let Some(cookie_header) = cookie_source
                .map(|source| source.header_value_for_url(target_url))
                .transpose()?
                .flatten()
            {
                let mut headers = HeaderMap::new();
                headers.insert(COOKIE, cookie_header);
                request = request.headers(headers);
            }
            Ok(request)
        },
        |_| {},
    )
    .await?;

    Ok(())
}

/// Checks if a URL is accessible by sending a HEAD request
async fn is_url_accessible(
    url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
    signal: &downloader::ShutdownSignal,
) -> Result<()> {
    is_url_accessible_with_policy(
        url,
        client,
        cookie_source,
        &downloader::RetryPolicy::default(),
        signal,
    )
    .await
}

/// Fetches and parses XML metadata from an already-resolved XML URL with an explicit retry policy.
async fn fetch_xml_metadata_from_url(
    base_url: reqwest::Url,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
) -> Result<(XmlFiles, reqwest::Url)> {
    let xml_url = base_url.as_str();
    spinner.set_message(format!(
        "{} Accessing XML metadata: {}",
        "⚙".blue(),
        xml_url.bold()
    ));

    // Check XML URL accessibility
    if let Err(e) =
        is_url_accessible_with_policy(&base_url, client, cookie_source, policy, signal).await
    {
        spinner.finish_with_message(format!(
            "{} XML metadata not accessible: {}",
            "✘".red().bold(),
            xml_url.bold()
        ));
        return Err(e); // Propagate the error
    }

    spinner.set_message(format!(
        "{} {}",
        "⚙".blue(),
        "Parsing archive metadata...".bold()
    ));

    let xml_content = downloader::fetch_text_with_retry(
        &base_url,
        policy,
        signal,
        |target_url| {
            let mut request = client.get(target_url.clone());
            if let Some(cookie_header) = cookie_source
                .map(|source| source.header_value_for_url(target_url))
                .transpose()?
                .flatten()
            {
                let mut headers = HeaderMap::new();
                headers.insert(COOKIE, cookie_header);
                request = request.headers(headers);
            }
            Ok(request)
        },
        |event| {
            spinner.set_message(format!(
                "{} Retrying XML metadata ({}/{}) after {} in {:.1}s...",
                "⟳".yellow().bold(),
                event.attempt,
                event.max_retries,
                event.reason,
                event.delay.as_secs_f64()
            ));
        },
    )
    .await?;

    // Parse XML content with improved error handling
    let files = parse_xml_files(&xml_content)?;

    Ok((files, base_url))
}

/// Fetches and parses XML metadata from archive.org
///
/// Combines XML URL generation, accessibility check, download, and parsing
/// into a single operation with integrated error handling.
///
/// # Arguments
/// * `target` - The parsed archive.org URL
/// * `client` - HTTP client for requests
/// * `spinner` - Progress spinner to update during processing
/// * `cookie_source` - Optional parsed cookie source evaluated per request URL
/// * `signal` - Shared shutdown signal for immediate Ctrl+C cancellation
///
/// # Returns
/// Tuple of (XmlFiles, base_url) for download processing
async fn fetch_xml_metadata(
    target: &ArchiveUrl,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    signal: &downloader::ShutdownSignal,
) -> Result<(XmlFiles, reqwest::Url)> {
    let xml_url = target.xml_url();
    let base_url = reqwest::Url::parse(&xml_url)?;

    fetch_xml_metadata_from_url(
        base_url,
        client,
        spinner,
        cookie_source,
        &downloader::RetryPolicy::default(),
        signal,
    )
    .await
}

/// Filters parsed archive metadata to the requested target URL.
///
/// For details URLs, returns all files unchanged. For direct download URLs,
/// selects the single matching entry by exact normalized name, falling back to
/// percent-decoded comparison if the metadata name is percent-encoded.
fn select_target_files(files: XmlFiles, target: &ArchiveUrl) -> Result<XmlFiles> {
    match target {
        ArchiveUrl::Details { .. } => Ok(files),
        ArchiveUrl::Download {
            identifier,
            file_path,
        } => {
            let target_norm = file_path.trim_start_matches('/');
            let matched_pos = files
                .files
                .iter()
                .position(|entry| entry.name.trim_start_matches('/') == target_norm)
                .or_else(|| {
                    files.files.iter().position(|entry| {
                        percent_decode_utf8(entry.name.trim_start_matches('/'))
                            .ok()
                            .as_deref()
                            == Some(target_norm)
                    })
                });

            match matched_pos {
                Some(idx) => {
                    let mut entries = files.files;
                    let matched = entries.swap_remove(idx);
                    Ok(XmlFiles {
                        files: vec![matched],
                    })
                }
                None => Err(IaGetError::FileNotFound {
                    file: file_path.clone(),
                    item: identifier.clone(),
                }),
            }
        }
    }
}

/// Builds the download URL for a file relative to the item's XML metadata URL.
fn build_file_download_url(base_url: &Url, file_name: &str) -> Url {
    let mut file_url = base_url.clone();
    file_url.set_query(None);
    file_url.set_fragment(None);
    if let Ok(mut segments) = file_url.path_segments_mut() {
        segments.pop();
        segments.extend(
            file_name
                .trim_start_matches('/')
                .split('/')
                .filter(|segment| !segment.is_empty()),
        );
    }
    file_url
}

/// Return formatted file rows for `--list` output.
fn list_file_rows(files: &XmlFiles) -> Vec<String> {
    files
        .files
        .iter()
        .map(|file| {
            let size = file
                .size
                .map(format_size)
                .unwrap_or_else(|| "unknown".to_string());
            format!("{size:>9} {}", file.name)
        })
        .collect()
}

/// Return a summary for `--list` output.
fn list_summary(files: &XmlFiles) -> String {
    let total_known_size: u64 = files.files.iter().filter_map(|file| file.size).sum();
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

/// Lists parsed filenames from XML metadata when --list/-l is used
fn list_files(files: &XmlFiles, spinner: &indicatif::ProgressBar) {
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!(
                "{} Archive has {}",
                "✔".green().bold(),
                list_summary(files).bold()
            ))
            .expect("Failed to set completion style"),
    );
    spinner.finish();
    for row in list_file_rows(files) {
        println!("{row}");
    }
}

/// Maps a completed batch download summary to the CLI exit result.
fn batch_exit_result(summary: &downloader::BatchSummary, total_files: usize) -> Result<()> {
    if summary.interrupted {
        Err(IaGetError::Interrupted)
    } else if !summary.failed.is_empty() {
        Err(IaGetError::BatchFailed {
            failed: summary.failed.len(),
            total: total_files,
        })
    } else {
        Ok(())
    }
}

/// Resolves the output directory path, defaulting to the current working directory.
fn resolve_output_dir(output_dir: Option<&Path>) -> &Path {
    output_dir.unwrap_or_else(|| Path::new("."))
}

/// Finalises the metadata phase by either listing files or preparing the output directory.
fn finalize_metadata_phase(
    cli: &Cli,
    files: &XmlFiles,
    spinner: &indicatif::ProgressBar,
) -> Result<Option<PathBuf>> {
    if cli.list {
        list_files(files, spinner);
        return Ok(None);
    }

    match downloader::prepare_output_dir(resolve_output_dir(cli.output_dir.as_deref())) {
        Ok(canonical_dir) => Ok(Some(canonical_dir)),
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            Err(e)
        }
    }
}

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = option_env!("BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
#[command(after_help = "\
Examples:
  ia-get https://archive.org/details/deftributetozzap64
  ia-get https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf
  ia-get -o ./downloads https://archive.org/details/deftributetozzap64
  ia-get --output-dir /tmp/zzap64 https://archive.org/details/deftributetozzap64
  ia-get --list https://archive.org/details/deftributetozzap64
  ia-get --list https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf
  ia-get -b cookies.txt https://archive.org/details/deftributetozzap64")]
struct Cli {
    /// Archive.org item details URL (/details/<item>) or direct file URL (/download/<item>/<path>)
    url: String,
    /// List files parsed from archive metadata XML and exit
    #[arg(short = 'l', long = "list")]
    list: bool,
    /// Cookie header or Netscape cookies.txt file for authenticated downloads
    #[arg(short = 'b', long = "cookies", value_name = "COOKIES")]
    cookies: Option<String>,
    /// Directory to save downloaded files (defaults to current working directory)
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
}

/// Main application entry point
///
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and initiates file downloads with built-in signal handling.
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let cookie_source = cli
        .cookies
        .as_deref()
        .map(CookieSource::from_input)
        .transpose()?;

    // Create a client with extended timeouts for large file downloads
    // Connection timeout is set high, but no read timeout since large files
    // may take a long time to transfer
    let client = downloader::http_client_builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(CONNECTION_TIMEOUT_SECS))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(1)
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .build()?;

    let shutdown = downloader::install_signal_handler();

    // Start a single spinner for the entire initialization process
    let spinner = create_spinner(&format!("Processing archive.org URL: {}", cli.url.bold()));

    // Parse and validate URL format
    let target = match parse_archive_url(&cli.url) {
        Ok(target) => target,
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            return Err(e.into());
        }
    };

    let details_url = Url::parse(&target.details_url())?;

    // Check URL accessibility
    if let Err(e) =
        is_url_accessible(&details_url, &client, cookie_source.as_ref(), &shutdown).await
    {
        spinner.finish_with_message(format!(
            "{} Archive.org URL not accessible: {}",
            "✘".red().bold(),
            cli.url.bold()
        ));
        return Err(e.into()); // Propagate error
    }

    // Fetch and parse XML metadata in one operation
    let (files, base_url) = fetch_xml_metadata(
        &target,
        &client,
        &spinner,
        cookie_source.as_ref(),
        &shutdown,
    )
    .await?;

    // Narrow metadata to the requested file when given a direct download URL
    let files = match select_target_files(files, &target) {
        Ok(files) => files,
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            return Err(e.into());
        }
    };

    // If requested, list parsed filenames and exit; otherwise prepare output directory
    let Some(output_dir) = finalize_metadata_phase(&cli, &files, &spinner)? else {
        return Ok(());
    };

    // Successfully finished initialization
    let file_label = if files.files.len() == 1 {
        "file"
    } else {
        "files"
    };
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!(
                "{} {} to download {} {file_label} from archive.org {}",
                "✔".green().bold(),
                "Ready".bold(),
                files.files.len().to_string().bold(),
                "★".yellow()
            ))
            .expect("Failed to set completion style"),
    );
    spinner.finish();

    // Prepare download data for batch processing
    let mut sanitized_count = 0;
    let download_data = files
        .files
        .into_iter()
        .map(|file| {
            let absolute_url = build_file_download_url(&base_url, &file.name);

            // Sanitize filename for filesystem compatibility
            let (sanitized_name, was_modified) = sanitize_filename(&file.name);

            // Warn user if filename was modified
            if was_modified {
                println!(
                    "{} {} {} → {}",
                    "⚠".yellow().bold(),
                    "Sanitized:".yellow(),
                    file.name.dimmed(),
                    sanitized_name.bold()
                );
                sanitized_count += 1;
            }

            (absolute_url.to_string(), sanitized_name, file.md5)
        })
        .collect::<Vec<_>>();

    // Show summary if any files were sanitized
    if sanitized_count > 0 {
        println!(
            "\n{} {} {} file{} for filesystem compatibility",
            "✓".green().bold(),
            "Sanitized".bold(),
            sanitized_count.to_string().bold(),
            if sanitized_count == 1 { "" } else { "s" }
        );
    }

    // Download all files with integrated signal handling
    let total_files = download_data.len();
    let summary = downloader::download_files(
        &client,
        Some(output_dir.as_path()),
        download_data,
        total_files,
        cookie_source.as_ref(),
    )
    .await?;

    batch_exit_result(&summary, total_files)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ia_get::utils::validate_archive_url;

    #[test]
    fn check_valid_pattern() {
        assert!(validate_archive_url("https://archive.org/details/Valid-Pattern").is_ok());
        assert!(validate_archive_url("https://archive.org/details/Valid-Pattern/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test123").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test123/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test_file-name.data").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test_file-name.data/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/user@domain").is_ok());
        assert!(validate_archive_url("https://archive.org/details/user@domain/").is_ok());
    }

    #[test]
    fn check_invalid_pattern() {
        assert!(validate_archive_url("https://archive.org/details/Invalid-Pattern-*").is_err());
        assert!(validate_archive_url("https://archive.org/details/").is_err()); // This should still be an error (empty identifier)
        assert!(validate_archive_url("https://example.com/details/test").is_err());
        assert!(validate_archive_url("http://archive.org/details/test").is_err());
        assert!(validate_archive_url("https://archive.org/details/test/extra").is_err());
        assert!(validate_archive_url("https://archive.org/details/test//").is_err());
        // Multiple trailing slashes
    }

    #[test]
    fn check_get_xml_url() {
        assert_eq!(
            parse_archive_url("https://archive.org/details/item1")
                .unwrap()
                .xml_url(),
            "https://archive.org/download/item1/item1_files.xml"
        );
        assert_eq!(
            parse_archive_url("https://archive.org/details/item1/")
                .unwrap()
                .xml_url(), // With trailing slash
            "https://archive.org/download/item1/item1_files.xml"
        );
        assert_eq!(
            parse_archive_url("https://archive.org/details/another-item_v2.0")
                .unwrap()
                .xml_url(),
            "https://archive.org/download/another-item_v2.0/another-item_v2.0_files.xml"
        );
        assert_eq!(
            parse_archive_url("https://archive.org/details/another-item_v2.0/")
                .unwrap()
                .xml_url(), // With trailing slash
            "https://archive.org/download/another-item_v2.0/another-item_v2.0_files.xml"
        );
        assert_eq!(
            parse_archive_url("https://archive.org/download/item1/dir/file.pdf")
                .unwrap()
                .xml_url(),
            "https://archive.org/download/item1/item1_files.xml"
        );
    }

    fn xml_file(name: &str, size: Option<u64>) -> ia_get::archive_metadata::XmlFile {
        ia_get::archive_metadata::XmlFile {
            name: name.to_string(),
            source: "original".to_string(),
            mtime: None,
            size,
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        }
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
            list_file_rows(&files),
            vec![
                "  12.06KB cover.jpg".to_string(),
                "  unknown metadata.xml".to_string(),
            ]
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

    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        request_line: String,
        headers: Vec<(String, String)>,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    struct MockResponse {
        status_line: &'static str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl MockResponse {
        fn ok(body: &[u8]) -> Self {
            Self {
                status_line: "200 OK",
                headers: vec![("Content-Length".to_string(), body.len().to_string())],
                body: body.to_vec(),
            }
        }

        fn with_headers(
            status_line: &'static str,
            headers: Vec<(&str, String)>,
            body: &[u8],
        ) -> Self {
            Self {
                status_line,
                headers: headers
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                body: body.to_vec(),
            }
        }
    }

    struct MockHttpServer {
        host: &'static str,
        addr: std::net::SocketAddr,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockHttpServer {
        async fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock metadata server");
            Self::start_with_listener(listener, "127.0.0.1", responses)
        }

        async fn start_on_localhost(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind(("localhost", 0))
                .await
                .expect("bind localhost mock metadata server");
            Self::start_with_listener(listener, "localhost", responses)
        }

        fn start_with_listener(
            listener: TcpListener,
            host: &'static str,
            responses: Vec<MockResponse>,
        ) -> Self {
            let addr = listener.local_addr().expect("mock server local_addr");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_clone = Arc::clone(&requests);

            let handle = tokio::spawn(async move {
                for response in responses {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = socket.set_nodelay(true);
                    let is_head = match read_http_request(&mut socket).await {
                        Some(req) => {
                            let is_head = req.request_line.starts_with("HEAD ");
                            requests_clone.lock().await.push(req);
                            is_head
                        }
                        None => false,
                    };
                    write_mock_response(&mut socket, response, is_head).await;
                }
            });

            Self {
                host,
                addr,
                requests,
                handle: Some(handle),
            }
        }

        fn url(&self, path: &str) -> Url {
            Url::parse(&format!(
                "http://{}:{}{}",
                self.host,
                self.addr.port(),
                path
            ))
            .expect("valid mock URL")
        }

        fn requests_handle(&self) -> Arc<Mutex<Vec<CapturedRequest>>> {
            Arc::clone(&self.requests)
        }

        async fn finish(mut self) -> Vec<CapturedRequest> {
            if let Some(handle) = self.handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
            }
            self.requests.lock().await.clone()
        }

        async fn abort_and_collect(mut self) -> Vec<CapturedRequest> {
            if let Some(handle) = self.handle.take() {
                handle.abort();
                let _ = handle.await;
            }
            self.requests.lock().await.clone()
        }
    }

    impl Drop for MockHttpServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    async fn read_http_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(buf.len());
        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next()?.to_string();
        let mut headers = Vec::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }

        Some(CapturedRequest {
            request_line,
            headers,
        })
    }

    async fn write_mock_response(socket: &mut TcpStream, response: MockResponse, is_head: bool) {
        let mut wire = format!("HTTP/1.1 {}\r\n", response.status_line);
        let mut has_connection = false;
        for (name, value) in &response.headers {
            if name.eq_ignore_ascii_case("connection") {
                has_connection = true;
            }
            wire.push_str(name);
            wire.push_str(": ");
            wire.push_str(value);
            wire.push_str("\r\n");
        }
        if !has_connection {
            wire.push_str("Connection: close\r\n");
        }
        wire.push_str("\r\n");

        let mut bytes = wire.into_bytes();
        if !is_head {
            bytes.extend_from_slice(&response.body);
        }
        let _ = socket.write_all(&bytes).await;
        let _ = socket.flush().await;
        let _ = socket.shutdown().await;
    }

    fn format_imf_fixdate(unix_secs: i64) -> String {
        const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        let days = unix_secs.div_euclid(86_400);
        let sec_of_day = unix_secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = mp + if mp < 10 { 3 } else { -9 };
        let year = y + i64::from(month <= 2);

        let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
        let month_name = MONTHS[(month - 1) as usize];
        let hour = sec_of_day / 3600;
        let minute = (sec_of_day % 3600) / 60;
        let second = sec_of_day % 60;

        format!("{weekday}, {day:02} {month_name} {year:04} {hour:02}:{minute:02}:{second:02} GMT")
    }

    fn future_imf_fixdate_header() -> String {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH");
        let offset_secs = if since_epoch.subsec_millis() >= 800 {
            2
        } else {
            1
        };
        format_imf_fixdate(since_epoch.as_secs() as i64 + offset_secs)
    }

    #[tokio::test]
    async fn is_url_accessible_retries_429_delta_seconds_and_succeeds() {
        let server = MockHttpServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "1".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(b""),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(5),
            max_retry_after: Duration::from_secs(5),
        };

        let start = Instant::now();
        is_url_accessible_with_policy(
            &server.url("/details/item"),
            &client,
            None,
            &policy,
            &signal,
        )
        .await
        .expect("HEAD 429 with Retry-After: 1 should retry and succeed");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].request_line.starts_with("HEAD /details/item "));
        assert!(requests[1].request_line.starts_with("HEAD /details/item "));
        assert!(
            elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(4),
            "HEAD Retry-After: 1 should honour ~1s delay, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn is_url_accessible_retries_503_http_date_and_succeeds() {
        let retry_after_date = future_imf_fixdate_header();
        let server = MockHttpServer::start(vec![
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", retry_after_date),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(b""),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };

        let start = Instant::now();
        is_url_accessible_with_policy(
            &server.url("/details/item"),
            &client,
            None,
            &policy,
            &signal,
        )
        .await
        .expect("HEAD 503 with future HTTP-date Retry-After should retry and succeed");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].request_line.starts_with("HEAD /details/item "));
        assert!(requests[1].request_line.starts_with("HEAD /details/item "));
        assert!(
            elapsed >= Duration::from_millis(100) && elapsed < Duration::from_secs(3),
            "HEAD HTTP-date Retry-After should wait for future timestamp, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn is_url_accessible_fails_fast_on_403_and_404() {
        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let slow_policy = downloader::RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(10),
        };

        for (code, status_line) in [("403", "403 Forbidden"), ("404", "404 Not Found")] {
            let server = MockHttpServer::start(vec![
                MockResponse::with_headers(
                    status_line,
                    vec![("Content-Length", "0".to_string())],
                    b"",
                ),
                MockResponse::ok(b""),
            ])
            .await;

            let start = Instant::now();
            let err = is_url_accessible_with_policy(
                &server.url("/details/private"),
                &client,
                None,
                &slow_policy,
                &signal,
            )
            .await
            .unwrap_err();
            let elapsed = start.elapsed();

            let requests = server.abort_and_collect().await;
            assert_eq!(requests.len(), 1, "HEAD {code} must fail fast");
            assert!(requests[0]
                .request_line
                .starts_with("HEAD /details/private "));
            assert!(
                elapsed < Duration::from_secs(1),
                "HEAD {code} should not wait before failing, took {:?}",
                elapsed
            );
            assert!(matches!(err, ia_get::IaGetError::Network(_)), "case {code}");
            assert!(err.to_string().contains(code), "case {code}: {err}");
        }
    }

    #[tokio::test]
    async fn fetch_xml_metadata_retries_503_on_get_and_parses_valid_xml() {
        let xml_body = br#"<files>
            <file name="issue01.pdf" source="original">
                <size>4096</size>
                <md5>0123456789abcdef0123456789abcdef</md5>
            </file>
        </files>"#;

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", "0".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
                b"busy",
            ),
            MockResponse::ok(xml_body),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };
        let xml_url = server.url("/download/item/item_files.xml");

        let (files, returned_url) =
            fetch_xml_metadata_from_url(xml_url.clone(), &client, &spinner, None, &policy, &signal)
                .await
                .expect("XML GET 503 should retry and parse valid XML");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert_eq!(returned_url, xml_url);
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "issue01.pdf");
        assert_eq!(files.files[0].size, Some(4096));
        assert_eq!(
            files.files[0].md5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }

    #[tokio::test]
    async fn fetch_xml_metadata_fails_fast_on_404() {
        let server = MockHttpServer::start(vec![
            MockResponse::with_headers(
                "404 Not Found",
                vec![("Content-Length", "0".to_string())],
                b"",
            ),
            MockResponse::ok(b"<files/>"),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(10),
        };

        let start = Instant::now();
        let err = fetch_xml_metadata_from_url(
            server.url("/download/missing/missing_files.xml"),
            &client,
            &spinner,
            None,
            &policy,
            &signal,
        )
        .await
        .unwrap_err();
        let elapsed = start.elapsed();

        let requests = server.abort_and_collect().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/missing/missing_files.xml "));
        assert!(
            elapsed < Duration::from_secs(1),
            "XML metadata 404 should fail immediately, took {:?}",
            elapsed
        );
        assert!(matches!(err, ia_get::IaGetError::Network(_)));
        assert!(err.to_string().contains("404"));
    }

    #[tokio::test]
    async fn is_url_accessible_ctrl_c_during_retry_wait_exits_promptly() {
        let server = MockHttpServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "30".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(b""),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(60),
        };

        let requests_handle = server.requests_handle();
        let trigger_signal = signal.clone();
        let trigger_task = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if !requests_handle.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });

        let start = Instant::now();
        let err = is_url_accessible_with_policy(
            &server.url("/details/item"),
            &client,
            None,
            &policy,
            &signal,
        )
        .await
        .unwrap_err();
        let elapsed = start.elapsed();
        trigger_task.await.unwrap();

        let requests = server.abort_and_collect().await;
        assert!(
            elapsed < Duration::from_secs(1),
            "Ctrl+C during HEAD Retry-After wait should exit promptly, took {:?}",
            elapsed
        );
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, ia_get::IaGetError::FileSystem(_)));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[tokio::test]
    async fn fetch_xml_metadata_ctrl_c_during_get_retry_wait_exits_promptly() {
        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "30".to_string()),
                    ("Content-Length", "12".to_string()),
                ],
                b"rate limited",
            ),
            MockResponse::ok(b"<files/>"),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(60),
        };

        let requests_handle = server.requests_handle();
        let trigger_signal = signal.clone();
        let trigger_task = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if requests_handle.lock().await.len() >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });

        let start = Instant::now();
        let err = fetch_xml_metadata_from_url(
            server.url("/download/item/item_files.xml"),
            &client,
            &spinner,
            None,
            &policy,
            &signal,
        )
        .await
        .unwrap_err();
        let elapsed = start.elapsed();
        trigger_task.await.unwrap();

        let requests = server.abort_and_collect().await;
        assert!(
            elapsed < Duration::from_secs(1),
            "Ctrl+C during XML GET Retry-After wait should exit promptly, took {:?}",
            elapsed
        );
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert!(matches!(err, ia_get::IaGetError::FileSystem(_)));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[test]
    fn batch_exit_result_returns_ok_for_successful_and_empty_batches() {
        let all_downloaded = downloader::BatchSummary {
            ok: 3,
            skipped: 0,
            failed: Vec::new(),
            interrupted: false,
        };
        assert!(batch_exit_result(&all_downloaded, 3).is_ok());

        let mixed_ok_and_skipped = downloader::BatchSummary {
            ok: 1,
            skipped: 2,
            failed: Vec::new(),
            interrupted: false,
        };
        assert!(batch_exit_result(&mixed_ok_and_skipped, 3).is_ok());

        let all_skipped = downloader::BatchSummary {
            ok: 0,
            skipped: 4,
            failed: Vec::new(),
            interrupted: false,
        };
        assert!(batch_exit_result(&all_skipped, 4).is_ok());

        let empty = downloader::BatchSummary::default();
        assert!(batch_exit_result(&empty, 0).is_ok());
    }

    #[test]
    fn batch_exit_result_returns_batch_failed_when_files_fail() {
        let summary = downloader::BatchSummary {
            ok: 2,
            skipped: 0,
            failed: vec![downloader::FailedFile {
                file_path: "missing.bin".to_string(),
                error: "Network error: HTTP error 404 Not Found".to_string(),
            }],
            interrupted: false,
        };

        let err = batch_exit_result(&summary, 3).unwrap_err();
        assert!(
            matches!(
                err,
                IaGetError::BatchFailed {
                    failed: 1,
                    total: 3
                }
            ),
            "expected BatchFailed {{ failed: 1, total: 3 }}, got: {err:?}"
        );
    }

    #[test]
    fn batch_exit_result_returns_interrupted_with_or_without_earlier_failures() {
        let interrupted_clean = downloader::BatchSummary {
            ok: 1,
            skipped: 0,
            failed: Vec::new(),
            interrupted: true,
        };
        assert!(matches!(
            batch_exit_result(&interrupted_clean, 3).unwrap_err(),
            IaGetError::Interrupted
        ));

        let interrupted_after_failure = downloader::BatchSummary {
            ok: 1,
            skipped: 0,
            failed: vec![downloader::FailedFile {
                file_path: "broken.bin".to_string(),
                error: "Network error: HTTP error 503 Service Unavailable".to_string(),
            }],
            interrupted: true,
        };
        assert!(matches!(
            batch_exit_result(&interrupted_after_failure, 3).unwrap_err(),
            IaGetError::Interrupted
        ));
    }

    struct TempCookieFile(std::path::PathBuf);

    impl TempCookieFile {
        fn write(label: &str, content: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ia-get-main-cookies-{}-{}-{}.txt",
                std::process::id(),
                label,
                id
            ));
            std::fs::write(&path, content).expect("write temp cookies.txt");
            Self(path)
        }

        fn as_str(&self) -> &str {
            self.0.to_str().expect("valid UTF-8 temp path")
        }
    }

    impl Drop for TempCookieFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[tokio::test]
    async fn is_url_accessible_preserves_head_and_strips_cookie_across_cross_host_redirect() {
        let target_server = MockHttpServer::start_on_localhost(vec![MockResponse::ok(b"")]).await;
        let redirect_url = target_server.url("/details/redirected-item");
        let origin_server = MockHttpServer::start(vec![MockResponse::with_headers(
            "302 Found",
            vec![
                ("Location", redirect_url.to_string()),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )])
        .await;

        let cookie_source =
            CookieSource::from_netscape_str("127.0.0.1\tFALSE\t/\tFALSE\t0\tauth\tsecret\n");
        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy::default();

        is_url_accessible_with_policy(
            &origin_server.url("/details/item"),
            &client,
            Some(&cookie_source),
            &policy,
            &signal,
        )
        .await
        .expect("HEAD across cross-host 302 redirect should succeed");

        let origin_requests = origin_server.finish().await;
        let target_requests = target_server.finish().await;
        assert_eq!(origin_requests.len(), 1);
        assert!(origin_requests[0]
            .request_line
            .starts_with("HEAD /details/item "));
        assert_eq!(origin_requests[0].header("cookie"), Some("auth=secret"));

        assert_eq!(target_requests.len(), 1);
        assert!(target_requests[0]
            .request_line
            .starts_with("HEAD /details/redirected-item "));
        assert_eq!(
            target_requests[0].header("cookie"),
            None,
            "redirected HEAD to non-matching host must strip Cookie header"
        );
    }

    #[tokio::test]
    async fn fetch_xml_metadata_rescopes_path_cookies_across_redirects_and_preserves_base_url() {
        let xml_body = br#"<files>
            <file name="issue01.pdf" source="original">
                <size>2048</size>
                <md5>abcdef0123456789abcdef0123456789</md5>
            </file>
        </files>"#;

        let server = MockHttpServer::start(vec![
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/download/public/item_files.xml".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(b""),
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/download/public/item_files.xml".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(xml_body),
        ])
        .await;

        let cookie_file = TempCookieFile::write(
            "path-scope",
            concat!(
                "# Netscape HTTP Cookie File\n",
                "127.0.0.1\tFALSE\t/\tFALSE\t0\troot\t1\n",
                "127.0.0.1\tFALSE\t/download/private\tFALSE\t0\tprivate\t1\n",
            ),
        );
        let cookie_source =
            CookieSource::from_input(cookie_file.as_str()).expect("load temp cookies.txt");

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy::default();
        let xml_url = server.url("/download/private/item_files.xml");

        let (files, returned_url) = fetch_xml_metadata_from_url(
            xml_url.clone(),
            &client,
            &spinner,
            Some(&cookie_source),
            &policy,
            &signal,
        )
        .await
        .expect("XML metadata HEAD and GET across redirects should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/private/item_files.xml "));
        assert_eq!(requests[0].header("cookie"), Some("private=1; root=1"));

        assert!(requests[1]
            .request_line
            .starts_with("HEAD /download/public/item_files.xml "));
        assert_eq!(requests[1].header("cookie"), Some("root=1"));

        assert!(requests[2]
            .request_line
            .starts_with("GET /download/private/item_files.xml "));
        assert_eq!(requests[2].header("cookie"), Some("private=1; root=1"));

        assert!(requests[3]
            .request_line
            .starts_with("GET /download/public/item_files.xml "));
        assert_eq!(requests[3].header("cookie"), Some("root=1"));

        assert_eq!(
            returned_url, xml_url,
            "returned base_url must preserve the original XML URL"
        );
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "issue01.pdf");
        assert_eq!(files.files[0].size, Some(2048));
        assert_eq!(
            files.files[0].md5.as_deref(),
            Some("abcdef0123456789abcdef0123456789")
        );
    }

    #[tokio::test]
    async fn fetch_xml_metadata_reevaluates_cookie_expiry_across_get_retry() {
        let xml_body = br#"<files>
            <file name="archive.zip" source="original">
                <size>1024</size>
                <md5>0123456789abcdef0123456789abcdef</md5>
            </file>
        </files>"#;

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", "2".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
                b"busy",
            ),
            MockResponse::ok(xml_body),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };
        let xml_url = server.url("/download/item/item_files.xml");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_secs();
        let cookie_source = CookieSource::from_netscape_str(&format!(
            "127.0.0.1\tFALSE\t/\tFALSE\t{}\tshort\ttemp\n\
             127.0.0.1\tFALSE\t/\tFALSE\t{}\tlong\tkeep\n",
            now + 2,
            now + 3600
        ));

        let (files, returned_url) = fetch_xml_metadata_from_url(
            xml_url.clone(),
            &client,
            &spinner,
            Some(&cookie_source),
            &policy,
            &signal,
        )
        .await
        .expect("XML GET retry after 503 should succeed with re-evaluated cookies");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert_eq!(requests[0].header("cookie"), Some("short=temp; long=keep"));

        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert_eq!(requests[1].header("cookie"), Some("short=temp; long=keep"));

        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert_eq!(
            requests[2].header("cookie"),
            Some("long=keep"),
            "expired short-lived cookie must be omitted on XML GET retry"
        );

        assert_eq!(returned_url, xml_url);
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "archive.zip");
    }

    struct TempTestDir(PathBuf);

    impl TempTestDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);

            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ia-get-main-test-{}-{}-{}",
                label,
                std::process::id(),
                id
            ));
            std::fs::create_dir_all(&path).expect("create temp test dir");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cli_command_debug_assert_and_help_include_output_dir_and_examples() {
        use clap::CommandFactory;

        Cli::command().debug_assert();

        let help = Cli::command().render_help().to_string();
        assert!(help.contains("-o"), "help should mention -o:\n{help}");
        assert!(
            help.contains("--output-dir"),
            "help should mention --output-dir:\n{help}"
        );
        assert!(help.contains("<DIR>"), "help should mention <DIR>:\n{help}");
        assert!(
            help.contains("Examples:"),
            "help should include Examples section:\n{help}"
        );
        assert!(
            help.contains("ia-get -o ./downloads"),
            "help should include -o usage example:\n{help}"
        );
        assert!(
            help.contains("ia-get --output-dir /tmp/zzap64"),
            "help should include --output-dir usage example:\n{help}"
        );
    }

    #[test]
    fn cli_parses_output_dir_variants_and_flag_combinations() {
        let url = "https://archive.org/details/deftributetozzap64";

        let omitted = Cli::try_parse_from(["ia-get", url]).unwrap();
        assert_eq!(omitted.url, url);
        assert_eq!(omitted.output_dir, None);
        assert!(!omitted.list);
        assert_eq!(omitted.cookies, None);

        let short = Cli::try_parse_from(["ia-get", "-o", "/tmp/out", url]).unwrap();
        assert_eq!(short.output_dir, Some(PathBuf::from("/tmp/out")));

        let long = Cli::try_parse_from(["ia-get", "--output-dir", "/tmp/out", url]).unwrap();
        assert_eq!(long.output_dir, Some(PathBuf::from("/tmp/out")));

        let with_list = Cli::try_parse_from(["ia-get", "-l", "-o", "/tmp/out", url]).unwrap();
        assert!(with_list.list);
        assert_eq!(with_list.output_dir, Some(PathBuf::from("/tmp/out")));

        let with_cookies = Cli::try_parse_from([
            "ia-get",
            "-b",
            "cookies.txt",
            "--output-dir",
            "/tmp/out",
            url,
        ])
        .unwrap();
        assert_eq!(with_cookies.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(with_cookies.output_dir, Some(PathBuf::from("/tmp/out")));

        let combined_trailing =
            Cli::try_parse_from(["ia-get", url, "-l", "-b", "auth=1", "-o", "./downloads"])
                .unwrap();
        assert!(combined_trailing.list);
        assert_eq!(combined_trailing.cookies.as_deref(), Some("auth=1"));
        assert_eq!(
            combined_trailing.output_dir,
            Some(PathBuf::from("./downloads"))
        );
    }

    #[test]
    fn cli_preserves_verbatim_windows_output_dir_paths() {
        let url = "https://archive.org/details/deftributetozzap64";

        let drive_root = Cli::try_parse_from(["ia-get", "-o", r"C:\", url]).unwrap();
        assert_eq!(drive_root.output_dir, Some(PathBuf::from(r"C:\")));

        let windows_abs =
            Cli::try_parse_from(["ia-get", "--output-dir", r"C:\downloads\archive", url]).unwrap();
        assert_eq!(
            windows_abs.output_dir,
            Some(PathBuf::from(r"C:\downloads\archive"))
        );

        let windows_verbatim =
            Cli::try_parse_from(["ia-get", "--output-dir", r"\\?\C:\downloads", url]).unwrap();
        assert_eq!(
            windows_verbatim.output_dir,
            Some(PathBuf::from(r"\\?\C:\downloads"))
        );
    }

    #[test]
    fn resolve_output_dir_defaults_to_dot_and_preserves_explicit_path() {
        assert_eq!(resolve_output_dir(None), Path::new("."));
        assert_eq!(
            resolve_output_dir(Some(Path::new("/tmp/out"))),
            Path::new("/tmp/out")
        );
    }

    #[test]
    fn finalize_metadata_phase_list_mode_does_not_create_missing_output_dir() {
        let sandbox = TempTestDir::new("finalize-list");
        let missing_dir = sandbox.join("missing/nested/output");
        assert!(!missing_dir.exists());

        let cli = Cli::try_parse_from([
            "ia-get",
            "--list",
            "-o",
            missing_dir.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let files = XmlFiles {
            files: vec![xml_file("disk1.zip", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let result = finalize_metadata_phase(&cli, &files, &spinner)
            .expect("--list should succeed without touching output directory");
        assert_eq!(result, None);
        assert!(!missing_dir.exists());
        assert!(!sandbox.join("missing").exists());
    }

    #[test]
    fn finalize_metadata_phase_creates_missing_output_dir_when_not_listing() {
        let sandbox = TempTestDir::new("finalize-create");
        let target_dir = sandbox.join("new/nested/output");
        assert!(!target_dir.exists());

        let cli = Cli::try_parse_from([
            "ia-get",
            "-o",
            target_dir.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let files = XmlFiles {
            files: vec![xml_file("disk1.zip", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let result = finalize_metadata_phase(&cli, &files, &spinner)
            .expect("non-list mode should create missing output directory");
        assert!(target_dir.is_dir());
        assert_eq!(result, Some(std::fs::canonicalize(&target_dir).unwrap()));
    }

    #[test]
    fn finalize_metadata_phase_fails_on_existing_regular_file() {
        let sandbox = TempTestDir::new("finalize-file-err");
        let existing_file = sandbox.join("not-a-dir.txt");
        std::fs::write(&existing_file, b"existing-data").unwrap();

        let cli = Cli::try_parse_from([
            "ia-get",
            "--output-dir",
            existing_file.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let files = XmlFiles {
            files: vec![xml_file("disk1.zip", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let err = finalize_metadata_phase(&cli, &files, &spinner).unwrap_err();
        assert!(
            matches!(err, IaGetError::FileSystem(_)),
            "expected FileSystem error, got: {err:?}"
        );
        assert!(
            err.to_string().contains("not a directory"),
            "unexpected error message: {err}"
        );
        assert_eq!(std::fs::read(&existing_file).unwrap(), b"existing-data");
    }

    #[test]
    fn finalize_metadata_phase_defaults_to_canonical_current_working_dir() {
        let cli = Cli::try_parse_from(["ia-get", "https://archive.org/details/item"]).unwrap();
        assert_eq!(cli.output_dir, None);
        assert!(!cli.list);

        let files = XmlFiles {
            files: vec![xml_file("disk1.zip", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();
        let expected = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();

        let result = finalize_metadata_phase(&cli, &files, &spinner)
            .expect("default output directory should resolve current working directory");
        assert_eq!(result, Some(expected));
    }

    fn xml_file_with_md5(
        name: &str,
        size: Option<u64>,
        md5: Option<&str>,
    ) -> ia_get::archive_metadata::XmlFile {
        let mut entry = xml_file(name, size);
        entry.md5 = md5.map(str::to_string);
        entry
    }

    #[test]
    fn select_target_files_details_returns_all_entries() {
        let files = XmlFiles {
            files: vec![
                xml_file_with_md5(
                    "cover.jpg",
                    Some(1024),
                    Some("11111111111111111111111111111111"),
                ),
                xml_file_with_md5(
                    "sub/dir/issue01.pdf",
                    Some(8192),
                    Some("22222222222222222222222222222222"),
                ),
            ],
        };
        let target = ArchiveUrl::Details {
            identifier: "item".to_string(),
        };

        let selected = select_target_files(files, &target).expect("details should return all");
        assert_eq!(selected.files.len(), 2);
        assert_eq!(selected.files[0].name, "cover.jpg");
        assert_eq!(selected.files[0].size, Some(1024));
        assert_eq!(
            selected.files[0].md5.as_deref(),
            Some("11111111111111111111111111111111")
        );
        assert_eq!(selected.files[1].name, "sub/dir/issue01.pdf");
        assert_eq!(selected.files[1].size, Some(8192));
        assert_eq!(
            selected.files[1].md5.as_deref(),
            Some("22222222222222222222222222222222")
        );
    }

    #[test]
    fn select_target_files_download_matches_top_level_and_nested_entries() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file_with_md5(
                    "issue01.pdf",
                    Some(4096),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                ),
                xml_file_with_md5(
                    "sub/dir/report.pdf",
                    Some(12_288),
                    Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                ),
                xml_file_with_md5("notes.txt", None, None),
            ],
        };

        let top_level_target = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "issue01.pdf".to_string(),
        };
        let selected_top = select_target_files(make_files(), &top_level_target)
            .expect("top-level file should match");
        assert_eq!(selected_top.files.len(), 1);
        assert_eq!(selected_top.files[0].name, "issue01.pdf");
        assert_eq!(selected_top.files[0].size, Some(4096));
        assert_eq!(
            selected_top.files[0].md5.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let nested_target = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "sub/dir/report.pdf".to_string(),
        };
        let selected_nested =
            select_target_files(make_files(), &nested_target).expect("nested file should match");
        assert_eq!(selected_nested.files.len(), 1);
        assert_eq!(selected_nested.files[0].name, "sub/dir/report.pdf");
        assert_eq!(selected_nested.files[0].size, Some(12_288));
        assert_eq!(
            selected_nested.files[0].md5.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn select_target_files_download_normalises_leading_slash_and_percent_encoded_xml_names() {
        let leading_slash_files = XmlFiles {
            files: vec![
                xml_file_with_md5(
                    "/sub/dir/file.pdf",
                    Some(2048),
                    Some("cccccccccccccccccccccccccccccccc"),
                ),
                xml_file_with_md5("other.txt", Some(16), None),
            ],
        };
        let target = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "sub/dir/file.pdf".to_string(),
        };
        let selected = select_target_files(leading_slash_files, &target)
            .expect("leading slash in XML name should match");
        assert_eq!(selected.files.len(), 1);
        assert_eq!(selected.files[0].name, "/sub/dir/file.pdf");
        assert_eq!(selected.files[0].size, Some(2048));
        assert_eq!(
            selected.files[0].md5.as_deref(),
            Some("cccccccccccccccccccccccccccccccc")
        );

        let encoded_xml_files = XmlFiles {
            files: vec![
                xml_file_with_md5("other.pdf", Some(512), None),
                xml_file_with_md5(
                    "/sub/my%20file%20caf%C3%A9.pdf",
                    Some(9999),
                    Some("dddddddddddddddddddddddddddddddd"),
                ),
            ],
        };
        let decoded_target = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "sub/my file café.pdf".to_string(),
        };
        let selected_encoded = select_target_files(encoded_xml_files, &decoded_target)
            .expect("percent-encoded XML name fallback should match");
        assert_eq!(selected_encoded.files.len(), 1);
        assert_eq!(
            selected_encoded.files[0].name,
            "/sub/my%20file%20caf%C3%A9.pdf"
        );
        assert_eq!(selected_encoded.files[0].size, Some(9999));
        assert_eq!(
            selected_encoded.files[0].md5.as_deref(),
            Some("dddddddddddddddddddddddddddddddd")
        );
    }

    #[test]
    fn select_target_files_download_returns_file_not_found_on_miss() {
        let files = XmlFiles {
            files: vec![
                xml_file_with_md5("cover.jpg", Some(1024), None),
                xml_file_with_md5("sub/existing.pdf", Some(2048), None),
            ],
        };
        let target = ArchiveUrl::Download {
            identifier: "my-item".to_string(),
            file_path: "sub/missing.pdf".to_string(),
        };

        let err = select_target_files(files, &target).unwrap_err();
        assert!(
            matches!(
                &err,
                IaGetError::FileNotFound { file, item }
                    if file == "sub/missing.pdf" && item == "my-item"
            ),
            "expected FileNotFound {{ file: \"sub/missing.pdf\", item: \"my-item\" }}, got {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "File 'sub/missing.pdf' not found in archive item 'my-item'"
        );
    }

    #[test]
    fn build_file_download_url_encodes_segments_and_strips_leading_slash_query_and_fragment() {
        let base_url =
            Url::parse("https://archive.org/download/item/item_files.xml").expect("valid base URL");

        let cases = [
            ("file.pdf", "https://archive.org/download/item/file.pdf"),
            (
                "sub/dir/file.pdf",
                "https://archive.org/download/item/sub/dir/file.pdf",
            ),
            (
                "sub/file name.pdf",
                "https://archive.org/download/item/sub/file%20name.pdf",
            ),
            (
                "sub/café.pdf",
                "https://archive.org/download/item/sub/caf%C3%A9.pdf",
            ),
            (
                "sub/notes?v=1#top.txt",
                "https://archive.org/download/item/sub/notes%3Fv=1%23top.txt",
            ),
            (
                "/sub/dir/file.pdf",
                "https://archive.org/download/item/sub/dir/file.pdf",
            ),
        ];

        for (file_name, expected) in cases {
            let built = build_file_download_url(&base_url, file_name);
            assert_eq!(built.as_str(), expected, "file_name: {file_name:?}");
            assert_eq!(built.query(), None, "query must be None for {file_name:?}");
            assert_eq!(
                built.fragment(),
                None,
                "fragment must be None for {file_name:?}"
            );
        }

        let base_with_query_and_fragment =
            Url::parse("https://archive.org/download/item/item_files.xml?download=1#section")
                .expect("valid base URL");
        let built = build_file_download_url(&base_with_query_and_fragment, "file.pdf");
        assert_eq!(built.as_str(), "https://archive.org/download/item/file.pdf");
        assert_eq!(built.query(), None);
        assert_eq!(built.fragment(), None);
    }

    #[test]
    fn list_mode_for_download_url_lists_only_matched_file_without_creating_output_dir() {
        let sandbox = TempTestDir::new("list-download-url");
        let missing_dir = sandbox.join("never/created");
        assert!(!missing_dir.exists());

        let url = "https://archive.org/download/item/sub/target%20file.pdf?download=1#page=2";
        let cli =
            Cli::try_parse_from(["ia-get", "--list", "-o", missing_dir.to_str().unwrap(), url])
                .unwrap();
        let target = parse_archive_url(&cli.url).expect("download URL should parse");

        let all_files = XmlFiles {
            files: vec![
                xml_file("cover.jpg", Some(2048)),
                xml_file("sub/target file.pdf", Some(10_240)),
                xml_file("notes.txt", None),
            ],
        };
        let selected = select_target_files(all_files, &target)
            .expect("target file should be selected from metadata");

        assert_eq!(selected.files.len(), 1);
        assert_eq!(
            list_file_rows(&selected),
            vec!["  10.00KB sub/target file.pdf".to_string()]
        );
        assert_eq!(list_summary(&selected), "1 file, 10.00KB total");

        let spinner = indicatif::ProgressBar::hidden();
        let output_dir = finalize_metadata_phase(&cli, &selected, &spinner)
            .expect("--list should succeed without touching output directory");
        assert_eq!(output_dir, None);
        assert!(!missing_dir.exists());
        assert!(!sandbox.join("never").exists());
    }

    #[tokio::test]
    async fn direct_download_end_to_end_verifies_md5_and_strips_query_and_fragment() {
        let target = parse_archive_url(
            "https://archive.org/download/item/sub/file%20name.pdf?download=1#page=2",
        )
        .expect("direct download URL should parse");
        assert_eq!(
            target,
            ArchiveUrl::Download {
                identifier: "item".to_string(),
                file_path: "sub/file name.pdf".to_string(),
            }
        );

        let payload = b"direct single-file payload for WW-386";
        let expected_md5 = format!("{:x}", md5::compute(payload));
        let xml_body = format!(
            r#"<files>
                <file name="cover.jpg" source="original">
                    <size>128</size>
                    <md5>11111111111111111111111111111111</md5>
                </file>
                <file name="sub/file name.pdf" source="original">
                    <size>{}</size>
                    <md5>{}</md5>
                </file>
                <file name="other/extra.zip" source="original">
                    <size>512</size>
                    <md5>22222222222222222222222222222222</md5>
                </file>
            </files>"#,
            payload.len(),
            expected_md5
        );

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (all_files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .expect("XML metadata fetch should succeed");

        let selected =
            select_target_files(all_files, &target).expect("single target file should be selected");
        assert_eq!(selected.files.len(), 1);

        let download_data: Vec<_> = selected
            .files
            .into_iter()
            .map(|file| {
                let absolute_url = build_file_download_url(&base_url, &file.name);
                let (sanitized_name, _) = sanitize_filename(&file.name);
                (absolute_url.to_string(), sanitized_name, file.md5)
            })
            .collect();

        let output_dir = TempTestDir::new("direct-download-ok");
        let summary = downloader::download_files_in_dir(
            &client,
            &output_dir.0,
            download_data,
            1,
            None,
            &policy,
            &signal,
        )
        .await
        .expect("single-file download should complete");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(batch_exit_result(&summary, 1).is_ok());

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert!(
            requests[2]
                .request_line
                .starts_with("GET /download/item/sub/file%20name.pdf "),
            "unexpected download request line: {}",
            requests[2].request_line
        );
        assert!(!requests[2].request_line.contains('?'));
        assert!(!requests[2].request_line.contains('#'));

        let saved_path = output_dir.join("sub/file name.pdf");
        assert_eq!(
            std::fs::read(&saved_path).expect("saved file should exist"),
            payload
        );
        assert!(!output_dir.join("cover.jpg").exists());
        assert!(!output_dir.join("other/extra.zip").exists());

        let sub_entries: Vec<_> = std::fs::read_dir(output_dir.join("sub"))
            .expect("sub directory should exist")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(sub_entries, vec!["file name.pdf".to_string()]);
    }

    #[tokio::test]
    async fn direct_download_end_to_end_fails_on_md5_mismatch_without_promoting_file() {
        let target = parse_archive_url("https://archive.org/download/item/sub/file%20name.pdf")
            .expect("direct download URL should parse");

        let expected_payload = b"expected clean content";
        let corrupt_payload = b"corrupted download bytes";
        let expected_md5 = format!("{:x}", md5::compute(expected_payload));
        let xml_body = format!(
            r#"<files>
                <file name="other.pdf" source="original">
                    <size>64</size>
                    <md5>11111111111111111111111111111111</md5>
                </file>
                <file name="sub/file name.pdf" source="original">
                    <size>{}</size>
                    <md5>{}</md5>
                </file>
            </files>"#,
            expected_payload.len(),
            expected_md5
        );

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(corrupt_payload),
        ])
        .await;

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (all_files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .expect("XML metadata fetch should succeed");

        let selected =
            select_target_files(all_files, &target).expect("single target file should be selected");
        let download_data: Vec<_> = selected
            .files
            .into_iter()
            .map(|file| {
                let absolute_url = build_file_download_url(&base_url, &file.name);
                let (sanitized_name, _) = sanitize_filename(&file.name);
                (absolute_url.to_string(), sanitized_name, file.md5)
            })
            .collect();

        let output_dir = TempTestDir::new("direct-download-md5-fail");
        let summary = downloader::download_files_in_dir(
            &client,
            &output_dir.0,
            download_data,
            1,
            None,
            &policy,
            &signal,
        )
        .await
        .expect("batch runner should return summary with failure");

        assert_eq!(summary.ok, 0);
        assert_eq!(summary.skipped, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "sub/file name.pdf");
        assert!(
            summary.failed[0].error.contains("MD5 mismatch"),
            "expected MD5 mismatch error, got: {}",
            summary.failed[0].error
        );
        assert!(matches!(
            batch_exit_result(&summary, 1),
            Err(IaGetError::BatchFailed {
                failed: 1,
                total: 1
            })
        ));
        assert!(
            !output_dir.join("sub/file name.pdf").exists(),
            "corrupt download must not be promoted to the final filename"
        );

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/sub/file%20name.pdf "));
    }

    #[test]
    fn cli_help_documents_details_and_download_urls_and_exact_flags() {
        use clap::CommandFactory;

        Cli::command().debug_assert();
        let mut cmd = Cli::command();

        let option_flags: Vec<(Option<char>, Option<&str>)> = cmd
            .get_arguments()
            .filter(|arg| {
                !arg.is_positional() && arg.get_id() != "help" && arg.get_id() != "version"
            })
            .map(|arg| (arg.get_short(), arg.get_long()))
            .collect();
        assert_eq!(
            option_flags,
            vec![
                (Some('l'), Some("list")),
                (Some('b'), Some("cookies")),
                (Some('o'), Some("output-dir")),
            ]
        );

        let help = cmd.render_help().to_string();
        assert!(
            help.contains("/details/<item>"),
            "help should document /details/<item>:\n{help}"
        );
        assert!(
            help.contains("/download/<item>/<path>"),
            "help should document /download/<item>/<path>:\n{help}"
        );
        assert!(
            help.contains("ia-get https://archive.org/details/deftributetozzap64"),
            "help should include /details/ example:\n{help}"
        );
        assert!(
            help.contains(
                "ia-get https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf"
            ),
            "help should include /download/ example:\n{help}"
        );
        assert!(
            help.contains(
                "ia-get --list https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf"
            ),
            "help should include --list /download/ example:\n{help}"
        );
        assert!(
            help.contains("-l, --list"),
            "help should expose -l/--list:\n{help}"
        );
        assert!(
            help.contains("-b, --cookies"),
            "help should expose -b/--cookies:\n{help}"
        );
        assert!(
            help.contains("-o, --output-dir"),
            "help should expose -o/--output-dir:\n{help}"
        );
    }
}
