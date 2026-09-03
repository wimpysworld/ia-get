//! Terminal output: the initialization spinner, per-file progress bars, the
//! file banners and status lines, and human-readable size/duration
//! formatting shared by every printed line.

use colored::*;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::time::Duration;

/// Spinner tick interval in milliseconds
const SPINNER_TICK_INTERVAL: u64 = 100;

/// Size constants for formatting
const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;

/// Create a progress bar with consistent styling
///
/// # Arguments
/// * `total` - Total value for the progress bar
/// * `action` - Action text to show at the beginning, pre-styled with the
///   `colored` crate (e.g., "╰╼ Downloading  ")
/// * `color` - Bar color style (e.g. "green/green", "blue/blue")
/// * `with_eta` - Whether to include ETA in the template
///
/// # Returns
/// A configured progress bar
pub fn create_progress_bar(total: u64, action: &str, color: &str, with_eta: bool) -> ProgressBar {
    let pb = ProgressBar::new(total);
    // Progress output belongs on stdout (where the status lines go);
    // indicatif's default target is stderr, which would put the bars into
    // the `--verbose 2> log` diagnostics capture
    pb.set_draw_target(ProgressDrawTarget::stdout());

    let template =
        format!("{action}{{elapsed_precise}} {{bar:40.{color}}} {{bytes}}/{{total_bytes}}");
    let template = if with_eta {
        format!("{template} (ETA: {{eta}})")
    } else {
        template
    };

    pb.set_style(
        ProgressStyle::default_bar()
            .template(&template)
            .expect("Failed to set progress bar style")
            .progress_chars("▓▒░"),
    );

    pb
}

/// Finishes and clears a progress bar, if one was created
pub fn finish_progress_bar(pb: &Option<ProgressBar>) {
    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
}

/// Tree glyph for a line with more lines following in the file's block.
pub fn branch_glyph() -> ColoredString {
    "├╼".cyan().dimmed()
}

/// Tree glyph for the last line of a file's block.
pub fn last_glyph() -> ColoredString {
    "╰╼".cyan().dimmed()
}

/// Print the "Filename / Count" banner for one file of a numbered list
pub fn print_file_banner(file_path: &str, number: usize, total: usize) {
    println!(
        "{}  {}     {}",
        "▣".bright_cyan().bold(),
        "Filename".white(),
        file_path.bold()
    );
    println!(
        "{} {}        {} {} of {}",
        branch_glyph(),
        "Count".white(),
        "#".blue().bold(),
        number.to_string().bold(),
        total.to_string().bold()
    );
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
    // Like the progress bars: the spinner is progress output, stdout —
    // not part of the `--verbose` stderr diagnostics
    spinner.set_draw_target(ProgressDrawTarget::stdout());
    // The template is a constant: user-controlled text (the archive URL)
    // rides in the {msg} property, never in the template itself — a '{'
    // in the URL would otherwise break the template parser and panic
    // before URL validation.
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.yellow} {msg}")
            .expect("constant spinner template"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(SPINNER_TICK_INTERVAL));
    spinner
}

/// Restyles a running spinner as a static completion message and finishes it
pub fn finish_spinner(spinner: &ProgressBar, message: &str) {
    // Constant template, message via the property: the completion text
    // (which may carry user input, e.g. the -o directory) must not be
    // parsed as template syntax
    spinner.set_style(ProgressStyle::with_template("{msg}").expect("constant style template"));
    spinner.set_message(message.to_string());
    spinner.finish();
}

/// Prints the "Failed ✘ Maximum retries (N) exceeded" status line
pub fn print_max_retries_exceeded(max_retries: u32) {
    println!(
        "{} {}       {} Maximum retries ({}) exceeded",
        branch_glyph(),
        "Failed".red().bold(),
        "✘".red().bold(),
        max_retries
    );
}

/// Prints the "Retry ⟳ kind (attempt x/y): detail" status line
pub fn print_retry_notice(kind: &str, attempt: u32, max_retries: u32, detail: &str) {
    println!(
        "{} {}        {} {} (attempt {attempt}/{max_retries}): {detail}",
        branch_glyph(),
        "Retry".yellow().bold(),
        "⟳".yellow().bold(),
        kind
    );
}

/// Prints the "Waiting N.Ns before retry" status line, noting when the
/// delay was requested by the server (Retry-After)
pub fn print_retry_wait(delay: &Duration, server_requested: bool) {
    println!(
        "{} {}         Waiting {:.1}s before retry{}",
        branch_glyph(),
        "Wait".white(),
        delay.as_secs_f64(),
        if server_requested {
            " (server requested)"
        } else {
            ""
        }
    );
}

/// Prints the "Partial ▲ the existing file failed verification,
/// re-downloading" status line
pub fn print_stale_file_redownload() {
    println!(
        "{} {}      {} the existing file failed verification, re-downloading",
        branch_glyph(),
        "Partial".white(),
        "▲".yellow().bold()
    );
}

/// Prints the "Resume ↻ the .part file is already complete, verifying it
/// in place" status line
pub fn print_complete_part_verification() {
    println!(
        "{} {}       {} the .part file is already complete, verifying it in place",
        branch_glyph(),
        "Resume".white(),
        "↻".green().bold()
    );
}

/// Prints the "Retry ⟳ Re-downloading from scratch (attempt x/y)" status line
pub fn print_redownload_from_scratch(attempt: u32, max_attempts: u32) {
    println!(
        "{} {}        {} Re-downloading from scratch (attempt {attempt}/{max_attempts})",
        branch_glyph(),
        "Retry".yellow().bold(),
        "⟳".yellow().bold()
    );
}

/// Prints the "⚠ Could not set last modified time" warning for a
/// best-effort mtime sync that failed; the batch carries on
pub fn print_mtime_warning(detail: &str) {
    println!(
        "{} {}      {}",
        "⚠".yellow().bold(),
        "Could not set last modified time".yellow(),
        detail.dimmed()
    );
}

/// Prints the end-of-batch "Download interrupted" line
pub fn print_download_interrupted() {
    println!(
        "\n{} Download interrupted. Run the command again to resume remaining files.",
        "✘".red().bold()
    );
}

/// Prints the "--check interrupted" line: the check is read-only, so there
/// is nothing to resume — just re-run it to finish.
pub fn print_check_interrupted() {
    println!(
        "\n{} Check interrupted. Run the command again to finish.",
        "✘".red().bold()
    );
}

/// Prints the end-of-batch summary line, mirroring the `--check` report's
/// closing tally: how many files the batch handled and how many succeeded
/// or failed.
pub fn print_download_summary(total: usize, ok: usize, failed: usize) {
    println!();
    println!(
        "{} downloaded {} file{}: {} ok, {} failed",
        "Σ".bold(),
        total,
        if total == 1 { "" } else { "s" },
        ok,
        failed
    );
}

/// Print the "Downloaded ↓ size" status line for a finished file
///
/// `prefix` is the pre-styled tree glyph: "├╼" when more lines follow in the
/// file's block, "╰╼" for its last line. It is taken by value so the
/// caller's styling survives (a `&ColoredString` coerced to `&str` would
/// deref to the plain, uncoloured text). When `elapsed` is present, the
/// transfer time and rate are appended; it is absent for files that never
/// crossed the network (e.g. the locally saved `_files.xml`).
pub fn print_downloaded_line(prefix: ColoredString, transferred: u64, elapsed: Option<Duration>) {
    let head = format!(
        "{} {}   {} {}",
        prefix,
        "Downloaded".white(),
        "↓".green().bold(),
        format_size(transferred).bold()
    );

    match elapsed {
        Some(elapsed) => {
            let elapsed_secs = elapsed.as_secs_f64();
            let rate = if elapsed_secs > 0.0 {
                transferred as f64 / elapsed_secs
            } else {
                0.0
            };
            let (rate, unit) = scaled_unit(rate);
            println!(
                "{head} in {} ({rate:.2} {unit}/s)",
                format_duration(elapsed).bold()
            );
        }
        None => println!("{head}"),
    }
}

/// Format a duration into a human-readable string
pub fn format_duration(duration: Duration) -> String {
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

/// Picks the human-readable unit (B/KB/MB/GB) for a byte count and returns
/// the value scaled to that unit
fn scaled_unit(value: f64) -> (f64, &'static str) {
    let kb = KB as f64;
    let mb = MB as f64;
    let gb = GB as f64;

    if value < kb {
        (value, "B")
    } else if value < mb {
        (value / kb, "KB")
    } else if value < gb {
        (value / mb, "MB")
    } else {
        (value / gb, "GB")
    }
}

/// Format a size in bytes to a human-readable string
pub fn format_size(size: u64) -> String {
    if size < KB {
        format!("{}B", size)
    } else {
        let (value, unit) = scaled_unit(size as f64);
        format!("{value:.2}{unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_templates_survive_user_controlled_braces() {
        // A URL (or -o path) carrying '{' must not break the template
        // parser: user text rides in the message property, the templates
        // are constants
        let spinner =
            create_spinner("Processing archive.org URL: https://archive.org/details/{bad}");
        finish_spinner(&spinner, "Verifying 1 file in {weird}");
    }
}
