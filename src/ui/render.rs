//! Pure rendering functions for the archival inline instrument, completion lines,
//! framed Archival Receipt summary card, and 3-tier adaptive exception ledger.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::downloader::BACKPRESSURE_RECOVERY_SUCCESSES;
use crate::ui::palette::{
    style_accent, style_bold, style_err, style_muted, style_ok, style_ratatui_to_ansi, style_warn,
    ColorLevel, Glyphs, Theme,
};
use crate::ui::state::{
    ActiveSlot, AttentionItem, CompletionKind, CompletionRecord, QueuedFile, SkipReason,
    TransferStage, UiState,
};
use crate::utils::{format_duration, format_size, format_transfer_rate};

/// Standard maximum height (in terminal rows) of the framed archival inline instrument.
pub const INSTRUMENT_HEIGHT: u16 = 8;

/// Height (in terminal rows) of the collapsed aggregate summary below minimum dimensions.
pub const COLLAPSED_HEIGHT: u16 = 2;

/// Minimum terminal width for the full bordered instrument (with mirror region badge and sparkline).
pub const FULL_WIDTH_BREAKPOINT: u16 = 80;

/// Minimum terminal width for the compact bordered instrument (hiding mirror region badge and sparkline).
pub const COMPACT_WIDTH_BREAKPOINT: u16 = 60;

/// Minimum terminal width below which the instrument collapses to a 2-line aggregate summary.
pub const MIN_BORDERED_WIDTH: u16 = 48;

/// Maximum rows shown in the adaptive exception ledger before capping.
const LEDGER_MAX_UNTRUNCATED_GROUP: usize = 20;

/// Number of rows kept when a ledger group exceeds [`LEDGER_MAX_UNTRUNCATED_GROUP`].
const LEDGER_CAPPED_ROWS: usize = 15;

/// Computes the steady transfer-bay row count for a batch (`min(4, total_files)`, at least 1).
pub const fn compute_bay_rows(total_files: usize) -> usize {
    if total_files == 0 {
        1
    } else if total_files > 4 {
        4
    } else {
        total_files
    }
}

/// Computes the inline viewport height for a terminal of `(width, height)`.
pub const fn compute_viewport_height(term_width: u16, term_height: u16) -> u16 {
    compute_viewport_height_for_batch(term_width, term_height, 4)
}

/// Computes the inline viewport height for a terminal of `(width, height)` and `total_files`.
pub const fn compute_viewport_height_for_batch(
    term_width: u16,
    term_height: u16,
    total_files: usize,
) -> u16 {
    let target = (4 + compute_bay_rows(total_files)) as u16;
    if term_width < MIN_BORDERED_WIDTH || term_height < target {
        if term_height < COLLAPSED_HEIGHT {
            term_height
        } else {
            COLLAPSED_HEIGHT
        }
    } else {
        target
    }
}

/// Truncates `s` in the middle to fit within `max_cells` terminal columns using `unicode-width`,
/// preserving both the prefix and basename/extension with zero UTF-8 splitting or width overflow.
pub fn middle_truncate(s: &str, max_cells: usize, ellipsis: &str) -> String {
    if max_cells == 0 {
        return String::new();
    }
    if s.width() <= max_cells {
        return s.to_string();
    }

    let ell_w = ellipsis.width();
    if max_cells <= ell_w {
        return tail_truncate(ellipsis, max_cells);
    }

    let budget = max_cells - ell_w;
    let head_budget = budget / 2;
    let tail_budget = budget - head_budget;

    let mut head = String::new();
    let mut head_w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if head_w + cw > head_budget {
            break;
        }
        head.push(ch);
        head_w += cw;
    }

    let mut tail_rev: Vec<char> = Vec::new();
    let mut tail_w = 0usize;
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if tail_w + cw > tail_budget {
            break;
        }
        // Avoid keeping a leading zero-width combining mark whose base char did not fit.
        if cw == 0 && tail_w == tail_budget {
            break;
        }
        tail_rev.push(ch);
        tail_w += cw;
    }
    // Strip any dangling zero-width combining marks at the start of the tail slice.
    while tail_rev
        .last()
        .is_some_and(|ch| ch.width().unwrap_or(0) == 0)
    {
        tail_rev.pop();
    }
    tail_rev.reverse();

    let mut out = String::with_capacity(head.len() + ellipsis.len() + tail_rev.len() * 4);
    out.push_str(&head);
    out.push_str(ellipsis);
    for ch in tail_rev {
        out.push(ch);
    }
    out
}

/// Truncates `s` from the right to fit within `max_cells` terminal columns.
pub fn tail_truncate(s: &str, max_cells: usize) -> String {
    if max_cells == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if width + cw > max_cells {
            break;
        }
        out.push(ch);
        width += cw;
    }
    out
}

/// Pads `s` with trailing spaces to reach `target_cells` display columns.
fn pad_right(s: &str, target_cells: usize) -> String {
    let w = s.width();
    if w >= target_cells {
        tail_truncate(s, target_cells)
    } else {
        let mut out = String::with_capacity(s.len() + (target_cells - w));
        out.push_str(s);
        for _ in 0..(target_cells - w) {
            out.push(' ');
        }
        out
    }
}

/// Formats a whole-second ETA value (`--:--`, `00:42`, `1h05m`, `12h05m`, `>99h`).
pub fn format_eta(eta_secs: Option<u64>) -> String {
    match eta_secs {
        None => "--:--".to_string(),
        Some(secs) if secs < 3600 => format!("{:02}:{:02}", secs / 60, secs % 60),
        Some(secs) if secs < 360_000 => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
        Some(_) => ">99h".to_string(),
    }
}

/// Formats a transfer speed in bytes/sec with compact unit labels.
pub fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 {
        return "---".to_string();
    }
    let (val, unit) = format_transfer_rate(bytes_per_sec);
    if unit == "B" {
        format!("{:.0} B/s", val)
    } else if val >= 100.0 {
        format!("{:.0} {unit}/s", val)
    } else {
        format!("{:.1} {unit}/s", val)
    }
}

/// Formats a byte count compactly for dense instrument rows (e.g. `1.42GB`, `512KB`).
fn format_compact_bytes(bytes: u64) -> String {
    format_size(bytes)
}

/// Builds smooth eighth-block progress bar spans with bronze-to-gold gradient (or slate for MD5).
fn build_bar_spans(ratio: f64, width: usize, is_hash: bool, theme: &Theme) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let glyphs = theme.glyphs();
    let total_eighths = (ratio.clamp(0.0, 1.0) * (width * 8) as f64).round() as usize;
    let full_cells = (total_eighths / 8).min(width);
    let remainder = if full_cells >= width {
        0
    } else {
        total_eighths % 8
    };
    let filled_total = full_cells + usize::from(remainder > 0);

    let mut spans = Vec::with_capacity(width);
    for cell in 0..full_cells {
        spans.push(Span::styled(
            glyphs.bar_blocks[7],
            theme.bar_fill_style(cell, width, is_hash),
        ));
    }
    if remainder > 0 {
        spans.push(Span::styled(
            glyphs.bar_blocks[remainder - 1],
            theme.bar_fill_style(full_cells, width, is_hash),
        ));
    }
    if filled_total < width {
        let empty = glyphs.bar_empty.repeat(width - filled_total);
        spans.push(Span::styled(empty, theme.track_style()));
    }
    spans
}

/// Builds styled spans and display width for a throughput sparkline slice.
///
/// Zero-value samples render in the muted bar-track style so idle slots stay calm.
fn build_sparkline_spans_from_samples(
    samples: impl ExactSizeIterator<Item = u64> + Clone,
    theme: &Theme,
) -> (Vec<Span<'static>>, usize) {
    let glyphs = theme.glyphs();
    let max_val = samples.clone().max().unwrap_or(0);
    let mut spans = Vec::with_capacity(samples.len());
    let mut width = 0usize;
    for sample in samples {
        let (glyph, style) = if max_val == 0 || sample == 0 {
            (glyphs.spark_blocks[0], theme.track_style())
        } else {
            let idx = (((sample as f64 / max_val as f64) * 7.0).round() as usize).min(7);
            (glyphs.spark_blocks[idx], theme.accent_style())
        };
        width += glyph.width();
        spans.push(Span::styled(glyph, style));
    }
    (spans, width)
}

/// Builds styled spans and display width for the live 12-sample throughput sparkline.
fn build_sparkline_spans(state: &UiState, theme: &Theme) -> (Vec<Span<'static>>, usize) {
    build_sparkline_spans_from_samples(state.sparkline.iter().copied(), theme)
}

/// Extracts the uppercase 2-letter region code from an archive.org mirror hostname
/// matching `*.<cc>.archive.org` (for example `ia801909.us.archive.org` -> `['U', 'S']`).
pub fn extract_mirror_region(host: &str) -> Option<[char; 2]> {
    let trimmed = host.trim().trim_end_matches('.');
    let labels: Vec<&str> = trimmed.split('.').collect();
    if labels.len() < 4 || labels.iter().any(|l| l.is_empty()) {
        return None;
    }
    let n = labels.len();
    if !labels[n - 2].eq_ignore_ascii_case("archive") || !labels[n - 1].eq_ignore_ascii_case("org")
    {
        return None;
    }
    let region = labels[n - 3];
    let bytes = region.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1].is_ascii_alphabetic() {
        Some([
            bytes[0].to_ascii_uppercase() as char,
            bytes[1].to_ascii_uppercase() as char,
        ])
    } else {
        None
    }
}

/// Formats a mirror host into a 2-cell regional indicator flag emoji (or 2-letter ASCII code
/// when `use_emoji` is `false`), returning `(badge, resolved)`.
///
/// Pre-redirect `archive.org` and hosts without a 2-letter region code return `("--", false)`.
pub fn format_mirror_region(host: Option<&str>, use_emoji: bool) -> (String, bool) {
    let Some([a, b]) = host.and_then(extract_mirror_region) else {
        return ("--".to_string(), false);
    };
    if use_emoji {
        let to_ri = |c: char| char::from_u32(0x1F1E6 + (c as u32 - 'A' as u32));
        if let (Some(ri_a), Some(ri_b)) = (to_ri(a), to_ri(b)) {
            let mut flag = String::with_capacity(8);
            flag.push(ri_a);
            flag.push(ri_b);
            return (flag, true);
        }
    }
    (format!("{a}{b}"), true)
}

/// Returns `(done, total)` file counts for the live header progress indicator.
fn completed_file_counts(state: &UiState) -> (usize, usize) {
    let done = state
        .ok_count
        .saturating_add(state.skip_count)
        .saturating_add(state.hash_mismatch_count())
        .saturating_add(state.fail_count);
    let total = state.total_files.max(done);
    (done, total)
}

/// Formats the filename cell for an active slot to fit `max_cells`.
fn format_slot_filename(slot: &ActiveSlot, max_cells: usize, glyphs: &Glyphs) -> String {
    if max_cells == 0 {
        return String::new();
    }
    pad_right(
        &middle_truncate(&slot.file_path, max_cells, glyphs.ellipsis),
        max_cells,
    )
}

/// Renders the header row (`ia-get · <identifier>` on left; file progress, sparkline, speed on right).
fn build_header_line(
    state: &UiState,
    inner_width: usize,
    show_sparkline: bool,
    compact_files: bool,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let (done, total) = completed_file_counts(state);
    let unit = if total == 1 { "file" } else { "files" };
    let done_str = done.to_string();
    let full_rest = format!("/{total} {unit}");
    let short_rest = format!("/{total}");
    let sep_str = format!("  {}  ", glyphs.sep);
    let speed_text = format_speed(state.aggregate_speed_bps);

    let (spark_spans, spark_extra_w) = if show_sparkline {
        let (spans, w) = build_sparkline_spans(state, theme);
        (Some(spans), sep_str.width() + w)
    } else {
        (None, 0)
    };

    let full_right_w =
        done_str.width() + full_rest.width() + spark_extra_w + sep_str.width() + speed_text.width();
    let use_short_files = compact_files || inner_width.saturating_sub(full_right_w + 2) < 16;
    let total_rest = if use_short_files {
        short_rest
    } else {
        full_rest
    };

    let mut right_spans: Vec<Span<'static>> = Vec::new();
    let mut right_width = 0usize;

    right_spans.push(Span::styled(done_str.clone(), theme.bold_style()));
    right_width += done_str.width();
    right_spans.push(Span::styled(total_rest.clone(), theme.muted_style()));
    right_width += total_rest.width();

    if let Some(spark_spans) = spark_spans {
        right_spans.push(Span::styled(sep_str.clone(), theme.muted_style()));
        right_spans.extend(spark_spans);
        right_width += spark_extra_w;
    }

    right_spans.push(Span::styled(sep_str.clone(), theme.muted_style()));
    right_spans.push(Span::styled(speed_text.clone(), theme.primary_style()));
    right_width += sep_str.width() + speed_text.width();

    let prefix = format!("ia-get {} ", glyphs.sep);
    let prefix_w = prefix.width();
    let left_budget = inner_width.saturating_sub(right_width + 2);
    let id_budget = left_budget.saturating_sub(prefix_w);

    let mut spans = Vec::new();
    let left_used = if left_budget > prefix_w {
        let id_trunc = middle_truncate(&state.identifier, id_budget, glyphs.ellipsis);
        let id_w = id_trunc.width();
        spans.push(Span::styled("ia-get", theme.title_style()));
        spans.push(Span::styled(
            format!(" {} ", glyphs.sep),
            theme.muted_style(),
        ));
        spans.push(Span::styled(id_trunc, theme.primary_style()));
        prefix_w + id_w
    } else {
        let short = tail_truncate("ia-get", left_budget);
        let w = short.width();
        spans.push(Span::styled(short, theme.title_style()));
        w
    };

    let pad = inner_width.saturating_sub(left_used + right_width);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend(right_spans);
    Line::from(spans)
}

/// Renders the `TOTAL` aggregate row with a locked eighth-block bar, bytes, calm counters, and ETA/cap.
fn build_total_line(
    state: &UiState,
    inner_width: usize,
    bar_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let pct_str = format!(" {:>3}%", state.total_display_percent());
    let cur_bytes = state.aggregate_current_bytes();
    let tot_bytes = state.aggregate_total_bytes();
    let bytes_str = if tot_bytes > 0 {
        format!(
            "  {}/{}",
            format_compact_bytes(cur_bytes),
            format_compact_bytes(tot_bytes)
        )
    } else {
        format!("  {}", format_compact_bytes(cur_bytes))
    };

    // Right-hand ETA or rate cap label.
    let right_label = match state.limit_rate {
        Some(cap) => {
            let cap_str = format!("cap {}", format_speed(cap as f64));
            let eta_str = format!("ETA {}", format_eta(state.eta_secs));
            if inner_width >= 96 {
                format!("{cap_str}  {eta_str}")
            } else {
                cap_str
            }
        }
        None => format!("ETA {}", format_eta(state.eta_secs)),
    };
    let right_w = right_label.width();

    let label = "TOTAL ";
    let left_fixed = label.width() + bar_width + pct_str.width();
    let mut middle_budget = inner_width.saturating_sub(left_fixed + 1 + right_w);

    // Optional middle segments: hide 0 skip, 0 warn, and 0 fail.
    let fail_spans = (state.fail_count > 0).then(|| {
        let rest = format!(" {} fail", state.fail_count);
        let w = 2 + glyphs.fail.width() + rest.width();
        (
            vec![
                Span::raw("  "),
                Span::styled(glyphs.fail, theme.bold_if_color(theme.err_style())),
                Span::styled(rest, theme.err_style()),
            ],
            w,
        )
    });
    let warn_spans = (state.warn_count > 0).then(|| {
        let rest = format!(" {} warn", state.warn_count);
        let w = 2 + glyphs.warn.width() + rest.width();
        (
            vec![
                Span::raw("  "),
                Span::styled(glyphs.warn, theme.bold_if_color(theme.warn_style())),
                Span::styled(rest, theme.warn_style()),
            ],
            w,
        )
    });
    let ok_text = format!("  {} ok", state.ok_count);
    let ok_w = ok_text.width();
    let ok_spans = (vec![Span::styled(ok_text, theme.ok_style())], ok_w);
    let skip_spans = (state.skip_count > 0).then(|| {
        let text = format!("  {} skip", state.skip_count);
        let w = text.width();
        (vec![Span::styled(text, theme.muted_style())], w)
    });

    // Allocate space in priority order (fail > warn > ok > skip > bytes) so narrow rows
    // drop bytes before hiding warning or failure counters.
    let take_if_fits = |group: Option<(Vec<Span<'static>>, usize)>,
                        budget: &mut usize|
     -> Option<(Vec<Span<'static>>, usize)> {
        let (spans, w) = group?;
        if w <= *budget {
            *budget -= w;
            Some((spans, w))
        } else {
            None
        }
    };

    let kept_fail = take_if_fits(fail_spans, &mut middle_budget);
    let kept_warn = take_if_fits(warn_spans, &mut middle_budget);
    let kept_ok = take_if_fits(Some(ok_spans), &mut middle_budget);
    let kept_skip = take_if_fits(skip_spans, &mut middle_budget);

    let all_counters_kept = (state.fail_count == 0 || kept_fail.is_some())
        && (state.warn_count == 0 || kept_warn.is_some())
        && kept_ok.is_some()
        && (state.skip_count == 0 || kept_skip.is_some());
    // Require one extra cell before right_label when bytes are shown.
    let show_bytes = all_counters_kept && bytes_str.width() < middle_budget;

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    spans.push(Span::styled("TOTAL", theme.title_style()));
    spans.push(Span::raw(" "));
    used += label.width();

    spans.extend(build_bar_spans(
        state.total_display_ratio,
        bar_width,
        false,
        theme,
    ));
    used += bar_width;

    spans.push(Span::styled(pct_str.clone(), theme.primary_style()));
    used += pct_str.width();

    if show_bytes {
        spans.push(Span::styled(bytes_str.clone(), theme.muted_style()));
        used += bytes_str.width();
    }

    for (group_spans, w) in [kept_ok, kept_skip, kept_warn, kept_fail]
        .into_iter()
        .flatten()
    {
        used += w;
        spans.extend(group_spans);
    }

    let pad = inner_width.saturating_sub(used + right_w);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    if used + right_w <= inner_width {
        spans.push(Span::styled(right_label, theme.muted_style()));
    }

    Line::from(spans)
}

/// Renders one active transfer slot row inside the instrument frame.
fn build_slot_line(
    slot: &ActiveSlot,
    state: &UiState,
    inner_width: usize,
    show_mirror: bool,
    bar_width: usize,
    use_region_emoji: bool,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let (stage_glyph, stage_style) = match slot.stage {
        TransferStage::Get => (glyphs.stage_get, theme.stage_get_style()),
        TransferStage::Resume => (glyphs.stage_resume, theme.stage_resume_style()),
        TransferStage::Redirect => (glyphs.stage_redirect, theme.stage_redirect_style()),
        TransferStage::Md5 => (
            glyphs.stage_md5[state.md5_spinner_frame % glyphs.stage_md5.len()],
            theme.stage_md5_style(),
        ),
        TransferStage::RateLimited => (glyphs.stage_rate_limited, theme.stage_rate_limited_style()),
    };
    // "<glyph>  " = 1 + 2 = 3 cells, matching queued rows.
    let prefix_w = stage_glyph.width() + 2;

    if slot.stage == TransferStage::RateLimited {
        let cd_secs = slot
            .cooldown_remaining
            .or(state.scheduler.cooldown_remaining)
            .map_or(0, |d| d.as_secs());
        let ceiling = state.scheduler.effective_ceiling.clamp(1, 4);
        let filled = state
            .scheduler
            .clean_successes
            .min(BACKPRESSURE_RECOVERY_SUCCESSES);
        let empty = BACKPRESSURE_RECOVERY_SUCCESSES.saturating_sub(filled);
        let pips = format!(
            "{}{}",
            glyphs.pip_filled.repeat(filled),
            glyphs.pip_empty.repeat(empty)
        );

        let full_status = format!("cooldown {cd_secs:02}s  ceiling {ceiling}/4  recovery {pips}");
        let compact_status = format!("cooldown {cd_secs:02}s  {ceiling}/4  {pips}");
        let tiny_status = format!("cooldown {cd_secs:02}s");
        let status = if inner_width >= prefix_w + 14 + 2 + full_status.width() {
            full_status
        } else if inner_width >= prefix_w + 10 + 2 + compact_status.width() {
            compact_status
        } else {
            tiny_status
        };
        let status_w = status.width();
        let name_budget = inner_width.saturating_sub(prefix_w + 2 + status_w);
        let name = format_slot_filename(slot, name_budget, glyphs);
        let used = prefix_w + name.width() + 2 + status_w;
        let pad = inner_width.saturating_sub(used);

        return Line::from(vec![
            Span::styled(stage_glyph, stage_style),
            Span::raw("  "),
            Span::styled(name, theme.primary_style()),
            Span::raw("  "),
            Span::raw(" ".repeat(pad)),
            Span::styled(status, theme.warn_style()),
        ]);
    }

    let is_hash = slot.stage == TransferStage::Md5;
    let pct_str = if slot.total_bytes.is_none() && !is_hash && slot.display_ratio == 0.0 {
        "  --%".to_string()
    } else {
        format!(" {:>3}%", slot.display_percent())
    };

    let (info_str, info_style, include_mirror) = match slot.stage {
        TransferStage::Md5 => {
            if show_mirror {
                // Occupy the exact same 15-cell right column (11 speed + 4 region) so bars stay aligned.
                ("  verifying MD5".to_string(), theme.ok_style(), false)
            } else {
                // Occupy the exact same 11-cell right column as `" {:>10}"` speed slots so bars stay aligned.
                (" verify MD5".to_string(), theme.ok_style(), false)
            }
        }
        TransferStage::Redirect => (
            format!(" {:>10}", "redirect"),
            theme.info_style(),
            show_mirror,
        ),
        _ => (
            format!(" {:>10}", format_speed(slot.speed_bps)),
            theme.muted_style(),
            show_mirror,
        ),
    };

    let mirror_col_w = if include_mirror { 4 } else { 0 }; // "  " + 2-cell region badge
    let right_w = bar_width + pct_str.width() + info_str.width() + mirror_col_w;
    let name_budget = inner_width.saturating_sub(prefix_w + 1 + right_w);
    let name = format_slot_filename(slot, name_budget, glyphs);

    let mut spans = Vec::with_capacity(bar_width + 8);
    spans.push(Span::styled(stage_glyph, stage_style));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(name, theme.primary_style()));
    spans.push(Span::raw(" "));
    spans.extend(build_bar_spans(
        slot.display_ratio,
        bar_width,
        is_hash,
        theme,
    ));
    spans.push(Span::styled(pct_str, theme.primary_style()));
    spans.push(Span::styled(info_str, info_style));

    if include_mirror {
        let (badge, resolved) = format_mirror_region(slot.mirror_host.as_deref(), use_region_emoji);
        let badge_style = if !resolved {
            theme.track_style()
        } else if use_region_emoji {
            theme.primary_style()
        } else {
            theme.muted_style()
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(badge, badge_style));
    }

    Line::from(spans)
}

/// Renders one dimmed up-next queued file row (`○  <filename> ... <size>`).
fn build_queued_line(
    queued: &QueuedFile,
    extra_more: Option<usize>,
    inner_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let qstyle = theme.queued_style();
    let prefix = format!("{}  ", glyphs.stage_queued);
    let prefix_w = prefix.width();

    let size_str = queued
        .size_bytes
        .map(format_size)
        .unwrap_or_else(|| "---".to_string());
    let right_str = match extra_more {
        Some(more) => format!("{size_str}  (+{more} more)"),
        None => size_str,
    };
    let right_w = right_str.width();

    let name_budget = inner_width.saturating_sub(prefix_w + 2 + right_w);
    let name = pad_right(
        &middle_truncate(&queued.name, name_budget, glyphs.ellipsis),
        name_budget,
    );

    Line::from(vec![
        Span::styled(prefix, qstyle),
        Span::styled(name, qstyle),
        Span::raw("  "),
        Span::styled(right_str, qstyle),
    ])
}

/// Renders the dimmed overflow summary line (`○  +N more in queue`).
fn build_queue_overflow_line(remaining: usize, inner_width: usize, theme: &Theme) -> Line<'static> {
    let glyphs = theme.glyphs();
    let text = format!("{}  +{remaining} more in queue", glyphs.stage_queued);
    Line::from(Span::styled(
        tail_truncate(&text, inner_width),
        theme.queued_style(),
    ))
}

/// Renders the collapsed 2-line aggregate summary used when width < 48 or height < target_height.
fn render_collapsed(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let glyphs = theme.glyphs();
    let width = area.width as usize;

    let (done, total) = completed_file_counts(state);
    let speed = format_speed(state.aggregate_speed_bps);
    let line1_plain = format!(
        "ia-get {} {}  {done}/{total}  {speed}",
        glyphs.sep, state.identifier
    );
    let line1_fit = middle_truncate(&line1_plain, width, glyphs.ellipsis);
    let line1_style = if state.scheduler.cooldown_remaining.is_some() {
        theme.warn_style()
    } else {
        theme.accent_style()
    };

    let mut line2_parts = vec![
        format!("{:>3}%", state.total_display_percent()),
        format!("{} ok", state.ok_count),
    ];
    if state.skip_count > 0 {
        line2_parts.push(format!("{} skip", state.skip_count));
    }
    if state.warn_count > 0 {
        line2_parts.push(format!("{}{}", glyphs.warn, state.warn_count));
    }
    if state.fail_count > 0 {
        line2_parts.push(format!("{}{}", glyphs.fail, state.fail_count));
    }
    line2_parts.push(format_eta(state.eta_secs));
    let line2_plain = line2_parts.join(" ");
    let line2_fit = tail_truncate(&line2_plain, width);

    let mut lines = vec![Line::from(Span::styled(line1_fit, line1_style))];
    if area.height >= 2 {
        lines.push(Line::from(Span::styled(line2_fit, theme.primary_style())));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Pure renderer for the Archival Inline Instrument.
///
/// Adapts across three responsive breakpoints without wrapping or panicking:
/// - `>= 80` cols (and `>= target_height` rows): full instrument with mirror region badge and sparkline
/// - `60..=79` cols (and `>= target_height` rows): compact instrument hiding mirror region badge and sparkline, 12-cell bars
/// - `48..=59` cols (and `>= target_height` rows): extra-compact bordered instrument with 8-cell bars
/// - `< 48` cols or `< target_height` rows: 2-line borderless aggregate summary
pub fn render_instrument(frame: &mut Frame, state: &UiState, theme: &Theme) {
    render_instrument_with_emoji(frame, state, theme, theme.use_region_flag_emoji());
}

fn render_instrument_with_emoji(
    frame: &mut Frame,
    state: &UiState,
    theme: &Theme,
    use_region_emoji: bool,
) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let bay_rows = compute_bay_rows(state.total_files);
    let target_height = (4 + bay_rows) as u16;
    if area.width < MIN_BORDERED_WIDTH || area.height < target_height {
        render_collapsed(frame, area, state, theme);
        return;
    }

    let glyphs = theme.glyphs();
    let border_set = border::Set {
        top_left: glyphs.top_left,
        top_right: glyphs.top_right,
        bottom_left: glyphs.bottom_left,
        bottom_right: glyphs.bottom_right,
        vertical_left: glyphs.vertical,
        vertical_right: glyphs.vertical,
        horizontal_top: glyphs.horizontal,
        horizontal_bottom: glyphs.horizontal,
    };

    let instrument_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: target_height.min(area.height),
    };

    let block = Block::bordered()
        .border_set(border_set)
        .border_style(theme.border_style());
    let inner = block.inner(instrument_area);
    frame.render_widget(block, instrument_area);

    if inner.width < 4 || inner.height < 2 {
        return;
    }

    // 1-cell left/right breathing margin inside the single bronze outer border.
    let padded = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    let inner_width = padded.width as usize;

    let is_full = area.width >= FULL_WIDTH_BREAKPOINT;
    let is_compact_narrow = area.width < COMPACT_WIDTH_BREAKPOINT;
    let show_queue = area.width >= COMPACT_WIDTH_BREAKPOINT && area.height >= target_height;
    let total_bar_width = if is_full {
        14
    } else if area.width >= COMPACT_WIDTH_BREAKPOINT {
        10
    } else {
        6
    };
    let slot_bar_width = if area.width >= 108 {
        22
    } else if is_full {
        16
    } else if area.width >= COMPACT_WIDTH_BREAKPOINT {
        12
    } else {
        8
    };

    let rows = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Length(1), // TOTAL
        Constraint::Min(0),    // Fixed transfer bay (1..=4 rows)
    ])
    .split(padded);

    let header_line = build_header_line(state, inner_width, is_full, is_compact_narrow, theme);
    frame.render_widget(Paragraph::new(header_line), rows[0]);

    let total_line = build_total_line(state, inner_width, total_bar_width, theme);
    frame.render_widget(Paragraph::new(total_line), rows[1]);

    let slot_rows = (rows[2].height as usize).min(bay_rows);
    if slot_rows == 0 {
        return;
    }

    let mut next_row = 0usize;

    if state.active_slots.is_empty() {
        if let Some(cd) = state.scheduler.cooldown_remaining {
            let ceiling = state.scheduler.effective_ceiling.clamp(1, 4);
            let msg = format!("Host cooldown {:02}s  ceiling {ceiling}/4", cd.as_secs());
            let line = Line::from(Span::styled(
                tail_truncate(&msg, inner_width),
                theme.warn_style(),
            ));
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y,
                width: rows[2].width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line), row_rect);
            next_row = 1;
        } else if (!show_queue || state.pending_queue.is_empty())
            && state.ok_count + state.skip_count + state.fail_count == 0
        {
            let line = Line::from(Span::styled(
                tail_truncate("Preparing transfers...", inner_width),
                theme.muted_style(),
            ));
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y,
                width: rows[2].width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line), row_rect);
            return;
        }
    } else {
        for slot in state.active_slots.iter().take(slot_rows) {
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y + next_row as u16,
                width: rows[2].width,
                height: 1,
            };
            let line = build_slot_line(
                slot,
                state,
                inner_width,
                is_full,
                slot_bar_width,
                use_region_emoji,
                theme,
            );
            frame.render_widget(Paragraph::new(line), row_rect);
            next_row += 1;
        }
    }

    // Fill idle bay rows below active transfers with dimmed `○` queued entries on >= 60 cols.
    let free_rows = slot_rows.saturating_sub(next_row);
    if !show_queue || free_rows == 0 || state.pending_queue.is_empty() {
        return;
    }

    let pending_len = state.pending_queue.len();
    if pending_len <= free_rows {
        for queued in &state.pending_queue {
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y + next_row as u16,
                width: rows[2].width,
                height: 1,
            };
            let line = build_queued_line(queued, None, inner_width, theme);
            frame.render_widget(Paragraph::new(line), row_rect);
            next_row += 1;
        }
    } else if free_rows >= 2 {
        let direct_count = free_rows - 1;
        for queued in state.pending_queue.iter().take(direct_count) {
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y + next_row as u16,
                width: rows[2].width,
                height: 1,
            };
            let line = build_queued_line(queued, None, inner_width, theme);
            frame.render_widget(Paragraph::new(line), row_rect);
            next_row += 1;
        }
        let remaining = pending_len - direct_count;
        let row_rect = Rect {
            x: rows[2].x,
            y: rows[2].y + next_row as u16,
            width: rows[2].width,
            height: 1,
        };
        let line = build_queue_overflow_line(remaining, inner_width, theme);
        frame.render_widget(Paragraph::new(line), row_rect);
    } else {
        // Exactly 1 free row and pending_len > 1: show next queued item with `(+N more)`,
        // or fall back to `○  +<pending_len> more in queue` if too narrow.
        let first = &state.pending_queue[0];
        let more = pending_len - 1;
        let size_str = first
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "---".to_string());
        let right_preview = format!("{size_str}  (+{more} more)");
        let line = if 3 + 8 + 2 + right_preview.width() <= inner_width {
            build_queued_line(first, Some(more), inner_width, theme)
        } else {
            build_queue_overflow_line(pending_len, inner_width, theme)
        };
        let row_rect = Rect {
            x: rows[2].x,
            y: rows[2].y + next_row as u16,
            width: rows[2].width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), row_rect);
    }
}

/// Formats a permanent single-line completion record as a styled string (for non-TTY or plain logs).
pub fn render_completion_line(
    record: &CompletionRecord,
    theme: &Theme,
    max_width: Option<usize>,
) -> String {
    let glyphs = theme.glyphs();
    let (icon, icon_styled, detail) = match &record.kind {
        CompletionKind::Downloaded {
            bytes,
            duration,
            verified_md5,
        } => {
            let md5_part = if *verified_md5 { ", MD5 ok" } else { "" };
            (
                glyphs.ok,
                style_ratatui_to_ansi(glyphs.ok, theme.bold_if_color(theme.ok_style()), theme),
                format!(
                    "({} in {}{md5_part})",
                    format_size(*bytes),
                    format_duration(*duration)
                ),
            )
        }
        CompletionKind::Skipped { reason, .. } => {
            let why = match reason {
                SkipReason::AlreadyVerified => "skipped, already verified",
                SkipReason::AlreadyExists => "skipped, already exists",
            };
            (
                glyphs.skip,
                style_muted(glyphs.skip, theme),
                format!("({why})"),
            )
        }
        CompletionKind::HashMismatch { .. } => (
            glyphs.warn,
            style_ratatui_to_ansi(glyphs.warn, theme.bold_if_color(theme.warn_style()), theme),
            "(MD5 mismatch, kept .part)".to_string(),
        ),
        CompletionKind::Failed { message } => (
            glyphs.fail,
            style_ratatui_to_ansi(glyphs.fail, theme.bold_if_color(theme.err_style()), theme),
            format!("({message})"),
        ),
    };

    let (name_display, detail_display) = match max_width {
        Some(limit) if limit > 0 => {
            let fixed = icon.width() + 1 + 2; // "<icon> <name>  <detail>"
            let detail_w = detail.width();
            if fixed + detail_w + 8 <= limit {
                let name_budget = limit - fixed - detail_w;
                (
                    middle_truncate(&record.file_path, name_budget, glyphs.ellipsis),
                    detail,
                )
            } else {
                let avail = limit.saturating_sub(icon.width() + 1);
                let combined = format!("{}  {detail}", record.file_path);
                return format!("{icon_styled} {}", tail_truncate(&combined, avail));
            }
        }
        _ => (record.file_path.clone(), detail),
    };

    let detail_styled = match &record.kind {
        CompletionKind::HashMismatch { .. } => style_warn(&detail_display, theme),
        CompletionKind::Failed { .. } => style_err(&detail_display, theme),
        _ => style_muted(&detail_display, theme),
    };

    format!("{icon_styled} {name_display}  {detail_styled}")
}

/// Clips and right-pads a span vector to exact display width `inner_width`.
fn clip_and_pad_spans(spans: Vec<Span<'static>>, inner_width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut used = 0usize;
    for span in spans {
        if used >= inner_width {
            break;
        }
        let w = span.content.width();
        if used + w <= inner_width {
            used += w;
            out.push(span);
        } else {
            let clipped = tail_truncate(&span.content, inner_width - used);
            used += clipped.width();
            out.push(Span::styled(clipped, span.style));
            break;
        }
    }
    if used < inner_width {
        out.push(Span::raw(" ".repeat(inner_width - used)));
    }
    out
}

/// Wraps inner receipt spans in `│ <inner> │`, guaranteeing exact display width `inner_width + 4`.
fn frame_receipt_row(
    spans: Vec<Span<'static>>,
    inner_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let mut row = Vec::with_capacity(spans.len() + 4);
    row.push(Span::styled(glyphs.vertical, theme.border_style()));
    row.push(Span::raw(" "));
    row.extend(clip_and_pad_spans(spans, inner_width));
    row.push(Span::raw(" "));
    row.push(Span::styled(glyphs.vertical, theme.border_style()));
    Line::from(row)
}

/// Allocates `bar_width` cells proportionally across `[ok, skip, warn, fail, unstarted]`,
/// ensuring every non-zero bucket gets at least 1 cell whenever width permits.
fn allocate_outcome_bar_widths(counts: [usize; 5], bar_width: usize) -> [usize; 5] {
    let mut widths = [0usize; 5];
    if bar_width == 0 {
        return widths;
    }
    let total: usize = counts.iter().sum();
    if total == 0 {
        widths[4] = bar_width;
        return widths;
    }

    let mut remainders = [0.0f64; 5];
    for i in 0..5 {
        if counts[i] == 0 {
            continue;
        }
        let exact = counts[i] as f64 * bar_width as f64 / total as f64;
        let floor = exact.floor() as usize;
        widths[i] = floor.max(1);
        remainders[i] = exact - floor as f64;
    }

    while widths.iter().sum::<usize>() > bar_width {
        if let Some((idx, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, &w)| w > 1)
            .max_by_key(|(_, &w)| w)
        {
            widths[idx] -= 1;
        } else if let Some((idx, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, &w)| w > 0)
            .max_by_key(|(_, &w)| w)
        {
            widths[idx] -= 1;
        } else {
            break;
        }
    }

    let mut deficit = bar_width.saturating_sub(widths.iter().sum::<usize>());
    if deficit > 0 {
        let mut order: Vec<usize> = (0..5).filter(|&i| counts[i] > 0).collect();
        order.sort_by(|&a, &b| {
            remainders[b]
                .partial_cmp(&remainders[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| counts[b].cmp(&counts[a]))
        });
        let mut cursor = 0usize;
        while deficit > 0 && !order.is_empty() {
            let idx = order[cursor % order.len()];
            widths[idx] += 1;
            deficit -= 1;
            cursor += 1;
        }
    }

    widths
}

/// Returns the Archival Receipt top border (`╭─ ia-get · <id> ─── <badge> ─╮`) at exact `card_width`.
fn build_receipt_top_border(state: &UiState, card_width: usize, theme: &Theme) -> Line<'static> {
    let glyphs = theme.glyphs();
    let (badge_text, badge_style) = if state.interrupted {
        (
            format!("{} interrupted", glyphs.interrupted),
            theme.bold_if_color(theme.warn_style()),
        )
    } else if state.fail_count > 0 {
        (
            format!("{} {} failed", glyphs.fail, state.fail_count),
            theme.bold_if_color(theme.err_style()),
        )
    } else if state.warn_count > 0 {
        let label = if state.warn_count == 1 {
            "warning"
        } else {
            "warnings"
        };
        (
            format!("{} {} {label}", glyphs.warn, state.warn_count),
            theme.bold_if_color(theme.warn_style()),
        )
    } else {
        (
            format!("{} complete", glyphs.ok),
            theme.bold_if_color(theme.ok_style()),
        )
    };

    // Frame pieces: `╭─ ` (3) + left + ` ` (1) + fill (>=1) + ` ` (1) + badge + ` ─╮` (3) = 8 + fill + left + badge.
    let max_badge = card_width.saturating_sub(10);
    let badge_fit = tail_truncate(&badge_text, max_badge);
    let badge_w = badge_fit.width();

    let left_budget = card_width.saturating_sub(9 + badge_w);
    let prefix = format!("ia-get {} ", glyphs.sep);
    let prefix_w = prefix.width();

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let left_w = if left_budget > prefix_w {
        let id_fit = middle_truncate(&state.identifier, left_budget - prefix_w, glyphs.ellipsis);
        let id_w = id_fit.width();
        left_spans.push(Span::styled("ia-get", theme.title_style()));
        left_spans.push(Span::styled(
            format!(" {} ", glyphs.sep),
            theme.muted_style(),
        ));
        left_spans.push(Span::styled(id_fit, theme.primary_style()));
        prefix_w + id_w
    } else if left_budget > 0 {
        let short = tail_truncate("ia-get", left_budget);
        let w = short.width();
        left_spans.push(Span::styled(short, theme.title_style()));
        w
    } else {
        0
    };

    let fill_count = card_width.saturating_sub(8 + left_w + badge_w);
    let mut spans = Vec::with_capacity(left_spans.len() + 8);
    spans.push(Span::styled(
        format!("{}{} ", glyphs.top_left, glyphs.horizontal),
        theme.border_style(),
    ));
    spans.extend(left_spans);
    spans.push(Span::raw(" "));
    if fill_count > 0 {
        spans.push(Span::styled(
            glyphs.horizontal.repeat(fill_count),
            theme.border_style(),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(badge_fit, badge_style));
    spans.push(Span::styled(
        format!(" {}{}", glyphs.horizontal, glyphs.top_right),
        theme.border_style(),
    ));
    Line::from(spans)
}

/// Builds Row 1 of the Archival Receipt: proportional segmented outcome bar + `<complete>/<total>  <pct>%`.
fn build_receipt_row_1(state: &UiState, inner_width: usize, theme: &Theme) -> Line<'static> {
    let glyphs = theme.glyphs();
    let hash_mismatches = state.hash_mismatch_count();
    let complete = state.ok_count.saturating_add(state.skip_count);
    let accounted = complete
        .saturating_add(hash_mismatches)
        .saturating_add(state.fail_count);
    let total = state.total_files.max(accounted);
    let unstarted = total.saturating_sub(accounted);
    let pct = if total == 0 {
        100u8
    } else {
        ((complete as f64 / total as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };

    let right_label = format!("{complete}/{total}  {pct}%");
    let right_w = right_label.width();
    let bar_width = inner_width.saturating_sub(right_w + 2);

    let widths = allocate_outcome_bar_widths(
        [
            state.ok_count,
            state.skip_count,
            hash_mismatches,
            state.fail_count,
            unstarted,
        ],
        bar_width,
    );

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(widths[0] + 6);
    if widths[0] > 0 {
        match theme.color_level {
            ColorLevel::TrueColor => {
                for cell in 0..widths[0] {
                    spans.push(Span::styled(
                        glyphs.bar_ok,
                        theme.bar_fill_style(cell, widths[0], false),
                    ));
                }
            }
            ColorLevel::Ansi16 => {
                spans.push(Span::styled(
                    glyphs.bar_ok.repeat(widths[0]),
                    theme.ok_style(),
                ));
            }
            ColorLevel::Monochrome => {
                spans.push(Span::styled(
                    glyphs.bar_ok.repeat(widths[0]),
                    Style::default(),
                ));
            }
        }
    }
    if widths[1] > 0 {
        spans.push(Span::styled(
            glyphs.bar_skip.repeat(widths[1]),
            theme.muted_style(),
        ));
    }
    if widths[2] > 0 {
        spans.push(Span::styled(
            glyphs.bar_warn.repeat(widths[2]),
            theme.warn_style(),
        ));
    }
    if widths[3] > 0 {
        spans.push(Span::styled(
            glyphs.bar_fail.repeat(widths[3]),
            theme.err_style(),
        ));
    }
    if widths[4] > 0 {
        spans.push(Span::styled(
            glyphs.bar_empty.repeat(widths[4]),
            theme.track_style(),
        ));
    }

    let used_bar: usize = widths.iter().sum();
    let pad = inner_width.saturating_sub(used_bar + right_w);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.push(Span::styled(right_label, theme.primary_style()));

    frame_receipt_row(spans, inner_width, theme)
}

/// Builds Row 2 of the Archival Receipt: outcome counts on left, downsampled sparkline + avg speed on right.
fn build_receipt_row_2(
    state: &UiState,
    card_width: usize,
    inner_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let mut left_w = 0usize;

    let ok_rest = format!(" {} ok", state.ok_count);
    left_w += glyphs.ok.width() + ok_rest.width();
    left_spans.push(Span::styled(
        glyphs.ok,
        theme.bold_if_color(theme.ok_style()),
    ));
    left_spans.push(Span::styled(ok_rest, theme.ok_style()));

    if state.skip_count > 0 {
        let skip_text = format!("  {} {} skip", glyphs.skip, state.skip_count);
        left_w += skip_text.width();
        left_spans.push(Span::styled(skip_text, theme.muted_style()));
    }
    if state.warn_count > 0 {
        let warn_rest = format!(" {} warn", state.warn_count);
        left_w += 2 + glyphs.warn.width() + warn_rest.width();
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            glyphs.warn,
            theme.bold_if_color(theme.warn_style()),
        ));
        left_spans.push(Span::styled(warn_rest, theme.warn_style()));
    }
    if state.fail_count > 0 {
        let fail_rest = format!(" {} fail", state.fail_count);
        left_w += 2 + glyphs.fail.width() + fail_rest.width();
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            glyphs.fail,
            theme.bold_if_color(theme.err_style()),
        ));
        left_spans.push(Span::styled(fail_rest, theme.err_style()));
    }

    // Show full-run downsampled 12-sample sparkline + average speed on >= 64 cols,
    // and keep compact `speed <avg>` visible whenever it fits (including 60..=63 cols).
    let avg_str = format_speed(state.average_speed_bps());
    let compact_right_w = "speed ".len() + avg_str.width();
    let mut rendered_sparkline = false;
    if card_width >= 64 {
        let samples = state.downsampled_run_sparkline();
        let (spark_spans, spark_w) = build_sparkline_spans_from_samples(samples.into_iter(), theme);
        let full_right_w = "speed ".len() + spark_w + 2 + avg_str.width();
        if left_w + 2 + full_right_w <= inner_width {
            let pad = inner_width - left_w - full_right_w;
            left_spans.push(Span::raw(" ".repeat(pad)));
            left_spans.push(Span::styled("speed ", theme.muted_style()));
            left_spans.extend(spark_spans);
            left_spans.push(Span::raw("  "));
            left_spans.push(Span::styled(avg_str.clone(), theme.primary_style()));
            rendered_sparkline = true;
        }
    }
    if !rendered_sparkline && left_w + 2 + compact_right_w <= inner_width {
        let pad = inner_width - left_w - compact_right_w;
        left_spans.push(Span::raw(" ".repeat(pad)));
        left_spans.push(Span::styled("speed ", theme.muted_style()));
        left_spans.push(Span::styled(avg_str, theme.primary_style()));
    }

    frame_receipt_row(left_spans, inner_width, theme)
}

/// Builds Row 3 (or Rows 3a + 3b when wrapped at <= 60 cols) of the Archival Receipt:
/// `data <bytes>   time <elapsed>   peak <peak>` and `dir <middle-truncated output_dir>`.
fn build_receipt_row_3_lines(
    state: &UiState,
    card_width: usize,
    inner_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let glyphs = theme.glyphs();
    let data_val = format_size(state.total_bytes_downloaded());
    let time_val = format_duration(state.elapsed());
    let peak_val = format_speed(state.peak_bytes_per_sec);

    let make_metrics = |gap: &str, include_peak: bool| -> (Vec<Span<'static>>, usize) {
        let mut spans = vec![
            Span::styled("data ", theme.muted_style()),
            Span::styled(data_val.clone(), theme.primary_style()),
            Span::raw(gap.to_string()),
            Span::styled("time ", theme.muted_style()),
            Span::styled(time_val.clone(), theme.primary_style()),
        ];
        let mut w = "data ".len() + data_val.width() + gap.len() + "time ".len() + time_val.width();
        if include_peak {
            spans.push(Span::raw(gap.to_string()));
            spans.push(Span::styled("peak ", theme.muted_style()));
            spans.push(Span::styled(peak_val.clone(), theme.primary_style()));
            w += gap.len() + "peak ".len() + peak_val.width();
        }
        (spans, w)
    };

    let (metrics_spans, metrics_w) = make_metrics("   ", true);
    let min_dir_display = state.output_dir.width().min(12);
    let single_row_needed = metrics_w + 3 + "dir ".len() + min_dir_display;

    if card_width > 60 && single_row_needed <= inner_width {
        let dir_budget = inner_width.saturating_sub(metrics_w + 3 + "dir ".len());
        let dir_trunc = middle_truncate(&state.output_dir, dir_budget, glyphs.ellipsis);
        let dir_w = "dir ".len() + dir_trunc.width();
        let pad = inner_width.saturating_sub(metrics_w + dir_w);

        let mut spans = metrics_spans;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled("dir ", theme.muted_style()));
        spans.push(Span::styled(dir_trunc, theme.primary_style()));
        vec![frame_receipt_row(spans, inner_width, theme)]
    } else {
        // Wrap cleanly onto two inner rows so narrow receipts (<= 60 cols) never overflow.
        let metrics_row = if metrics_w <= inner_width {
            metrics_spans
        } else {
            let (m2, w2) = make_metrics("  ", true);
            if w2 <= inner_width {
                m2
            } else {
                let (m1, w1) = make_metrics(" ", true);
                if w1 <= inner_width {
                    m1
                } else {
                    make_metrics("  ", false).0
                }
            }
        };

        let dir_budget = inner_width.saturating_sub("dir ".len());
        let dir_trunc = middle_truncate(&state.output_dir, dir_budget, glyphs.ellipsis);
        let dir_row = vec![
            Span::styled("dir ", theme.muted_style()),
            Span::styled(dir_trunc, theme.primary_style()),
        ];

        vec![
            frame_receipt_row(metrics_row, inner_width, theme),
            frame_receipt_row(dir_row, inner_width, theme),
        ]
    }
}

/// Builds the rounded Design 1 Archival Receipt card lines for an interactive TTY exit.
///
/// Every returned line has exact display width `width as usize` (when `width >= 16`).
pub fn build_archival_receipt_lines(
    state: &UiState,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let card_width = (width as usize).max(16);
    let inner_width = card_width.saturating_sub(4);
    let glyphs = theme.glyphs();

    let mut lines = Vec::with_capacity(6);
    lines.push(build_receipt_top_border(state, card_width, theme));
    lines.push(build_receipt_row_1(state, inner_width, theme));
    lines.push(build_receipt_row_2(state, card_width, inner_width, theme));
    lines.extend(build_receipt_row_3_lines(
        state,
        card_width,
        inner_width,
        theme,
    ));
    lines.push(Line::from(Span::styled(
        format!(
            "{}{}{}",
            glyphs.bottom_left,
            glyphs.horizontal.repeat(card_width.saturating_sub(2)),
            glyphs.bottom_right
        ),
        theme.border_style(),
    )));
    lines
}

/// Returns up to `max_chars` leading characters of a hex digest without splitting UTF-8.
fn short_hash(digest: &str, max_chars: usize) -> String {
    digest.chars().take(max_chars).collect()
}

/// Clips a line's spans in place so total display width never exceeds `max_width` (without padding).
fn clip_line_to_max_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    let mut out = Vec::with_capacity(line.spans.len());
    let mut used = 0usize;
    for span in line.spans {
        if used >= max_width {
            break;
        }
        let w = span.content.width();
        if used + w <= max_width {
            used += w;
            out.push(span);
        } else {
            let clipped = tail_truncate(&span.content, max_width - used);
            if !clipped.is_empty() {
                out.push(Span::styled(clipped, span.style));
            }
            break;
        }
    }
    Line::from(out)
}

/// Appends a copy-clean `┃` ledger group header (wrapping an optional explanatory note onto a
/// second indented `┃` line whenever `title + note` would exceed `max_width`).
#[allow(clippy::too_many_arguments)]
fn push_ledger_group_header(
    out: &mut Vec<Line<'static>>,
    bar: &'static str,
    bar_style: Style,
    icon: &'static str,
    title: String,
    note: Option<&'static str>,
    max_width: usize,
    theme: &Theme,
) {
    let glyphs = theme.glyphs();
    let icon_style = theme.bold_if_color(bar_style);
    let prefix_w = bar.width() + 1 + icon.width() + 1; // "┃ ! "
    if let Some(note_text) = note {
        let sep_part = format!(" {} ", glyphs.sep);
        let combined_w = prefix_w + title.width() + sep_part.width() + note_text.width();
        if combined_w <= max_width {
            out.push(Line::from(vec![
                Span::styled(bar, bar_style),
                Span::raw(" "),
                Span::styled(icon, icon_style),
                Span::raw(" "),
                Span::styled(title, theme.bold_style()),
                Span::styled(sep_part, theme.muted_style()),
                Span::styled(note_text, theme.muted_style()),
            ]));
            return;
        }
        let title_budget = max_width.saturating_sub(prefix_w);
        out.push(Line::from(vec![
            Span::styled(bar, bar_style),
            Span::raw(" "),
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(tail_truncate(&title, title_budget), theme.bold_style()),
        ]));
        let note_prefix = if max_width >= 48 { "   " } else { " " };
        let note_budget = max_width.saturating_sub(bar.width() + note_prefix.len());
        out.push(Line::from(vec![
            Span::styled(bar, bar_style),
            Span::raw(note_prefix),
            Span::styled(tail_truncate(note_text, note_budget), theme.muted_style()),
        ]));
    } else {
        let title_budget = max_width.saturating_sub(prefix_w);
        out.push(Line::from(vec![
            Span::styled(bar, bar_style),
            Span::raw(" "),
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(tail_truncate(&title, title_budget), theme.bold_style()),
        ]));
    }
}

/// Builds the 3-tier adaptive copy-clean (`┃`) exception ledger rendered below the Archival Receipt.
///
/// Returns an empty `Vec` when there are no warnings, failures, or skipped private files.
/// Never draws a right border so users can drag-select or triple-click filenames and MD5 hashes.
pub fn build_exception_ledger_lines(
    state: &UiState,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    if state.attention_items.is_empty() || width == 0 {
        return Vec::new();
    }

    let mut errors = Vec::new();
    let mut mismatches = Vec::new();
    let mut skipped_private = 0usize;
    for item in &state.attention_items {
        match item {
            AttentionItem::TransferError { file_path, message } => {
                errors.push((file_path.as_str(), message.as_str()));
            }
            AttentionItem::HashMismatch {
                file_path,
                got,
                expected,
            } => {
                mismatches.push((file_path.as_str(), got.as_str(), expected.as_str()));
            }
            AttentionItem::SkippedPrivate { count } => {
                skipped_private += *count;
            }
        }
    }

    let total_attention = errors.len() + mismatches.len() + usize::from(skipped_private > 0);
    let expanded = total_attention <= 3;
    let max_width = width as usize;
    let glyphs = theme.glyphs();
    let bar = glyphs.ledger_bar;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // 1. Transfer failures (crimson ✗)
    if !errors.is_empty() {
        let style = theme.err_style();
        let icon_style = theme.bold_if_color(style);
        push_ledger_group_header(
            &mut lines,
            bar,
            style,
            glyphs.fail,
            format!("Transfer failures ({})", errors.len()),
            None,
            max_width,
            theme,
        );

        if expanded {
            let name_indent = if max_width >= 48 { "   " } else { " " };
            let msg_indent = if max_width >= 48 { "     " } else { "   " };
            for (file_path, message) in &errors {
                let name_budget = max_width
                    .saturating_sub(bar.width() + name_indent.len() + glyphs.fail.width() + 1);
                let name_fit = middle_truncate(file_path, name_budget, glyphs.ellipsis);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(name_indent),
                    Span::styled(glyphs.fail, icon_style),
                    Span::raw(" "),
                    Span::styled(name_fit, theme.bold_style()),
                ]));

                let msg_budget = max_width.saturating_sub(bar.width() + msg_indent.len());
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(msg_indent),
                    Span::styled(tail_truncate(message, msg_budget), theme.muted_style()),
                ]));
            }
        } else {
            let shown = if errors.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                LEDGER_CAPPED_ROWS
            } else {
                errors.len()
            };
            let indent = if max_width >= 48 { "   " } else { " " };
            let prefix_w = bar.width() + indent.len();
            let row_budget = max_width.saturating_sub(prefix_w);
            let avail = row_budget.saturating_sub(2);
            let msg_cap = avail / 2;
            let max_shown_msg_w = errors[..shown]
                .iter()
                .map(|(_, msg)| msg.width())
                .max()
                .unwrap_or(0);
            let msg_col_w = max_shown_msg_w.min(msg_cap);
            let name_budget = avail.saturating_sub(msg_col_w);
            let max_shown_name_w = errors[..shown]
                .iter()
                .map(|(file_path, _)| file_path.width())
                .max()
                .unwrap_or(0);
            let name_col_w = max_shown_name_w.min(name_budget);

            for (file_path, message) in &errors[..shown] {
                if max_width > prefix_w && msg_col_w > 0 && name_col_w > 0 {
                    let msg_fit = tail_truncate(message, msg_col_w);
                    let name_fit = pad_right(
                        &middle_truncate(file_path, name_col_w, glyphs.ellipsis),
                        name_col_w,
                    );
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                        Span::raw("  "),
                        Span::styled(msg_fit, style),
                    ]));
                } else if max_width > prefix_w {
                    let name_fit = middle_truncate(file_path, row_budget, glyphs.ellipsis);
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        tail_truncate(bar, max_width),
                        style,
                    )));
                }
            }
            if errors.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                let more = errors.len() - LEDGER_CAPPED_ROWS;
                let note = format!("+{more} more, re-run with -v for the full list");
                let budget = max_width.saturating_sub(prefix_w);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(tail_truncate(&note, budget), theme.muted_style()),
                ]));
            }
        }
    }

    // 2. MD5 mismatches (amber !)
    if !mismatches.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let style = theme.warn_style();
        let icon_style = theme.bold_if_color(style);
        push_ledger_group_header(
            &mut lines,
            bar,
            style,
            glyphs.warn,
            format!("MD5 mismatches ({})", mismatches.len()),
            Some("archive.org metadata can be stale, .part kept"),
            max_width,
            theme,
        );

        if expanded {
            let name_indent = if max_width >= 48 { "   " } else { " " };
            let hash_indent = if max_width >= 48 { "     " } else { " " };
            for (file_path, got, expected) in &mismatches {
                let name_budget = max_width
                    .saturating_sub(bar.width() + name_indent.len() + glyphs.warn.width() + 1);
                let name_fit = middle_truncate(file_path, name_budget, glyphs.ellipsis);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(name_indent),
                    Span::styled(glyphs.warn, icon_style),
                    Span::raw(" "),
                    Span::styled(name_fit, theme.bold_style()),
                ]));

                let hash_budget = max_width.saturating_sub(bar.width() + hash_indent.len() + 9);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(hash_indent),
                    Span::styled("got      ", theme.muted_style()),
                    Span::styled(tail_truncate(got, hash_budget), theme.accent_style()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(hash_indent),
                    Span::styled("expected ", theme.muted_style()),
                    Span::styled(tail_truncate(expected, hash_budget), theme.muted_style()),
                ]));
            }
        } else {
            let shown = if mismatches.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                LEDGER_CAPPED_ROWS
            } else {
                mismatches.len()
            };
            let show_part_tag = max_width >= 68;
            let indent = if max_width >= 48 { "   " } else { " " };
            let prefix_w = bar.width() + indent.len();
            let row_budget = max_width.saturating_sub(prefix_w);
            let part_suffix = if show_part_tag { "  (.part)" } else { "" };

            let max_hash_w = mismatches[..shown]
                .iter()
                .map(|(_, got, expected)| {
                    short_hash(got, 8).width()
                        + 1
                        + glyphs.not_equal.width()
                        + 1
                        + short_hash(expected, 8).width()
                        + part_suffix.width()
                })
                .max()
                .unwrap_or(0);
            let name_budget = row_budget.saturating_sub(2 + max_hash_w);
            let max_shown_name_w = mismatches[..shown]
                .iter()
                .map(|(file_path, _, _)| file_path.width())
                .max()
                .unwrap_or(0);
            let name_col_w = max_shown_name_w.min(name_budget);

            for (file_path, got, expected) in &mismatches[..shown] {
                if max_width > prefix_w && name_col_w > 0 && row_budget > 2 + max_hash_w {
                    let got_short = short_hash(got, 8);
                    let exp_short = short_hash(expected, 8);
                    let name_fit = pad_right(
                        &middle_truncate(file_path, name_col_w, glyphs.ellipsis),
                        name_col_w,
                    );
                    let mut row_spans = vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                        Span::raw("  "),
                        Span::styled(got_short, theme.accent_style()),
                        Span::styled(format!(" {} ", glyphs.not_equal), style),
                        Span::styled(exp_short, theme.muted_style()),
                    ];
                    if show_part_tag {
                        row_spans.push(Span::styled("  (.part)", theme.muted_style()));
                    }
                    lines.push(Line::from(row_spans));
                } else if max_width > prefix_w {
                    let name_fit = middle_truncate(file_path, row_budget, glyphs.ellipsis);
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        tail_truncate(bar, max_width),
                        style,
                    )));
                }
            }
            if mismatches.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                let more = mismatches.len() - LEDGER_CAPPED_ROWS;
                let note = format!("+{more} more, re-run with -v for the full list");
                let budget = max_width.saturating_sub(prefix_w);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(tail_truncate(&note, budget), theme.muted_style()),
                ]));
            }
        }
    }

    // 3. Skipped private (muted stone ↷)
    if skipped_private > 0 {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let style = theme.muted_style();
        push_ledger_group_header(
            &mut lines,
            bar,
            style,
            glyphs.skip,
            format!("Skipped private ({skipped_private})"),
            Some("archive.org access restricted"),
            max_width,
            theme,
        );
        let indent = if max_width >= 48 { "   " } else { " " };
        let file_label = if skipped_private == 1 {
            "file"
        } else {
            "files"
        };
        let detail = format!(
            "{skipped_private} private {file_label} skipped (supply -b/--cookies to download)"
        );
        let budget = max_width.saturating_sub(bar.width() + indent.len());
        lines.push(Line::from(vec![
            Span::styled(bar, style),
            Span::raw(indent),
            Span::styled(tail_truncate(&detail, budget), theme.muted_style()),
        ]));
    }

    lines
        .into_iter()
        .map(|line| clip_line_to_max_width(line, max_width))
        .collect()
}

/// Builds the complete interactive TTY post-run output: Archival Receipt card plus optional
/// `┃` exception ledger (separated by one blank line).
pub fn build_post_run_tty_lines(state: &UiState, theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = build_archival_receipt_lines(state, theme, width);
    if state.interrupted {
        let hint = "Run the command again to resume remaining files.";
        lines.push(Line::from(Span::styled(
            tail_truncate(hint, width as usize),
            theme.muted_style(),
        )));
    }
    let ledger = build_exception_ledger_lines(state, theme, width);
    if !ledger.is_empty() {
        lines.push(Line::default());
        lines.extend(ledger);
    }
    lines
}

/// Converts styled Ratatui lines to a newline-joined ANSI string (zero ANSI when monochrome).
pub fn render_lines_to_ansi(lines: &[Line<'static>], theme: &Theme) -> String {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        for span in &line.spans {
            out.push_str(&style_ratatui_to_ansi(&span.content, span.style, theme));
        }
    }
    out
}

/// Extracts plain unstyled text rows from Ratatui lines (useful for display-width verification).
pub fn render_lines_to_plain(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let mut s = String::new();
            for span in &line.spans {
                s.push_str(&span.content);
            }
            s
        })
        .collect()
}

/// Formats the interactive TTY post-run Archival Receipt and adaptive exception ledger as a string.
pub fn format_tty_summary(state: &UiState, theme: &Theme, width: u16) -> String {
    let lines = build_post_run_tty_lines(state, theme, width);
    render_lines_to_ansi(&lines, theme)
}

/// Formats the calm post-run plain-text summary and uncapped `Needs attention` section (non-TTY).
pub fn format_final_summary(state: &UiState, theme: &Theme) -> String {
    let glyphs = theme.glyphs();
    let mut out = String::new();

    let sep = format!(" {} ", style_muted(glyphs.sep, theme));

    let mut segments = vec![
        format!("{} ok", style_ok(&state.ok_count.to_string(), theme)),
        format!(
            "{} skipped",
            style_muted(&state.skip_count.to_string(), theme)
        ),
    ];
    if state.warn_count > 0 {
        let label = if state.warn_count == 1 {
            "warning"
        } else {
            "warnings"
        };
        segments.push(style_warn(&format!("{} {label}", state.warn_count), theme));
    }
    if state.fail_count > 0 {
        segments.push(style_err(&format!("{} failed", state.fail_count), theme));
    }
    let joined = segments.join(&sep);

    let bold_ok_icon =
        style_ratatui_to_ansi(glyphs.ok, theme.bold_if_color(theme.ok_style()), theme);
    let bold_warn_icon =
        style_ratatui_to_ansi(glyphs.warn, theme.bold_if_color(theme.warn_style()), theme);
    let bold_fail_icon =
        style_ratatui_to_ansi(glyphs.fail, theme.bold_if_color(theme.err_style()), theme);

    if state.interrupted {
        out.push_str(&format!(
            "{bold_fail_icon} {} {joined}. Run the command again to resume remaining files.",
            style_bold("Interrupted:", theme),
        ));
    } else {
        let icon = if state.fail_count > 0 {
            &bold_fail_icon
        } else if state.warn_count > 0 {
            &bold_warn_icon
        } else {
            &bold_ok_icon
        };
        out.push_str(&format!(
            "{icon} {} {joined}",
            style_bold("Completed:", theme)
        ));
    }

    if state.attention_items.is_empty() {
        return out;
    }

    let mut mismatches = Vec::new();
    let mut skipped_private = 0usize;
    let mut errors = Vec::new();
    for item in &state.attention_items {
        match item {
            AttentionItem::HashMismatch {
                file_path,
                got,
                expected,
            } => mismatches.push((file_path.as_str(), got.as_str(), expected.as_str())),
            AttentionItem::SkippedPrivate { count } => skipped_private += *count,
            AttentionItem::TransferError { file_path, message } => {
                errors.push((file_path.as_str(), message.as_str()));
            }
        }
    }

    out.push_str(&format!("\n\n{}", style_bold("Needs attention:", theme)));

    if !mismatches.is_empty() {
        out.push_str(&format!(
            "\n  {} {}",
            style_warn("MD5 mismatches", theme),
            style_muted(
                "(archive.org metadata can be stale; .part files kept unpromoted):",
                theme
            )
        ));
        for (file_path, got, expected) in mismatches {
            out.push_str(&format!(
                "\n    {bold_warn_icon} {}",
                style_bold(file_path, theme)
            ));
            out.push_str(&format!(
                "\n      {} {}",
                style_muted("got     ", theme),
                style_accent(got, theme)
            ));
            out.push_str(&format!(
                "\n      {} {}",
                style_muted("expected", theme),
                style_muted(expected, theme)
            ));
        }
    }

    if skipped_private > 0 {
        let file_label = if skipped_private == 1 {
            "file"
        } else {
            "files"
        };
        out.push_str(&format!(
            "\n  {}",
            style_warn("Skipped private files:", theme)
        ));
        out.push_str(&format!(
            "\n    {bold_warn_icon} {} private {file_label} skipped {}",
            style_bold(&skipped_private.to_string(), theme),
            style_muted("(supply -b/--cookies to download private files)", theme)
        ));
    }

    if !errors.is_empty() {
        out.push_str(&format!("\n  {}", style_err("Transfer errors:", theme)));
        for (file_path, message) in errors {
            out.push_str(&format!(
                "\n    {bold_fail_icon} {}: {message}",
                style_bold(file_path, theme)
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::downloader::SchedulerSnapshot;
    use crate::ui::state::UiEvent;

    fn buffer_to_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        let mut lines = Vec::with_capacity(area.height as usize);
        for y in 0..area.height {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            lines.push(row);
        }
        lines
    }

    fn render_snapshot_with_emoji(
        width: u16,
        height: u16,
        state: &UiState,
        theme: &Theme,
        use_region_emoji: bool,
    ) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        terminal
            .draw(|frame| render_instrument_with_emoji(frame, state, theme, use_region_emoji))
            .expect("draw instrument");
        terminal
    }

    fn render_snapshot(
        width: u16,
        height: u16,
        state: &UiState,
        theme: &Theme,
    ) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("create test terminal");
        terminal
            .draw(|frame| render_instrument(frame, state, theme))
            .expect("draw instrument");
        terminal
    }

    fn sample_mixed_state() -> UiState {
        let t0 = Instant::now();
        let mut state = UiState::new("deftributetozzap64", 18, 2_000_000_000, None, t0);
        state.scheduler = SchedulerSnapshot {
            in_flight_units: 4,
            effective_ceiling: 4,
            clean_successes: 0,
            cooldown_remaining: None,
        };
        state.ok_count = 10;
        state.skip_count = 2;
        state.warn_count = 1;
        state.fail_count = 1;
        state.finished_bytes = 900_000_000;
        state.total_display_ratio = 0.62;
        state.aggregate_speed_bps = 14_200_000.0;
        state.eta_secs = Some(64);
        for sample in [
            2_000_000, 4_000_000, 7_000_000, 10_000_000, 12_000_000, 15_000_000, 14_000_000,
            13_000_000, 16_000_000, 15_500_000, 14_800_000, 14_200_000,
        ] {
            state.record_spark_sample(sample);
        }

        // Slot 1: GET small file on US mirror
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "issue_001_cover.jpg".to_string(),
                permit_cost: 1,
                total_bytes: Some(2_400_000),
                mirror_host: Some("ia801504.us.archive.org".to_string()),
            },
            t0,
        );
        state.active_slots[0].display_ratio = 0.84;
        state.active_slots[0].target_ratio = 0.84;
        state.active_slots[0].speed_bps = 4_200_000.0;

        // Slot 2: RES .part resume on CA mirror
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 2,
                file_path: "zzap64_issue_001_600dpi.pdf".to_string(),
                permit_cost: 2,
                total_bytes: Some(42_000_000),
                mirror_host: Some("dn790007.ca.archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotResumeBaseline {
                slot_id: 2,
                start_offset: 22_000_000,
                total_bytes: Some(42_000_000),
            },
            t0,
        );
        state.active_slots[1].speed_bps = 6_800_000.0;

        // Slot 3: MD5 hashing on EU mirror
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 3,
                file_path: "日本語_archive_冊子_😀.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(5_000_000),
                mirror_host: Some("dn760105.eu.archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStage {
                slot_id: 3,
                stage: TransferStage::Md5,
                mirror_host: None,
                cooldown: None,
                retry: None,
            },
            t0,
        );
        state.active_slots[2].display_ratio = 0.91;
        state.active_slots[2].target_ratio = 0.91;

        state
    }

    #[test]
    fn middle_truncate_respects_unicode_width_for_ascii_cjk_combining_and_emoji() {
        let cases = [
            ("short.txt", 20, "…"),
            ("very_long_archive_filename_issue_001.pdf", 18, "…"),
            ("日本語の長いアーカイブファイル名_001.pdf", 16, "…"),
            ("emoji_😀🎉🚀_archive_scan.jp2", 16, "…"),
            ("cafe\u{0301}_archive_document_name.txt", 14, "…"),
            ("very_long_archive_filename_issue_001.pdf", 18, "..."),
            ("abcdef", 3, "..."),
            ("abcdef", 1, "…"),
            ("abcdef", 0, "…"),
        ];

        for (input, max_cells, ell) in cases {
            let truncated = middle_truncate(input, max_cells, ell);
            assert!(
                truncated.width() <= max_cells,
                "middle_truncate({input:?}, {max_cells}, {ell:?}) = {truncated:?} exceeded width {} > {max_cells}",
                truncated.width()
            );
            if input.width() <= max_cells {
                assert_eq!(truncated, input);
            }
        }
    }

    #[test]
    fn test_backend_renders_120x30_and_80x24_full_instrument_with_cjk_resume_md5_and_counters() {
        let state = sample_mixed_state();
        let theme = Theme::archival_truecolor();
        let expected_us = if theme.use_region_flag_emoji() {
            "🇺🇸"
        } else {
            "US"
        };
        let expected_ca = if theme.use_region_flag_emoji() {
            "🇨🇦"
        } else {
            "CA"
        };

        for (w, h) in [(120, 30), (80, 24)] {
            let term = render_snapshot(w, h, &state, &theme);
            let lines = buffer_to_lines(&term);
            let joined = lines.join("\n");

            assert!(
                lines[0].starts_with('╭'),
                "{w}x{h} missing rounded top-left:\n{joined}"
            );
            assert!(
                lines[7].starts_with('╰'),
                "{w}x{h} missing rounded bottom-left:\n{joined}"
            );
            assert!(
                lines[1].contains("ia-get · deftributetozzap64"),
                "{w}x{h} header:\n{joined}"
            );
            assert!(
                lines[1].contains("14/18 files"),
                "{w}x{h} file progress:\n{joined}"
            );
            assert!(
                !lines[1].contains("4/4") && !lines[1].contains("[██"),
                "{w}x{h} permit gauge must be removed:\n{joined}"
            );
            assert!(lines[2].contains("TOTAL"), "{w}x{h} TOTAL row:\n{joined}");
            assert!(lines[2].contains("10 ok"), "{w}x{h} ok counter:\n{joined}");
            assert!(
                lines[2].contains("! 1 warn"),
                "{w}x{h} warn counter:\n{joined}"
            );
            assert!(
                lines[2].contains("✗ 1 fail"),
                "{w}x{h} fail counter:\n{joined}"
            );
            assert!(
                lines[3].contains("│ ▼  ") && !lines[3].contains("1u"),
                "{w}x{h} GET glyph without 1u prefix:\n{joined}"
            );
            assert!(
                lines[3].contains(expected_us) && !lines[3].contains("ia801504"),
                "{w}x{h} US mirror region badge:\n{joined}"
            );
            assert!(
                lines[4].contains("│ ►  ") && !lines[4].contains("2u"),
                "{w}x{h} RESUME glyph without 2u prefix:\n{joined}"
            );
            assert!(
                lines[4].contains(expected_ca) && !lines[4].contains("dn790007"),
                "{w}x{h} CA mirror region badge:\n{joined}"
            );
            assert!(
                !lines[4].contains("(.part)"),
                "{w}x{h} redundant .part tag must be omitted:\n{joined}"
            );
            assert!(
                lines[4].contains("zzap64"),
                "{w}x{h} resumed filename missing:\n{joined}"
            );
            assert!(
                lines[5].contains("│ ◐  ") && !lines[5].contains("1u"),
                "{w}x{h} MD5 glyph without 1u prefix:\n{joined}"
            );
            assert!(
                lines[5].contains("verifying MD5"),
                "{w}x{h} verifying MD5 label:\n{joined}"
            );
        }
    }

    #[test]
    fn test_backend_renders_79x23_and_60x20_compact_hiding_mirror_and_sparkline() {
        let state = sample_mixed_state();
        let theme = Theme::archival_truecolor();

        for (w, h) in [(79, 23), (60, 20)] {
            let term = render_snapshot(w, h, &state, &theme);
            let lines = buffer_to_lines(&term);
            let joined = lines.join("\n");

            assert!(lines[0].starts_with('╭'), "{w}x{h} top border:\n{joined}");
            assert!(
                lines[1].contains("14/18 files"),
                "{w}x{h} file progress:\n{joined}"
            );
            assert!(
                !lines[1].contains("4/4"),
                "{w}x{h} permit gauge must be removed:\n{joined}"
            );
            // Sparkline and mirror region badge must be hidden below 80 columns.
            assert!(
                !lines[1].contains('▁') && !lines[1].contains('▆'),
                "{w}x{h} sparkline must hide:\n{joined}"
            );
            assert!(
                !lines[3].contains("🇺🇸") && !lines[3].contains("  US"),
                "{w}x{h} mirror region badge must hide:\n{joined}"
            );
            assert!(lines[3].contains("│ ▼  "), "{w}x{h} slot 1:\n{joined}");
            assert!(lines[4].contains("│ ►  "), "{w}x{h} slot 2:\n{joined}");
            assert!(
                lines[5].contains(" verify MD5"),
                "{w}x{h} slot 3 verify MD5:\n{joined}"
            );

            // Narrow MD5 progress bar must align vertically with sibling speed-slot bars
            // in actual terminal cell columns (accounting for 2-cell CJK/emoji glyphs).
            let buf = term.backend().buffer();
            let bar_start_x = |y: u16| -> u16 {
                (0..w)
                    .find(|&x| matches!(buf[(x, y)].symbol(), "█" | "─"))
                    .expect("missing bar cell")
            };
            let col1 = bar_start_x(3);
            let col2 = bar_start_x(4);
            let col3 = bar_start_x(5);
            assert_eq!(
                (col1, col2, col3),
                (col1, col1, col1),
                "{w}x{h} MD5 bar must align vertically with sibling bars:\n{joined}"
            );
        }
    }

    #[test]
    fn test_backend_renders_44x7_collapsed_without_panic_or_border() {
        let state = sample_mixed_state();
        let theme = Theme::archival_truecolor();

        let term = render_snapshot(44, 7, &state, &theme);
        let lines = buffer_to_lines(&term);
        let joined = lines.join("\n");

        assert!(
            !lines[0].contains('╭'),
            "44x7 must collapse borderless:\n{joined}"
        );
        assert!(lines[0].contains("ia-get"), "44x7 line 1:\n{joined}");
        assert!(lines[1].contains("62%"), "44x7 line 2:\n{joined}");
        assert!(lines[1].contains("10 ok"), "44x7 ok count:\n{joined}");
        assert!(lines[1].contains("!1"), "44x7 warn count:\n{joined}");
        assert!(lines[1].contains("✗1"), "44x7 fail count:\n{joined}");
    }

    #[test]
    fn test_backend_renders_4_wide_2_wide_1_wide_and_429_cooldown_with_recovery_pips() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();

        // 4-wide small files
        let mut small = UiState::new("small-batch", 4, 4_000_000, None, t0);
        small.scheduler.in_flight_units = 4;
        for i in 0..4 {
            small.on_event(
                UiEvent::SlotStarted {
                    slot_id: i,
                    file_path: format!("small_{i}.jpg"),
                    permit_cost: 1,
                    total_bytes: Some(1_000_000),
                    mirror_host: Some("ia801504.us.archive.org".to_string()),
                },
                t0,
            );
        }
        let term = render_snapshot(80, 24, &small, &theme);
        let joined = buffer_to_lines(&term).join("\n");
        for i in 0..4 {
            assert!(
                joined.contains(&format!("small_{i}.jpg")),
                "missing small_{i}:\n{joined}"
            );
        }

        // 2-wide medium files
        let mut med = UiState::new("med-batch", 2, 40_000_000, None, t0);
        med.scheduler.in_flight_units = 4;
        for i in 0..2 {
            med.on_event(
                UiEvent::SlotStarted {
                    slot_id: i,
                    file_path: format!("medium_{i}.pdf"),
                    permit_cost: 2,
                    total_bytes: Some(20_000_000),
                    mirror_host: Some("ia801504.us.archive.org".to_string()),
                },
                t0,
            );
        }
        let term = render_snapshot(80, 24, &med, &theme);
        let joined = buffer_to_lines(&term).join("\n");
        assert!(
            joined.contains("│ ▼  ") && !joined.contains("2u"),
            "missing medium slots without 2u prefix:\n{joined}"
        );

        // 1-wide large file + 429 cooldown with recovery pips
        let mut throttled = UiState::new("throttled-item", 2, 200_000_000, None, t0);
        throttled.scheduler = SchedulerSnapshot {
            in_flight_units: 2,
            effective_ceiling: 2,
            clean_successes: 2,
            cooldown_remaining: Some(Duration::from_secs(8)),
        };
        throttled.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "huge_scan.zip".to_string(),
                permit_cost: 4,
                total_bytes: Some(200_000_000),
                mirror_host: Some("ia801504.us.archive.org".to_string()),
            },
            t0,
        );
        throttled.on_event(
            UiEvent::SlotStage {
                slot_id: 1,
                stage: TransferStage::RateLimited,
                mirror_host: None,
                cooldown: Some(Duration::from_secs(8)),
                retry: Some((1, 5)),
            },
            t0,
        );
        let term = render_snapshot(80, 24, &throttled, &theme);
        let joined = buffer_to_lines(&term).join("\n");
        assert!(
            joined.contains("│ ▲  ") && !joined.contains("4u"),
            "missing ▲ without 4u prefix:\n{joined}"
        );
        assert!(
            joined.contains("cooldown 08s"),
            "missing cooldown:\n{joined}"
        );
        assert!(joined.contains("ceiling 2/4"), "missing ceiling:\n{joined}");
        assert!(
            joined.contains("recovery ●●○○"),
            "missing recovery pips:\n{joined}"
        );
    }

    #[test]
    fn test_backend_no_color_and_ascii_fallback_emit_reset_styles_and_ascii_glyphs() {
        let state = sample_mixed_state();

        // NO_COLOR (monochrome Unicode): every cell has default fg colour and uses 2-letter region fallback.
        let mono_uni = Theme::monochrome_unicode();
        let term = render_snapshot(80, 24, &state, &mono_uni);
        let mono_lines = buffer_to_lines(&term);
        assert!(
            mono_lines[3].contains("  US") && !mono_lines[3].contains("🇺🇸"),
            "NO_COLOR must emit 2-letter US fallback, not emoji:\n{}",
            mono_lines.join("\n")
        );
        let buf = term.backend().buffer();
        for cell in buf.content() {
            assert_eq!(
                cell.fg,
                ratatui::style::Color::Reset,
                "NO_COLOR frame must not set fg colour on {:?}",
                cell.symbol()
            );
        }

        // ASCII fallback: zero non-ASCII frame/bar/icon glyphs on ASCII-filename state.
        let t0 = Instant::now();
        let mut ascii_state = UiState::new("zzap64", 1, 1000, None, t0);
        ascii_state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "issue_001.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(1000),
                mirror_host: Some("ia801504.us.archive.org".to_string()),
            },
            t0,
        );
        ascii_state.active_slots[0].display_ratio = 0.5;
        let mono_ascii = Theme::monochrome_ascii();
        let term = render_snapshot(80, 24, &ascii_state, &mono_ascii);
        let joined = buffer_to_lines(&term).join("\n");
        assert!(
            joined.is_ascii(),
            "ASCII fallback must emit pure ASCII:\n{joined}"
        );
        assert!(
            joined.starts_with('+'),
            "ASCII frame top-left must be '+':\n{joined}"
        );
        assert!(
            joined.contains("  US"),
            "ASCII fallback must emit 2-letter US region code:\n{joined}"
        );
    }

    #[test]
    fn final_summary_groups_mismatches_skipped_private_and_transfer_errors_calmly() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 5, 500, None, t0);
        state.ok_count = 2;
        state.skip_count = 1;
        state.on_event(UiEvent::SkippedPrivate { count: 2 }, t0);
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 1,
                file_path: "stale_meta.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: "1111aaaa".to_string(),
                    expected: "2222bbbb".to_string(),
                },
                finished_index: 4,
                total_files: 5,
            }),
            t0,
        );
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 2,
                file_path: "missing.bin".to_string(),
                kind: CompletionKind::Failed {
                    message: "HTTP error 404".to_string(),
                },
                finished_index: 5,
                total_files: 5,
            }),
            t0,
        );

        let summary = format_final_summary(&state, &Theme::monochrome_unicode());
        assert!(
            !summary.contains('\x1b'),
            "monochrome summary must have zero ANSI"
        );
        assert!(summary.contains("Completed: 2 ok · 1 skipped · 3 warnings · 1 failed"));
        assert!(summary.contains("Needs attention:"));
        assert!(summary.contains("archive.org metadata can be stale; .part files kept unpromoted"));
        assert!(summary.contains("stale_meta.pdf"));
        assert!(summary.contains("got      1111aaaa"));
        assert!(summary.contains("expected 2222bbbb"));
        assert!(summary
            .contains("2 private files skipped (supply -b/--cookies to download private files)"));
        assert!(summary.contains("missing.bin: HTTP error 404"));
    }

    #[test]
    fn test_backend_hides_zero_warn_fail_counters_and_shows_rate_cap_and_redirect_stage() {
        let t0 = Instant::now();
        let mut state = UiState::new("zzap64", 4, 4_000_000, Some(500 * 1024), t0);
        state.ok_count = 2;
        state.skip_count = 1;
        state.total_display_ratio = 0.75;
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "redirected_file.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(1_000_000),
                mirror_host: Some("archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStage {
                slot_id: 1,
                stage: TransferStage::Redirect,
                mirror_host: Some("ia801504.us.archive.org".to_string()),
                cooldown: None,
                retry: None,
            },
            t0,
        );

        let theme = Theme::archival_truecolor();
        let expected_us = if theme.use_region_flag_emoji() {
            "🇺🇸"
        } else {
            "US"
        };
        let term = render_snapshot(80, 24, &state, &theme);
        let lines = buffer_to_lines(&term);
        let joined = lines.join("\n");

        assert!(
            !lines[2].contains("warn"),
            "zero warn must stay hidden:\n{joined}"
        );
        assert!(
            !lines[2].contains("fail"),
            "zero fail must stay hidden:\n{joined}"
        );
        assert!(
            lines[2].contains("cap 500 KB/s"),
            "rate cap missing on TOTAL:\n{joined}"
        );
        assert!(
            lines[3].contains("│ »  ") && !lines[3].contains("1u"),
            "redirect glyph missing:\n{joined}"
        );
        assert!(
            lines[3].contains("redirect"),
            "redirect label missing:\n{joined}"
        );
        assert!(
            lines[3].contains(expected_us),
            "updated mirror region badge missing:\n{joined}"
        );
    }

    #[test]
    fn completion_line_renders_all_outcomes_plain_and_truncates_cjk() {
        let records = [
            CompletionRecord {
                slot_id: 1,
                file_path: "日本語の長いアーカイブファイル名_issue_001.pdf".to_string(),
                kind: CompletionKind::Downloaded {
                    bytes: 1_500_000,
                    duration: Duration::from_millis(420),
                    verified_md5: true,
                },
                finished_index: 1,
                total_files: 4,
            },
            CompletionRecord {
                slot_id: 2,
                file_path: "cached.pdf".to_string(),
                kind: CompletionKind::Skipped {
                    reason: SkipReason::AlreadyVerified,
                    bytes: 1000,
                },
                finished_index: 2,
                total_files: 4,
            },
            CompletionRecord {
                slot_id: 3,
                file_path: "stale.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: "aaaa".to_string(),
                    expected: "bbbb".to_string(),
                },
                finished_index: 3,
                total_files: 4,
            },
            CompletionRecord {
                slot_id: 4,
                file_path: "missing.bin".to_string(),
                kind: CompletionKind::Failed {
                    message: "HTTP error 404".to_string(),
                },
                finished_index: 4,
                total_files: 4,
            },
        ];

        let expected_prefixes = ["✓ ", "↷ ", "! ", "✗ "];
        let mono = Theme::monochrome_unicode();
        for (record, prefix) in records.iter().zip(expected_prefixes) {
            let plain = render_completion_line(record, &mono, None);
            assert!(
                plain.starts_with(prefix) && !plain.starts_with(&format!("{prefix} ")),
                "completion line must use a single space after icon: {plain:?}"
            );
            let bounded = render_completion_line(record, &mono, Some(48));
            assert!(
                bounded.width() <= 48,
                "bounded completion line exceeded 48 cols: {bounded:?}"
            );
        }
    }

    #[test]
    fn test_backend_zero_sparkline_samples_use_track_style_and_nonzero_use_accent() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();
        let mut state = UiState::new("spark-item", 4, 4_000_000, None, t0);
        state.aggregate_speed_bps = 1_500_000.0;
        for sample in [
            0, 0, 0, 0, 0, 0, 500_000, 1_000_000, 1_500_000, 1_200_000, 800_000, 1_500_000,
        ] {
            state.record_spark_sample(sample);
        }

        let term = render_snapshot(80, 24, &state, &theme);
        let buf = term.backend().buffer();
        let header_y = 1u16;
        let spark_chars = theme.glyphs().spark_blocks;
        let spark_xs: Vec<u16> = (0..80)
            .filter(|&x| spark_chars.contains(&buf[(x, header_y)].symbol()))
            .collect();
        assert_eq!(
            spark_xs.len(),
            12,
            "expected exactly 12 sparkline cells, found {:?}",
            spark_xs
        );
        let spark_slice = &spark_xs[..];
        let track_fg = theme.track_style().fg.expect("track fg");
        let accent_fg = theme.accent_style().fg.expect("accent fg");

        for &x in &spark_slice[..6] {
            assert_eq!(buf[(x, header_y)].symbol(), "▁");
            assert_eq!(
                buf[(x, header_y)].fg,
                track_fg,
                "zero sparkline sample at x={x} must use track_style"
            );
        }
        for &x in &spark_slice[6..] {
            assert_eq!(
                buf[(x, header_y)].fg,
                accent_fg,
                "non-zero sparkline sample at x={x} must use accent_style"
            );
        }

        // All-zero sparkline renders every sample in track_style.
        let idle = UiState::new("idle-item", 2, 2_000_000, None, t0);
        let idle_term = render_snapshot(80, 24, &idle, &theme);
        let idle_buf = idle_term.backend().buffer();
        let idle_xs: Vec<u16> = (0..80)
            .filter(|&x| idle_buf[(x, header_y)].symbol() == "▁")
            .collect();
        assert_eq!(idle_xs.len(), 12);
        for x in idle_xs {
            assert_eq!(idle_buf[(x, header_y)].fg, track_fg);
        }
    }

    #[test]
    fn test_backend_locks_total_bar_width_hides_zero_skip_and_preserves_warn_fail_on_narrow() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();

        let count_total_bar_cells = |lines: &[String]| -> usize {
            let row = &lines[2];
            let after_total = row.split_once("TOTAL ").expect("TOTAL prefix missing").1;
            after_total
                .chars()
                .take_while(|ch| !ch.is_ascii_whitespace())
                .count()
        };

        let mut clean = UiState::new("clean-item", 10, 1_000_000_000, None, t0);
        clean.ok_count = 3;
        clean.skip_count = 0;
        clean.finished_bytes = 300_000_000;
        clean.total_display_ratio = 0.30;
        clean.eta_secs = Some(90);

        let mut crowded = sample_mixed_state();
        crowded.ok_count = 999;
        crowded.skip_count = 42;
        crowded.warn_count = 12;
        crowded.fail_count = 7;

        for (w, h, expected_bar) in [
            (120, 30, 14),
            (80, 24, 14),
            (79, 23, 10),
            (60, 20, 10),
            (48, 8, 6),
        ] {
            let clean_lines = buffer_to_lines(&render_snapshot(w, h, &clean, &theme));
            let crowded_lines = buffer_to_lines(&render_snapshot(w, h, &crowded, &theme));
            assert_eq!(
                count_total_bar_cells(&clean_lines),
                expected_bar,
                "clean {w}x{h} TOTAL bar width:\n{}",
                clean_lines[2]
            );
            assert_eq!(
                count_total_bar_cells(&crowded_lines),
                expected_bar,
                "crowded {w}x{h} TOTAL bar width:\n{}",
                crowded_lines[2]
            );
            assert!(
                !clean_lines[2].contains("skip"),
                "clean {w}x{h} must hide 0 skip:\n{}",
                clean_lines[2]
            );
        }

        // At 48x8, bytes must drop before warn or fail counters.
        let mut narrow_warn = UiState::new("narrow-warn", 12, 2_000_000_000, None, t0);
        narrow_warn.ok_count = 10;
        narrow_warn.warn_count = 1;
        narrow_warn.finished_bytes = 900_000_000;
        narrow_warn.total_display_ratio = 0.45;
        narrow_warn.eta_secs = Some(64);
        let warn_lines = buffer_to_lines(&render_snapshot(48, 8, &narrow_warn, &theme));
        assert!(
            warn_lines[2].contains("! 1 warn"),
            "48x8 must keep warn counter visible:\n{}",
            warn_lines[2]
        );
        assert!(
            !warn_lines[2].contains("MB") && !warn_lines[2].contains("GB"),
            "48x8 must drop bytes before warn:\n{}",
            warn_lines[2]
        );

        let mut narrow_fail = UiState::new("narrow-fail", 12, 2_000_000_000, None, t0);
        narrow_fail.ok_count = 10;
        narrow_fail.fail_count = 1;
        narrow_fail.finished_bytes = 900_000_000;
        narrow_fail.total_display_ratio = 0.45;
        narrow_fail.eta_secs = Some(64);
        let fail_lines = buffer_to_lines(&render_snapshot(48, 8, &narrow_fail, &theme));
        assert!(
            fail_lines[2].contains("✗ 1 fail"),
            "48x8 must keep fail counter visible:\n{}",
            fail_lines[2]
        );
        assert!(
            !fail_lines[2].contains("MB") && !fail_lines[2].contains("GB"),
            "48x8 must drop bytes before fail:\n{}",
            fail_lines[2]
        );
    }

    #[test]
    fn format_eta_and_total_row_preserve_space_for_long_hour_etas() {
        assert_eq!(format_eta(None), "--:--");
        assert_eq!(format_eta(Some(42)), "00:42");
        assert_eq!(format_eta(Some(3_900)), "1h05m");
        assert_eq!(format_eta(Some(12 * 3600 + 5 * 60)), "12h05m");
        assert_eq!(format_eta(Some(400_000)), ">99h");

        let t0 = Instant::now();
        let mut state = UiState::new("long-transfer", 4, 10_000_000_000, None, t0);
        state.eta_secs = Some(12 * 3600 + 5 * 60);
        let term = render_snapshot(80, 24, &state, &Theme::archival_truecolor());
        let lines = buffer_to_lines(&term);
        assert!(
            lines[2].contains("ETA 12h05m"),
            "TOTAL row must keep a space after ETA:\n{}",
            lines[2]
        );
        assert!(
            !lines[2].contains("ETA12h05m"),
            "TOTAL row must never concatenate ETA and hours:\n{}",
            lines[2]
        );
    }

    #[test]
    fn final_summary_interrupted_notice_is_calm_and_plain_in_monochrome() {
        let t0 = Instant::now();
        let mut state = UiState::new("item", 4, 400, None, t0);
        state.ok_count = 1;
        state.on_event(UiEvent::Interrupted, t0);

        let text = format_final_summary(&state, &Theme::monochrome_unicode());
        assert!(!text.contains('\x1b'));
        assert!(text.contains(
            "Interrupted: 1 ok · 0 skipped. Run the command again to resume remaining files."
        ));
    }

    #[test]
    fn fixed_bay_shows_dimmed_queue_preview_overflow_and_hides_queue_below_60_cols() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();
        let mut state = UiState::new("zzap64", 8, 500_000_000, None, t0);
        state.on_event(
            UiEvent::BatchStarted {
                queue: vec![
                    QueuedFile {
                        name: "active_large.zip".to_string(),
                        size_bytes: Some(120_000_000),
                        permits: 4,
                    },
                    QueuedFile {
                        name: "next_one.pdf".to_string(),
                        size_bytes: Some(12_000_000),
                        permits: 2,
                    },
                    QueuedFile {
                        name: "next_two.pdf".to_string(),
                        size_bytes: Some(18_000_000),
                        permits: 2,
                    },
                    QueuedFile {
                        name: "next_three.zip".to_string(),
                        size_bytes: Some(90_000_000),
                        permits: 4,
                    },
                    QueuedFile {
                        name: "next_four.zip".to_string(),
                        size_bytes: Some(110_000_000),
                        permits: 4,
                    },
                ],
                output_dir: "~/Downloads/zzap64".to_string(),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "active_large.zip".to_string(),
                permit_cost: 4,
                total_bytes: Some(120_000_000),
                mirror_host: Some("ia801504".to_string()),
            },
            t0,
        );

        // 80x24: 1 active row + 2 queued items + 1 overflow row (`+2 more in queue`), steady 8-row box.
        let term = render_snapshot(80, 24, &state, &theme);
        let lines = buffer_to_lines(&term);
        let joined = lines.join("\n");
        assert!(lines[0].starts_with('╭'), "top border:\n{joined}");
        assert!(lines[7].starts_with('╰'), "steady 4-row bay:\n{joined}");
        assert!(
            lines[3].contains("│ ▼  ") && !lines[3].contains("4u"),
            "active slot:\n{joined}"
        );
        assert!(
            lines[4].contains("│ ○  ") && !lines[4].contains("2u"),
            "queued row 1:\n{joined}"
        );
        assert!(
            lines[4].contains("next_one.pdf"),
            "queued file 1:\n{joined}"
        );
        assert!(
            lines[5].contains("│ ○  ") && !lines[5].contains("2u"),
            "queued row 2:\n{joined}"
        );
        assert!(
            lines[5].contains("next_two.pdf"),
            "queued file 2:\n{joined}"
        );
        assert!(
            lines[6].contains("│ ○  +2 more in queue"),
            "queue overflow row:\n{joined}"
        );

        // With 3 active slots and 4 pending items -> free_rows == 1 -> next item with `(+3 more)`.
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 2,
                file_path: "extra_a.jpg".to_string(),
                permit_cost: 1,
                total_bytes: Some(1_000_000),
                mirror_host: Some("ia801504.us.archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 3,
                file_path: "extra_b.jpg".to_string(),
                permit_cost: 1,
                total_bytes: Some(1_000_000),
                mirror_host: Some("ia801504.us.archive.org".to_string()),
            },
            t0,
        );
        let term_one_free = render_snapshot(80, 24, &state, &theme);
        let one_free_lines = buffer_to_lines(&term_one_free);
        assert!(
            one_free_lines[6].contains("│ ○  ")
                && !one_free_lines[6].contains("2u")
                && one_free_lines[6].contains("next_one.pdf")
                && one_free_lines[6].contains("(+3 more)"),
            "free_rows == 1 must show next item with (+3 more):\n{}",
            one_free_lines.join("\n")
        );

        // Narrow terminal (< 60 cols, e.g. 52x20) drops queued preview so active transfers keep priority.
        let narrow = render_snapshot(52, 20, &state, &theme);
        let narrow_joined = buffer_to_lines(&narrow).join("\n");
        assert!(
            !narrow_joined.contains("next_one.pdf") && !narrow_joined.contains("more in queue"),
            "width < 60 must hide queued preview rows:\n{narrow_joined}"
        );
    }

    fn sample_completed_state(t0: Instant) -> UiState {
        let mut state = UiState::new("deftributetozzap64", 18, 48_000_000, None, t0);
        state.on_event(
            UiEvent::BatchStarted {
                queue: Vec::new(),
                output_dir: "~/Downloads/deftributetozzap64".to_string(),
            },
            t0,
        );
        state.ok_count = 16;
        state.skip_count = 2;
        state.session_network_bytes = 48_200_000;
        for sample in [
            2_000_000, 4_500_000, 8_200_000, 11_000_000, 14_500_000, 13_800_000, 10_200_000,
            9_100_000, 12_400_000, 14_900_000, 11_800_000, 8_400_000,
        ] {
            state.record_spark_sample(sample);
        }
        state.on_event(UiEvent::Finish, t0 + Duration::from_millis(5300));
        state
    }

    #[test]
    fn archival_receipt_matches_exact_width_across_breakpoints_no_color_and_ascii() {
        let t0 = Instant::now();
        let state = sample_completed_state(t0);
        let tc = Theme::archival_truecolor();
        let mono = Theme::monochrome_unicode();
        let ascii = Theme::monochrome_ascii();

        for (w, h) in [(120u16, 30u16), (80, 24), (62, 20), (60, 20), (44, 10)] {
            let lines = build_archival_receipt_lines(&state, &tc, w);
            let plain = render_lines_to_plain(&lines);
            assert!(
                plain.first().is_some_and(|l| l.starts_with("╭─ ")),
                "{w}x{h} missing top border:\n{}",
                plain.join("\n")
            );
            assert!(
                plain
                    .last()
                    .is_some_and(|l| l.starts_with('╰') && l.ends_with('╯')),
                "{w}x{h} missing bottom border:\n{}",
                plain.join("\n")
            );
            for (idx, row) in plain.iter().enumerate() {
                assert_eq!(
                    row.width(),
                    w as usize,
                    "{w}x{h} receipt line {idx} width mismatch ({} != {w}): {row:?}\n{}",
                    row.width(),
                    plain.join("\n")
                );
            }
            let joined = plain.join("\n");
            assert!(joined.contains("✓ complete"), "{w}x{h}:\n{joined}");
            assert!(joined.contains("18/18  100%"), "{w}x{h}:\n{joined}");
            assert!(joined.contains("✓ 16 ok"), "{w}x{h}:\n{joined}");
            assert!(joined.contains("↷ 2 skip"), "{w}x{h}:\n{joined}");
            assert!(joined.contains("data "), "{w}x{h}:\n{joined}");
            assert!(joined.contains("time "), "{w}x{h}:\n{joined}");
            assert!(joined.contains("dir "), "{w}x{h}:\n{joined}");

            let row2 = &plain[2];
            let has_sparkline = tc.glyphs().spark_blocks.iter().any(|b| row2.contains(b));
            assert!(
                row2.contains("speed "),
                "{w}x{h} should keep average speed visible when it fits:\n{joined}"
            );
            if w >= 64 {
                assert!(
                    has_sparkline,
                    "{w}x{h} should include sparkline on >= 64 cols:\n{joined}"
                );
            } else {
                assert!(
                    !has_sparkline,
                    "{w}x{h} must omit sparkline below 64 cols:\n{joined}"
                );
            }
            if w <= 60 {
                // Metrics and dir wrap to 2 clean inner rows at <= 60 cols (total 6 lines).
                assert_eq!(
                    plain.len(),
                    6,
                    "{w}x{h} should wrap row 3 into 2 inner rows:\n{joined}"
                );
            }

            // Render via Ratatui TestBackend as well to verify zero panic / clean cells.
            let backend = TestBackend::new(w, h);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new(lines.clone()), frame.area());
                })
                .expect("draw archival receipt");
        }

        // NO_COLOR emits zero ANSI escape bytes while preserving exact width.
        let no_color_out = format_tty_summary(&state, &mono, 80);
        assert!(
            !no_color_out.contains('\x1b'),
            "NO_COLOR receipt must contain zero ANSI:\n{no_color_out}"
        );
        for line in no_color_out.lines() {
            assert_eq!(line.width(), 80, "NO_COLOR line width mismatch: {line:?}");
        }

        // ASCII mode emits pure 7-bit ASCII and exact width.
        let ascii_out = format_tty_summary(&state, &ascii, 80);
        assert!(
            ascii_out.is_ascii() && !ascii_out.contains('\x1b'),
            "ASCII receipt must be pure ASCII with zero ANSI:\n{ascii_out}"
        );
        assert!(ascii_out.contains("+ complete"));
        for line in ascii_out.lines() {
            assert_eq!(line.width(), 80, "ASCII line width mismatch: {line:?}");
        }
    }

    #[test]
    fn exception_ledger_adapts_across_1_17_25_mismatches_and_mixed_severity_groups() {
        let t0 = Instant::now();
        let tc = Theme::archival_truecolor();
        let mono = Theme::monochrome_unicode();
        let ascii = Theme::monochrome_ascii();

        // Tier 1: 1 mismatch -> expanded format with full 32-char hashes.
        let got_full = "a3ed214e9900112233445566778899aa";
        let exp_full = "fe7612e2aabbccddeeff001122334455";
        let mut one = sample_completed_state(t0);
        one.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 100,
                file_path: "stale_scan.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: got_full.to_string(),
                    expected: exp_full.to_string(),
                },
                finished_index: 19,
                total_files: 19,
            }),
            t0 + Duration::from_secs(6),
        );
        let one_lines = build_exception_ledger_lines(&one, &tc, 80);
        let one_plain = render_lines_to_plain(&one_lines);
        let one_joined = one_plain.join("\n");
        assert!(one_joined.contains("MD5 mismatches (1)"), "{one_joined}");
        assert!(one_joined.contains("! stale_scan.pdf"), "{one_joined}");
        assert!(
            one_joined.contains(&format!("got      {got_full}")),
            "{one_joined}"
        );
        assert!(
            one_joined.contains(&format!("expected {exp_full}")),
            "{one_joined}"
        );
        for row in &one_plain {
            assert!(row.width() <= 80, "expanded row overflowed 80: {row:?}");
            assert!(
                row.is_empty()
                    || (row.starts_with('┃') && !row.ends_with('│') && !row.ends_with('┃')),
                "ledger row must have left ┃ and no right border: {row:?}"
            );
        }

        // Tier 2: 17 mismatches -> compact 1-line aligned table, all 17 shown, every line <= width.
        let mut seventeen = sample_completed_state(t0);
        for i in 1..=17 {
            seventeen.on_event(
                UiEvent::SlotFinished(CompletionRecord {
                    slot_id: i,
                    file_path: format!(
                        "sub/dir/zzap64_issue_{i:03}_very_long_hi_res_scan_600dpi.jp2"
                    ),
                    kind: CompletionKind::HashMismatch {
                        got: format!("a3ed21{i:02}9900112233445566778899aa"),
                        expected: format!("fe7612{i:02}aabbccddeeff001122334455"),
                    },
                    finished_index: i as usize,
                    total_files: 35,
                }),
                t0 + Duration::from_secs(6),
            );
        }
        let sev_80 = render_lines_to_plain(&build_exception_ledger_lines(&seventeen, &tc, 80));
        let sev_80_joined = sev_80.join("\n");
        let mismatch_rows_80: Vec<&String> = sev_80.iter().filter(|l| l.contains('≠')).collect();
        assert_eq!(
            mismatch_rows_80.len(),
            17,
            "all 17 mismatches must be shown:\n{sev_80_joined}"
        );
        for row in &sev_80 {
            assert!(row.width() <= 80, "compact 80 row overflow: {row:?}");
            assert!(row.is_empty() || row.starts_with('┃'));
            assert!(!row.ends_with('│'));
        }
        assert!(
            mismatch_rows_80.iter().all(|r| r.contains("(.part)")),
            "80 cols must show (.part):\n{sev_80_joined}"
        );

        // Below 68 cols (e.g. 60 cols), drop `(.part)` cleanly while keeping all 17 lines <= 60.
        let sev_60 = render_lines_to_plain(&build_exception_ledger_lines(&seventeen, &tc, 60));
        let mismatch_rows_60: Vec<&String> = sev_60.iter().filter(|l| l.contains('≠')).collect();
        assert_eq!(mismatch_rows_60.len(), 17);
        assert!(
            mismatch_rows_60.iter().all(|r| !r.contains("(.part)")),
            "below 68 cols must drop (.part):\n{}",
            sev_60.join("\n")
        );
        for row in &sev_60 {
            assert!(row.width() <= 60, "compact 60 row overflow: {row:?}");
        }

        // ASCII mode uses `!=` and `|` bar.
        let sev_ascii =
            render_lines_to_plain(&build_exception_ledger_lines(&seventeen, &ascii, 80));
        let sev_ascii_joined = sev_ascii.join("\n");
        assert!(sev_ascii_joined.is_ascii());
        assert_eq!(sev_ascii.iter().filter(|l| l.contains("!=")).count(), 17);

        // Tier 3: 25 mismatches -> 15 shown + `+10 more, re-run with -v for the full list`.
        let mut twenty_five = sample_completed_state(t0);
        for i in 1..=25 {
            twenty_five.on_event(
                UiEvent::SlotFinished(CompletionRecord {
                    slot_id: i,
                    file_path: format!("scan_{i:02}.pdf"),
                    kind: CompletionKind::HashMismatch {
                        got: format!("a3ed21{i:02}9900112233445566778899aa"),
                        expected: format!("fe7612{i:02}aabbccddeeff001122334455"),
                    },
                    finished_index: i as usize,
                    total_files: 43,
                }),
                t0 + Duration::from_secs(6),
            );
        }
        let tf_lines = render_lines_to_plain(&build_exception_ledger_lines(&twenty_five, &tc, 80));
        let tf_joined = tf_lines.join("\n");
        assert_eq!(
            tf_lines.iter().filter(|l| l.contains('≠')).count(),
            15,
            "over 20 mismatches must cap at 15 rows:\n{tf_joined}"
        );
        assert!(
            tf_joined.contains("+10 more, re-run with -v for the full list"),
            "missing overflow hint:\n{tf_joined}"
        );

        // Non-TTY plain summary remains uncapped (all 25 hashes listed), zero ANSI, zero box borders.
        let plain_non_tty = format_final_summary(&twenty_five, &mono);
        assert!(!plain_non_tty.contains('\x1b'));
        assert!(!plain_non_tty.contains('╭') && !plain_non_tty.contains('┃'));
        assert_eq!(plain_non_tty.matches("scan_").count(), 25);

        // Mixed failure + mismatch + skipped private: verify exact severity ordering.
        let mut mixed = sample_completed_state(t0);
        mixed.on_event(UiEvent::SkippedPrivate { count: 3 }, t0);
        mixed.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 201,
                file_path: "stale.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: got_full.to_string(),
                    expected: exp_full.to_string(),
                },
                finished_index: 19,
                total_files: 20,
            }),
            t0,
        );
        mixed.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 202,
                file_path: "gone.bin".to_string(),
                kind: CompletionKind::Failed {
                    message: "HTTP error 404".to_string(),
                },
                finished_index: 20,
                total_files: 20,
            }),
            t0,
        );
        let mixed_plain = render_lines_to_plain(&build_exception_ledger_lines(&mixed, &tc, 80));
        let mixed_joined = mixed_plain.join("\n");
        let pos_fail = mixed_joined
            .find("Transfer failures (1)")
            .expect("missing Transfer failures");
        let pos_md5 = mixed_joined
            .find("MD5 mismatches (1)")
            .expect("missing MD5 mismatches");
        let pos_priv = mixed_joined
            .find("Skipped private (3)")
            .expect("missing Skipped private");
        assert!(
            pos_fail < pos_md5 && pos_md5 < pos_priv,
            "severity order must be failures < md5 < private:\n{mixed_joined}"
        );
    }

    #[test]
    fn small_batches_1_to_4_files_render_rounded_frame_and_queued_rows_at_viewport_height() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();

        for total_files in 1..=4usize {
            let vp_h = compute_viewport_height_for_batch(80, 24, total_files);
            assert_eq!(vp_h, (4 + total_files) as u16);
            // Even when terminal height itself equals target viewport height, frame must not collapse.
            assert_eq!(
                compute_viewport_height_for_batch(80, vp_h, total_files),
                vp_h
            );

            let mut queue = Vec::new();
            for i in 0..total_files {
                queue.push(QueuedFile {
                    name: format!("file_{i}.pdf"),
                    size_bytes: Some(1_000_000),
                    permits: 2,
                });
            }
            let mut state = UiState::new("small-item", total_files, 4_000_000, None, t0);
            state.on_event(
                UiEvent::BatchStarted {
                    queue,
                    output_dir: "~/small-item".to_string(),
                },
                t0,
            );
            state.on_event(
                UiEvent::SlotStarted {
                    slot_id: 1,
                    file_path: "file_0.pdf".to_string(),
                    permit_cost: 2,
                    total_bytes: Some(1_000_000),
                    mirror_host: Some("ia801504".to_string()),
                },
                t0,
            );

            let term = render_snapshot(80, vp_h, &state, &theme);
            let lines = buffer_to_lines(&term);
            let joined = lines.join("\n");
            assert!(
                lines[0].starts_with('╭'),
                "{total_files}-file batch at height {vp_h} collapsed top border:\n{joined}"
            );
            assert!(
                lines[vp_h as usize - 1].starts_with('╰'),
                "{total_files}-file batch at height {vp_h} collapsed bottom border:\n{joined}"
            );
            assert!(
                joined.contains("file_0.pdf"),
                "{total_files}-file batch missing active slot:\n{joined}"
            );
            if total_files >= 2 {
                assert!(
                    joined.contains("file_1.pdf"),
                    "{total_files}-file batch at height {vp_h} must render queued row:\n{joined}"
                );
            }
        }
    }

    #[test]
    fn receipt_proportional_bar_uses_truecolor_gradient_ansi16_green_and_distinct_empty_vs_skip() {
        use ratatui::style::Color;

        let t0 = Instant::now();
        let mut state = UiState::new("item", 10, 10_000, None, t0);
        state.ok_count = 4;
        state.skip_count = 2;
        state.warn_count = 1;
        state.fail_count = 1;
        state.interrupted = true; // leaves 2 unstarted (empty track)

        let tc = Theme::archival_truecolor();
        let tc_lines = build_archival_receipt_lines(&state, &tc, 80);
        let row1_spans = &tc_lines[1].spans;
        let ok_spans: Vec<_> = row1_spans
            .iter()
            .filter(|s| s.content == tc.glyphs().bar_ok)
            .collect();
        assert!(
            ok_spans.len() > 1,
            "TrueColor ok bar should render per-cell gradient spans"
        );
        assert_ne!(
            ok_spans.first().unwrap().style.fg,
            ok_spans.last().unwrap().style.fg,
            "TrueColor ok bar gradient must interpolate from bronze to gold"
        );
        let plain_row1 = render_lines_to_plain(&tc_lines)[1].clone();
        assert!(
            plain_row1.contains('░') && plain_row1.contains('─'),
            "skip (░) and unstarted empty track (─) must be visually distinct:\n{plain_row1}"
        );

        let ansi = Theme::new(ColorLevel::Ansi16, crate::ui::palette::GlyphSet::Unicode);
        let ansi_lines = build_archival_receipt_lines(&state, &ansi, 80);
        let ok_ansi_span = ansi_lines[1]
            .spans
            .iter()
            .find(|s| s.content.contains(ansi.glyphs().bar_ok))
            .expect("missing ok bar span on Ansi16");
        assert_eq!(
            ok_ansi_span.style.fg,
            Some(Color::Green),
            "Ansi16 ok bar must use green (not yellow) to avoid colliding with warnings"
        );
    }

    #[test]
    fn compact_ledger_aligns_without_wide_void_fits_single_line_md5_header_and_bounds_narrow() {
        let t0 = Instant::now();
        let tc = Theme::archival_truecolor();
        let mut state = sample_completed_state(t0);
        for i in 1..=5 {
            state.on_event(
                UiEvent::SlotFinished(CompletionRecord {
                    slot_id: i,
                    file_path: format!("f{i}.bin"),
                    kind: CompletionKind::Failed {
                        message: if i == 1 {
                            "HTTP error 404".to_string()
                        } else {
                            "timeout".to_string()
                        },
                    },
                    finished_index: i as usize,
                    total_files: 20,
                }),
                t0,
            );
            state.on_event(
                UiEvent::SlotFinished(CompletionRecord {
                    slot_id: 100 + i,
                    file_path: format!("m{i}.pdf"),
                    kind: CompletionKind::HashMismatch {
                        got: format!("a3ed21{i:02}9900112233445566778899aa"),
                        expected: format!("fe7612{i:02}aabbccddeeff001122334455"),
                    },
                    finished_index: (5 + i) as usize,
                    total_files: 20,
                }),
                t0,
            );
        }

        // At 120 cols, short filenames must not create a 75-cell blank void and columns must align.
        let wide_plain = render_lines_to_plain(&build_exception_ledger_lines(&state, &tc, 120));
        let fail_rows: Vec<&String> = wide_plain
            .iter()
            .filter(|r| r.contains("f1.bin") || r.contains("f2.bin"))
            .collect();
        assert_eq!(fail_rows.len(), 2);
        assert!(
            fail_rows.iter().all(|r| r.width() < 40),
            "compact transfer failures must not have wide blank void:\n{}",
            wide_plain.join("\n")
        );
        let md5_rows: Vec<&String> = wide_plain.iter().filter(|r| r.contains('≠')).collect();
        assert_eq!(md5_rows.len(), 5);
        assert!(
            md5_rows.iter().all(|r| r.width() < 60),
            "compact MD5 mismatches must not have wide blank void:\n{}",
            wide_plain.join("\n")
        );
        let hash_col = md5_rows[0].find("a3ed").unwrap();
        assert!(
            md5_rows.iter().all(|r| r.find("a3ed") == Some(hash_col)),
            "compact MD5 hashes must align vertically:\n{}",
            wide_plain.join("\n")
        );

        // At 80 cols, MD5 header + shortened note must fit on one line (<= 76 cells).
        let cols_80 = render_lines_to_plain(&build_exception_ledger_lines(&state, &tc, 80));
        let md5_hdr = cols_80
            .iter()
            .find(|r| r.contains("MD5 mismatches (5)"))
            .expect("missing MD5 header");
        assert!(
            md5_hdr.contains("archive.org metadata can be stale, .part kept"),
            "MD5 header must stay on one line at 80 cols: {md5_hdr:?}"
        );
        assert!(
            md5_hdr.width() <= 76,
            "MD5 header exceeded 76 cells ({}): {md5_hdr:?}",
            md5_hdr.width()
        );

        // Extreme narrow widths (< 24 cols) must never overflow `max_width`.
        for narrow_w in [23u16, 20, 16, 12, 8, 4] {
            let narrow_plain =
                render_lines_to_plain(&build_exception_ledger_lines(&state, &tc, narrow_w));
            for row in &narrow_plain {
                assert!(
                    row.width() <= narrow_w as usize,
                    "ledger row exceeded narrow width {narrow_w}: {row:?}"
                );
            }
        }
    }

    #[test]
    fn receipt_badges_ledger_icons_and_status_glyphs_use_bold_in_color_themes() {
        use ratatui::style::Modifier;

        let t0 = Instant::now();
        let tc = Theme::archival_truecolor();
        let mut state = sample_completed_state(t0);
        state.on_event(
            UiEvent::SlotFinished(CompletionRecord {
                slot_id: 101,
                file_path: "bad.pdf".to_string(),
                kind: CompletionKind::HashMismatch {
                    got: "aaaa".to_string(),
                    expected: "bbbb".to_string(),
                },
                finished_index: 19,
                total_files: 20,
            }),
            t0,
        );

        let receipt = build_archival_receipt_lines(&state, &tc, 80);
        let badge_span = receipt[0]
            .spans
            .iter()
            .find(|s| s.content.contains("warning"))
            .expect("missing receipt badge");
        assert!(badge_span.style.add_modifier.contains(Modifier::BOLD));

        let ok_icon_span = receipt[2]
            .spans
            .iter()
            .find(|s| s.content == "✓")
            .expect("missing receipt ok icon");
        assert!(ok_icon_span.style.add_modifier.contains(Modifier::BOLD));

        let warn_icon_span = receipt[2]
            .spans
            .iter()
            .find(|s| s.content == "!")
            .expect("missing receipt warn icon");
        assert!(warn_icon_span.style.add_modifier.contains(Modifier::BOLD));

        let ledger = build_exception_ledger_lines(&state, &tc, 80);
        let ledger_hdr_icon = ledger[0]
            .spans
            .iter()
            .find(|s| s.content == "!")
            .expect("missing ledger header icon");
        assert!(ledger_hdr_icon.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn extract_and_format_mirror_region_covers_archive_hosts_fallback_and_width_two() {
        for (host, expected_code, expected_flag, expected_ascii) in [
            ("ia801909.us.archive.org", Some(['U', 'S']), "🇺🇸", "US"),
            ("dn790007.ca.archive.org", Some(['C', 'A']), "🇨🇦", "CA"),
            ("dn760105.eu.archive.org", Some(['E', 'U']), "🇪🇺", "EU"),
            ("node01.de.archive.org", Some(['D', 'E']), "🇩🇪", "DE"),
            ("IA801909.US.ARCHIVE.ORG.", Some(['U', 'S']), "🇺🇸", "US"),
            ("sub.node.gb.archive.org", Some(['G', 'B']), "🇬🇧", "GB"),
        ] {
            assert_eq!(
                extract_mirror_region(host),
                expected_code,
                "extract_mirror_region({host:?})"
            );
            let (emoji_badge, emoji_resolved) = format_mirror_region(Some(host), true);
            assert!(emoji_resolved, "host {host:?} must resolve in emoji mode");
            assert_eq!(emoji_badge, expected_flag);
            assert_eq!(
                emoji_badge.width(),
                2,
                "flag emoji {emoji_badge:?} must be exactly 2 cells wide"
            );

            let (ascii_badge, ascii_resolved) = format_mirror_region(Some(host), false);
            assert!(
                ascii_resolved,
                "host {host:?} must resolve in fallback mode"
            );
            assert_eq!(ascii_badge, expected_ascii);
            assert_eq!(
                ascii_badge.width(),
                2,
                "fallback code {ascii_badge:?} must be exactly 2 cells wide"
            );
        }

        for unresolved in [
            "archive.org",
            "www.archive.org",
            "us.archive.org",
            "ia801909.archive.org",
            "ia801909.12.archive.org",
            "ia801909.usa.archive.org",
            ".us.archive.org",
            "cdn.example.com",
            "127.0.0.1",
            "",
        ] {
            assert_eq!(
                extract_mirror_region(unresolved),
                None,
                "host {unresolved:?} must not resolve a region"
            );
            for use_emoji in [true, false] {
                let (badge, resolved) = format_mirror_region(Some(unresolved), use_emoji);
                assert!(!resolved);
                assert_eq!(badge, "--");
                assert_eq!(badge.width(), 2);
            }
        }

        for use_emoji in [true, false] {
            let (badge, resolved) = format_mirror_region(None, use_emoji);
            assert!(!resolved);
            assert_eq!(badge, "--");
            assert_eq!(badge.width(), 2);
        }
    }

    #[test]
    fn live_frame_header_progress_and_slot_alignment_hold_in_emoji_and_fallback_modes() {
        let t0 = Instant::now();
        let theme = Theme::archival_truecolor();
        let mut state = UiState::new("zzapp_64_issue_001_600dpi", 10, 500_000_000, None, t0);
        state.ok_count = 6;
        state.skip_count = 1;
        state.finished_bytes = 280_000_000;
        state.total_display_ratio = 0.70;
        state.aggregate_speed_bps = 3_400_000.0;
        state.eta_secs = Some(62);
        for sample in [
            1_000_000, 1_800_000, 2_400_000, 3_100_000, 3_400_000, 3_600_000, 3_200_000, 3_500_000,
            3_300_000, 3_700_000, 3_400_000, 3_400_000,
        ] {
            state.record_spark_sample(sample);
        }

        // Slot 1: US GET
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "zzap64_issue_001_600dpi_ch01.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(25_000_000),
                mirror_host: Some("ia801909.us.archive.org".to_string()),
            },
            t0,
        );
        state.active_slots[0].display_ratio = 0.65;
        state.active_slots[0].speed_bps = 1_800_000.0;

        // Slot 2: CA RESUME
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 2,
                file_path: "zzap64_issue_001_600dpi_ch02.pdf".to_string(),
                permit_cost: 2,
                total_bytes: Some(40_000_000),
                mirror_host: Some("dn790007.ca.archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotResumeBaseline {
                slot_id: 2,
                start_offset: 16_000_000,
                total_bytes: Some(40_000_000),
            },
            t0,
        );
        state.active_slots[1].speed_bps = 1_600_000.0;

        // Slot 3: pre-redirect archive.org -> dimmed "--"
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 3,
                file_path: "zzap64_issue_001_600dpi_ch03.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(12_000_000),
                mirror_host: Some("archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStage {
                slot_id: 3,
                stage: TransferStage::Redirect,
                mirror_host: None,
                cooldown: None,
                retry: None,
            },
            t0,
        );

        // Slot 4: EU MD5 verification
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 4,
                file_path: "zzap64_issue_001_600dpi_ch04.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(18_000_000),
                mirror_host: Some("dn760105.eu.archive.org".to_string()),
            },
            t0,
        );
        state.on_event(
            UiEvent::SlotStage {
                slot_id: 4,
                stage: TransferStage::Md5,
                mirror_host: None,
                cooldown: None,
                retry: None,
            },
            t0,
        );
        state.active_slots[3].display_ratio = 0.80;

        for use_emoji in [true, false] {
            for (w, h) in [(120u16, 30u16), (80, 24), (60, 20), (48, 8)] {
                let term = render_snapshot_with_emoji(w, h, &state, &theme, use_emoji);
                let lines = buffer_to_lines(&term);
                let joined = lines.join("\n");
                let buf = term.backend().buffer();

                // Exact right border alignment on every framed row.
                assert_eq!(buf[(0, 0)].symbol(), "╭");
                assert_eq!(buf[(w - 1, 0)].symbol(), "╮");
                assert_eq!(buf[(0, 7)].symbol(), "╰");
                assert_eq!(buf[(w - 1, 7)].symbol(), "╯");
                for y in 1..=6 {
                    assert_eq!(
                        buf[(0, y)].symbol(),
                        "│",
                        "left border broken at {w}x{h} row {y} (emoji={use_emoji}):\n{joined}"
                    );
                    assert_eq!(
                        buf[(w - 1, y)].symbol(),
                        "│",
                        "right border broken at {w}x{h} row {y} (emoji={use_emoji}):\n{joined}"
                    );
                }

                // Header file progress, no permit meter or 1u/2u/4u clutter.
                if w >= 60 {
                    assert!(
                        lines[1].contains("7/10 files"),
                        "{w}x{h} missing '7/10 files':\n{joined}"
                    );
                } else {
                    assert!(
                        lines[1].contains("7/10") && !lines[1].contains("files"),
                        "{w}x{h} narrow header must shorten to '7/10':\n{joined}"
                    );
                }
                assert!(
                    !lines[1].contains("/4") && !joined.contains("1u ") && !joined.contains("2u "),
                    "{w}x{h} unexpected permit clutter:\n{joined}"
                );

                // Mirror region badges on >= 80 cols; hidden below 80 cols.
                let expected_us = if use_emoji { "🇺🇸" } else { "US" };
                let expected_ca = if use_emoji { "🇨🇦" } else { "CA" };
                if w >= 80 {
                    assert!(
                        lines[3].contains(expected_us),
                        "{w}x{h} missing {expected_us} (emoji={use_emoji}):\n{joined}"
                    );
                    assert!(
                        lines[4].contains(expected_ca),
                        "{w}x{h} missing {expected_ca} (emoji={use_emoji}):\n{joined}"
                    );
                    assert!(
                        lines[5].contains("  --"),
                        "{w}x{h} pre-redirect archive.org must emit dimmed '--':\n{joined}"
                    );
                    // Pre-redirect "--" uses dimmed track_style foreground.
                    let dash_x = (0..w)
                        .rfind(|&x| buf[(x, 5)].symbol() == "-")
                        .expect("missing '--' cell");
                    assert_eq!(
                        buf[(dash_x, 5)].fg,
                        theme.track_style().fg.expect("track fg")
                    );
                    // Full 32-char filename fits untruncated at 80 cols thanks to reclaimed width.
                    assert!(
                        lines[3].contains("zzap64_issue_001_600dpi_ch01.pdf"),
                        "{w}x{h} 32-char filename should fit untruncated:\n{joined}"
                    );
                } else {
                    assert!(
                        !lines[3].contains(expected_us),
                        "{w}x{h} region badge must hide below 80 cols:\n{joined}"
                    );
                }

                // Every active slot's progress bar starts at the exact same terminal column.
                let bar_start_x = |y: u16| -> u16 {
                    (0..w)
                        .find(|&x| matches!(buf[(x, y)].symbol(), "█" | "─"))
                        .unwrap_or_else(|| panic!("missing bar cell at {w}x{h} row {y}:\n{joined}"))
                };
                let cols: Vec<u16> = (3..=6).map(bar_start_x).collect();
                assert!(
                    cols.iter().all(|&c| c == cols[0]),
                    "{w}x{h} (emoji={use_emoji}) active slot bars misaligned {cols:?}:\n{joined}"
                );
            }
        }
    }
}
