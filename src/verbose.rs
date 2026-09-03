//! Opt-in diagnostic logging (`--verbose`): the request URLs, HTTP status
//! codes, and the session settings, printed to stderr so they can be captured
//! or filtered separately from the pretty stdout output.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns `--verbose` on or off for the whole process; called once at startup.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

/// Whether diagnostic logging is on.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Prints `message` to stderr when `--verbose` is on, prefixed so the line is
/// easy to spot. No-op otherwise.
pub fn log(message: &str) {
    if enabled() {
        eprintln!("[ia-get] {message}");
    }
}
