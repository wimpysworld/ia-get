//! Building the `Cookie` header from a raw string or a Netscape cookies.txt
//! file, and applying it to requests, so every authenticated request carries
//! the right cookies.

use crate::error::io_error_with_path;
use crate::{IaGetError, Result};
use colored::*;
use reqwest::RequestBuilder;
use reqwest::header::{COOKIE, HeaderValue};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// Guard against a maliciously huge file being buffered in memory.
const MAX_COOKIE_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// The only host this tool ever talks to: URL validation restricts every
/// request to archive.org.
const COOKIE_HOST: &str = "archive.org";

/// One entry of a Netscape cookies.txt file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetscapeCookie {
    pub(crate) domain: String,
    pub(crate) include_subdomains: bool,
    pub(crate) path: String,
    pub(crate) secure: bool,
    pub(crate) expires: Option<u64>,
    pub(crate) name: String,
    pub(crate) value: String,
}

/// Reads a cookie file, naming it in I/O errors and refusing to buffer a
/// file larger than `MAX_COOKIE_FILE_BYTES` in memory.
fn read_cookie_file(input: &str) -> Result<String> {
    let meta = fs::metadata(input).map_err(|e| io_error_with_path(input, e))?;
    if meta.len() > MAX_COOKIE_FILE_BYTES {
        return Err(IaGetError::FileSystem {
            detail: format!("{input}: cookie file exceeds {MAX_COOKIE_FILE_BYTES} bytes"),
            source: None,
        });
    }
    fs::read_to_string(input).map_err(|e| io_error_with_path(input, e))
}

/// The session's cookie source: a raw `--cookies` string applies to every
/// request as is, while a parsed cookies.txt is scoped against each request
/// URL (RFC 6265 domain/path/expiry/secure) — a cookie scoped to one file
/// path must not ride along on unrelated requests, and vice versa.
#[derive(Debug, Clone)]
pub enum CookieSource {
    /// A raw cookie header string: one prebuilt header for all requests
    Raw(HeaderValue),
    /// Parsed cookies.txt entries (unencodable ones already dropped),
    /// matched against each request URL
    Netscape(Vec<NetscapeCookie>),
}

/// Builds an HTTP Cookie header value from a raw cookie string or cookies.txt
/// path for a single `url` — the one-shot form. A session that requests many
/// URLs should use [`cookie_source`] + [`cookie_header_for`] instead, so each
/// URL sees exactly the cookies scoped to it.
pub fn cookie_header_from_input(input: &str, url: &Url) -> Result<String> {
    let source = cookie_source(Some(input), url)?;
    let header = cookie_header_for(source.as_ref(), url)?;
    Ok(header
        .map(|header| header.to_str().unwrap_or_default().to_string())
        .unwrap_or_default())
}

/// Resolves the `--cookies` CLI input (a raw string or a cookies.txt path)
/// into a session cookie source, or `None` when no cookies were given.
///
/// The input is only treated as a file when it names an existing file that
/// either holds at least one recognizable Netscape cookie line or does not
/// look like a raw cookie pair (no `=`): a raw cookie string that merely
/// collides with a filename in the working directory is kept as a cookie
/// string instead of being silently swallowed as an empty cookies.txt.
///
/// A file whose cookies could apply to no request of this session (all
/// expired, or scoped to another host — the requests use the host of `url`
/// exactly, so a subdomain-scoped cookie like `www.archive.org` counts as
/// inapplicable) yields a warning so an unauthenticated-looking 401/403 has
/// an obvious cause.
pub fn cookie_source(cookie_input: Option<&str>, url: &Url) -> Result<Option<CookieSource>> {
    let Some(cookie_input) = cookie_input else {
        return Ok(None);
    };
    if Path::new(cookie_input).is_file() {
        let cookie_file = read_cookie_file(cookie_input)?;
        let cookies = parse_netscape_cookies(&cookie_file);
        if !cookies.is_empty() || !cookie_input.contains('=') {
            let cookies = filter_encodable(cookies);
            let now = now_secs()?;
            if !cookies
                .iter()
                .any(|c| c.expires.is_none_or(|e| e > now) && cookie_domain_matches(c, url))
            {
                println!(
                    "{} {} {}",
                    "⚠".yellow().bold(),
                    "No applicable cookies found in".yellow(),
                    cookie_input.dimmed()
                );
            }
            return Ok(Some(CookieSource::Netscape(cookies)));
        }
    }
    let header = HeaderValue::from_str(cookie_input.trim())
        .map_err(|e| IaGetError::InvalidCookie(e.to_string()))?;
    Ok(Some(CookieSource::Raw(header)))
}

/// The `Cookie` header for one request URL, from the session source: the
/// raw string applies as is, the cookies.txt entries are scoped against
/// `url` (an empty result is `None` — the request goes without a Cookie
/// header rather than carrying a cookie that does not belong to the path).
pub fn cookie_header_for(source: Option<&CookieSource>, url: &Url) -> Result<Option<HeaderValue>> {
    let Some(source) = source else {
        return Ok(None);
    };
    match source {
        CookieSource::Raw(header) => Ok(Some(header.clone())),
        CookieSource::Netscape(cookies) => {
            let header = cookie_header_from_cookies(cookies, url)?;
            if header.is_empty() {
                return Ok(None);
            }
            let value = HeaderValue::from_str(&header)
                .map_err(|e| IaGetError::InvalidCookie(e.to_string()))?;
            Ok(Some(value))
        }
    }
}

/// Parses Netscape cookies.txt content into the cookies it defines,
/// skipping blank lines, comments and malformed entries.
fn parse_netscape_cookies(content: &str) -> Vec<NetscapeCookie> {
    content.lines().filter_map(parse_netscape_cookie).collect()
}

fn parse_netscape_cookie(line: &str) -> Option<NetscapeCookie> {
    let line = line.trim();
    let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);

    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() < 7 {
        return None;
    }

    let expires = match fields[4].parse::<u64>().unwrap_or(0) {
        0 => None,
        value => Some(value),
    };

    Some(NetscapeCookie {
        domain: fields[0].trim_start_matches('.').to_ascii_lowercase(),
        include_subdomains: fields[1].eq_ignore_ascii_case("TRUE"),
        path: fields[2].to_string(),
        secure: fields[3].eq_ignore_ascii_case("TRUE"),
        expires,
        name: fields[5].to_string(),
        value: fields[6].to_string(),
    })
}

/// True when `domain` (already lower-cased, see `parse_netscape_cookie`)
/// is archive.org itself or one of its subdomains.
fn is_archive_org_domain(domain: &str) -> bool {
    domain == COOKIE_HOST || domain.ends_with(&format!(".{COOKIE_HOST}"))
}

fn cookie_domain_matches(cookie: &NetscapeCookie, url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    // A cookie scoped to any other domain can never apply: every request
    // goes to archive.org, and a bare RFC 6265 domain match would wrongly
    // treat a public-suffix cookie (e.g. ".com" with TRUE) as applicable
    // without a public-suffix list.
    if !is_archive_org_domain(&cookie.domain) {
        return false;
    }

    let host = host.to_ascii_lowercase();
    host == cookie.domain
        || (cookie.include_subdomains && host.ends_with(&format!(".{}", cookie.domain)))
}

fn cookie_path_matches(cookie: &NetscapeCookie, url: &Url) -> bool {
    let cookie_path = if cookie.path.is_empty() {
        "/"
    } else {
        &cookie.path
    };
    let request_path = url.path();

    request_path == cookie_path
        || request_path
            .strip_prefix(cookie_path)
            .is_some_and(|remainder| cookie_path.ends_with('/') || remainder.starts_with('/'))
}

fn cookie_applies_to_url(cookie: &NetscapeCookie, url: &Url, now: u64) -> bool {
    if cookie.expires.is_some_and(|expires| expires <= now) {
        return false;
    }

    if cookie.secure && url.scheme() != "https" {
        return false;
    }

    cookie_domain_matches(cookie, url) && cookie_path_matches(cookie, url)
}

/// The current unix seconds (cookie expiry checks); a clock before the
/// epoch is a hard error, like the old inline version was.
fn now_secs() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(IaGetError::SystemTime)
        .map(|now| now.as_secs())
}

/// Keeps the cookies that can become an HTTP header value; an unencodable
/// one (a control character or DEL, e.g. from a corrupted browser export)
/// is dropped with a warning instead of failing the whole run — the
/// remaining cookies still authenticate the request.
fn filter_encodable(cookies: Vec<NetscapeCookie>) -> Vec<NetscapeCookie> {
    cookies
        .into_iter()
        .filter(|cookie| {
            let pair = format!("{}={}", cookie.name, cookie.value);
            match HeaderValue::from_str(&pair) {
                Ok(_) => true,
                Err(_) => {
                    println!(
                        "{} {} {}: the cookie value cannot be encoded as an HTTP header",
                        "⚠".yellow().bold(),
                        "Skipped cookie".yellow(),
                        cookie.name.dimmed()
                    );
                    false
                }
            }
        })
        .collect()
}

/// Builds the HTTP Cookie header value from the parsed cookies that apply
/// to `url` (domain, path, expiry, `secure` scheme). The callers pass
/// encodable cookies only (see `filter_encodable`).
///
/// When several applicable cookies share a name, servers commonly read the
/// first one in the header: the list is ordered longest path first (RFC
/// 6265 §5.4), so the most specific value leads. Ties keep the file order
/// (stable sort).
fn cookie_header_from_cookies(cookies: &[NetscapeCookie], url: &Url) -> Result<String> {
    let now = now_secs()?;
    let mut applicable: Vec<&NetscapeCookie> = cookies
        .iter()
        .filter(|cookie| cookie_applies_to_url(cookie, url, now))
        .collect();
    applicable.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));

    Ok(applicable
        .iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; "))
}

/// Parses Netscape cookies.txt content into an HTTP Cookie header value.
pub fn cookie_header_from_netscape_file(content: &str, url: &Url) -> Result<String> {
    let cookies = filter_encodable(parse_netscape_cookies(content));
    cookie_header_from_cookies(&cookies, url)
}

/// Resolves the `--cookies` CLI input (raw string or cookies.txt path) into
/// a `Cookie` header value for requests to `url`, or `None` when no cookies
/// apply — the one-shot form of [`cookie_source`] + [`cookie_header_for`].
pub fn cookie_header_value(cookie_input: Option<&str>, url: &Url) -> Result<Option<HeaderValue>> {
    cookie_header_for(cookie_source(cookie_input, url)?.as_ref(), url)
}

/// Adds the `Cookie` header to a request builder when a cookie value is
/// present, so every authenticated request is built the same way.
pub fn with_cookie(mut request: RequestBuilder, cookie: Option<&HeaderValue>) -> RequestBuilder {
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn cookie_test_url(path: &str) -> Url {
        Url::parse(&format!("https://archive.org{path}")).unwrap()
    }

    #[test]
    fn cookie_header_accepts_raw_cookie_string() {
        assert_eq!(
            cookie_header_from_input(
                "logged-in-user=yes; logged-in-sig=abc123",
                &cookie_test_url("/download/item/item_files.xml"),
            )
            .unwrap(),
            "logged-in-user=yes; logged-in-sig=abc123"
        );
    }

    #[test]
    fn cookie_header_parses_netscape_cookie_file_content() {
        let cookies = "# Netscape HTTP Cookie File\n\
.archive.org\tTRUE\t/\tFALSE\t2145916800\tlogged-in-user\tyes\n\
archive.org\tFALSE\t/\tTRUE\t2145916800\tlogged-in-sig\tabc123\n";

        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/item/item_files.xml")
            )
            .unwrap(),
            "logged-in-user=yes; logged-in-sig=abc123"
        );
    }

    #[test]
    fn cookie_header_respects_domain_and_path_scoping() {
        let cookies = "# Netscape HTTP Cookie File\n\
.archive.org\tTRUE\t/download\tFALSE\t2145916800\tdownload-root\tyes\n\
archive.org\tFALSE\t/account\tFALSE\t2145916800\taccount-only\tnope\n\
example.com\tFALSE\t/download\tFALSE\t2145916800\twrong-domain\tnope\n\
archive.org\tFALSE\t/download/private\tFALSE\t2145916800\tprivate-only\tsecret\n";

        assert_eq!(
            cookie_header_from_netscape_file(cookies, &cookie_test_url("/download/item/file.zip"))
                .unwrap(),
            "download-root=yes"
        );

        // Two applicable cookies: the most specific path leads
        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/private/file.zip")
            )
            .unwrap(),
            "private-only=secret; download-root=yes"
        );
    }

    #[test]
    fn cookie_header_orders_same_name_by_path_specificity() {
        // A server that reads only the first value of a repeated name must
        // see the file-scoped cookie, not the route-wide one: longest path
        // first (RFC 6265 §5.4), ties keeping the file order.
        let cookies = "# Netscape HTTP Cookie File\n\
archive.org\tFALSE\t/download/item1\tFALSE\t2145916800\tsession\tfile-scope\n\
archive.org\tFALSE\t/\tFALSE\t2145916800\tsession\troute-scope\n";

        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/item1/item1_files.xml")
            )
            .unwrap(),
            "session=file-scope; session=route-scope"
        );
    }

    #[test]
    fn cookie_header_never_matches_foreign_or_wildcard_domains() {
        // A public-suffix cookie (".com" with TRUE) must not match
        // archive.org, a lookalike host must not either, and a
        // subdomain-scoped cookie must not apply to the parent host.
        let cookies = "# Netscape HTTP Cookie File\n\
.com\tTRUE\t/\tFALSE\t2145916800\twildcard\tvalue\n\
evilarchive.org\tFALSE\t/\tFALSE\t2145916800\tlookalike\tvalue\n\
www.archive.org\tTRUE\t/\tFALSE\t2145916800\tsub-scoped\tvalue\n";

        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/item/item_files.xml"),
            )
            .unwrap(),
            "",
            "none of these cookies may reach the request"
        );
    }

    #[test]
    fn cookie_header_skips_non_encodable_cookie_and_keeps_others() {
        // A control character cannot become an HTTP header value; the
        // cookie must be dropped with a warning, not fail the whole run.
        let cookies = "archive.org\tFALSE\t/\tFALSE\t2145916800\tsession\tabc123\n\
archive.org\tFALSE\t/\tFALSE\t2145916800\tbinary\tbad\x01value\n";

        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/item/item_files.xml")
            )
            .unwrap(),
            "session=abc123",
            "the encodable cookie must survive"
        );
    }

    #[test]
    fn cookie_header_ignores_expired_netscape_cookies() {
        let cookies = "archive.org\tFALSE\t/\tFALSE\t1\told\tvalue\n\
archive.org\tFALSE\t/\tFALSE\t2145916800\tcurrent\tvalue\n";

        assert_eq!(
            cookie_header_from_netscape_file(
                cookies,
                &cookie_test_url("/download/item/item_files.xml")
            )
            .unwrap(),
            "current=value"
        );
    }

    fn cookie_input_file(dir: &TempDir, name: &str, content: &str) -> String {
        let path = dir.join(name);
        fs::write(&path, content).expect("failed to write temp cookie file");
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn cookie_header_file_with_cookie_lines_is_parsed() {
        // A real cookies.txt (even when the path itself contains '=') is
        // still parsed, not treated as a raw cookie string.
        let dir = TempDir::new("cookie_lines");
        let input = cookie_input_file(
            &dir,
            "ia-get-cookie-review=1",
            "archive.org\tFALSE\t/\tFALSE\t2145916800\tsession\tvalue\n",
        );

        let header = cookie_header_from_input(&input, &cookie_test_url("/download/item/f.xml"))
            .expect("cookies.txt must parse");
        assert_eq!(header, "session=value");
    }

    #[test]
    fn cookie_header_file_with_only_expired_cookies_is_empty() {
        // A recognizable-but-expired cookie file must yield an empty header
        // (with a warning), not the file name or the stale cookie.
        let dir = TempDir::new("cookie_expired_only");
        let input = cookie_input_file(
            &dir,
            "ia-get-cookie-expired-only",
            "archive.org\tFALSE\t/\tFALSE\t1\told\tvalue\n",
        );

        let header = cookie_header_from_input(&input, &cookie_test_url("/download/item/f.xml"))
            .expect("an expired-cookies file must still parse");
        assert_eq!(header, "", "expired cookies must not reach the header");
    }

    #[test]
    fn cookie_header_colliding_filename_is_kept_as_raw_string() {
        // A file that holds no recognizable cookie line must not swallow a
        // cookie-looking input: the raw cookie string wins.
        let dir = TempDir::new("cookie_collide");
        let input = cookie_input_file(&dir, "ia-get-cookie-collide=1", "not a cookie file\n");

        let header = cookie_header_from_input(&input, &cookie_test_url("/download/item/f.xml"))
            .expect("input must be kept as a cookie string");
        assert_eq!(
            header,
            input.trim(),
            "the cookie-looking input must survive"
        );
    }

    #[test]
    fn cookie_source_keeps_subdomain_scoped_cookies_but_never_sends_them() {
        // A cookie scoped to www.archive.org can apply to no request of this
        // session (every request uses the archive.org host): the warning
        // must not be suppressed by subdomain acceptance, and the cookie
        // must never reach a header
        let dir = TempDir::new("cookie_source_www");
        let input = cookie_input_file(
            &dir,
            "cookies.txt",
            "# Netscape HTTP Cookie File\n\
www.archive.org\tTRUE\t/\tFALSE\t2145916800\tsession\tvalue\n",
        );
        let source = cookie_source(
            Some(&input),
            &cookie_test_url("/download/item1/item1_files.xml"),
        )
        .expect("the entry must still be retained")
        .expect("a source was given");
        assert_eq!(
            cookie_header_for(Some(&source), &cookie_test_url("/download/item1/file.bin")).unwrap(),
            None,
            "a www-scoped cookie must never reach an archive.org request"
        );
    }

    #[test]
    fn cookie_source_scopes_each_request_url_its_own_way() {
        // A file with a route cookie and an item-path cookie: each request
        // URL is matched on its own, so the file under item1 sees both
        // (most specific first) and the unrelated item2 sees only the route
        // one.
        let dir = TempDir::new("cookie_source_scope");
        let input = cookie_input_file(
            &dir,
            "cookies.txt",
            "# Netscape HTTP Cookie File\n\
archive.org\tFALSE\t/\tFALSE\t2145916800\tlogin\troot\n\
archive.org\tFALSE\t/download/item1\tFALSE\t2145916800\tlogin\titem-scope\n",
        );
        let source = cookie_source(
            Some(&input),
            &cookie_test_url("/download/item1/item1_files.xml"),
        )
        .expect("the file must parse into a source")
        .expect("cookies were given");

        let xml_url = cookie_test_url("/download/item1/item1_files.xml");
        let other_item_url = cookie_test_url("/download/item2/other.bin");
        assert_eq!(
            cookie_header_for(Some(&source), &xml_url)
                .unwrap()
                .unwrap()
                .to_str()
                .unwrap(),
            "login=item-scope; login=root"
        );
        assert_eq!(
            cookie_header_for(Some(&source), &other_item_url)
                .unwrap()
                .unwrap()
                .to_str()
                .unwrap(),
            "login=root",
            "the item-scoped cookie must not ride along on item2"
        );
    }
}
