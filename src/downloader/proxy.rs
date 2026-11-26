//! Environment-based HTTP, HTTPS, and SOCKS proxy configuration, validation, and redaction.

use std::ffi::OsString;

use reqwest::{Client, ClientBuilder, Url};

use crate::error::IaGetError;
use crate::Result;

/// Supported proxy URL schemes for environment-based proxy configuration.
const ALLOWED_PROXY_SCHEMES: &[&str] = &["http", "https", "socks4", "socks4a", "socks5", "socks5h"];

/// Validated HTTP, HTTPS, SOCKS, and bypass proxy configuration resolved from environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    pub http_proxy: Option<Url>,
    pub https_proxy: Option<Url>,
    pub all_proxy: Option<Url>,
    pub no_proxy: Option<String>,
}

fn is_rfc3986_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

/// Redacts embedded credentials (`userinfo@`) from a proxy URL before including it in errors.
pub(super) fn redact_proxy_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let (scheme_prefix, rest) = match trimmed.find("://") {
        Some(idx) if is_rfc3986_scheme(&trimmed[..idx]) => trimmed.split_at(idx + 3),
        _ => ("", trimmed),
    };

    if let Some(at_idx) = rest.rfind('@') {
        format!("{scheme_prefix}<redacted>@{}", &rest[at_idx + 1..])
    } else {
        trimmed.to_string()
    }
}

fn invalid_proxy(var: &str, raw: &str, reason: impl Into<String>) -> IaGetError {
    IaGetError::InvalidProxy {
        var: var.to_string(),
        value: redact_proxy_value(raw),
        reason: reason.into(),
    }
}

fn validate_parsed_proxy_url(var: &str, raw_for_error: &str, url: &Url) -> Result<()> {
    let scheme = url.scheme();
    if !ALLOWED_PROXY_SCHEMES.contains(&scheme) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            format!(
                "unsupported scheme '{scheme}'; expected one of http, https, socks4, socks4a, socks5, socks5h"
            ),
        ));
    }

    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            "proxy URL must include a host",
        ));
    }

    if url.port() == Some(0) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            "proxy port must not be 0",
        ));
    }

    Ok(())
}

fn parse_proxy_url(var: &str, raw: &str) -> Result<Option<Url>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must not contain whitespace or control characters",
        ));
    }

    if !trimmed.contains("://") {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must include a scheme followed by '://'",
        ));
    }

    let url = Url::parse(trimmed)
        .map_err(|err| invalid_proxy(var, trimmed, format!("invalid URL ({err})")))?;

    if !trimmed
        .get(url.scheme().len()..)
        .is_some_and(|rest| rest.starts_with("://"))
    {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must include '://' immediately after the scheme",
        ));
    }

    validate_parsed_proxy_url(var, trimmed, &url)?;
    Ok(Some(url))
}

fn lookup_os_pair<F>(lookup: &F, lower: &str, upper: &str) -> Result<Option<(String, String)>>
where
    F: Fn(&str) -> Option<OsString>,
{
    let (var, raw) = if let Some(val) = lookup(lower) {
        (lower, val)
    } else if let Some(val) = lookup(upper) {
        (upper, val)
    } else {
        return Ok(None);
    };

    let value = raw.into_string().map_err(|_| IaGetError::InvalidProxy {
        var: var.to_string(),
        value: "<non-UTF-8>".to_string(),
        reason: "environment variable value is not valid UTF-8".to_string(),
    })?;

    Ok(Some((var.to_string(), value)))
}

impl ProxyConfig {
    /// Resolves and validates proxy configuration from an OS-string lookup function.
    pub fn from_os_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<OsString>,
    {
        let http_proxy = lookup_os_pair(&lookup, "http_proxy", "HTTP_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let https_proxy = lookup_os_pair(&lookup, "https_proxy", "HTTPS_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let all_proxy = lookup_os_pair(&lookup, "all_proxy", "ALL_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let no_proxy = lookup_os_pair(&lookup, "no_proxy", "NO_PROXY")?.and_then(|(_, value)| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        Ok(Self {
            http_proxy,
            https_proxy,
            all_proxy,
            no_proxy,
        })
    }

    /// Resolves and validates proxy configuration from the current process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_os_lookup(|name| std::env::var_os(name))
    }
}

/// Returns a [`reqwest::ClientBuilder`] with automatic redirect following disabled
/// so redirects are handled manually with per-hop cookie resolution.
pub fn http_client_builder() -> ClientBuilder {
    Client::builder().redirect(reqwest::redirect::Policy::none())
}

fn build_reqwest_proxy(
    var: &str,
    url: &Url,
    no_proxy: Option<reqwest::NoProxy>,
    ctor: fn(Url) -> std::result::Result<reqwest::Proxy, reqwest::Error>,
) -> Result<reqwest::Proxy> {
    validate_parsed_proxy_url(var, url.as_str(), url)?;
    let proxy = ctor(url.clone())
        .map_err(|err| invalid_proxy(var, url.as_str(), err.without_url().to_string()))?;
    Ok(proxy.no_proxy(no_proxy))
}

/// Returns a [`reqwest::ClientBuilder`] with automatic redirect following disabled
/// and explicit proxy rules configured from `proxy_config`.
pub fn http_client_builder_with_proxy(proxy_config: &ProxyConfig) -> Result<ClientBuilder> {
    let mut builder = http_client_builder().no_proxy();
    let no_proxy = proxy_config
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);

    if let Some(url) = &proxy_config.http_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "HTTP_PROXY",
            url,
            no_proxy.clone(),
            reqwest::Proxy::http,
        )?);
    }

    if let Some(url) = &proxy_config.https_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "HTTPS_PROXY",
            url,
            no_proxy.clone(),
            reqwest::Proxy::https,
        )?);
    }

    if let Some(url) = &proxy_config.all_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "ALL_PROXY",
            url,
            no_proxy,
            reqwest::Proxy::all,
        )?);
    }

    Ok(builder)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::downloader::tests::DownloadFixture;
    use crate::downloader::FileOutcome;
    use crate::test_support::{
        fast_retry_policy, md5_hex, CapturedRequest, MockResponse, MockServer,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;

    fn proxy_config_from_vars(vars: &[(&str, &str)]) -> Result<ProxyConfig> {
        ProxyConfig::from_os_lookup(|name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(*v))
        })
    }

    fn proxy_client(vars: &[(&str, &str)]) -> reqwest::Client {
        let config = proxy_config_from_vars(vars).unwrap();
        http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap()
    }

    fn assert_invalid_proxy(
        err: IaGetError,
        expected_var: &str,
        expected_value: Option<&str>,
        forbidden_tokens: &[&str],
        expect_redacted: bool,
        context: &str,
    ) {
        let rendered = err.to_string();
        match err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, expected_var, "{context}: unexpected var");
                if let Some(expected) = expected_value {
                    assert_eq!(value, expected, "{context}: unexpected redacted value");
                }
                if expect_redacted {
                    assert!(
                        value.contains("<redacted>@"),
                        "{context}: expected redaction marker in value: {value}"
                    );
                    assert!(
                        rendered.contains("<redacted>@"),
                        "{context}: expected redaction marker in Display: {rendered}"
                    );
                }
                for token in forbidden_tokens {
                    assert!(
                        !value.contains(token),
                        "{context}: forbidden credential token leaked in value"
                    );
                    assert!(
                        !rendered.contains(token),
                        "{context}: forbidden credential token leaked in Display"
                    );
                }
            }
            other => panic!("{context}: expected InvalidProxy, got {other:?}"),
        }
    }

    #[test]
    fn proxy_config_env_precedence_and_empty_values() {
        struct ProxyEnvCase<'a> {
            label: &'a str,
            vars: &'a [(&'a str, &'a str)],
            http: Option<&'a str>,
            https: Option<&'a str>,
            all: Option<&'a str>,
            no_proxy: Option<&'a str>,
        }

        let cases = [
            ProxyEnvCase {
                label: "unset",
                vars: &[],
                http: None,
                https: None,
                all: None,
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "whitespace-only",
                vars: &[
                    ("HTTP_PROXY", ""),
                    ("HTTPS_PROXY", "   "),
                    ("ALL_PROXY", "\t \n"),
                    ("NO_PROXY", "  \t "),
                ],
                http: None,
                https: None,
                all: None,
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "empty-lowercase-blocks-uppercase-fallback",
                vars: &[
                    ("http_proxy", ""),
                    ("HTTP_PROXY", "http://upper-http.example.com:8080"),
                    ("https_proxy", "   "),
                    ("HTTPS_PROXY", "https://upper-https.example.com:8443"),
                    ("all_proxy", "  socks5h://127.0.0.1:1080  "),
                    ("ALL_PROXY", "socks5://upper-all.example.com:1080"),
                    ("no_proxy", ""),
                    ("NO_PROXY", "archive.org"),
                ],
                http: None,
                https: None,
                all: Some("socks5h://127.0.0.1:1080"),
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "uppercase-fallback-when-lowercase-unset",
                vars: &[
                    ("HTTP_PROXY", "http://proxy.example.com:8080"),
                    ("HTTPS_PROXY", "https://secure.example.com:8443"),
                    ("ALL_PROXY", "socks5://127.0.0.1:1080"),
                    ("NO_PROXY", " localhost, .archive.org "),
                ],
                http: Some("http://proxy.example.com:8080"),
                https: Some("https://secure.example.com:8443"),
                all: Some("socks5://127.0.0.1:1080"),
                no_proxy: Some("localhost, .archive.org"),
            },
            ProxyEnvCase {
                label: "non-empty-lowercase-overrides-uppercase",
                vars: &[
                    ("http_proxy", "http://lower-http.example:8080"),
                    ("HTTP_PROXY", "http://upper-http.example:8080"),
                    ("https_proxy", "https://lower-https.example:8443"),
                    ("HTTPS_PROXY", "https://upper-https.example:8443"),
                    ("all_proxy", "socks5://lower-all.example:1080"),
                    ("ALL_PROXY", "socks5://upper-all.example:1080"),
                    ("no_proxy", "127.0.0.1,localhost"),
                    ("NO_PROXY", "archive.org"),
                ],
                http: Some("http://lower-http.example:8080"),
                https: Some("https://lower-https.example:8443"),
                all: Some("socks5://lower-all.example:1080"),
                no_proxy: Some("127.0.0.1,localhost"),
            },
        ];

        for case in cases {
            let config = proxy_config_from_vars(case.vars).unwrap();
            assert_eq!(
                config.http_proxy,
                case.http.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(
                config.https_proxy,
                case.https.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(
                config.all_proxy,
                case.all.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(config.no_proxy.as_deref(), case.no_proxy, "{}", case.label);
            if case.http.is_none()
                && case.https.is_none()
                && case.all.is_none()
                && case.no_proxy.is_none()
            {
                assert_eq!(config, ProxyConfig::default(), "{}", case.label);
                http_client_builder_with_proxy(&config)
                    .unwrap()
                    .build()
                    .unwrap();
            }
        }
    }

    #[test]
    fn proxy_config_accepts_supported_schemes_ipv6_credentials_and_omitted_ports() {
        let valid_cases = [
            ("http://127.0.0.1:8080", "http"),
            ("https://127.0.0.1:8443", "https"),
            ("socks4://127.0.0.1:1080", "socks4"),
            ("socks4a://127.0.0.1:1080", "socks4a"),
            ("socks5://127.0.0.1:1080", "socks5"),
            ("socks5h://user:pass@127.0.0.1:1080", "socks5h"),
            ("HTTP://127.0.0.1:8080", "http"),
            ("HTTPS://127.0.0.1:8443", "https"),
            ("SOCKS4://127.0.0.1:1080", "socks4"),
            ("SOCKS4A://127.0.0.1:1080", "socks4a"),
            ("SOCKS5://127.0.0.1:1080", "socks5"),
            ("SOCKS5H://127.0.0.1:1080", "socks5h"),
            ("http://[::1]:8080", "http"),
            ("https://[::1]:8443", "https"),
            ("socks5://[::1]:1080", "socks5"),
            ("socks5h://[::1]:1080", "socks5h"),
            ("socks4://127.0.0.1", "socks4"),
            ("socks4a://proxy.example", "socks4a"),
            ("socks5://127.0.0.1", "socks5"),
            ("socks5h://proxy.example", "socks5h"),
            ("http://alice:secret@127.0.0.1:8080", "http"),
            ("https://token@127.0.0.1:8443", "https"),
            ("socks5://bob:pw@[::1]:1080", "socks5"),
        ];

        for (raw, expected_scheme) in valid_cases {
            let config = proxy_config_from_vars(&[("ALL_PROXY", raw)])
                .unwrap_or_else(|err| panic!("expected '{raw}' to be valid, got {err}"));
            assert_eq!(
                config.all_proxy.as_ref().expect("all_proxy set").scheme(),
                expected_scheme
            );
            http_client_builder_with_proxy(&config)
                .unwrap_or_else(|err| panic!("builder failed for '{raw}': {err}"))
                .build()
                .unwrap_or_else(|err| panic!("client build failed for '{raw}': {err}"));
        }
    }

    #[test]
    fn proxy_config_rejects_invalid_urls_and_redacts_credentials() {
        let invalid_cases: [(&str, &str, Option<&str>, &[&str]); 27] = [
            ("HTTP_PROXY", "proxy.example.com:8080", None, &[]),
            (
                "HTTP_PROXY",
                "user:topsecret@proxy.example.com:8080",
                None,
                &["topsecret"],
            ),
            (
                "HTTPS_PROXY",
                "ftp://alice:s3cr3t@proxy.example.com:21",
                None,
                &["s3cr3t"],
            ),
            ("ALL_PROXY", "socks://127.0.0.1:1080", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:pw@proxy.example.com:0",
                None,
                &["pw"],
            ),
            ("ALL_PROXY", "socks5://:1080", None, &[]),
            ("ALL_PROXY", "socks5:///path", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:hunter2@proxy host:8080",
                None,
                &["hunter2"],
            ),
            (
                "HTTP_PROXY",
                "http://user:hidden@proxy.example.com:8080\0",
                None,
                &["hidden"],
            ),
            ("HTTP_PROXY", "http:example.com/a://b", None, &[]),
            ("HTTP_PROXY", "not-a-url", None, &[]),
            ("HTTP_PROXY", "proxy.local:8080", None, &[]),
            ("HTTP_PROXY", "http://", None, &[]),
            ("HTTP_PROXY", "http://:8080", None, &[]),
            ("ALL_PROXY", "socks5://", None, &[]),
            ("HTTP_PROXY", "http://127.0.0.1:99999", None, &[]),
            ("HTTPS_PROXY", "https://127.0.0.1:abc", None, &[]),
            ("ALL_PROXY", "socks5://127.0.0.1:0", None, &[]),
            ("HTTP_PROXY", "://bad", None, &[]),
            ("ALL_PROXY", "socks5://127.0.0.1\t:1080", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:p@ssword@proxy.local:99999",
                None,
                &["p@ssword"],
            ),
            (
                "ALL_PROXY",
                "socks5://alice:supersecret@:1080",
                None,
                &["supersecret"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p/ass@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p/ass"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p?w@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p?w"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p#w@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p#w"],
            ),
            (
                "HTTP_PROXY",
                "alice:p/ass@proxy.local:8080",
                Some("<redacted>@proxy.local:8080"),
                &["alice", "p/ass"],
            ),
            (
                "HTTP_PROXY",
                "http:alice:secret@proxy.local/a://b",
                Some("<redacted>@proxy.local/a://b"),
                &["alice", "secret"],
            ),
        ];

        for (var_name, raw, expected_redacted, forbidden_tokens) in invalid_cases {
            let err = proxy_config_from_vars(&[(var_name, raw)])
                .expect_err(&format!("expected '{raw}' to fail"));
            assert_invalid_proxy(
                err,
                var_name,
                expected_redacted,
                forbidden_tokens,
                !forbidden_tokens.is_empty(),
                raw,
            );
        }

        // Direct builder validation also rejects unsupported schemes and redacts credentials.
        let direct_invalid = ProxyConfig {
            http_proxy: Some(Url::parse("ftp://alice:s3cr3t@proxy.local:21").unwrap()),
            ..ProxyConfig::default()
        };
        let err = http_client_builder_with_proxy(&direct_invalid)
            .expect_err("direct invalid scheme must be rejected");
        assert_invalid_proxy(err, "HTTP_PROXY", None, &["s3cr3t"], true, "direct-ftp");
    }

    #[cfg(unix)]
    #[test]
    fn proxy_config_rejects_non_utf8_os_values() {
        use std::os::unix::ffi::OsStringExt;

        let cases: [(&str, Vec<(&str, OsString)>); 3] = [
            (
                "http_proxy",
                vec![
                    ("http_proxy", OsString::from_vec(vec![0xff, 0xfe])),
                    ("HTTP_PROXY", OsString::from("http://127.0.0.1:8080")),
                ],
            ),
            (
                "ALL_PROXY",
                vec![("ALL_PROXY", OsString::from_vec(vec![0xff, 0x80]))],
            ),
            (
                "NO_PROXY",
                vec![("NO_PROXY", OsString::from_vec(vec![0xfe, 0xff]))],
            ),
        ];

        for (expected_var, vars) in cases {
            let err = ProxyConfig::from_os_lookup(|name| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
            })
            .expect_err(&format!("non-UTF-8 {expected_var} must fail"));
            assert_invalid_proxy(
                err,
                expected_var,
                Some("<non-UTF-8>"),
                &[],
                false,
                expected_var,
            );
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockSocks5Target {
        Ipv4([u8; 4], u16),
        Domain(String, u16),
        Ipv6([u8; 16], u16),
    }

    #[derive(Debug, Clone)]
    struct CapturedSocks5Connection {
        target: MockSocks5Target,
        request: CapturedRequest,
    }

    struct MockSocks5Server {
        addr: std::net::SocketAddr,
        connections: Arc<Mutex<Vec<CapturedSocks5Connection>>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockSocks5Server {
        async fn start(bodies: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock SOCKS5 server");
            let addr = listener.local_addr().expect("mock SOCKS5 local_addr");
            let connections = Arc::new(Mutex::new(Vec::new()));
            let connections_clone = Arc::clone(&connections);

            let handle = tokio::spawn(async move {
                for body in bodies {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = socket.set_nodelay(true);

                    let Some(target) = socks5_handshake(&mut socket).await else {
                        continue;
                    };
                    let Some(request) = socks5_read_http_request(&mut socket).await else {
                        continue;
                    };
                    connections_clone
                        .lock()
                        .await
                        .push(CapturedSocks5Connection { target, request });
                    let mut wire = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    wire.extend_from_slice(&body);
                    let _ = socket.write_all(&wire).await;
                    let _ = socket.flush().await;
                    let _ = socket.shutdown().await;
                }
            });

            Self {
                addr,
                connections,
                handle: Some(handle),
            }
        }

        fn proxy_url(&self, scheme: &str) -> String {
            format!("{scheme}://127.0.0.1:{}", self.addr.port())
        }

        async fn finish(mut self) -> Vec<CapturedSocks5Connection> {
            if let Some(handle) = self.handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
            }
            self.connections.lock().await.clone()
        }

        async fn abort(mut self) -> Vec<CapturedSocks5Connection> {
            if let Some(handle) = self.handle.take() {
                handle.abort();
                let _ = handle.await;
            }
            self.connections.lock().await.clone()
        }
    }

    impl Drop for MockSocks5Server {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    async fn socks5_read_http_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(buf.len());
        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next()?.to_string();
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect();
        Some(CapturedRequest {
            request_line,
            headers,
        })
    }

    async fn socks5_handshake(socket: &mut TcpStream) -> Option<MockSocks5Target> {
        // RFC 1928 greeting: VER=0x05, NMETHODS, METHODS[NMETHODS]
        let mut greeting = [0u8; 2];
        socket.read_exact(&mut greeting).await.ok()?;
        if greeting[0] != 0x05 {
            return None;
        }
        let mut methods = vec![0u8; usize::from(greeting[1])];
        socket.read_exact(&mut methods).await.ok()?;
        socket.write_all(&[0x05, 0x00]).await.ok()?;
        socket.flush().await.ok()?;

        // RFC 1928 request: VER=0x05, CMD=0x01 (CONNECT), RSV=0x00, ATYP
        let mut req_head = [0u8; 4];
        socket.read_exact(&mut req_head).await.ok()?;
        if req_head[0] != 0x05 || req_head[1] != 0x01 || req_head[2] != 0x00 {
            return None;
        }

        let target = match req_head[3] {
            0x01 => {
                let mut ipv4 = [0u8; 4];
                let mut port = [0u8; 2];
                socket.read_exact(&mut ipv4).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Ipv4(ipv4, u16::from_be_bytes(port))
            }
            0x03 => {
                let mut len = [0u8; 1];
                socket.read_exact(&mut len).await.ok()?;
                let mut domain = vec![0u8; usize::from(len[0])];
                let mut port = [0u8; 2];
                socket.read_exact(&mut domain).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Domain(String::from_utf8(domain).ok()?, u16::from_be_bytes(port))
            }
            0x04 => {
                let mut ipv6 = [0u8; 16];
                let mut port = [0u8; 2];
                socket.read_exact(&mut ipv6).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Ipv6(ipv6, u16::from_be_bytes(port))
            }
            _ => return None,
        };

        socket
            .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
            .await
            .ok()?;
        socket.flush().await.ok()?;
        Some(target)
    }

    #[tokio::test]
    async fn proxy_http_proxy_routing_and_precedence() {
        // 1. Direct HTTP_PROXY routing; 2. ALL_PROXY fallback; 3. HTTP_PROXY overrides ALL_PROXY.
        for (label, use_http_proxy, use_all_proxy, filename) in [
            ("http-proxy-route", true, false, "test.bin"),
            ("all-proxy-fallback", false, true, "fallback.bin"),
            ("http-proxy-precedence", true, true, "precedence.bin"),
        ] {
            let payload = format!("payload for {label}").into_bytes();
            let primary = MockServer::start(vec![MockResponse::ok(&payload)]).await;
            let primary_url = format!("http://127.0.0.1:{}", primary.port());
            let unused = if use_http_proxy && use_all_proxy {
                Some(MockServer::start(vec![MockResponse::ok(b"wrong-proxy")]).await)
            } else {
                None
            };
            let unused_url = unused
                .as_ref()
                .map(|s| format!("http://127.0.0.1:{}", s.port()));

            let mut vars = Vec::new();
            if use_http_proxy {
                vars.push(("HTTP_PROXY", primary_url.as_str()));
                if let Some(ref url) = unused_url {
                    vars.push(("ALL_PROXY", url.as_str()));
                }
            } else if use_all_proxy {
                vars.push(("ALL_PROXY", primary_url.as_str()));
            }

            let fx = DownloadFixture::with_client(label, proxy_client(&vars));
            let target_url = format!("http://archive.example/download/item/{filename}");
            let outcome = fx
                .download(&target_url, filename, Some(&md5_hex(&payload)), None)
                .await
                .unwrap_or_else(|err| panic!("case {label} failed: {err}"));

            assert_eq!(outcome, FileOutcome::Downloaded, "{label}");
            let requests = primary.finish().await;
            assert_eq!(requests.len(), 1, "{label}");
            assert_eq!(
                requests[0].request_line,
                format!("GET {target_url} HTTP/1.1"),
                "{label}"
            );
            if let Some(unused_server) = unused {
                assert!(
                    unused_server.abort().await.is_empty(),
                    "ALL_PROXY must not be contacted when HTTP_PROXY matches"
                );
            }
            fx.assert_downloaded(filename, &payload);
        }
    }

    #[tokio::test]
    async fn proxy_https_proxy_sends_connect_without_external_network() {
        let proxy_server =
            MockServer::start(vec![MockResponse::status("502 Bad Gateway", b"")]).await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.port());

        let fx = DownloadFixture::with_client(
            "https-proxy-connect",
            proxy_client(&[("HTTPS_PROXY", &proxy_url)]),
        );
        let policy = fast_retry_policy(0);

        let result = fx
            .download_with_policy(
                "https://archive.example/download/item/test.bin",
                "test.bin",
                None,
                None,
                &policy,
            )
            .await;
        assert!(
            result.is_err(),
            "CONNECT rejection with 502 Bad Gateway should fail the download"
        );

        let requests = proxy_server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].request_line,
            "CONNECT archive.example:443 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn proxy_no_proxy_bypasses_http_and_socks5_proxies() {
        // 1. NO_PROXY bypasses HTTP_PROXY
        let http_payload = b"served directly from local mock origin".to_vec();
        let origin_http = MockServer::start(vec![MockResponse::ok(&http_payload)]).await;
        let http_proxy = MockServer::start(vec![MockResponse::ok(b"wrong-from-proxy")]).await;
        let http_proxy_url = format!("http://127.0.0.1:{}", http_proxy.port());
        let fx_http = DownloadFixture::with_client(
            "no-proxy-http-bypass",
            proxy_client(&[("HTTP_PROXY", &http_proxy_url), ("NO_PROXY", "127.0.0.1")]),
        );

        let outcome = fx_http
            .download(
                &origin_http.url("/direct.bin"),
                "direct.bin",
                Some(&md5_hex(&http_payload)),
                None,
            )
            .await
            .expect("NO_PROXY bypassed HTTP download should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_reqs = origin_http.finish().await;
        assert_eq!(origin_reqs.len(), 1);
        assert_eq!(origin_reqs[0].request_line, "GET /direct.bin HTTP/1.1");
        assert!(http_proxy.abort().await.is_empty());
        fx_http.assert_downloaded("direct.bin", &http_payload);

        // 2. NO_PROXY bypasses SOCKS5 ALL_PROXY
        let socks_payload = b"origin payload bypassing SOCKS5 proxy".to_vec();
        let origin_socks = MockServer::start(vec![MockResponse::ok(&socks_payload)]).await;
        let socks_proxy = MockSocks5Server::start(vec![b"wrong-from-socks5".to_vec()]).await;
        let socks_url = socks_proxy.proxy_url("socks5");
        let fx_socks = DownloadFixture::with_client(
            "no-proxy-socks5-bypass",
            proxy_client(&[("ALL_PROXY", &socks_url), ("NO_PROXY", "127.0.0.1")]),
        );

        let outcome = fx_socks
            .download(
                &origin_socks.url("/bypass-socks.bin"),
                "bypass-socks.bin",
                Some(&md5_hex(&socks_payload)),
                None,
            )
            .await
            .expect("NO_PROXY bypassed SOCKS5 download should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_reqs = origin_socks.finish().await;
        assert_eq!(origin_reqs.len(), 1);
        assert_eq!(
            origin_reqs[0].request_line,
            "GET /bypass-socks.bin HTTP/1.1"
        );
        assert!(socks_proxy.abort().await.is_empty());
        fx_socks.assert_downloaded("bypass-socks.bin", &socks_payload);
    }

    #[tokio::test]
    async fn proxy_all_proxy_socks5_and_socks5h_tunnel_downloads() {
        let cases = [
            (
                "socks5",
                "http://127.0.0.1:80/socks5.bin",
                "socks5.bin",
                MockSocks5Target::Ipv4([127, 0, 0, 1], 80),
                "GET /socks5.bin HTTP/1.1",
                None,
            ),
            (
                "socks5h",
                "http://archive.example/socks5h.bin",
                "socks5h.bin",
                MockSocks5Target::Domain("archive.example".to_string(), 80),
                "GET /socks5h.bin HTTP/1.1",
                Some("archive.example"),
            ),
        ];

        for (scheme, url, filename, expected_target, expected_line, expected_host) in cases {
            let payload = format!("tunneled through {scheme} proxy").into_bytes();
            let socks_server = MockSocks5Server::start(vec![payload.clone()]).await;
            let socks_url = socks_server.proxy_url(scheme);
            let fx = DownloadFixture::with_client(
                &format!("{scheme}-tunnel"),
                proxy_client(&[("ALL_PROXY", &socks_url)]),
            );

            let outcome = fx
                .download(url, filename, Some(&md5_hex(&payload)), None)
                .await
                .unwrap_or_else(|err| panic!("{scheme} tunneled download failed: {err}"));

            assert_eq!(outcome, FileOutcome::Downloaded, "{scheme}");
            let connections = socks_server.finish().await;
            assert_eq!(connections.len(), 1, "{scheme}");
            assert_eq!(connections[0].target, expected_target, "{scheme}");
            assert_eq!(
                connections[0].request.request_line, expected_line,
                "{scheme}"
            );
            if let Some(host) = expected_host {
                assert_eq!(
                    connections[0].request.header("Host"),
                    Some(host),
                    "{scheme}"
                );
            }
            fx.assert_downloaded(filename, &payload);
        }
    }
}
