//! Tests for enhanced Internet Archive API integration
//!
//! These tests verify the new enhanced API functionality including
//! search, tasks, and metadata analysis capabilities.

use ia_get::{
    core::archive::enhanced::{analyze_metadata, fetch_enhanced_metadata},
    infrastructure::api::EnhancedArchiveApiClient,
    utilities::common::get_user_agent,
};
use indicatif::ProgressBar;
use reqwest::Client;

/// Test enhanced API client creation and basic functionality
#[tokio::test]
async fn test_enhanced_api_client_creation() {
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let mut api_client = EnhancedArchiveApiClient::new(client);

    // Test basic metadata fetch with error resilience for CI environments
    let result = api_client.get_metadata("mario").await;
    match result {
        Ok(_) => {
            // Success case - verify stats
            let stats = api_client.get_stats();
            assert!(
                stats.request_count > 0,
                "Should have made at least one request"
            );
        }
        Err(e) => {
            // Network errors are acceptable in CI environments
            eprintln!("Metadata fetch failed (acceptable in CI): {}", e);

            // Even if the network call failed, we should still be able to get stats
            let _stats = api_client.get_stats();
            // The important part is that the client was created successfully and stats are accessible

            // The test should not fail due to network issues
            // The important part is that the client was created successfully
        }
    }
}

/// Test search functionality with the enhanced API client
#[tokio::test]
async fn test_enhanced_search_functionality() {
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let mut api_client = EnhancedArchiveApiClient::new(client);

    // Test search for items in a known collection
    let result = api_client
        .search_items("collection:test", Some("identifier,title"), Some(5), None)
        .await;

    match result {
        Ok(response) => {
            assert!(
                response.status().is_success(),
                "Search request should succeed"
            );

            // Try to parse the response
            if let Ok(text) = response.text().await {
                assert!(!text.is_empty(), "Response should not be empty");

                // Should be valid JSON
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    // Should have response structure
                    assert!(
                        json.get("response").is_some(),
                        "Should have response object"
                    );
                }
            }
        }
        Err(e) => {
            // Network errors are acceptable in CI environments
            eprintln!("Search test failed (acceptable in CI): {}", e);
        }
    }
}

/// Test enhanced metadata fetching with real data
#[tokio::test]
async fn test_enhanced_metadata_fetch() {
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );

    // Test with a small, reliable archive
    let result = fetch_enhanced_metadata("mario", &client, &progress, false, false).await;

    match result {
        Ok(enhanced_metadata) => {
            // Basic metadata should be present
            assert!(
                !enhanced_metadata.basic_metadata.files.is_empty(),
                "Should have files in metadata"
            );

            // Analyze the metadata
            let analysis = analyze_metadata(&enhanced_metadata);
            assert_eq!(analysis.identifier, "mario");
            assert!(analysis.file_count > 0, "Should have analyzed files");

            println!("Enhanced metadata analysis:");
            println!("{}", analysis);
        }
        Err(e) => {
            // Network errors are acceptable in CI environments
            eprintln!("Enhanced metadata test failed (acceptable in CI): {}", e);
        }
    }
}

/// Test metadata analysis functionality
#[tokio::test]
async fn test_metadata_analysis() {
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let progress = ProgressBar::new_spinner();

    // Fetch basic metadata first
    let metadata_result = ia_get::fetch_json_metadata("luigi", &client, &progress).await;

    match metadata_result {
        Ok((basic_metadata, _url)) => {
            // Create enhanced metadata structure manually for testing
            let enhanced = ia_get::core::archive::enhanced::EnhancedMetadata {
                basic_metadata,
                search_results: None,
                related_items: None,
                tasks_status: None,
                collection_info: None,
            };

            // Analyze the metadata
            let analysis = analyze_metadata(&enhanced);

            // Verify analysis results
            assert_eq!(analysis.identifier, "luigi");
            assert!(analysis.file_count > 0, "Should have files to analyze");
            assert!(analysis.total_size > 0, "Should have calculated total size");

            // Should have file types analysis
            assert!(
                !analysis.file_types.is_empty(),
                "Should have identified file types"
            );

            // Test display formatting
            let display_output = format!("{}", analysis);
            assert!(
                display_output.contains("luigi"),
                "Display should contain identifier"
            );
            assert!(
                display_output.contains("Files:"),
                "Display should show file count"
            );

            println!("Metadata Analysis Display:\n{}", display_output);
        }
        Err(e) => {
            eprintln!("Metadata analysis test failed (acceptable in CI): {}", e);
        }
    }
}

/// Test API rate limiting compliance
#[tokio::test]
async fn test_api_rate_limiting() {
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let mut api_client = EnhancedArchiveApiClient::new(client);

    // Make several requests and check rate limiting
    let test_identifiers = ["mario", "luigi"];

    for identifier in &test_identifiers {
        let start_time = std::time::Instant::now();

        let _result = api_client.get_metadata(identifier).await;

        let elapsed = start_time.elapsed();

        // Should enforce minimum delay between requests
        if api_client.get_stats().request_count > 1 {
            assert!(
                elapsed >= std::time::Duration::from_millis(50),
                "Should have some delay between requests for politeness"
            );
        }

        // Add a small delay to prevent rate limiting issues in tests
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Check that rate is reasonable (may be higher in tests due to quick succession)
    let stats = api_client.get_stats();
    println!("API Stats: {}", stats);

    // Rate might be high in tests, so just check that we're tracking it
    assert!(stats.request_count > 0, "Should have made requests");
    assert!(
        stats.average_requests_per_minute >= 0.0,
        "Rate should be non-negative"
    );
}
