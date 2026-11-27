//! Sibling Verification Suite renderer for `--check` on interactive TTY terminals.
//!
//! Non-TTY streams keep plain `✗ <file>: <issues>` and `Verification summary:` output
//! inside `src/verify.rs`.

use std::collections::BTreeMap;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::ui::list::split_folder_and_name;
use crate::ui::palette::{ColorLevel, Theme};
use crate::ui::render::{
    allocate_outcome_bar_widths, alphanumeric_cmp, build_micro_bar_spans, clip_line_to_max_width,
    format_speed, frame_receipt_row, middle_truncate, pad_right, push_ledger_group_header,
    render_lines_to_ansi, short_hash, tail_truncate, MicroBarStyle, LEDGER_CAPPED_ROWS,
    LEDGER_MAX_UNTRUNCATED_GROUP,
};
use crate::ui::state::FilterContext;
use crate::utils::{format_duration, format_size};
use crate::verify::{
    VerificationIssue, VerificationReport, VerifiedFileRecord, VerifiedFileStatus,
};

/// Maximum file count for which `--check` on TTY renders every checked file inline below the card.
/// Above this threshold, passing `✓` rows are omitted and issues render in the 3-tier `┃` ledger.
pub const INLINE_CHECK_FILE_LIMIT: usize = 25;

/// Width in cells of the fixed proportional micro-bar column (matches `--list`).
const MICRO_BAR_WIDTH: usize = 8;

/// Spaces preceding the filename column in inline file rows:
/// 10 (size) + 2 + 8 (bar) + 2 + 1 (glyph) + 1 = 24 cells.
const FILENAME_COLUMN_INDENT: &str = "                        ";

/// Builds the Verification Card top border:
/// `╭─ ia-get · check · <identifier> ─── <badge> ─╮`.
fn build_check_top_border(
    identifier: &str,
    report: &VerificationReport,
    card_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let total_issues = report.summary.failed.len();
    let hard_fails = report.fail_count();

    let (badge_text, badge_style) = if report.summary.interrupted {
        (
            format!("{} interrupted", glyphs.interrupted),
            theme.bold_if_color(theme.warn_style()),
        )
    } else if total_issues == 0 {
        (
            format!("{} verified", glyphs.ok),
            theme.bold_if_color(theme.ok_style()),
        )
    } else {
        let label = if total_issues == 1 { "issue" } else { "issues" };
        let (glyph, style) = if hard_fails > 0 {
            (glyphs.fail, theme.bold_if_color(theme.err_style()))
        } else {
            (glyphs.warn, theme.bold_if_color(theme.warn_style()))
        };
        (format!("{glyph} {total_issues} {label}"), style)
    };

    let max_badge = card_width.saturating_sub(10);
    let badge_fit = tail_truncate(&badge_text, max_badge);
    let badge_w = badge_fit.width();

    let left_budget = card_width.saturating_sub(9 + badge_w);
    let full_prefix = format!("ia-get {} check {} ", glyphs.sep, glyphs.sep);
    let full_prefix_w = full_prefix.width();
    let short_prefix = format!("ia-get {} check", glyphs.sep);

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let left_w = if left_budget > full_prefix_w {
        let id_fit = middle_truncate(identifier, left_budget - full_prefix_w, glyphs.ellipsis);
        let id_w = id_fit.width();
        left_spans.push(Span::styled("ia-get", theme.title_style()));
        left_spans.push(Span::styled(
            format!(" {} ", glyphs.sep),
            theme.muted_style(),
        ));
        left_spans.push(Span::styled("check", theme.muted_style()));
        left_spans.push(Span::styled(
            format!(" {} ", glyphs.sep),
            theme.muted_style(),
        ));
        left_spans.push(Span::styled(id_fit, theme.primary_style()));
        full_prefix_w + id_w
    } else if left_budget >= short_prefix.width() {
        left_spans.push(Span::styled("ia-get", theme.title_style()));
        left_spans.push(Span::styled(
            format!(" {} check", glyphs.sep),
            theme.muted_style(),
        ));
        short_prefix.width()
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

/// Builds Row 1 of the Verification Card: proportional outcome bar + `<ok>/<total>  <pct>%`.
fn build_check_row_1(
    report: &VerificationReport,
    inner_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let ok = report.summary.ok;
    let warn = report.warn_count();
    let fail = report.fail_count();
    let checked = ok.saturating_add(warn).saturating_add(fail);
    let total = report.total_files.max(checked);
    let unverified = total.saturating_sub(checked);

    let pct = if total == 0 {
        100u8
    } else {
        ((ok as f64 / total as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };

    let right_label = format!("{ok}/{total}  {pct}%");
    let right_w = right_label.width();
    let bar_width = inner_width.saturating_sub(right_w + 2);

    let widths = allocate_outcome_bar_widths([ok, 0, warn, fail, unverified], bar_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(widths[0] + 6);
    if widths[0] > 0 {
        match theme.color_level {
            ColorLevel::TrueColor => {
                for cell in 0..widths[0] {
                    spans.push(Span::styled(
                        glyphs.bar_ok,
                        theme.bar_fill_style(cell, widths[0], true),
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

/// Builds Row 2 of the Verification Card:
/// `✓ <ok> ok  ! <warn> warn  ✗ <fail> fail` on left; `hashed <bytes> · <elapsed> · <rate>` on right.
fn build_check_row_2(
    report: &VerificationReport,
    inner_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let ok = report.summary.ok;
    let warn = report.warn_count();
    let fail = report.fail_count();

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let mut left_w = 0usize;

    let ok_rest = format!(" {ok} ok");
    left_w += glyphs.ok.width() + ok_rest.width();
    left_spans.push(Span::styled(
        glyphs.ok,
        theme.bold_if_color(theme.ok_style()),
    ));
    left_spans.push(Span::styled(ok_rest, theme.ok_style()));

    if warn > 0 {
        let warn_rest = format!(" {warn} warn");
        left_w += 2 + glyphs.warn.width() + warn_rest.width();
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            glyphs.warn,
            theme.bold_if_color(theme.warn_style()),
        ));
        left_spans.push(Span::styled(warn_rest, theme.warn_style()));
    }
    if fail > 0 {
        let fail_rest = format!(" {fail} fail");
        left_w += 2 + glyphs.fail.width() + fail_rest.width();
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            glyphs.fail,
            theme.bold_if_color(theme.err_style()),
        ));
        left_spans.push(Span::styled(fail_rest, theme.err_style()));
    }

    let bytes_str = format_size(report.total_hashed_bytes);
    let elapsed_str = format_duration(report.elapsed);
    let secs = report.elapsed.as_secs_f64();
    let rate_opt = (report.total_hashed_bytes > 0 && secs > 0.0)
        .then(|| format_speed(report.total_hashed_bytes as f64 / secs));

    let sep = format!(" {} ", glyphs.sep);
    let candidates: Vec<String> = match rate_opt {
        Some(rate) => vec![
            format!("hashed {bytes_str}{sep}{elapsed_str}{sep}{rate}"),
            format!("hashed {bytes_str}{sep}{elapsed_str}"),
            format!("{bytes_str}{sep}{elapsed_str}"),
        ],
        None => vec![
            format!("hashed {bytes_str}{sep}{elapsed_str}"),
            format!("{bytes_str}{sep}{elapsed_str}"),
        ],
    };

    if let Some(right_text) = candidates
        .into_iter()
        .find(|cand| left_w + 2 + cand.width() <= inner_width)
    {
        let pad = inner_width - left_w - right_text.width();
        left_spans.push(Span::raw(" ".repeat(pad)));
        if let Some(rest) = right_text.strip_prefix("hashed ") {
            left_spans.push(Span::styled("hashed ", theme.muted_style()));
            left_spans.push(Span::styled(rest.to_string(), theme.primary_style()));
        } else {
            left_spans.push(Span::styled(right_text, theme.muted_style()));
        }
    }

    frame_receipt_row(left_spans, inner_width, theme)
}

/// Builds Row 3 (or 2 wrapped rows on <= 60 cols) of the Verification Card:
/// `filter: <filters_or_all>` on left; `dir <~-collapsed path>` on right.
fn build_check_row_3_lines(
    output_dir: &str,
    filter: Option<&FilterContext>,
    card_width: usize,
    inner_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let glyphs = theme.glyphs();
    let filter_prefix = "filter: ";
    let filter_full = filter.map_or_else(|| "filter: all".to_string(), FilterContext::label);
    let filter_val = filter_full
        .strip_prefix(filter_prefix)
        .unwrap_or(&filter_full);
    let filter_val_style = if filter.is_some_and(FilterContext::is_active) {
        theme.primary_style()
    } else {
        theme.muted_style()
    };

    let min_dir_w = "dir ".len() + output_dir.width().min(12);
    let single_row_needed = filter_full.width() + 3 + min_dir_w;

    if card_width > 60 && single_row_needed <= inner_width {
        let dir_budget = inner_width.saturating_sub(filter_full.width() + 3 + "dir ".len());
        let dir_trunc = middle_truncate(output_dir, dir_budget, glyphs.ellipsis);
        let dir_w = "dir ".len() + dir_trunc.width();
        let pad = inner_width.saturating_sub(filter_full.width() + dir_w);

        vec![frame_receipt_row(
            vec![
                Span::styled(filter_prefix, theme.muted_style()),
                Span::styled(filter_val.to_string(), filter_val_style),
                Span::raw(" ".repeat(pad)),
                Span::styled("dir ", theme.muted_style()),
                Span::styled(dir_trunc, theme.primary_style()),
            ],
            inner_width,
            theme,
        )]
    } else {
        let filter_budget = inner_width.saturating_sub(filter_prefix.len());
        let filter_fit = middle_truncate(filter_val, filter_budget, glyphs.ellipsis);
        let dir_budget = inner_width.saturating_sub("dir ".len());
        let dir_fit = middle_truncate(output_dir, dir_budget, glyphs.ellipsis);

        vec![
            frame_receipt_row(
                vec![
                    Span::styled(filter_prefix, theme.muted_style()),
                    Span::styled(filter_fit, filter_val_style),
                ],
                inner_width,
                theme,
            ),
            frame_receipt_row(
                vec![
                    Span::styled("dir ", theme.muted_style()),
                    Span::styled(dir_fit, theme.primary_style()),
                ],
                inner_width,
                theme,
            ),
        ]
    }
}

/// Builds the 5-line rounded Verification Card lines for `--check` on TTY.
///
/// Every returned line has exact display width `(width as usize).max(16)`.
pub fn build_check_card_lines(
    identifier: &str,
    output_dir: &str,
    report: &VerificationReport,
    filter: Option<&FilterContext>,
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
    lines.push(build_check_top_border(
        identifier, report, card_width, theme,
    ));
    lines.push(build_check_row_1(report, inner_width, theme));
    lines.push(build_check_row_2(report, inner_width, theme));
    lines.extend(build_check_row_3_lines(
        output_dir,
        filter,
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

/// Formats a concise human-readable inline reason for a non-ok verified file row.
fn concise_issue_summary(
    issues: &[VerificationIssue],
    include_compact_md5: bool,
    glyphs_ne: &str,
) -> String {
    let has_part = issues
        .iter()
        .any(|i| matches!(i, VerificationIssue::LeftoverPart { .. }));
    let has_md5 = issues
        .iter()
        .any(|i| matches!(i, VerificationIssue::Md5Mismatch { .. }));

    let mut parts: Vec<String> = Vec::new();
    for issue in issues {
        match issue {
            VerificationIssue::Missing => parts.push("missing".to_string()),
            VerificationIssue::LeftoverPart { .. } => {
                if !has_md5 {
                    parts.push("leftover .part".to_string());
                }
            }
            VerificationIssue::SizeMismatch { expected, actual } => {
                parts.push(format!(
                    "size mismatch ({} vs {})",
                    format_size(*actual),
                    format_size(*expected)
                ));
            }
            VerificationIssue::Md5Mismatch { expected, actual } => {
                let tag = if has_part {
                    "MD5 mismatch (.part)"
                } else {
                    "MD5 mismatch"
                };
                if include_compact_md5 {
                    parts.push(format!(
                        "{tag}  {} {glyphs_ne} {}",
                        short_hash(actual, 8),
                        short_hash(expected, 8)
                    ));
                } else {
                    parts.push(tag.to_string());
                }
            }
            VerificationIssue::UnreadableTarget { reason } => {
                parts.push(format!("unreadable ({reason})"));
            }
        }
    }
    parts.join(", ")
}

/// Flat folder group of verified records.
struct CheckedFolderGroup<'a> {
    dir: &'a str,
    entries: Vec<(&'a str, &'a VerifiedFileRecord)>,
    ok_count: usize,
}

fn group_records_flat(records: &[VerifiedFileRecord]) -> Vec<CheckedFolderGroup<'_>> {
    let mut map: BTreeMap<&str, Vec<(&str, &VerifiedFileRecord)>> = BTreeMap::new();
    for record in records {
        let (dir, display_name) = split_folder_and_name(&record.file_path);
        map.entry(dir).or_default().push((display_name, record));
    }

    let mut groups: Vec<CheckedFolderGroup<'_>> = map
        .into_iter()
        .map(|(dir, mut entries)| {
            entries.sort_by(|a, b| alphanumeric_cmp(a.0, b.0));
            let ok_count = entries
                .iter()
                .filter(|(_, r)| r.status == VerifiedFileStatus::Ok)
                .count();
            CheckedFolderGroup {
                dir,
                entries,
                ok_count,
            }
        })
        .collect();

    groups.sort_by(|a, b| match (a.dir.is_empty(), b.dir.is_empty()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => alphanumeric_cmp(a.dir, b.dir),
    });

    groups
}

/// Builds inline flat-grouped rows for `<= 25` checked files.
fn build_inline_check_rows(
    records: &[VerifiedFileRecord],
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let glyphs = theme.glyphs();
    let hash_indent = if width >= 65 {
        FILENAME_COLUMN_INDENT
    } else if width >= 48 {
        "    "
    } else {
        "  "
    };
    let max_size: u64 = records
        .iter()
        .filter_map(|r| r.actual_size.or(r.expected_size))
        .max()
        .unwrap_or(0);

    let total_md5_mismatches = records
        .iter()
        .filter(|r| {
            r.issues
                .iter()
                .any(|i| matches!(i, VerificationIssue::Md5Mismatch { .. }))
        })
        .count();
    let expand_hashes = (1..=3).contains(&total_md5_mismatches);

    let groups = group_records_flat(records);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut first_section = true;

    for group in &groups {
        if group.entries.is_empty() {
            continue;
        }
        if !group.dir.is_empty() {
            if !first_section {
                lines.push(Line::default());
            }
            lines.push(Line::from(vec![
                Span::styled(glyphs.folder, theme.bold_if_color(theme.accent_style())),
                Span::raw(" "),
                Span::styled(format!("{}/", group.dir), theme.bold_style()),
                Span::raw("  "),
                Span::styled(
                    format!("({}/{} ok)", group.ok_count, group.entries.len()),
                    theme.muted_style(),
                ),
            ]));
        }
        first_section = false;

        for (display_name, record) in &group.entries {
            let display_size = record.actual_size.or(record.expected_size);
            let (size_text, size_style, ratio) = match display_size {
                Some(bytes) => {
                    let ratio = if max_size > 0
                        && (record.status != VerifiedFileStatus::Failed
                            || record.actual_size.is_some())
                    {
                        Some(bytes as f64 / max_size as f64)
                    } else {
                        None
                    };
                    (
                        format!("{:>10}", format_size(bytes)),
                        if record.actual_size.is_some() {
                            theme.primary_style()
                        } else {
                            theme.muted_style()
                        },
                        ratio,
                    )
                }
                None => (format!("{:>10}", "unknown"), theme.muted_style(), None),
            };

            let (bar_mode, status_glyph, status_style, reason_style) = match record.status {
                VerifiedFileStatus::Ok => (
                    MicroBarStyle::SageOk,
                    glyphs.ok,
                    theme.bold_if_color(theme.ok_style()),
                    theme.muted_style(),
                ),
                VerifiedFileStatus::Warning => (
                    MicroBarStyle::AmberWarn,
                    glyphs.warn,
                    theme.bold_if_color(theme.warn_style()),
                    theme.warn_style(),
                ),
                VerifiedFileStatus::Failed => (
                    MicroBarStyle::CrimsonFail,
                    glyphs.fail,
                    theme.bold_if_color(theme.err_style()),
                    theme.err_style(),
                ),
            };

            let mut spans = Vec::with_capacity(MICRO_BAR_WIDTH + 8);
            spans.push(Span::styled(size_text, size_style));
            spans.push(Span::raw("  "));
            spans.extend(build_micro_bar_spans(
                ratio,
                MICRO_BAR_WIDTH,
                bar_mode,
                theme,
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(status_glyph, status_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                (*display_name).to_string(),
                if record.status == VerifiedFileStatus::Ok {
                    theme.primary_style()
                } else {
                    theme.bold_style()
                },
            ));

            if !record.issues.is_empty() {
                let reason =
                    concise_issue_summary(&record.issues, !expand_hashes, glyphs.not_equal);
                spans.push(Span::raw("  "));
                spans.push(Span::styled(reason, reason_style));
            }
            lines.push(Line::from(spans));

            if expand_hashes {
                for issue in &record.issues {
                    if let VerificationIssue::Md5Mismatch { expected, actual } = issue {
                        lines.push(Line::from(vec![
                            Span::raw(hash_indent),
                            Span::styled("got      ", theme.muted_style()),
                            Span::styled(actual.clone(), theme.accent_style()),
                        ]));
                        lines.push(Line::from(vec![
                            Span::raw(hash_indent),
                            Span::styled("expected ", theme.muted_style()),
                            Span::styled(expected.clone(), theme.muted_style()),
                        ]));
                    }
                }
            }
        }
    }

    lines
}

/// Builds the 3-tier adaptive `┃` ledger for large `--check` runs (`> 25` files) where passing
/// `✓` rows are omitted.
fn build_large_check_ledger(
    records: &[VerifiedFileRecord],
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut hard_failures: Vec<(&str, String)> = Vec::new();
    let mut md5_mismatches: Vec<(&str, &str, &str, bool)> = Vec::new();
    let mut other_warnings: Vec<(&str, String)> = Vec::new();

    for record in records {
        if record.status == VerifiedFileStatus::Ok {
            continue;
        }
        let has_part = record
            .issues
            .iter()
            .any(|i| matches!(i, VerificationIssue::LeftoverPart { .. }));
        if record.status == VerifiedFileStatus::Failed {
            hard_failures.push((
                record.file_path.as_str(),
                concise_issue_summary(&record.issues, false, theme.glyphs().not_equal),
            ));
        } else {
            let mut matched_md5 = false;
            for issue in &record.issues {
                if let VerificationIssue::Md5Mismatch { expected, actual } = issue {
                    md5_mismatches.push((
                        record.file_path.as_str(),
                        actual.as_str(),
                        expected.as_str(),
                        has_part,
                    ));
                    matched_md5 = true;
                }
            }
            if !matched_md5 {
                other_warnings.push((
                    record.file_path.as_str(),
                    concise_issue_summary(&record.issues, false, theme.glyphs().not_equal),
                ));
            }
        }
    }

    if hard_failures.is_empty() && md5_mismatches.is_empty() && other_warnings.is_empty() {
        return Vec::new();
    }

    let total_issues = hard_failures.len() + md5_mismatches.len() + other_warnings.len();
    let expanded = total_issues <= 3;
    let max_width = width as usize;
    let glyphs = theme.glyphs();
    let bar = glyphs.ledger_bar;
    let indent = if max_width >= 48 { "   " } else { " " };
    let prefix_w = bar.width() + indent.len();
    let row_budget = max_width.saturating_sub(prefix_w);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // 1. Missing or unreadable (crimson ✗)
    if !hard_failures.is_empty() {
        let style = theme.err_style();
        let icon_style = theme.bold_if_color(style);
        push_ledger_group_header(
            &mut lines,
            bar,
            style,
            glyphs.fail,
            format!("Missing or unreadable ({})", hard_failures.len()),
            None,
            max_width,
            theme,
        );
        if expanded {
            let msg_indent = if max_width >= 48 { "     " } else { "   " };
            for (path, reason) in &hard_failures {
                let name_budget =
                    max_width.saturating_sub(bar.width() + indent.len() + glyphs.fail.width() + 1);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(glyphs.fail, icon_style),
                    Span::raw(" "),
                    Span::styled(
                        middle_truncate(path, name_budget, glyphs.ellipsis),
                        theme.bold_style(),
                    ),
                ]));
                let msg_budget = max_width.saturating_sub(bar.width() + msg_indent.len());
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(msg_indent),
                    Span::styled(tail_truncate(reason, msg_budget), theme.muted_style()),
                ]));
            }
        } else {
            let shown = if hard_failures.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                LEDGER_CAPPED_ROWS
            } else {
                hard_failures.len()
            };
            let avail = row_budget.saturating_sub(2);
            let max_msg_w = hard_failures[..shown]
                .iter()
                .map(|(_, m)| m.width())
                .max()
                .unwrap_or(0)
                .min(avail / 2);
            let name_budget = avail.saturating_sub(max_msg_w);
            let name_col_w = hard_failures[..shown]
                .iter()
                .map(|(p, _)| p.width())
                .max()
                .unwrap_or(0)
                .min(name_budget);
            for (path, reason) in &hard_failures[..shown] {
                if name_col_w > 0 && max_msg_w > 0 {
                    let name_fit = pad_right(
                        &middle_truncate(path, name_col_w, glyphs.ellipsis),
                        name_col_w,
                    );
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                        Span::raw("  "),
                        Span::styled(tail_truncate(reason, max_msg_w), style),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(
                            middle_truncate(path, row_budget, glyphs.ellipsis),
                            theme.primary_style(),
                        ),
                    ]));
                }
            }
            if hard_failures.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                let more = hard_failures.len() - LEDGER_CAPPED_ROWS;
                let note = format!("+{more} more, re-run with -v for all");
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(tail_truncate(&note, row_budget), theme.muted_style()),
                ]));
            }
        }
    }

    // 2. MD5 mismatches (amber !)
    if !md5_mismatches.is_empty() {
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
            format!("MD5 mismatches ({})", md5_mismatches.len()),
            None,
            max_width,
            theme,
        );
        if expanded {
            let hash_indent = if max_width >= 48 { "     " } else { " " };
            for (path, got, expected, has_part) in &md5_mismatches {
                let suffix = if *has_part { "  (.part)" } else { "" };
                let name_budget = max_width.saturating_sub(
                    bar.width() + indent.len() + glyphs.warn.width() + 1 + suffix.width(),
                );
                let mut row = vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(glyphs.warn, icon_style),
                    Span::raw(" "),
                    Span::styled(
                        middle_truncate(path, name_budget, glyphs.ellipsis),
                        theme.bold_style(),
                    ),
                ];
                if *has_part {
                    row.push(Span::styled("  (.part)", theme.muted_style()));
                }
                lines.push(Line::from(row));

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
            let shown = if md5_mismatches.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                LEDGER_CAPPED_ROWS
            } else {
                md5_mismatches.len()
            };
            let show_part_tag = max_width >= 68;
            let max_hash_w = md5_mismatches[..shown]
                .iter()
                .map(|(_, got, expected, has_part)| {
                    let part_w = if show_part_tag && *has_part { 9 } else { 0 };
                    short_hash(got, 8).width()
                        + 1
                        + glyphs.not_equal.width()
                        + 1
                        + short_hash(expected, 8).width()
                        + part_w
                })
                .max()
                .unwrap_or(0);
            let name_budget = row_budget.saturating_sub(2 + max_hash_w);
            let name_col_w = md5_mismatches[..shown]
                .iter()
                .map(|(p, _, _, _)| p.width())
                .max()
                .unwrap_or(0)
                .min(name_budget);

            for (path, got, expected, has_part) in &md5_mismatches[..shown] {
                if name_col_w > 0 && row_budget > 2 + max_hash_w {
                    let name_fit = pad_right(
                        &middle_truncate(path, name_col_w, glyphs.ellipsis),
                        name_col_w,
                    );
                    let mut row = vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                        Span::raw("  "),
                        Span::styled(short_hash(got, 8), theme.accent_style()),
                        Span::styled(format!(" {} ", glyphs.not_equal), style),
                        Span::styled(short_hash(expected, 8), theme.muted_style()),
                    ];
                    if show_part_tag && *has_part {
                        row.push(Span::styled("  (.part)", theme.muted_style()));
                    }
                    lines.push(Line::from(row));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(
                            middle_truncate(path, row_budget, glyphs.ellipsis),
                            theme.primary_style(),
                        ),
                    ]));
                }
            }
            if md5_mismatches.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                let more = md5_mismatches.len() - LEDGER_CAPPED_ROWS;
                let note = format!("+{more} more, re-run with -v for all");
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(tail_truncate(&note, row_budget), theme.muted_style()),
                ]));
            }
        }
    }

    // 3. Size mismatches & leftover .part (amber !)
    if !other_warnings.is_empty() {
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
            format!("Size or partial file issues ({})", other_warnings.len()),
            None,
            max_width,
            theme,
        );
        if expanded {
            let msg_indent = if max_width >= 48 { "     " } else { "   " };
            for (path, reason) in &other_warnings {
                let name_budget =
                    max_width.saturating_sub(bar.width() + indent.len() + glyphs.warn.width() + 1);
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(glyphs.warn, icon_style),
                    Span::raw(" "),
                    Span::styled(
                        middle_truncate(path, name_budget, glyphs.ellipsis),
                        theme.bold_style(),
                    ),
                ]));
                let msg_budget = max_width.saturating_sub(bar.width() + msg_indent.len());
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(msg_indent),
                    Span::styled(tail_truncate(reason, msg_budget), theme.muted_style()),
                ]));
            }
        } else {
            let shown = if other_warnings.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                LEDGER_CAPPED_ROWS
            } else {
                other_warnings.len()
            };
            let avail = row_budget.saturating_sub(2);
            let max_msg_w = other_warnings[..shown]
                .iter()
                .map(|(_, m)| m.width())
                .max()
                .unwrap_or(0)
                .min(avail / 2);
            let name_budget = avail.saturating_sub(max_msg_w);
            let name_col_w = other_warnings[..shown]
                .iter()
                .map(|(p, _)| p.width())
                .max()
                .unwrap_or(0)
                .min(name_budget);
            for (path, reason) in &other_warnings[..shown] {
                if name_col_w > 0 && max_msg_w > 0 {
                    let name_fit = pad_right(
                        &middle_truncate(path, name_col_w, glyphs.ellipsis),
                        name_col_w,
                    );
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(name_fit, theme.primary_style()),
                        Span::raw("  "),
                        Span::styled(tail_truncate(reason, max_msg_w), style),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(bar, style),
                        Span::raw(indent),
                        Span::styled(
                            middle_truncate(path, row_budget, glyphs.ellipsis),
                            theme.primary_style(),
                        ),
                    ]));
                }
            }
            if other_warnings.len() > LEDGER_MAX_UNTRUNCATED_GROUP {
                let more = other_warnings.len() - LEDGER_CAPPED_ROWS;
                let note = format!("+{more} more, re-run with -v for all");
                lines.push(Line::from(vec![
                    Span::styled(bar, style),
                    Span::raw(indent),
                    Span::styled(tail_truncate(&note, row_budget), theme.muted_style()),
                ]));
            }
        }
    }

    lines
        .into_iter()
        .map(|line| clip_line_to_max_width(line, max_width))
        .collect()
}

/// Builds the complete interactive TTY `--check` output: Verification Card + inline listing
/// (`<= 25` files) or 3-tier adaptive `┃` ledger (`> 25` files).
pub fn build_check_tty_lines(
    identifier: &str,
    output_dir: &str,
    report: &VerificationReport,
    filter: Option<&FilterContext>,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = build_check_card_lines(identifier, output_dir, report, filter, theme, width);

    if report.summary.interrupted {
        let hint = "Run the command again to verify remaining files.";
        lines.push(Line::from(Span::styled(
            tail_truncate(hint, (width as usize).max(16)),
            theme.muted_style(),
        )));
    }

    if report.records.is_empty() {
        return lines;
    }

    if report.total_files <= INLINE_CHECK_FILE_LIMIT {
        lines.push(Line::default());
        lines.extend(build_inline_check_rows(&report.records, theme, width));
    } else {
        let ledger = build_large_check_ledger(&report.records, theme, width);
        if !ledger.is_empty() {
            lines.push(Line::default());
            lines.extend(ledger);
        }
    }

    lines
}

/// Formats the interactive TTY `--check` Verification Suite as an ANSI string.
pub fn format_check_tty(
    identifier: &str,
    output_dir: &str,
    report: &VerificationReport,
    filter: Option<&FilterContext>,
    theme: &Theme,
    width: u16,
) -> String {
    let lines = build_check_tty_lines(identifier, output_dir, report, filter, theme, width);
    render_lines_to_ansi(&lines, theme)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ui::render::render_lines_to_plain;
    use crate::verify::{FailedVerification, VerificationSummary};

    fn make_ok_record(path: &str, size: u64) -> VerifiedFileRecord {
        VerifiedFileRecord {
            file_path: path.to_string(),
            status: VerifiedFileStatus::Ok,
            expected_size: Some(size),
            actual_size: Some(size),
            hashed_bytes: size,
            issues: Vec::new(),
        }
    }

    fn make_md5_record(
        path: &str,
        size: u64,
        actual: &str,
        expected: &str,
        with_part: bool,
    ) -> VerifiedFileRecord {
        let mut issues = Vec::new();
        if with_part {
            issues.push(VerificationIssue::LeftoverPart {
                part_path: format!("{path}.part"),
                size: size / 2,
            });
        }
        issues.push(VerificationIssue::Md5Mismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
        VerifiedFileRecord {
            file_path: path.to_string(),
            status: VerifiedFileStatus::Warning,
            expected_size: Some(size),
            actual_size: Some(size),
            hashed_bytes: size,
            issues,
        }
    }

    fn make_missing_record(path: &str, expected: u64) -> VerifiedFileRecord {
        VerifiedFileRecord {
            file_path: path.to_string(),
            status: VerifiedFileStatus::Failed,
            expected_size: Some(expected),
            actual_size: None,
            hashed_bytes: 0,
            issues: vec![VerificationIssue::Missing],
        }
    }

    fn report_from_records(
        records: Vec<VerifiedFileRecord>,
        total_files: usize,
        interrupted: bool,
    ) -> VerificationReport {
        let ok = records
            .iter()
            .filter(|r| r.status == VerifiedFileStatus::Ok)
            .count();
        let failed: Vec<FailedVerification> = records
            .iter()
            .filter(|r| !r.issues.is_empty())
            .map(|r| FailedVerification {
                file_path: r.file_path.clone(),
                issues: r.issues.clone(),
            })
            .collect();
        let total_hashed_bytes: u64 = records.iter().map(|r| r.hashed_bytes).sum();
        VerificationReport {
            summary: VerificationSummary {
                ok,
                failed,
                interrupted,
            },
            records,
            total_files,
            total_hashed_bytes,
            elapsed: Duration::from_millis(420),
        }
    }

    #[test]
    fn check_tty_clean_2_file_renders_verified_card_and_ok_rows() {
        let records = vec![
            make_ok_record("deftributetozzap64_djvu.txt", 312_000),
            make_ok_record("deftributetozzap64_djvu.xml", 4_200_000),
        ];
        let report = report_from_records(records, 2, false);
        let filter = FilterContext {
            patterns: vec!["*djvu.*".to_string()],
            matched: 2,
            total: 12,
        };
        let theme = Theme::archival_truecolor();

        for width in [120u16, 80, 60] {
            let card = build_check_card_lines(
                "deftributetozzap64",
                "~/deftributetozzap64",
                &report,
                Some(&filter),
                &theme,
                width,
            );
            let card_plain = render_lines_to_plain(&card);
            for (idx, row) in card_plain.iter().enumerate() {
                assert_eq!(
                    row.width(),
                    width as usize,
                    "width {width} card row {idx} mismatch: {row:?}"
                );
            }
            assert!(card_plain[0].contains("ia-get · check · deftributetozzap64"));
            assert!(card_plain[0].contains("✓ verified"));
            let expected_rows = if width <= 60 { 6 } else { 5 };
            assert_eq!(
                card_plain.len(),
                expected_rows,
                "width {width} verification card must have exact height {expected_rows}:\n{}",
                card_plain.join("\n")
            );
            for inner_row in &card_plain[1..card_plain.len() - 1] {
                let inner = inner_row
                    .strip_prefix('│')
                    .and_then(|r| r.strip_suffix('│'))
                    .expect("inner card row must be bordered");
                assert!(
                    !inner.trim().is_empty(),
                    "width {width} verification card must have zero blank inner rows:\n{}",
                    card_plain.join("\n")
                );
            }
        }

        let lines = build_check_tty_lines(
            "deftributetozzap64",
            "~/deftributetozzap64",
            &report,
            Some(&filter),
            &theme,
            80,
        );
        let joined = render_lines_to_plain(&lines).join("\n");
        assert!(joined.contains("2/2  100%"), "{joined}");
        assert!(joined.contains("✓ 2 ok"), "{joined}");
        assert!(joined.contains("filter: *djvu.* (2 of 12)"), "{joined}");
        assert!(joined.contains("dir ~/deftributetozzap64"), "{joined}");
        assert!(joined.contains("✓ deftributetozzap64_djvu.txt"), "{joined}");
        assert!(joined.contains("✓ deftributetozzap64_djvu.xml"), "{joined}");
    }

    #[test]
    fn check_tty_mixed_12_file_shows_all_rows_and_tucked_32_char_hashes() {
        let got_hex = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let exp_hex = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";
        let mut records = Vec::new();
        for i in 1..=9 {
            records.push(make_ok_record(
                &format!("docs/page_{i:02}.pdf"),
                i * 100_000,
            ));
        }
        records.push(make_md5_record(
            "docs/corrupt.pdf",
            500_000,
            got_hex,
            exp_hex,
            true,
        ));
        records.push(make_missing_record("cover.jpg", 250_000));
        records.push(VerifiedFileRecord {
            file_path: "truncated.bin".to_string(),
            status: VerifiedFileStatus::Warning,
            expected_size: Some(2048),
            actual_size: Some(1024),
            hashed_bytes: 0,
            issues: vec![VerificationIssue::SizeMismatch {
                expected: 2048,
                actual: 1024,
            }],
        });

        let theme = Theme::archival_truecolor();
        let report = report_from_records(records, 12, false);
        let lines = build_check_tty_lines("mixed-item", "~/mixed", &report, None, &theme, 80);
        let plain = render_lines_to_plain(&lines);
        let joined = plain.join("\n");

        // Hard failures present -> crimson `✗ 3 issues` badge.
        assert!(joined.contains("✗ 3 issues"), "{joined}");
        let fail_badge = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("✗ 3 issues"))
            .expect("missing hard-failure badge span");
        assert_eq!(fail_badge.style, theme.bold_if_color(theme.err_style()));
        assert!(joined.contains("✓ 9 ok  ! 2 warn  ✗ 1 fail"), "{joined}");
        assert!(
            plain.iter().any(|l| l.starts_with("▼ docs/  (9/10 ok)")),
            "{joined}"
        );
        assert!(
            joined.contains("! corrupt.pdf  MD5 mismatch (.part)"),
            "{joined}"
        );
        assert!(joined.contains("✗ cover.jpg  missing"), "{joined}");
        assert!(
            joined.contains("! truncated.bin  size mismatch"),
            "{joined}"
        );
        // Aligned 32-char hashes tucked beneath filename column (24 spaces) at >= 65 cols.
        assert!(
            plain
                .iter()
                .any(|l| l == &format!("{FILENAME_COLUMN_INDENT}got      {got_hex}")),
            "expected aligned 32-char got hash:\n{joined}"
        );
        assert!(
            plain
                .iter()
                .any(|l| l == &format!("{FILENAME_COLUMN_INDENT}expected {exp_hex}")),
            "expected aligned 32-char expected hash:\n{joined}"
        );

        // At 60 cols (< 65), expanded MD5 hashes use 4-space indent so they never wrap.
        let lines_60 = build_check_tty_lines("mixed-item", "~/mixed", &report, None, &theme, 60);
        let plain_60 = render_lines_to_plain(&lines_60);
        let joined_60 = plain_60.join("\n");
        let got_60 = format!("    got      {got_hex}");
        let exp_60 = format!("    expected {exp_hex}");
        assert!(got_60.width() <= 60 && exp_60.width() <= 60);
        assert!(
            plain_60.iter().any(|l| l == &got_60),
            "60-col inline got hash must use 4-space indent:\n{joined_60}"
        );
        assert!(
            plain_60.iter().any(|l| l == &exp_60),
            "60-col inline expected hash must use 4-space indent:\n{joined_60}"
        );

        // Below 48 cols (44 cols), expanded MD5 hashes use 2-space indent.
        let lines_44 = build_check_tty_lines("mixed-item", "~/mixed", &report, None, &theme, 44);
        let plain_44 = render_lines_to_plain(&lines_44);
        assert!(plain_44
            .iter()
            .any(|l| l == &format!("  got      {got_hex}")));
        assert!(plain_44
            .iter()
            .any(|l| l == &format!("  expected {exp_hex}")));

        // Warning-only report -> amber `! 1 issue` badge.
        let warn_only_report = report_from_records(
            vec![
                make_ok_record("ok.pdf", 100_000),
                make_md5_record("corrupt.pdf", 200_000, got_hex, exp_hex, false),
            ],
            2,
            false,
        );
        let warn_card =
            build_check_card_lines("warn-only", "~/warn", &warn_only_report, None, &theme, 80);
        let warn_badge = warn_card[0]
            .spans
            .iter()
            .find(|s| s.content.contains("! 1 issue"))
            .expect("missing warning-only badge span");
        assert_eq!(warn_badge.style, theme.bold_if_color(theme.warn_style()));
    }

    #[test]
    fn check_tty_over_25_files_omits_ok_rows_and_uses_adaptive_ledger() {
        let got_hex = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let exp_hex = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";

        // 17 mismatches in a 40-file check -> compact aligned 8-char `got ≠ exp`, zero ✓ rows.
        let mut records_17 = Vec::new();
        for i in 1..=23 {
            records_17.push(make_ok_record(&format!("ok_{i:02}.pdf"), 100_000));
        }
        for i in 1..=17 {
            records_17.push(make_md5_record(
                &format!("bad_{i:02}.pdf"),
                200_000,
                got_hex,
                exp_hex,
                false,
            ));
        }
        let rep_17 = report_from_records(records_17, 40, false);
        let out_17 = render_lines_to_plain(&build_check_tty_lines(
            "large-17",
            "~/large",
            &rep_17,
            None,
            &Theme::archival_truecolor(),
            80,
        ))
        .join("\n");
        assert!(
            !out_17.contains("ok_01.pdf"),
            "passing rows must be omitted:\n{out_17}"
        );
        assert!(out_17.contains("MD5 mismatches (17)"), "{out_17}");
        assert!(out_17.contains("a1b2c3d4 ≠ 9f8e7d6c"), "{out_17}");
        assert!(!out_17.contains("more, re-run with -v"), "{out_17}");

        // 25 mismatches in a 30-file check -> capped at 15 + `+10 more, re-run with -v for all`.
        let mut records_25 = Vec::new();
        for i in 1..=5 {
            records_25.push(make_ok_record(&format!("ok_{i:02}.pdf"), 100_000));
        }
        for i in 1..=25 {
            records_25.push(make_md5_record(
                &format!("bad_{i:02}.pdf"),
                200_000,
                got_hex,
                exp_hex,
                false,
            ));
        }
        let rep_25 = report_from_records(records_25, 30, false);
        let out_25 = render_lines_to_plain(&build_check_tty_lines(
            "large-25",
            "~/large",
            &rep_25,
            None,
            &Theme::archival_truecolor(),
            80,
        ))
        .join("\n");
        assert!(out_25.contains("MD5 mismatches (25)"), "{out_25}");
        assert_eq!(out_25.matches("bad_").count(), 15, "{out_25}");
        assert!(
            out_25.contains("+10 more, re-run with -v for all"),
            "{out_25}"
        );
    }

    #[test]
    fn check_tty_interrupted_no_color_and_ascii_render_cleanly() {
        let records = vec![make_ok_record("done.pdf", 50_000)];
        let report = report_from_records(records, 4, true);

        let mono = format_check_tty(
            "interrupted-item",
            "~/dir",
            &report,
            None,
            &Theme::monochrome_unicode(),
            80,
        );
        assert!(!mono.contains('\x1b'));
        assert!(mono.contains("■ interrupted"), "{mono}");
        assert!(mono.contains("1/4  25%"), "{mono}");
        assert!(
            mono.contains("Run the command again to verify remaining files."),
            "{mono}"
        );

        let ascii = format_check_tty(
            "interrupted-item",
            "~/dir",
            &report,
            None,
            &Theme::monochrome_ascii(),
            60,
        );
        assert!(!ascii.contains('\x1b'));
        assert!(ascii.is_ascii(), "ASCII check must be pure ASCII:\n{ascii}");
        assert!(ascii.contains("# interrupted"), "{ascii}");
        assert!(ascii.contains("+ 1 ok"), "{ascii}");
    }
}
