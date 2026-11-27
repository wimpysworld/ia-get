//! Semantic archival colour tokens, glyph sets, and inline styling helpers.
//!
//! Colour tokens are derived from `pages/style.css` (warm parchment, bronze,
//! gold, and stone) and adapt automatically across truecolour, ANSI-16, and
//! monochrome (`NO_COLOR`, `TERM=dumb`, or non-TTY) environments.

use std::io::IsTerminal;

use crossterm::style::{Attribute, Color as CtColor, ContentStyle, StyledContent};
use ratatui::style::{Color, Modifier, Style};

/// Terminal colour capability level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLevel {
    /// 24-bit RGB colour (warm archival bronze/gold palette).
    TrueColor,
    /// 16-colour ANSI palette respecting the user's terminal theme.
    Ansi16,
    /// Plain unstyled monochrome (`NO_COLOR`, `TERM=dumb`, or non-TTY).
    Monochrome,
}

impl ColorLevel {
    /// Detects the appropriate colour capability for an output stream.
    pub fn detect(is_tty: bool) -> Self {
        let no_color = std::env::var("NO_COLOR").ok();
        let term = std::env::var("TERM").ok();
        let colorterm = std::env::var("COLORTERM").ok();
        let term_program = std::env::var("TERM_PROGRAM").ok();
        let wt_session = std::env::var("WT_SESSION").ok();
        Self::from_env(
            is_tty,
            no_color.as_deref(),
            term.as_deref(),
            colorterm.as_deref(),
            term_program.as_deref(),
            wt_session.as_deref(),
        )
    }

    /// Pure environment-based colour capability detection (testable without mutating process env).
    pub fn from_env(
        is_tty: bool,
        no_color: Option<&str>,
        term: Option<&str>,
        colorterm: Option<&str>,
        term_program: Option<&str>,
        wt_session: Option<&str>,
    ) -> Self {
        if !is_tty {
            return Self::Monochrome;
        }
        if no_color.is_some_and(|v| !v.is_empty()) {
            return Self::Monochrome;
        }
        let term_lower = term.unwrap_or("").to_ascii_lowercase();
        if term_lower == "dumb" {
            return Self::Monochrome;
        }
        if wt_session.is_some_and(|v| !v.is_empty()) {
            return Self::TrueColor;
        }
        if term_lower.is_empty() {
            return Self::Monochrome;
        }
        let ct_lower = colorterm.unwrap_or("").to_ascii_lowercase();
        if ct_lower.contains("truecolor") || ct_lower.contains("24bit") {
            return Self::TrueColor;
        }
        if term_lower.contains("direct") {
            return Self::TrueColor;
        }
        if let Some(prog) = term_program {
            let prog_lower = prog.to_ascii_lowercase();
            if matches!(
                prog_lower.as_str(),
                "wezterm" | "iterm.app" | "ghostty" | "vscode" | "alacritty" | "kitty"
            ) {
                return Self::TrueColor;
            }
        }
        Self::Ansi16
    }
}

/// Detects whether `$COLORFGBG` suggests a light terminal background.
pub(crate) fn is_light_background_from_env(colorfgbg: Option<&str>) -> bool {
    let Some(raw) = colorfgbg else {
        return false;
    };
    let Some(bg_str) = raw.rsplit(';').next() else {
        return false;
    };
    matches!(bg_str.trim().parse::<u8>(), Ok(7) | Ok(15))
}

/// Unicode or ASCII glyph table used across the inline instrument and summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet {
    /// Rounded Unicode box drawing, eighth-block bars, and Braille spinners.
    Unicode,
    /// Pure 7-bit ASCII fallbacks for legacy terminals or non-UTF-8 locales.
    Ascii,
}

impl GlyphSet {
    /// Detects whether the current terminal environment supports Unicode glyphs.
    pub fn detect() -> Self {
        let term = std::env::var("TERM").ok();
        let lc_all = std::env::var("LC_ALL").ok();
        let lc_ctype = std::env::var("LC_CTYPE").ok();
        let lang = std::env::var("LANG").ok();
        Self::from_env(
            term.as_deref(),
            lc_all.as_deref(),
            lc_ctype.as_deref(),
            lang.as_deref(),
        )
    }

    /// Pure environment-based glyph capability detection.
    pub fn from_env(
        term: Option<&str>,
        lc_all: Option<&str>,
        lc_ctype: Option<&str>,
        lang: Option<&str>,
    ) -> Self {
        if term.is_some_and(|t| t.eq_ignore_ascii_case("dumb")) {
            return Self::Ascii;
        }
        let locale = [lc_all, lc_ctype, lang]
            .into_iter()
            .flatten()
            .find(|v| !v.is_empty());
        if let Some(loc) = locale {
            let lower = loc.to_ascii_lowercase();
            if !(lower.contains("utf-8") || lower.contains("utf8")) {
                return Self::Ascii;
            }
        }
        Self::Unicode
    }

    /// Returns the concrete glyph strings for this glyph set.
    pub const fn glyphs(self) -> &'static Glyphs {
        match self {
            Self::Unicode => &UNICODE_GLYPHS,
            Self::Ascii => &ASCII_GLYPHS,
        }
    }
}

/// Concrete glyph strings for borders, bars, sparklines, and status icons.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub top_left: &'static str,
    pub top_right: &'static str,
    pub bottom_left: &'static str,
    pub bottom_right: &'static str,
    pub horizontal: &'static str,
    pub vertical: &'static str,
    /// Heavy left accent bar for the copy-clean exception ledger (`┃` / `|`).
    pub ledger_bar: &'static str,
    /// Eight sub-cell fill blocks ordered 1/8 through 8/8.
    pub bar_blocks: [&'static str; 8],
    pub bar_empty: &'static str,
    /// Outcome segmented bar fills for the Archival Receipt.
    pub bar_ok: &'static str,
    pub bar_skip: &'static str,
    pub bar_warn: &'static str,
    pub bar_fail: &'static str,
    /// Eight sparkline heights ordered lowest to highest.
    pub spark_blocks: [&'static str; 8],
    /// High-visibility geometric stage glyphs (all display width 1).
    pub stage_get: &'static str,
    pub stage_resume: &'static str,
    pub stage_redirect: &'static str,
    pub stage_md5: [&'static str; 4],
    pub stage_rate_limited: &'static str,
    pub stage_queued: &'static str,
    pub ok: &'static str,
    pub skip: &'static str,
    pub warn: &'static str,
    pub fail: &'static str,
    pub interrupted: &'static str,
    pub not_equal: &'static str,
    pub pip_filled: &'static str,
    pub pip_empty: &'static str,
    pub sep: &'static str,
    pub ellipsis: &'static str,
    pub arrow: &'static str,
    pub spinner: &'static [&'static str],
}

pub const UNICODE_GLYPHS: Glyphs = Glyphs {
    top_left: "╭",
    top_right: "╮",
    bottom_left: "╰",
    bottom_right: "╯",
    horizontal: "─",
    vertical: "│",
    ledger_bar: "┃",
    bar_blocks: ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
    bar_empty: "─",
    bar_ok: "█",
    bar_skip: "░",
    bar_warn: "▓",
    bar_fail: "▒",
    spark_blocks: ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
    stage_get: "▼",
    stage_resume: "►",
    stage_redirect: "»",
    stage_md5: ["◐", "◓", "◑", "◒"],
    stage_rate_limited: "▲",
    stage_queued: "○",
    ok: "✓",
    skip: "↷",
    warn: "!",
    fail: "✗",
    interrupted: "■",
    not_equal: "≠",
    pip_filled: "●",
    pip_empty: "○",
    sep: "·",
    ellipsis: "…",
    arrow: "→",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

pub const ASCII_GLYPHS: Glyphs = Glyphs {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    horizontal: "-",
    vertical: "|",
    ledger_bar: "|",
    bar_blocks: ["-", "-", "-", "-", "=", "=", "=", "#"],
    bar_empty: "-",
    bar_ok: "#",
    bar_skip: ":",
    bar_warn: "!",
    bar_fail: "x",
    spark_blocks: [".", ".", "_", "_", "-", "=", "=", "#"],
    stage_get: "v",
    stage_resume: ">",
    stage_redirect: "~",
    stage_md5: ["#", "#", "#", "#"],
    stage_rate_limited: "!",
    stage_queued: ".",
    ok: "+",
    skip: ">",
    warn: "!",
    fail: "x",
    interrupted: "#",
    not_equal: "!=",
    pip_filled: "*",
    pip_empty: "o",
    sep: "|",
    ellipsis: "...",
    arrow: "->",
    spinner: &["-", "\\", "|", "/"],
};

/// Semantic archival RGB endpoints for progress-bar gradients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn to_ratatui(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }

    pub const fn to_crossterm(self) -> CtColor {
        CtColor::Rgb {
            r: self.0,
            g: self.1,
            b: self.2,
        }
    }

    /// Linearly interpolates between `self` and `other` by `t` in `0.0..=1.0`.
    pub fn lerp(self, other: Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| -> u8 {
            (f64::from(a) + (f64::from(b) - f64::from(a)) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
        )
    }
}

/// Archival dark-theme brand tokens from `pages/style.css`.
pub const GOLD: Rgb = Rgb(0xF2, 0xC9, 0x6B);
pub const BRONZE: Rgb = Rgb(0xC4, 0x84, 0x25);
pub const DEEP_BRONZE: Rgb = Rgb(0x94, 0x5B, 0x0E);
pub const PARCHMENT: Rgb = Rgb(0xF5, 0xEF, 0xE6);
pub const STONE: Rgb = Rgb(0xC4, 0xB6, 0xA4);
const QUEUED_DARK: Rgb = Rgb(0x96, 0x85, 0x74);
pub const TRACK_DARK: Rgb = Rgb(0x63, 0x4D, 0x3C);
pub const SAGE: Rgb = Rgb(0x8B, 0xB8, 0x7E);
pub const SAGE_BRIGHT: Rgb = Rgb(0xA9, 0xD4, 0x9C);
pub const AMBER: Rgb = Rgb(0xE5, 0xA9, 0x3B);
pub const CRIMSON: Rgb = Rgb(0xD9, 0x6B, 0x6B);
pub const SLATE_INFO: Rgb = Rgb(0x7F, 0xA7, 0xB8);
pub const SLATE_INFO_BRIGHT: Rgb = Rgb(0xA8, 0xC9, 0xD8);

/// Archival light-theme adjustments from `pages/style.css`.
const STONE_LIGHT: Rgb = Rgb(0x5C, 0x4D, 0x40);
const QUEUED_LIGHT: Rgb = Rgb(0x78, 0x6A, 0x5D);
const TRACK_LIGHT: Rgb = Rgb(0xB5, 0xA6, 0x92);
const SAGE_LIGHT: Rgb = Rgb(0x2F, 0x6E, 0x3A);
const SAGE_LIGHT_END: Rgb = Rgb(0x4A, 0x8A, 0x54);
const AMBER_LIGHT: Rgb = Rgb(0x8F, 0x57, 0x00);
const CRIMSON_LIGHT: Rgb = Rgb(0xB3, 0x3A, 0x3A);
const SLATE_LIGHT: Rgb = Rgb(0x2B, 0x62, 0x78);

/// Semantic theme combining adaptive colour tokens and glyph tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub color_level: ColorLevel,
    pub glyph_set: GlyphSet,
    pub light_bg: bool,
}

impl Theme {
    /// Creates a theme with explicit colour level and glyph set (dark background default).
    pub const fn new(color_level: ColorLevel, glyph_set: GlyphSet) -> Self {
        Self {
            color_level,
            glyph_set,
            light_bg: false,
        }
    }

    /// Detects the active theme for `stdout`.
    pub fn detect_stdout() -> Self {
        Self::detect_for_tty(std::io::stdout().is_terminal())
    }

    /// Detects the active theme for `stderr`.
    pub fn detect_stderr() -> Self {
        Self::detect_for_tty(std::io::stderr().is_terminal())
    }

    /// Detects the active theme given whether the target stream is a TTY.
    pub fn detect_for_tty(is_tty: bool) -> Self {
        let color_level = ColorLevel::detect(is_tty);
        let glyph_set = GlyphSet::detect();
        let light_bg = is_light_background_from_env(std::env::var("COLORFGBG").ok().as_deref());
        Self {
            color_level,
            glyph_set,
            light_bg,
        }
    }

    /// Returns a truecolour Unicode archival theme (for dark terminals and snapshots).
    pub const fn archival_truecolor() -> Self {
        Self::new(ColorLevel::TrueColor, GlyphSet::Unicode)
    }

    /// Returns a monochrome Unicode theme (`NO_COLOR`).
    pub const fn monochrome_unicode() -> Self {
        Self::new(ColorLevel::Monochrome, GlyphSet::Unicode)
    }

    /// Returns a monochrome ASCII theme (`TERM=dumb` / legacy fallback).
    pub const fn monochrome_ascii() -> Self {
        Self::new(ColorLevel::Monochrome, GlyphSet::Ascii)
    }

    /// Returns the concrete glyph table for this theme.
    pub const fn glyphs(&self) -> &'static Glyphs {
        self.glyph_set.glyphs()
    }

    /// Returns `true` when ANSI colours must not be emitted.
    pub const fn is_monochrome(&self) -> bool {
        matches!(self.color_level, ColorLevel::Monochrome)
    }

    /// Returns `true` when regional flag emoji can be rendered cleanly
    /// (Unicode glyph set, colour output enabled, and non-Windows terminal).
    pub const fn use_region_flag_emoji(&self) -> bool {
        matches!(self.glyph_set, GlyphSet::Unicode) && !self.is_monochrome() && !cfg!(windows)
    }

    // ---- Ratatui Styles ----

    pub fn border_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                DEEP_BRONZE.to_ratatui()
            } else {
                BRONZE.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Yellow),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn title_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default()
                .fg(if self.light_bg {
                    DEEP_BRONZE.to_ratatui()
                } else {
                    GOLD.to_ratatui()
                })
                .add_modifier(Modifier::BOLD),
            ColorLevel::Ansi16 => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn primary_style(&self) -> Style {
        Style::default()
    }

    pub fn muted_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                STONE_LIGHT.to_ratatui()
            } else {
                STONE.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::DarkGray),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn track_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                TRACK_LIGHT.to_ratatui()
            } else {
                TRACK_DARK.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::DarkGray),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn ok_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                SAGE_LIGHT.to_ratatui()
            } else {
                SAGE.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Green),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn warn_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                AMBER_LIGHT.to_ratatui()
            } else {
                AMBER.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Yellow),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn err_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                CRIMSON_LIGHT.to_ratatui()
            } else {
                CRIMSON.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Red),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn info_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                SLATE_LIGHT.to_ratatui()
            } else {
                SLATE_INFO.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Cyan),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn accent_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                DEEP_BRONZE.to_ratatui()
            } else {
                GOLD.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::Yellow),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    pub fn bold_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor | ColorLevel::Ansi16 => {
                Style::default().add_modifier(Modifier::BOLD)
            }
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Adds `Modifier::BOLD` to `style` when colour output is enabled (`NO_COLOR` stays plain).
    pub fn bold_if_color(&self, style: Style) -> Style {
        if self.is_monochrome() {
            style
        } else {
            style.add_modifier(Modifier::BOLD)
        }
    }

    /// Stage glyph style for fresh download (`▼` / `v`): bright gold, bold.
    pub fn stage_get_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default()
                .fg(if self.light_bg {
                    DEEP_BRONZE.to_ratatui()
                } else {
                    GOLD.to_ratatui()
                })
                .add_modifier(Modifier::BOLD),
            ColorLevel::Ansi16 => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Stage glyph style for `.part` resume (`►` / `>`): warm amber-gold, bold.
    pub fn stage_resume_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default()
                .fg(if self.light_bg {
                    AMBER_LIGHT.to_ratatui()
                } else {
                    AMBER.to_ratatui()
                })
                .add_modifier(Modifier::BOLD),
            ColorLevel::Ansi16 => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Stage glyph style for HTTP redirect (`»` / `~`): slate blue, bold.
    pub fn stage_redirect_style(&self) -> Style {
        self.bold_if_color(self.info_style())
    }

    /// Stage glyph style for local MD5 verification (`◐◓◑◒` / `#`): sage green, bold.
    pub fn stage_md5_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default()
                .fg(if self.light_bg {
                    SAGE_LIGHT.to_ratatui()
                } else {
                    SAGE.to_ratatui()
                })
                .add_modifier(Modifier::BOLD),
            ColorLevel::Ansi16 => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Stage glyph style for 429/503 cooldown (`▲` / `!`): bright amber, bold.
    pub fn stage_rate_limited_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default()
                .fg(if self.light_bg {
                    AMBER_LIGHT.to_ratatui()
                } else {
                    AMBER.to_ratatui()
                })
                .add_modifier(Modifier::BOLD),
            ColorLevel::Ansi16 => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Muted stone style for up-next queued entries (`○` / `.`) without stacking `DIM`.
    pub fn queued_style(&self) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => Style::default().fg(if self.light_bg {
                QUEUED_LIGHT.to_ratatui()
            } else {
                QUEUED_DARK.to_ratatui()
            }),
            ColorLevel::Ansi16 => Style::default().fg(Color::DarkGray),
            ColorLevel::Monochrome => Style::default(),
        }
    }

    /// Computes the foreground style for a filled cell at index `cell` of a `width`-cell bar.
    pub fn bar_fill_style(&self, cell: usize, width: usize, is_hash: bool) -> Style {
        match self.color_level {
            ColorLevel::TrueColor => {
                let t = if width <= 1 {
                    1.0
                } else {
                    cell as f64 / (width - 1) as f64
                };
                let (start, end) = if is_hash {
                    if self.light_bg {
                        (SAGE_LIGHT, SAGE_LIGHT_END)
                    } else {
                        (SAGE, SAGE_BRIGHT)
                    }
                } else if self.light_bg {
                    (DEEP_BRONZE, BRONZE)
                } else {
                    (BRONZE, GOLD)
                };
                Style::default().fg(start.lerp(end, t).to_ratatui())
            }
            ColorLevel::Ansi16 => {
                Style::default().fg(if is_hash { Color::Green } else { Color::Yellow })
            }
            ColorLevel::Monochrome => Style::default(),
        }
    }

    // ---- Crossterm Inline Styling ----

    fn apply_inline(&self, text: &str, ct_color: Option<CtColor>, bold: bool) -> String {
        if self.is_monochrome() {
            return text.to_string();
        }
        let mut style = ContentStyle::new();
        if let Some(color) = ct_color {
            style.foreground_color = Some(color);
        }
        if bold {
            style.attributes.set(Attribute::Bold);
        }
        StyledContent::new(style, text).to_string()
    }
}

/// Returns the detected theme for `stdout`.
pub fn stdout_theme() -> Theme {
    Theme::detect_stdout()
}

/// Styles `text` with the semantic success colour (plain text in monochrome/non-TTY).
pub fn style_ok(text: &str, theme: &Theme) -> String {
    let color = match theme.color_level {
        ColorLevel::TrueColor => Some(if theme.light_bg {
            SAGE_LIGHT.to_crossterm()
        } else {
            SAGE.to_crossterm()
        }),
        ColorLevel::Ansi16 => Some(CtColor::Green),
        ColorLevel::Monochrome => None,
    };
    theme.apply_inline(text, color, false)
}

/// Styles `text` with the semantic warning colour (plain text in monochrome/non-TTY).
pub fn style_warn(text: &str, theme: &Theme) -> String {
    let color = match theme.color_level {
        ColorLevel::TrueColor => Some(if theme.light_bg {
            AMBER_LIGHT.to_crossterm()
        } else {
            AMBER.to_crossterm()
        }),
        ColorLevel::Ansi16 => Some(CtColor::Yellow),
        ColorLevel::Monochrome => None,
    };
    theme.apply_inline(text, color, false)
}

/// Styles `text` with the semantic error colour (plain text in monochrome/non-TTY).
pub fn style_err(text: &str, theme: &Theme) -> String {
    let color = match theme.color_level {
        ColorLevel::TrueColor => Some(if theme.light_bg {
            CRIMSON_LIGHT.to_crossterm()
        } else {
            CRIMSON.to_crossterm()
        }),
        ColorLevel::Ansi16 => Some(CtColor::Red),
        ColorLevel::Monochrome => None,
    };
    theme.apply_inline(text, color, false)
}

/// Styles `text` with the archival gold/bronze accent colour (plain text in monochrome/non-TTY).
pub fn style_accent(text: &str, theme: &Theme) -> String {
    let color = match theme.color_level {
        ColorLevel::TrueColor => Some(if theme.light_bg {
            DEEP_BRONZE.to_crossterm()
        } else {
            GOLD.to_crossterm()
        }),
        ColorLevel::Ansi16 => Some(CtColor::Yellow),
        ColorLevel::Monochrome => None,
    };
    theme.apply_inline(text, color, false)
}

/// Styles `text` with the muted stone colour (plain text in monochrome/non-TTY).
pub fn style_muted(text: &str, theme: &Theme) -> String {
    let color = match theme.color_level {
        ColorLevel::TrueColor => Some(if theme.light_bg {
            STONE_LIGHT.to_crossterm()
        } else {
            STONE.to_crossterm()
        }),
        ColorLevel::Ansi16 => Some(CtColor::DarkGrey),
        ColorLevel::Monochrome => None,
    };
    theme.apply_inline(text, color, false)
}

/// Styles `text` with bold emphasis (plain text in monochrome/non-TTY).
pub fn style_bold(text: &str, theme: &Theme) -> String {
    theme.apply_inline(text, None, true)
}

fn ratatui_color_to_crossterm(color: Color) -> Option<CtColor> {
    match color {
        Color::Rgb(r, g, b) => Some(CtColor::Rgb { r, g, b }),
        Color::Black => Some(CtColor::Black),
        Color::Red => Some(CtColor::DarkRed),
        Color::Green => Some(CtColor::DarkGreen),
        Color::Yellow => Some(CtColor::DarkYellow),
        Color::Blue => Some(CtColor::DarkBlue),
        Color::Magenta => Some(CtColor::DarkMagenta),
        Color::Cyan => Some(CtColor::DarkCyan),
        Color::Gray => Some(CtColor::Grey),
        Color::DarkGray => Some(CtColor::DarkGrey),
        Color::LightRed => Some(CtColor::Red),
        Color::LightGreen => Some(CtColor::Green),
        Color::LightYellow => Some(CtColor::Yellow),
        Color::LightBlue => Some(CtColor::Blue),
        Color::LightMagenta => Some(CtColor::Magenta),
        Color::LightCyan => Some(CtColor::Cyan),
        Color::White => Some(CtColor::White),
        Color::Indexed(idx) => Some(CtColor::AnsiValue(idx)),
        Color::Reset => None,
    }
}

/// Converts a Ratatui [`Style`] and text slice into an inline Crossterm ANSI string
/// (or plain text when `theme` is monochrome or `style` is default).
pub fn style_ratatui_to_ansi(text: &str, style: Style, theme: &Theme) -> String {
    if theme.is_monochrome() || text.is_empty() || style == Style::default() {
        return text.to_string();
    }
    let mut ct_style = ContentStyle::new();
    ct_style.foreground_color = style.fg.and_then(ratatui_color_to_crossterm);
    ct_style.background_color = style.bg.and_then(ratatui_color_to_crossterm);
    if style.add_modifier.contains(Modifier::BOLD) {
        ct_style.attributes.set(Attribute::Bold);
    }
    if style.add_modifier.contains(Modifier::DIM) {
        ct_style.attributes.set(Attribute::Dim);
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        ct_style.attributes.set(Attribute::Italic);
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        ct_style.attributes.set(Attribute::Underlined);
    }
    if ct_style.foreground_color.is_none()
        && ct_style.background_color.is_none()
        && ct_style.attributes.is_empty()
    {
        return text.to_string();
    }
    StyledContent::new(ct_style, text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_level_honours_non_tty_no_color_dumb_and_truecolor() {
        assert_eq!(
            ColorLevel::from_env(
                false,
                None,
                Some("xterm-256color"),
                Some("truecolor"),
                None,
                None
            ),
            ColorLevel::Monochrome
        );
        assert_eq!(
            ColorLevel::from_env(
                true,
                Some("1"),
                Some("xterm-256color"),
                Some("truecolor"),
                None,
                None
            ),
            ColorLevel::Monochrome
        );
        // Empty NO_COLOR does not disable colour per no-color.org.
        assert_eq!(
            ColorLevel::from_env(
                true,
                Some(""),
                Some("xterm-256color"),
                Some("truecolor"),
                None,
                None
            ),
            ColorLevel::TrueColor
        );
        assert_eq!(
            ColorLevel::from_env(true, None, Some("dumb"), Some("truecolor"), None, None),
            ColorLevel::Monochrome
        );
        assert_eq!(
            ColorLevel::from_env(true, None, Some("xterm-256color"), None, None, None),
            ColorLevel::Ansi16
        );
        assert_eq!(
            ColorLevel::from_env(
                true,
                None,
                Some("xterm-256color"),
                None,
                Some("ghostty"),
                None
            ),
            ColorLevel::TrueColor
        );
        assert_eq!(
            ColorLevel::from_env(true, None, Some("xterm-256color"), None, None, Some("1")),
            ColorLevel::TrueColor
        );
        // Windows Terminal leaves TERM unset (or empty) while setting WT_SESSION.
        assert_eq!(
            ColorLevel::from_env(true, None, None, None, None, Some("wt-guid-1234")),
            ColorLevel::TrueColor
        );
        assert_eq!(
            ColorLevel::from_env(true, None, Some(""), None, None, Some("wt-guid-1234")),
            ColorLevel::TrueColor
        );
        // Explicit NO_COLOR and TERM=dumb still override WT_SESSION.
        assert_eq!(
            ColorLevel::from_env(true, Some("1"), None, None, None, Some("wt-guid-1234")),
            ColorLevel::Monochrome
        );
        assert_eq!(
            ColorLevel::from_env(true, None, Some("dumb"), None, None, Some("wt-guid-1234")),
            ColorLevel::Monochrome
        );
    }

    #[test]
    fn primary_style_uses_terminal_default_and_track_tokens_match_contrast_targets() {
        let dark_tc = Theme::archival_truecolor();
        let mut light_tc = Theme::archival_truecolor();
        light_tc.light_bg = true;
        let ansi = Theme::new(ColorLevel::Ansi16, GlyphSet::Unicode);
        let mono = Theme::monochrome_unicode();

        for theme in [dark_tc, light_tc, ansi, mono] {
            assert_eq!(theme.primary_style(), Style::default());
        }

        assert_eq!(TRACK_DARK, Rgb(0x63, 0x4D, 0x3C));
        assert_eq!(
            dark_tc.track_style(),
            Style::default().fg(Color::Rgb(0x63, 0x4D, 0x3C))
        );
        assert_eq!(
            light_tc.track_style(),
            Style::default().fg(Color::Rgb(0xB5, 0xA6, 0x92))
        );
    }

    #[test]
    fn glyph_set_falls_back_to_ascii_for_dumb_or_c_locale() {
        assert_eq!(
            GlyphSet::from_env(Some("dumb"), Some("en_US.UTF-8"), None, None),
            GlyphSet::Ascii
        );
        assert_eq!(
            GlyphSet::from_env(Some("xterm"), Some("C"), None, None),
            GlyphSet::Ascii
        );
        assert_eq!(
            GlyphSet::from_env(Some("xterm"), Some("POSIX"), None, None),
            GlyphSet::Ascii
        );
        assert_eq!(
            GlyphSet::from_env(Some("xterm-256color"), None, None, Some("en_GB.UTF-8")),
            GlyphSet::Unicode
        );
        assert_eq!(
            GlyphSet::from_env(Some("xterm-256color"), None, None, None),
            GlyphSet::Unicode
        );
    }

    #[test]
    fn inline_helpers_emit_zero_ansi_in_monochrome_and_ansi_in_color() {
        let mono = Theme::monochrome_unicode();
        for styled in [
            style_ok("ok", &mono),
            style_warn("warn", &mono),
            style_err("err", &mono),
            style_accent("accent", &mono),
            style_muted("muted", &mono),
            style_bold("bold", &mono),
        ] {
            assert!(
                !styled.contains('\x1b'),
                "monochrome helper must emit zero ANSI: {styled:?}"
            );
        }

        let tc = Theme::archival_truecolor();
        assert!(style_ok("ok", &tc).contains('\x1b'));
        assert!(style_warn("warn", &tc).contains('\x1b'));
        assert!(style_err("err", &tc).contains('\x1b'));
        assert!(style_accent("accent", &tc).contains('\x1b'));
        assert!(style_muted("muted", &tc).contains('\x1b'));
        assert!(style_bold("bold", &tc).contains('\x1b'));
    }

    #[test]
    fn light_background_detection_parses_colorfgbg() {
        assert!(is_light_background_from_env(Some("0;15")));
        assert!(is_light_background_from_env(Some("0;default;7")));
        assert!(!is_light_background_from_env(Some("15;0")));
        assert!(!is_light_background_from_env(None));
    }

    #[test]
    fn every_stage_and_status_glyph_has_display_width_one() {
        use unicode_width::UnicodeWidthStr;

        for (label, glyphs) in [("unicode", &UNICODE_GLYPHS), ("ascii", &ASCII_GLYPHS)] {
            let stage_glyphs = [
                ("stage_get", glyphs.stage_get),
                ("stage_resume", glyphs.stage_resume),
                ("stage_redirect", glyphs.stage_redirect),
                ("stage_md5[0]", glyphs.stage_md5[0]),
                ("stage_md5[1]", glyphs.stage_md5[1]),
                ("stage_md5[2]", glyphs.stage_md5[2]),
                ("stage_md5[3]", glyphs.stage_md5[3]),
                ("stage_rate_limited", glyphs.stage_rate_limited),
                ("stage_queued", glyphs.stage_queued),
                ("ok", glyphs.ok),
                ("skip", glyphs.skip),
                ("warn", glyphs.warn),
                ("fail", glyphs.fail),
                ("interrupted", glyphs.interrupted),
                ("ledger_bar", glyphs.ledger_bar),
                ("bar_empty", glyphs.bar_empty),
                ("bar_ok", glyphs.bar_ok),
                ("bar_skip", glyphs.bar_skip),
                ("bar_warn", glyphs.bar_warn),
                ("bar_fail", glyphs.bar_fail),
            ];
            for (name, g) in stage_glyphs {
                assert_eq!(
                    g.width(),
                    1,
                    "{label} glyph {name} ({g:?}) must have display width 1"
                );
            }

            // Receipt outcome bar segments and empty track must never collide.
            let bar_glyphs = [
                glyphs.bar_ok,
                glyphs.bar_skip,
                glyphs.bar_warn,
                glyphs.bar_fail,
                glyphs.bar_empty,
            ];
            for i in 0..bar_glyphs.len() {
                for j in (i + 1)..bar_glyphs.len() {
                    assert_ne!(
                        bar_glyphs[i], bar_glyphs[j],
                        "{label} receipt bar glyphs at {i} and {j} must be distinct"
                    );
                }
            }
        }
    }

    #[test]
    fn redirect_glyph_queued_style_light_contrast_and_ansi16_mapping() {
        assert_eq!(UNICODE_GLYPHS.stage_redirect, "»");
        assert_eq!(AMBER_LIGHT, Rgb(0x8F, 0x57, 0x00));
        assert_eq!(SAGE_LIGHT_END, Rgb(0x4A, 0x8A, 0x54));

        let dark_tc = Theme::archival_truecolor();
        let mut light_tc = Theme::archival_truecolor();
        light_tc.light_bg = true;
        let ansi = Theme::new(ColorLevel::Ansi16, GlyphSet::Unicode);
        let mono = Theme::monochrome_unicode();

        // Redirect style is bold slate blue on colour themes, plain on monochrome.
        assert_eq!(
            dark_tc.stage_redirect_style(),
            dark_tc.info_style().add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            ansi.stage_redirect_style(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(mono.stage_redirect_style(), Style::default());

        // queued_style never stacks Modifier::DIM on TrueColor or Ansi16.
        for theme in [dark_tc, light_tc, ansi, mono] {
            assert!(
                !theme.queued_style().add_modifier.contains(Modifier::DIM),
                "queued_style must not stack DIM on {theme:?}"
            );
        }
        assert_eq!(
            dark_tc.queued_style(),
            Style::default().fg(QUEUED_DARK.to_ratatui())
        );
        assert_eq!(
            light_tc.queued_style(),
            Style::default().fg(QUEUED_LIGHT.to_ratatui())
        );
        assert_eq!(ansi.queued_style(), Style::default().fg(Color::DarkGray));

        // Light MD5 bar gradient ends at #4A8A54.
        assert_eq!(
            light_tc.bar_fill_style(9, 10, true),
            Style::default().fg(Color::Rgb(0x4A, 0x8A, 0x54))
        );

        // style_ratatui_to_ansi maps standard ANSI-16 to Dark* (SGR 31..36), Light* to 91..96, and bg.
        assert_eq!(
            ratatui_color_to_crossterm(Color::Green),
            Some(CtColor::DarkGreen)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::Yellow),
            Some(CtColor::DarkYellow)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::Red),
            Some(CtColor::DarkRed)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::Cyan),
            Some(CtColor::DarkCyan)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::Blue),
            Some(CtColor::DarkBlue)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::Magenta),
            Some(CtColor::DarkMagenta)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::LightGreen),
            Some(CtColor::Green)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::LightYellow),
            Some(CtColor::Yellow)
        );
        assert_eq!(
            ratatui_color_to_crossterm(Color::LightRed),
            Some(CtColor::Red)
        );

        let bg_only = style_ratatui_to_ansi("x", Style::default().bg(Color::Blue), &ansi);
        assert!(
            bg_only.contains('\x1b') && bg_only != "x" && bg_only.contains("48;5;4"),
            "background-only style must emit ANSI bg SGR: {bg_only:?}"
        );
    }

    #[test]
    fn region_flag_emoji_respects_unicode_color_and_platform() {
        let tc_uni = Theme::archival_truecolor();
        let ansi_uni = Theme::new(ColorLevel::Ansi16, GlyphSet::Unicode);
        let mono_uni = Theme::monochrome_unicode();
        let tc_ascii = Theme::new(ColorLevel::TrueColor, GlyphSet::Ascii);
        let mono_ascii = Theme::monochrome_ascii();

        assert_eq!(tc_uni.use_region_flag_emoji(), !cfg!(windows));
        assert_eq!(ansi_uni.use_region_flag_emoji(), !cfg!(windows));
        assert!(!mono_uni.use_region_flag_emoji());
        assert!(!tc_ascii.use_region_flag_emoji());
        assert!(!mono_ascii.use_region_flag_emoji());
    }
}
