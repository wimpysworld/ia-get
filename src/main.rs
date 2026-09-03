//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//!
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use clap::Parser;
use colored::*;
use ia_get::archive_metadata::{
    ArchiveTarget, XmlFile, XmlFiles, XmlMetadata, fetch_and_parse_xml, get_xml_url,
    list_file_rows, list_summary, parse_archive_url, save_xml_metadata, xml_file_name_of,
};
use ia_get::check::check_directory;
use ia_get::cookie::cookie_source;
use ia_get::display::{
    create_spinner, finish_spinner, format_size, last_glyph, print_check_interrupted,
    print_downloaded_line, print_file_banner,
};
use ia_get::downloader::{self, DownloadTask, parse_rate};
use ia_get::error::io_error_with_path;
use ia_get::file_filter::FileFilter;
use ia_get::fs::available_space;
use ia_get::plan::{
    DownloadPlan, join_output_dir, plan_download_tasks, required_download_space, select_files,
};
use ia_get::verbose;
use ia_get::{IaGetError, Result};
use indicatif::ProgressBar;
use reqwest::{Client, Proxy, Url};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Timeout for establishing a single TCP+TLS connection. Stalled or
/// unreachable hosts must fail fast enough for the retry loop to kick in;
/// slow transfers mid-body are covered by READ_TIMEOUT_SECS instead.
const CONNECTION_TIMEOUT_SECS: u64 = 60;

/// Idle timeout for reading the response body. Applies to each read and resets
/// after a successful one, so a stalled mid-transfer becomes a retryable
/// mid-stream error (resumed later) instead of an infinite hang.
const READ_TIMEOUT_SECS: u64 = 300;

/// Idle timeout for a pooled connection
const POOL_IDLE_TIMEOUT_SECS: u64 = 90;

/// Maximum number of idle connections kept per host
const POOL_MAX_IDLE_PER_HOST: usize = 1;

/// Interval between TCP keepalive probes
const TCP_KEEPALIVE_SECS: u64 = 60;

/// User agent string for HTTP requests: tool name and version, as archive.org
/// asks clients to identify themselves with
const USER_AGENT: &str = concat!("ia-get/", env!("CARGO_PKG_VERSION"));

/// Number assigned to the saved `_files.xml` in the per-file listing; the
/// archive's files are numbered right after it.
const XML_FILE_NUMBER: usize = 1;

/// HTTP client for the download session: a bounded connection timeout,
/// plus a per-read idle timeout that resets after each successful read, so
/// a stalled mid-transfer becomes a retryable error instead of a hang.
///
/// `proxy`, when present, routes every request through it.
fn build_client(proxy: Option<&Proxy>) -> Result<Client> {
    let mut builder = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECTION_TIMEOUT_SECS))
        .read_timeout(Duration::from_secs(READ_TIMEOUT_SECS))
        .pool_idle_timeout(Duration::from_secs(POOL_IDLE_TIMEOUT_SECS))
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(Duration::from_secs(TCP_KEEPALIVE_SECS))
        // Redirects are followed manually (downloader::stream::
        // send_following_redirects) so the Cookie header is re-resolved
        // against each target URL instead of riding along fixed
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy.clone());
    }
    Ok(builder.build()?)
}

/// Resolves the proxy the session should use: an explicit `--proxy` value
/// always wins, otherwise the `HTTPS_PROXY` (or `https_proxy`) environment
/// variable is used. A bare `host:port` is treated as an `http://` proxy.
fn resolve_proxy(cli: Option<&str>) -> Result<Option<Proxy>> {
    let value = match cli {
        Some(explicit) if !explicit.trim().is_empty() => Some(explicit.to_string()),
        _ => std::env::var("HTTPS_PROXY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                std::env::var("https_proxy")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            }),
    };
    let Some(raw) = value else {
        return Ok(None);
    };

    let url = if raw.contains("://") {
        raw.clone()
    } else {
        format!("http://{raw}")
    };
    let proxy = Proxy::all(&url)
        .map_err(|e| IaGetError::InvalidProxy(format!("cannot use proxy {raw:?}: {e}")))?;
    verbose::log(&format!("proxy: {url}"));
    Ok(Some(proxy))
}

/// Parses the `--limit-rate` value into bytes/second, or `None` when the
/// flag was not given (unlimited). `0` is kept as `Some(0)`, which the
/// rate limiter treats as "no limit".
fn parse_rate_limit(input: Option<&str>) -> Result<Option<u64>> {
    let Some(input) = input else {
        return Ok(None);
    };
    let bytes = parse_rate(input)?;
    if bytes > 0 {
        verbose::log(&format!(
            "rate limit: {bytes} bytes/s (~{}/s)",
            format_size(bytes)
        ));
    }
    Ok(Some(bytes))
}

/// Fails the run before downloading anything when the plan cannot fit on the
/// target volume.
///
/// `required_download_space` is a lower bound (files with unknown sizes are
/// excluded, files already present at their expected size are not counted),
/// so this is a "will it clearly not fit" guard, not a hard guarantee. The
/// check is skipped — never fatal — when the free space cannot be read.
fn check_disk_space(output_dir: &str, tasks: &[DownloadTask]) -> Result<()> {
    let required = required_download_space(tasks);
    if required == 0 {
        return Ok(());
    }
    // The output directory exists by the time this runs (ensure_output_dir
    // already created it); "" means the current directory.
    let probe = if output_dir.is_empty() {
        Path::new(".")
    } else {
        Path::new(output_dir)
    };
    let Some(available) = available_space(probe) else {
        return Ok(());
    };
    verbose::log(&format!(
        "disk space in {}: need ~{}, {} free",
        probe.display(),
        format_size(required),
        format_size(available)
    ));
    if available < required {
        return Err(IaGetError::InsufficientDiskSpace {
            required: format_size(required),
            available: format_size(available),
            path: probe.display().to_string(),
        });
    }
    Ok(())
}

/// Finishes `spinner` with a red ✘ and the error's message when it is still
/// running, so every failure path leaves a context-rich line on screen.
///
/// `spinner` may already be finished (a deeper failure can finish it on its
/// own before the error reaches here); finishing it again would just repaint
/// the same line, so the call is skipped in that case.
fn report_spinner_error(spinner: &ProgressBar, error: &IaGetError) {
    if !spinner.is_finished() {
        // finish_spinner (not finish_with_message): the error text may
        // carry user input, and the constant "{msg}" template is the only
        // one that cannot misparse it
        finish_spinner(spinner, &format!("{} {}", "✘".red().bold(), error));
    }
}

/// Runs one initialization step of `run`, reporting its failure on the
/// spinner before the error propagates: on screen the user sees the
/// context-rich error line, and `main` only has to decide the exit code.
fn init_step<T>(spinner: &ProgressBar, step: Result<T>) -> Result<T> {
    match step {
        Ok(value) => Ok(value),
        Err(error) => {
            report_spinner_error(spinner, &error);
            Err(error)
        }
    }
}

/// Persists the freshly fetched `_files.xml` (overwriting any previous copy)
/// and prints its "#1" file block.
///
/// `total_files` is the "#1 of N" denominator, computed by the caller from
/// the planned tasks plus this saved copy.
fn save_and_announce_xml(
    base_url: &Url,
    output_dir: &str,
    content: &str,
    last_modified: Option<SystemTime>,
    total_files: usize,
) -> Result<()> {
    let xml_path = join_output_dir(output_dir, xml_file_name_of(base_url));
    save_xml_metadata(Path::new(&xml_path), content, last_modified)?;

    println!(" ");
    print_file_banner(&xml_path, XML_FILE_NUMBER, total_files);
    // The file never crossed the network, so its line carries no time/rate
    print_downloaded_line(last_glyph(), content.len() as u64, None);

    Ok(())
}

/// Normalizes the `-o` argument into the bare directory the files land in:
/// "" for the current directory, trailing separators trimmed away (only
/// trailing: a leading separator still means "root of the filesystem").
/// A path that is nothing but separators (or empty) names no directory.
fn normalize_output_dir(arg: Option<&str>) -> Result<String> {
    let Some(dir) = arg else {
        return Ok(String::new());
    };
    let trimmed = dir.trim_end_matches(['/', '\\']);
    // On Windows "C:\\" trims to "C:", which is drive-relative (the current
    // directory on that drive), not the requested root: restore the
    // separator for a bare drive root.
    #[cfg(windows)]
    if trimmed.len() == 2 && trimmed.ends_with(':') && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        return Ok(format!("{trimmed}\\"));
    }
    if trimmed.is_empty() {
        return Err(IaGetError::InvalidOutputDir(dir.to_string()));
    }
    Ok(trimmed.to_string())
}

/// Creates the `-o` directory (with its parents) so the metadata save and
/// the downloads can write into it; "" means the current directory, which
/// already exists. The directory is user-chosen, so following a symlink the
/// user pointed at is their intent, not a pre-planted one.
fn ensure_output_dir(output_dir: &str) -> Result<()> {
    if output_dir.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(output_dir).map_err(|e| io_error_with_path(output_dir, e))
}

/// Narrows the archive's entries to this run's candidates and converts
/// them into the download plan — the pipeline a download and a `--check`
/// run share:
///
/// 1. the URL's selection (the whole item minus the self-referencing
///    `_files.xml` entry, or the single file a download URL names);
/// 2. the `--include`/`--exclude` name filters — so excluded files never
///    take part in collision detection either;
/// 3. the plan itself, built before anything is announced, so the counts
///    the summaries print only include the tasks that will actually run
///    (entries skipped for empty names or path collisions never appear
///    in the numbering).
///
/// Returns the plan plus the counts the download's summaries need: how
/// many entries the URL selected and how many of those the filters kept
/// out.
fn build_plan(
    files: Vec<XmlFile>,
    xml_file_name: &str,
    base_url: &Url,
    output_dir: &str,
    target: &ArchiveTarget,
    filter: &FileFilter,
) -> Result<(DownloadPlan, usize, usize)> {
    let selected = select_files(files, xml_file_name, target)?;
    let selected_total = selected.len();
    let candidates = selected
        .into_iter()
        .filter(|file| filter.matches(&file.name))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(IaGetError::NoFilesSelected {
            identifier: target.identifier.clone(),
        });
    }
    let filtered_out = selected_total - candidates.len();
    let plan = plan_download_tasks(candidates, base_url, output_dir, target)?;
    Ok((plan, selected_total, filtered_out))
}

/// The `--check` flow: verify the directory a download would produce,
/// against the same plan a download would build. Read-only — nothing is
/// downloaded or created — and a directory that does not exist is a clear
/// error, unlike a download which creates its output directory.
///
/// Synchronous on purpose: none of its steps await, and an `async fn`
/// without an await point trips clippy's `unused_async`.
fn run_check(
    spinner: &ProgressBar,
    cli: &Cli,
    target: &ArchiveTarget,
    base_url: &Url,
    files: XmlFiles,
) -> Result<()> {
    let output_dir = init_step(spinner, normalize_output_dir(cli.output_dir.as_deref()))?;
    // Unlike a download, check must not create the directory: it is the
    // thing being verified, so a missing one is a clear error.
    let probe = if output_dir.is_empty() {
        Path::new(".")
    } else {
        Path::new(&output_dir)
    };
    if !probe.is_dir() {
        return init_step(
            spinner,
            Err(IaGetError::CheckDirectoryNotFound(
                probe.display().to_string(),
            )),
        );
    }

    let whole_item = target.file_path.is_none();
    let xml_file_name = xml_file_name_of(base_url);
    let filter = FileFilter::new(cli.include.clone(), cli.exclude.clone());
    let (plan, _, _) = init_step(
        spinner,
        build_plan(
            files.files,
            xml_file_name,
            base_url,
            &output_dir,
            target,
            &filter,
        ),
    )?;

    finish_spinner(
        spinner,
        &format!(
            "{} Verifying {} file{} in {}",
            "⚙".blue(),
            plan.tasks.len().to_string().bold(),
            if plan.tasks.len() == 1 { "" } else { "s" },
            probe.display().to_string().bold()
        ),
    );

    let report = match check_directory(&plan, xml_file_name, whole_item, &output_dir, cli.md5) {
        Ok(report) => report,
        // check_directory surfaces a Ctrl+C during the --md5 hash here;
        // the spinner is already finished and main only maps the result
        // to an exit code, so announce it — otherwise a stop mid-check
        // exits silently (the download side gets the same treatment in
        // download_files_with_signal).
        Err(error) => {
            if matches!(error, IaGetError::Interrupted) {
                print_check_interrupted();
            }
            return Err(error);
        }
    };
    report.print();

    if report.is_clean(cli.strict) {
        println!(
            "\n{} {}",
            "✔".green().bold(),
            "Directory matches the archive".green().bold()
        );
        return Ok(());
    }

    println!(
        "\n{} {}",
        "✘".red().bold(),
        format!("{} problem(s) found", report.failing_count(cli.strict))
            .red()
            .bold()
    );
    Err(IaGetError::CheckFailed {
        problems: report.failing_count(cli.strict),
    })
}

/// Lists parsed filenames from XML metadata when --list/-l is used
fn list_files(files: &XmlFiles, spinner: &ProgressBar, xml_file_name: &str) {
    finish_spinner(
        spinner,
        &format!(
            "{} Archive has {}",
            "✔".green().bold(),
            list_summary(files).bold()
        ),
    );
    for row in list_file_rows(files, xml_file_name) {
        println!("{row}");
    }
    // The archive's own _files.xml entry counts in the list above but not
    // in the download plan (it is saved locally as file #1): bridge the
    // two numbers so they do not look inconsistent.
    if files.files.iter().any(|file| file.name == xml_file_name) {
        let remaining = files.files.len() - 1;
        println!(
            "\n{} {} {}",
            "ⓘ".blue().bold(),
            "Note:".blue(),
            format!(
                "{} is the archive's own metadata (saved locally as file #1): a download fetches the remaining {} file{}",
                xml_file_name,
                remaining,
                if remaining == 1 { "" } else { "s" }
            )
            .dimmed()
        );
    }
}

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
struct Cli {
    /// archive.org URL: a details page (whole item) or a download URL naming a single file
    url: String,
    /// List files parsed from archive metadata XML and exit; lists the whole item even for a single-file URL
    #[arg(short = 'l', long = "list")]
    list: bool,
    /// Cookie header or Netscape cookies.txt file for authenticated downloads
    #[arg(short = 'b', long = "cookies", value_name = "COOKIES")]
    cookies: Option<String>,
    /// Directory the files are written into, created if missing (default: the current directory)
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<String>,
    /// Only download files whose name matches this glob; repeatable. '*' matches any run of characters across '/', '?' one character. With no --include, every file is a candidate
    #[arg(long, value_name = "PATTERN")]
    include: Vec<String>,
    /// Skip files whose name matches this glob; repeatable. Patterns match the archive's original names, before sanitization
    #[arg(long, value_name = "PATTERN")]
    exclude: Vec<String>,
    /// Stop at the first failed file instead of continuing with the rest
    #[arg(long)]
    stop_on_error: bool,
    /// Cap the download throughput in bytes/second, e.g. "1M", "512K" or a
    /// plain number; unlimited when omitted (0 disables the cap)
    #[arg(long, value_name = "RATE")]
    limit_rate: Option<String>,
    /// Proxy to route requests through, e.g. "http://127.0.0.1:3128"; falls
    /// back to the HTTPS_PROXY (or https_proxy) environment variable
    #[arg(long, value_name = "URL")]
    proxy: Option<String>,
    /// Enable diagnostic logging to stderr: request URLs, HTTP status codes
    /// and the session settings (proxy, rate limit, disk space)
    #[arg(long)]
    verbose: bool,
    /// Verify the files in the `-o` directory (or the current directory)
    /// against the archive's metadata instead of downloading; nothing is
    /// written
    #[arg(long)]
    check: bool,
    /// In --check mode, also verify each file's MD5 hash (slower; off by
    /// default)
    #[arg(long)]
    md5: bool,
    /// In --check mode, treat date and extra-file mismatches as errors, not
    /// warnings
    #[arg(long)]
    strict: bool,
}

/// Main application entry point
///
/// Parses the command line arguments and hands off to [`run`]. `run` prints
/// every failure it returns before returning it — initialization errors on
/// the spinner, a Ctrl+C during `--check`'s MD5 pass, and the download
/// flow's own interrupted / per-file / summary lines — so the error is
/// already on screen; `main` only maps the result to a process exit code,
/// without letting the runtime print it a second time.
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    verbose::set_enabled(cli.verbose);
    if run(&cli).await.is_err() {
        // The failure was already printed by run's spinner; just exit
        // non-zero so a shell or wrapping script sees it.
        std::process::exit(1);
    }
}

/// Validate the URL, fetch and parse the archive metadata, and download
/// the files.
///
/// A single spinner is shown for the whole initialization; every failure
/// path finishes it with a human-readable message before the error is
/// returned, so the caller only has to decide on the exit code.
async fn run(cli: &Cli) -> Result<()> {
    // Start a single spinner for the entire initialization process
    let spinner = create_spinner(&format!("Processing archive.org URL: {}", cli.url.bold()));

    // Session settings are resolved up front so a bad --proxy or
    // --limit-rate fails before any network work, and build the client
    // against the resolved proxy.
    let proxy = init_step(&spinner, resolve_proxy(cli.proxy.as_deref()))?;
    let rate_limit = init_step(&spinner, parse_rate_limit(cli.limit_rate.as_deref()))?;
    let client = init_step(&spinner, build_client(proxy.as_ref()))?;

    // Parse the URL into its target: the item identifier plus, for a URL
    // with a file path, the single file it names
    let target = init_step(&spinner, parse_archive_url(&cli.url))?;

    // The cookie source is resolved once, and each request then picks the
    // cookies scoped to its own URL, so a path-scoped cookie applies where
    // it belongs and nowhere else.
    let xml_url = init_step(&spinner, get_xml_url(&target.identifier))?;
    let cookie_source = init_step(&spinner, cookie_source(cli.cookies.as_deref(), &xml_url))?;

    // The spinner switches from "processing the URL" to the parsing stage
    // before the metadata is fetched: any failure (the accessibility
    // pre-check, a failed GET, an unparseable document) is reported by
    // init_step with the full error detail.
    spinner.set_message(format!(
        "{} {}",
        "⚙".blue(),
        "Parsing archive metadata...".bold()
    ));

    let XmlMetadata {
        files,
        base_url,
        content,
        last_modified,
    } = init_step(
        &spinner,
        fetch_and_parse_xml(&xml_url, &client, cookie_source.as_ref()).await,
    )?;

    // If requested, list parsed filenames and exit: a read-only preview,
    // nothing is written to the working directory (the whole item, even
    // for a single-file URL)
    if cli.list {
        list_files(&files, &spinner, xml_file_name_of(&base_url));
        return Ok(());
    }

    // In --check mode we verify an existing directory against the metadata
    // instead of writing one: the read-only flow, nothing is downloaded
    // or created.
    if cli.check {
        return run_check(&spinner, cli, &target, &base_url, files);
    }

    // A whole-item run saves the freshly fetched _files.xml as file #1;
    // a single-file run never does and numbers its one file as #1.
    let whole_item = target.file_path.is_none();

    // Where the files land: the current directory by default, or the -o
    // directory (created if missing) when one was given.
    let output_dir = init_step(&spinner, normalize_output_dir(cli.output_dir.as_deref()))?;
    init_step(&spinner, ensure_output_dir(&output_dir))?;

    // Narrow the archive's entries to this run's candidates and build the
    // download plan (see build_plan for the pipeline steps).
    let filter = FileFilter::new(cli.include.clone(), cli.exclude.clone());
    let xml_file_name = xml_file_name_of(&base_url);
    let (plan, selected_total, filtered_out) = init_step(
        &spinner,
        build_plan(
            files.files,
            xml_file_name,
            &base_url,
            &output_dir,
            &target,
            &filter,
        ),
    )?;

    // Fail fast if the plan clearly cannot fit on the target volume, before
    // any bytes cross the network (and before the metadata is saved).
    init_step(&spinner, check_disk_space(&output_dir, &plan.tasks))?;

    let total_files = plan.tasks.len() + usize::from(whole_item);

    // Persist the freshly fetched _files.xml (overwriting any previous copy)
    // with the server's Last-Modified time, and announce it as file #1 —
    // whole-item runs only.
    if whole_item {
        init_step(
            &spinner,
            save_and_announce_xml(&base_url, &output_dir, &content, last_modified, total_files),
        )?;
    }

    // Successfully finished initialization; separate the banner from the
    // saved-metadata block above.
    println!();
    finish_spinner(
        &spinner,
        &format!(
            "{} {} to download {} files from archive.org {}",
            "✔".green().bold(),
            "Ready".bold(),
            plan.tasks.len().to_string().bold(),
            "★".yellow()
        ),
    );

    // The plan's warnings (sanitized, collided and skipped entries)
    // print after the Ready line.
    for warning in &plan.warnings {
        println!("{warning}");
    }

    // Show summary if any files were sanitized
    if plan.sanitized_count > 0 {
        println!(
            "\n{} {} {} file{} for filesystem compatibility",
            "✓".green().bold(),
            "Sanitized".bold(),
            plan.sanitized_count.to_string().bold(),
            if plan.sanitized_count == 1 { "" } else { "s" }
        );
    }

    // A summary of what the name filters kept out, like the sanitized one
    if !filter.is_empty() && filtered_out > 0 {
        println!(
            "\n{} {} {} of {} files by --include/--exclude",
            "ⓘ".blue().bold(),
            "Filtered out".bold(),
            filtered_out.to_string().bold(),
            selected_total
        );
    }

    // Download all files with integrated signal handling; numbering starts
    // right after the _files.xml saved above (or at 1 in a single-file run,
    // which never saves it).
    let first_file_number = if whole_item { XML_FILE_NUMBER + 1 } else { 1 };
    downloader::download_files(
        &client,
        plan.tasks,
        total_files,
        first_file_number,
        cookie_source.as_ref(),
        cli.stop_on_error,
        rate_limit,
        &output_dir,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_output_dir_trims_trailing_separators() {
        assert_eq!(normalize_output_dir(None).unwrap(), "");
        assert_eq!(normalize_output_dir(Some("out")).unwrap(), "out");
        assert_eq!(normalize_output_dir(Some("out/")).unwrap(), "out");
        assert_eq!(normalize_output_dir(Some("out/sub\\")).unwrap(), "out/sub");
        assert_eq!(normalize_output_dir(Some("out/sub/")).unwrap(), "out/sub");
        // A leading separator is kept: it names a root-level directory
        assert_eq!(normalize_output_dir(Some("/out/")).unwrap(), "/out");
        // A bare drive root keeps its separator: "C:" alone is
        // drive-relative on Windows, not the requested root
        #[cfg(windows)]
        {
            assert_eq!(normalize_output_dir(Some("C:\\")).unwrap(), "C:\\");
            assert_eq!(normalize_output_dir(Some("c://")).unwrap(), "c:\\");
        }
    }

    #[test]
    fn normalize_output_dir_rejects_paths_that_name_no_directory() {
        for arg in ["", "/", "\\", "///"] {
            assert!(
                matches!(
                    normalize_output_dir(Some(arg)),
                    Err(IaGetError::InvalidOutputDir(_))
                ),
                "{arg:?} must be an InvalidOutputDir error"
            );
        }
    }

    #[test]
    fn resolve_proxy_uses_the_explicit_value() {
        // An explicit http:// URL passes through and yields a proxy.
        assert!(
            resolve_proxy(Some("http://127.0.0.1:3128"))
                .unwrap()
                .is_some()
        );
        // A bare host:port is treated as an http proxy.
        assert!(resolve_proxy(Some("127.0.0.1:3128")).unwrap().is_some());
        // An unparseable proxy is a clean error, not a panic.
        assert!(matches!(
            resolve_proxy(Some("http://")).unwrap_err(),
            IaGetError::InvalidProxy(_)
        ));
    }

    #[test]
    fn parse_rate_limit_maps_the_flag_to_bytes() {
        assert_eq!(parse_rate_limit(None).unwrap(), None);
        assert_eq!(parse_rate_limit(Some("1M")).unwrap(), Some(1024 * 1024));
        // 0 is kept as Some(0): the limiter treats it as "no limit".
        assert_eq!(parse_rate_limit(Some("0")).unwrap(), Some(0));
        assert!(parse_rate_limit(Some("bogus")).is_err());
    }

    #[test]
    fn check_disk_space_skips_when_nothing_is_needed() {
        // A file with an unknown size needs 0 bytes, so the check passes no
        // matter how little space is left.
        let task = DownloadTask {
            url: "https://archive.org/download/item/file.bin".into(),
            file_path: "ia-get-disk-space-never-created.bin".into(),
            expected_md5: None,
            expected_size: None,
            expected_mtime: None,
        };
        assert!(check_disk_space("", &[task]).is_ok());
    }

    #[test]
    fn run_check_missing_directory_is_a_clear_error() {
        // -o pointing at a directory that does not exist must yield the
        // distinct CheckDirectoryNotFound, not a CheckFailed "everything
        // missing" report: a walk of a non-existent root lists nothing.
        let root = std::env::temp_dir().join(format!(
            "ia-get-check-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let missing = format!("{}/absent", root.display());

        let cli = Cli::parse_from(vec![
            "ia-get",
            "--check",
            "https://archive.org/details/item1",
            "-o",
            &missing,
        ]);
        let target = parse_archive_url("https://archive.org/details/item1").unwrap();
        let base_url = get_xml_url(&target.identifier).unwrap();
        // One real entry: with the guard mutated away, the run sails past
        // build_plan into the walk and fails as CheckFailed instead
        let files = XmlFiles {
            files: vec![XmlFile {
                name: "scan.jpg".to_string(),
                size: Some(1),
                ..Default::default()
            }],
        };

        let err = run_check(&create_spinner("test"), &cli, &target, &base_url, files).unwrap_err();

        assert!(
            matches!(err, IaGetError::CheckDirectoryNotFound(_)),
            "got: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
