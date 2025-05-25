//! Application constants for ia-get

/// User agent string for HTTP requests
pub const USER_AGENT: &str = "ia-get";

/// Timeout for all HTTP requests in seconds
pub const HTTP_TIMEOUT: u64 = 60;

/// Regex pattern for validating archive.org details URLs
pub const URL_PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+/?$";

/// Maximum length for XML content in debug output (characters)
pub const XML_DEBUG_TRUNCATE_LEN: usize = 1000;
