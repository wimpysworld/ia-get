//! Pure rendering functions for the archival inline instrument, completion lines,
//! and post-run summary.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::downloader::BACKPRESSURE_RECOVERY_SUCCESSES;
use crate::ui::palette::{
    style_accent, style_bold, style_err, style_muted, style_ok, style_warn, Glyphs, Theme,
};
use crate::ui::state::{
    ActiveSlot, AttentionItem, CompletionKind, CompletionRecord, SkipReason, TransferStage, UiState,
};
use crate::utils::{format_duration, format_size, format_transfer_rate};

/// Standard height (in terminal rows) of the framed archival inline instrument.
pub const INSTRUMENT_HEIGHT: u16 = 8;

/// Height (in terminal rows) of the collapsed aggregate summary below minimum dimensions.
pub const COLLAPSED_HEIGHT: u16 = 2;

/// Minimum terminal width for the full bordered instrument (with mirror host and sparkline).
pub const FULL_WIDTH_BREAKPOINT: u16 = 80;

/// Minimum terminal width for the compact bordered instrument (hiding mirror host and sparkline).
pub const COMPACT_WIDTH_BREAKPOINT: u16 = 60;

/// Minimum terminal width below which the instrument collapses to a 2-line aggregate summary.
pub const MIN_BORDERED_WIDTH: u16 = 48;

/// Computes the inline viewport height for a terminal of `(width, height)`.
pub const fn compute_viewport_height(term_width: u16, term_height: u16) -> u16 {
    if term_width < MIN_BORDERED_WIDTH || term_height < INSTRUMENT_HEIGHT {
        if term_height < COLLAPSED_HEIGHT {
            term_height
        } else {
            COLLAPSED_HEIGHT
        }
    } else {
        INSTRUMENT_HEIGHT
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

/// Builds styled spans and display width for the 12-sample throughput sparkline.
///
/// Zero-value samples render in the muted bar-track style so idle slots stay calm.
fn build_sparkline_spans(state: &UiState, theme: &Theme) -> (Vec<Span<'static>>, usize) {
    let glyphs = theme.glyphs();
    let max_val = state.sparkline.iter().copied().max().unwrap_or(0);
    let mut spans = Vec::with_capacity(state.sparkline.len());
    let mut width = 0usize;
    for &sample in &state.sparkline {
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

/// Formats the permit gauge `[████] 4/4` using active units and effective ceiling.
fn build_permit_gauge(state: &UiState, glyphs: &Glyphs) -> (String, bool) {
    let ceiling = state.scheduler.effective_ceiling.clamp(1, 4);
    let in_flight = state.scheduler.in_flight_units.min(4);
    let filled_blocks = in_flight;
    let empty_blocks = 4usize.saturating_sub(filled_blocks);

    let step = 4 / ceiling;
    let active_slots = if ceiling == 4 {
        in_flight
    } else if in_flight == 0 {
        0
    } else {
        in_flight.div_ceil(step).min(ceiling)
    };

    let gauge = format!(
        "[{}{}] {}/{}",
        glyphs.permit_filled.repeat(filled_blocks),
        glyphs.permit_empty.repeat(empty_blocks),
        active_slots,
        ceiling
    );
    (gauge, ceiling < 4)
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

/// Renders the header row (`ia-get · <identifier>` on left; permit gauge, sparkline, speed on right).
fn build_header_line(
    state: &UiState,
    inner_width: usize,
    show_sparkline: bool,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let (gauge_text, throttled) = build_permit_gauge(state, glyphs);
    let gauge_style = if throttled {
        theme.warn_style()
    } else {
        theme.accent_style()
    };
    let speed_text = format!("{:>10}", format_speed(state.aggregate_speed_bps));

    let mut right_spans: Vec<Span<'static>> = Vec::new();
    let mut right_width = 0usize;

    right_spans.push(Span::styled(gauge_text.clone(), gauge_style));
    right_width += gauge_text.width();

    if show_sparkline {
        let (spark_spans, spark_width) = build_sparkline_spans(state, theme);
        right_spans.push(Span::raw("  "));
        right_spans.extend(spark_spans);
        right_width += 2 + spark_width;
    }

    right_spans.push(Span::raw("  "));
    right_spans.push(Span::styled(speed_text.clone(), theme.primary_style()));
    right_width += 2 + speed_text.width();

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
    let fail_span = (state.fail_count > 0).then(|| {
        Span::styled(
            format!("  {} {} fail", glyphs.fail, state.fail_count),
            theme.err_style(),
        )
    });
    let warn_span = (state.warn_count > 0).then(|| {
        Span::styled(
            format!("  {} {} warn", glyphs.warn, state.warn_count),
            theme.warn_style(),
        )
    });
    let ok_span = Span::styled(format!("  {} ok", state.ok_count), theme.ok_style());
    let skip_span = (state.skip_count > 0)
        .then(|| Span::styled(format!("  {} skip", state.skip_count), theme.muted_style()));

    // Allocate space in priority order (fail > warn > ok > skip > bytes) so narrow rows
    // drop bytes before hiding warning or failure counters.
    let take_if_fits = |span: Option<Span<'static>>, budget: &mut usize| -> Option<Span<'static>> {
        let s = span?;
        let w = s.content.width();
        if w <= *budget {
            *budget -= w;
            Some(s)
        } else {
            None
        }
    };

    let kept_fail = take_if_fits(fail_span, &mut middle_budget);
    let kept_warn = take_if_fits(warn_span, &mut middle_budget);
    let kept_ok = take_if_fits(Some(ok_span), &mut middle_budget);
    let kept_skip = take_if_fits(skip_span, &mut middle_budget);

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

    for span in [kept_ok, kept_skip, kept_warn, kept_fail]
        .into_iter()
        .flatten()
    {
        used += span.content.width();
        spans.push(span);
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
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let permit_str = format!("{}u ", slot.permit_cost);
    let stage_str = format!("{:<3} ", slot.stage.verb());
    let stage_style = match slot.stage {
        TransferStage::Get => theme.border_style(),
        TransferStage::Resume => theme.accent_style(),
        TransferStage::Redirect | TransferStage::Md5 => theme.info_style(),
        TransferStage::RateLimited => theme.warn_style(),
    };
    let prefix_w = permit_str.width() + stage_str.width();

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
        let compact_status = format!("cd {cd_secs:02}s  {ceiling}/4  {pips}");
        let status = if inner_width >= prefix_w + 14 + 2 + full_status.width() {
            full_status
        } else {
            compact_status
        };
        let status_w = status.width();
        let name_budget = inner_width.saturating_sub(prefix_w + 2 + status_w);
        let name = format_slot_filename(slot, name_budget, glyphs);
        let used = prefix_w + name.width() + 2 + status_w;
        let pad = inner_width.saturating_sub(used);

        return Line::from(vec![
            Span::styled(permit_str, theme.muted_style()),
            Span::styled(stage_str, stage_style),
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

    let info_str = match slot.stage {
        TransferStage::Md5 => format!(" {:>10}", "hashing"),
        TransferStage::Redirect => format!(" {:>10}", "redirect"),
        _ => format!(" {:>10}", format_speed(slot.speed_bps)),
    };
    let info_style = match slot.stage {
        TransferStage::Md5 | TransferStage::Redirect => theme.info_style(),
        _ => theme.muted_style(),
    };

    let mirror_col_w = if show_mirror { 14 } else { 0 }; // "  " + 12-cell host
    let right_w = bar_width + pct_str.width() + info_str.width() + mirror_col_w;
    let name_budget = inner_width.saturating_sub(prefix_w + 1 + right_w);
    let name = format_slot_filename(slot, name_budget, glyphs);

    let mut spans = Vec::with_capacity(bar_width + 8);
    spans.push(Span::styled(permit_str, theme.muted_style()));
    spans.push(Span::styled(stage_str, stage_style));
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

    if show_mirror {
        let host = slot
            .mirror_host
            .as_deref()
            .map(|h| middle_truncate(h, 12, glyphs.ellipsis))
            .unwrap_or_default();
        spans.push(Span::raw("  "));
        spans.push(Span::styled(pad_right(&host, 12), theme.muted_style()));
    }

    Line::from(spans)
}

/// Renders the collapsed 2-line aggregate summary used when width < 48 or height < 8.
fn render_collapsed(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let glyphs = theme.glyphs();
    let width = area.width as usize;

    let (gauge, throttled) = build_permit_gauge(state, glyphs);
    let speed = format_speed(state.aggregate_speed_bps);
    let line1_plain = format!(
        "ia-get {} {}  {gauge}  {speed}",
        glyphs.sep, state.identifier
    );
    let line1_fit = middle_truncate(&line1_plain, width, glyphs.ellipsis);
    let line1_style = if throttled {
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
/// - `>= 80` cols (and `>= 8` rows): full instrument with mirror host and sparkline
/// - `60..=79` cols (and `>= 8` rows): compact instrument hiding mirror host and sparkline, 12-cell bars
/// - `48..=59` cols (and `>= 8` rows): extra-compact bordered instrument with 8-cell bars
/// - `< 48` cols or `< 8` rows: 2-line borderless aggregate summary
pub fn render_instrument(frame: &mut Frame, state: &UiState, theme: &Theme) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    if area.width < MIN_BORDERED_WIDTH || area.height < INSTRUMENT_HEIGHT {
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
        height: INSTRUMENT_HEIGHT.min(area.height),
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
    let total_bar_width = if is_full {
        14
    } else if area.width >= COMPACT_WIDTH_BREAKPOINT {
        10
    } else {
        6
    };
    let slot_bar_width = if area.width >= 108 {
        20
    } else if is_full {
        14
    } else if area.width >= COMPACT_WIDTH_BREAKPOINT {
        12
    } else {
        8
    };

    let rows = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Length(1), // TOTAL
        Constraint::Min(0),    // Up to 4 active slots
    ])
    .split(padded);

    let header_line = build_header_line(state, inner_width, is_full, theme);
    frame.render_widget(Paragraph::new(header_line), rows[0]);

    let total_line = build_total_line(state, inner_width, total_bar_width, theme);
    frame.render_widget(Paragraph::new(total_line), rows[1]);

    let slot_rows = rows[2].height as usize;
    if state.active_slots.is_empty() && slot_rows > 0 {
        let idle_msg = if let Some(cd) = state.scheduler.cooldown_remaining {
            let ceiling = state.scheduler.effective_ceiling.clamp(1, 4);
            format!("Host cooldown {:02}s  ceiling {ceiling}/4", cd.as_secs())
        } else if state.ok_count + state.skip_count + state.fail_count == 0 {
            "Preparing transfers...".to_string()
        } else {
            String::new()
        };
        if !idle_msg.is_empty() {
            let style = if state.scheduler.cooldown_remaining.is_some() {
                theme.warn_style()
            } else {
                theme.muted_style()
            };
            let line = Line::from(Span::styled(tail_truncate(&idle_msg, inner_width), style));
            let row_rect = Rect {
                x: rows[2].x,
                y: rows[2].y,
                width: rows[2].width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line), row_rect);
        }
        return;
    }

    for (idx, slot) in state.active_slots.iter().take(slot_rows).enumerate() {
        let row_rect = Rect {
            x: rows[2].x,
            y: rows[2].y + idx as u16,
            width: rows[2].width,
            height: 1,
        };
        let line = build_slot_line(slot, state, inner_width, is_full, slot_bar_width, theme);
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
                style_ok(glyphs.ok, theme),
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
            style_warn(glyphs.warn, theme),
            "(MD5 mismatch, kept .part)".to_string(),
        ),
        CompletionKind::Failed { message } => (
            glyphs.fail,
            style_err(glyphs.fail, theme),
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

/// Renders a single permanent completion line directly into a Ratatui `Buffer` for `insert_before(1, ...)`.
pub(crate) fn render_completion_line_to_buf(
    record: &CompletionRecord,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let area = buf.area;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let glyphs = theme.glyphs();
    let (icon, icon_style, detail, detail_style) = match &record.kind {
        CompletionKind::Downloaded {
            bytes,
            duration,
            verified_md5,
        } => {
            let md5_part = if *verified_md5 { ", MD5 ok" } else { "" };
            (
                glyphs.ok,
                theme.ok_style(),
                format!(
                    "({} in {}{md5_part})",
                    format_size(*bytes),
                    format_duration(*duration)
                ),
                theme.muted_style(),
            )
        }
        CompletionKind::Skipped { reason, .. } => {
            let why = match reason {
                SkipReason::AlreadyVerified => "skipped, already verified",
                SkipReason::AlreadyExists => "skipped, already exists",
            };
            (
                glyphs.skip,
                theme.muted_style(),
                format!("({why})"),
                theme.muted_style(),
            )
        }
        CompletionKind::HashMismatch { .. } => (
            glyphs.warn,
            theme.warn_style(),
            "(MD5 mismatch, kept .part)".to_string(),
            theme.warn_style(),
        ),
        CompletionKind::Failed { message } => (
            glyphs.fail,
            theme.err_style(),
            format!("({message})"),
            theme.err_style(),
        ),
    };

    let limit = area.width as usize;
    let icon_w = icon.width();
    let detail_w = detail.width();
    let fixed = icon_w + 3; // "<icon> <name>  <detail>"

    let line = if fixed + detail_w + 8 <= limit {
        let name = middle_truncate(&record.file_path, limit - fixed - detail_w, glyphs.ellipsis);
        Line::from(vec![
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(name, theme.primary_style()),
            Span::raw("  "),
            Span::styled(detail, detail_style),
        ])
    } else {
        let avail = limit.saturating_sub(icon_w + 1);
        let combined = format!("{}  {detail}", record.file_path);
        Line::from(vec![
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(tail_truncate(&combined, avail), Style::default()),
        ])
    };

    Paragraph::new(line).render(area, buf);
}

/// Formats the calm post-run summary line and structured `Needs attention` section.
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

    if state.interrupted {
        out.push_str(&format!(
            "{} {} {joined}. Run the command again to resume remaining files.",
            style_err(glyphs.fail, theme),
            style_bold("Interrupted:", theme),
        ));
    } else {
        let icon = if state.fail_count > 0 {
            style_err(glyphs.fail, theme)
        } else if state.warn_count > 0 {
            style_warn(glyphs.warn, theme)
        } else {
            style_ok(glyphs.ok, theme)
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
                "\n    {} {}",
                style_warn(glyphs.warn, theme),
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
            "\n    {} {} private {file_label} skipped {}",
            style_warn(glyphs.warn, theme),
            style_bold(&skipped_private.to_string(), theme),
            style_muted("(supply -b/--cookies to download private files)", theme)
        ));
    }

    if !errors.is_empty() {
        out.push_str(&format!("\n  {}", style_err("Transfer errors:", theme)));
        for (file_path, message) in errors {
            out.push_str(&format!(
                "\n    {} {}: {message}",
                style_err(glyphs.fail, theme),
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

        // Slot 1: 1u GET small file
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 1,
                file_path: "issue_001_cover.jpg".to_string(),
                permit_cost: 1,
                total_bytes: Some(2_400_000),
                mirror_host: Some("ia801504".to_string()),
            },
            t0,
        );
        state.active_slots[0].display_ratio = 0.84;
        state.active_slots[0].target_ratio = 0.84;
        state.active_slots[0].speed_bps = 4_200_000.0;

        // Slot 2: 2u RES .part resume
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 2,
                file_path: "zzap64_issue_001_600dpi.pdf".to_string(),
                permit_cost: 2,
                total_bytes: Some(42_000_000),
                mirror_host: Some("ia801504".to_string()),
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

        // Slot 3: 1u MD5 hashing
        state.on_event(
            UiEvent::SlotStarted {
                slot_id: 3,
                file_path: "日本語_archive_冊子_😀.pdf".to_string(),
                permit_cost: 1,
                total_bytes: Some(5_000_000),
                mirror_host: Some("dn720004".to_string()),
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
                lines[1].contains("[████] 4/4"),
                "{w}x{h} permit gauge:\n{joined}"
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
            assert!(lines[3].contains("1u GET"), "{w}x{h} GET slot:\n{joined}");
            assert!(
                lines[3].contains("ia801504"),
                "{w}x{h} mirror host:\n{joined}"
            );
            assert!(lines[4].contains("2u RES"), "{w}x{h} RES slot:\n{joined}");
            assert!(
                !lines[4].contains("(.part)"),
                "{w}x{h} redundant .part tag must be omitted:\n{joined}"
            );
            assert!(
                lines[4].contains("zzap64"),
                "{w}x{h} resumed filename missing:\n{joined}"
            );
            assert!(lines[5].contains("1u MD5"), "{w}x{h} MD5 slot:\n{joined}");
            assert!(
                lines[5].contains("hashing"),
                "{w}x{h} hashing label:\n{joined}"
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
            assert!(lines[1].contains("[████] 4/4"), "{w}x{h} gauge:\n{joined}");
            // Sparkline and mirror host must be hidden below 80 columns.
            assert!(
                !lines[1].contains('▁') && !lines[1].contains('▆'),
                "{w}x{h} sparkline must hide:\n{joined}"
            );
            assert!(
                !lines[3].contains("ia801504"),
                "{w}x{h} mirror host must hide:\n{joined}"
            );
            assert!(lines[3].contains("1u GET"), "{w}x{h} slot 1:\n{joined}");
            assert!(lines[4].contains("2u RES"), "{w}x{h} slot 2:\n{joined}");
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
                    mirror_host: Some("ia801504".to_string()),
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
                    mirror_host: Some("ia801504".to_string()),
                },
                t0,
            );
        }
        let term = render_snapshot(80, 24, &med, &theme);
        let joined = buffer_to_lines(&term).join("\n");
        assert!(joined.contains("2u GET"), "missing 2u slots:\n{joined}");

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
                mirror_host: Some("ia801504".to_string()),
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
        assert!(joined.contains("4u 429"), "missing 4u 429:\n{joined}");
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

        // NO_COLOR (monochrome Unicode): every cell has default fg colour.
        let mono_uni = Theme::monochrome_unicode();
        let term = render_snapshot(80, 24, &state, &mono_uni);
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
                mirror_host: Some("ia801504".to_string()),
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
                mirror_host: Some("ia801504".to_string()),
                cooldown: None,
                retry: None,
            },
            t0,
        );

        let term = render_snapshot(80, 24, &state, &Theme::archival_truecolor());
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
        assert!(lines[3].contains("1u RED"), "RED stage missing:\n{joined}");
        assert!(
            lines[3].contains("redirect"),
            "redirect label missing:\n{joined}"
        );
        assert!(
            lines[3].contains("ia801504"),
            "updated mirror host missing:\n{joined}"
        );
    }

    #[test]
    fn completion_line_buffer_renders_all_outcomes_and_truncates_cjk_without_wrapping() {
        let theme = Theme::archival_truecolor();
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
            let mut buf = Buffer::empty(Rect::new(0, 0, 48, 1));
            render_completion_line_to_buf(record, &mut buf, &theme);
            let row: String = (0..48).map(|x| buf[(x, 0)].symbol()).collect();
            assert!(
                row.starts_with(prefix) && !row.starts_with(&format!("{prefix} ")),
                "completion buffer row must use a single space after icon: {row:?}"
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
            .filter(|&x| {
                // Skip permit gauge `[████]` by matching only the 12-cell sparkline block range.
                let sym = buf[(x, header_y)].symbol();
                spark_chars.contains(&sym)
                    && x > 20
                    && (buf[(x.saturating_sub(1), header_y)].symbol() != "["
                        && buf[(x + 1, header_y)].symbol() != "]")
            })
            .collect();
        // Take the last 12 contiguous sparkline cells on the header row.
        assert!(
            spark_xs.len() >= 12,
            "expected at least 12 sparkline cells, found {:?}",
            spark_xs
        );
        let spark_slice = &spark_xs[spark_xs.len() - 12..];
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
}
