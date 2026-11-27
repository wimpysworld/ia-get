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
    /// Eight sub-cell fill blocks ordered 1/8 through 8/8.
    pub bar_blocks: [&'static str; 8],
    pub bar_empty: &'static str,
    /// Eight sparkline heights ordered lowest to highest.
    pub spark_blocks: [&'static str; 8],
    pub permit_filled: &'static str,
    pub permit_empty: &'static str,
    pub ok: &'static str,
    pub skip: &'static str,
    pub warn: &'static str,
    pub fail: &'static str,
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
    bar_blocks: ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
    bar_empty: "░",
    spark_blocks: ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
    permit_filled: "█",
    permit_empty: "░",
    ok: "✓",
    skip: "↷",
    warn: "!",
    fail: "✗",
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
    bar_blocks: ["-", "-", "-", "-", "=", "=", "=", "#"],
    bar_empty: ".",
    spark_blocks: [".", ".", "_", "_", "-", "=", "=", "#"],
    permit_filled: "#",
    permit_empty: ".",
    ok: "+",
    skip: ">",
    warn: "!",
    fail: "x",
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
pub const TRACK_DARK: Rgb = Rgb(0x63, 0x4D, 0x3C);
pub const SAGE: Rgb = Rgb(0x8B, 0xB8, 0x7E);
pub const AMBER: Rgb = Rgb(0xE5, 0xA9, 0x3B);
pub const CRIMSON: Rgb = Rgb(0xD9, 0x6B, 0x6B);
pub const SLATE_INFO: Rgb = Rgb(0x7F, 0xA7, 0xB8);
pub const SLATE_INFO_BRIGHT: Rgb = Rgb(0xA8, 0xC9, 0xD8);

/// Archival light-theme adjustments from `pages/style.css`.
const STONE_LIGHT: Rgb = Rgb(0x5C, 0x4D, 0x40);
const TRACK_LIGHT: Rgb = Rgb(0xB5, 0xA6, 0x92);
const SAGE_LIGHT: Rgb = Rgb(0x2F, 0x6E, 0x3A);
const AMBER_LIGHT: Rgb = Rgb(0xA8, 0x68, 0x00);
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
                        (SLATE_LIGHT, SLATE_INFO)
                    } else {
                        (SLATE_INFO, SLATE_INFO_BRIGHT)
                    }
                } else if self.light_bg {
                    (DEEP_BRONZE, BRONZE)
                } else {
                    (BRONZE, GOLD)
                };
                Style::default().fg(start.lerp(end, t).to_ratatui())
            }
            ColorLevel::Ansi16 => {
                Style::default().fg(if is_hash { Color::Cyan } else { Color::Yellow })
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
}
