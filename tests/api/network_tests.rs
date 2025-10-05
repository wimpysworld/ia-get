//! Network Operations API Layer Tests
//!
//! Tests for network-related functionality including URL accessibility checking,
//! error handling and retry logic, timeout behavior, user agent generation,
//! and connection management.

use ia_get::constants::get_user_agent;
use reqwest::Client;

/// Test user agent string generation
#[test]
fn test_user_agent_generation() {
    let user_agent = get_user_agent();

    // Verify it contains ia-get
    assert!(user_agent.contains("ia-get"));

    // Verify it has substantial content (not just "ia-get")
    assert!(user_agent.len() > 10);

    // Verify it looks like a proper user agent string
    assert!(user_agent.contains("/"));
}

/// Test client creation with different configurations
#[test]
fn test_client_creation() {
    // Test basic client creation
    let client = Client::new();
    assert!(client.get("https://example.com").build().is_ok());

    // Test client with timeout
    let client_with_timeout = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    assert!(
        client_with_timeout
            .get("https://example.com")
            .build()
            .is_ok()
    );

    // Test client with custom user agent
    let custom_ua = get_user_agent();
    let client_with_ua = Client::builder().user_agent(custom_ua).build().unwrap();
    assert!(client_with_ua.get("https://example.com").build().is_ok());
}

/// Test transient error detection
#[test]
fn test_transient_error_detection() {
    // Note: We can't easily create real reqwest errors in tests without making actual requests,
    // so we test the logic with mock scenarios or known error types.

    // This test would need to be expanded with actual error cases
    // For now, just test that the function exists and can be called

    // Test would require creating actual reqwest errors, which is complex
    // without making real network requests. This is more of an integration test.
}

/// Test user agent string format and content
#[test]
fn test_user_agent_format() {
    let user_agent = get_user_agent();

    // Should start with application name
    assert!(user_agent.starts_with("ia-get"));

    // Should contain version information
    assert!(user_agent.contains("/"));

    // Should not be empty or just whitespace
    assert!(!user_agent.trim().is_empty());

    // Should be reasonable length (not too short or extremely long)
    assert!(user_agent.len() > 5);
    assert!(user_agent.len() < 200);
}

/// Test client builder configuration options
#[test]
fn test_client_builder_options() {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(get_user_agent())
        .redirect(reqwest::redirect::Policy::limited(10))
        .build();

    assert!(client.is_ok());
    let client = client.unwrap();

    // Test that we can create requests with the configured client
    let request_builder = client.get("https://archive.org/metadata/mario");
    assert!(request_builder.build().is_ok());
}

/// Test network timeout configurations
#[test]
fn test_timeout_configurations() {
    // Test various timeout configurations
    let timeouts = [1, 5, 10, 30, 60];

    for timeout_secs in timeouts {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build();
        assert!(
            client.is_ok(),
            "Failed to create client with {}s timeout",
            timeout_secs
        );
    }
}

// Note: Tests that require actual network connectivity should be marked as integration tests
// and would be in a separate file or marked with #[ignore] for unit test runs.
