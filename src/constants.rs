//! Application constants for ia-get

/// User agent string for HTTP requests
pub const USER_AGENT: &str = concat!(
    "ia-get/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/wimpysworld/ia-get)"
);

/// Per-chunk idle read timeout in seconds while streaming file downloads
pub const CHUNK_READ_TIMEOUT_SECS: u64 = 60;

/// Maximum length for XML content in debug output (bytes, truncated on a UTF-8 char boundary)
pub const XML_DEBUG_TRUNCATE_LEN: usize = 1000;

/// Maximum response size in bytes accepted when fetching `_files.xml` metadata (64 MiB)
pub const MAX_METADATA_XML_BYTES: usize = 64 * 1024 * 1024;

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
