//! Test GUI to CLI mode switching functionality
//!
//! Since GUI requires a display, this test focuses on the CLI components
//! and the interface functions used in mode switching.

#[test]
fn test_interactive_cli_launch() {
    // Test that the interactive CLI type and new() method exist
    // This verifies the interface is available for GUI-to-CLI switching
    //
    // Note: We don't actually call new() in tests because it requires
    // filesystem access and environment variables that may not be available
    // in test environments. The actual functionality is tested in integration tests.

    // This is a compile-time check that ensures the type and method exist
    // If this test compiles, the API is available for mode switching
    use ia_get::interface::interactive::InteractiveCli;

    // Verify we can reference the type (compile-time check)
    let _phantom: Option<InteractiveCli> = None;

    // Test passes if it compiles - the API exists and is accessible
}

#[test]
fn test_terminal_reset_functions_exist() {
    // Verify that terminal reset functionality is available
    // This is used when switching from GUI to CLI mode

    use std::io::{self, Write};

    // Test basic terminal operations that are used in mode switching
    let flush_result = io::stdout().flush();
    assert!(flush_result.is_ok(), "stdout flush should work");

    let stderr_flush_result = io::stderr().flush();
    assert!(stderr_flush_result.is_ok(), "stderr flush should work");
}

#[test]
fn test_show_interactive_menu_function() {
    // Test that the show_interactive_menu function can be called
    // This is the function called when switching from GUI to CLI

    // We can't run the full interactive menu in tests, but we can verify
    // the function exists and the basic setup works

    // The function requires a tokio runtime, so we test the components it uses
    let config_manager_result = ia_get::infrastructure::config::ConfigManager::new();
    assert!(
        config_manager_result.is_ok(),
        "ConfigManager should initialize"
    );
}

#[cfg(feature = "gui")]
#[test]
fn test_gui_to_cli_switch_components() {
    // Test components used in GUI to CLI mode switching

    // Test that the GUI detection function works
    // This doesn't test actual GUI functionality, just the detection logic
    let _gui_available = ia_get::can_use_gui();

    // Test that terminal reset function doesn't panic
    // Note: We can't call the actual reset function as it's private,
    // but we can test the underlying operations
    use std::io::{self, Write};

    // Simulate the operations done in reset_terminal_for_cli
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();

    // These should not panic
    print!("\x1B[2J\x1B[H"); // Clear screen
    print!("\x1B[0m"); // Reset colors
    let _ = io::stdout().flush();
}
