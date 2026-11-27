//! Archival Inline Instrument TUI and non-TTY output driver.
//!
//! Uses Ratatui `Viewport::Inline` over Crossterm in cooked terminal mode only
//! (never enabling raw mode or entering the alternate screen) so Ctrl+C signals
//! and shell scrollback work naturally.

pub mod check;
pub mod list;
pub mod palette;
pub mod render;
pub mod state;

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{execute, queue};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::downloader::{ElasticScheduler, ShutdownSignal};
use crate::ui::palette::{style_accent, style_ratatui_to_ansi, Theme};
use crate::ui::render::{
    compute_viewport_height_for_batch, format_final_summary, format_tty_summary,
    render_completion_line, render_instrument,
};
use crate::ui::state::{FilterContext, QueuedFile, UiEvent, UiState};

/// Target UI refresh interval (~30 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// Spinner tick interval (80ms per `tui-design` Braille recommendation).
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Number of active live inline instruments currently hiding the terminal cursor.
static CURSOR_HIDDEN_DEPTH: AtomicUsize = AtomicUsize::new(0);
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

fn install_cursor_panic_hook() {
    PANIC_HOOK_INSTALLED.get_or_init(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CURSOR_HIDDEN_DEPTH.load(Ordering::SeqCst) > 0 {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, Show);
            }
            prev(info);
        }));
    });
}

/// RAII guard hiding the terminal cursor in cooked mode and restoring it on every exit path.
struct CursorGuard {
    active: bool,
}

impl CursorGuard {
    fn hide() -> Self {
        install_cursor_panic_hook();
        CURSOR_HIDDEN_DEPTH.fetch_add(1, Ordering::SeqCst);
        let mut stdout = io::stdout();
        let active = execute!(stdout, Hide).is_ok();
        if !active {
            CURSOR_HIDDEN_DEPTH.fetch_sub(1, Ordering::SeqCst);
        }
        Self { active }
    }
}

impl Drop for CursorGuard {
    fn drop(&mut self) {
        if self.active {
            let mut stdout = io::stdout();
            let _ = execute!(stdout, Show);
            CURSOR_HIDDEN_DEPTH.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Returns `true` when `stdout` is an interactive terminal capable of inline TUI rendering.
pub fn is_interactive_stdout() -> bool {
    is_interactive_stream(io::stdout().is_terminal())
}

/// Returns `true` when `stderr` is an interactive terminal capable of styled diagnostic callouts.
pub fn is_interactive_stderr() -> bool {
    is_interactive_stream(io::stderr().is_terminal())
}

fn is_interactive_stream(is_tty: bool) -> bool {
    if !is_tty {
        return false;
    }
    if std::env::var("TERM")
        .ok()
        .is_some_and(|t| t.eq_ignore_ascii_case("dumb") || t.is_empty())
    {
        return false;
    }
    matches!(crossterm::terminal::size(), Ok((w, h)) if w > 0 && h > 0)
}

/// Returns the current terminal width in columns for `stdout` (falling back to 80).
pub fn stdout_terminal_width() -> u16 {
    crossterm::terminal::size()
        .map(|(w, _)| w)
        .ok()
        .filter(|&w| w > 0)
        .unwrap_or(80)
}

/// Configuration for launching a live batch UI session.
#[derive(Debug, Clone)]
pub struct UiSessionConfig {
    /// Archive item identifier displayed in the instrument header.
    pub identifier: String,
    /// Formatted destination directory displayed in the post-run Archival Receipt.
    pub output_dir: String,
    /// Total number of files scheduled in this batch.
    pub total_files: usize,
    /// Sum of known expected sizes across scheduled files.
    pub planned_total_bytes: u64,
    /// Optional user-supplied bandwidth cap in bytes/sec (`--limit-rate`).
    pub limit_rate: Option<u64>,
    /// Number of `<private>` files skipped before the batch began (without cookies).
    pub skipped_private_count: usize,
    /// Specific `<private>` filenames skipped before the batch began (without cookies).
    pub skipped_private_files: Vec<String>,
    /// Optional active `-f/--filter` context displayed in the Archival Receipt.
    pub filter: Option<FilterContext>,
    /// Ordered initial queue of files awaiting scheduler permits.
    pub initial_queue: Vec<QueuedFile>,
}

/// Cheaply cloneable, non-blocking handle for emitting [`UiEvent`] updates from transfer tasks.
#[derive(Clone, Debug)]
pub struct UiHandle {
    tx: mpsc::UnboundedSender<UiEvent>,
}

impl UiHandle {
    /// Sends a telemetry event to the UI task without blocking network or disk I/O.
    pub fn send(&self, event: UiEvent) {
        let _ = self.tx.send(event);
    }
}

/// Owns the spawned UI render task and coordinates clean teardown + post-run summary output.
pub struct UiDriver {
    tx: mpsc::UnboundedSender<UiEvent>,
    task: JoinHandle<()>,
}

impl UiDriver {
    /// Starts the batch UI driver on `stdout`, choosing inline Ratatui on TTY or plain lines on non-TTY.
    pub fn start(
        config: UiSessionConfig,
        scheduler: Option<ElasticScheduler>,
        signal: Option<ShutdownSignal>,
    ) -> (UiHandle, Self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = UiHandle { tx: tx.clone() };
        let interactive = is_interactive_stdout();
        let theme = Theme::detect_stdout();

        let task = if interactive {
            tokio::spawn(run_inline_tty_loop(config, theme, rx, scheduler, signal))
        } else {
            tokio::spawn(run_plain_stdout_loop(config, theme, rx))
        };

        (handle, Self { tx, task })
    }

    /// Starts a deterministic non-TTY plain UI driver writing into `writer` (for unit/integration tests).
    pub fn start_plain_with_writer<W>(
        config: UiSessionConfig,
        theme: Theme,
        writer: W,
    ) -> (UiHandle, Self)
    where
        W: Write + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = UiHandle { tx: tx.clone() };
        let task = tokio::spawn(run_plain_writer_loop(config, theme, rx, writer));
        (handle, Self { tx, task })
    }

    /// Signals completion, flushes remaining events, clears the inline frame, and prints the summary.
    pub async fn finish(self) {
        let _ = self.tx.send(UiEvent::Finish);
        let _ = self.task.await;
    }
}

fn init_state(config: &UiSessionConfig, now: Instant) -> UiState {
    let mut state = UiState::new(
        config.identifier.clone(),
        config.total_files,
        config.planned_total_bytes,
        config.limit_rate,
        now,
    );
    state.filter = config.filter.clone();
    state.on_event(
        UiEvent::BatchStarted {
            queue: config.initial_queue.clone(),
            output_dir: config.output_dir.clone(),
        },
        now,
    );
    if !config.skipped_private_files.is_empty() {
        state.on_event(
            UiEvent::SkippedPrivateFiles(config.skipped_private_files.clone()),
            now,
        );
    } else if config.skipped_private_count > 0 {
        state.on_event(
            UiEvent::SkippedPrivate {
                count: config.skipped_private_count,
            },
            now,
        );
    }
    state
}

async fn run_inline_tty_loop(
    config: UiSessionConfig,
    theme: Theme,
    mut rx: mpsc::UnboundedReceiver<UiEvent>,
    scheduler: Option<ElasticScheduler>,
    signal: Option<ShutdownSignal>,
) {
    let (term_w, term_h) = match crossterm::terminal::size() {
        Ok((w, h)) if w > 0 && h > 0 => (w, h),
        _ => {
            run_plain_stdout_loop(config, theme, rx).await;
            return;
        }
    };

    let viewport_height = compute_viewport_height_for_batch(term_w, term_h, config.total_files);
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height),
        },
    ) {
        Ok(term) => term,
        Err(_) => {
            // Fall back cleanly if the terminal rejects cursor-position queries.
            run_plain_stdout_loop(config, theme, rx).await;
            return;
        }
    };

    let _live_guard = signal.as_ref().map(ShutdownSignal::enter_live_ui);
    let cursor_guard = CursorGuard::hide();

    let mut state = init_state(&config, Instant::now());
    let mut ticker = tokio::time::interval(FRAME_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut finished = false;
    let mut viewport_top: Option<u16> = None;

    // Initial draw so the user sees the instrument immediately.
    let _ = terminal.draw(|frame| {
        viewport_top = Some(frame.area().y);
        render_instrument(frame, &state, &theme);
    });
    state.mark_clean();

    while !finished {
        tokio::select! {
            biased;
            maybe_event = rx.recv() => {
                let now = Instant::now();
                match maybe_event {
                    Some(ev) => {
                        if handle_event_for_tty(ev, &mut state, now) {
                            finished = true;
                        }
                        while let Ok(next_ev) = rx.try_recv() {
                            if handle_event_for_tty(next_ev, &mut state, now) {
                                finished = true;
                            }
                        }
                    }
                    None => {
                        state.on_event(UiEvent::Finish, now);
                        finished = true;
                    }
                }
                if !finished && state.is_dirty() {
                    let _ = terminal.draw(|frame| {
                        viewport_top = Some(frame.area().y);
                        render_instrument(frame, &state, &theme);
                    });
                    state.mark_clean();
                }
            }
            _ = ticker.tick() => {
                let now = Instant::now();
                if let Some(ref sched) = scheduler {
                    state.on_event(UiEvent::SchedulerUpdate(sched.snapshot()), now);
                }
                state.tick(now);
                if state.is_dirty() || state.is_animating() {
                    let _ = terminal.draw(|frame| {
                        viewport_top = Some(frame.area().y);
                        render_instrument(frame, &state, &theme);
                    });
                    state.mark_clean();
                }
            }
        }
    }

    // Drain any events queued just before Finish.
    let now = Instant::now();
    while let Ok(ev) = rx.try_recv() {
        handle_event_for_tty(ev, &mut state, now);
    }
    if state.finished_at.is_none() {
        state.on_event(UiEvent::Finish, now);
    }

    // Capture the final viewport origin, clear the transient live instrument, and
    // park the cursor at column 0 of `viewport_top` so the permanent Archival Receipt
    // replaces the inline box cleanly with zero blank gap or scrollback spam.
    let mut final_width = term_w;
    let _ = terminal.draw(|frame| {
        viewport_top = Some(frame.area().y);
        final_width = frame.area().width;
    });
    let _ = terminal.clear();
    drop(terminal);
    drop(cursor_guard);

    let receipt_width = crossterm::terminal::size()
        .map(|(w, _)| w)
        .ok()
        .filter(|&w| w > 0)
        .unwrap_or(final_width);
    let summary = format_tty_summary(&state, &theme, receipt_width);
    {
        let mut stdout = io::stdout().lock();
        if let Some(top_y) = viewport_top {
            let _ = queue!(
                stdout,
                crossterm::cursor::MoveTo(0, top_y),
                Clear(ClearType::FromCursorDown)
            );
        }
        let _ = writeln!(stdout, "{summary}");
        let _ = stdout.flush();
    }
    drop(_live_guard);
}

fn handle_event_for_tty(event: UiEvent, state: &mut UiState, now: Instant) -> bool {
    match event {
        UiEvent::Finish => {
            state.on_event(UiEvent::Finish, now);
            true
        }
        other => {
            state.on_event(other, now);
            false
        }
    }
}

/// Line-buffered stdout writer using `print!` so `cargo test` captures non-TTY output cleanly.
struct StdoutCaptureWriter;

impl Write for StdoutCaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        print!("{text}");
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

async fn run_plain_stdout_loop(
    config: UiSessionConfig,
    theme: Theme,
    rx: mpsc::UnboundedReceiver<UiEvent>,
) {
    run_plain_writer_loop(config, theme, rx, StdoutCaptureWriter).await;
}

async fn run_plain_writer_loop<W: Write>(
    config: UiSessionConfig,
    theme: Theme,
    mut rx: mpsc::UnboundedReceiver<UiEvent>,
    mut writer: W,
) {
    let mut state = init_state(&config, Instant::now());

    while let Some(event) = rx.recv().await {
        let now = Instant::now();
        match event {
            UiEvent::Finish => {
                state.on_event(UiEvent::Finish, now);
                break;
            }
            UiEvent::SlotFinished(record) => {
                let line = render_completion_line(&record, &theme, None);
                let _ = writeln!(writer, "{line}");
                let _ = writer.flush();
                state.on_event(UiEvent::SlotFinished(record), now);
            }
            other => {
                state.on_event(other, now);
            }
        }
    }

    while let Ok(event) = rx.try_recv() {
        let now = Instant::now();
        if let UiEvent::SlotFinished(record) = event {
            let line = render_completion_line(&record, &theme, None);
            let _ = writeln!(writer, "{line}");
            let _ = writer.flush();
            state.on_event(UiEvent::SlotFinished(record), now);
        } else if !matches!(event, UiEvent::Finish) {
            state.on_event(event, now);
        }
    }

    let summary = format_final_summary(&state, &theme);
    let _ = writeln!(writer, "{summary}");
    let _ = writer.flush();
}

/// Lightweight cooked-mode inline status indicator for pre-download metadata and disk checks.
///
/// Animates cleanly on interactive TTY `stdout` without raw mode or alternate screen, and stays
/// silent (emitting only completion/error lines) when `stdout` is non-TTY or hidden in tests.
pub struct StatusSpinner {
    hidden: bool,
    interactive: bool,
    theme: Theme,
    message: Arc<Mutex<String>>,
    active: Arc<AtomicBool>,
    finished: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl StatusSpinner {
    /// Starts a cooked-mode status indicator on `stdout` with an initial `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self::spawn_with_mode(message, false, Duration::ZERO)
    }

    /// Starts a cooked-mode sage MD5 verification spinner (`◐◓◑◒`) with a brief initial delay
    /// so very fast local checks do not flicker before printing the Verification Card.
    pub fn new_md5(message: impl Into<String>) -> Self {
        Self::spawn_with_mode(message, true, Duration::from_millis(80))
    }

    fn spawn_with_mode(
        message: impl Into<String>,
        md5_mode: bool,
        initial_delay: Duration,
    ) -> Self {
        let interactive = is_interactive_stdout();
        let theme = Theme::detect_stdout();
        let msg = Arc::new(Mutex::new(message.into()));
        let active = Arc::new(AtomicBool::new(interactive));

        let task = if interactive && tokio::runtime::Handle::try_current().is_ok() {
            let msg_clone = Arc::clone(&msg);
            let active_clone = Arc::clone(&active);
            Some(tokio::spawn(async move {
                if !initial_delay.is_zero() {
                    tokio::time::sleep(initial_delay).await;
                    if !active_clone.load(Ordering::SeqCst) {
                        return;
                    }
                }
                let glyphs = theme.glyphs();
                let mut tick_idx = 0usize;
                let mut interval = tokio::time::interval(if md5_mode {
                    Duration::from_millis(120)
                } else {
                    SPINNER_INTERVAL
                });
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                while active_clone.load(Ordering::SeqCst) {
                    interval.tick().await;
                    if !active_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    let styled_frame = if md5_mode {
                        let frame = glyphs.stage_md5[tick_idx % glyphs.stage_md5.len()];
                        style_ratatui_to_ansi(frame, theme.stage_md5_style(), &theme)
                    } else {
                        let frame = glyphs.spinner[tick_idx % glyphs.spinner.len()];
                        style_accent(frame, &theme)
                    };
                    tick_idx = tick_idx.wrapping_add(1);
                    let text = msg_clone.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let cols = crossterm::terminal::size()
                        .map(|(w, _)| w as usize)
                        .unwrap_or(80);
                    let max_text_cells = cols.saturating_sub(3);
                    let clipped = clip_ansi_line(&text, max_text_cells, glyphs.ellipsis);
                    let mut stdout = io::stdout().lock();
                    if !active_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = queue!(stdout, Clear(ClearType::CurrentLine));
                    let _ = write!(stdout, "\r{styled_frame} {clipped}");
                    let _ = stdout.flush();
                }
            }))
        } else {
            None
        };

        Self {
            hidden: false,
            interactive,
            theme,
            message: msg,
            active,
            finished: AtomicBool::new(false),
            task: Mutex::new(task),
        }
    }

    /// Creates a completely silent status indicator (for unit tests).
    pub fn hidden() -> Self {
        Self {
            hidden: true,
            interactive: false,
            theme: Theme::monochrome_unicode(),
            message: Arc::new(Mutex::new(String::new())),
            active: Arc::new(AtomicBool::new(false)),
            finished: AtomicBool::new(false),
            task: Mutex::new(None),
        }
    }

    /// Returns `true` after [`Self::finish_ok`], [`Self::finish_err`], or [`Self::finish_and_clear`].
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// Returns the detected theme for inline styling of messages passed to this spinner.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Updates the displayed status message.
    pub fn set_message(&self, message: impl Into<String>) {
        if self.hidden {
            return;
        }
        *self.message.lock().unwrap_or_else(|p| p.into_inner()) = message.into();
    }

    fn stop_animation(&self) {
        self.active.store(false, Ordering::SeqCst);
        if let Some(handle) = self.task.lock().unwrap_or_else(|p| p.into_inner()).take() {
            handle.abort();
        }
        if self.interactive {
            let mut stdout = io::stdout().lock();
            let _ = queue!(stdout, Clear(ClearType::CurrentLine));
            let _ = write!(stdout, "\r");
            let _ = stdout.flush();
        }
    }

    /// Temporarily clears the live spinner line while executing `f`.
    pub fn suspend<R>(&self, f: impl FnOnce() -> R) -> R {
        if self.interactive && self.active.load(Ordering::SeqCst) {
            let mut stdout = io::stdout().lock();
            let _ = queue!(stdout, Clear(ClearType::CurrentLine));
            let _ = write!(stdout, "\r");
            let _ = stdout.flush();
        }
        f()
    }

    /// Stops the spinner and prints a clean success line (no-op when hidden).
    pub fn finish_ok(&self, message: impl std::fmt::Display) {
        self.finished.store(true, Ordering::SeqCst);
        if self.hidden {
            return;
        }
        self.stop_animation();
        let glyphs = self.theme.glyphs();
        let ok_glyph = style_ratatui_to_ansi(
            glyphs.ok,
            self.theme.bold_if_color(self.theme.ok_style()),
            &self.theme,
        );
        println!("{ok_glyph} {message}");
    }

    /// Stops the spinner and clears the line without printing a final message.
    pub fn finish_and_clear(&self) {
        self.finished.store(true, Ordering::SeqCst);
        if self.hidden {
            return;
        }
        self.stop_animation();
    }

    /// Stops the spinner and prints a clean failure line (no-op when hidden).
    pub fn finish_err(&self, message: impl std::fmt::Display) {
        self.finished.store(true, Ordering::SeqCst);
        if self.hidden {
            return;
        }
        self.stop_animation();
        let glyphs = self.theme.glyphs();
        let fail_glyph = style_ratatui_to_ansi(
            glyphs.fail,
            self.theme.bold_if_color(self.theme.err_style()),
            &self.theme,
        );
        println!("{fail_glyph} {message}");
    }
}

impl Drop for StatusSpinner {
    fn drop(&mut self) {
        if !self.hidden && self.active.load(Ordering::SeqCst) {
            self.stop_animation();
        }
    }
}

/// Clips a single-line string that may contain ANSI SGR escapes so its visible display width
/// never exceeds `max_cells`, preventing cooked-mode spinner lines from wrapping and stacking.
fn clip_ansi_line(input: &str, max_cells: usize, ellipsis: &str) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    if max_cells == 0 {
        return String::new();
    }

    // Measure visible width first; pass through unchanged when already within budget.
    let mut visible_width = 0usize;
    let mut in_esc = false;
    for ch in input.chars() {
        if in_esc {
            if ch.is_ascii_alphabetic() {
                in_esc = false;
            }
            continue;
        }
        if ch == '\x1b' {
            in_esc = true;
            continue;
        }
        visible_width += ch.width().unwrap_or(0);
    }
    if visible_width <= max_cells {
        return input.to_string();
    }

    let ellipsis_w = ellipsis.width();
    let budget = max_cells.saturating_sub(ellipsis_w);
    let mut out = String::with_capacity(input.len() + ellipsis.len() + 4);
    let mut used = 0usize;
    let mut saw_ansi = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            saw_ansi = true;
            out.push(ch);
            for esc_ch in chars.by_ref() {
                out.push(esc_ch);
                if esc_ch.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if used + cw > budget {
            break;
        }
        out.push(ch);
        used += cw;
    }
    if max_cells >= ellipsis_w {
        out.push_str(ellipsis);
    }
    if saw_ansi {
        out.push_str("\x1b[0m");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CapturedLogs;
    use crate::ui::state::{CompletionKind, CompletionRecord, SkipReason};

    #[test]
    fn clip_ansi_line_prevents_spinner_wrapping_and_resets_sgr() {
        assert_eq!(clip_ansi_line("short line", 20, "…"), "short line");
        assert_eq!(
            clip_ansi_line("Fetching metadata: very_long_filename.xml", 20, "…"),
            "Fetching metadata: …"
        );
        let styled = "Fetching \x1b[1mvery_long_metadata_url\x1b[0m";
        let clipped = clip_ansi_line(styled, 16, "…");
        assert_eq!(clipped, "Fetching \x1b[1mvery_l…\x1b[0m");
    }

    #[tokio::test]
    async fn plain_driver_emits_clean_non_interleaved_lines_and_summary_without_ansi() {
        let captured = CapturedLogs::new();
        let config = UiSessionConfig {
            identifier: "test-item".to_string(),
            output_dir: "./downloads".to_string(),
            total_files: 3,
            planned_total_bytes: 300,
            limit_rate: None,
            skipped_private_count: 1,
            skipped_private_files: Vec::new(),
            filter: None,
            initial_queue: Vec::new(),
        };
        let (handle, driver) = UiDriver::start_plain_with_writer(
            config,
            Theme::monochrome_unicode(),
            captured.clone(),
        );

        handle.send(UiEvent::SlotFinished(CompletionRecord {
            slot_id: 1,
            file_path: "ok.bin".to_string(),
            kind: CompletionKind::Downloaded {
                bytes: 100,
                duration: Duration::from_millis(120),
                verified_md5: true,
            },
            finished_index: 1,
            total_files: 3,
        }));
        handle.send(UiEvent::SlotFinished(CompletionRecord {
            slot_id: 2,
            file_path: "cached.bin".to_string(),
            kind: CompletionKind::Skipped {
                reason: SkipReason::AlreadyVerified,
                bytes: 100,
            },
            finished_index: 2,
            total_files: 3,
        }));
        handle.send(UiEvent::SlotFinished(CompletionRecord {
            slot_id: 3,
            file_path: "mismatch.bin".to_string(),
            kind: CompletionKind::HashMismatch {
                got: "aaaa".to_string(),
                expected: "bbbb".to_string(),
            },
            finished_index: 3,
            total_files: 3,
        }));

        driver.finish().await;

        let raw = captured.as_bytes();
        assert!(
            !raw.contains(&0x1b),
            "plain non-TTY output must contain zero ANSI escape codes"
        );
        let output = String::from_utf8(raw).unwrap();
        assert!(
            output.contains("✓ ok.bin  (100B in 0.12s, MD5 ok)"),
            "output:\n{output}"
        );
        assert!(
            output.contains("↷ cached.bin  (skipped, already verified)"),
            "output:\n{output}"
        );
        assert!(
            output.contains("! mismatch.bin  (MD5 mismatch, kept .part)"),
            "output:\n{output}"
        );
        assert!(
            output.contains("Completed: 1 ok · 1 skipped · 2 warnings"),
            "output:\n{output}"
        );
        assert!(output.contains("Needs attention:"), "output:\n{output}");
        assert!(output.contains("got      aaaa"), "output:\n{output}");
        assert!(output.contains("expected bbbb"), "output:\n{output}");
        assert!(
            output.contains("1 private file skipped"),
            "output:\n{output}"
        );
    }
}
