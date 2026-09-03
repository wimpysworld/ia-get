//! Ctrl+C signal handling for graceful batch shutdown.
//!
//! The first press asks the batch to stop gracefully; a second one quits the
//! process immediately, so a long wait (e.g. a server-requested Retry-After)
//! can always be aborted.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use colored::*;

/// Process-wide "should stop" flag, registered on first use.
static RUNNING_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Sets up signal handling for graceful shutdown on Ctrl+C
///
/// Returns an Arc<AtomicBool> that can be checked to see if the process
/// should stop. The first Ctrl+C sets it to false; a second one quits the
/// process immediately, so a long Retry-After wait can always be aborted.
///
/// Idempotent: repeated calls return the same flag; the handler is
/// registered only once (a second `ctrlc::set_handler` would panic).
pub(crate) fn setup_signal_handler() -> Arc<AtomicBool> {
    let running = RUNNING_FLAG.get_or_init(|| {
        let running = Arc::new(AtomicBool::new(true));
        let presses = Arc::new(AtomicU32::new(0));
        let r = running.clone();
        let p = presses.clone();

        ctrlc::set_handler(move || match handle_ctrl_c(&r, &p) {
            CtrlCAction::GracefulStop => {}
            CtrlCAction::QuitNow => std::process::exit(1),
        })
        .expect("Error setting Ctrl+C handler");

        running
    });
    running.clone()
}

/// What a Ctrl+C press must do
enum CtrlCAction {
    /// The batch was asked to stop gracefully
    GracefulStop,
    /// The process must terminate immediately
    QuitNow,
}

/// Ctrl+C handling: the first press asks the batch to stop gracefully, the
/// second one asks for an immediate quit. Kept apart from the handler
/// registration so the behaviour is testable without registering a second
/// (panicking) Ctrl+C handler.
fn handle_ctrl_c(running: &Arc<AtomicBool>, presses: &Arc<AtomicU32>) -> CtrlCAction {
    if presses.fetch_add(1, Ordering::SeqCst) == 0 {
        running.store(false, Ordering::SeqCst);
        println!(
            "\n{} Received Ctrl+C, finishing current operation...",
            "✘".red().bold()
        );
        CtrlCAction::GracefulStop
    } else {
        println!(
            "\n{} {} Quitting now.",
            "✘".red().bold(),
            "Ctrl+C".red().bold()
        );
        CtrlCAction::QuitNow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_ctrl_c_press_requests_immediate_quit() {
        let running = Arc::new(AtomicBool::new(true));
        let presses = Arc::new(AtomicU32::new(0));

        assert!(
            matches!(handle_ctrl_c(&running, &presses), CtrlCAction::GracefulStop),
            "the first press must stop the batch gracefully"
        );
        assert!(
            !running.load(Ordering::SeqCst),
            "the first press must flag the batch to stop"
        );

        assert!(
            matches!(handle_ctrl_c(&running, &presses), CtrlCAction::QuitNow),
            "the second press must ask the handler to exit the process"
        );
    }
}
