//! HTTP cookie parsing and per-URL scoping for archive.org downloads.
//!
//! Enforces host, scheme, path, and expiry checks for every request URL,
//! including across HTTP redirects, so credentials are not leaked to
//! third-party hosts or downgraded over plain HTTP.

use crate::{IaGetError, Result};
use reqwest::header::HeaderValue;
use reqwest::Url;
use std::cmp::Reverse;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Host;

/// A single cookie entry parsed from a Netscape `cookies.txt` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetscapeCookie {
    /// Lowercased domain or IP address without a leading dot.
    pub domain: String,
    /// Whether subdomains of `domain` are also matched.
    pub include_subdomains: bool,
    /// Cookie path attribute (empty path defaults to `"/"` during matching).
    pub path: String,
    /// Whether the cookie may only be sent over HTTPS.
    pub secure: bool,
    /// Expiration timestamp in Unix seconds, or `None` for a session cookie.
    pub expires: Option<u64>,
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
}

/// Source of cookies supplied via CLI or API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieSource {
    /// Raw `Cookie` header value, scoped strictly to HTTPS `archive.org` hosts.
    Raw(String),
    /// Parsed Netscape `cookies.txt` entries, evaluated per URL.
    NetscapeFile(Vec<NetscapeCookie>),
}

impl CookieSource {
    /// Builds a [`CookieSource`] from a CLI input string.
    ///
    /// If `input` refers to an existing file, reads and parses it as a Netscape
    /// `cookies.txt` file. If `input` refers to a directory, returns a file
    /// system error. Otherwise treats `input` as a raw `Cookie` header value.
    pub fn from_input(input: &str) -> Result<Self> {
        let path = Path::new(input);
        if path.is_file() {
            let content = fs::read_to_string(path)?;
            Ok(Self::from_netscape_str(&content))
        } else if path.is_dir() {
            Err(IaGetError::FileSystem(format!(
                "Cookie path is a directory: {}",
                input
            )))
        } else {
            Self::from_raw(input)
        }
    }

    /// Builds a [`CookieSource::Raw`] from a raw cookie header string.
    ///
    /// Surrounding whitespace is trimmed. Non-empty values are validated as
    /// legal HTTP header values.
    pub fn from_raw(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            HeaderValue::from_str(trimmed)
                .map_err(|e| IaGetError::Network(format!("Invalid cookie header: {}", e)))?;
        }
        Ok(Self::Raw(trimmed.to_string()))
    }

    /// Parses Netscape `cookies.txt` content into a [`CookieSource::NetscapeFile`].
    pub fn from_netscape_str(content: &str) -> Self {
        let cookies = content.lines().filter_map(parse_netscape_cookie).collect();
        Self::NetscapeFile(cookies)
    }

    /// Computes the `Cookie` header string applicable to `url` at the current
    /// wall-clock time.
    pub fn header_for_url(&self, url: &Url) -> Result<Option<String>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| IaGetError::FileSystem(e.to_string()))?
            .as_secs();
        Ok(self.header_for_url_at(url, now))
    }

    /// Computes the `Cookie` header string applicable to `url` at an explicit
    /// Unix timestamp in seconds.
    pub fn header_for_url_at(&self, url: &Url, now_unix_secs: u64) -> Option<String> {
        match self {
            Self::Raw(raw) => {
                if !raw.is_empty() && url.scheme() == "https" && is_archive_org_host(url) {
                    Some(raw.clone())
                } else {
                    None
                }
            }
            Self::NetscapeFile(cookies) => {
                let mut matching: Vec<&NetscapeCookie> = cookies
                    .iter()
                    .filter(|cookie| cookie_applies_to_url(cookie, url, now_unix_secs))
                    .collect();

                if matching.is_empty() {
                    return None;
                }

                // RFC 6265 Section 5.4: longer paths first, preserving file order on ties.
                matching.sort_by_key(|cookie| Reverse(effective_cookie_path(cookie).len()));

                let header = matching
                    .into_iter()
                    .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                    .collect::<Vec<_>>()
                    .join("; ");

                Some(header)
            }
        }
    }

    /// Resolves and validates the `Cookie` [`HeaderValue`] applicable to `url`.
    pub fn header_value_for_url(&self, url: &Url) -> Result<Option<HeaderValue>> {
        let Some(header) = self.header_for_url(url)? else {
            return Ok(None);
        };

        let value = HeaderValue::from_str(&header)
            .map_err(|e| IaGetError::Network(format!("Invalid cookie header: {}", e)))?;
        Ok(Some(value))
    }
}

/// Returns `true` if `url` has a valid domain host equal to `archive.org` or
/// a subdomain of `archive.org`.
pub fn is_archive_org_host(url: &Url) -> bool {
    let Some(Host::Domain(domain)) = url.host() else {
        return false;
    };

    let domain = domain.to_ascii_lowercase();
    if domain.starts_with('.') || domain.contains("..") {
        return false;
    }

    domain == "archive.org" || domain.ends_with(".archive.org")
}

/// Returns the effective path of a cookie, defaulting an empty path to `"/"`.
fn effective_cookie_path(cookie: &NetscapeCookie) -> &str {
    if cookie.path.is_empty() {
        "/"
    } else {
        &cookie.path
    }
}

/// Parses a single line from a Netscape `cookies.txt` file.
fn parse_netscape_cookie(line: &str) -> Option<NetscapeCookie> {
    // Strip only trailing line endings so empty values after the 6th tab survive.
    let line = line.trim_end_matches(['\r', '\n']);
    let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);

    if line.is_empty() || line.starts_with('#') || line.trim_start().starts_with('#') {
        return None;
    }

    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 7 {
        return None;
    }

    let domain = fields[0].trim_start_matches('.').to_ascii_lowercase();
    let name = fields[5];
    if domain.is_empty() || name.is_empty() {
        return None;
    }

    let expires = match fields[4].parse::<i64>().ok()? {
        0 => None,
        value if value > 0 => Some(value as u64),
        _ => Some(0),
    };

    Some(NetscapeCookie {
        domain,
        include_subdomains: fields[1].eq_ignore_ascii_case("TRUE"),
        path: fields[2].to_string(),
        secure: fields[3].eq_ignore_ascii_case("TRUE"),
        expires,
        name: name.to_string(),
        value: fields[6].to_string(),
    })
}

/// Checks whether a cookie's domain matches the host of `url`.
fn cookie_domain_matches(cookie: &NetscapeCookie, url: &Url) -> bool {
    let cookie_domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
    if cookie_domain.is_empty() {
        return false;
    }

    match url.host() {
        Some(Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            host == cookie_domain
                || (cookie.include_subdomains
                    && host
                        .strip_suffix(&cookie_domain)
                        .and_then(|prefix| prefix.strip_suffix('.'))
                        .is_some_and(|label| !label.is_empty()))
        }
        Some(Host::Ipv4(ip)) => cookie_domain
            .parse::<Ipv4Addr>()
            .is_ok_and(|cookie_ip| cookie_ip == ip),
        Some(Host::Ipv6(ip)) => {
            let stripped = cookie_domain
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(&cookie_domain);
            stripped
                .parse::<Ipv6Addr>()
                .is_ok_and(|cookie_ip| cookie_ip == ip)
        }
        None => false,
    }
}

/// Checks whether a cookie's path matches the path of `url` per RFC 6265 Section 5.1.4.
fn cookie_path_matches(cookie: &NetscapeCookie, url: &Url) -> bool {
    let cookie_path = effective_cookie_path(cookie);
    let request_path = url.path();

    request_path == cookie_path
        || request_path
            .strip_prefix(cookie_path)
            .is_some_and(|remainder| cookie_path.ends_with('/') || remainder.starts_with('/'))
}

/// Checks whether a cookie applies to `url` at Unix timestamp `now`.
fn cookie_applies_to_url(cookie: &NetscapeCookie, url: &Url, now: u64) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }

    if cookie.secure && url.scheme() != "https" {
        return false;
    }

    if let Some(expires) = cookie.expires {
        if expires <= now {
            return false;
        }
    }

    cookie_domain_matches(cookie, url) && cookie_path_matches(cookie, url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "ia-get-cookie-test-{}-{}-{}",
                std::process::id(),
                label,
                id
            ));
            fs::create_dir_all(&dir).expect("create temp test dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid test URL")
    }

    #[test]
    fn is_archive_org_host_accepts_apex_and_subdomains() {
        assert!(is_archive_org_host(&url("https://archive.org/download/x")));
        assert!(is_archive_org_host(&url("https://ARCHIVE.ORG/download/x")));
        assert!(is_archive_org_host(&url(
            "https://ia800100.us.archive.org/1/items/x"
        )));
        assert!(is_archive_org_host(&url("https://user:pass@archive.org/x")));
    }

    #[test]
    fn is_archive_org_host_rejects_lookalikes_ips_and_malformed_domains() {
        assert!(!is_archive_org_host(&url("https://notarchive.org/x")));
        assert!(!is_archive_org_host(&url("https://archive.org.evil.com/x")));
        assert!(!is_archive_org_host(&url("https://evil-archive.org/x")));
        assert!(!is_archive_org_host(&url("https://.archive.org/x")));
        assert!(!is_archive_org_host(&url("https://a..archive.org/x")));
        assert!(!is_archive_org_host(&url("https://archive.org./x")));
        assert!(!is_archive_org_host(&url("https://127.0.0.1/x")));
        assert!(!is_archive_org_host(&url("https://[::1]/x")));
        assert!(!is_archive_org_host(&url("https://archive.org@evil.com/x")));
    }

    #[test]
    fn raw_cookie_scoping_enforces_https_and_archive_org_host() {
        let source = CookieSource::from_raw("  logged-in-user=yes; logged-in-sig=abc123  ")
            .expect("valid raw cookie");

        assert_eq!(
            source.header_for_url_at(&url("https://archive.org/download/item"), 1_000),
            Some("logged-in-user=yes; logged-in-sig=abc123".to_string())
        );
        assert_eq!(
            source.header_for_url_at(&url("https://ia800100.us.archive.org/1/items/item"), 1_000),
            Some("logged-in-user=yes; logged-in-sig=abc123".to_string())
        );
        assert_eq!(
            source.header_for_url_at(&url("https://user:pass@archive.org/download/item"), 1_000),
            Some("logged-in-user=yes; logged-in-sig=abc123".to_string())
        );

        // HTTP downgrade must be rejected
        assert_eq!(
            source.header_for_url_at(&url("http://archive.org/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("http://ia800100.us.archive.org/1/items/item"), 1_000),
            None
        );

        // Lookalikes, IPs, and userinfo spoofing must be rejected
        assert_eq!(
            source.header_for_url_at(&url("https://notarchive.org/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("https://archive.org.evil.com/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("https://127.0.0.1/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("https://[::1]/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("https://archive.org@evil.com/download/item"), 1_000),
            None
        );
        assert_eq!(
            source.header_for_url_at(&url("ftp://archive.org/download/item"), 1_000),
            None
        );
    }

    #[test]
    fn raw_cookie_handles_empty_and_invalid_header_values() {
        let empty = CookieSource::from_raw("   \t ").expect("empty raw cookie allowed");
        assert_eq!(
            empty.header_for_url_at(&url("https://archive.org/download/item"), 1_000),
            None
        );
        assert_eq!(
            empty
                .header_value_for_url(&url("https://archive.org/download/item"))
                .expect("header value check"),
            None
        );

        let err = CookieSource::from_raw("a=b\r\nX-Injected: yes").unwrap_err();
        assert!(matches!(err, IaGetError::Network(_)));

        let err_ctrl = CookieSource::from_raw("a=\x07").unwrap_err();
        assert!(matches!(err_ctrl, IaGetError::Network(_)));
    }

    #[test]
    fn parse_netscape_cookie_handles_edge_cases() {
        // Empty value after 6th tab must survive (including with CRLF)
        let empty_val =
            parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t0\tempty_flag\t\r\n").unwrap();
        assert_eq!(empty_val.name, "empty_flag");
        assert_eq!(empty_val.value, "");
        assert_eq!(empty_val.expires, None);

        // #HttpOnly_ prefix and domain normalisation
        let http_only = parse_netscape_cookie(
            "#HttpOnly_.Archive.ORG\tTRUE\t/download\tTRUE\t2000\tsession\tsecret",
        )
        .unwrap();
        assert_eq!(
            http_only,
            NetscapeCookie {
                domain: "archive.org".to_string(),
                include_subdomains: true,
                path: "/download".to_string(),
                secure: true,
                expires: Some(2000),
                name: "session".to_string(),
                value: "secret".to_string(),
            }
        );

        // Negative expiry becomes Some(0) (already expired)
        let negative =
            parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t-1\texpired\told").unwrap();
        assert_eq!(negative.expires, Some(0));

        // Comments, blanks, malformed expiry, empty domain, empty name, too few fields are skipped
        assert!(parse_netscape_cookie("").is_none());
        assert!(parse_netscape_cookie("\r\n").is_none());
        assert!(parse_netscape_cookie("# Netscape HTTP Cookie File").is_none());
        assert!(parse_netscape_cookie("   # indented comment").is_none());
        assert!(parse_netscape_cookie("#HttpOnly_").is_none());
        assert!(
            parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\tnotanint\tname\tval").is_none()
        );
        assert!(parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t\tname\tval").is_none());
        assert!(parse_netscape_cookie("\tTRUE\t/\tFALSE\t0\tname\tval").is_none());
        assert!(parse_netscape_cookie(".\tTRUE\t/\tFALSE\t0\tname\tval").is_none());
        assert!(parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t0\t\tval").is_none());
        assert!(parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t0\tonlysix").is_none());
    }

    #[test]
    fn netscape_domain_and_ip_matching() {
        let apex_only = parse_netscape_cookie("archive.org\tFALSE\t/\tFALSE\t0\ta\t1").unwrap();
        assert!(cookie_domain_matches(
            &apex_only,
            &url("https://archive.org/")
        ));
        assert!(!cookie_domain_matches(
            &apex_only,
            &url("https://sub.archive.org/")
        ));

        let with_subs = parse_netscape_cookie(".archive.org\tTRUE\t/\tFALSE\t0\ta\t1").unwrap();
        assert!(cookie_domain_matches(
            &with_subs,
            &url("https://archive.org/")
        ));
        assert!(cookie_domain_matches(
            &with_subs,
            &url("https://ia800100.us.archive.org/")
        ));
        assert!(!cookie_domain_matches(
            &with_subs,
            &url("https://notarchive.org/")
        ));
        assert!(!cookie_domain_matches(
            &with_subs,
            &url("https://archive.org.evil.com/")
        ));
        assert!(!cookie_domain_matches(
            &with_subs,
            &url("https://.archive.org/")
        ));

        // IPv4 exact match only, no suffix matching even with include_subdomains=true
        let ipv4 = parse_netscape_cookie("127.0.0.1\tFALSE\t/\tFALSE\t0\tip4\t1").unwrap();
        assert!(cookie_domain_matches(&ipv4, &url("http://127.0.0.1/")));
        assert!(!cookie_domain_matches(&ipv4, &url("http://127.0.0.2/")));

        let ipv4_suffix = parse_netscape_cookie("0.0.1\tTRUE\t/\tFALSE\t0\tbad\t1").unwrap();
        assert!(!cookie_domain_matches(
            &ipv4_suffix,
            &url("http://127.0.0.1/")
        ));

        // IPv6 exact match with or without brackets and canonical comparison
        let ipv6_bare = parse_netscape_cookie("::1\tFALSE\t/\tFALSE\t0\tip6\t1").unwrap();
        let ipv6_bracketed = parse_netscape_cookie("[::1]\tFALSE\t/\tFALSE\t0\tip6b\t1").unwrap();
        let ipv6_expanded =
            parse_netscape_cookie("[2001:0DB8::0001]\tTRUE\t/\tFALSE\t0\tip6e\t1").unwrap();
        assert!(cookie_domain_matches(&ipv6_bare, &url("http://[::1]/")));
        assert!(cookie_domain_matches(
            &ipv6_bracketed,
            &url("http://[::1]/")
        ));
        assert!(!cookie_domain_matches(
            &ipv6_bracketed,
            &url("http://[::2]/")
        ));
        assert!(cookie_domain_matches(
            &ipv6_expanded,
            &url("https://[2001:db8::1]/")
        ));
    }

    #[test]
    fn netscape_path_matching_follows_rfc6265_boundaries() {
        let empty_path = parse_netscape_cookie("archive.org\tFALSE\t\tFALSE\t0\ta\t1").unwrap();
        assert!(cookie_path_matches(
            &empty_path,
            &url("https://archive.org/download/file")
        ));

        let dir_no_slash =
            parse_netscape_cookie("archive.org\tFALSE\t/download\tFALSE\t0\ta\t1").unwrap();
        assert!(cookie_path_matches(
            &dir_no_slash,
            &url("https://archive.org/download")
        ));
        assert!(cookie_path_matches(
            &dir_no_slash,
            &url("https://archive.org/download/")
        ));
        assert!(cookie_path_matches(
            &dir_no_slash,
            &url("https://archive.org/download/item/file.zip")
        ));
        assert!(!cookie_path_matches(
            &dir_no_slash,
            &url("https://archive.org/downloader/item")
        ));

        let dir_with_slash =
            parse_netscape_cookie("archive.org\tFALSE\t/download/\tFALSE\t0\ta\t1").unwrap();
        assert!(!cookie_path_matches(
            &dir_with_slash,
            &url("https://archive.org/download")
        ));
        assert!(cookie_path_matches(
            &dir_with_slash,
            &url("https://archive.org/download/")
        ));
        assert!(cookie_path_matches(
            &dir_with_slash,
            &url("https://archive.org/download/file.zip")
        ));
    }

    #[test]
    fn netscape_orders_longest_path_first_and_preserves_file_order_on_ties() {
        let content = concat!(
            "archive.org\tFALSE\t/\tFALSE\t0\troot_first\t1\n",
            "archive.org\tFALSE\t\tFALSE\t0\tempty_path_second\t2\n",
            "archive.org\tFALSE\t/download\tFALSE\t0\tdl_first\t3\n",
            "archive.org\tFALSE\t/download/private\tFALSE\t0\tdeep\t4\n",
            "archive.org\tFALSE\t/download\tFALSE\t0\tdl_second\t5\n",
            "archive.org\tFALSE\t/\tFALSE\t0\troot_third\t\n",
        );

        let source = CookieSource::from_netscape_str(content);
        let header = source.header_for_url_at(&url("https://archive.org/download/private/x"), 500);

        assert_eq!(
            header,
            Some(
                "deep=4; dl_first=3; dl_second=5; root_first=1; empty_path_second=2; root_third="
                    .to_string()
            )
        );
    }

    #[test]
    fn netscape_expiry_and_scheme_transitions_are_deterministic() {
        let content = concat!(
            "archive.org\tFALSE\t/\tFALSE\t1000\tshort_lived\tok\n",
            "archive.org\tFALSE\t/\tFALSE\t0\tsession\talive\n",
            "archive.org\tFALSE\t/\tFALSE\t-10\tnegative\tdead\n",
            "archive.org\tFALSE\t/\tTRUE\t0\tsecure_only\thttps_only\n",
        );
        let source = CookieSource::from_netscape_str(content);

        // Before expiry on HTTPS: short_lived, session, and secure_only apply
        assert_eq!(
            source.header_for_url_at(&url("https://archive.org/download/x"), 999),
            Some("short_lived=ok; session=alive; secure_only=https_only".to_string())
        );

        // At exact expiry timestamp: short_lived is expired
        assert_eq!(
            source.header_for_url_at(&url("https://archive.org/download/x"), 1000),
            Some("session=alive; secure_only=https_only".to_string())
        );

        // Even at now = 0, negative expiry cookie is still expired
        assert_eq!(
            source.header_for_url_at(&url("http://archive.org/download/x"), 0),
            Some("short_lived=ok; session=alive".to_string())
        );

        // Non-HTTP(S) schemes receive no cookies
        assert_eq!(
            source.header_for_url_at(&url("ftp://archive.org/download/x"), 0),
            None
        );
    }

    #[test]
    fn header_value_for_url_validates_resolved_netscape_header() {
        let valid =
            CookieSource::from_netscape_str("archive.org\tFALSE\t/\tTRUE\t0\tsig\tabc123\n");
        let hv = valid
            .header_value_for_url(&url("https://archive.org/download/x"))
            .expect("valid header")
            .expect("cookie present");
        assert_eq!(hv, HeaderValue::from_static("sig=abc123"));

        let invalid =
            CookieSource::from_netscape_str("archive.org\tFALSE\t/\tTRUE\t0\tbad\t\x07\n");
        let err = invalid
            .header_value_for_url(&url("https://archive.org/download/x"))
            .unwrap_err();
        assert!(matches!(err, IaGetError::Network(_)));
    }

    #[test]
    fn from_input_disambiguates_file_directory_and_raw_cookie() {
        let dir = TempDirGuard::new("from-input");
        let cookie_file = dir.path().join("cookies.txt");
        fs::write(
            &cookie_file,
            "# Netscape HTTP Cookie File\n.archive.org\tTRUE\t/\tTRUE\t0\tlogged-in-user\tyes\n",
        )
        .expect("write cookie file");

        // File -> NetscapeFile
        let from_file =
            CookieSource::from_input(cookie_file.to_str().unwrap()).expect("load cookie file");
        assert!(matches!(from_file, CookieSource::NetscapeFile(ref c) if c.len() == 1));
        assert_eq!(
            from_file.header_for_url_at(&url("https://archive.org/download/x"), 10),
            Some("logged-in-user=yes".to_string())
        );

        // Directory -> FileSystem error
        let dir_err = CookieSource::from_input(dir.path().to_str().unwrap()).unwrap_err();
        assert!(matches!(dir_err, IaGetError::FileSystem(_)));

        // Raw cookie string -> Raw
        let from_raw =
            CookieSource::from_input("logged-in-user=yes; logged-in-sig=abc").expect("raw cookie");
        assert_eq!(
            from_raw,
            CookieSource::Raw("logged-in-user=yes; logged-in-sig=abc".to_string())
        );
    }
}
