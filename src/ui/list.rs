//! Hybrid Archival Manifest Suite renderer for `--list` on interactive TTY terminals.
//!
//! Non-TTY streams continue using the plain raw XML listing in `src/main.rs`.

use std::collections::BTreeMap;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::archive_metadata::{XmlFile, XmlFiles};
use crate::ui::palette::{ColorLevel, Theme};
use crate::ui::render::{
    alphanumeric_cmp, build_micro_bar_spans, frame_receipt_row, middle_truncate,
    render_lines_to_ansi, tail_truncate, MicroBarStyle,
};
use crate::ui::state::FilterContext;
use crate::utils::format_size;

/// File-count threshold above which TTY `--list` switches to large-archive preview scaling.
pub const LARGE_ARCHIVE_THRESHOLD: usize = 40;

/// Maximum individual file rows displayed on TTY when `files.len() > LARGE_ARCHIVE_THRESHOLD`.
pub const LARGE_ARCHIVE_FILE_CAP: usize = 20;

/// Maximum folders shown in the large-archive Folders breakdown before `+K more folders`.
pub const LARGE_ARCHIVE_FOLDER_CAP: usize = 10;

/// Width in cells of the fixed proportional micro-bar column.
const MICRO_BAR_WIDTH: usize = 8;

/// Leading spaces aligning `+N more folders` directly under folder entries:
/// 10 (size) + 2 + 8 (bar) + 2 = 22 cells.
const FOLDER_OVERFLOW_INDENT: &str = "                      ";

/// Coarse semantic file category for verified width-1 file-type glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Archive,
    Document,
    Media,
    Image,
    Data,
    Other,
}

impl FileCategory {
    /// Classifies an archive filename by its lowercased last extension.
    pub fn classify(path: &str) -> Self {
        let Some(ext) = extract_extension(path) else {
            return Self::Other;
        };
        match ext.as_str() {
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "iso" | "chd" | "zst" => {
                Self::Archive
            }
            "pdf" | "djvu" | "epub" | "txt" | "md" | "rtf" | "cbz" | "cbr" | "doc" | "docx"
            | "html" => Self::Document,
            "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "mp4" | "mkv" | "avi" | "webm"
            | "mov" => Self::Media,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "jp2" | "svg" => {
                Self::Image
            }
            "xml" | "json" | "sqlite" | "db" | "torrent" | "csv" | "yaml" | "yml" | "sha1"
            | "md5" => Self::Data,
            _ => Self::Other,
        }
    }

    /// Returns `(glyph, style)` for this file category in `theme`.
    pub fn glyph_and_style(self, theme: &Theme) -> (&'static str, Style) {
        let g = theme.glyphs();
        match self {
            Self::Archive => (g.file_archive, theme.file_archive_style()),
            Self::Document => (g.file_doc, theme.file_doc_style()),
            Self::Media => (g.file_media, theme.file_media_style()),
            Self::Image => (g.file_image, theme.file_image_style()),
            Self::Data => (g.file_data, theme.file_data_style()),
            Self::Other => (g.file_other, theme.file_other_style()),
        }
    }
}

/// Extracts the lowercased last extension of a path's basename (ignoring dotfiles like `.env`).
fn extract_extension(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    let basename = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let (stem, ext) = basename.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Splits an archive relative path into `(folder_key, display_name)`.
///
/// Root files return `("", trimmed_name)`; nested files return `("dir/sub", "file.ext")`.
/// Trailing slashes (e.g. `"dir/sub/"`) stay unsplit so no blank filename row is emitted, and
/// redundant trailing slashes on `dir` (e.g. `"dir//file.txt"`) are trimmed.
pub(crate) fn split_folder_and_name(raw: &str) -> (&str, &str) {
    let trimmed = raw.trim_start_matches('/');
    match trimmed.rsplit_once('/') {
        Some((dir, base)) if !base.is_empty() => {
            let dir = dir.trim_end_matches('/');
            if !dir.is_empty() {
                (dir, base)
            } else {
                ("", base)
            }
        }
        _ => ("", trimmed),
    }
}

/// Allocates `bar_width` cells proportionally across up to 3 extension byte buckets,
/// guaranteeing every non-zero bucket gets at least 1 cell whenever width permits.
fn allocate_ribbon_widths(buckets: [u64; 3], bar_width: usize) -> [usize; 3] {
    let mut widths = [0usize; 3];
    if bar_width == 0 {
        return widths;
    }
    let total: u128 = buckets.iter().map(|&v| u128::from(v)).sum();
    if total == 0 {
        return widths;
    }

    let mut remainders = [0.0f64; 3];
    for i in 0..3 {
        if buckets[i] == 0 {
            continue;
        }
        let exact = buckets[i] as f64 * bar_width as f64 / total as f64;
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
        let mut order: Vec<usize> = (0..3).filter(|&i| buckets[i] > 0).collect();
        order.sort_by(|&a, &b| {
            remainders[b]
                .partial_cmp(&remainders[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| buckets[b].cmp(&buckets[a]))
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

/// Computes sorted `(extension, total_bytes)` descending by known bytes (tie-break alphabetical).
fn extension_byte_breakdown(files: &[XmlFile]) -> Vec<(String, u64)> {
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    for file in files {
        let Some(size) = file.size.filter(|&s| s > 0) else {
            continue;
        };
        let ext = extract_extension(&file.name).unwrap_or_else(|| "other".to_string());
        *totals.entry(ext).or_insert(0) += size;
    }
    let mut ranked: Vec<(String, u64)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

/// Builds the Manifest Card top border:
/// `╭─ ia-get · <identifier> ─── <N> files · <total_size> ─╮`.
fn build_manifest_top_border(
    identifier: &str,
    file_count: usize,
    total_known_bytes: u64,
    has_known_size: bool,
    card_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let glyphs = theme.glyphs();
    let file_unit = if file_count == 1 { "file" } else { "files" };
    let size_part = if has_known_size {
        format_size(total_known_bytes)
    } else {
        "unknown size".to_string()
    };
    let full_badge = format!("{file_count} {file_unit} {} {size_part}", glyphs.sep);
    let compact_badge = format!("{file_count} {file_unit}");

    let max_badge = card_width.saturating_sub(14);
    let badge_text = if full_badge.width() <= max_badge {
        full_badge
    } else {
        tail_truncate(&compact_badge, card_width.saturating_sub(10))
    };
    let badge_w = badge_text.width();

    let left_budget = card_width.saturating_sub(9 + badge_w);
    let prefix = format!("ia-get {} ", glyphs.sep);
    let prefix_w = prefix.width();

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let left_w = if left_budget > prefix_w {
        let id_fit = middle_truncate(identifier, left_budget - prefix_w, glyphs.ellipsis);
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
    spans.push(Span::styled(badge_text, theme.bold_style()));
    spans.push(Span::styled(
        format!(" {}{}", glyphs.horizontal, glyphs.top_right),
        theme.border_style(),
    ));
    Line::from(spans)
}

/// Builds Row 1 of the Manifest Card: proportional top-extension composition ribbon + legend.
fn build_manifest_row_1(files: &[XmlFile], inner_width: usize, theme: &Theme) -> Line<'static> {
    let glyphs = theme.glyphs();
    let ranked = extension_byte_breakdown(files);
    let total_known: u64 = ranked.iter().map(|(_, b)| *b).sum();

    if total_known == 0 {
        let label = if files.iter().any(|f| f.size.is_none()) {
            "unknown sizes"
        } else {
            "0B"
        };
        let label_fit = tail_truncate(label, inner_width);
        let label_w = label_fit.width();
        let bar_w = inner_width.saturating_sub(label_w + 2);
        let mut spans = Vec::new();
        if bar_w > 0 {
            spans.push(Span::styled(
                glyphs.bar_empty.repeat(bar_w),
                theme.track_style(),
            ));
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(label_fit, theme.muted_style()));
        return frame_receipt_row(spans, inner_width, theme);
    }

    let pct_of = |bytes: u64| -> u8 {
        ((bytes as f64 / total_known as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };

    let full_legend = if ranked.len() >= 2 {
        format!(
            "{} {}% {} {} {}%",
            ranked[0].0,
            pct_of(ranked[0].1),
            glyphs.sep,
            ranked[1].0,
            pct_of(ranked[1].1)
        )
    } else {
        format!("{} {}%", ranked[0].0, pct_of(ranked[0].1))
    };
    let short_legend = format!("{} {}%", ranked[0].0, pct_of(ranked[0].1));

    let legend = if inner_width >= full_legend.width() + 10 {
        full_legend
    } else if inner_width >= short_legend.width() + 6 {
        short_legend
    } else {
        tail_truncate(&short_legend, inner_width)
    };
    let legend_w = legend.width();
    let bar_width = inner_width.saturating_sub(legend_w + 2);

    let b0 = ranked.first().map_or(0, |(_, b)| *b);
    let b1 = ranked.get(1).map_or(0, |(_, b)| *b);
    let b2: u64 = ranked.iter().skip(2).map(|(_, b)| *b).sum();
    let widths = allocate_ribbon_widths([b0, b1, b2], bar_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(widths[0] + 6);
    if widths[0] > 0 {
        match theme.color_level {
            ColorLevel::TrueColor => {
                for cell in 0..widths[0] {
                    spans.push(Span::styled(
                        glyphs.ribbon_fills[0],
                        theme.bar_fill_style(cell, widths[0], false),
                    ));
                }
            }
            ColorLevel::Ansi16 => {
                spans.push(Span::styled(
                    glyphs.ribbon_fills[0].repeat(widths[0]),
                    theme.accent_style(),
                ));
            }
            ColorLevel::Monochrome => {
                spans.push(Span::styled(
                    glyphs.ribbon_fills[0].repeat(widths[0]),
                    Style::default(),
                ));
            }
        }
    }
    if widths[1] > 0 {
        spans.push(Span::styled(
            glyphs.ribbon_fills[1].repeat(widths[1]),
            theme.info_style(),
        ));
    }
    if widths[2] > 0 {
        spans.push(Span::styled(
            glyphs.ribbon_fills[2].repeat(widths[2]),
            theme.muted_style(),
        ));
    }

    let used_bar: usize = widths.iter().sum();
    let pad = inner_width.saturating_sub(used_bar + legend_w);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.push(Span::styled(legend, theme.muted_style()));

    frame_receipt_row(spans, inner_width, theme)
}

/// Builds Row 2 (wrapping cleanly onto an extra row only when narrow) of the Manifest Card:
/// `known <size>   <U> unknown size(s)   <P> private` on left, `filter: ...` on right.
fn build_manifest_row_2_lines(
    total_known_bytes: u64,
    unknown_count: usize,
    private_count: usize,
    filter: Option<&FilterContext>,
    card_width: usize,
    inner_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let glyphs = theme.glyphs();
    let known_val = format_size(total_known_bytes);
    let unknown_label = if unknown_count == 1 {
        "unknown size"
    } else {
        "unknown sizes"
    };

    let build_left = |gap: &str| -> (Vec<Span<'static>>, usize) {
        let mut spans = vec![
            Span::styled("known ", theme.muted_style()),
            Span::styled(known_val.clone(), theme.primary_style()),
        ];
        let mut w = "known ".len() + known_val.width();

        if unknown_count > 0 {
            let u_num = unknown_count.to_string();
            let u_suffix = format!(" {unknown_label}");
            w += gap.len() + u_num.width() + u_suffix.width();
            spans.push(Span::raw(gap.to_string()));
            spans.push(Span::styled(u_num, theme.primary_style()));
            spans.push(Span::styled(u_suffix, theme.muted_style()));
        }

        if private_count > 0 {
            let p_num = private_count.to_string();
            let p_suffix = " private";
            w += gap.len() + p_num.width() + p_suffix.len();
            spans.push(Span::raw(gap.to_string()));
            spans.push(Span::styled(p_num, theme.bold_if_color(theme.warn_style())));
            spans.push(Span::styled(p_suffix, theme.muted_style()));
        }

        (spans, w)
    };

    let filter_text = filter.map_or_else(|| "filter: all".to_string(), FilterContext::label);
    let filter_prefix = "filter: ";
    let filter_value = filter_text
        .strip_prefix(filter_prefix)
        .unwrap_or(&filter_text);

    let (left_spans_3, left_w_3) = build_left("   ");
    let (left_spans, left_w) = if left_w_3 + 2 + filter_text.width() <= inner_width {
        (left_spans_3, left_w_3)
    } else {
        build_left("  ")
    };

    let min_filter_w = filter_prefix.len() + filter_value.width().min(10);
    if card_width >= 48 && left_w + 2 + min_filter_w <= inner_width {
        let filter_budget = inner_width.saturating_sub(left_w + 2 + filter_prefix.len());
        let val_fit = middle_truncate(filter_value, filter_budget, glyphs.ellipsis);
        let right_w = filter_prefix.len() + val_fit.width();
        let pad = inner_width.saturating_sub(left_w + right_w);
        let mut spans = left_spans;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(filter_prefix, theme.muted_style()));
        spans.push(Span::styled(
            val_fit,
            if filter.is_some_and(FilterContext::is_active) {
                theme.primary_style()
            } else {
                theme.muted_style()
            },
        ));
        vec![frame_receipt_row(spans, inner_width, theme)]
    } else {
        // Ultra-narrow (< 48 cols) or very long left/filter text: wrap cleanly onto 2 inner rows.
        let left_row = if left_w <= inner_width {
            left_spans
        } else {
            let mut parts = vec![format!("known {known_val}")];
            if unknown_count > 0 {
                parts.push(format!("{unknown_count} unk"));
            }
            if private_count > 0 {
                parts.push(format!("{private_count} priv"));
            }
            let compact = parts.join("  ");
            vec![Span::styled(
                tail_truncate(&compact, inner_width),
                theme.muted_style(),
            )]
        };
        let val_budget = inner_width.saturating_sub(filter_prefix.len());
        let val_fit = middle_truncate(filter_value, val_budget, glyphs.ellipsis);
        let right_row = vec![
            Span::styled(filter_prefix, theme.muted_style()),
            Span::styled(
                val_fit,
                if filter.is_some_and(FilterContext::is_active) {
                    theme.primary_style()
                } else {
                    theme.muted_style()
                },
            ),
        ];
        vec![
            frame_receipt_row(left_row, inner_width, theme),
            frame_receipt_row(right_row, inner_width, theme),
        ]
    }
}

/// Builds the rounded Manifest Card lines for `--list` on TTY.
///
/// Every returned line has exact display width `card_width` (`(width as usize).max(16)`).
pub fn build_list_manifest_card_lines(
    identifier: &str,
    files: &XmlFiles,
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

    let total_known_bytes: u64 = files.files.iter().filter_map(|f| f.size).sum();
    let has_known_size = files.files.iter().any(|f| f.size.is_some()) || files.files.is_empty();
    let unknown_count = files.files.iter().filter(|f| f.size.is_none()).count();
    let private_count = files.files.iter().filter(|f| f.is_private()).count();

    let mut lines = Vec::with_capacity(5);
    lines.push(build_manifest_top_border(
        identifier,
        files.files.len(),
        total_known_bytes,
        has_known_size,
        card_width,
        theme,
    ));
    lines.push(build_manifest_row_1(&files.files, inner_width, theme));
    lines.extend(build_manifest_row_2_lines(
        total_known_bytes,
        unknown_count,
        private_count,
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

/// A flat folder group containing sorted archive files.
struct FolderGroup<'a> {
    dir: &'a str,
    entries: Vec<(&'a str, &'a XmlFile)>,
    total_known_bytes: u64,
    has_known_size: bool,
}

/// Groups archive entries into root (`""`) first, then subdirectories sorted case-insensitively
/// alphanumerically, with each folder's files also sorted case-insensitively alphanumerically.
fn group_files_flat(files: &[XmlFile]) -> Vec<FolderGroup<'_>> {
    let mut map: BTreeMap<&str, Vec<(&str, &XmlFile)>> = BTreeMap::new();
    for file in files {
        let (dir, display_name) = split_folder_and_name(&file.name);
        map.entry(dir).or_default().push((display_name, file));
    }

    let mut groups: Vec<FolderGroup<'_>> = map
        .into_iter()
        .map(|(dir, mut entries)| {
            entries.sort_by(|a, b| alphanumeric_cmp(a.0, b.0));
            let total_known_bytes: u64 = entries.iter().filter_map(|(_, f)| f.size).sum();
            let has_known_size = entries.iter().any(|(_, f)| f.size.is_some());
            FolderGroup {
                dir,
                entries,
                total_known_bytes,
                has_known_size,
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

/// Renders one folder group header (`▼ <dir>/  (<N> file(s) · <subtotal>)`) with zero indent.
fn build_folder_header_line(group: &FolderGroup<'_>, theme: &Theme) -> Line<'static> {
    let glyphs = theme.glyphs();
    let count = group.entries.len();
    let unit = if count == 1 { "file" } else { "files" };
    let subtotal = if group.has_known_size {
        format_size(group.total_known_bytes)
    } else {
        "unknown".to_string()
    };
    Line::from(vec![
        Span::styled(glyphs.folder, theme.bold_if_color(theme.accent_style())),
        Span::raw(" "),
        Span::styled(format!("{}/", group.dir), theme.bold_style()),
        Span::raw("  "),
        Span::styled(
            format!("({count} {unit} {} {subtotal})", glyphs.sep),
            theme.muted_style(),
        ),
    ])
}

/// Renders one copy-clean file row (no right border):
/// `<size:>10>  <8-cell micro-bar>  <type_glyph> <name>[  priv]`.
fn build_file_row_line(
    display_name: &str,
    file: &XmlFile,
    max_file_size: u64,
    theme: &Theme,
) -> Line<'static> {
    let (size_text, size_style, ratio) = match file.size {
        Some(bytes) => {
            let ratio = if max_file_size > 0 {
                Some(bytes as f64 / max_file_size as f64)
            } else {
                None
            };
            (
                format!("{:>10}", format_size(bytes)),
                theme.primary_style(),
                ratio,
            )
        }
        None => (format!("{:>10}", "unknown"), theme.muted_style(), None),
    };

    let (glyph, glyph_style) = FileCategory::classify(&file.name).glyph_and_style(theme);

    let mut spans = Vec::with_capacity(MICRO_BAR_WIDTH + 8);
    spans.push(Span::styled(size_text, size_style));
    spans.push(Span::raw("  "));
    spans.extend(build_micro_bar_spans(
        ratio,
        MICRO_BAR_WIDTH,
        MicroBarStyle::ArchivalGold,
        theme,
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(glyph, glyph_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        display_name.to_string(),
        theme.primary_style(),
    ));
    if file.is_private() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "priv",
            theme.bold_if_color(theme.warn_style()),
        ));
    }
    Line::from(spans)
}

/// Builds the complete interactive TTY `--list` output: Manifest Card, optional Folders breakdown
/// (for large archives > 40 files with subdirectories), and flat-grouped alphanumeric file rows.
pub fn build_list_tty_lines(
    identifier: &str,
    files: &XmlFiles,
    filter: Option<&FilterContext>,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = build_list_manifest_card_lines(identifier, files, filter, theme, width);
    if files.files.is_empty() {
        return lines;
    }

    let groups = group_files_flat(&files.files);
    let large_mode = files.files.len() > LARGE_ARCHIVE_THRESHOLD;

    lines.push(Line::default());

    if large_mode {
        let mut subdirs: Vec<&FolderGroup<'_>> =
            groups.iter().filter(|g| !g.dir.is_empty()).collect();
        if !subdirs.is_empty() {
            subdirs.sort_by(|a, b| {
                b.total_known_bytes
                    .cmp(&a.total_known_bytes)
                    .then_with(|| b.entries.len().cmp(&a.entries.len()))
                    .then_with(|| alphanumeric_cmp(a.dir, b.dir))
            });
            let max_folder_bytes = subdirs
                .iter()
                .map(|g| g.total_known_bytes)
                .max()
                .unwrap_or(0);
            let glyphs = theme.glyphs();

            lines.push(Line::from(Span::styled("Folders", theme.bold_style())));
            for folder in subdirs.iter().take(LARGE_ARCHIVE_FOLDER_CAP) {
                let (size_text, size_style, ratio) = if folder.has_known_size {
                    let ratio = if max_folder_bytes > 0 {
                        Some(folder.total_known_bytes as f64 / max_folder_bytes as f64)
                    } else {
                        None
                    };
                    (
                        format!("{:>10}", format_size(folder.total_known_bytes)),
                        theme.primary_style(),
                        ratio,
                    )
                } else {
                    (format!("{:>10}", "unknown"), theme.muted_style(), None)
                };
                let count = folder.entries.len();
                let unit = if count == 1 { "file" } else { "files" };
                let mut row = Vec::with_capacity(MICRO_BAR_WIDTH + 8);
                row.push(Span::styled(size_text, size_style));
                row.push(Span::raw("  "));
                row.extend(build_micro_bar_spans(
                    ratio,
                    MICRO_BAR_WIDTH,
                    MicroBarStyle::ArchivalGold,
                    theme,
                ));
                row.push(Span::raw("  "));
                row.push(Span::styled(
                    glyphs.folder,
                    theme.bold_if_color(theme.accent_style()),
                ));
                row.push(Span::raw(" "));
                row.push(Span::styled(format!("{}/", folder.dir), theme.bold_style()));
                row.push(Span::raw("  "));
                row.push(Span::styled(
                    format!("({count} {unit})"),
                    theme.muted_style(),
                ));
                lines.push(Line::from(row));
            }
            if subdirs.len() > LARGE_ARCHIVE_FOLDER_CAP {
                let more = subdirs.len() - LARGE_ARCHIVE_FOLDER_CAP;
                let unit = if more == 1 { "folder" } else { "folders" };
                lines.push(Line::from(Span::styled(
                    format!("{FOLDER_OVERFLOW_INDENT}+{more} more {unit}"),
                    theme.muted_style(),
                )));
            }
            lines.push(Line::default());
        }
    }

    let row_limit = if large_mode {
        LARGE_ARCHIVE_FILE_CAP
    } else {
        files.files.len()
    };

    // Scale micro-bars against the largest displayed file so bars remain legible even in preview mode.
    let mut displayed_max: u64 = 0;
    let mut counted = 0usize;
    'scan: for group in &groups {
        for (_, file) in &group.entries {
            if let Some(sz) = file.size {
                displayed_max = displayed_max.max(sz);
            }
            counted += 1;
            if counted >= row_limit {
                break 'scan;
            }
        }
    }

    let mut emitted = 0usize;
    let mut first_section = true;
    'emit: for group in &groups {
        if group.entries.is_empty() {
            continue;
        }
        if !group.dir.is_empty() {
            if !first_section {
                lines.push(Line::default());
            }
            lines.push(build_folder_header_line(group, theme));
        }
        first_section = false;
        for (display_name, file) in &group.entries {
            lines.push(build_file_row_line(
                display_name,
                file,
                displayed_max,
                theme,
            ));
            emitted += 1;
            if emitted >= row_limit {
                break 'emit;
            }
        }
    }

    if large_mode {
        lines.push(Line::default());
        let max_w = (width as usize).max(16);
        let full_footer = format!(
            "Showing {LARGE_ARCHIVE_FILE_CAP} of {} files, narrow with -f '<glob>' or pipe to less for all",
            files.files.len()
        );
        let compact_footer = format!(
            "Showing {LARGE_ARCHIVE_FILE_CAP} of {} files (-f '<glob>' or pipe to less)",
            files.files.len()
        );
        let footer = if max_w >= 72 && full_footer.width() <= max_w {
            full_footer
        } else if compact_footer.width() <= max_w {
            compact_footer
        } else {
            let glyphs = theme.glyphs();
            let ell_w = glyphs.ellipsis.width();
            if max_w > ell_w {
                format!(
                    "{}{}",
                    tail_truncate(&compact_footer, max_w - ell_w),
                    glyphs.ellipsis
                )
            } else {
                tail_truncate(&compact_footer, max_w)
            }
        };
        lines.push(Line::from(Span::styled(footer, theme.muted_style())));
    }

    lines
}

/// Formats the interactive TTY `--list` Hybrid Archival Manifest Suite as an ANSI string.
pub fn format_list_tty(
    identifier: &str,
    files: &XmlFiles,
    filter: Option<&FilterContext>,
    theme: &Theme,
    width: u16,
) -> String {
    let lines = build_list_tty_lines(identifier, files, filter, theme, width);
    render_lines_to_ansi(&lines, theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_metadata::XmlFile;
    use crate::ui::render::render_lines_to_plain;

    fn mk_file(name: &str, size: Option<u64>, private: bool) -> XmlFile {
        XmlFile {
            name: name.to_string(),
            size,
            md5: None,
            private: private.then(|| "true".to_string()),
        }
    }

    fn sample_12_files() -> XmlFiles {
        XmlFiles {
            files: vec![
                mk_file("deftributetozzap64_jp2.zip", Some(93_500_000), false),
                mk_file("deftributetozzap64.pdf", Some(42_000_000), false),
                mk_file("deftributetozzap64_meta.xml", Some(3_400), false),
                mk_file("deftributetozzap64_files.xml", Some(4_200), false),
                mk_file("cover.jpg", Some(480_000), false),
                mk_file("audio/intro.mp3", Some(2_400_000), false),
                mk_file("audio/outro.flac", Some(8_200_000), false),
                mk_file("scans/page_10.png", Some(1_200_000), false),
                mk_file("scans/page_2.png", Some(1_100_000), false),
                mk_file("scans/secret.pdf", Some(900_000), true),
                mk_file("notes.txt", None, false),
                mk_file("raw.bin", Some(128), false),
            ],
        }
    }

    #[test]
    fn file_category_classification_covers_all_six_groups() {
        assert_eq!(FileCategory::classify("disk.zip"), FileCategory::Archive);
        assert_eq!(
            FileCategory::classify("dir/game.chd"),
            FileCategory::Archive
        );
        assert_eq!(FileCategory::classify("book.PDF"), FileCategory::Document);
        assert_eq!(FileCategory::classify("comic.cbz"), FileCategory::Document);
        assert_eq!(FileCategory::classify("track.flac"), FileCategory::Media);
        assert_eq!(FileCategory::classify("clip.mkv"), FileCategory::Media);
        assert_eq!(FileCategory::classify("cover.jp2"), FileCategory::Image);
        assert_eq!(FileCategory::classify("icon.svg"), FileCategory::Image);
        assert_eq!(FileCategory::classify("item_files.xml"), FileCategory::Data);
        assert_eq!(FileCategory::classify("item.torrent"), FileCategory::Data);
        assert_eq!(FileCategory::classify("firmware.bin"), FileCategory::Other);
        assert_eq!(FileCategory::classify("README"), FileCategory::Other);
    }

    #[test]
    fn list_tty_12_files_renders_manifest_card_flat_groups_microbars_and_priv_tag() {
        let files = sample_12_files();
        let theme = Theme::archival_truecolor();
        let filter = FilterContext {
            patterns: vec!["!*.torrent".to_string()],
            matched: 12,
            total: 14,
        };

        for width in [120u16, 80, 60, 44] {
            let card = build_list_manifest_card_lines(
                "deftributetozzap64",
                &files,
                Some(&filter),
                &theme,
                width,
            );
            let card_plain = render_lines_to_plain(&card);
            for (idx, row) in card_plain.iter().enumerate() {
                assert_eq!(
                    row.width(),
                    width as usize,
                    "width {width} card row {idx} width mismatch: {row:?}"
                );
            }
            assert!(card_plain[0].starts_with('╭') && card_plain[0].ends_with('╮'));
            assert!(
                card_plain.last().unwrap().starts_with('╰')
                    && card_plain.last().unwrap().ends_with('╯')
            );
            for inner_row in &card_plain[1..card_plain.len() - 1] {
                let inner = inner_row
                    .strip_prefix('│')
                    .and_then(|r| r.strip_suffix('│'))
                    .expect("inner card row must be bordered");
                assert!(
                    !inner.trim().is_empty(),
                    "width {width} manifest card must have zero blank inner rows:\n{}",
                    card_plain.join("\n")
                );
            }
        }

        let lines = build_list_tty_lines("deftributetozzap64", &files, Some(&filter), &theme, 80);
        let plain = render_lines_to_plain(&lines);
        let joined = plain.join("\n");

        assert!(joined.contains("ia-get · deftributetozzap64"), "{joined}");
        assert!(joined.contains("12 files"), "{joined}");
        assert!(joined.contains("zip 62% · pdf 29%"), "{joined}");
        assert!(joined.contains("1 unknown size"), "{joined}");
        assert!(joined.contains("1 private"), "{joined}");
        assert!(joined.contains("filter: !*.torrent (12 of 14)"), "{joined}");

        // Flat folder headers with zero leading indentation and natural alphanumeric ordering.
        assert!(plain.iter().any(|l| l.starts_with("▼ audio/")), "{joined}");
        assert!(plain.iter().any(|l| l.starts_with("▼ scans/")), "{joined}");
        let pos_p2 = joined.find("page_2.png").expect("missing page_2.png");
        let pos_p10 = joined.find("page_10.png").expect("missing page_10.png");
        assert!(pos_p2 < pos_p10, "natural alphanumeric sort:\n{joined}");

        // Verified file-type glyphs and priv tag appear.
        assert!(joined.contains("◆ deftributetozzap64_jp2.zip"), "{joined}");
        assert!(joined.contains("≡ deftributetozzap64.pdf"), "{joined}");
        assert!(joined.contains("► intro.mp3"), "{joined}");
        assert!(joined.contains("◉ cover.jpg"), "{joined}");
        assert!(joined.contains("○ deftributetozzap64_meta.xml"), "{joined}");
        assert!(joined.contains("● raw.bin"), "{joined}");
        assert!(joined.contains("secret.pdf  priv"), "{joined}");
        assert!(joined.contains("   unknown  ·"), "{joined}");

        // File rows have no right box border so paths copy-paste cleanly.
        for row in &plain[5..] {
            assert!(
                !row.ends_with('│'),
                "file listing row must have no right border: {row:?}"
            );
        }
    }

    #[test]
    fn list_tty_large_archive_over_40_files_shows_folder_breakdown_and_20_row_cap() {
        let mut entries = vec![mk_file("readme.txt", Some(1024), false)];
        for dir_idx in 1..=12 {
            for f_idx in 1..=4 {
                entries.push(mk_file(
                    &format!("vol_{dir_idx:02}/track_{f_idx:02}.mp3"),
                    Some((dir_idx as u64) * 100_000 + (f_idx as u64) * 1024),
                    false,
                ));
            }
        }
        let files = XmlFiles { files: entries };
        assert_eq!(files.files.len(), 49);

        let lines = build_list_tty_lines(
            "large-archive",
            &files,
            None,
            &Theme::archival_truecolor(),
            80,
        );
        let plain = render_lines_to_plain(&lines);
        let joined = plain.join("\n");

        assert!(joined.contains("49 files"), "{joined}");
        assert!(joined.contains("Folders"), "{joined}");
        assert!(
            plain
                .iter()
                .any(|l| l == &format!("{FOLDER_OVERFLOW_INDENT}+2 more folders")),
            "expected +2 more folders aligned at 22 spaces:\n{joined}"
        );
        assert_eq!(FOLDER_OVERFLOW_INDENT.len(), 22);
        assert!(
            !joined.contains("0 unknown") && !joined.contains("0 private"),
            "zero unknown/private counts must be hidden on Manifest Row 2:\n{joined}"
        );
        assert!(
            joined.contains(
                "Showing 20 of 49 files, narrow with -f '<glob>' or pipe to less for all"
            ),
            "{joined}"
        );
        // Count emitted file rows (each has a verified file-type glyph).
        let file_row_count = plain
            .iter()
            .filter(|l| l.contains("≡ ") || l.contains("► "))
            .count();
        assert_eq!(file_row_count, 20, "must cap at 20 file rows:\n{joined}");

        // At 60 cols (< 72), use compact footer without hard-clipping `less`.
        let lines_60 = build_list_tty_lines(
            "large-archive",
            &files,
            None,
            &Theme::archival_truecolor(),
            60,
        );
        let plain_60 = render_lines_to_plain(&lines_60);
        assert_eq!(
            plain_60.last().unwrap(),
            "Showing 20 of 49 files (-f '<glob>' or pipe to less)"
        );
        assert!(plain_60.last().unwrap().width() <= 60);

        // Below compact footer width (44 cols), truncate cleanly with ellipsis.
        let lines_44 = build_list_tty_lines(
            "large-archive",
            &files,
            None,
            &Theme::archival_truecolor(),
            44,
        );
        let plain_44 = render_lines_to_plain(&lines_44);
        let footer_44 = plain_44.last().unwrap();
        assert!(footer_44.width() <= 44, "{footer_44:?}");
        assert!(footer_44.ends_with('…'), "{footer_44:?}");
    }

    #[test]
    fn split_folder_and_name_handles_trailing_and_double_slashes() {
        assert_eq!(split_folder_and_name("file.txt"), ("", "file.txt"));
        assert_eq!(split_folder_and_name("/file.txt"), ("", "file.txt"));
        assert_eq!(split_folder_and_name("dir/file.txt"), ("dir", "file.txt"));
        assert_eq!(
            split_folder_and_name("dir/sub/file.txt"),
            ("dir/sub", "file.txt")
        );
        assert_eq!(split_folder_and_name("dir//file.txt"), ("dir", "file.txt"));
        assert_eq!(split_folder_and_name("dir///file.txt"), ("dir", "file.txt"));
        assert_eq!(split_folder_and_name("dir/sub/"), ("", "dir/sub/"));
        assert_eq!(split_folder_and_name("/dir/sub/"), ("", "dir/sub/"));
    }

    #[test]
    fn manifest_row_1_styles_bucket_1_with_info_style_across_truecolor_and_ansi16() {
        let files = sample_12_files();
        for theme in [
            Theme::archival_truecolor(),
            Theme::new(ColorLevel::Ansi16, crate::ui::palette::GlyphSet::Unicode),
        ] {
            let card = build_list_manifest_card_lines("item", &files, None, &theme, 80);
            let bucket_1_span = card[1]
                .spans
                .iter()
                .find(|s| s.content.contains(theme.glyphs().ribbon_fills[1]))
                .expect("missing bucket 1 ribbon span");
            assert_eq!(
                bucket_1_span.style,
                theme.info_style(),
                "bucket 1 must use info_style on {theme:?}"
            );
        }
    }

    #[test]
    fn list_tty_monochrome_and_ascii_fallbacks_are_clean() {
        let files = sample_12_files();
        let mono = Theme::monochrome_unicode();
        let mono_out = format_list_tty("item", &files, None, &mono, 80);
        assert!(!mono_out.contains('\x1b'), "NO_COLOR must emit zero ANSI");
        assert!(mono_out.contains("filter: all"));

        let ascii = Theme::monochrome_ascii();
        let ascii_out = format_list_tty("item", &files, None, &ascii, 80);
        assert!(!ascii_out.contains('\x1b'));
        assert!(
            ascii_out.is_ascii(),
            "ASCII fallback must be pure ASCII:\n{ascii_out}"
        );
        assert!(ascii_out.starts_with("+- ia-get | item"));
        assert!(ascii_out.contains("> audio/"));
        assert!(ascii_out.contains("A deftributetozzap64_jp2.zip"));
        assert!(ascii_out.contains("T deftributetozzap64.pdf"));
        assert!(ascii_out.contains("M intro.mp3"));
        assert!(ascii_out.contains("I cover.jpg"));
        assert!(ascii_out.contains("D deftributetozzap64_meta.xml"));
        assert!(ascii_out.contains("F raw.bin"));
    }

    #[test]
    fn list_tty_unknown_only_sizes_displays_unknown_sizes_ribbon() {
        let files = XmlFiles {
            files: vec![mk_file("a.txt", None, false), mk_file("b.txt", None, false)],
        };
        let out = format_list_tty("unknowns", &files, None, &Theme::monochrome_unicode(), 80);
        assert!(out.contains("unknown sizes"), "{out}");
        assert!(out.contains("2 unknown sizes"), "{out}");
    }
}
