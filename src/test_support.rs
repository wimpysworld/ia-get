//! Shared helpers for unit tests, used by both the library and the binary.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use digest::Digest;
use reqwest::{Client, StatusCode};
use url::Url;

use crate::archive_metadata::XmlFile;
use crate::downloader::{DownloadTask, download_file_content, prepare_file_for_download};

/// A unique temp directory for a test that removes itself on drop, so a
/// test that panics (or simply ends) never leaves a directory behind in
/// the OS temp dir.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates a unique, freshly-created temp directory for a test.
    pub fn new(test_name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ia-get-test-{}-{}-{}",
            std::process::id(),
            test_name,
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("failed to create temp dir");
        Self(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Returns a file's mtime as unix seconds, if readable.
pub fn mtime_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// The lowercase hex MD5 of `content`, in the form archive.org reports.
pub fn md5_hex(content: &[u8]) -> String {
    crate::downloader::digest_hex(md5::Md5::digest(content))
}

/// A minimal `XmlFile` test fixture: only the name and size are set.
pub fn xml_file(name: &str, size: Option<u64>) -> XmlFile {
    XmlFile {
        name: name.to_string(),
        size,
        ..Default::default()
    }
}

/// A "running" flag set to true, for tests that drive the download pipeline
/// without registering a real Ctrl+C handler.
pub fn test_running() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

/// Near-instant retry delay so tests do not wait for real backoff
pub fn fast_retry(_attempt: u32) -> Duration {
    Duration::from_millis(1)
}

/// Builds a `DownloadTask`
pub fn task(
    url: impl Into<String>,
    file_path: impl Into<String>,
    md5: Option<String>,
    size: Option<u64>,
    mtime: Option<u64>,
) -> DownloadTask {
    DownloadTask {
        url: url.into(),
        file_path: file_path.into(),
        expected_md5: md5,
        expected_size: size,
        expected_mtime: mtime,
    }
}

/// A 200 response with an empty body — the fallback for most scripts
pub fn ok_empty() -> MockResponse {
    MockResponse::new(200, MockBody::Full(vec![]))
}

/// Mock server serving "/file.bin" from `responses`, plus a fresh temp
/// dir; the file under test is `dir.join("file.bin")`
pub fn file_server(
    name: &str,
    responses: VecDeque<MockResponse>,
    fallback: MockResponse,
) -> (MockServer, TempDir) {
    let mut scripts = HashMap::new();
    scripts.insert("/file.bin".to_string(), responses);
    let server = MockServer::start(scripts, fallback);
    (server, TempDir::new(name))
}

/// A single-file batch task for "/file.bin" in `dir`
pub fn file_task(
    server: &MockServer,
    dir: &Path,
    md5: Option<String>,
    size: Option<u64>,
    mtime: Option<u64>,
) -> DownloadTask {
    task(
        server.url("/file.bin"),
        dir.join("file.bin").to_str().unwrap().to_string(),
        md5,
        size,
        mtime,
    )
}

/// Runs a single download against the mock server and returns the
/// captured `Last-Modified` time.
pub async fn run_download(
    url: &str,
    part_path: &str,
    expected_size: Option<u64>,
    running: &Arc<AtomicBool>,
) -> crate::Result<Option<SystemTime>> {
    run_download_with_rate(url, part_path, expected_size, running, None).await
}

/// Like `run_download`, but with a `--limit-rate` value (bytes/second)
/// driving the pacing, for interrupt-during-pace scenarios.
pub async fn run_download_with_rate(
    url: &str,
    part_path: &str,
    expected_size: Option<u64>,
    running: &Arc<AtomicBool>,
    rate_limit: Option<u64>,
) -> crate::Result<Option<SystemTime>> {
    let client = Client::new();
    let mut file = prepare_file_for_download(part_path)?;
    let result = download_file_content(
        &client,
        url,
        &mut file,
        running,
        None,
        expected_size,
        fast_retry,
        rate_limit,
    )
    .await;
    drop(file);
    result
}

/// Body variant for mock server responses
#[derive(Clone)]
pub enum MockBody {
    /// Send the whole body
    Full(Vec<u8>),
    /// Announce `announced_len` bytes via Content-Length, send only
    /// `partial` bytes, then close the connection mid-body
    Truncated {
        announced_len: u64,
        partial: Vec<u8>,
    },
    /// Announce `announced_len` bytes via Content-Length, then hold the
    /// connection open without ever sending the body (a stalled server)
    Stalled { announced_len: u64 },
    /// Serve `data` honoring the request's `Range` header like a real origin
    /// server: a full 200 with no Range, a 206 with the tail and a matching
    /// `Content-Range` for a satisfying `bytes=N-`, or a 416 when `N` is at
    /// or past the end. Lets resume tests verify the client's offset against
    /// behaviour, not script order.
    Ranged(Vec<u8>),
}

/// A scripted response: status, body and extra headers
#[derive(Clone)]
pub struct MockResponse {
    status: u16,
    body: MockBody,
    extra_headers: Vec<(String, String)>,
}

impl MockResponse {
    pub fn new(status: u16, body: MockBody) -> Self {
        Self {
            status,
            body,
            extra_headers: vec![],
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.extra_headers
            .push((name.to_string(), value.to_string()));
        self
    }

    /// A 200 that announces `announced_len` bytes via Content-Length but
    /// never delivers the body: simulates a stalled server connection
    pub fn stalled(announced_len: u64) -> Self {
        Self {
            status: 200,
            body: MockBody::Stalled { announced_len },
            extra_headers: vec![],
        }
    }

    /// A body that honors the request's `Range` header like a real origin
    /// server (200/206/416): the scripted status is ignored, the status is
    /// derived from the Range so resume is verified against behaviour.
    pub fn ranged(data: Vec<u8>) -> Self {
        Self {
            status: 200,
            body: MockBody::Ranged(data),
            extra_headers: vec![],
        }
    }
}

struct ServerState {
    scripts: HashMap<String, VecDeque<MockResponse>>,
    fallback: MockResponse,
    request_count: usize,
    methods: Vec<String>,
    ranges: Vec<Option<u64>>,
    cookies: Vec<Option<String>>,
}

/// A mock HTTP server on 127.0.0.1 that serves scripted responses per path
/// (falling back to `fallback` once a path's script is exhausted).
///
/// Each request pops one response from the path's script queue; `HEAD`
/// requests get the response's status and headers but no body, as a real
/// server would send. Cloning shares the same listener and request log.
#[derive(Clone)]
pub struct MockServer {
    base_url: String,
    state: Arc<Mutex<ServerState>>,
}

impl MockServer {
    /// Starts the server; the listener thread is detached and the server
    /// lives until the process exits
    pub fn start(scripts: HashMap<String, VecDeque<MockResponse>>, fallback: MockResponse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind mock server");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(ServerState {
            scripts,
            fallback,
            request_count: 0,
            methods: Vec::new(),
            ranges: Vec::new(),
            cookies: Vec::new(),
        }));
        let handler_state = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                // One thread per connection: a stalled body (which holds
                // its connection open) must not block later requests on
                // the same server
                let state = handler_state.clone();
                thread::spawn(move || handle_connection(stream, &state));
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            state,
        }
    }

    /// URL for the given absolute path (e.g. "/file.bin")
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Starts a server with a single scripted path: the last response
    /// doubles as the fallback once the script is exhausted. Returns the
    /// server and the absolute URL of that path.
    pub fn scripted(path: &str, responses: Vec<MockResponse>) -> (Self, Url) {
        let fallback = responses
            .last()
            .cloned()
            .unwrap_or_else(|| MockResponse::new(404, MockBody::Full(vec![])));
        let mut scripts = HashMap::new();
        scripts.insert(path.to_string(), VecDeque::from(responses));
        let server = Self::start(scripts, fallback);
        let url = Url::parse(&server.url(path)).expect("mock URL must parse");
        (server, url)
    }

    pub fn request_count(&self) -> usize {
        self.state.lock().unwrap().request_count
    }

    /// HTTP methods of the received requests, in order
    pub fn methods(&self) -> Vec<String> {
        self.state.lock().unwrap().methods.clone()
    }

    /// Parsed `Range` header start offsets, in order (`None` when absent)
    pub fn ranges(&self) -> Vec<Option<u64>> {
        self.state.lock().unwrap().ranges.clone()
    }

    /// `Cookie` header values of the received requests, in order
    /// (`None` when the header was absent)
    pub fn cookies(&self) -> Vec<Option<String>> {
        self.state.lock().unwrap().cookies.clone()
    }
}

/// Resolves a `Ranged` body against the request's `Range` header the way a
/// real origin server does, so resume tests verify the client's offset
/// against behaviour rather than script order: a satisfying `bytes=N-`
/// yields a 206 with the tail and a matching `Content-Range`, an
/// unsatisfiable one (`N` at or past the end) yields a 416, and no Range
/// yields the full 200. Returns (status, announced_len, body, stalled,
/// extra_headers).
fn ranged_response(
    data: &[u8],
    range: Option<u64>,
) -> (u16, u64, Vec<u8>, bool, Vec<(String, String)>) {
    let len = data.len() as u64;
    let (status, announced, body, content_range) = match range {
        None => (200, len, data.to_vec(), None),
        Some(start) if start < len => (
            206,
            len - start,
            data[start as usize..].to_vec(),
            Some((
                "Content-Range".to_string(),
                format!("bytes {start}-{end}/{total}", end = len - 1, total = len),
            )),
        ),
        Some(_) => (
            416,
            0,
            Vec::new(),
            Some(("Content-Range".to_string(), format!("bytes */{len}"))),
        ),
    };
    let mut headers = Vec::new();
    if let Some(cr) = content_range {
        headers.push(cr);
    }
    (status, announced, body, false, headers)
}

/// The request details a scripted response depends on: the HTTP method, the
/// absolute path, the `Range` header start offset (None when absent) and
/// the `Cookie` header (None when absent).
fn parse_request_head(head: &str) -> (String, String, Option<u64>, Option<String>) {
    let request_line = head.lines().next().unwrap_or("");
    let method = request_line
        .split_whitespace()
        .next()
        .unwrap_or("GET")
        .to_string();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let range = head
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("range:"))
        .and_then(|line| {
            line.split_once(':')?
                .1
                .trim()
                .strip_prefix("bytes=")?
                .split('-')
                .next()?
                .trim()
                .parse::<u64>()
                .ok()
        });
    let cookie = head
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("cookie:"))
        .and_then(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        });
    (method, path, range, cookie)
}

/// Resolves a scripted response into (status, announced length, body bytes,
/// hold-open, extra headers). A HEAD response carries the status and headers
/// but no body; a Stalled body holds the connection open after the headers;
/// a Ranged body is resolved against the request's Range header like a real
/// origin server; every other body is served exactly as scripted, regardless
/// of any Range header.
fn resolve_response(
    response: &MockResponse,
    method: &str,
    range: Option<u64>,
) -> (u16, u64, Vec<u8>, bool, Vec<(String, String)>) {
    if method.eq_ignore_ascii_case("HEAD") {
        return (
            response.status,
            0,
            Vec::new(),
            false,
            response.extra_headers.clone(),
        );
    }
    match &response.body {
        MockBody::Full(data) => (
            response.status,
            data.len() as u64,
            data.clone(),
            false,
            response.extra_headers.clone(),
        ),
        MockBody::Truncated {
            announced_len,
            partial,
        } => (
            response.status,
            *announced_len,
            partial.clone(),
            false,
            response.extra_headers.clone(),
        ),
        MockBody::Stalled { announced_len } => (
            response.status,
            *announced_len,
            Vec::new(),
            true,
            response.extra_headers.clone(),
        ),
        MockBody::Ranged(data) => {
            // A ranged response's status/Content-Range are derived from the
            // request, but extra headers (Last-Modified, Retry-After, ...)
            // are scripted facts about the server and ride along
            let (status, announced, body, stalled, mut headers) = ranged_response(data, range);
            headers.extend(response.extra_headers.iter().cloned());
            (status, announced, body, stalled, headers)
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &Arc<Mutex<ServerState>>) {
    // Read the request head
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let head = String::from_utf8_lossy(&buf).into_owned();
    let (method, path, range, cookie) = parse_request_head(&head);

    let response = {
        let mut st = state.lock().unwrap();
        st.request_count += 1;
        st.methods.push(method.clone());
        st.ranges.push(range);
        st.cookies.push(cookie);
        st.scripts
            .get_mut(&path)
            .and_then(|q| q.pop_front())
            .unwrap_or_else(|| st.fallback.clone())
    };

    let (status, announced_len, body_bytes, stalled, extra_headers) =
        resolve_response(&response, &method, range);

    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Unknown")
        .to_string();
    let mut out = format!("HTTP/1.1 {} {}\r\n", status, reason);
    out.push_str(&format!("Content-Length: {announced_len}\r\n"));
    for (name, value) in &extra_headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");

    let _ = stream.write_all(out.as_bytes());
    if stalled {
        // Hold the connection open without delivering the body: a stalled
        // server. The sleep is far longer than any test may run.
        thread::sleep(Duration::from_secs(3_600));
    }
    // The stream is dropped here: for Truncated bodies this closes the
    // connection before the announced Content-Length has been delivered.
    let _ = stream.write_all(&body_bytes);
}
