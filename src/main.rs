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
use ia_get::disk_space;
use ia_get::downloader;
use ia_get::utils::{
    create_spinner, format_size, parse_archive_url, percent_decode_utf8, sanitize_filename,
    sanitize_url_for_log, ArchiveUrl,
};
use ia_get::verify;
use ia_get::{CookieSource, FileFilter, IaGetError, Result, VerboseLogger};
use indicatif::ProgressStyle;
use reqwest::header::{HeaderMap, COOKIE};
use reqwest::{Client, Url};

/// Extended timeout for large file downloads (10 minutes for connection, no read timeout)
const CONNECTION_TIMEOUT_SECS: u64 = 600;

/// Checks if a URL is accessible by sending a HEAD request with an explicit retry policy and logger.
async fn is_url_accessible_with_logger(
    url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<()> {
    let _response = downloader::send_with_retry_with_logger(
        url,
        policy,
        signal,
        logger,
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

/// Checks if a URL is accessible by sending a HEAD request with an explicit retry policy.
#[allow(dead_code)]
async fn is_url_accessible_with_policy(
    url: &Url,
    client: &Client,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
) -> Result<()> {
    is_url_accessible_with_logger(
        url,
        client,
        cookie_source,
        policy,
        signal,
        &VerboseLogger::disabled(),
    )
    .await
}

/// Checks if a URL is accessible by sending a HEAD request
#[allow(dead_code)]
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

/// Fetches and parses XML metadata from an already-resolved XML URL with an explicit retry policy
/// and diagnostic logger.
async fn fetch_xml_metadata_from_url_with_logger(
    base_url: reqwest::Url,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<(XmlFiles, reqwest::Url)> {
    let xml_url = base_url.as_str();
    spinner.set_message(format!(
        "{} Accessing XML metadata: {}",
        "⚙".blue(),
        xml_url.bold()
    ));

    // Check XML URL accessibility
    if let Err(e) =
        is_url_accessible_with_logger(&base_url, client, cookie_source, policy, signal, logger)
            .await
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

    let xml_content = downloader::fetch_text_with_retry_with_logger(
        &base_url,
        policy,
        signal,
        logger,
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
    logger.log(
        "metadata",
        format_args!(
            "parsed {} file(s) from {}",
            files.files.len(),
            sanitize_url_for_log(&base_url)
        ),
    );

    Ok((files, base_url))
}

/// Fetches and parses XML metadata from an already-resolved XML URL with an explicit retry policy.
#[allow(dead_code)]
async fn fetch_xml_metadata_from_url(
    base_url: reqwest::Url,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
) -> Result<(XmlFiles, reqwest::Url)> {
    fetch_xml_metadata_from_url_with_logger(
        base_url,
        client,
        spinner,
        cookie_source,
        policy,
        signal,
        &VerboseLogger::disabled(),
    )
    .await
}

/// Fetches and parses XML metadata from archive.org with diagnostic logging.
async fn fetch_xml_metadata_with_logger(
    target: &ArchiveUrl,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    signal: &downloader::ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<(XmlFiles, reqwest::Url)> {
    let xml_url = target.xml_url();
    let base_url = reqwest::Url::parse(&xml_url)?;

    fetch_xml_metadata_from_url_with_logger(
        base_url,
        client,
        spinner,
        cookie_source,
        &downloader::RetryPolicy::default(),
        signal,
        logger,
    )
    .await
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
#[allow(dead_code)]
async fn fetch_xml_metadata(
    target: &ArchiveUrl,
    client: &Client,
    spinner: &indicatif::ProgressBar,
    cookie_source: Option<&CookieSource>,
    signal: &downloader::ShutdownSignal,
) -> Result<(XmlFiles, reqwest::Url)> {
    fetch_xml_metadata_with_logger(
        target,
        client,
        spinner,
        cookie_source,
        signal,
        &VerboseLogger::disabled(),
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

/// Selects files matching the target URL and applies any compiled user filters.
fn select_and_filter_files(
    files: XmlFiles,
    target: &ArchiveUrl,
    filter: &FileFilter,
) -> Result<XmlFiles> {
    let selected = select_target_files(files, target)?;
    filter.apply(selected)
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

/// Maps a completed verification summary to the CLI exit result.
fn verify_exit_result(summary: &verify::VerificationSummary, total_files: usize) -> Result<()> {
    if summary.interrupted {
        Err(IaGetError::Interrupted)
    } else if !summary.failed.is_empty() {
        Err(IaGetError::VerificationFailed {
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

/// Sanitised download plan built from selected archive metadata entries.
#[derive(Debug)]
struct DownloadPlan {
    items: Vec<disk_space::PlannedDownload>,
    sanitized: Vec<(String, String)>,
}

/// Builds an unvalidated download plan tuple of `(items, original_names, sanitized_renames)`.
#[allow(clippy::type_complexity)]
fn build_unvalidated_download_plan(
    files: XmlFiles,
    base_url: &Url,
) -> (
    Vec<disk_space::PlannedDownload>,
    Vec<String>,
    Vec<(String, String)>,
) {
    let mut items = Vec::with_capacity(files.files.len());
    let mut original_names = Vec::with_capacity(files.files.len());
    let mut sanitized = Vec::new();

    for file in files.files {
        let absolute_url = build_file_download_url(base_url, &file.name);
        let (sanitized_name, was_modified) = sanitize_filename(&file.name);

        if was_modified {
            sanitized.push((file.name.clone(), sanitized_name.clone()));
        }

        original_names.push(file.name);
        items.push(disk_space::PlannedDownload {
            url: absolute_url.to_string(),
            file_path: sanitized_name,
            expected_md5: file.md5,
            expected_size: file.size,
        });
    }

    (items, original_names, sanitized)
}

/// Builds and validates a sanitised download plan from selected archive metadata entries
/// using an explicit path case-sensitivity rule.
#[cfg(test)]
fn plan_downloads_with_case_sensitivity(
    files: XmlFiles,
    base_url: &Url,
    case_sensitivity: downloader::PathCaseSensitivity,
) -> Result<DownloadPlan> {
    let (items, original_names, sanitized) = build_unvalidated_download_plan(files, base_url);

    downloader::validate_download_paths_with_case_sensitivity(
        original_names
            .iter()
            .zip(items.iter())
            .map(|(original, item)| (original.as_str(), item.file_path.as_str())),
        case_sensitivity,
    )?;

    Ok(DownloadPlan { items, sanitized })
}

/// Builds and validates a sanitised download plan from selected archive metadata entries
/// according to the destination directory's filesystem case sensitivity.
///
/// Rejects plans where two entries map to the same sanitised destination path or where one
/// entry's `.part` staging path collides with another entry's destination path.
fn plan_downloads_for_dir(
    files: XmlFiles,
    base_url: &Url,
    output_dir: &Path,
) -> Result<DownloadPlan> {
    let (items, original_names, sanitized) = build_unvalidated_download_plan(files, base_url);

    downloader::validate_download_paths_for_dir(
        output_dir,
        original_names
            .iter()
            .zip(items.iter())
            .map(|(original, item)| (original.as_str(), item.file_path.as_str())),
    )?;

    Ok(DownloadPlan { items, sanitized })
}

/// Builds and validates a sanitised download plan using case-sensitive path matching.
#[cfg(test)]
fn plan_downloads(files: XmlFiles, base_url: &Url) -> Result<DownloadPlan> {
    plan_downloads_with_case_sensitivity(
        files,
        base_url,
        downloader::PathCaseSensitivity::CaseSensitive,
    )
}

/// Prints per-file warnings and a summary when filenames were sanitised.
fn print_sanitization_warnings(sanitized: &[(String, String)]) {
    if sanitized.is_empty() {
        return;
    }

    for (original_name, sanitized_name) in sanitized {
        println!(
            "{} {} {} → {}",
            "⚠".yellow().bold(),
            "Sanitized:".yellow(),
            original_name.dimmed(),
            sanitized_name.bold()
        );
    }

    let sanitized_count = sanitized.len();
    println!(
        "\n{} {} {} file{} for filesystem compatibility",
        "✓".green().bold(),
        "Sanitized".bold(),
        sanitized_count.to_string().bold(),
        if sanitized_count == 1 { "" } else { "s" }
    );
}

/// Runs the pre-download disk-space check and, when it succeeds, downloads all planned files
/// using explicit [`downloader::DownloadOptions`] and [`VerboseLogger`].
#[allow(clippy::too_many_arguments)]
async fn run_planned_downloads_with_logger<F>(
    client: &Client,
    output_dir: &Path,
    plan: DownloadPlan,
    cookie_source: Option<&CookieSource>,
    options: &downloader::DownloadOptions,
    signal: &downloader::ShutdownSignal,
    logger: &VerboseLogger,
    check_space: F,
) -> Result<downloader::BatchSummary>
where
    F: FnOnce(&Path, &[disk_space::PlannedDownload], &downloader::ShutdownSignal) -> Result<()>,
{
    downloader::validate_download_paths_for_dir(
        output_dir,
        plan.items
            .iter()
            .map(|item| (item.file_path.as_str(), item.file_path.as_str())),
    )?;

    check_space(output_dir, &plan.items, signal)?;

    print_sanitization_warnings(&plan.sanitized);

    let total_files = plan.items.len();
    let download_data = plan
        .items
        .into_iter()
        .map(|item| (item.url, item.file_path, item.expected_md5));

    downloader::download_files_in_dir_with_logger(
        client,
        output_dir,
        download_data,
        total_files,
        cookie_source,
        options,
        signal,
        logger,
    )
    .await
}

/// Runs the pre-download disk-space check and, when it succeeds, downloads all planned files
/// using explicit [`downloader::DownloadOptions`].
#[allow(dead_code)]
async fn run_planned_downloads_with_options<F>(
    client: &Client,
    output_dir: &Path,
    plan: DownloadPlan,
    cookie_source: Option<&CookieSource>,
    options: &downloader::DownloadOptions,
    signal: &downloader::ShutdownSignal,
    check_space: F,
) -> Result<downloader::BatchSummary>
where
    F: FnOnce(&Path, &[disk_space::PlannedDownload], &downloader::ShutdownSignal) -> Result<()>,
{
    let logger = VerboseLogger::from_enabled(options.verbose);
    run_planned_downloads_with_logger(
        client,
        output_dir,
        plan,
        cookie_source,
        options,
        signal,
        &logger,
        check_space,
    )
    .await
}

/// Runs the pre-download disk-space check and, when it succeeds, downloads all planned files.
#[cfg(test)]
async fn run_planned_downloads_with_space_check<F>(
    client: &Client,
    output_dir: &Path,
    plan: DownloadPlan,
    cookie_source: Option<&CookieSource>,
    policy: &downloader::RetryPolicy,
    signal: &downloader::ShutdownSignal,
    check_space: F,
) -> Result<downloader::BatchSummary>
where
    F: FnOnce(&Path, &[disk_space::PlannedDownload], &downloader::ShutdownSignal) -> Result<()>,
{
    let options = downloader::DownloadOptions {
        retry_policy: *policy,
        limit_rate: None,
        verbose: false,
    };
    run_planned_downloads_with_options(
        client,
        output_dir,
        plan,
        cookie_source,
        &options,
        signal,
        check_space,
    )
    .await
}

/// Validates a check directory or prepares the download output directory.
fn prepare_phase_dir(cli: &Cli, spinner: &indicatif::ProgressBar) -> Result<PathBuf> {
    let target_dir = resolve_output_dir(cli.output_dir.as_deref());
    let dir_result = if cli.check {
        verify::validate_check_dir(target_dir)
    } else {
        downloader::prepare_output_dir(target_dir)
    };

    match dir_result {
        Ok(canonical_dir) => Ok(canonical_dir),
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            Err(e)
        }
    }
}

/// Finalises the metadata phase by listing files, validating a check directory,
/// or preparing the output directory.
fn finalize_metadata_phase(
    cli: &Cli,
    files: &XmlFiles,
    spinner: &indicatif::ProgressBar,
) -> Result<Option<PathBuf>> {
    if cli.list {
        list_files(files, spinner);
        return Ok(None);
    }

    prepare_phase_dir(cli, spinner).map(Some)
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
  ia-get --check https://archive.org/details/deftributetozzap64
  ia-get --check -o ./downloads https://archive.org/details/deftributetozzap64
  ia-get -f '*.pdf' https://archive.org/details/deftributetozzap64
  ia-get -f '!*.torrent' https://archive.org/details/deftributetozzap64
  ia-get -b cookies.txt https://archive.org/details/deftributetozzap64
  ia-get --limit-rate 500K https://archive.org/details/deftributetozzap64
  ia-get -v https://archive.org/details/deftributetozzap64 2> ia-get.log")]
struct Cli {
    /// Archive.org item details URL (/details/<item>) or direct file URL (/download/<item>/<path>)
    url: String,
    /// List files parsed from archive metadata XML and exit
    #[arg(short = 'l', long = "list")]
    list: bool,
    /// Verify local files against archive metadata without downloading or modifying files
    #[arg(long = "check", conflicts_with = "list")]
    check: bool,
    /// Cookie header or Netscape cookies.txt file for authenticated downloads
    #[arg(short = 'b', long = "cookies", value_name = "COOKIES")]
    cookies: Option<String>,
    /// Directory to save downloaded files (defaults to current working directory)
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
    /// Filter files by glob pattern (repeatable; prefix with '!' to exclude)
    #[arg(short = 'f', long = "filter", value_name = "GLOB")]
    filters: Vec<String>,
    /// Limit download bandwidth (e.g. 500K, 2M, 1G, or raw bytes/sec; 1024-byte units)
    #[arg(
        long = "limit-rate",
        value_name = "RATE",
        value_parser = downloader::parse_rate_limit
    )]
    limit_rate: Option<u64>,
    /// Emit structured diagnostic logs to stderr while keeping progress output on stdout
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
}

/// Builds the application HTTP client with extended timeouts and explicit proxy configuration.
fn build_http_client(proxy_config: &downloader::ProxyConfig) -> Result<Client> {
    // Create a client with extended timeouts for large file downloads
    // Connection timeout is set high, but no read timeout since large files
    // may take a long time to transfer
    let client = downloader::http_client_builder_with_proxy(proxy_config)?
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(CONNECTION_TIMEOUT_SECS))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(1)
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .build()?;
    Ok(client)
}

/// Main application entry point
///
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and initiates file downloads with built-in signal handling.
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let logger = VerboseLogger::from_enabled(cli.verbose);

    let cookie_source = cli
        .cookies
        .as_deref()
        .map(CookieSource::from_input)
        .transpose()?;

    let file_filter = FileFilter::compile(&cli.filters)?;

    if cli.check {
        verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref()))?;
    }

    let proxy_config = downloader::ProxyConfig::from_env()?;
    let client = build_http_client(&proxy_config)?;

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
    if let Err(e) = is_url_accessible_with_logger(
        &details_url,
        &client,
        cookie_source.as_ref(),
        &downloader::RetryPolicy::default(),
        &shutdown,
        &logger,
    )
    .await
    {
        spinner.finish_with_message(format!(
            "{} Archive.org URL not accessible: {}",
            "✘".red().bold(),
            cli.url.bold()
        ));
        return Err(e.into()); // Propagate error
    }

    // Fetch and parse XML metadata in one operation
    let (files, base_url) = fetch_xml_metadata_with_logger(
        &target,
        &client,
        &spinner,
        cookie_source.as_ref(),
        &shutdown,
        &logger,
    )
    .await?;

    // Narrow metadata to the requested target and apply any user-supplied glob filters
    let files = match select_and_filter_files(files, &target, &file_filter) {
        Ok(files) => files,
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            return Err(e.into());
        }
    };

    // If requested, list parsed filenames and exit; or validate directory and verify local files
    if cli.list || cli.check {
        let Some(output_dir) = finalize_metadata_phase(&cli, &files, &spinner)? else {
            return Ok(());
        };

        let file_label = if files.files.len() == 1 {
            "file"
        } else {
            "files"
        };

        spinner.set_style(
            ProgressStyle::default_spinner()
                .template(&format!(
                    "{} {} {} {file_label} against archive.org metadata",
                    "✔".green().bold(),
                    "Checking".bold(),
                    files.files.len().to_string().bold(),
                ))
                .expect("Failed to set completion style"),
        );
        spinner.finish();

        let summary =
            verify::verify_files_with_logger(&output_dir, &files.files, &shutdown, &logger)?;
        verify_exit_result(&summary, files.files.len())?;
        return Ok(());
    }

    // Build and validate the download plan against the destination filesystem's case sensitivity
    // before creating missing output directories, checking disk space, or starting transfers.
    let target_dir = resolve_output_dir(cli.output_dir.as_deref());
    let plan = match plan_downloads_for_dir(files, &base_url, target_dir) {
        Ok(plan) => plan,
        Err(e) => {
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
            return Err(e.into());
        }
    };
    let output_dir = prepare_phase_dir(&cli, &spinner)?;
    let total_files = plan.items.len();
    let file_label = if total_files == 1 { "file" } else { "files" };
    let options = downloader::DownloadOptions {
        retry_policy: downloader::RetryPolicy::default(),
        limit_rate: cli.limit_rate,
        verbose: cli.verbose,
    };

    let summary = run_planned_downloads_with_logger(
        &client,
        &output_dir,
        plan,
        cookie_source.as_ref(),
        &options,
        &shutdown,
        &logger,
        |dir, items, signal| {
            spinner.set_message(format!(
                "{} {}",
                "⚙".blue(),
                "Checking available disk space...".bold()
            ));
            if let Err(e) = disk_space::check_disk_space_with_signal(dir, items, signal) {
                spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
                return Err(e);
            }

            // Successfully finished initialization
            spinner.set_style(
                ProgressStyle::default_spinner()
                    .template(&format!(
                        "{} {} to download {} {file_label} from archive.org {}",
                        "✔".green().bold(),
                        "Ready".bold(),
                        total_files.to_string().bold(),
                        "★".yellow()
                    ))
                    .expect("Failed to set completion style"),
            );
            spinner.finish();
            Ok(())
        },
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
                (None, Some("check")),
                (Some('b'), Some("cookies")),
                (Some('o'), Some("output-dir")),
                (Some('f'), Some("filter")),
                (None, Some("limit-rate")),
                (Some('v'), Some("verbose")),
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
            help.contains("ia-get --check https://archive.org/details/deftributetozzap64"),
            "help should include --check example:\n{help}"
        );
        assert!(
            help.contains(
                "ia-get --check -o ./downloads https://archive.org/details/deftributetozzap64"
            ),
            "help should include --check -o example:\n{help}"
        );
        assert!(
            help.contains("ia-get -f '*.pdf' https://archive.org/details/deftributetozzap64"),
            "help should include -f include example:\n{help}"
        );
        assert!(
            help.contains("ia-get -f '!*.torrent' https://archive.org/details/deftributetozzap64"),
            "help should include -f exclude example:\n{help}"
        );
        assert!(
            help.contains("-l, --list"),
            "help should expose -l/--list:\n{help}"
        );
        assert!(
            help.contains("--check"),
            "help should expose --check:\n{help}"
        );
        assert!(
            help.contains("-b, --cookies"),
            "help should expose -b/--cookies:\n{help}"
        );
        assert!(
            help.contains("-o, --output-dir"),
            "help should expose -o/--output-dir:\n{help}"
        );
        assert!(
            help.contains("-f, --filter"),
            "help should expose -f/--filter:\n{help}"
        );
        assert!(
            help.contains("--limit-rate <RATE>"),
            "help should expose --limit-rate <RATE>:\n{help}"
        );
        assert!(
            help.contains(
                "ia-get --limit-rate 500K https://archive.org/details/deftributetozzap64"
            ),
            "help should include --limit-rate example:\n{help}"
        );
        assert!(
            help.contains("-v, --verbose"),
            "help should expose -v/--verbose:\n{help}"
        );
        assert!(
            help.contains("ia-get -v https://archive.org/details/deftributetozzap64 2> ia-get.log"),
            "help should include -v stderr redirection example:\n{help}"
        );
    }

    #[test]
    fn cli_parses_repeatable_filters_without_splitting_brace_commas() {
        let cli = Cli::try_parse_from([
            "ia-get",
            "-f",
            "*.{pdf,cbz}",
            "--filter",
            "!*draft*",
            "https://archive.org/details/deftributetozzap64",
        ])
        .unwrap();

        assert_eq!(cli.filters, vec!["*.{pdf,cbz}", "!*draft*"]);
    }

    #[test]
    fn select_and_filter_files_applies_target_selection_then_filters() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.pdf", Some(1024)),
                xml_file("issue01-draft.pdf", Some(512)),
                xml_file("cover.jpg", Some(256)),
            ],
        };
        let details = ArchiveUrl::Details {
            identifier: "item".to_string(),
        };
        let filter = FileFilter::compile(["*.pdf", "!*draft*"]).unwrap();

        let filtered = select_and_filter_files(make_files(), &details, &filter).unwrap();
        assert_eq!(filtered.files.len(), 1);
        assert_eq!(filtered.files[0].name, "issue01.pdf");

        let direct = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "cover.jpg".to_string(),
        };
        let err = select_and_filter_files(make_files(), &direct, &filter).unwrap_err();
        assert!(matches!(err, IaGetError::NoMatchingFiles { .. }));
    }

    #[test]
    fn cli_parses_filter_variants_combinations_trailing_flags_and_rejects_include_exclude() {
        use clap::error::ErrorKind;

        let url = "https://archive.org/details/deftributetozzap64";

        let omitted = Cli::try_parse_from(["ia-get", url]).unwrap();
        assert!(omitted.filters.is_empty());

        let short = Cli::try_parse_from(["ia-get", "-f", "*.pdf", url]).unwrap();
        assert_eq!(short.filters, vec!["*.pdf"]);

        let long = Cli::try_parse_from(["ia-get", "--filter", "!*.torrent", url]).unwrap();
        assert_eq!(long.filters, vec!["!*.torrent"]);

        let repeated =
            Cli::try_parse_from(["ia-get", "-f", "*.cbz", "-f", "!*thumb*", url]).unwrap();
        assert_eq!(repeated.filters, vec!["*.cbz", "!*thumb*"]);

        let brace_glob = Cli::try_parse_from(["ia-get", "-f", "*.{pdf,cbz}", url]).unwrap();
        assert_eq!(brace_glob.filters, vec!["*.{pdf,cbz}"]);

        let combined_leading = Cli::try_parse_from([
            "ia-get",
            "-l",
            "-b",
            "cookies.txt",
            "-o",
            "./downloads",
            "-f",
            "*.cbz",
            "--filter",
            "!*thumb*",
            url,
        ])
        .unwrap();
        assert!(combined_leading.list);
        assert_eq!(combined_leading.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(
            combined_leading.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(combined_leading.filters, vec!["*.cbz", "!*thumb*"]);

        let combined_trailing = Cli::try_parse_from([
            "ia-get",
            url,
            "-l",
            "-b",
            "auth=1",
            "-o",
            "./out",
            "-f",
            "*.{pdf,cbz}",
            "--filter",
            "!*thumb*",
        ])
        .unwrap();
        assert!(combined_trailing.list);
        assert_eq!(combined_trailing.cookies.as_deref(), Some("auth=1"));
        assert_eq!(combined_trailing.output_dir, Some(PathBuf::from("./out")));
        assert_eq!(combined_trailing.filters, vec!["*.{pdf,cbz}", "!*thumb*"]);

        let include_err = Cli::try_parse_from(["ia-get", "--include", "*.pdf", url])
            .err()
            .expect("--include should be rejected");
        assert_eq!(include_err.kind(), ErrorKind::UnknownArgument);

        let exclude_err = Cli::try_parse_from(["ia-get", "--exclude", "*.torrent", url])
            .err()
            .expect("--exclude should be rejected");
        assert_eq!(exclude_err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn cli_help_documents_filter_flag_value_name_and_examples() {
        use clap::CommandFactory;

        Cli::command().debug_assert();

        let help = Cli::command().render_help().to_string();
        assert!(help.contains("-f"), "help should mention -f:\n{help}");
        assert!(
            help.contains("--filter"),
            "help should mention --filter:\n{help}"
        );
        assert!(
            help.contains("<GLOB>"),
            "help should mention <GLOB> value name:\n{help}"
        );
        assert!(
            help.contains("-f '*.pdf'"),
            "help should include -f '*.pdf' example:\n{help}"
        );
        assert!(
            help.contains("-f '!*.torrent'"),
            "help should include -f '!*.torrent' example:\n{help}"
        );
    }

    #[test]
    fn invalid_cli_filter_fails_early_before_network_or_output_dir_creation() {
        let sandbox = TempTestDir::new("invalid-filter-early");
        let missing_dir = sandbox.join("never/created");
        assert!(!missing_dir.exists());

        for bad_pattern in ["", "!", "[unclosed", "![unclosed"] {
            let cli = Cli::try_parse_from([
                "ia-get",
                "-o",
                missing_dir.to_str().unwrap(),
                "-f",
                "*.pdf",
                "-f",
                bad_pattern,
                "not-even-a-valid-url",
            ])
            .unwrap();

            let err = FileFilter::compile(&cli.filters).unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::InvalidFilter { ref pattern, .. } if pattern == bad_pattern
                ),
                "bad_pattern {bad_pattern:?}: unexpected error {err:?}"
            );
            assert!(
                err.to_string()
                    .contains(&format!("Invalid filter pattern '{bad_pattern}':")),
                "bad_pattern {bad_pattern:?}: unexpected message {err}"
            );
            assert!(!missing_dir.exists());
            assert!(!sandbox.join("never").exists());
        }
    }

    #[test]
    fn select_and_filter_files_handles_details_download_excluded_and_leading_slash() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file_with_md5(
                    "issue01.pdf",
                    Some(2048),
                    Some("11111111111111111111111111111111"),
                ),
                xml_file_with_md5(
                    "/sub/issue02.pdf",
                    Some(4096),
                    Some("22222222222222222222222222222222"),
                ),
                xml_file_with_md5(
                    "item.torrent",
                    Some(512),
                    Some("33333333333333333333333333333333"),
                ),
            ],
        };

        let details = ArchiveUrl::Details {
            identifier: "item".to_string(),
        };
        let inactive = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        let all = select_and_filter_files(make_files(), &details, &inactive).unwrap();
        assert_eq!(all.files.len(), 3);

        let no_torrents = FileFilter::compile(["!*.torrent"]).unwrap();
        let details_filtered =
            select_and_filter_files(make_files(), &details, &no_torrents).unwrap();
        assert_eq!(
            details_filtered
                .files
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["issue01.pdf", "/sub/issue02.pdf"]
        );
        assert_eq!(
            details_filtered.files[0].md5.as_deref(),
            Some("11111111111111111111111111111111")
        );

        let matching_download = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "issue01.pdf".to_string(),
        };
        let pdf_filter = FileFilter::compile(["*.pdf"]).unwrap();
        let single =
            select_and_filter_files(make_files(), &matching_download, &pdf_filter).unwrap();
        assert_eq!(single.files.len(), 1);
        assert_eq!(single.files[0].name, "issue01.pdf");

        let excluded_download = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "item.torrent".to_string(),
        };
        let err =
            select_and_filter_files(make_files(), &excluded_download, &no_torrents).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "!*.torrent"
        ));

        let leading_slash_download = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "sub/issue02.pdf".to_string(),
        };
        let sub_pdf_filter = FileFilter::compile(["sub/*.pdf"]).unwrap();
        let matched_leading_slash =
            select_and_filter_files(make_files(), &leading_slash_download, &sub_pdf_filter)
                .unwrap();
        assert_eq!(matched_leading_slash.files.len(), 1);
        assert_eq!(matched_leading_slash.files[0].name, "/sub/issue02.pdf");

        let exclude_sub_pdf = FileFilter::compile(["!sub/*.pdf"]).unwrap();
        let err = select_and_filter_files(make_files(), &leading_slash_download, &exclude_sub_pdf)
            .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "!sub/*.pdf"
        ));

        let missing_download = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "does-not-exist.pdf".to_string(),
        };
        let err =
            select_and_filter_files(make_files(), &missing_download, &pdf_filter).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::FileNotFound { ref file, ref item }
                if file == "does-not-exist.pdf" && item == "item"
        ));
    }

    #[test]
    fn list_mode_with_filters_reflects_filtered_subset_and_leaves_missing_output_dir_untouched() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.pdf", Some(1024)),
                xml_file("docs/issue02.pdf", Some(2048)),
                xml_file("comics/issue01.cbz", Some(4096)),
                xml_file("comics/issue01_thumb.cbz", Some(512)),
                xml_file("item.torrent", Some(256)),
                xml_file("notes.txt", None),
            ],
        };

        let cases: &[(&str, &[&str], &[&str], &str)] = &[
            (
                "positive-pdf",
                &["-f", "*.pdf"],
                &["   1.00KB issue01.pdf", "   2.00KB docs/issue02.pdf"],
                "2 files, 3.00KB total",
            ),
            (
                "negative-torrent",
                &["-f", "!*.torrent"],
                &[
                    "   1.00KB issue01.pdf",
                    "   2.00KB docs/issue02.pdf",
                    "   4.00KB comics/issue01.cbz",
                    "     512B comics/issue01_thumb.cbz",
                    "  unknown notes.txt",
                ],
                "5 files, 7.50KB total known size, 1 unknown size",
            ),
            (
                "combined-cbz-no-thumb",
                &["-f", "*.cbz", "-f", "!*thumb*"],
                &["   4.00KB comics/issue01.cbz"],
                "1 file, 4.00KB total",
            ),
        ];

        for (label, filter_args, expected_rows, expected_summary) in cases {
            let sandbox = TempTestDir::new(&format!("list-filter-{label}"));
            let missing_dir = sandbox.join("missing/nested/output");
            assert!(!missing_dir.exists());

            let mut args = vec!["ia-get", "--list", "-o", missing_dir.to_str().unwrap()];
            args.extend_from_slice(filter_args);
            args.push("https://archive.org/details/item");

            let cli = Cli::try_parse_from(args).unwrap();
            let filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            let filtered = select_and_filter_files(make_files(), &target, &filter).unwrap();

            let expected_rows_vec: Vec<String> =
                expected_rows.iter().map(|row| (*row).to_string()).collect();
            assert_eq!(list_file_rows(&filtered), expected_rows_vec, "case {label}");
            assert_eq!(list_summary(&filtered), *expected_summary, "case {label}");

            let spinner = indicatif::ProgressBar::hidden();
            let output_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
                .expect("--list should succeed without touching output directory");
            assert_eq!(output_dir, None, "case {label}");
            assert!(!missing_dir.exists(), "case {label}");
            assert!(!sandbox.join("missing").exists(), "case {label}");
        }
    }

    async fn assert_filtered_download_end_to_end(
        label: &str,
        filter_args: &[&str],
        expected_files: &[&str],
        absent_paths: &[&str],
    ) {
        let fixture_files: &[(&str, &[u8])] = &[
            ("issue01.pdf", b"pdf-one-payload"),
            ("docs/nested/issue02.pdf", b"pdf-two-nested-payload"),
            ("comics/issue01.cbz", b"cbz-one-payload"),
            ("comics/sub/issue02.cbz", b"cbz-two-nested-payload"),
            ("comics/issue01_thumb.cbz", b"cbz-thumb-excluded-payload"),
            ("thumbs/preview.cbz", b"cbz-thumbs-dir-excluded-payload"),
            ("item_archive.torrent", b"torrent-top-excluded-payload"),
            ("meta/extra.torrent", b"torrent-nested-excluded-payload"),
        ];

        let mut xml_body = String::from("<files>\n");
        for (name, payload) in fixture_files {
            let digest = format!("{:x}", md5::compute(payload));
            xml_body.push_str(&format!(
                "  <file name=\"{name}\" source=\"original\">\n    <size>{}</size>\n    <md5>{digest}</md5>\n  </file>\n",
                payload.len()
            ));
        }
        xml_body.push_str("</files>");

        let mut responses = vec![MockResponse::ok(b""), MockResponse::ok(xml_body.as_bytes())];
        for expected_name in expected_files {
            let (_, payload) = fixture_files
                .iter()
                .find(|(name, _)| name == expected_name)
                .expect("expected file should exist in fixture");
            responses.push(MockResponse::ok(payload));
        }

        let server = MockHttpServer::start(responses).await;
        let output_dir = TempTestDir::new(&format!("filtered-download-{label}"));

        let mut args = vec!["ia-get", "-o", output_dir.0.to_str().unwrap()];
        args.extend_from_slice(filter_args);
        args.push("https://archive.org/details/item");

        let cli = Cli::try_parse_from(args).unwrap();
        let file_filter = FileFilter::compile(&cli.filters).unwrap();
        let target = parse_archive_url(&cli.url).unwrap();

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

        let filtered = select_and_filter_files(all_files, &target, &file_filter)
            .expect("filter should match expected files");
        assert_eq!(filtered.files.len(), expected_files.len(), "case {label}");

        let prepared_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
            .expect("output directory preparation should succeed")
            .expect("non-list mode should return output directory");

        let download_data: Vec<_> = filtered
            .files
            .into_iter()
            .map(|file| {
                let absolute_url = build_file_download_url(&base_url, &file.name);
                let (sanitized_name, _) = sanitize_filename(&file.name);
                (absolute_url.to_string(), sanitized_name, file.md5)
            })
            .collect();

        let total = download_data.len();
        let summary = downloader::download_files_in_dir(
            &client,
            &prepared_dir,
            download_data,
            total,
            None,
            &policy,
            &signal,
        )
        .await
        .expect("filtered batch download should complete");

        assert_eq!(summary.ok, expected_files.len(), "case {label}");
        assert_eq!(summary.skipped, 0, "case {label}");
        assert!(
            summary.failed.is_empty(),
            "case {label}: {:?}",
            summary.failed
        );
        assert!(!summary.interrupted, "case {label}");
        assert!(batch_exit_result(&summary, total).is_ok(), "case {label}");

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            2 + expected_files.len(),
            "case {label}: unexpected HTTP request count: {requests:?}"
        );
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        for (idx, expected_name) in expected_files.iter().enumerate() {
            let expected_prefix = format!("GET /download/item/{expected_name} ");
            assert!(
                requests[idx + 2].request_line.starts_with(&expected_prefix),
                "case {label}: request {} was {:?}, expected prefix {:?}",
                idx + 2,
                requests[idx + 2].request_line,
                expected_prefix
            );
        }

        for expected_name in expected_files {
            let (_, payload) = fixture_files
                .iter()
                .find(|(name, _)| name == expected_name)
                .unwrap();
            let saved = output_dir.join(expected_name);
            assert_eq!(
                std::fs::read(&saved)
                    .unwrap_or_else(|e| panic!("case {label}: expected saved file {saved:?}: {e}")),
                *payload,
                "case {label}: content mismatch for {expected_name}"
            );
        }

        for absent in absent_paths {
            let path = output_dir.join(absent);
            assert!(
                !path.exists(),
                "case {label}: excluded path {path:?} must not exist on disk"
            );
        }
    }

    #[tokio::test]
    async fn filtered_download_positive_pdf_only_requests_and_saves_matching_files() {
        assert_filtered_download_end_to_end(
            "pdf-only",
            &["-f", "*.pdf"],
            &["issue01.pdf", "docs/nested/issue02.pdf"],
            &[
                "comics/issue01.cbz",
                "comics/sub/issue02.cbz",
                "comics/issue01_thumb.cbz",
                "thumbs/preview.cbz",
                "item_archive.torrent",
                "meta/extra.torrent",
                "comics",
                "thumbs",
                "meta",
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn filtered_download_negative_excludes_torrents_and_saves_remaining_files() {
        assert_filtered_download_end_to_end(
            "exclude-torrents",
            &["-f", "!*.torrent"],
            &[
                "issue01.pdf",
                "docs/nested/issue02.pdf",
                "comics/issue01.cbz",
                "comics/sub/issue02.cbz",
                "comics/issue01_thumb.cbz",
                "thumbs/preview.cbz",
            ],
            &["item_archive.torrent", "meta/extra.torrent", "meta"],
        )
        .await;
    }

    #[tokio::test]
    async fn filtered_download_combined_cbz_excluding_thumbs_saves_only_non_thumb_cbz() {
        assert_filtered_download_end_to_end(
            "cbz-no-thumbs",
            &["-f", "*.cbz", "-f", "!*thumb*"],
            &["comics/issue01.cbz", "comics/sub/issue02.cbz"],
            &[
                "issue01.pdf",
                "docs/nested/issue02.pdf",
                "comics/issue01_thumb.cbz",
                "thumbs/preview.cbz",
                "item_archive.torrent",
                "meta/extra.torrent",
                "docs",
                "thumbs",
                "meta",
            ],
        )
        .await;
    }

    #[test]
    fn cli_check_flag_parses_conflicts_with_list_and_maps_exit_results() {
        use clap::error::ErrorKind;

        let url = "https://archive.org/details/deftributetozzap64";

        let parsed = Cli::try_parse_from(["ia-get", "--check", "-o", "./downloads", url]).unwrap();
        assert!(parsed.check);
        assert!(!parsed.list);
        assert_eq!(parsed.output_dir, Some(PathBuf::from("./downloads")));

        let conflict = Cli::try_parse_from(["ia-get", "--check", "--list", url])
            .err()
            .expect("--check and --list must conflict");
        assert_eq!(conflict.kind(), ErrorKind::ArgumentConflict);

        let ok_summary = verify::VerificationSummary {
            ok: 2,
            failed: Vec::new(),
            interrupted: false,
        };
        assert!(verify_exit_result(&ok_summary, 2).is_ok());

        let failed_summary = verify::VerificationSummary {
            ok: 1,
            failed: vec![verify::FailedVerification {
                file_path: "missing.pdf".to_string(),
                issues: vec![verify::VerificationIssue::Missing],
            }],
            interrupted: false,
        };
        let err = verify_exit_result(&failed_summary, 2).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::VerificationFailed {
                failed: 1,
                total: 2
            }
        ));
        assert_eq!(err.to_string(), "1 of 2 file(s) failed verification");

        let interrupted_summary = verify::VerificationSummary {
            ok: 0,
            failed: failed_summary.failed,
            interrupted: true,
        };
        assert!(matches!(
            verify_exit_result(&interrupted_summary, 2),
            Err(IaGetError::Interrupted)
        ));
    }

    #[test]
    fn finalize_metadata_phase_check_mode_never_creates_missing_directory() {
        let sandbox = TempTestDir::new("check-finalize");
        let missing_dir = sandbox.join("missing/nested");
        let files = XmlFiles {
            files: vec![xml_file("issue01.pdf", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let cli_missing = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            missing_dir.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let err = finalize_metadata_phase(&cli_missing, &files, &spinner).unwrap_err();
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(!missing_dir.exists());
        assert!(!sandbox.join("missing").exists());

        let cli_existing = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            sandbox.0.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let dir = finalize_metadata_phase(&cli_existing, &files, &spinner)
            .unwrap()
            .expect("--check should return canonical target directory");
        assert_eq!(dir, std::fs::canonicalize(&sandbox.0).unwrap());
    }

    #[test]
    fn cli_check_flag_combinations_conflicts_unknown_flags_and_help() {
        use clap::error::ErrorKind;
        use clap::CommandFactory;

        let details_url = "https://archive.org/details/deftributetozzap64";
        let download_url = "https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf";

        let combined_leading = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            "./downloads",
            "-f",
            "*.pdf",
            "-f",
            "!*thumb*",
            "-b",
            "cookies.txt",
            details_url,
        ])
        .unwrap();
        assert!(combined_leading.check);
        assert!(!combined_leading.list);
        assert_eq!(
            combined_leading.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(combined_leading.filters, vec!["*.pdf", "!*thumb*"]);
        assert_eq!(combined_leading.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(combined_leading.url, details_url);

        let combined_trailing = Cli::try_parse_from([
            "ia-get",
            download_url,
            "--check",
            "--output-dir",
            "./out",
            "--filter",
            "*.pdf",
            "--cookies",
            "auth=1",
        ])
        .unwrap();
        assert!(combined_trailing.check);
        assert_eq!(combined_trailing.output_dir, Some(PathBuf::from("./out")));
        assert_eq!(combined_trailing.filters, vec!["*.pdf"]);
        assert_eq!(combined_trailing.cookies.as_deref(), Some("auth=1"));
        assert_eq!(combined_trailing.url, download_url);

        for conflict_args in [
            ["ia-get", "-l", "--check", details_url],
            ["ia-get", "--list", "--check", details_url],
        ] {
            let err = Cli::try_parse_from(conflict_args)
                .err()
                .expect("--list/-l and --check must conflict");
            assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
        }

        for unknown_flag in ["--md5", "--strict"] {
            let err = Cli::try_parse_from(["ia-get", unknown_flag, details_url])
                .err()
                .unwrap_or_else(|| panic!("{unknown_flag} must be rejected"));
            assert_eq!(err.kind(), ErrorKind::UnknownArgument);
        }

        Cli::command().debug_assert();
        let help = Cli::command().render_help().to_string();
        assert!(
            help.contains("--check"),
            "help should mention --check:\n{help}"
        );
        assert!(
            help.contains("ia-get --check https://archive.org/details/deftributetozzap64"),
            "help should include --check example:\n{help}"
        );
        assert!(
            help.contains(
                "ia-get --check -o ./downloads https://archive.org/details/deftributetozzap64"
            ),
            "help should include --check -o example:\n{help}"
        );
    }

    #[test]
    fn finalize_metadata_phase_check_mode_rejects_regular_file_and_handles_empty_summary() {
        let sandbox = TempTestDir::new("check-finalize-file");
        let file_path = sandbox.join("not_a_dir.txt");
        std::fs::write(&file_path, b"keep-me").unwrap();

        let files = XmlFiles {
            files: vec![xml_file("issue01.pdf", Some(1024))],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let cli_file = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            file_path.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let err = finalize_metadata_phase(&cli_file, &files, &spinner).unwrap_err();
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert_eq!(std::fs::read(&file_path).unwrap(), b"keep-me");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let missing_target = sandbox.join("broken-target");
            let broken_link = sandbox.join("broken-dir-link");
            symlink(&missing_target, &broken_link).unwrap();

            let cli_broken = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                broken_link.to_str().unwrap(),
                "https://archive.org/details/item",
            ])
            .unwrap();
            let err = finalize_metadata_phase(&cli_broken, &files, &spinner).unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(err.to_string().contains("broken symlink"));
            assert!(!missing_target.exists());
        }

        let empty_summary = verify::VerificationSummary::default();
        assert!(verify_exit_result(&empty_summary, 0).is_ok());
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CheckDirEntrySnapshot {
        rel_path: PathBuf,
        is_dir: bool,
        is_symlink: bool,
        contents: Option<Vec<u8>>,
        modified: Option<SystemTime>,
    }

    fn snapshot_check_dir(root: &Path) -> (Option<SystemTime>, Vec<CheckDirEntrySnapshot>) {
        fn walk(base: &Path, current: &Path, out: &mut Vec<CheckDirEntrySnapshot>) {
            let mut entries: Vec<_> = std::fs::read_dir(current)
                .expect("read dir for snapshot")
                .map(|entry| entry.expect("dir entry").path())
                .collect();
            entries.sort();

            for path in entries {
                let meta = std::fs::symlink_metadata(&path).expect("symlink_metadata for snapshot");
                let rel_path = path.strip_prefix(base).unwrap().to_path_buf();
                let is_dir = meta.file_type().is_dir();
                let is_symlink = meta.file_type().is_symlink();
                let contents = if meta.file_type().is_file() {
                    Some(std::fs::read(&path).expect("read file for snapshot"))
                } else {
                    None
                };
                let modified = meta.modified().ok();
                out.push(CheckDirEntrySnapshot {
                    rel_path,
                    is_dir,
                    is_symlink,
                    contents,
                    modified,
                });
                if is_dir && !is_symlink {
                    walk(base, &path, out);
                }
            }
        }

        let root_mtime = std::fs::symlink_metadata(root)
            .expect("root metadata for snapshot")
            .modified()
            .ok();
        let mut out = Vec::new();
        walk(root, root, &mut out);
        (root_mtime, out)
    }

    fn build_xml_metadata(entries: &[(&str, usize, String)]) -> String {
        let mut xml = String::from("<files>\n");
        for (name, size, digest) in entries {
            xml.push_str(&format!(
                "  <file name=\"{name}\" source=\"original\">\n    <size>{size}</size>\n    <md5>{digest}</md5>\n  </file>\n"
            ));
        }
        xml.push_str("</files>");
        xml
    }

    #[tokio::test]
    async fn check_mode_end_to_end_clean_with_output_dir_and_filter_fetches_only_xml_and_makes_zero_writes(
    ) {
        let pdf_one = b"pdf-one-verified-payload";
        let pdf_two = b"pdf-two-nested-verified-payload";
        let pdf_sanitized = b"pdf-sanitized-filename-payload";
        let excluded_torrent = b"torrent-excluded-payload";
        let excluded_cbz = b"cbz-excluded-payload";

        let xml_body = build_xml_metadata(&[
            (
                "issue01.pdf",
                pdf_one.len(),
                format!("{:x}", md5::compute(pdf_one)),
            ),
            (
                "docs/nested/issue02.pdf",
                pdf_two.len(),
                format!("{:x}", md5::compute(pdf_two)),
            ),
            (
                "Season 1/Episode?.pdf",
                pdf_sanitized.len(),
                format!("{:X}", md5::compute(pdf_sanitized)),
            ),
            (
                "item_archive.torrent",
                excluded_torrent.len(),
                format!("{:x}", md5::compute(excluded_torrent)),
            ),
            (
                "comics/corrupt.cbz",
                excluded_cbz.len(),
                format!("{:x}", md5::compute(excluded_cbz)),
            ),
        ]);

        // Provide only HEAD + GET responses for `_files.xml`; any file download would fail.
        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;

        let output_dir = TempTestDir::new("check-e2e-clean");
        std::fs::write(output_dir.join("issue01.pdf"), pdf_one).unwrap();
        std::fs::create_dir_all(output_dir.join("docs/nested")).unwrap();
        std::fs::write(output_dir.join("docs/nested/issue02.pdf"), pdf_two).unwrap();
        std::fs::create_dir_all(output_dir.join("Season 1")).unwrap();
        std::fs::write(output_dir.join("Season 1/Episode_.pdf"), pdf_sanitized).unwrap();
        // Deliberately leave `item_archive.torrent` absent and `comics/corrupt.cbz` corrupt
        // to prove `-f '*.pdf'` skips excluded files during `--check`.
        std::fs::create_dir_all(output_dir.join("comics")).unwrap();
        std::fs::write(output_dir.join("comics/corrupt.cbz"), b"wrong").unwrap();

        let before = snapshot_check_dir(&output_dir.0);

        let cli = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            output_dir.0.to_str().unwrap(),
            "-f",
            "*.pdf",
            "https://archive.org/details/item",
        ])
        .unwrap();
        let file_filter = FileFilter::compile(&cli.filters).unwrap();
        let target = parse_archive_url(&cli.url).unwrap();
        verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref())).unwrap();

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
        let (all_files, _base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .expect("XML metadata fetch should succeed");

        let filtered = select_and_filter_files(all_files, &target, &file_filter)
            .expect("filter should match PDF files");
        assert_eq!(filtered.files.len(), 3);

        let verified_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
            .expect("check directory validation should succeed")
            .expect("check mode should return canonical directory");

        let summary = verify::verify_files(&verified_dir, &filtered.files, &signal)
            .expect("verification run should complete");
        assert_eq!(summary.ok, 3);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(verify_exit_result(&summary, filtered.files.len()).is_ok());

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            2,
            "--check must fetch only XML metadata (HEAD + GET): {requests:?}"
        );
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));

        let after = snapshot_check_dir(&output_dir.0);
        assert_eq!(before, after, "--check must make zero writes to disk");
    }

    #[tokio::test]
    async fn check_mode_end_to_end_failing_with_output_dir_and_filter_reports_issues_with_zero_writes(
    ) {
        let good_bytes = b"good-pdf-payload";
        let missing_bytes = b"missing-nested-pdf";
        let bad_size_expected = b"expected-size-payload";
        let bad_md5_expected = b"expected-md5-payload";
        let leftover_bytes = b"complete-pdf-payload";
        let ignored_torrent = b"ignored-torrent";

        let xml_body = build_xml_metadata(&[
            (
                "good.pdf",
                good_bytes.len(),
                format!("{:x}", md5::compute(good_bytes)),
            ),
            (
                "missing_sub/absent.pdf",
                missing_bytes.len(),
                format!("{:x}", md5::compute(missing_bytes)),
            ),
            (
                "bad_size.pdf",
                bad_size_expected.len(),
                format!("{:x}", md5::compute(bad_size_expected)),
            ),
            (
                "bad_md5.pdf",
                bad_md5_expected.len(),
                format!("{:x}", md5::compute(bad_md5_expected)),
            ),
            (
                "leftover.pdf",
                leftover_bytes.len(),
                format!("{:x}", md5::compute(leftover_bytes)),
            ),
            (
                "ignored.torrent",
                ignored_torrent.len(),
                format!("{:x}", md5::compute(ignored_torrent)),
            ),
        ]);

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;

        let output_dir = TempTestDir::new("check-e2e-failing");
        std::fs::write(output_dir.join("good.pdf"), good_bytes).unwrap();
        std::fs::write(output_dir.join("bad_size.pdf"), b"short").unwrap();
        let corrupt_same_len = b"corrupted-md5-payloa";
        assert_eq!(corrupt_same_len.len(), bad_md5_expected.len());
        std::fs::write(output_dir.join("bad_md5.pdf"), corrupt_same_len).unwrap();
        std::fs::write(output_dir.join("leftover.pdf"), leftover_bytes).unwrap();
        std::fs::write(output_dir.join("leftover.pdf.part"), b"partial").unwrap();

        let before = snapshot_check_dir(&output_dir.0);

        let cli = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            output_dir.0.to_str().unwrap(),
            "-f",
            "*.pdf",
            "https://archive.org/details/item",
        ])
        .unwrap();
        let file_filter = FileFilter::compile(&cli.filters).unwrap();
        let target = parse_archive_url(&cli.url).unwrap();
        verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref())).unwrap();

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
        let (all_files, _base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .expect("XML metadata fetch should succeed");

        let filtered = select_and_filter_files(all_files, &target, &file_filter).unwrap();
        assert_eq!(filtered.files.len(), 5);

        let verified_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
            .unwrap()
            .unwrap();

        let summary = verify::verify_files(&verified_dir, &filtered.files, &signal).unwrap();
        assert_eq!(summary.ok, 1);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 4);

        assert_eq!(summary.failed[0].file_path, "missing_sub/absent.pdf");
        assert_eq!(
            summary.failed[0].issues,
            vec![verify::VerificationIssue::Missing]
        );

        assert_eq!(summary.failed[1].file_path, "bad_size.pdf");
        assert_eq!(
            summary.failed[1].issues,
            vec![verify::VerificationIssue::SizeMismatch {
                expected: bad_size_expected.len() as u64,
                actual: 5,
            }]
        );

        assert_eq!(summary.failed[2].file_path, "bad_md5.pdf");
        assert_eq!(
            summary.failed[2].issues,
            vec![verify::VerificationIssue::Md5Mismatch {
                expected: format!("{:x}", md5::compute(bad_md5_expected)),
                actual: format!("{:x}", md5::compute(corrupt_same_len)),
            }]
        );

        assert_eq!(summary.failed[3].file_path, "leftover.pdf");
        assert_eq!(
            summary.failed[3].issues,
            vec![verify::VerificationIssue::LeftoverPart {
                part_path: "leftover.pdf.part".to_string(),
                size: 7,
            }]
        );

        assert!(
            !summary
                .failed
                .iter()
                .any(|entry| entry.file_path == "ignored.torrent"),
            "excluded files must not be verified or reported"
        );

        let exit_err = verify_exit_result(&summary, filtered.files.len()).unwrap_err();
        assert!(matches!(
            exit_err,
            IaGetError::VerificationFailed {
                failed: 4,
                total: 5
            }
        ));

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            2,
            "failing --check must fetch only XML metadata (HEAD + GET): {requests:?}"
        );
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));

        let after = snapshot_check_dir(&output_dir.0);
        assert_eq!(
            before, after,
            "failing --check must make zero writes to disk"
        );
        assert!(!output_dir.join("missing_sub").exists());
    }

    #[tokio::test]
    async fn check_mode_end_to_end_direct_download_url_verifies_only_target_file() {
        let target_bytes = b"direct-target-pdf-bytes";
        let sibling_a = b"sibling-a-not-on-disk";
        let sibling_b = b"sibling-b-not-on-disk";

        let xml_body = build_xml_metadata(&[
            (
                "docs/target.pdf",
                target_bytes.len(),
                format!("{:x}", md5::compute(target_bytes)),
            ),
            (
                "sibling.pdf",
                sibling_a.len(),
                format!("{:x}", md5::compute(sibling_a)),
            ),
            (
                "nested/other.zip",
                sibling_b.len(),
                format!("{:x}", md5::compute(sibling_b)),
            ),
        ]);

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        // Pass case: target file matches, absent sibling files are ignored.
        {
            let server = MockHttpServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
            ])
            .await;

            let output_dir = TempTestDir::new("check-e2e-direct-pass");
            std::fs::create_dir_all(output_dir.join("docs")).unwrap();
            std::fs::write(output_dir.join("docs/target.pdf"), target_bytes).unwrap();
            let before = snapshot_check_dir(&output_dir.0);

            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                output_dir.0.to_str().unwrap(),
                "https://archive.org/download/item/docs/target.pdf",
            ])
            .unwrap();
            let file_filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref())).unwrap();

            let xml_url = server.url("/download/item/item_files.xml");
            let (all_files, _) =
                fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                    .await
                    .unwrap();
            let selected = select_and_filter_files(all_files, &target, &file_filter).unwrap();
            assert_eq!(selected.files.len(), 1);
            assert_eq!(selected.files[0].name, "docs/target.pdf");

            let verified_dir = finalize_metadata_phase(&cli, &selected, &spinner)
                .unwrap()
                .unwrap();
            let summary = verify::verify_files(&verified_dir, &selected.files, &signal).unwrap();
            assert_eq!(summary.ok, 1);
            assert!(summary.failed.is_empty());
            assert!(verify_exit_result(&summary, selected.files.len()).is_ok());

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2);
            assert!(requests[0]
                .request_line
                .starts_with("HEAD /download/item/item_files.xml "));
            assert!(requests[1]
                .request_line
                .starts_with("GET /download/item/item_files.xml "));
            assert_eq!(before, snapshot_check_dir(&output_dir.0));
        }

        // Failure case: target file is corrupt, absent sibling files are still ignored.
        {
            let server = MockHttpServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
            ])
            .await;

            let output_dir = TempTestDir::new("check-e2e-direct-fail");
            std::fs::create_dir_all(output_dir.join("docs")).unwrap();
            let corrupt_target = b"corrupt-target-pdf-byte";
            assert_eq!(corrupt_target.len(), target_bytes.len());
            std::fs::write(output_dir.join("docs/target.pdf"), corrupt_target).unwrap();
            let before = snapshot_check_dir(&output_dir.0);

            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                output_dir.0.to_str().unwrap(),
                "https://archive.org/download/item/docs/target.pdf",
            ])
            .unwrap();
            let file_filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref())).unwrap();

            let xml_url = server.url("/download/item/item_files.xml");
            let (all_files, _) =
                fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                    .await
                    .unwrap();
            let selected = select_and_filter_files(all_files, &target, &file_filter).unwrap();
            assert_eq!(selected.files.len(), 1);

            let verified_dir = finalize_metadata_phase(&cli, &selected, &spinner)
                .unwrap()
                .unwrap();
            let summary = verify::verify_files(&verified_dir, &selected.files, &signal).unwrap();
            assert_eq!(summary.ok, 0);
            assert_eq!(summary.failed.len(), 1);
            assert_eq!(summary.failed[0].file_path, "docs/target.pdf");
            assert_eq!(
                summary.failed[0].issues,
                vec![verify::VerificationIssue::Md5Mismatch {
                    expected: format!("{:x}", md5::compute(target_bytes)),
                    actual: format!("{:x}", md5::compute(corrupt_target)),
                }]
            );
            assert!(matches!(
                verify_exit_result(&summary, selected.files.len()),
                Err(IaGetError::VerificationFailed {
                    failed: 1,
                    total: 1
                })
            ));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2);
            assert_eq!(before, snapshot_check_dir(&output_dir.0));
        }
    }

    #[test]
    fn plan_downloads_builds_items_and_records_sanitized_filenames() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let mut first = xml_file("docs/readme.txt", Some(128));
        first.md5 = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string());
        let second = xml_file("Season 1/Episode?.mp4", Some(4096));
        let third = xml_file("notes.txt", None);

        let plan = plan_downloads(
            XmlFiles {
                files: vec![first, second, third],
            },
            &base_url,
        )
        .unwrap();

        assert_eq!(plan.items.len(), 3);
        assert_eq!(
            plan.items[0],
            disk_space::PlannedDownload {
                url: "https://archive.org/download/item/docs/readme.txt".to_string(),
                file_path: "docs/readme.txt".to_string(),
                expected_md5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                expected_size: Some(128),
            }
        );
        assert_eq!(
            plan.items[1],
            disk_space::PlannedDownload {
                url: "https://archive.org/download/item/Season%201/Episode%3F.mp4".to_string(),
                file_path: "Season 1/Episode_.mp4".to_string(),
                expected_md5: None,
                expected_size: Some(4096),
            }
        );
        assert_eq!(plan.items[2].expected_size, None);
        assert_eq!(
            plan.sanitized,
            vec![(
                "Season 1/Episode?.mp4".to_string(),
                "Season 1/Episode_.mp4".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn run_planned_downloads_with_space_check_aborts_before_file_requests_when_insufficient()
    {
        let payload = b"archive-file-body-bytes";
        let xml_body = build_xml_metadata(&[(
            "archive.bin",
            payload.len(),
            format!("{:x}", md5::compute(payload)),
        )]);

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;

        let output_dir = TempTestDir::new("space-check-fail");
        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let plan = plan_downloads(files, &base_url).unwrap();
        let err = run_planned_downloads_with_space_check(
            &client,
            &output_dir.0,
            plan,
            None,
            &policy,
            &signal,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(dir, items, 5, sig)
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            IaGetError::InsufficientDiskSpace {
                required,
                available: 5,
                ..
            } if required == payload.len() as u64
        ));
        assert!(!output_dir.join("archive.bin").exists());
        assert!(!output_dir.join("archive.bin.part").exists());

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            2,
            "insufficient disk space must stop before any file download HTTP requests: {requests:?}"
        );
    }

    #[tokio::test]
    async fn run_planned_downloads_with_space_check_downloads_when_space_sufficient() {
        let payload = b"archive-file-body-bytes";
        let xml_body = build_xml_metadata(&[(
            "archive.bin",
            payload.len(),
            format!("{:x}", md5::compute(payload)),
        )]);

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let output_dir = TempTestDir::new("space-check-pass");
        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let plan = plan_downloads(files, &base_url).unwrap();
        let summary = run_planned_downloads_with_space_check(
            &client,
            &output_dir.0,
            plan,
            None,
            &policy,
            &signal,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(
                    dir,
                    items,
                    payload.len() as u64,
                    sig,
                )
            },
        )
        .await
        .unwrap();

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert_eq!(
            std::fs::read(output_dir.join("archive.bin")).unwrap(),
            payload
        );

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/archive.bin "));
    }

    #[test]
    fn plan_downloads_after_select_and_filter_retains_fields_for_details_and_download_urls() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let make_files = || XmlFiles {
            files: vec![
                xml_file_with_md5(
                    "docs/issue01.pdf",
                    Some(2_048),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                ),
                xml_file_with_md5(
                    "Season 1/Episode?.pdf",
                    Some(4_096),
                    Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                ),
                xml_file_with_md5(
                    "docs/draft_issue.pdf",
                    Some(1_024),
                    Some("cccccccccccccccccccccccccccccccc"),
                ),
                xml_file_with_md5("notes.txt", None, None),
            ],
        };

        // /details/ URL with include + exclude filters
        let details_target = parse_archive_url("https://archive.org/details/item").unwrap();
        let filter = FileFilter::compile(["*.pdf", "!*draft*"]).unwrap();
        let selected_details =
            select_and_filter_files(make_files(), &details_target, &filter).unwrap();
        let details_plan = plan_downloads(selected_details, &base_url).unwrap();

        assert_eq!(
            details_plan.items,
            vec![
                disk_space::PlannedDownload {
                    url: "https://archive.org/download/item/docs/issue01.pdf".to_string(),
                    file_path: "docs/issue01.pdf".to_string(),
                    expected_md5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                    expected_size: Some(2_048),
                },
                disk_space::PlannedDownload {
                    url: "https://archive.org/download/item/Season%201/Episode%3F.pdf".to_string(),
                    file_path: "Season 1/Episode_.pdf".to_string(),
                    expected_md5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                    expected_size: Some(4_096),
                },
            ]
        );
        assert_eq!(
            details_plan.sanitized,
            vec![(
                "Season 1/Episode?.pdf".to_string(),
                "Season 1/Episode_.pdf".to_string(),
            )]
        );

        // /download/ URL selects single file and retains metadata + sanitised path + URL
        let download_target =
            parse_archive_url("https://archive.org/download/item/Season%201/Episode%3F.pdf")
                .unwrap();
        let no_filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        let selected_download =
            select_and_filter_files(make_files(), &download_target, &no_filter).unwrap();
        let download_plan = plan_downloads(selected_download, &base_url).unwrap();

        assert_eq!(
            download_plan.items,
            vec![disk_space::PlannedDownload {
                url: "https://archive.org/download/item/Season%201/Episode%3F.pdf".to_string(),
                file_path: "Season 1/Episode_.pdf".to_string(),
                expected_md5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                expected_size: Some(4_096),
            }]
        );
        assert_eq!(
            download_plan.sanitized,
            vec![(
                "Season 1/Episode?.pdf".to_string(),
                "Season 1/Episode_.pdf".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn disk_space_e2e_filtered_plan_fails_before_file_http_requests() {
        let alpha = vec![b'a'; 20];
        let beta = vec![b'b'; 30];
        let excluded_torrent = vec![b't'; 500];

        let xml_body = build_xml_metadata(&[
            (
                "alpha.bin",
                alpha.len(),
                format!("{:x}", md5::compute(&alpha)),
            ),
            (
                "nested/beta?.bin",
                beta.len(),
                format!("{:x}", md5::compute(&beta)),
            ),
            (
                "item_archive.torrent",
                excluded_torrent.len(),
                format!("{:x}", md5::compute(&excluded_torrent)),
            ),
        ]);

        // Include a sentinel 3rd response so any unexpected file request would be captured.
        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(&alpha),
        ])
        .await;

        let output_dir = TempTestDir::new("space-e2e-filtered-fail");
        let before = snapshot_check_dir(&output_dir.0);

        let cli = Cli::try_parse_from([
            "ia-get",
            "-o",
            output_dir.0.to_str().unwrap(),
            "-f",
            "*.bin",
            "https://archive.org/details/item",
        ])
        .unwrap();
        let file_filter = FileFilter::compile(&cli.filters).unwrap();
        let target = parse_archive_url(&cli.url).unwrap();

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (all_files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let filtered = select_and_filter_files(all_files, &target, &file_filter).unwrap();
        let plan = plan_downloads(filtered, &base_url).unwrap();
        let expected_required = (alpha.len() + beta.len()) as u64;
        let available = expected_required - 1;

        let err = run_planned_downloads_with_space_check(
            &client,
            &output_dir.0,
            plan,
            None,
            &policy,
            &signal,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(dir, items, available, sig)
            },
        )
        .await
        .unwrap_err();

        match err {
            IaGetError::InsufficientDiskSpace {
                required,
                available: err_avail,
                path,
            } => {
                assert_eq!(required, expected_required);
                assert_eq!(err_avail, available);
                assert_eq!(path, output_dir.0);
            }
            other => panic!("expected InsufficientDiskSpace, got {other:?}"),
        }

        let requests = server.abort_and_collect().await;
        assert_eq!(
            requests.len(),
            2,
            "server must see only HEAD + GET for XML metadata and 0 file requests: {requests:?}"
        );
        assert!(requests[0]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));

        let after = snapshot_check_dir(&output_dir.0);
        assert_eq!(
            before, after,
            "failed disk-space check must not create files, .part files, or subdirectories"
        );
    }

    #[tokio::test]
    async fn disk_space_e2e_resume_credit_allows_batch_when_remaining_fits() {
        let verified_payload: Vec<u8> = (0u8..40).collect();
        let resumed_payload: Vec<u8> = (100u8..160).collect();
        let prefix_len = 25usize;
        let remaining_bytes = (resumed_payload.len() - prefix_len) as u64;
        let total_archive_bytes = (verified_payload.len() + resumed_payload.len()) as u64;
        assert!(remaining_bytes < total_archive_bytes);

        let xml_body = build_xml_metadata(&[
            (
                "verified.bin",
                verified_payload.len(),
                format!("{:x}", md5::compute(&verified_payload)),
            ),
            (
                "resumed.bin",
                resumed_payload.len(),
                format!("{:x}", md5::compute(&resumed_payload)),
            ),
        ]);

        let range_header = format!(
            "bytes {prefix_len}-{}/{}",
            resumed_payload.len() - 1,
            resumed_payload.len()
        );
        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::with_headers(
                "206 Partial Content",
                vec![
                    ("Content-Range", range_header),
                    ("Content-Length", remaining_bytes.to_string()),
                ],
                &resumed_payload[prefix_len..],
            ),
        ])
        .await;

        let output_dir = TempTestDir::new("space-e2e-resume-credit");
        std::fs::write(output_dir.join("verified.bin"), &verified_payload).unwrap();
        std::fs::write(
            output_dir.join("resumed.bin.part"),
            &resumed_payload[..prefix_len],
        )
        .unwrap();

        let target = parse_archive_url("https://archive.org/details/item").unwrap();
        let filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (all_files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let selected = select_and_filter_files(all_files, &target, &filter).unwrap();
        let plan = plan_downloads(selected, &base_url).unwrap();

        let summary = run_planned_downloads_with_space_check(
            &client,
            &output_dir.0,
            plan,
            None,
            &policy,
            &signal,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(
                    dir,
                    items,
                    remaining_bytes,
                    sig,
                )
            },
        )
        .await
        .expect("batch should succeed when available bytes cover remaining resumed bytes");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 1);
        assert!(summary.failed.is_empty());
        assert!(batch_exit_result(&summary, 2).is_ok());

        assert_eq!(
            std::fs::read(output_dir.join("verified.bin")).unwrap(),
            verified_payload
        );
        assert_eq!(
            std::fs::read(output_dir.join("resumed.bin")).unwrap(),
            resumed_payload
        );
        assert!(!output_dir.join("resumed.bin.part").exists());

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            3,
            "verified file must be skipped and only resumed file fetched: {requests:?}"
        );
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/resumed.bin "));
        assert_eq!(
            requests[2].header("Range"),
            Some(format!("bytes={prefix_len}-").as_str())
        );
    }

    #[tokio::test]
    async fn list_and_check_modes_bypass_disk_space_check_on_u64_max_metadata() {
        let small_bytes = b"verified-small-payload";
        let small_md5 = format!("{:x}", md5::compute(small_bytes));
        let xml_body = format!(
            "<files>\n\
             \x20 <file name=\"small.txt\" source=\"original\">\n\
             \x20   <size>{}</size>\n\
             \x20   <md5>{small_md5}</md5>\n\
             \x20 </file>\n\
             \x20 <file name=\"impossibly_huge.bin\" source=\"original\">\n\
             \x20   <size>{}</size>\n\
             \x20   <md5>0123456789abcdef0123456789abcdef</md5>\n\
             \x20 </file>\n\
             </files>",
            small_bytes.len(),
            u64::MAX
        );

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        // --list succeeds on u64::MAX metadata without disk-space failure or file/dir writes
        {
            let list_xml = format!(
                "<files>\n\
                 \x20 <file name=\"impossibly_huge.bin\" source=\"original\">\n\
                 \x20   <size>{}</size>\n\
                 \x20   <md5>0123456789abcdef0123456789abcdef</md5>\n\
                 \x20 </file>\n\
                 </files>",
                u64::MAX
            );
            let server = MockHttpServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(list_xml.as_bytes()),
            ])
            .await;

            let parent_dir = TempTestDir::new("list-u64-max");
            let missing_output = parent_dir.join("never-created");

            let cli = Cli::try_parse_from([
                "ia-get",
                "--list",
                "-o",
                missing_output.to_str().unwrap(),
                "https://archive.org/details/item",
            ])
            .unwrap();
            let file_filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();

            let xml_url = server.url("/download/item/item_files.xml");
            let (all_files, _base_url) =
                fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                    .await
                    .unwrap();
            let filtered = select_and_filter_files(all_files, &target, &file_filter).unwrap();

            assert_eq!(filtered.files.len(), 1);
            assert_eq!(filtered.files[0].size, Some(u64::MAX));
            assert!(finalize_metadata_phase(&cli, &filtered, &spinner)
                .expect("--list must bypass disk-space check")
                .is_none());
            assert!(!missing_output.exists());

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2);
        }

        // --check bypasses disk-space check on u64::MAX metadata and makes zero writes
        {
            let server = MockHttpServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
            ])
            .await;

            let output_dir = TempTestDir::new("check-u64-max");
            std::fs::write(output_dir.join("small.txt"), small_bytes).unwrap();
            let before = snapshot_check_dir(&output_dir.0);

            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                output_dir.0.to_str().unwrap(),
                "https://archive.org/details/item",
            ])
            .unwrap();
            let file_filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            verify::validate_check_dir(resolve_output_dir(cli.output_dir.as_deref())).unwrap();

            let xml_url = server.url("/download/item/item_files.xml");
            let (all_files, base_url) =
                fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                    .await
                    .unwrap();
            let filtered = select_and_filter_files(all_files, &target, &file_filter).unwrap();

            let verified_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
                .expect("--check must bypass disk-space check")
                .expect("--check returns canonical directory");

            let summary = verify::verify_files(&verified_dir, &filtered.files, &signal)
                .expect("--check verification must complete without InsufficientDiskSpace");
            assert_eq!(summary.ok, 1);
            assert_eq!(summary.failed.len(), 1);
            assert_eq!(summary.failed[0].file_path, "impossibly_huge.bin");
            assert_eq!(
                summary.failed[0].issues,
                vec![verify::VerificationIssue::Missing]
            );
            assert!(matches!(
                verify_exit_result(&summary, filtered.files.len()),
                Err(IaGetError::VerificationFailed {
                    failed: 1,
                    total: 2
                })
            ));

            // Confirm the exact same metadata would fail download mode's disk-space check.
            let plan = plan_downloads(filtered, &base_url).unwrap();
            assert!(matches!(
                disk_space::check_disk_space(&verified_dir, &plan.items),
                Err(IaGetError::InsufficientDiskSpace {
                    required: u64::MAX,
                    ..
                })
            ));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2);
            assert_eq!(before, snapshot_check_dir(&output_dir.0));
        }
    }

    #[test]
    fn cli_parses_limit_rate_variants_positions_and_flag_combinations() {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * 1024 * 1024;
        let url = "https://archive.org/details/deftributetozzap64";

        let omitted = Cli::try_parse_from(["ia-get", url]).unwrap();
        assert_eq!(omitted.limit_rate, None);

        let spaced_k = Cli::try_parse_from(["ia-get", "--limit-rate", "500K", url]).unwrap();
        assert_eq!(spaced_k.limit_rate, Some(500 * KIB));

        let equals_m = Cli::try_parse_from(["ia-get", "--limit-rate=2M", url]).unwrap();
        assert_eq!(equals_m.limit_rate, Some(2 * MIB));

        let gig = Cli::try_parse_from(["ia-get", "--limit-rate", "1G", url]).unwrap();
        assert_eq!(gig.limit_rate, Some(GIB));

        let raw = Cli::try_parse_from(["ia-get", "--limit-rate", "1024", url]).unwrap();
        assert_eq!(raw.limit_rate, Some(1024));

        let trailing = Cli::try_parse_from(["ia-get", url, "--limit-rate", "500K"]).unwrap();
        assert_eq!(trailing.limit_rate, Some(500 * KIB));

        let with_list_combo = Cli::try_parse_from([
            "ia-get",
            "--list",
            "-o",
            "./downloads",
            "-f",
            "*.pdf",
            "-b",
            "cookies.txt",
            "--limit-rate",
            "500K",
            url,
        ])
        .unwrap();
        assert!(with_list_combo.list);
        assert_eq!(
            with_list_combo.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(with_list_combo.filters, vec!["*.pdf"]);
        assert_eq!(with_list_combo.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(with_list_combo.limit_rate, Some(500 * KIB));

        let with_check_combo = Cli::try_parse_from([
            "ia-get",
            url,
            "--check",
            "-o",
            "./downloads",
            "-f",
            "!*.torrent",
            "-b",
            "auth=1",
            "--limit-rate=2M",
        ])
        .unwrap();
        assert!(with_check_combo.check);
        assert_eq!(
            with_check_combo.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(with_check_combo.filters, vec!["!*.torrent"]);
        assert_eq!(with_check_combo.cookies.as_deref(), Some("auth=1"));
        assert_eq!(with_check_combo.limit_rate, Some(2 * MIB));
    }

    #[test]
    fn cli_rejects_invalid_limit_rate_values_with_clear_error() {
        use clap::error::ErrorKind;

        let url = "https://archive.org/details/deftributetozzap64";

        let separate_cases = [
            ("0", "greater than zero"),
            ("0K", "greater than zero"),
            ("abc", "positive integer"),
            ("1.5M", "decimal"),
            ("", "empty"),
            ("18446744073709551616", "exceeds maximum"),
            ("18446744073709551615K", "exceeds maximum"),
        ];

        for (bad_rate, expected_reason) in separate_cases {
            let err = Cli::try_parse_from(["ia-get", "--limit-rate", bad_rate, url])
                .err()
                .unwrap_or_else(|| panic!("expected --limit-rate '{bad_rate}' to fail"));
            assert_eq!(
                err.kind(),
                ErrorKind::ValueValidation,
                "unexpected clap error kind for '{bad_rate}': {err}"
            );
            let rendered = err.to_string();
            assert!(
                rendered.contains(&format!("Invalid rate limit '{bad_rate}':")),
                "error output for '{bad_rate}' missing Invalid rate limit header:\n{rendered}"
            );
            assert!(
                rendered.contains(expected_reason),
                "error output for '{bad_rate}' missing '{expected_reason}':\n{rendered}"
            );
        }

        let negative_err = Cli::try_parse_from(["ia-get", "--limit-rate=-500K", url])
            .err()
            .expect("expected --limit-rate=-500K to fail");
        assert_eq!(negative_err.kind(), ErrorKind::ValueValidation);
        let rendered = negative_err.to_string();
        assert!(
            rendered.contains("Invalid rate limit '-500K':"),
            "error output for negative rate missing header:\n{rendered}"
        );
        assert!(
            rendered.contains("without a sign"),
            "error output for negative rate missing reason:\n{rendered}"
        );
    }

    #[test]
    fn cli_help_documents_limit_rate_flag_units_and_example() {
        use clap::CommandFactory;

        Cli::command().debug_assert();

        let help = Cli::command().render_help().to_string();
        assert!(
            help.contains("--limit-rate"),
            "help should mention --limit-rate:\n{help}"
        );
        assert!(
            help.contains("<RATE>"),
            "help should mention <RATE> value name:\n{help}"
        );
        assert!(
            help.contains("1024-byte"),
            "help should note 1024-byte/binary units:\n{help}"
        );
        assert!(
            help.contains("--limit-rate 500K"),
            "help should include --limit-rate 500K example:\n{help}"
        );
    }

    #[tokio::test]
    async fn run_planned_downloads_with_options_wires_limit_rate_throttling_e2e() {
        let payload: Vec<u8> = (0..(40 * 1024)).map(|i| ((i * 5) % 251) as u8).collect();
        let expected_md5 = format!("{:x}", md5::compute(&payload));
        let xml_body =
            build_xml_metadata(&[("throttled.bin", payload.len(), expected_md5.clone())]);

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(&payload),
        ])
        .await;

        let output_dir = TempTestDir::new("limit-rate-e2e");
        let cli = Cli::try_parse_from([
            "ia-get",
            "--limit-rate",
            "100K",
            "https://archive.org/details/item",
        ])
        .unwrap();
        assert_eq!(cli.limit_rate, Some(100 * 1024));

        let target = parse_archive_url(&cli.url).unwrap();
        let filter = FileFilter::compile(&cli.filters).unwrap();
        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (all_files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let selected = select_and_filter_files(all_files, &target, &filter).unwrap();
        let plan = plan_downloads(selected, &base_url).unwrap();
        let options = downloader::DownloadOptions {
            retry_policy: policy,
            limit_rate: cli.limit_rate,
            verbose: false,
        };

        let started = Instant::now();
        let summary = run_planned_downloads_with_options(
            &client,
            &output_dir.0,
            plan,
            None,
            &options,
            &signal,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(
                    dir,
                    items,
                    payload.len() as u64,
                    sig,
                )
            },
        )
        .await
        .expect("e2e rate-limited download should succeed");
        let elapsed = started.elapsed();

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(batch_exit_result(&summary, 1).is_ok());
        assert!(
            elapsed >= Duration::from_millis(320),
            "e2e 40 KiB download at 100 KiB/s (~400ms expected) completed too quickly: {elapsed:?}"
        );

        let saved = std::fs::read(output_dir.join("throttled.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(format!("{:x}", md5::compute(&saved)), expected_md5);
        assert!(!output_dir.join("throttled.bin.part").exists());

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
            .starts_with("GET /download/item/throttled.bin "));
    }

    #[test]
    fn build_http_client_succeeds_with_empty_and_configured_proxies() {
        build_http_client(&downloader::ProxyConfig::default())
            .expect("client should build without proxies");

        let config = downloader::ProxyConfig::from_lookup(|name| match name {
            "HTTP_PROXY" => Some("http://127.0.0.1:8080".to_string()),
            "HTTPS_PROXY" => Some("https://127.0.0.1:8443".to_string()),
            "ALL_PROXY" => Some("socks5h://127.0.0.1:1080".to_string()),
            "NO_PROXY" => Some("localhost,127.0.0.1".to_string()),
            _ => None,
        })
        .unwrap();

        build_http_client(&config).expect("client should build with configured proxies");
    }

    #[test]
    fn cli_does_not_accept_proxy_flag() {
        let result = Cli::try_parse_from([
            "ia-get",
            "--proxy",
            "http://127.0.0.1:8080",
            "https://archive.org/details/deftributetozzap64",
        ]);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_http_client_routes_fetch_xml_metadata_through_http_proxy() {
        let expected_md5 = "0123456789abcdef0123456789abcdef".to_string();
        let xml_body = build_xml_metadata(&[("proxied.bin", 1234, expected_md5.clone())]);

        let proxy_server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.addr.port());

        let proxy_config = downloader::ProxyConfig::from_lookup(|name| match name {
            "HTTP_PROXY" => Some(proxy_url.clone()),
            _ => None,
        })
        .unwrap();
        let client = build_http_client(&proxy_config).unwrap();

        let xml_url =
            Url::parse("http://archive.example/download/proxyitem/proxyitem_files.xml").unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let (files, base_url) =
            fetch_xml_metadata_from_url(xml_url.clone(), &client, &spinner, None, &policy, &signal)
                .await
                .expect("fetch_xml_metadata_from_url should succeed via HTTP_PROXY");

        assert_eq!(base_url, xml_url);
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "proxied.bin");
        assert_eq!(files.files[0].size, Some(1234));
        assert_eq!(files.files[0].md5.as_deref(), Some(expected_md5.as_str()));

        let requests = proxy_server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].request_line,
            "HEAD http://archive.example/download/proxyitem/proxyitem_files.xml HTTP/1.1"
        );
        assert_eq!(
            requests[1].request_line,
            "GET http://archive.example/download/proxyitem/proxyitem_files.xml HTTP/1.1"
        );
        assert_eq!(requests[0].header("User-Agent"), Some(USER_AGENT));
        assert_eq!(requests[1].header("User-Agent"), Some(USER_AGENT));
    }

    #[derive(Clone, Default)]
    struct CapturedLogs {
        bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        fn new() -> Self {
            Self::default()
        }

        fn as_bytes(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
        }

        fn as_string(&self) -> String {
            String::from_utf8(self.as_bytes()).expect("verbose logs must be valid UTF-8")
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn cli_parses_verbose_short_and_long_flags_and_defaults_false() {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * KIB;
        let url = "https://archive.org/details/deftributetozzap64";

        let default_cli = Cli::try_parse_from(["ia-get", url]).unwrap();
        assert!(!default_cli.verbose);

        let short_cli = Cli::try_parse_from(["ia-get", "-v", url]).unwrap();
        assert!(short_cli.verbose);

        let long_cli = Cli::try_parse_from(["ia-get", "--verbose", url]).unwrap();
        assert!(long_cli.verbose);

        let with_list_combo = Cli::try_parse_from([
            "ia-get",
            "-v",
            "--list",
            "-b",
            "cookies.txt",
            "-o",
            "./downloads",
            "-f",
            "*.pdf",
            "--limit-rate",
            "500K",
            url,
        ])
        .unwrap();
        assert!(with_list_combo.verbose);
        assert!(with_list_combo.list);
        assert_eq!(with_list_combo.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(
            with_list_combo.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(with_list_combo.filters, vec!["*.pdf"]);
        assert_eq!(with_list_combo.limit_rate, Some(500 * KIB));

        let with_check_combo = Cli::try_parse_from([
            "ia-get",
            "--verbose",
            "--check",
            "-b",
            "auth=1",
            "-o",
            "./downloads",
            "-f",
            "!*.torrent",
            "--limit-rate=2M",
            url,
        ])
        .unwrap();
        assert!(with_check_combo.verbose);
        assert!(with_check_combo.check);
        assert_eq!(with_check_combo.cookies.as_deref(), Some("auth=1"));
        assert_eq!(
            with_check_combo.output_dir,
            Some(PathBuf::from("./downloads"))
        );
        assert_eq!(with_check_combo.filters, vec!["!*.torrent"]);
        assert_eq!(with_check_combo.limit_rate, Some(2 * MIB));
    }

    #[test]
    fn cli_help_documents_verbose_flag_and_stderr_redirection_example() {
        use clap::CommandFactory;

        Cli::command().debug_assert();

        let help = Cli::command().render_help().to_string();
        assert!(help.contains("-v"), "help should mention -v:\n{help}");
        assert!(
            help.contains("--verbose"),
            "help should mention --verbose:\n{help}"
        );
        assert!(
            help.contains("stderr"),
            "help should mention stderr:\n{help}"
        );
        assert!(
            help.contains("2> ia-get.log"),
            "help should include 2> ia-get.log redirection example:\n{help}"
        );
    }

    #[tokio::test]
    async fn verbose_pipeline_e2e_logs_head_get_cookies_and_stays_silent_when_disabled() {
        const SENTINEL_NAME: &str = "ia_e2e_sentinel_cookie";
        const SENTINEL_VALUE: &str = "E2E_TOP_SECRET_TOKEN_a1b2c3d4";

        let payload = b"end-to-end verbose pipeline payload";
        let expected_md5 = format!("{:x}", md5::compute(payload));
        let xml_body = build_xml_metadata(&[("asset.bin", payload.len(), expected_md5.clone())]);

        let netscape =
            format!("127.0.0.1\tFALSE\t/\tFALSE\t0\t{SENTINEL_NAME}\t{SENTINEL_VALUE}\n");
        let cookie_source = CookieSource::from_netscape_str(&netscape);
        let expected_cookie_header = format!("{SENTINEL_NAME}={SENTINEL_VALUE}");

        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        // Run 1: verbose logging enabled with captured writer
        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let details_url = server.url("/details/item");
        let xml_url = server.url("/download/item/item_files.xml");
        let asset_url = server.url("/download/item/asset.bin");
        let output_dir = TempTestDir::new("verbose-e2e-enabled");
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone());
        let options = downloader::DownloadOptions {
            retry_policy: policy,
            limit_rate: None,
            verbose: true,
        };

        is_url_accessible_with_logger(
            &details_url,
            &client,
            Some(&cookie_source),
            &policy,
            &signal,
            &logger,
        )
        .await
        .expect("details HEAD check should succeed");

        let (files, base_url) = fetch_xml_metadata_from_url_with_logger(
            xml_url.clone(),
            &client,
            &spinner,
            Some(&cookie_source),
            &policy,
            &signal,
            &logger,
        )
        .await
        .expect("XML metadata fetch should succeed");

        let plan = plan_downloads(files, &base_url).unwrap();
        let summary = run_planned_downloads_with_logger(
            &client,
            &output_dir.0,
            plan,
            Some(&cookie_source),
            &options,
            &signal,
            &logger,
            |_, _, _| Ok(()),
        )
        .await
        .expect("planned download should succeed");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_eq!(
            std::fs::read(output_dir.join("asset.bin")).unwrap(),
            payload
        );

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[0].request_line.starts_with("HEAD /details/item "));
        assert!(requests[1]
            .request_line
            .starts_with("HEAD /download/item/item_files.xml "));
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/item_files.xml "));
        assert!(requests[3]
            .request_line
            .starts_with("GET /download/item/asset.bin "));
        for (idx, req) in requests.iter().enumerate() {
            assert_eq!(
                req.header("Cookie"),
                Some(expected_cookie_header.as_str()),
                "request #{idx} ({}) must genuinely transmit sentinel cookie",
                req.request_line
            );
        }

        let raw_bytes = captured.as_bytes();
        assert!(
            !raw_bytes.contains(&0x1b),
            "e2e verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.as_string();
        assert!(
            !logs.contains(SENTINEL_NAME),
            "e2e verbose logs must never leak cookie name:\n{logs}"
        );
        assert!(
            !logs.contains(SENTINEL_VALUE),
            "e2e verbose logs must never leak cookie value:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: -> HEAD {details_url} (cookies=yes)"
            )),
            "missing details HEAD request log:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: <- 200 OK HEAD {details_url} (cookies=yes)"
            )),
            "missing details HEAD 200 OK response log:\n{logs}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: -> HEAD {xml_url} (cookies=yes)")),
            "missing XML HEAD request log:\n{logs}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: -> GET {xml_url} (cookies=yes)")),
            "missing XML GET request log:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: <- 200 OK GET {xml_url} (cookies=yes)"
            )),
            "missing XML GET 200 OK response log:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] metadata: parsed 1 file(s) from {xml_url}"
            )),
            "missing metadata parse log:\n{logs}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: -> GET {asset_url} (cookies=yes)")),
            "missing asset GET request log:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: <- 200 OK GET {asset_url} (cookies=yes)"
            )),
            "missing asset GET 200 OK response log:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] download: completed 'asset.bin' (1/1)"),
            "missing download completed log:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] summary: ok=1 skipped=0 failed=0 interrupted=false"),
            "missing summary log:\n{logs}"
        );

        // Run 2: verbose logging disabled stays silent
        let silent_server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let silent_details_url = silent_server.url("/details/item");
        let silent_xml_url = silent_server.url("/download/item/item_files.xml");
        let silent_output_dir = TempTestDir::new("verbose-e2e-disabled");
        let disabled_logger = VerboseLogger::disabled();
        assert!(!disabled_logger.is_enabled());
        assert!(!VerboseLogger::from_enabled(false).is_enabled());
        let silent_options = downloader::DownloadOptions {
            retry_policy: policy,
            limit_rate: None,
            verbose: false,
        };

        is_url_accessible_with_logger(
            &silent_details_url,
            &client,
            Some(&cookie_source),
            &policy,
            &signal,
            &disabled_logger,
        )
        .await
        .expect("silent details HEAD check should succeed");

        let (silent_files, silent_base_url) = fetch_xml_metadata_from_url_with_logger(
            silent_xml_url,
            &client,
            &spinner,
            Some(&cookie_source),
            &policy,
            &signal,
            &disabled_logger,
        )
        .await
        .expect("silent XML metadata fetch should succeed");

        let silent_plan = plan_downloads(silent_files, &silent_base_url).unwrap();
        let silent_summary = run_planned_downloads_with_logger(
            &client,
            &silent_output_dir.0,
            silent_plan,
            Some(&cookie_source),
            &silent_options,
            &signal,
            &disabled_logger,
            |_, _, _| Ok(()),
        )
        .await
        .expect("silent planned download should succeed");

        assert_eq!(silent_summary.ok, 1);
        assert_eq!(silent_summary.skipped, 0);
        assert!(silent_summary.failed.is_empty());
        assert!(!silent_summary.interrupted);
        assert_eq!(
            std::fs::read(silent_output_dir.join("asset.bin")).unwrap(),
            payload
        );

        let silent_requests = silent_server.finish().await;
        assert_eq!(silent_requests.len(), 4);
    }

    #[test]
    fn plan_downloads_rejects_sanitized_final_path_collisions() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let cases = [
            ("foo?.txt", "foo*.txt", "foo_.txt"),
            ("CON.txt", "CON .txt", "CON_.txt"),
            ("a/file.txt", "/a/file.txt", "a/file.txt"),
        ];

        for (first, second, expected_local) in cases {
            let err = plan_downloads(
                XmlFiles {
                    files: vec![xml_file(first, Some(10)), xml_file(second, Some(20))],
                },
                &base_url,
            )
            .unwrap_err();

            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first
                        && second_entry == second
                ),
                "case ({first}, {second}): expected PathCollision for '{expected_local}', got {err:?}"
            );
            let msg = err.to_string();
            assert!(msg.contains(first), "error missing '{first}': {msg}");
            assert!(msg.contains(second), "error missing '{second}': {msg}");
            assert!(
                msg.contains(expected_local),
                "error missing '{expected_local}': {msg}"
            );
        }
    }

    #[test]
    fn plan_downloads_rejects_staging_part_path_collisions_in_either_order() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let cases = [
            ("foo.txt", "foo.txt.part", "foo.txt.part"),
            ("foo.txt.part", "foo.txt", "foo.txt.part"),
            ("foo?.txt", "foo*.txt.part", "foo_.txt.part"),
            ("foo*.txt.part", "foo?.txt", "foo_.txt.part"),
        ];

        for (first, second, expected_local) in cases {
            let err = plan_downloads(
                XmlFiles {
                    files: vec![xml_file(first, Some(10)), xml_file(second, Some(20))],
                },
                &base_url,
            )
            .unwrap_err();

            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first
                        && second_entry == second
                ),
                "case ({first}, {second}): expected PathCollision for '{expected_local}', got {err:?}"
            );
        }
    }

    #[test]
    fn plan_downloads_allows_non_colliding_and_case_differing_entries_when_case_sensitive() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let plan = plan_downloads_with_case_sensitivity(
            XmlFiles {
                files: vec![
                    xml_file("foo.txt", Some(10)),
                    xml_file("foo.txt.bak", Some(20)),
                    xml_file("a/foo.txt", Some(30)),
                    xml_file("b/foo.txt", Some(40)),
                    xml_file("README.md", Some(50)),
                    xml_file("readme.md", Some(60)),
                    xml_file("Dir/a.txt", Some(70)),
                    xml_file("dir/A.txt", Some(80)),
                    xml_file("A.TXT.part", Some(90)),
                    xml_file("a.txt.PART", Some(100)),
                    xml_file("a.txt", Some(110)),
                ],
            },
            &base_url,
            downloader::PathCaseSensitivity::CaseSensitive,
        )
        .expect(
            "non-colliding and case-differing entries should succeed on case-sensitive volumes",
        );

        assert_eq!(plan.items.len(), 11);
        assert!(plan.sanitized.is_empty());
    }

    #[test]
    fn plan_downloads_case_insensitive_rejects_case_only_final_and_staging_collisions() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let cases = [
            ("Foo.txt", "foo.txt", "foo.txt"),
            ("Dir/a.txt", "dir/A.txt", "dir/A.txt"),
            ("a.txt", "A.TXT.part", "A.TXT.part"),
            ("a.txt", "a.txt.PART", "a.txt.PART"),
            ("A.TXT.part", "a.txt", "a.txt.part"),
            ("a.txt.PART", "a.txt", "a.txt.part"),
            ("foo?.txt", "FOO*.TXT", "FOO_.TXT"),
            ("Caf\u{00e9}.txt", "CAF\u{00c9}.txt", "CAF\u{00c9}.txt"),
        ];

        for (first, second, expected_local) in cases {
            let err = plan_downloads_with_case_sensitivity(
                XmlFiles {
                    files: vec![xml_file(first, Some(10)), xml_file(second, Some(20))],
                },
                &base_url,
                downloader::PathCaseSensitivity::CaseInsensitive,
            )
            .unwrap_err();

            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ref reason,
                    } if local_path == expected_local
                        && first_entry == first
                        && second_entry == second
                        && reason.contains("case-insensitive")
                ),
                "case ({first}, {second}): expected case-insensitive PathCollision for '{expected_local}', got {err:?}"
            );
        }
    }

    #[test]
    fn plan_downloads_for_dir_matches_host_filesystem_and_preserves_missing_dir() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let sandbox = TempTestDir::new("plan-case-host");
        let missing_dir = sandbox.join("not-created-yet");
        let host_sensitivity = downloader::detect_path_case_sensitivity(&missing_dir);

        let result = plan_downloads_for_dir(
            XmlFiles {
                files: vec![xml_file("Foo.txt", Some(10)), xml_file("foo.txt", Some(20))],
            },
            &base_url,
            &missing_dir,
        );

        match host_sensitivity {
            downloader::PathCaseSensitivity::CaseSensitive => {
                let plan = result.expect("case-sensitive host should allow Foo.txt and foo.txt");
                assert_eq!(plan.items.len(), 2);
            }
            downloader::PathCaseSensitivity::CaseInsensitive => {
                let err = result.unwrap_err();
                assert!(
                    matches!(err, IaGetError::PathCollision { .. }),
                    "case-insensitive host should reject Foo.txt and foo.txt, got {err:?}"
                );
            }
        }
        assert!(
            !missing_dir.exists(),
            "plan_downloads_for_dir must not create missing output directories"
        );
    }

    #[test]
    fn list_mode_remains_unaffected_by_colliding_archive_entries() {
        let cli =
            Cli::try_parse_from(["ia-get", "--list", "https://archive.org/details/item"]).unwrap();
        let files = XmlFiles {
            files: vec![
                xml_file("foo?.txt", Some(10)),
                xml_file("foo*.txt", Some(20)),
                xml_file("foo.txt", Some(30)),
                xml_file("foo.txt.part", Some(40)),
            ],
        };
        let spinner = indicatif::ProgressBar::hidden();

        let result = finalize_metadata_phase(&cli, &files, &spinner)
            .expect("--list should succeed even when entries would collide on download");
        assert!(result.is_none());
        assert!(spinner.is_finished());
    }

    #[tokio::test]
    async fn plan_downloads_collision_e2e_aborts_before_file_http_requests() {
        let cases = [
            ("sanitized-final", "foo?.txt", "foo*.txt", "foo_.txt"),
            (
                "staging-then-final",
                "foo.txt",
                "foo.txt.part",
                "foo.txt.part",
            ),
            (
                "final-then-staging",
                "foo.txt.part",
                "foo.txt",
                "foo.txt.part",
            ),
        ];

        let client = downloader::http_client_builder().build().unwrap();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        for (label, first, second, expected_local) in cases {
            let payload_a = b"entry-a-bytes";
            let payload_b = b"entry-b-bytes";
            let xml_body = build_xml_metadata(&[
                (
                    first,
                    payload_a.len(),
                    format!("{:x}", md5::compute(payload_a)),
                ),
                (
                    second,
                    payload_b.len(),
                    format!("{:x}", md5::compute(payload_b)),
                ),
            ]);

            let server = MockHttpServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
                MockResponse::ok(b"must never be requested"),
            ])
            .await;

            let output_dir = TempTestDir::new(&format!("plan-collision-{label}"));
            let before = snapshot_check_dir(&output_dir.0);
            let spinner = indicatif::ProgressBar::hidden();

            let xml_url = server.url("/download/item/item_files.xml");
            let (files, base_url) =
                fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                    .await
                    .unwrap();

            let err = plan_downloads_for_dir(files, &base_url, &output_dir.0).unwrap_err();
            spinner.finish_with_message(format!("{} {}", "✘".red().bold(), err));

            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first
                        && second_entry == second
                ),
                "case {label}: unexpected error {err:?}"
            );
            assert!(spinner.is_finished());

            let requests = server.abort_and_collect().await;
            assert_eq!(
                requests.len(),
                2,
                "case {label}: collision must abort before any file HTTP requests: {requests:?}"
            );
            assert!(requests[0]
                .request_line
                .starts_with("HEAD /download/item/item_files.xml "));
            assert!(requests[1]
                .request_line
                .starts_with("GET /download/item/item_files.xml "));
            assert_eq!(before, snapshot_check_dir(&output_dir.0));
        }
    }

    #[tokio::test]
    async fn plan_downloads_non_colliding_entries_e2e_succeed() {
        let p1 = b"root-foo-txt";
        let p2 = b"root-foo-txt-bak";
        let p3 = b"dir-a-foo-txt";
        let p4 = b"dir-b-foo-txt";
        let xml_body = build_xml_metadata(&[
            ("foo.txt", p1.len(), format!("{:x}", md5::compute(p1))),
            ("foo.txt.bak", p2.len(), format!("{:x}", md5::compute(p2))),
            ("a/foo.txt", p3.len(), format!("{:x}", md5::compute(p3))),
            ("b/foo.txt", p4.len(), format!("{:x}", md5::compute(p4))),
        ]);

        let server = MockHttpServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(p1),
            MockResponse::ok(p2),
            MockResponse::ok(p3),
            MockResponse::ok(p4),
        ])
        .await;

        let output_dir = TempTestDir::new("plan-non-colliding-e2e");
        let client = downloader::http_client_builder().build().unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let policy = downloader::RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let xml_url = server.url("/download/item/item_files.xml");
        let (files, base_url) =
            fetch_xml_metadata_from_url(xml_url, &client, &spinner, None, &policy, &signal)
                .await
                .unwrap();

        let plan = plan_downloads_for_dir(files, &base_url, &output_dir.0).unwrap();
        let summary = run_planned_downloads_with_space_check(
            &client,
            &output_dir.0,
            plan,
            None,
            &policy,
            &signal,
            |_, _, _| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(summary.ok, 4);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert_eq!(std::fs::read(output_dir.join("foo.txt")).unwrap(), p1);
        assert_eq!(std::fs::read(output_dir.join("foo.txt.bak")).unwrap(), p2);
        assert_eq!(std::fs::read(output_dir.join("a/foo.txt")).unwrap(), p3);
        assert_eq!(std::fs::read(output_dir.join("b/foo.txt")).unwrap(), p4);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 6);
    }
}
