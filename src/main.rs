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
            let request = client
                .head(target_url.clone())
                .timeout(std::time::Duration::from_secs(60));
            downloader::attach_cookie_header(request, cookie_source, target_url)
        },
        |_| {},
    )
    .await?;

    Ok(())
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
            downloader::attach_cookie_header(
                client.get(target_url.clone()),
                cookie_source,
                target_url,
            )
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

fn finish_spinner_ok(spinner: &indicatif::ProgressBar, message: impl std::fmt::Display) {
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!("{} {message}", "✔".green().bold()))
            .expect("Failed to set completion style"),
    );
    spinner.finish();
}

fn finish_spinner_on_err<T>(spinner: &indicatif::ProgressBar, result: Result<T>) -> Result<T> {
    result.inspect_err(|err| {
        spinner.finish_with_message(format!("{} {err}", "✘".red().bold()));
    })
}

/// Lists parsed filenames from XML metadata when --list/-l is used
fn list_files(files: &XmlFiles, spinner: &indicatif::ProgressBar) {
    finish_spinner_ok(
        spinner,
        format_args!("Archive has {}", list_summary(files).bold()),
    );
    for row in list_file_rows(files) {
        println!("{row}");
    }
}

fn run_exit_result(
    interrupted: bool,
    failed: usize,
    total_files: usize,
    make_failed_error: impl FnOnce(usize, usize) -> IaGetError,
) -> Result<()> {
    if interrupted {
        Err(IaGetError::Interrupted)
    } else if failed > 0 {
        Err(make_failed_error(failed, total_files))
    } else {
        Ok(())
    }
}

/// Maps a completed batch download summary to the CLI exit result.
fn batch_exit_result(summary: &downloader::BatchSummary, total_files: usize) -> Result<()> {
    run_exit_result(
        summary.interrupted,
        summary.failed.len(),
        total_files,
        |failed, total| IaGetError::BatchFailed { failed, total },
    )
}

/// Maps a completed verification summary to the CLI exit result.
fn verify_exit_result(summary: &verify::VerificationSummary, total_files: usize) -> Result<()> {
    run_exit_result(
        summary.interrupted,
        summary.failed.len(),
        total_files,
        |failed, total| IaGetError::VerificationFailed { failed, total },
    )
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
/// according to each parent directory's filesystem lookup rules.
///
/// Rejects plans where two entries map to the same sanitised destination path or where one
/// entry's `.part` staging path or planned directory collides with another entry.
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

/// Validates a check directory or prepares the download output directory.
fn prepare_phase_dir(cli: &Cli, spinner: &indicatif::ProgressBar) -> Result<PathBuf> {
    let target_dir = resolve_output_dir(cli.output_dir.as_deref());
    let dir_result = if cli.check {
        verify::validate_check_dir(target_dir)
    } else {
        downloader::prepare_output_dir(target_dir)
    };

    finish_spinner_on_err(spinner, dir_result)
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
    let target = finish_spinner_on_err(&spinner, parse_archive_url(&cli.url))?;

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
    let files = finish_spinner_on_err(
        &spinner,
        select_and_filter_files(files, &target, &file_filter),
    )?;

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

        finish_spinner_ok(
            &spinner,
            format_args!(
                "{} {} {file_label} against archive.org metadata",
                "Checking".bold(),
                files.files.len().to_string().bold(),
            ),
        );

        let summary =
            verify::verify_files_with_logger(&output_dir, &files.files, &shutdown, &logger)?;
        verify_exit_result(&summary, files.files.len())?;
        return Ok(());
    }

    // Build and validate the download plan against each parent directory's filesystem lookup rules
    // before creating missing output directories, checking disk space, or starting transfers.
    let target_dir = resolve_output_dir(cli.output_dir.as_deref());
    let plan = finish_spinner_on_err(
        &spinner,
        plan_downloads_for_dir(files, &base_url, target_dir),
    )?;
    let output_dir = prepare_phase_dir(&cli, &spinner)?;
    let total_files = plan.items.len();
    let file_label = if total_files == 1 { "file" } else { "files" };
    let options = downloader::DownloadOptions {
        retry_policy: downloader::RetryPolicy::default(),
        limit_rate: cli.limit_rate,
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
            finish_spinner_on_err(
                &spinner,
                disk_space::check_disk_space_with_signal(dir, items, signal),
            )?;

            // Successfully finished initialization
            finish_spinner_ok(
                &spinner,
                format_args!(
                    "{} to download {} {file_label} from archive.org {}",
                    "Ready".bold(),
                    total_files.to_string().bold(),
                    "★".yellow()
                ),
            );
            Ok(())
        },
    )
    .await?;

    batch_exit_result(&summary, total_files)?;

    Ok(())
}

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use clap::error::ErrorKind;
    use clap::CommandFactory;

    use super::*;
    use crate::test_support::{
        build_xml_metadata, fast_retry_policy, md5_hex, snapshot_dir, xml_file, CapturedLogs,
        MockResponse, MockServer, TempTestDir,
    };

    fn test_client() -> Client {
        downloader::http_client_builder().build().unwrap()
    }

    fn fast_options(max_retries: u32, limit_rate: Option<u64>) -> downloader::DownloadOptions {
        downloader::DownloadOptions {
            retry_policy: fast_retry_policy(max_retries),
            limit_rate,
        }
    }

    fn xml_from_payloads(entries: &[(&str, &[u8])]) -> String {
        let rows: Vec<(&str, u64, String)> = entries
            .iter()
            .map(|(name, bytes)| (*name, bytes.len() as u64, md5_hex(bytes)))
            .collect();
        let refs: Vec<(&str, u64, &str)> = rows
            .iter()
            .map(|(name, len, digest)| (*name, *len, digest.as_str()))
            .collect();
        build_xml_metadata(&refs)
    }

    async fn fetch_test_xml(
        server: &MockServer,
        client: &Client,
        max_retries: u32,
        signal: &downloader::ShutdownSignal,
    ) -> (XmlFiles, Url) {
        let spinner = indicatif::ProgressBar::hidden();
        let logger = VerboseLogger::disabled();
        fetch_xml_metadata_from_url_with_logger(
            server.parsed_url("/download/item/item_files.xml"),
            client,
            &spinner,
            None,
            &fast_retry_policy(max_retries),
            signal,
            &logger,
        )
        .await
        .unwrap()
    }

    #[test]
    fn cli_help_documents_all_flags_urls_and_examples() {
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
        for needle in [
            "/details/<item>",
            "/download/<item>/<path>",
            "-l, --list",
            "--check",
            "-b, --cookies <COOKIES>",
            "-o, --output-dir <DIR>",
            "-f, --filter <GLOB>",
            "--limit-rate <RATE>",
            "1024-byte",
            "-v, --verbose",
            "stderr",
            "Examples:",
            "ia-get https://archive.org/details/deftributetozzap64",
            "ia-get https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf",
            "ia-get -o ./downloads https://archive.org/details/deftributetozzap64",
            "ia-get --output-dir /tmp/zzap64 https://archive.org/details/deftributetozzap64",
            "ia-get --list https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf",
            "ia-get --check https://archive.org/details/deftributetozzap64",
            "ia-get --check -o ./downloads https://archive.org/details/deftributetozzap64",
            "ia-get -f '*.pdf' https://archive.org/details/deftributetozzap64",
            "ia-get -f '!*.torrent' https://archive.org/details/deftributetozzap64",
            "ia-get --limit-rate 500K https://archive.org/details/deftributetozzap64",
            "ia-get -v https://archive.org/details/deftributetozzap64 2> ia-get.log",
        ] {
            assert!(help.contains(needle), "help missing {needle:?}:\n{help}");
        }
    }

    #[test]
    fn cli_parses_flag_variants_combinations_and_windows_paths() {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * KIB;
        const GIB: u64 = 1024 * MIB;
        let details = "https://archive.org/details/deftributetozzap64";
        let download = "https://archive.org/download/deftributetozzap64/deftributetozzap64.pdf";

        let defaults = Cli::try_parse_from(["ia-get", details]).unwrap();
        assert_eq!(defaults.url, details);
        assert!(!defaults.list);
        assert!(!defaults.check);
        assert_eq!(defaults.cookies, None);
        assert_eq!(defaults.output_dir, None);
        assert!(defaults.filters.is_empty());
        assert_eq!(defaults.limit_rate, None);
        assert!(!defaults.verbose);

        let list_leading = Cli::try_parse_from([
            "ia-get",
            "-l",
            "-v",
            "-b",
            "cookies.txt",
            "-o",
            "./downloads",
            "-f",
            "*.{pdf,cbz}",
            "--filter",
            "!*thumb*",
            "--limit-rate",
            "500K",
            details,
        ])
        .unwrap();
        assert!(list_leading.list);
        assert!(!list_leading.check);
        assert!(list_leading.verbose);
        assert_eq!(list_leading.cookies.as_deref(), Some("cookies.txt"));
        assert_eq!(list_leading.output_dir, Some(PathBuf::from("./downloads")));
        assert_eq!(list_leading.filters, vec!["*.{pdf,cbz}", "!*thumb*"]);
        assert_eq!(list_leading.limit_rate, Some(500 * KIB));

        let check_trailing = Cli::try_parse_from([
            "ia-get",
            download,
            "--check",
            "--verbose",
            "--cookies",
            "auth=1",
            "--output-dir",
            "./out",
            "-f",
            "!*.torrent",
            "--limit-rate=2M",
        ])
        .unwrap();
        assert!(check_trailing.check);
        assert!(!check_trailing.list);
        assert!(check_trailing.verbose);
        assert_eq!(check_trailing.url, download);
        assert_eq!(check_trailing.cookies.as_deref(), Some("auth=1"));
        assert_eq!(check_trailing.output_dir, Some(PathBuf::from("./out")));
        assert_eq!(check_trailing.filters, vec!["!*.torrent"]);
        assert_eq!(check_trailing.limit_rate, Some(2 * MIB));

        for (rate_arg, expected) in [("1024", 1024), ("1G", GIB)] {
            let parsed =
                Cli::try_parse_from(["ia-get", "--limit-rate", rate_arg, details]).unwrap();
            assert_eq!(parsed.limit_rate, Some(expected), "rate {rate_arg}");
        }

        for win_path in [r"C:\", r"C:\downloads\archive", r"\\?\C:\downloads"] {
            let parsed =
                Cli::try_parse_from(["ia-get", "--output-dir", win_path, details]).unwrap();
            assert_eq!(parsed.output_dir, Some(PathBuf::from(win_path)));
        }
    }

    #[test]
    fn cli_rejects_conflicts_unknown_flags_and_invalid_limit_rates() {
        let url = "https://archive.org/details/deftributetozzap64";

        for args in [
            vec!["ia-get", "-l", "--check", url],
            vec!["ia-get", "--list", "--check", url],
        ] {
            let err = Cli::try_parse_from(&args).err().expect("expected conflict");
            assert_eq!(err.kind(), ErrorKind::ArgumentConflict, "args: {args:?}");
        }

        for unknown in [
            vec!["ia-get", "--include", "*.pdf", url],
            vec!["ia-get", "--exclude", "*.torrent", url],
            vec!["ia-get", "--md5", url],
            vec!["ia-get", "--strict", url],
            vec!["ia-get", "--proxy", "http://127.0.0.1:8080", url],
        ] {
            let err = Cli::try_parse_from(&unknown)
                .err()
                .expect("expected unknown argument");
            assert_eq!(err.kind(), ErrorKind::UnknownArgument, "args: {unknown:?}");
        }

        let rate_cases: &[(&[&str], &str, &str)] = &[
            (
                &["ia-get", "--limit-rate", "0", url],
                "0",
                "greater than zero",
            ),
            (
                &["ia-get", "--limit-rate", "0K", url],
                "0K",
                "greater than zero",
            ),
            (
                &["ia-get", "--limit-rate", "abc", url],
                "abc",
                "positive integer",
            ),
            (&["ia-get", "--limit-rate", "1.5M", url], "1.5M", "decimal"),
            (&["ia-get", "--limit-rate", "", url], "", "empty"),
            (
                &["ia-get", "--limit-rate", "18446744073709551616", url],
                "18446744073709551616",
                "exceeds maximum",
            ),
            (
                &["ia-get", "--limit-rate", "18446744073709551615K", url],
                "18446744073709551615K",
                "exceeds maximum",
            ),
            (
                &["ia-get", "--limit-rate=-500K", url],
                "-500K",
                "without a sign",
            ),
        ];
        for (args, raw, reason) in rate_cases {
            let err = Cli::try_parse_from(*args)
                .err()
                .unwrap_or_else(|| panic!("expected rate {raw:?} to fail"));
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "rate {raw:?}");
            let msg = err.to_string();
            assert!(
                msg.contains(&format!("Invalid rate limit '{raw}':")),
                "missing header for {raw:?}:\n{msg}"
            );
            assert!(
                msg.contains(reason),
                "missing reason {reason:?} for {raw:?}:\n{msg}"
            );
        }
    }

    #[test]
    fn batch_and_verify_exit_results_map_ok_failed_and_interrupted() {
        for (ok, skipped, total) in [(3, 0, 3), (1, 2, 3), (0, 4, 4), (0, 0, 0)] {
            let summary = downloader::BatchSummary {
                ok,
                skipped,
                failed: Vec::new(),
                interrupted: false,
            };
            assert!(batch_exit_result(&summary, total).is_ok());
        }

        let batch_fail = downloader::BatchSummary {
            ok: 2,
            skipped: 0,
            failed: vec![downloader::FailedFile {
                file_path: "missing.bin".to_string(),
                error: "404".to_string(),
            }],
            interrupted: false,
        };
        assert!(matches!(
            batch_exit_result(&batch_fail, 3),
            Err(IaGetError::BatchFailed {
                failed: 1,
                total: 3
            })
        ));

        for failed in [Vec::new(), batch_fail.failed] {
            let interrupted = downloader::BatchSummary {
                ok: 1,
                skipped: 0,
                failed,
                interrupted: true,
            };
            assert!(matches!(
                batch_exit_result(&interrupted, 3),
                Err(IaGetError::Interrupted)
            ));
        }

        assert!(verify_exit_result(&verify::VerificationSummary::default(), 0).is_ok());
        assert!(verify_exit_result(
            &verify::VerificationSummary {
                ok: 2,
                failed: Vec::new(),
                interrupted: false,
            },
            2,
        )
        .is_ok());

        let failed_entry = verify::FailedVerification {
            file_path: "missing.pdf".to_string(),
            issues: vec![verify::VerificationIssue::Missing],
        };
        let verify_fail = verify::VerificationSummary {
            ok: 1,
            failed: vec![failed_entry],
            interrupted: false,
        };
        let err = verify_exit_result(&verify_fail, 2).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::VerificationFailed {
                failed: 1,
                total: 2
            }
        ));
        assert_eq!(err.to_string(), "1 of 2 file(s) failed verification");

        let verify_interrupted = verify::VerificationSummary {
            ok: 0,
            failed: verify_fail.failed,
            interrupted: true,
        };
        assert!(matches!(
            verify_exit_result(&verify_interrupted, 2),
            Err(IaGetError::Interrupted)
        ));
    }

    #[test]
    fn finalize_metadata_phase_handles_list_check_and_download_dirs() {
        assert_eq!(resolve_output_dir(None), Path::new("."));
        assert_eq!(
            resolve_output_dir(Some(Path::new("/tmp/out"))),
            Path::new("/tmp/out")
        );

        let sandbox = TempTestDir::new("phase-dir");
        let files = XmlFiles {
            files: vec![xml_file("disk1.zip", Some(1024), None)],
        };
        let spinner = indicatif::ProgressBar::hidden();
        let url = "https://archive.org/details/item";

        // --list never creates missing output directories
        let list_missing = sandbox.join("list-missing/nested");
        let list_cli = Cli::try_parse_from([
            "ia-get",
            "--list",
            "-o",
            list_missing.to_str().unwrap(),
            url,
        ])
        .unwrap();
        assert_eq!(
            finalize_metadata_phase(&list_cli, &files, &spinner).unwrap(),
            None
        );
        assert!(!sandbox.join("list-missing").exists());

        // --check never creates missing directories and succeeds on an existing directory
        let check_missing = sandbox.join("check-missing/nested");
        let check_missing_cli = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            check_missing.to_str().unwrap(),
            url,
        ])
        .unwrap();
        assert!(matches!(
            finalize_metadata_phase(&check_missing_cli, &files, &spinner),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(!sandbox.join("check-missing").exists());

        let check_ok_cli = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            sandbox.path().to_str().unwrap(),
            url,
        ])
        .unwrap();
        assert_eq!(
            finalize_metadata_phase(&check_ok_cli, &files, &spinner).unwrap(),
            Some(std::fs::canonicalize(sandbox.path()).unwrap())
        );

        // Regular file is rejected in both download and --check modes without modifying contents
        let regular_file = sandbox.write("not-a-dir.txt", b"keep-me");
        for args in [
            vec!["ia-get", "-o", regular_file.to_str().unwrap(), url],
            vec![
                "ia-get",
                "--check",
                "-o",
                regular_file.to_str().unwrap(),
                url,
            ],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            let err = finalize_metadata_phase(&cli, &files, &spinner).unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)), "args: {args:?}");
            assert_eq!(std::fs::read(&regular_file).unwrap(), b"keep-me");
        }

        // Broken symlink is rejected by --check without creating target
        #[cfg(unix)]
        {
            let missing_target = sandbox.join("broken-target");
            let broken_link = sandbox.join("broken-dir-link");
            std::os::unix::fs::symlink(&missing_target, &broken_link).unwrap();
            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                broken_link.to_str().unwrap(),
                url,
            ])
            .unwrap();
            let err = finalize_metadata_phase(&cli, &files, &spinner).unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(err.to_string().contains("broken symlink"));
            assert!(!missing_target.exists());
        }

        // Download mode creates missing output dir and defaults to canonical cwd
        let created = sandbox.join("created/nested");
        let create_cli =
            Cli::try_parse_from(["ia-get", "-o", created.to_str().unwrap(), url]).unwrap();
        assert_eq!(
            finalize_metadata_phase(&create_cli, &files, &spinner).unwrap(),
            Some(std::fs::canonicalize(&created).unwrap())
        );
        assert!(created.is_dir());

        let default_cli = Cli::try_parse_from(["ia-get", url]).unwrap();
        assert_eq!(
            finalize_metadata_phase(&default_cli, &files, &spinner).unwrap(),
            Some(std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap())
        );
    }

    #[test]
    fn list_mode_formats_rows_summary_with_download_url_filters_and_collisions() {
        let mixed = XmlFiles {
            files: vec![
                xml_file("disk1.zip", Some(1_048_576), None),
                xml_file("disk2.zip", Some(2_097_152), None),
                xml_file("notes.txt", None, None),
            ],
        };
        assert_eq!(
            list_file_rows(&mixed),
            vec![
                "   1.00MB disk1.zip",
                "   2.00MB disk2.zip",
                "  unknown notes.txt",
            ]
        );
        assert_eq!(
            list_summary(&mixed),
            "3 files, 3.00MB total known size, 1 unknown size"
        );

        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.pdf", Some(1024), None),
                xml_file("docs/issue02.pdf", Some(2048), None),
                xml_file("comics/issue01.cbz", Some(4096), None),
                xml_file("comics/issue01_thumb.cbz", Some(512), None),
                xml_file("sub/target file.pdf", Some(10_240), None),
                xml_file("item.torrent", Some(256), None),
                xml_file("notes.txt", None, None),
            ],
        };

        struct ListCase<'a> {
            label: &'a str,
            url: &'a str,
            filters: &'a [&'a str],
            rows: &'a [&'a str],
            summary: &'a str,
        }

        let cases = [
            ListCase {
                label: "download-url",
                url: "https://archive.org/download/item/sub/target%20file.pdf?download=1#page=2",
                filters: &[],
                rows: &["  10.00KB sub/target file.pdf"],
                summary: "1 file, 10.00KB total",
            },
            ListCase {
                label: "positive-pdf",
                url: "https://archive.org/details/item",
                filters: &["-f", "*.pdf"],
                rows: &[
                    "   1.00KB issue01.pdf",
                    "   2.00KB docs/issue02.pdf",
                    "  10.00KB sub/target file.pdf",
                ],
                summary: "3 files, 13.00KB total",
            },
            ListCase {
                label: "negative-torrent",
                url: "https://archive.org/details/item",
                filters: &["-f", "!*.torrent", "-f", "!sub/*"],
                rows: &[
                    "   1.00KB issue01.pdf",
                    "   2.00KB docs/issue02.pdf",
                    "   4.00KB comics/issue01.cbz",
                    "     512B comics/issue01_thumb.cbz",
                    "  unknown notes.txt",
                ],
                summary: "5 files, 7.50KB total known size, 1 unknown size",
            },
            ListCase {
                label: "combined-cbz-no-thumb",
                url: "https://archive.org/details/item",
                filters: &["-f", "*.cbz", "-f", "!*thumb*"],
                rows: &["   4.00KB comics/issue01.cbz"],
                summary: "1 file, 4.00KB total",
            },
        ];

        for case in cases {
            let sandbox = TempTestDir::new(&format!("list-{}", case.label));
            let missing_dir = sandbox.join("never/created");
            let mut args = vec!["ia-get", "--list", "-o", missing_dir.to_str().unwrap()];
            args.extend_from_slice(case.filters);
            args.push(case.url);

            let cli = Cli::try_parse_from(args).unwrap();
            let filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            let selected = select_and_filter_files(make_files(), &target, &filter).unwrap();

            assert_eq!(list_file_rows(&selected), case.rows, "case {}", case.label);
            assert_eq!(list_summary(&selected), case.summary, "case {}", case.label);

            let spinner = indicatif::ProgressBar::hidden();
            assert_eq!(
                finalize_metadata_phase(&cli, &selected, &spinner).unwrap(),
                None,
                "case {}",
                case.label
            );
            assert!(!sandbox.join("never").exists(), "case {}", case.label);
        }

        // Colliding filenames do not block --list
        let colliding = XmlFiles {
            files: vec![
                xml_file("foo?.txt", Some(10), None),
                xml_file("foo*.txt", Some(20), None),
                xml_file("foo.txt", Some(30), None),
                xml_file("foo.txt.part", Some(40), None),
            ],
        };
        let cli =
            Cli::try_parse_from(["ia-get", "--list", "https://archive.org/details/item"]).unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        assert_eq!(
            finalize_metadata_phase(&cli, &colliding, &spinner).unwrap(),
            None
        );
        assert!(spinner.is_finished());
    }

    #[test]
    fn select_target_files_handles_details_download_normalization_and_miss() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file(
                    "issue01.pdf",
                    Some(4096),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                ),
                xml_file(
                    "/sub/dir/file.pdf",
                    Some(2048),
                    Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                ),
                xml_file(
                    "/sub/my%20file%20caf%C3%A9.pdf",
                    Some(9999),
                    Some("cccccccccccccccccccccccccccccccc"),
                ),
            ],
        };

        let details = ArchiveUrl::Details {
            identifier: "item".to_string(),
        };
        assert_eq!(
            select_target_files(make_files(), &details)
                .unwrap()
                .files
                .len(),
            3
        );

        for (file_path, expected_name, expected_md5) in [
            (
                "issue01.pdf",
                "issue01.pdf",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            (
                "sub/dir/file.pdf",
                "/sub/dir/file.pdf",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            (
                "sub/my file café.pdf",
                "/sub/my%20file%20caf%C3%A9.pdf",
                "cccccccccccccccccccccccccccccccc",
            ),
        ] {
            let target = ArchiveUrl::Download {
                identifier: "item".to_string(),
                file_path: file_path.to_string(),
            };
            let selected = select_target_files(make_files(), &target).unwrap();
            assert_eq!(selected.files.len(), 1, "target {file_path}");
            assert_eq!(selected.files[0].name, expected_name);
            assert_eq!(selected.files[0].md5.as_deref(), Some(expected_md5));
        }

        let miss = ArchiveUrl::Download {
            identifier: "my-item".to_string(),
            file_path: "sub/missing.pdf".to_string(),
        };
        let err = select_target_files(make_files(), &miss).unwrap_err();
        assert!(matches!(
            &err,
            IaGetError::FileNotFound { file, item }
                if file == "sub/missing.pdf" && item == "my-item"
        ));
        assert_eq!(
            err.to_string(),
            "File 'sub/missing.pdf' not found in archive item 'my-item'"
        );
    }

    #[test]
    fn select_and_filter_files_composes_target_selection_and_globs() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file(
                    "issue01.pdf",
                    Some(2048),
                    Some("11111111111111111111111111111111"),
                ),
                xml_file("issue01-draft.pdf", Some(512), None),
                xml_file(
                    "/sub/issue02.pdf",
                    Some(4096),
                    Some("22222222222222222222222222222222"),
                ),
                xml_file("item.torrent", Some(256), None),
            ],
        };
        let details = ArchiveUrl::Details {
            identifier: "item".to_string(),
        };

        let inactive = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        assert_eq!(
            select_and_filter_files(make_files(), &details, &inactive)
                .unwrap()
                .files
                .len(),
            4
        );

        let pdf_no_draft = FileFilter::compile(["*.pdf", "!*draft*"]).unwrap();
        let details_filtered =
            select_and_filter_files(make_files(), &details, &pdf_no_draft).unwrap();
        let names: Vec<&str> = details_filtered
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["issue01.pdf", "/sub/issue02.pdf"]);

        let leading_slash_download = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "sub/issue02.pdf".to_string(),
        };
        let sub_pdf = FileFilter::compile(["sub/*.pdf"]).unwrap();
        let matched =
            select_and_filter_files(make_files(), &leading_slash_download, &sub_pdf).unwrap();
        assert_eq!(matched.files.len(), 1);
        assert_eq!(matched.files[0].name, "/sub/issue02.pdf");

        let exclude_sub = FileFilter::compile(["!sub/*.pdf"]).unwrap();
        assert!(matches!(
            select_and_filter_files(make_files(), &leading_slash_download, &exclude_sub),
            Err(IaGetError::NoMatchingFiles { ref patterns }) if patterns == "!sub/*.pdf"
        ));

        let missing = ArchiveUrl::Download {
            identifier: "item".to_string(),
            file_path: "does-not-exist.pdf".to_string(),
        };
        assert!(matches!(
            select_and_filter_files(make_files(), &missing, &pdf_no_draft),
            Err(IaGetError::FileNotFound { ref file, ref item })
                if file == "does-not-exist.pdf" && item == "item"
        ));
    }

    #[test]
    fn build_file_download_url_encodes_segments_and_strips_leading_slash_query_and_fragment() {
        let base_url =
            Url::parse("https://archive.org/download/item/item_files.xml").expect("valid base URL");

        for (file_name, expected) in [
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
        ] {
            let built = build_file_download_url(&base_url, file_name);
            assert_eq!(built.as_str(), expected, "file_name: {file_name:?}");
            assert_eq!(built.query(), None);
            assert_eq!(built.fragment(), None);
        }

        let base_with_extras =
            Url::parse("https://archive.org/download/item/item_files.xml?download=1#section")
                .unwrap();
        let built = build_file_download_url(&base_with_extras, "file.pdf");
        assert_eq!(built.as_str(), "https://archive.org/download/item/file.pdf");
        assert_eq!(built.query(), None);
        assert_eq!(built.fragment(), None);
    }

    #[test]
    fn plan_downloads_for_dir_builds_sanitized_items_and_respects_host_fs() {
        let base_url = Url::parse("https://archive.org/download/item/item_files.xml").unwrap();
        let sandbox = TempTestDir::new("plan-downloads");
        let missing_dir = sandbox.join("not-created-yet");

        let make_files = || XmlFiles {
            files: vec![
                xml_file(
                    "docs/issue01.pdf",
                    Some(2_048),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                ),
                xml_file(
                    "Season 1/Episode?.pdf",
                    Some(4_096),
                    Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                ),
                xml_file("docs/draft.pdf", Some(1_024), None),
                xml_file("notes.txt", None, None),
            ],
        };

        // /details/ URL + include/exclude filter -> planned items retain URL, sanitised path, md5, size
        let details_target = parse_archive_url("https://archive.org/details/item").unwrap();
        let filter = FileFilter::compile(["*.pdf", "!*draft*"]).unwrap();
        let selected = select_and_filter_files(make_files(), &details_target, &filter).unwrap();
        let plan = plan_downloads_for_dir(selected, &base_url, &missing_dir).unwrap();
        assert_eq!(
            plan.items,
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
            plan.sanitized,
            vec![(
                "Season 1/Episode?.pdf".to_string(),
                "Season 1/Episode_.pdf".to_string(),
            )]
        );

        // /download/ URL retains single target metadata
        let download_target =
            parse_archive_url("https://archive.org/download/item/Season%201/Episode%3F.pdf")
                .unwrap();
        let no_filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        let single = select_and_filter_files(make_files(), &download_target, &no_filter).unwrap();
        let single_plan = plan_downloads_for_dir(single, &base_url, &missing_dir).unwrap();
        assert_eq!(single_plan.items.len(), 1);
        assert_eq!(single_plan.items[0].file_path, "Season 1/Episode_.pdf");

        // Host filesystem case-sensitivity check leaves missing output dir untouched and cleans probes
        let probe_dir = TempTestDir::new("plan-downloads-case-oracle");
        probe_dir.write("case_probe.txt", b"x");
        let host_is_case_insensitive = probe_dir.join("CASE_PROBE.TXT").exists();
        let case_result = plan_downloads_for_dir(
            XmlFiles {
                files: vec![
                    xml_file("Foo.txt", Some(10), None),
                    xml_file("foo.txt", Some(20), None),
                ],
            },
            &base_url,
            &missing_dir,
        );
        if host_is_case_insensitive {
            assert!(matches!(case_result, Err(IaGetError::PathCollision { .. })));
        } else {
            assert_eq!(case_result.unwrap().items.len(), 2);
        }
        assert!(!missing_dir.exists());
        assert_eq!(std::fs::read_dir(sandbox.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn fetch_xml_metadata_retries_503_on_get_and_parses_valid_xml() {
        let xml_body =
            build_xml_metadata(&[("issue01.pdf", 4096, "0123456789abcdef0123456789abcdef")]);
        let server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::retry_after("503 Service Unavailable", "0", b"busy"),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let xml_url = server.parsed_url("/download/item/item_files.xml");

        let (files, returned_url) = fetch_xml_metadata_from_url_with_logger(
            xml_url.clone(),
            &client,
            &spinner,
            None,
            &fast_retry_policy(2),
            &signal,
            &logger,
        )
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
    async fn metadata_redirect_wiring_rescopes_cookies_and_preserves_base_url() {
        let client = test_client();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let policy = fast_retry_policy(1);

        // Cross-host 302 on HEAD preserves method and strips non-matching cookies
        let target_server = MockServer::start_on_localhost(vec![MockResponse::ok(b"")]).await;
        let redirect_url = target_server.url("/details/redirected-item");
        let origin_server =
            MockServer::start(vec![MockResponse::redirect("302 Found", redirect_url)]).await;
        let host_cookies =
            CookieSource::from_netscape_str("127.0.0.1\tFALSE\t/\tFALSE\t0\tauth\tsecret\n");

        is_url_accessible_with_logger(
            &origin_server.parsed_url("/details/item"),
            &client,
            Some(&host_cookies),
            &policy,
            &signal,
            &logger,
        )
        .await
        .expect("HEAD across cross-host 302 redirect should succeed");

        let origin_reqs = origin_server.finish().await;
        let target_reqs = target_server.finish().await;
        assert_eq!(origin_reqs.len(), 1);
        assert!(origin_reqs[0]
            .request_line
            .starts_with("HEAD /details/item "));
        assert_eq!(origin_reqs[0].header("cookie"), Some("auth=secret"));
        assert_eq!(target_reqs.len(), 1);
        assert!(target_reqs[0]
            .request_line
            .starts_with("HEAD /details/redirected-item "));
        assert_eq!(target_reqs[0].header("cookie"), None);

        // Path-scoped cookies are re-evaluated across HEAD+GET redirects while preserving base_url
        let xml_body =
            build_xml_metadata(&[("issue01.pdf", 2048, "abcdef0123456789abcdef0123456789")]);
        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/download/public/item_files.xml"),
            MockResponse::ok(b""),
            MockResponse::redirect("302 Found", "/download/public/item_files.xml"),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;

        let sandbox = TempTestDir::new("redirect-cookies");
        let cookie_path = sandbox.write(
            "cookies.txt",
            concat!(
                "# Netscape HTTP Cookie File\n",
                "127.0.0.1\tFALSE\t/\tFALSE\t0\troot\t1\n",
                "127.0.0.1\tFALSE\t/download/private\tFALSE\t0\tprivate\t1\n",
            ),
        );
        let path_cookies = CookieSource::from_input(cookie_path.to_str().unwrap()).unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let xml_url = server.parsed_url("/download/private/item_files.xml");

        let (files, returned_url) = fetch_xml_metadata_from_url_with_logger(
            xml_url.clone(),
            &client,
            &spinner,
            Some(&path_cookies),
            &policy,
            &signal,
            &logger,
        )
        .await
        .expect("XML metadata across redirects should succeed");

        let reqs = server.finish().await;
        assert_eq!(reqs.len(), 4);
        assert!(reqs[0]
            .request_line
            .starts_with("HEAD /download/private/item_files.xml "));
        assert_eq!(reqs[0].header("cookie"), Some("private=1; root=1"));
        assert!(reqs[1]
            .request_line
            .starts_with("HEAD /download/public/item_files.xml "));
        assert_eq!(reqs[1].header("cookie"), Some("root=1"));
        assert!(reqs[2]
            .request_line
            .starts_with("GET /download/private/item_files.xml "));
        assert_eq!(reqs[2].header("cookie"), Some("private=1; root=1"));
        assert!(reqs[3]
            .request_line
            .starts_with("GET /download/public/item_files.xml "));
        assert_eq!(reqs[3].header("cookie"), Some("root=1"));
        assert_eq!(returned_url, xml_url);
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "issue01.pdf");
    }

    #[tokio::test]
    async fn direct_download_end_to_end_verifies_md5_and_strips_query_and_fragment() {
        let target = parse_archive_url(
            "https://archive.org/download/item/sub/file%20name.pdf?download=1#page=2",
        )
        .unwrap();
        assert_eq!(
            target,
            ArchiveUrl::Download {
                identifier: "item".to_string(),
                file_path: "sub/file name.pdf".to_string(),
            }
        );

        let payload = b"direct single-file payload";
        let xml_body = xml_from_payloads(&[
            ("cover.jpg", b"cover"),
            ("sub/file name.pdf", payload),
            ("other/extra.zip", b"extra"),
        ]);
        let server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let client = test_client();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let options = fast_options(1, None);

        let (all_files, base_url) = fetch_test_xml(&server, &client, 1, &signal).await;
        let selected = select_target_files(all_files, &target).unwrap();
        assert_eq!(selected.files.len(), 1);

        let output_dir = TempTestDir::new("direct-download-ok");
        let plan = plan_downloads_for_dir(selected, &base_url, output_dir.path()).unwrap();
        let summary = run_planned_downloads_with_logger(
            &client,
            output_dir.path(),
            plan,
            None,
            &options,
            &signal,
            &logger,
            |_, _, _| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(batch_exit_result(&summary, 1).is_ok());

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/sub/file%20name.pdf "));
        assert!(!requests[2].request_line.contains('?'));
        assert!(!requests[2].request_line.contains('#'));

        assert_eq!(
            std::fs::read(output_dir.join("sub/file name.pdf")).unwrap(),
            payload
        );
        assert!(!output_dir.join("cover.jpg").exists());
        assert!(!output_dir.join("other").exists());
    }

    #[tokio::test]
    async fn filtered_download_e2e_fetches_and_saves_only_matching_files() {
        let cbz_one = b"cbz-one-payload";
        let cbz_two = b"cbz-two-nested-payload";
        let xml_body = xml_from_payloads(&[
            ("issue01.pdf", b"pdf-excluded"),
            ("docs/issue02.pdf", b"pdf-nested-excluded"),
            ("comics/issue01.cbz", cbz_one),
            ("comics/sub/issue02.cbz", cbz_two),
            ("comics/issue01_thumb.cbz", b"thumb-excluded"),
            ("thumbs/preview.cbz", b"thumbs-dir-excluded"),
            ("meta/item.torrent", b"torrent-excluded"),
        ]);

        let server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(cbz_one),
            MockResponse::ok(cbz_two),
        ])
        .await;

        let output_dir = TempTestDir::new("filtered-download-e2e");
        let cli = Cli::try_parse_from([
            "ia-get",
            "-o",
            output_dir.path().to_str().unwrap(),
            "-f",
            "*.cbz",
            "-f",
            "!*thumb*",
            "https://archive.org/details/item",
        ])
        .unwrap();
        let file_filter = FileFilter::compile(&cli.filters).unwrap();
        let target = parse_archive_url(&cli.url).unwrap();

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let options = fast_options(1, None);

        let (all_files, base_url) = fetch_test_xml(&server, &client, 1, &signal).await;
        let filtered = select_and_filter_files(all_files, &target, &file_filter).unwrap();
        assert_eq!(filtered.files.len(), 2);

        let prepared_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
            .unwrap()
            .unwrap();
        let plan = plan_downloads_for_dir(filtered, &base_url, &prepared_dir).unwrap();
        let summary = run_planned_downloads_with_logger(
            &client,
            &prepared_dir,
            plan,
            None,
            &options,
            &signal,
            &logger,
            |_, _, _| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(summary.ok, 2);
        assert!(summary.failed.is_empty());
        assert!(batch_exit_result(&summary, 2).is_ok());

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item/comics/issue01.cbz "));
        assert!(requests[3]
            .request_line
            .starts_with("GET /download/item/comics/sub/issue02.cbz "));

        assert_eq!(
            std::fs::read(output_dir.join("comics/issue01.cbz")).unwrap(),
            cbz_one
        );
        assert_eq!(
            std::fs::read(output_dir.join("comics/sub/issue02.cbz")).unwrap(),
            cbz_two
        );
        for absent in [
            "issue01.pdf",
            "docs",
            "comics/issue01_thumb.cbz",
            "thumbs",
            "meta",
        ] {
            assert!(
                !output_dir.join(absent).exists(),
                "unexpected path {absent}"
            );
        }
    }

    #[tokio::test]
    async fn check_mode_e2e_verifies_filtered_files_and_makes_zero_writes() {
        let pdf_one = b"pdf-one-verified-payload";
        let pdf_two = b"pdf-two-nested-verified-payload";
        let pdf_sanitized = b"pdf-sanitized-filename-payload";
        let xml_body = build_xml_metadata(&[
            ("issue01.pdf", pdf_one.len() as u64, &md5_hex(pdf_one)),
            (
                "docs/nested/issue02.pdf",
                pdf_two.len() as u64,
                &md5_hex(pdf_two),
            ),
            (
                "Season 1/Episode?.pdf",
                pdf_sanitized.len() as u64,
                &md5_hex(pdf_sanitized).to_ascii_uppercase(),
            ),
            ("comics/corrupt.cbz", 16, "0123456789abcdef0123456789abcdef"),
        ]);

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();

        // (label, local_files, expected_ok, expected_failed)
        let clean_files: &[(&str, &[u8])] = &[
            ("issue01.pdf", pdf_one),
            ("docs/nested/issue02.pdf", pdf_two),
            ("Season 1/Episode_.pdf", pdf_sanitized),
            ("comics/corrupt.cbz", b"wrong"),
        ];
        let fail_files: &[(&str, &[u8])] = &[
            ("issue01.pdf", pdf_one),
            ("Season 1/Episode_.pdf", b"corrupted-payload-bytes-wrong!"),
        ];

        for (label, local_files, expected_ok, expected_failed) in [
            ("clean", clean_files, 3usize, 0usize),
            ("fail", fail_files, 1usize, 2usize),
        ] {
            let dir = TempTestDir::new(&format!("check-e2e-{label}"));
            for (rel, data) in local_files {
                dir.write(rel, data);
            }
            let before = snapshot_dir(dir.path());
            let server = MockServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
            ])
            .await;

            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                dir.path().to_str().unwrap(),
                "-f",
                "*.pdf",
                "https://archive.org/details/item",
            ])
            .unwrap();
            let filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();
            let (all_files, _) = fetch_test_xml(&server, &client, 1, &signal).await;
            let filtered = select_and_filter_files(all_files, &target, &filter).unwrap();
            let verified_dir = finalize_metadata_phase(&cli, &filtered, &spinner)
                .unwrap()
                .unwrap();
            let summary =
                verify::verify_files_with_logger(&verified_dir, &filtered.files, &signal, &logger)
                    .unwrap();

            assert_eq!(summary.ok, expected_ok, "case {label}");
            assert_eq!(summary.failed.len(), expected_failed, "case {label}");
            if expected_failed == 0 {
                assert!(verify_exit_result(&summary, 3).is_ok());
            } else {
                assert!(matches!(
                    verify_exit_result(&summary, 3),
                    Err(IaGetError::VerificationFailed {
                        failed: 2,
                        total: 3
                    })
                ));
                assert!(!dir.join("docs").exists());
            }
            assert_eq!(server.finish().await.len(), 2, "case {label}");
            assert_eq!(before, snapshot_dir(dir.path()), "case {label}");
        }
    }

    #[tokio::test]
    async fn check_mode_end_to_end_direct_download_url_verifies_only_target_file() {
        let target_bytes = b"direct-target-pdf-bytes";
        let xml_body = xml_from_payloads(&[
            ("docs/target.pdf", target_bytes),
            ("sibling.pdf", b"sibling-absent"),
        ]);

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();

        for (label, on_disk, expect_ok) in [
            ("pass", target_bytes.as_slice(), true),
            ("fail", b"corrupt-target-pdf-byte".as_slice(), false),
        ] {
            let server = MockServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
            ])
            .await;
            let output_dir = TempTestDir::new(&format!("check-direct-{label}"));
            output_dir.write("docs/target.pdf", on_disk);
            let before = snapshot_dir(output_dir.path());

            let cli = Cli::try_parse_from([
                "ia-get",
                "--check",
                "-o",
                output_dir.path().to_str().unwrap(),
                "https://archive.org/download/item/docs/target.pdf",
            ])
            .unwrap();
            let filter = FileFilter::compile(&cli.filters).unwrap();
            let target = parse_archive_url(&cli.url).unwrap();

            let (all_files, _) = fetch_test_xml(&server, &client, 1, &signal).await;
            let selected = select_and_filter_files(all_files, &target, &filter).unwrap();
            assert_eq!(selected.files.len(), 1);
            let verified_dir = finalize_metadata_phase(&cli, &selected, &spinner)
                .unwrap()
                .unwrap();
            let summary =
                verify::verify_files_with_logger(&verified_dir, &selected.files, &signal, &logger)
                    .unwrap();

            if expect_ok {
                assert_eq!(summary.ok, 1);
                assert!(verify_exit_result(&summary, 1).is_ok());
            } else {
                assert_eq!(summary.ok, 0);
                assert_eq!(summary.failed.len(), 1);
                assert!(matches!(
                    verify_exit_result(&summary, 1),
                    Err(IaGetError::VerificationFailed {
                        failed: 1,
                        total: 1
                    })
                ));
            }
            assert_eq!(server.finish().await.len(), 2);
            assert_eq!(before, snapshot_dir(output_dir.path()));
        }
    }

    #[tokio::test]
    async fn run_planned_downloads_space_gate_aborts_or_downloads() {
        let alpha = vec![b'a'; 20];
        let beta = vec![b'b'; 30];
        let excluded = vec![b't'; 500];
        let xml_body = xml_from_payloads(&[
            ("alpha.bin", &alpha),
            ("nested/beta?.bin", &beta),
            ("item_archive.torrent", &excluded),
        ]);

        let client = test_client();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let options = fast_options(0, None);
        let filter = FileFilter::compile(["*.bin"]).unwrap();
        let target = parse_archive_url("https://archive.org/details/item").unwrap();
        let required_bytes = (alpha.len() + beta.len()) as u64;

        // Insufficient space for filtered plan aborts before any file HTTP requests with zero writes
        let fail_dir = TempTestDir::new("space-gate-fail");
        let before = snapshot_dir(fail_dir.path());
        let fail_server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(&alpha),
        ])
        .await;
        let (all_files, base_url) = fetch_test_xml(&fail_server, &client, 0, &signal).await;
        let filtered = select_and_filter_files(all_files, &target, &filter).unwrap();
        let plan = plan_downloads_for_dir(filtered, &base_url, fail_dir.path()).unwrap();

        let err = run_planned_downloads_with_logger(
            &client,
            fail_dir.path(),
            plan,
            None,
            &options,
            &signal,
            &logger,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(
                    dir,
                    items,
                    required_bytes - 1,
                    sig,
                )
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InsufficientDiskSpace {
                required,
                available,
                ref path,
            } if required == required_bytes && available == required_bytes - 1 && path == fail_dir.path()
        ));
        assert_eq!(fail_server.abort().await.len(), 2);
        assert_eq!(before, snapshot_dir(fail_dir.path()));

        // Sufficient space proceeds to download and verify all planned files
        let pass_dir = TempTestDir::new("space-gate-pass");
        let pass_server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(&alpha),
            MockResponse::ok(&beta),
        ])
        .await;
        let (all_files, base_url) = fetch_test_xml(&pass_server, &client, 0, &signal).await;
        let filtered = select_and_filter_files(all_files, &target, &filter).unwrap();
        let plan = plan_downloads_for_dir(filtered, &base_url, pass_dir.path()).unwrap();
        let summary = run_planned_downloads_with_logger(
            &client,
            pass_dir.path(),
            plan,
            None,
            &options,
            &signal,
            &logger,
            |dir, items, sig| {
                disk_space::check_disk_space_with_available_and_signal(
                    dir,
                    items,
                    required_bytes,
                    sig,
                )
            },
        )
        .await
        .unwrap();

        assert_eq!(summary.ok, 2);
        assert!(summary.failed.is_empty());
        assert_eq!(std::fs::read(pass_dir.join("alpha.bin")).unwrap(), alpha);
        assert_eq!(
            std::fs::read(pass_dir.join("nested/beta_.bin")).unwrap(),
            beta
        );
        assert_eq!(pass_server.finish().await.len(), 4);
    }

    #[tokio::test]
    async fn list_and_check_modes_bypass_disk_space_check_on_u64_max_metadata() {
        let small = b"verified-small-payload";
        let xml_body = build_xml_metadata(&[
            ("small.txt", small.len() as u64, &md5_hex(small)),
            (
                "impossibly_huge.bin",
                u64::MAX,
                "0123456789abcdef0123456789abcdef",
            ),
        ]);

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();

        // --list succeeds without creating output dir or checking disk space
        let list_xml = build_xml_metadata(&[(
            "impossibly_huge.bin",
            u64::MAX,
            "0123456789abcdef0123456789abcdef",
        )]);
        let list_server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(list_xml.as_bytes()),
        ])
        .await;
        let list_parent = TempTestDir::new("list-u64-max");
        let missing_output = list_parent.join("never-created");
        let list_cli = Cli::try_parse_from([
            "ia-get",
            "--list",
            "-o",
            missing_output.to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let (files, _) = fetch_test_xml(&list_server, &client, 0, &signal).await;
        assert!(finalize_metadata_phase(&list_cli, &files, &spinner)
            .unwrap()
            .is_none());
        assert!(!missing_output.exists());
        assert_eq!(list_server.finish().await.len(), 2);

        // --check bypasses disk-space check while download mode rejects the same metadata
        let check_server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;
        let check_dir = TempTestDir::new("check-u64-max");
        check_dir.write("small.txt", small);
        let before = snapshot_dir(check_dir.path());
        let check_cli = Cli::try_parse_from([
            "ia-get",
            "--check",
            "-o",
            check_dir.path().to_str().unwrap(),
            "https://archive.org/details/item",
        ])
        .unwrap();
        let (files, base_url) = fetch_test_xml(&check_server, &client, 0, &signal).await;
        let verified_dir = finalize_metadata_phase(&check_cli, &files, &spinner)
            .unwrap()
            .unwrap();
        let summary =
            verify::verify_files_with_logger(&verified_dir, &files.files, &signal, &logger)
                .unwrap();
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "impossibly_huge.bin");

        let plan = plan_downloads_for_dir(files, &base_url, &verified_dir).unwrap();
        assert!(matches!(
            disk_space::check_disk_space_with_signal(&verified_dir, &plan.items, &signal),
            Err(IaGetError::InsufficientDiskSpace {
                required: u64::MAX,
                ..
            })
        ));
        assert_eq!(check_server.finish().await.len(), 2);
        assert_eq!(before, snapshot_dir(check_dir.path()));
    }

    #[tokio::test]
    async fn run_planned_downloads_wires_limit_rate_throttling_e2e() {
        let payload: Vec<u8> = (0..(40 * 1024)).map(|i| ((i * 5) % 251) as u8).collect();
        let xml_body = xml_from_payloads(&[("throttled.bin", &payload)]);

        let server = MockServer::start(vec![
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

        let client = test_client();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();
        let options = fast_options(0, cli.limit_rate);

        let (all_files, base_url) = fetch_test_xml(&server, &client, 0, &signal).await;
        let plan = plan_downloads_for_dir(all_files, &base_url, output_dir.path()).unwrap();

        let started = Instant::now();
        let summary = run_planned_downloads_with_logger(
            &client,
            output_dir.path(),
            plan,
            None,
            &options,
            &signal,
            &logger,
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
        let elapsed = started.elapsed();

        assert_eq!(summary.ok, 1);
        assert!(summary.failed.is_empty());
        assert!(
            elapsed >= Duration::from_millis(320),
            "40 KiB download at 100 KiB/s completed too quickly: {elapsed:?}"
        );
        assert_eq!(
            std::fs::read(output_dir.join("throttled.bin")).unwrap(),
            payload
        );
        assert!(!output_dir.part_path("throttled.bin").exists());
        assert_eq!(server.finish().await.len(), 3);
    }

    #[test]
    fn build_http_client_succeeds_with_empty_and_configured_proxies() {
        build_http_client(&downloader::ProxyConfig::default())
            .expect("client should build without proxies");

        let config = downloader::ProxyConfig::from_os_lookup(|name| {
            match name {
                "HTTP_PROXY" => Some("http://127.0.0.1:8080"),
                "HTTPS_PROXY" => Some("https://127.0.0.1:8443"),
                "ALL_PROXY" => Some("socks5h://127.0.0.1:1080"),
                "NO_PROXY" => Some("localhost,127.0.0.1"),
                _ => None,
            }
            .map(std::ffi::OsString::from)
        })
        .unwrap();

        build_http_client(&config).expect("client should build with configured proxies");
    }

    #[tokio::test]
    async fn build_http_client_routes_fetch_xml_metadata_through_http_proxy() {
        let expected_md5 = "0123456789abcdef0123456789abcdef";
        let xml_body = build_xml_metadata(&[("proxied.bin", 1234, expected_md5)]);

        let proxy_server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
        ])
        .await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.port());
        let proxy_config = downloader::ProxyConfig::from_os_lookup(|name| match name {
            "HTTP_PROXY" => Some(std::ffi::OsString::from(&proxy_url)),
            _ => None,
        })
        .unwrap();
        let client = build_http_client(&proxy_config).unwrap();

        let xml_url =
            Url::parse("http://archive.example/download/proxyitem/proxyitem_files.xml").unwrap();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let logger = VerboseLogger::disabled();

        let (files, base_url) = fetch_xml_metadata_from_url_with_logger(
            xml_url.clone(),
            &client,
            &spinner,
            None,
            &fast_retry_policy(0),
            &signal,
            &logger,
        )
        .await
        .expect("fetch_xml_metadata_from_url_with_logger should succeed via HTTP_PROXY");

        assert_eq!(base_url, xml_url);
        assert_eq!(files.files.len(), 1);
        assert_eq!(files.files[0].name, "proxied.bin");
        assert_eq!(files.files[0].md5.as_deref(), Some(expected_md5));

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

    #[tokio::test]
    async fn verbose_pipeline_e2e_logs_requests_redacts_cookies_and_strips_ansi() {
        const SENTINEL_NAME: &str = "ia_e2e_sentinel_cookie";
        const SENTINEL_VALUE: &str = "E2E_TOP_SECRET_TOKEN_a1b2c3d4";

        assert!(!VerboseLogger::disabled().is_enabled());
        assert!(!VerboseLogger::from_enabled(false).is_enabled());

        let payload = b"end-to-end verbose pipeline payload";
        let xml_body = xml_from_payloads(&[("asset.bin", payload)]);
        let cookie_source = CookieSource::from_netscape_str(&format!(
            "127.0.0.1\tFALSE\t/\tFALSE\t0\t{SENTINEL_NAME}\t{SENTINEL_VALUE}\n"
        ));
        let expected_cookie_header = format!("{SENTINEL_NAME}={SENTINEL_VALUE}");

        let server = MockServer::start(vec![
            MockResponse::ok(b""),
            MockResponse::ok(b""),
            MockResponse::ok(xml_body.as_bytes()),
            MockResponse::ok(payload),
        ])
        .await;

        let details_url = server.parsed_url("/details/item");
        let xml_url = server.parsed_url("/download/item/item_files.xml");
        let asset_url = server.parsed_url("/download/item/asset.bin");
        let output_dir = TempTestDir::new("verbose-e2e");
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone_writer());

        let client = test_client();
        let spinner = indicatif::ProgressBar::hidden();
        let signal = downloader::ShutdownSignal::new();
        let options = fast_options(0, None);

        is_url_accessible_with_logger(
            &details_url,
            &client,
            Some(&cookie_source),
            &options.retry_policy,
            &signal,
            &logger,
        )
        .await
        .unwrap();
        let (files, base_url) = fetch_xml_metadata_from_url_with_logger(
            xml_url.clone(),
            &client,
            &spinner,
            Some(&cookie_source),
            &options.retry_policy,
            &signal,
            &logger,
        )
        .await
        .unwrap();
        let plan = plan_downloads_for_dir(files, &base_url, output_dir.path()).unwrap();
        let summary = run_planned_downloads_with_logger(
            &client,
            output_dir.path(),
            plan,
            Some(&cookie_source),
            &options,
            &signal,
            &logger,
            |_, _, _| Ok(()),
        )
        .await
        .unwrap();
        assert_eq!(summary.ok, 1);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        for req in &requests {
            assert_eq!(req.header("Cookie"), Some(expected_cookie_header.as_str()));
        }

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "verbose logs must contain zero ANSI escape bytes"
        );
        let logs = captured.text();
        assert!(!logs.contains(SENTINEL_NAME), "cookie name leaked:\n{logs}");
        assert!(
            !logs.contains(SENTINEL_VALUE),
            "cookie value leaked:\n{logs}"
        );

        for expected in [
            format!("[verbose] http: -> HEAD {details_url} (cookies=yes)"),
            format!("[verbose] http: <- 200 OK HEAD {details_url} (cookies=yes)"),
            format!("[verbose] http: -> HEAD {xml_url} (cookies=yes)"),
            format!("[verbose] http: -> GET {xml_url} (cookies=yes)"),
            format!("[verbose] http: <- 200 OK GET {xml_url} (cookies=yes)"),
            format!("[verbose] metadata: parsed 1 file(s) from {xml_url}"),
            format!("[verbose] http: -> GET {asset_url} (cookies=yes)"),
            format!("[verbose] http: <- 200 OK GET {asset_url} (cookies=yes)"),
            "[verbose] download: completed 'asset.bin' (1/1)".to_string(),
            "[verbose] summary: ok=1 skipped=0 failed=0 interrupted=false".to_string(),
        ] {
            assert!(logs.contains(&expected), "missing {expected:?} in:\n{logs}");
        }
    }

    #[tokio::test]
    async fn plan_downloads_collision_e2e_aborts_before_file_http_requests() {
        let client = test_client();
        let signal = downloader::ShutdownSignal::new();

        for (label, first, second, expected_local) in [
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
        ] {
            let xml_body = xml_from_payloads(&[(first, b"entry-a"), (second, b"entry-b")]);
            let server = MockServer::start(vec![
                MockResponse::ok(b""),
                MockResponse::ok(xml_body.as_bytes()),
                MockResponse::ok(b"must never be requested"),
            ])
            .await;

            let output_dir = TempTestDir::new(&format!("plan-collision-{label}"));
            let before = snapshot_dir(output_dir.path());
            let (files, base_url) = fetch_test_xml(&server, &client, 0, &signal).await;

            let err = plan_downloads_for_dir(files, &base_url, output_dir.path()).unwrap_err();
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
            assert_eq!(server.abort().await.len(), 2, "case {label}");
            assert_eq!(before, snapshot_dir(output_dir.path()), "case {label}");
        }
    }
}
