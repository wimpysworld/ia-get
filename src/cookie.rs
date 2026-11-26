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
pub(crate) fn is_archive_org_host(url: &Url) -> bool {
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
    use crate::test_support::TempTestDir;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid test URL")
    }

    #[test]
    fn is_archive_org_host_accepts_apex_and_subdomains() {
        for valid in [
            "https://archive.org/download/x",
            "https://ARCHIVE.ORG/download/x",
            "https://ia800100.us.archive.org/1/items/x",
            "https://user:pass@archive.org/x",
        ] {
            assert!(is_archive_org_host(&url(valid)), "should accept {valid}");
        }
    }

    #[test]
    fn is_archive_org_host_rejects_lookalikes_ips_and_malformed_domains() {
        for invalid in [
            "https://notarchive.org/x",
            "https://archive.org.evil.com/x",
            "https://evil-archive.org/x",
            "https://.archive.org/x",
            "https://a..archive.org/x",
            "https://archive.org./x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://archive.org@evil.com/x",
        ] {
            assert!(
                !is_archive_org_host(&url(invalid)),
                "should reject {invalid}"
            );
        }
    }

    #[test]
    fn raw_cookie_scoping_enforces_https_and_archive_org_host() {
        let source = CookieSource::from_raw("  logged-in-user=yes; logged-in-sig=abc123  ")
            .expect("valid raw cookie");
        let expected = Some("logged-in-user=yes; logged-in-sig=abc123".to_string());

        let cases = [
            ("https://archive.org/download/item", true),
            ("https://ia800100.us.archive.org/1/items/item", true),
            ("https://user:pass@archive.org/download/item", true),
            // HTTP downgrade, lookalikes, IPs, userinfo spoofing, and non-HTTP schemes
            ("http://archive.org/download/item", false),
            ("http://ia800100.us.archive.org/1/items/item", false),
            ("https://notarchive.org/download/item", false),
            ("https://archive.org.evil.com/download/item", false),
            ("https://127.0.0.1/download/item", false),
            ("https://[::1]/download/item", false),
            ("https://archive.org@evil.com/download/item", false),
            ("ftp://archive.org/download/item", false),
        ];

        for (target, allowed) in cases {
            assert_eq!(
                source.header_for_url_at(&url(target), 1_000),
                if allowed { expected.clone() } else { None },
                "target: {target}"
            );
        }
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

        for invalid in ["a=b\r\nX-Injected: yes", "a=\x07"] {
            assert!(
                matches!(CookieSource::from_raw(invalid), Err(IaGetError::Network(_))),
                "expected Network error for {invalid:?}"
            );
        }
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

        // Comments, blanks, malformed expiry, empty domain/name, and too few fields are skipped
        for skipped in [
            "",
            "\r\n",
            "# Netscape HTTP Cookie File",
            "   # indented comment",
            "#HttpOnly_",
            "archive.org\tFALSE\t/\tFALSE\tnotanint\tname\tval",
            "archive.org\tFALSE\t/\tFALSE\t\tname\tval",
            "\tTRUE\t/\tFALSE\t0\tname\tval",
            ".\tTRUE\t/\tFALSE\t0\tname\tval",
            "archive.org\tFALSE\t/\tFALSE\t0\t\tval",
            "archive.org\tFALSE\t/\tFALSE\t0\tonlysix",
        ] {
            assert!(
                parse_netscape_cookie(skipped).is_none(),
                "expected None for {skipped:?}"
            );
        }
    }

    #[test]
    fn netscape_domain_and_ip_matching() {
        let cases = [
            // Apex-only host cookie
            ("archive.org", "FALSE", "https://archive.org/", true),
            ("archive.org", "FALSE", "https://sub.archive.org/", false),
            // Domain cookie with subdomains
            (".archive.org", "TRUE", "https://archive.org/", true),
            (
                ".archive.org",
                "TRUE",
                "https://ia800100.us.archive.org/",
                true,
            ),
            (".archive.org", "TRUE", "https://notarchive.org/", false),
            (
                ".archive.org",
                "TRUE",
                "https://archive.org.evil.com/",
                false,
            ),
            (".archive.org", "TRUE", "https://.archive.org/", false),
            // IPv4 exact match only, no suffix matching even with include_subdomains=true
            ("127.0.0.1", "FALSE", "http://127.0.0.1/", true),
            ("127.0.0.1", "FALSE", "http://127.0.0.2/", false),
            ("0.0.1", "TRUE", "http://127.0.0.1/", false),
            // IPv6 exact match with or without brackets and canonical comparison
            ("::1", "FALSE", "http://[::1]/", true),
            ("[::1]", "FALSE", "http://[::1]/", true),
            ("[::1]", "FALSE", "http://[::2]/", false),
            ("[2001:0DB8::0001]", "TRUE", "https://[2001:db8::1]/", true),
        ];

        for (domain, include_sub, target, expected) in cases {
            let line = format!("{domain}\t{include_sub}\t/\tFALSE\t0\ta\t1");
            let cookie = parse_netscape_cookie(&line).unwrap();
            assert_eq!(
                cookie_domain_matches(&cookie, &url(target)),
                expected,
                "domain={domain:?}, include_sub={include_sub}, target={target}"
            );
        }
    }

    #[test]
    fn netscape_path_matching_follows_rfc6265_boundaries() {
        let cases = [
            ("", "https://archive.org/download/file", true),
            ("/download", "https://archive.org/download", true),
            ("/download", "https://archive.org/download/", true),
            (
                "/download",
                "https://archive.org/download/item/file.zip",
                true,
            ),
            ("/download", "https://archive.org/downloader/item", false),
            ("/download/", "https://archive.org/download", false),
            ("/download/", "https://archive.org/download/", true),
            ("/download/", "https://archive.org/download/file.zip", true),
        ];

        for (cookie_path, target, expected) in cases {
            let line = format!("archive.org\tFALSE\t{cookie_path}\tFALSE\t0\ta\t1");
            let cookie = parse_netscape_cookie(&line).unwrap();
            assert_eq!(
                cookie_path_matches(&cookie, &url(target)),
                expected,
                "cookie_path={cookie_path:?}, target={target}"
            );
        }
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

        let cases = [
            // Before expiry on HTTPS
            (
                "https://archive.org/download/x",
                999,
                Some("short_lived=ok; session=alive; secure_only=https_only"),
            ),
            // At exact expiry timestamp
            (
                "https://archive.org/download/x",
                1000,
                Some("session=alive; secure_only=https_only"),
            ),
            // Even at now = 0, negative expiry cookie is still expired; secure_only skipped on HTTP
            (
                "http://archive.org/download/x",
                0,
                Some("short_lived=ok; session=alive"),
            ),
            // Non-HTTP(S) schemes receive no cookies
            ("ftp://archive.org/download/x", 0, None),
        ];

        for (target, now, expected) in cases {
            assert_eq!(
                source.header_for_url_at(&url(target), now).as_deref(),
                expected,
                "target={target}, now={now}"
            );
        }
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
        let dir = TempTestDir::new("cookie-from-input");
        let cookie_file = dir.write(
            "cookies.txt",
            "# Netscape HTTP Cookie File\n.archive.org\tTRUE\t/\tTRUE\t0\tlogged-in-user\tyes\n",
        );

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
