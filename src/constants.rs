//! Application constants for ia-get
//! 
//! Note: XML files from Internet Archive often have mismatched MD5 hashes due to dynamic
//! generation or updates after the hash was calculated. The downloader uses alternative
//! validation for .xml files, checking file size and XML structure instead of hash validation.

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

/// Regex pattern for validating archive.org details URLs
pub const URL_PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+/?$";

/// Maximum length for XML content in debug output (characters)
pub const XML_DEBUG_TRUNCATE_LEN: usize = 1000;
