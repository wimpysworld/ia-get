//! Manual HTTP redirect resolution, downgrade checks, and per-hop cookie scoping.

use std::collections::HashSet;

use reqwest::{RequestBuilder, Response, StatusCode, Url};

use super::proxy::redact_proxy_value;
use super::retry::{await_interruptible, ShutdownSignal};
use crate::cookie::CookieSource;
use crate::error::IaGetError;
use crate::utils::{sanitize_url_for_log, VerboseLogger};
use crate::Result;

fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn resolve_redirect_target(current_url: &Url, location: &str) -> Result<Url> {
    let safe_current = sanitize_url_for_log(current_url);
    let location = location.trim();
    if location.is_empty() {
        return Err(IaGetError::Network(format!(
            "Redirect from {} has empty Location header",
            safe_current
        )));
    }

    let next_url = current_url.join(location).map_err(|err| {
        IaGetError::Network(format!(
            "Invalid redirect Location '{}' from {}: {}",
            redact_proxy_value(location),
            safe_current,
            err
        ))
    })?;
    let safe_next = sanitize_url_for_log(&next_url);

    if !matches!(next_url.scheme(), "http" | "https") {
        return Err(IaGetError::Network(format!(
            "Unsupported redirect scheme '{}' in Location '{}' from {}",
            next_url.scheme(),
            safe_next,
            safe_current
        )));
    }

    if current_url.scheme() == "https" && next_url.scheme() != "https" {
        return Err(IaGetError::Network(format!(
            "Refusing HTTPS-to-{} redirect downgrade from {} to {}",
            next_url.scheme(),
            safe_current,
            safe_next
        )));
    }

    Ok(next_url)
}

pub(super) enum RedirectSendOutcome {
    Response(Response),
    TransportError(reqwest::Error),
}

/// Extracts the full mirror hostname from a download or redirect URL.
///
/// Preserves regional subdomains (for example `ia801909.us.archive.org`,
/// `dn790007.ca.archive.org`, `dn760105.eu.archive.org`) so the live UI can
/// resolve the 2-letter mirror region badge.
pub(crate) fn mirror_host(url: &Url) -> Option<String> {
    url.host_str().map(str::to_owned)
}

pub(super) async fn send_following_redirects<F>(
    initial_url: &Url,
    signal: &ShutdownSignal,
    max_redirects: usize,
    logger: &VerboseLogger,
    build_request: F,
) -> Result<RedirectSendOutcome>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
{
    send_following_redirects_with_hook(
        initial_url,
        signal,
        max_redirects,
        logger,
        build_request,
        |_| {},
    )
    .await
}

pub(super) async fn send_following_redirects_with_hook<F, H>(
    initial_url: &Url,
    signal: &ShutdownSignal,
    max_redirects: usize,
    logger: &VerboseLogger,
    mut build_request: F,
    mut on_redirect: H,
) -> Result<RedirectSendOutcome>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
    H: FnMut(&Url),
{
    let mut current_url = initial_url.clone();
    let mut redirects_followed: usize = 0;
    let mut visited = HashSet::new();
    let mut normalised_initial = current_url.clone();
    normalised_initial.set_fragment(None);
    visited.insert(normalised_initial);

    loop {
        let (client, request_result) = build_request(&current_url)?.build_split();
        let request = request_result.map_err(reqwest::Error::without_url)?;
        let method = request.method().clone();
        let cookies_label = if request.headers().contains_key(reqwest::header::COOKIE) {
            "yes"
        } else {
            "no"
        };
        let range = request
            .headers()
            .get(reqwest::header::RANGE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let safe_url = sanitize_url_for_log(request.url());

        if let Some(ref range_value) = range {
            logger.log(
                "http",
                format_args!(
                    "-> {method} {safe_url} (cookies={cookies_label}, range={range_value})"
                ),
            );
        } else {
            logger.log(
                "http",
                format_args!("-> {method} {safe_url} (cookies={cookies_label})"),
            );
        }

        let response = match await_interruptible(client.execute(request), signal).await? {
            Ok(response) => {
                logger.log(
                    "http",
                    format_args!(
                        "<- {} {method} {safe_url} (cookies={cookies_label})",
                        response.status()
                    ),
                );
                response
            }
            Err(err) => {
                let err = err.without_url();
                logger.log(
                    "http",
                    format_args!("<- error {method} {safe_url} (cookies={cookies_label}): {err}"),
                );
                return Ok(RedirectSendOutcome::TransportError(err));
            }
        };

        let status = response.status();
        if !is_redirect_status(status) {
            return Ok(RedirectSendOutcome::Response(response));
        }

        if redirects_followed >= max_redirects {
            logger.log(
                "redirect",
                format_args!("limit exceeded ({max_redirects}) while requesting {safe_url}"),
            );
            return Err(IaGetError::Network(format!(
                "Too many redirects (maximum {} exceeded) while requesting {}",
                max_redirects, safe_url
            )));
        }

        let location_header = match response.headers().get(reqwest::header::LOCATION) {
            Some(header) => header,
            None => {
                logger.log(
                    "redirect",
                    format_args!("{status} {safe_url} missing Location header"),
                );
                return Err(IaGetError::Network(format!(
                    "HTTP {} redirect from {} missing Location header",
                    status, safe_url
                )));
            }
        };

        let location = match location_header.to_str() {
            Ok(location) => location,
            Err(_) => {
                logger.log(
                    "redirect",
                    format_args!("{status} {safe_url} has non-UTF-8 Location header"),
                );
                return Err(IaGetError::Network(format!(
                    "HTTP {} redirect from {} has non-UTF-8 Location header",
                    status, safe_url
                )));
            }
        };

        let next_url = match resolve_redirect_target(&current_url, location) {
            Ok(next_url) => next_url,
            Err(err) => {
                logger.log("redirect", format_args!("error: {err}"));
                return Err(err);
            }
        };
        let next_safe = sanitize_url_for_log(&next_url);

        let mut normalised_next = next_url.clone();
        normalised_next.set_fragment(None);
        if !visited.insert(normalised_next) {
            logger.log(
                "redirect",
                format_args!("loop detected {safe_url} -> {next_safe}"),
            );
            return Err(IaGetError::Network(format!(
                "Redirect loop detected while following redirect to {}",
                next_safe
            )));
        }

        let hop = redirects_followed + 1;
        logger.log(
            "redirect",
            format_args!("{status} {safe_url} -> {next_safe} (hop {hop}/{max_redirects})"),
        );
        on_redirect(&next_url);

        drop(response);
        redirects_followed = hop;
        current_url = next_url;
    }
}

/// Attaches a scoped `Cookie` header to `request` when `cookie_source` yields one for `target_url`.
pub fn attach_cookie_header(
    request: RequestBuilder,
    cookie_source: Option<&CookieSource>,
    target_url: &Url,
) -> Result<RequestBuilder> {
    if let Some(cookie_value) = cookie_source
        .map(|source| source.header_value_for_url(target_url))
        .transpose()?
        .flatten()
    {
        Ok(request.header(reqwest::header::COOKIE, cookie_value))
    } else {
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::constants::MAX_REDIRECTS;
    use crate::downloader::tests::DownloadFixture;
    use crate::test_support::{md5_hex, slow_retry_policy, MockResponse, MockServer};

    #[tokio::test]
    async fn test_download_cross_host_redirect_strips_cookie_and_verifies_md5() {
        let payload = b"cross-host redirected archive payload with verified md5";
        let expected_md5 = md5_hex(payload);

        let target_server = MockServer::start_on_localhost(vec![MockResponse::ok(payload)]).await;
        let origin_server = MockServer::start(vec![MockResponse::redirect(
            "302 Found",
            target_server.url("/final/item.bin"),
        )])
        .await;

        let cookie_source =
            CookieSource::from_netscape_str("127.0.0.1\tFALSE\t/\tFALSE\t0\tsession\tsecret123\n");
        let fx = DownloadFixture::new("redirect-strip-cookie");

        fx.download(
            &origin_server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
            Some(&cookie_source),
        )
        .await
        .expect("cross-host redirect download should succeed and pass MD5 check");

        let origin_requests = origin_server.finish().await;
        let target_requests = target_server.finish().await;
        assert_eq!(origin_requests.len(), 1);
        assert!(origin_requests[0]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert_eq!(
            origin_requests[0].header("cookie"),
            Some("session=secret123")
        );

        assert_eq!(target_requests.len(), 1);
        assert!(target_requests[0]
            .request_line
            .starts_with("GET /final/item.bin "));
        assert_eq!(
            target_requests[0].header("cookie"),
            None,
            "redirect to non-matching host must strip Cookie header"
        );
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_same_host_redirect_rescopes_and_orders_path_cookies() {
        let payload = b"same-host path-scoped redirect payload";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/download/public/item.bin"),
            MockResponse::ok(payload),
        ])
        .await;

        // Place shorter path before longer path in file order to verify RFC 6265 longest-path-first sorting.
        let cookie_source = CookieSource::from_netscape_str(concat!(
            "127.0.0.1\tFALSE\t/\tFALSE\t0\troot\t1\n",
            "127.0.0.1\tFALSE\t/download/private\tFALSE\t0\tprivate\t1\n",
        ));

        let fx = DownloadFixture::new("redirect-path-scope");
        fx.download(
            &server.url("/download/private/item.bin"),
            "item.bin",
            Some(&expected_md5),
            Some(&cookie_source),
        )
        .await
        .expect("same-host path-scoped redirect download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .request_line
            .starts_with("GET /download/private/item.bin "));
        assert_eq!(requests[0].header("cookie"), Some("private=1; root=1"));
        assert!(requests[1]
            .request_line
            .starts_with("GET /download/public/item.bin "));
        assert_eq!(requests[1].header("cookie"), Some("root=1"));
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_resume_preserves_range_across_all_redirect_statuses() {
        let prefix = b"partial-prefix-";
        let suffix = b"resumed-payload-after-redirect";
        let full: Vec<u8> = [prefix.as_slice(), suffix.as_slice()].concat();
        let expected_md5 = md5_hex(&full);
        let expected_range = format!("bytes={}-", prefix.len());

        for (code, status_line) in [
            ("301", "301 Moved Permanently"),
            ("302", "302 Found"),
            ("303", "303 See Other"),
            ("307", "307 Temporary Redirect"),
            ("308", "308 Permanent Redirect"),
        ] {
            let fx = DownloadFixture::new(&format!("resume-redirect-{code}"));
            fx.write_part("resumed.bin", prefix);

            let server = MockServer::start(vec![
                MockResponse::redirect(status_line, "/cdn/resumed.bin"),
                MockResponse::partial_content(prefix.len() as u64, full.len() as u64, suffix),
            ])
            .await;

            fx.download(
                &server.url("/download/resumed.bin"),
                "resumed.bin",
                Some(&expected_md5),
                None,
            )
            .await
            .unwrap_or_else(|err| panic!("resume across {code} redirect should succeed: {err}"));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "status {code}");
            assert!(
                requests[0]
                    .request_line
                    .starts_with("GET /download/resumed.bin "),
                "status {code}"
            );
            assert_eq!(
                requests[0].header("range"),
                Some(expected_range.as_str()),
                "status {code} hop 0 must send Range"
            );
            assert!(
                requests[1]
                    .request_line
                    .starts_with("GET /cdn/resumed.bin "),
                "status {code}"
            );
            assert_eq!(
                requests[1].header("range"),
                Some(expected_range.as_str()),
                "status {code} hop 1 must preserve Range"
            );
            fx.assert_downloaded("resumed.bin", &full);
        }
    }

    #[tokio::test]
    async fn test_download_raw_vs_cookies_txt_disambiguation_end_to_end() {
        let payload = b"cookie source disambiguation payload";
        let expected_md5 = md5_hex(payload);

        // Raw cookie string: strictly scoped to HTTPS archive.org, so http://127.0.0.1 receives no Cookie.
        let raw_source = CookieSource::from_input("logged-in-user=yes; logged-in-sig=abc123")
            .expect("raw cookie string should parse");
        assert!(matches!(raw_source, CookieSource::Raw(_)));

        let fx_raw = DownloadFixture::new("cookie-raw-e2e");
        let server_raw = MockServer::start(vec![MockResponse::ok(payload)]).await;
        fx_raw
            .download(
                &server_raw.url("/download/item.bin"),
                "item.bin",
                Some(&expected_md5),
                Some(&raw_source),
            )
            .await
            .expect("download with raw cookie source should succeed");

        let raw_requests = server_raw.finish().await;
        assert_eq!(raw_requests.len(), 1);
        assert_eq!(
            raw_requests[0].header("cookie"),
            None,
            "raw cookie must not be sent to plain HTTP 127.0.0.1"
        );
        fx_raw.assert_downloaded("item.bin", payload);

        // Netscape cookies.txt file: attaches only matching host and path cookies.
        let fx_file = DownloadFixture::new("cookie-file-e2e");
        let cookie_file = fx_file.write(
            "cookies.txt",
            concat!(
                "# Netscape HTTP Cookie File\n",
                "127.0.0.1\tFALSE\t/download\tFALSE\t0\tsession\tfromfile\n",
                "127.0.0.1\tFALSE\t/other\tFALSE\t0\tother\tskip\n",
                "example.com\tFALSE\t/\tFALSE\t0\tforeign\tskip\n",
            ),
        );
        let file_source = CookieSource::from_input(cookie_file.to_str().unwrap())
            .expect("cookies.txt file should parse");
        assert!(matches!(file_source, CookieSource::NetscapeFile(_)));

        let server_file = MockServer::start(vec![MockResponse::ok(payload)]).await;
        fx_file
            .download(
                &server_file.url("/download/item.bin"),
                "item.bin",
                Some(&expected_md5),
                Some(&file_source),
            )
            .await
            .expect("download with cookies.txt file should succeed");

        let file_requests = server_file.finish().await;
        assert_eq!(file_requests.len(), 1);
        assert_eq!(file_requests[0].header("cookie"), Some("session=fromfile"));
        fx_file.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_redirect_errors_fail_fast_without_retrying() {
        let policy = slow_retry_policy(3);

        let mut too_many_responses: Vec<MockResponse> = (0..=MAX_REDIRECTS)
            .map(|hop| MockResponse::redirect("302 Found", format!("/hop-{}", hop + 1)))
            .collect();
        too_many_responses.push(MockResponse::ok(b"should not retry"));

        let cases: Vec<(&str, &str, Vec<MockResponse>, usize, &str)> = vec![
            (
                "direct-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/a"),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Redirect loop",
            ),
            (
                "two-hop-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/b"),
                    MockResponse::redirect("302 Found", "/a"),
                    MockResponse::ok(b"should not retry"),
                ],
                2,
                "Redirect loop",
            ),
            (
                "fragment-only-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/a#section"),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Redirect loop",
            ),
            (
                "too-many-redirects",
                "/hop-0",
                too_many_responses,
                MAX_REDIRECTS + 1,
                "Too many redirects",
            ),
            (
                "missing-location",
                "/missing",
                vec![
                    MockResponse::status("302 Found", b""),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "missing Location",
            ),
            (
                "empty-location",
                "/empty",
                vec![
                    MockResponse::redirect("302 Found", "   "),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "empty Location",
            ),
            (
                "non-http-scheme",
                "/scheme",
                vec![
                    MockResponse::redirect("302 Found", "file:///etc/passwd"),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Unsupported redirect scheme",
            ),
        ];

        for (label, initial_path, responses, expected_requests, expected_msg) in cases {
            let fx = DownloadFixture::new(&format!("redirect-err-{label}"));
            let server = MockServer::start(responses).await;

            let start = std::time::Instant::now();
            let err = fx
                .download_with_policy(&server.url(initial_path), "item.bin", None, None, &policy)
                .await
                .unwrap_err();
            let elapsed = start.elapsed();

            let requests = server.abort().await;
            assert_eq!(requests.len(), expected_requests, "case {label}");
            assert!(
                elapsed < Duration::from_secs(2),
                "case {label}: redirect error must fail fast, took {elapsed:?}"
            );
            assert!(
                matches!(err, IaGetError::Network(_)),
                "case {label}: {err:?}"
            );
            assert!(
                err.to_string().contains(expected_msg),
                "case {label}: expected '{expected_msg}' in '{err}'"
            );
            assert!(!fx.join("item.bin").exists(), "case {label}");
            assert!(!fx.part_path("item.bin").exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_retry_after_redirected_503_restarts_from_initial_hop() {
        let payload = b"payload recovered after redirected 503";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/cdn/item.bin"),
            MockResponse::retry_after("503 Service Unavailable", "0", b"busy"),
            MockResponse::redirect("302 Found", "/cdn/item.bin"),
            MockResponse::ok(payload),
        ])
        .await;

        let fx = DownloadFixture::new("redirect-retry-503");
        let policy = slow_retry_policy(2);

        let start = std::time::Instant::now();
        fx.download_with_policy(
            &server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
            None,
            &policy,
        )
        .await
        .expect("retry after redirected 503 should restart from hop 0 and succeed");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[0]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert!(requests[1].request_line.starts_with("GET /cdn/item.bin "));
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert!(requests[3].request_line.starts_with("GET /cdn/item.bin "));
        assert!(
            elapsed < Duration::from_secs(2),
            "Retry-After: 0 should retry immediately, took {elapsed:?}"
        );
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_cookie_expiry_reevaluated_across_retry_wait() {
        let payload = b"payload delivered after short-lived cookie expired";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::retry_after("503 Service Unavailable", "2", b"busy"),
            MockResponse::ok(payload),
        ])
        .await;

        let fx = DownloadFixture::new("cookie-expiry-retry");
        let policy = slow_retry_policy(2);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_secs();
        let cookie_source = CookieSource::from_netscape_str(&format!(
            "127.0.0.1\tFALSE\t/\tFALSE\t{}\tshort\ttemp\n\
             127.0.0.1\tFALSE\t/\tFALSE\t{}\tlong\tkeep\n",
            now + 2,
            now + 3600
        ));

        fx.download_with_policy(
            &server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
            Some(&cookie_source),
            &policy,
        )
        .await
        .expect("retry after 503 should succeed with re-evaluated cookies");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("cookie"), Some("short=temp; long=keep"));
        assert_eq!(
            requests[1].header("cookie"),
            Some("long=keep"),
            "expired short-lived cookie must be omitted on retry"
        );
        fx.assert_downloaded("item.bin", payload);
    }

    #[test]
    fn test_resolve_redirect_target_allows_valid_transitions() {
        for (current_str, location, expected) in [
            (
                "https://archive.org/download/item/a",
                "https://mirror.example/download/item/a",
                "https://mirror.example/download/item/a",
            ),
            (
                "https://archive.org/download/item/a",
                "/cdn/item/a",
                "https://archive.org/cdn/item/a",
            ),
            (
                "https://archive.org/download/item/a",
                "b",
                "https://archive.org/download/item/b",
            ),
            (
                "http://archive.org/download/item/a",
                "http://mirror.example/a",
                "http://mirror.example/a",
            ),
            (
                "http://archive.org/download/item/a",
                "https://archive.org/download/item/a",
                "https://archive.org/download/item/a",
            ),
        ] {
            let current = Url::parse(current_str).unwrap();
            let resolved = resolve_redirect_target(&current, location)
                .unwrap_or_else(|err| panic!("{current_str} -> {location} failed: {err}"));
            assert_eq!(resolved.as_str(), expected);
        }
    }

    #[test]
    fn test_resolve_redirect_target_rejects_downgrade_and_invalid_locations() {
        let current = Url::parse("https://archive.org/download/item/a").unwrap();

        for (location, expected_msg) in [
            ("http://archive.org/download/item/a", "downgrade"),
            ("http://mirror.example/a", "downgrade"),
            ("", "empty Location"),
            ("   ", "empty Location"),
            ("file:///etc/passwd", "Unsupported redirect scheme"),
            (
                "ftp://archive.org/download/item/a",
                "Unsupported redirect scheme",
            ),
            ("http://[::1", "Invalid redirect Location"),
        ] {
            let err = resolve_redirect_target(&current, location).unwrap_err();
            assert!(
                matches!(err, IaGetError::Network(_)),
                "location '{location}': expected Network error, got {err:?}"
            );
            assert!(
                err.to_string().contains(expected_msg),
                "location '{location}': expected '{expected_msg}' in '{err}'"
            );
        }
    }
}
