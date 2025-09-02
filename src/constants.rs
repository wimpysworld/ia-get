//! Application constants for ia-get
//!
//! Contains user agent strings, timeout values, and other configuration constants
//! optimized for Internet Archive API compliance.

/// Generate a compliant user agent string following Internet Archive recommendations
/// 
/// Format follows Mozilla/RFC standards while being descriptive enough for IA server logs:
/// - Tool name and version
/// - Contact information (GitHub repo)
/// - Brief description of purpose
/// - Platform information for compatibility tracking
pub fn get_user_agent() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let repo_url = env!("CARGO_PKG_REPOSITORY");

    format!(
        "ia-get-cli/{} (+{}; {}-{}) - Internet Archive batch downloader for research and archival purposes",
        version, repo_url, os, arch
    )
}

/// Static user agent string for HTTP requests (fallback)
pub const USER_AGENT: &str = "ia-get-cli/1.5.0 (+https://github.com/Gameaday/ia-get-cli) - Internet Archive batch downloader";

/// Timeout for all HTTP requests in seconds
pub const HTTP_TIMEOUT: u64 = 60;

/// Maximum concurrent connections per host (Internet Archive recommendation)
pub const MAX_CONCURRENT_CONNECTIONS: usize = 5;

/// Minimum delay between requests to same server (milliseconds)
pub const MIN_REQUEST_DELAY_MS: u64 = 100;

/// Default retry delay for transient errors (seconds)
pub const DEFAULT_RETRY_DELAY_SECS: u64 = 30;

/// Maximum retry delay cap (seconds) - 10 minutes is reasonable for large downloads
pub const MAX_RETRY_DELAY_SECS: u64 = 600;
