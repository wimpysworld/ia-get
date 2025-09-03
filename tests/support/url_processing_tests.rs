//! URL Processing Support Layer Tests
//!
//! Tests for URL validation, identifier extraction, and URL construction
//! functionality in the support layer.

use ia_get::url_processing::{
    construct_download_url, construct_metadata_url, extract_identifier_from_url, is_archive_url,
    normalize_archive_identifier, validate_and_process_url,
};

#[test]
fn test_validate_and_process_url_valid_archive_url() {
    let url = "https://archive.org/details/example";
    let result = validate_and_process_url(url).unwrap();
    assert_eq!(result, "https://archive.org/details/example");
}

#[test]
fn test_validate_and_process_url_identifier() {
    let identifier = "example";
    let result = validate_and_process_url(identifier).unwrap();
    assert_eq!(result, "https://archive.org/details/example");
}

#[test]
fn test_validate_and_process_url_invalid_url() {
    let url = "https://example.com/test";
    let result = validate_and_process_url(url);
    assert!(result.is_err());
}

#[test]
fn test_is_archive_url() {
    assert!(is_archive_url("https://archive.org/details/example"));
    assert!(is_archive_url("https://web.archive.org/details/example"));
    assert!(!is_archive_url("https://example.com"));
    assert!(!is_archive_url("invalid"));
}

#[test]
fn test_extract_identifier_from_url() {
    let url = "https://archive.org/details/example";
    let result = extract_identifier_from_url(url).unwrap();
    assert_eq!(result, "example");
}

#[test]
fn test_extract_identifier_from_url_metadata() {
    let url = "https://archive.org/metadata/example";
    let result = extract_identifier_from_url(url).unwrap();
    assert_eq!(result, "example");
}

#[test]
fn test_extract_identifier_from_url_download() {
    let url = "https://archive.org/download/example";
    let result = extract_identifier_from_url(url).unwrap();
    assert_eq!(result, "example");
}

#[test]
fn test_extract_identifier_complex_names() {
    // Test complex identifiers like the user's examples
    let url1 = "https://archive.org/details/mario";
    assert_eq!(extract_identifier_from_url(url1).unwrap(), "mario");

    let url2 = "https://archive.org/details/ikaos-som-dragon-ball-complete-001-153-r2j-dragon-box-multi-audio-v4_202301";
    assert_eq!(
        extract_identifier_from_url(url2).unwrap(),
        "ikaos-som-dragon-ball-complete-001-153-r2j-dragon-box-multi-audio-v4_202301"
    );

    let url3 = "https://archive.org/download/luigi";
    assert_eq!(extract_identifier_from_url(url3).unwrap(), "luigi");
}

#[test]
fn test_extract_identifier_from_url_invalid() {
    let url = "https://example.com/test";
    let result = extract_identifier_from_url(url);
    assert!(result.is_err());
}

#[test]
fn test_construct_metadata_url() {
    let identifier = "example";
    let result = construct_metadata_url(identifier);
    assert_eq!(result, "https://archive.org/metadata/example");
}

#[test]
fn test_construct_download_url() {
    let identifier = "example";
    let result = construct_download_url(identifier);
    assert_eq!(result, "https://archive.org/download/example");
}

#[test]
fn test_normalize_archive_identifier() {
    // Test plain identifier
    assert_eq!(normalize_archive_identifier("mario").unwrap(), "mario");

    // Test details URL
    assert_eq!(
        normalize_archive_identifier("https://archive.org/details/mario").unwrap(),
        "mario"
    );

    // Test download URL
    assert_eq!(
        normalize_archive_identifier("https://archive.org/download/luigi").unwrap(),
        "luigi"
    );

    // Test metadata URL
    assert_eq!(
        normalize_archive_identifier("https://archive.org/metadata/example").unwrap(),
        "example"
    );

    // Test complex identifier
    let complex_id = "ikaos-som-dragon-ball-complete-001-153-r2j-dragon-box-multi-audio-v4_202301";
    assert_eq!(
        normalize_archive_identifier(complex_id).unwrap(),
        complex_id
    );
    assert_eq!(
        normalize_archive_identifier(&format!("https://archive.org/details/{}", complex_id))
            .unwrap(),
        complex_id
    );
}

#[test]
fn test_normalize_archive_identifier_invalid() {
    // Test invalid archive URL structure (should fail since it's an archive.org URL with invalid path)
    let result = normalize_archive_identifier("https://archive.org/invalid/path");
    assert!(result.is_err());

    // Test malformed archive URL (no identifier after /details/)
    let result = normalize_archive_identifier("https://archive.org/details/");
    assert!(result.is_err());

    // Test malformed archive URL (no identifier after /download/)
    let result = normalize_archive_identifier("https://archive.org/download/");
    assert!(result.is_err());

    // Note: Non-archive URLs like "https://example.com/test" are treated as plain identifiers,
    // which is valid behavior - they get returned as-is
}
