//! Application constants for ia-get
//!
//! Contains user agent strings, timeout values, and other configuration constants.

/// Generate a dynamic user agent string with system information
pub fn get_user_agent() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // Get additional system info if available
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let username = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".to_string());

    format!(
        "ia-get-cli/{} ({}-{}; user:{}; host:{}) Internet Archive Downloader",
        version, os, arch, username, hostname
    )
}

/// Static user agent string for HTTP requests (fallback)
pub const USER_AGENT: &str = "ia-get-cli/1.0 (Internet Archive File Downloader)";

/// Timeout for all HTTP requests in seconds
pub const HTTP_TIMEOUT: u64 = 60;
