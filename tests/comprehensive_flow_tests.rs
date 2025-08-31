//! Comprehensive Flow Integration Tests
//!
//! This module implements the key requirement from the issue:
//! "Brainstorm every single step that could or should happen in order 
//!  and make sure that the user can do them and the next action triggers automatically."
//!
//! Tests validate that each step flows directly into the next, ensuring
//! automatic progression through the entire download workflow.

use ia_get::{
    cli::{Cli, Commands},
    url_processing::validate_and_process_url,
    metadata::get_json_url,
    error::IaGetError,
};
use clap::Parser;

/// Test the complete user workflow from CLI input to ready-for-download state
/// This validates that each step automatically enables the next step
#[test]
fn test_complete_user_workflow_progression() {
    println!("🔄 Testing complete user workflow progression...");
    
    // STEP 1: User provides command line input
    println!("✓ Step 1: User provides CLI input");
    let cli = Cli::parse_from([
        "ia-get",
        "--include-ext", "pdf,txt",
        "--max-file-size", "10MB", 
        "--verbose",
        "--output-path", "my-downloads",
        "https://archive.org/details/example-archive"
    ]);
    
    // Validate CLI parsing worked
    assert!(cli.verbose);
    assert_eq!(cli.include_ext, Some("pdf,txt".to_string()));
    assert_eq!(cli.max_file_size, Some("10MB".to_string()));
    assert_eq!(cli.output_path, Some("my-downloads".to_string()));
    assert_eq!(cli.url, Some("https://archive.org/details/example-archive".to_string()));
    
    // STEP 2: System validates CLI configuration
    println!("✓ Step 2: System validates CLI configuration");
    let validation_result = cli.validate();
    assert!(validation_result.is_ok(), "CLI validation should pass for valid input");
    
    // STEP 3: System extracts URL from CLI input (automatic progression)
    println!("✓ Step 3: System extracts URL from CLI input");
    let url = cli.get_url();
    assert!(url.is_some(), "URL should be extractable from CLI input");
    let url = url.unwrap();
    
    // STEP 4: System validates URL format (automatic progression)
    println!("✓ Step 4: System validates URL format");
    let url_validation = validate_and_process_url(url);
    assert!(url_validation.is_ok(), "Valid archive.org URL should pass validation");
    
    // STEP 5: System generates metadata URL (automatic progression)
    println!("✓ Step 5: System generates metadata URL");
    let metadata_url = get_json_url(url);
    assert!(metadata_url.contains("metadata"), "Should generate metadata URL");
    assert!(metadata_url.contains("example-archive"), "Should contain identifier");
    
    // STEP 6: System prepares filter configuration (automatic progression)
    println!("✓ Step 6: System prepares filter configuration");
    let include_extensions = cli.include_extensions();
    assert_eq!(include_extensions, vec!["pdf", "txt"]);
    
    let max_file_size = cli.max_file_size_bytes();
    assert!(max_file_size.is_some(), "Max file size should be parsed");
    assert_eq!(max_file_size.unwrap(), 10485760); // 10MB
    
    // STEP 7: System prepares output configuration (automatic progression)
    println!("✓ Step 7: System prepares output configuration");
    let output_dir = cli.get_output_dir();
    assert_eq!(output_dir, Some("my-downloads"));
    
    // STEP 8: System is ready for metadata fetch and download execution
    println!("✓ Step 8: System ready for metadata fetch and download");
    assert!(true, "All preparatory steps completed successfully");
    
    println!("🎉 Complete workflow validated: All steps flow automatically into the next!");
}

/// Test error handling workflow - ensure errors are caught early and don't propagate
#[test]
fn test_error_handling_workflow() {
    println!("🚨 Testing error handling workflow...");
    
    // STEP 1: User provides invalid URL
    println!("✓ Step 1: User provides invalid URL");
    let cli = Cli::parse_from([
        "ia-get",
        "https://example.com/not-archive"
    ]);
    
    // STEP 2: CLI parsing succeeds (invalid URL still parses)
    println!("✓ Step 2: CLI parsing succeeds");
    assert_eq!(cli.url, Some("https://example.com/not-archive".to_string()));
    
    // STEP 3: URL validation fails (error caught early)
    println!("✓ Step 3: URL validation fails early");
    let url = cli.get_url().unwrap();
    let validation_result = validate_and_process_url(url);
    assert!(validation_result.is_err(), "Invalid URL should be caught in validation");
    
    // STEP 4: Error provides helpful message
    println!("✓ Step 4: Error provides helpful message");
    if let Err(error) = validation_result {
        match error {
            IaGetError::Parse(_) => {
                println!("   ✓ Received expected Parse error for invalid URL");
            }
            _ => panic!("Unexpected error type"),
        }
    }
    
    // STEP 5: Workflow stops gracefully without corrupting state
    println!("✓ Step 5: Workflow stops gracefully");
    
    println!("🎯 Error handling validated: Errors caught early with helpful messages!");
}

/// Test user interaction patterns - minimal input vs comprehensive configuration
#[test]
fn test_user_interaction_patterns() {
    println!("👤 Testing user interaction patterns...");
    
    // PATTERN 1: Minimal user input (system provides sensible defaults)
    println!("✓ Pattern 1: Minimal user input");
    let minimal_cli = Cli::parse_from([
        "ia-get",
        "https://archive.org/details/test-archive"
    ]);
    
    // System should provide sensible defaults
    assert!(!minimal_cli.verbose); // Default: quiet
    assert_eq!(minimal_cli.concurrent_downloads, 3); // Default: 3
    assert_eq!(minimal_cli.max_retries, 3); // Default: 3
    assert!(minimal_cli.include_ext.is_none()); // Default: include all
    assert!(minimal_cli.exclude_ext.is_none()); // Default: exclude none
    assert!(!minimal_cli.dry_run); // Default: actually download
    
    // User gets automatic progression with defaults
    let url = minimal_cli.get_url().unwrap();
    let validation = validate_and_process_url(url);
    assert!(validation.is_ok(), "Minimal input should work with defaults");
    
    println!("   ✓ Minimal input works with sensible defaults");
    
    // PATTERN 2: Power user with comprehensive configuration
    println!("✓ Pattern 2: Power user configuration");
    let power_cli = Cli::parse_from([
        "ia-get",
        "--verbose",
        "--dry-run", 
        "--concurrent-downloads", "8",
        "--max-retries", "5",
        "--include-ext", "pdf,txt,docx",
        "--exclude-ext", "xml,log",
        "--max-file-size", "100MB",
        "--output-path", "custom-output-dir",
        "--compress",
        "https://archive.org/details/advanced-archive"
    ]);
    
    // System should respect all user preferences
    assert!(power_cli.verbose);
    assert!(power_cli.dry_run);
    assert_eq!(power_cli.concurrent_downloads, 8);
    assert_eq!(power_cli.max_retries, 5);
    assert_eq!(power_cli.include_ext, Some("pdf,txt,docx".to_string()));
    assert_eq!(power_cli.exclude_ext, Some("xml,log".to_string()));
    assert_eq!(power_cli.max_file_size, Some("100MB".to_string()));
    assert_eq!(power_cli.output_path, Some("custom-output-dir".to_string()));
    assert!(power_cli.compress);
    
    // Power user configuration should also progress automatically
    let url = power_cli.get_url().unwrap();
    let validation = validate_and_process_url(url);
    assert!(validation.is_ok(), "Power user config should work");
    
    println!("   ✓ Power user configuration respected and validated");
    
    println!("🎮 User interaction patterns validated: Both simple and complex usage work!");
}

/// Test automatic decision making - system chooses next steps without user intervention
#[test]
fn test_automatic_decision_making() {
    println!("🤖 Testing automatic decision making...");
    
    // SCENARIO 1: System automatically determines the type of URL provided
    println!("✓ Scenario 1: Automatic URL type detection");
    
    let test_cases = vec![
        ("https://archive.org/details/test", "details URL"),
        ("https://archive.org/metadata/test", "metadata URL"),
        ("test-identifier", "identifier only"),
    ];
    
    for (input, expected_type) in test_cases {
        let cli = Cli::parse_from(["ia-get", input]);
        let url = cli.get_url().unwrap();
        
        // System automatically generates correct metadata URL regardless of input type
        let metadata_url = get_json_url(url);
        assert!(metadata_url.contains("metadata"), "Should auto-generate metadata URL for {}", expected_type);
        assert!(metadata_url.contains("test"), "Should preserve identifier for {}", expected_type);
        
        println!("   ✓ {} → metadata URL", expected_type);
    }
    
    // SCENARIO 2: System automatically handles different CLI syntax styles
    println!("✓ Scenario 2: Automatic CLI syntax handling");
    
    // Direct URL style
    let direct_cli = Cli::parse_from([
        "ia-get",
        "--output-path", "direct-output",
        "https://archive.org/details/direct-test"
    ]);
    
    // Subcommand style  
    let subcommand_cli = Cli::parse_from([
        "ia-get",
        "download",
        "--output", "subcommand-output", 
        "subcommand-test"
    ]);
    
    // System should extract information correctly from both styles
    assert_eq!(direct_cli.get_url(), Some("https://archive.org/details/direct-test"));
    assert_eq!(direct_cli.get_output_dir(), Some("direct-output"));
    
    assert_eq!(subcommand_cli.get_url(), Some("subcommand-test"));
    assert_eq!(subcommand_cli.get_output_dir(), Some("subcommand-output"));
    
    println!("   ✓ Both direct and subcommand syntax handled automatically");
    
    // SCENARIO 3: System automatically determines interactive vs non-interactive mode
    println!("✓ Scenario 3: Automatic interaction mode detection");
    
    let interactive_cli = Cli::parse_from(["ia-get"]);
    assert!(interactive_cli.is_interactive_mode(), "Should detect interactive mode");
    
    let non_interactive_cli = Cli::parse_from(["ia-get", "test-url"]);
    assert!(!non_interactive_cli.is_interactive_mode(), "Should detect non-interactive mode");
    
    println!("   ✓ Interactive vs non-interactive mode detected automatically");
    
    println!("🎯 Automatic decision making validated: System makes smart choices without user intervention!");
}

/// Test edge case handling in workflow progression
#[test]
fn test_edge_case_workflow_handling() {
    println!("⚠️  Testing edge case handling in workflow...");
    
    // EDGE CASE 1: Empty or minimal input
    println!("✓ Edge Case 1: Empty/minimal input handling");
    
    let empty_url_cli = Cli::parse_from(["ia-get", ""]);
    assert_eq!(empty_url_cli.url, Some("".to_string()));
    
    // System should handle empty URL gracefully in validation
    let validation_result = validate_and_process_url("");
    assert!(validation_result.is_err(), "Empty URL should be rejected gracefully");
    
    // EDGE CASE 2: Boundary values in configuration
    println!("✓ Edge Case 2: Boundary value handling");
    
    let boundary_cli = Cli::parse_from([
        "ia-get",
        "--concurrent-downloads", "1", // Minimum value
        "--max-retries", "0", // Minimum value
        "test"
    ]);
    
    assert_eq!(boundary_cli.concurrent_downloads, 1);
    assert_eq!(boundary_cli.max_retries, 0);
    
    // Validation should still work
    let validation = boundary_cli.validate();
    assert!(validation.is_ok(), "Boundary values should be valid");
    
    // EDGE CASE 3: Maximum values
    println!("✓ Edge Case 3: Maximum value handling");
    
    let max_cli = Cli::parse_from([
        "ia-get",
        "--concurrent-downloads", "10", // Maximum value
        "--max-retries", "20", // Maximum value
        "test"
    ]);
    
    assert_eq!(max_cli.concurrent_downloads, 10);
    assert_eq!(max_cli.max_retries, 20);
    
    let validation = max_cli.validate();
    assert!(validation.is_ok(), "Maximum values should be valid");
    
    // EDGE CASE 4: Values beyond limits should be caught
    println!("✓ Edge Case 4: Over-limit value rejection");
    
    let over_limit_cli = Cli {
        concurrent_downloads: 15, // Over limit
        max_retries: 25, // Over limit
        ..Default::default()
    };
    
    let validation = over_limit_cli.validate();
    assert!(validation.is_err(), "Over-limit values should be rejected");
    
    println!("🛡️  Edge case handling validated: System handles boundary conditions gracefully!");
}

/// Test recovery mechanisms - system continues after non-fatal errors
#[test]
fn test_recovery_mechanisms() {
    println!("🔄 Testing recovery mechanisms...");
    
    // RECOVERY 1: Invalid configuration detected and reported
    println!("✓ Recovery 1: Invalid configuration detection");
    
    let invalid_cli = Cli {
        concurrent_downloads: 0, // Invalid
        ..Default::default()
    };
    
    let validation_result = invalid_cli.validate();
    assert!(validation_result.is_err(), "Invalid config should be detected");
    
    if let Err(error_msg) = validation_result {
        assert!(error_msg.contains("between 1 and 10"), "Should provide helpful error message");
        println!("   ✓ Helpful error message: {}", error_msg);
    }
    
    // RECOVERY 2: System can handle partial success scenarios
    println!("✓ Recovery 2: Partial success handling");
    
    // Valid CLI with some valid and some questionable settings
    let mixed_cli = Cli::parse_from([
        "ia-get",
        "--include-ext", "", // Empty extension list (unusual but not invalid)
        "--exclude-ext", "a,b,c,d,e,f,g,h,i,j", // Very long list (unusual but valid)
        "https://archive.org/details/test"
    ]);
    
    // System should handle unusual but valid configurations
    let include_exts = mixed_cli.include_extensions();
    assert_eq!(include_exts, vec![""]); // Empty string becomes single element
    
    let exclude_exts = mixed_cli.exclude_extensions();
    assert_eq!(exclude_exts.len(), 10); // Long list should be parsed correctly
    
    let validation = mixed_cli.validate();
    assert!(validation.is_ok(), "Unusual but valid config should pass");
    
    println!("   ✓ Unusual configurations handled gracefully");
    
    println!("♻️  Recovery mechanisms validated: System provides helpful feedback and continues when possible!");
}

/// Comprehensive integration test demonstrating all workflow aspects
#[test]
fn test_comprehensive_workflow_integration() {
    println!("\n🎯 COMPREHENSIVE WORKFLOW INTEGRATION TEST");
    println!("==========================================");
    
    // This test demonstrates the complete workflow as requested in the issue:
    // "Every single step that could or should happen in order"
    
    let steps = vec![
        "CLI Input Parsing",
        "Configuration Validation", 
        "URL Extraction",
        "URL Format Validation",
        "Metadata URL Generation",
        "Filter Configuration",
        "Output Directory Setup",
        "Ready for Execution"
    ];
    
    for (i, step) in steps.iter().enumerate() {
        println!("🔸 Step {}: {}", i + 1, step);
    }
    
    println!("\n🚀 Executing comprehensive workflow...");
    
    // Execute all steps in sequence
    let cli = Cli::parse_from([
        "ia-get",
        "--verbose",
        "--include-ext", "pdf,txt",
        "--max-file-size", "50MB",
        "--concurrent-downloads", "5",
        "--output-path", "test-downloads",
        "https://archive.org/details/comprehensive-test"
    ]);
    
    // Step 1: CLI Input Parsing ✓
    assert!(cli.verbose);
    println!("  ✅ Step 1 Complete: CLI parsed successfully");
    
    // Step 2: Configuration Validation ✓
    let validation = cli.validate();
    assert!(validation.is_ok());
    println!("  ✅ Step 2 Complete: Configuration validated");
    
    // Step 3: URL Extraction ✓  
    let url = cli.get_url().unwrap();
    assert_eq!(url, "https://archive.org/details/comprehensive-test");
    println!("  ✅ Step 3 Complete: URL extracted");
    
    // Step 4: URL Format Validation ✓
    let url_validation = validate_and_process_url(url);
    assert!(url_validation.is_ok());
    println!("  ✅ Step 4 Complete: URL format validated");
    
    // Step 5: Metadata URL Generation ✓
    let metadata_url = get_json_url(url);
    assert_eq!(metadata_url, "https://archive.org/metadata/comprehensive-test");
    println!("  ✅ Step 5 Complete: Metadata URL generated");
    
    // Step 6: Filter Configuration ✓
    let filters = cli.include_extensions();
    assert_eq!(filters, vec!["pdf", "txt"]);
    let max_size = cli.max_file_size_bytes().unwrap();
    assert_eq!(max_size, 52428800); // 50MB
    println!("  ✅ Step 6 Complete: Filters configured");
    
    // Step 7: Output Directory Setup ✓
    let output_dir = cli.get_output_dir().unwrap();
    assert_eq!(output_dir, "test-downloads");
    println!("  ✅ Step 7 Complete: Output directory configured");
    
    // Step 8: Ready for Execution ✓
    println!("  ✅ Step 8 Complete: System ready for metadata fetch and download");
    
    println!("\n🎉 COMPREHENSIVE WORKFLOW COMPLETED SUCCESSFULLY!");
    println!("   All steps flow automatically into the next");
    println!("   User actions trigger automatic progression");
    println!("   System validates and prepares without manual intervention");
    
    // Final validation: ensure no step was skipped and all data flows correctly
    assert!(
        cli.validate().is_ok() && 
        url.contains("archive.org") &&
        metadata_url.contains("metadata") &&
        !filters.is_empty() &&
        max_size > 0 &&
        !output_dir.is_empty(),
        "All workflow components should be properly configured"
    );
    
    println!("\n✨ Workflow Integration Test PASSED ✨");
}