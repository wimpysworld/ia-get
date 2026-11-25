//! Application constants for ia-get

/// User agent string for HTTP requests
pub const USER_AGENT: &str = "ia-get";

/// Regex pattern for validating archive.org details URLs
pub const URL_PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+/?$";

/// Maximum length for XML content in debug output (characters)
pub const XML_DEBUG_TRUNCATE_LEN: usize = 1000;

/// Maximum number of retry attempts for transient request or download failures
pub const MAX_RETRIES: u32 = 3;

/// Initial delay between retries in milliseconds (doubles with each retry)
pub const INITIAL_RETRY_DELAY_MS: u64 = 1000;

/// Maximum exponential backoff delay between retries in seconds
pub const MAX_RETRY_DELAY_SECS: u64 = 60;

/// Maximum honourable `Retry-After` delay in seconds
pub const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Maximum number of HTTP redirects to follow per request
pub const MAX_REDIRECTS: usize = 10;
