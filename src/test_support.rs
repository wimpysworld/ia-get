#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use ia_get::archive_metadata::XmlFile;
use ia_get::downloader::{RetryPolicy, ShutdownSignal};
use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;

/// Isolated temporary directory removed automatically on drop.
pub struct TempTestDir {
    dir: tempfile::TempDir,
}

impl TempTestDir {
    pub fn new(prefix: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("ia-get-test-{prefix}-"))
            .tempdir()
            .expect("create temp test dir");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn part_path(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}.part"))
    }

    pub fn write(&self, rel: impl AsRef<Path>, data: impl AsRef<[u8]>) -> PathBuf {
        let full = self.dir.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dirs in temp test dir");
        }
        fs::write(&full, data).expect("write test fixture file");
        full
    }
}

/// Thread-safe in-memory writer for capturing `VerboseLogger` output.
#[derive(Clone, Default)]
pub struct CapturedLogs {
    bytes: Arc<StdMutex<Vec<u8>>>,
}

impl CapturedLogs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clone_writer(&self) -> Self {
        self.clone()
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        self.bytes.lock().unwrap().clone()
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.as_bytes()).expect("verbose logs must be valid UTF-8")
    }

    pub fn lines(&self) -> Vec<String> {
        self.text().lines().map(str::to_string).collect()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn md5_hex(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

pub fn xml_file(name: &str, size: Option<u64>, md5: Option<&str>) -> XmlFile {
    XmlFile {
        name: name.to_string(),
        size,
        md5: md5.map(str::to_string),
    }
}

pub fn build_xml_metadata(entries: &[(&str, u64, &str)]) -> String {
    let mut xml = String::from("<files>\n");
    for (name, size, digest) in entries {
        xml.push_str(&format!(
            "  <file name=\"{name}\" source=\"original\">\n    <size>{size}</size>\n    <md5>{digest}</md5>\n  </file>\n"
        ));
    }
    xml.push_str("</files>");
    xml
}

#[derive(Debug, PartialEq, Eq)]
pub struct DirEntrySnapshot {
    pub rel_path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub contents: Option<Vec<u8>>,
    pub modified: Option<SystemTime>,
}

/// Captures the root directory's metadata and all recursive entries for read-only assertions.
pub fn snapshot_dir(root: &Path) -> Vec<DirEntrySnapshot> {
    fn walk(base: &Path, current: &Path, out: &mut Vec<DirEntrySnapshot>) {
        let mut entries: Vec<_> = fs::read_dir(current)
            .expect("read dir for snapshot")
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        entries.sort();

        for path in entries {
            let meta = fs::symlink_metadata(&path).expect("symlink_metadata for snapshot");
            let rel_path = path.strip_prefix(base).unwrap().to_path_buf();
            let is_dir = meta.file_type().is_dir();
            let is_symlink = meta.file_type().is_symlink();
            let contents = if meta.file_type().is_file() {
                Some(fs::read(&path).expect("read file for snapshot"))
            } else {
                None
            };
            let modified = meta.modified().ok();
            out.push(DirEntrySnapshot {
                rel_path,
                is_dir,
                is_symlink,
                contents,
                modified,
            });
            if is_dir && !is_symlink {
                walk(base, &path, out);
            }
        }
    }

    let root_meta = fs::symlink_metadata(root).expect("root metadata for snapshot");
    let mut out = vec![DirEntrySnapshot {
        rel_path: PathBuf::new(),
        is_dir: root_meta.file_type().is_dir(),
        is_symlink: root_meta.file_type().is_symlink(),
        contents: None,
        modified: root_meta.modified().ok(),
    }];
    walk(root, root, &mut out);
    out
}

pub fn fast_retry_policy(max_retries: u32) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(10),
        max_retry_after: Duration::from_millis(50),
    }
}

pub fn slow_retry_policy(max_retries: u32) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        initial_delay: Duration::from_secs(10),
        max_delay: Duration::from_secs(30),
        max_retry_after: Duration::from_secs(60),
    }
}

#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
}

impl CapturedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub enum MockResponse {
    Static {
        status_line: &'static str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    MidStreamInterrupt {
        status_line: &'static str,
        headers: Vec<(String, String)>,
        first_chunk: Vec<u8>,
        second_chunk: Option<Vec<u8>>,
        watch_path: PathBuf,
        expected_part_len: u64,
        signal: ShutdownSignal,
    },
}

impl MockResponse {
    pub fn ok(body: &[u8]) -> Self {
        Self::status("200 OK", body)
    }

    pub fn status(status_line: &'static str, body: &[u8]) -> Self {
        Self::Static {
            status_line,
            headers: vec![("Content-Length".to_string(), body.len().to_string())],
            body: body.to_vec(),
        }
    }

    pub fn with_headers(
        status_line: &'static str,
        headers: Vec<(&str, String)>,
        body: &[u8],
    ) -> Self {
        Self::Static {
            status_line,
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            body: body.to_vec(),
        }
    }

    pub fn redirect(status_line: &'static str, location: impl Into<String>) -> Self {
        Self::with_headers(
            status_line,
            vec![
                ("Location", location.into()),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )
    }

    pub fn retry_after(
        status_line: &'static str,
        retry_after: impl Into<String>,
        body: &[u8],
    ) -> Self {
        Self::with_headers(
            status_line,
            vec![
                ("Retry-After", retry_after.into()),
                ("Content-Length", body.len().to_string()),
            ],
            body,
        )
    }

    pub fn range_416(total_len: u64) -> Self {
        Self::with_headers(
            "416 Range Not Satisfiable",
            vec![
                ("Content-Range", format!("bytes */{total_len}")),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )
    }

    pub fn partial_content(start: u64, total_len: u64, chunk: &[u8]) -> Self {
        assert!(!chunk.is_empty(), "partial_content chunk must be non-empty");
        let end = start + chunk.len() as u64 - 1;
        Self::with_headers(
            "206 Partial Content",
            vec![
                ("Content-Range", format!("bytes {start}-{end}/{total_len}")),
                ("Content-Length", chunk.len().to_string()),
            ],
            chunk,
        )
    }
}

pub struct MockServer {
    host: &'static str,
    addr: SocketAddr,
    requests: Arc<AsyncMutex<Vec<CapturedRequest>>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl MockServer {
    pub async fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        Self::start_with_listener(listener, "127.0.0.1", responses)
    }

    pub async fn start_on_localhost(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind(("localhost", 0))
            .await
            .expect("bind localhost mock server");
        Self::start_with_listener(listener, "localhost", responses)
    }

    fn start_with_listener(
        listener: TcpListener,
        host: &'static str,
        responses: Vec<MockResponse>,
    ) -> Self {
        let addr = listener.local_addr().expect("mock server local_addr");
        let requests = Arc::new(AsyncMutex::new(Vec::new()));
        let requests_clone = Arc::clone(&requests);

        let handle = tokio::spawn(async move {
            for response in responses {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let _ = socket.set_nodelay(true);
                let is_head = match read_http_request(&mut socket).await {
                    Some(req) => {
                        let is_head = req.request_line.starts_with("HEAD ");
                        requests_clone.lock().await.push(req);
                        is_head
                    }
                    None => false,
                };
                write_mock_response(&mut socket, response, is_head).await;
            }
        });

        Self {
            host,
            addr,
            requests,
            handle: Some(handle),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}:{}{}", self.host, self.addr.port(), path)
    }

    pub fn parsed_url(&self, path: &str) -> Url {
        Url::parse(&self.url(path)).expect("valid mock URL")
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn requests_handle(&self) -> Arc<AsyncMutex<Vec<CapturedRequest>>> {
        Arc::clone(&self.requests)
    }

    pub async fn finish(mut self) -> Vec<CapturedRequest> {
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }
        self.requests.lock().await.clone()
    }

    pub async fn abort(mut self) -> Vec<CapturedRequest> {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        self.requests.lock().await.clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

pub type ConcurrentHandler = Arc<
    dyn Fn(CapturedRequest) -> Pin<Box<dyn Future<Output = MockResponse> + Send>> + Send + Sync,
>;

/// Mock HTTP server that handles each accepted connection concurrently in its own Tokio task
/// while recording all requests and peak simultaneous in-flight request count.
pub struct ConcurrentMockServer {
    addr: SocketAddr,
    requests: Arc<AsyncMutex<Vec<CapturedRequest>>>,
    max_concurrent: Arc<AtomicUsize>,
    accept_handle: Option<tokio::task::JoinHandle<()>>,
    connection_handles: Arc<AsyncMutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl ConcurrentMockServer {
    pub async fn start<F, Fut>(handler: F) -> Self
    where
        F: Fn(CapturedRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = MockResponse> + Send + 'static,
    {
        let handler: ConcurrentHandler = Arc::new(move |req| Box::pin(handler(req)));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind concurrent mock server");
        let addr = listener.local_addr().expect("concurrent server local_addr");
        let requests = Arc::new(AsyncMutex::new(Vec::new()));
        let requests_clone = Arc::clone(&requests);
        let active = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));
        let max_clone = Arc::clone(&max_concurrent);
        let connection_handles = Arc::new(AsyncMutex::new(Vec::new()));
        let handles_clone = Arc::clone(&connection_handles);

        let accept_handle = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let handler = Arc::clone(&handler);
                let requests = Arc::clone(&requests_clone);
                let active = Arc::clone(&active);
                let max_concurrent = Arc::clone(&max_clone);
                let conn_handle = tokio::spawn(async move {
                    let _ = socket.set_nodelay(true);
                    let Some(req) = read_http_request(&mut socket).await else {
                        return;
                    };
                    let is_head = req.request_line.starts_with("HEAD ");
                    requests.lock().await.push(req.clone());

                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(current, Ordering::SeqCst);

                    let response = handler(req).await;
                    write_mock_response(&mut socket, response, is_head).await;

                    active.fetch_sub(1, Ordering::SeqCst);
                });
                handles_clone.lock().await.push(conn_handle);
            }
        });

        Self {
            addr,
            requests,
            max_concurrent,
            accept_handle: Some(accept_handle),
            connection_handles,
        }
    }

    /// Starts a concurrent mock server serving queued responses keyed by request path.
    pub async fn from_routes(routes: Vec<(&str, Vec<MockResponse>)>) -> Self {
        let mut map: HashMap<String, VecDeque<MockResponse>> = HashMap::new();
        for (path, responses) in routes {
            map.insert(path.to_string(), VecDeque::from(responses));
        }
        let table = Arc::new(StdMutex::new(map));
        Self::start(move |req| {
            let table = Arc::clone(&table);
            async move {
                let path = req
                    .request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let mut guard = table.lock().unwrap();
                guard
                    .get_mut(&path)
                    .and_then(VecDeque::pop_front)
                    .unwrap_or_else(|| MockResponse::status("404 Not Found", b"route not found"))
            }
        })
        .await
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.addr.port(), path)
    }

    pub fn parsed_url(&self, path: &str) -> Url {
        Url::parse(&self.url(path)).expect("valid concurrent mock URL")
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent.load(Ordering::SeqCst)
    }

    pub async fn stop(mut self) -> Vec<CapturedRequest> {
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        let handles: Vec<_> = self.connection_handles.lock().await.drain(..).collect();
        for handle in handles {
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        }
        self.requests.lock().await.clone()
    }
}

impl Drop for ConcurrentMockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
        }
    }
}

async fn read_http_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
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
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    Some(CapturedRequest {
        request_line,
        headers,
    })
}

fn format_response_head(status_line: &str, headers: &[(String, String)]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status_line}\r\n");
    let mut has_connection = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if !has_connection {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    head.into_bytes()
}

async fn write_mock_response(socket: &mut TcpStream, response: MockResponse, is_head: bool) {
    match response {
        MockResponse::Static {
            status_line,
            headers,
            body,
        } => {
            let mut wire = format_response_head(status_line, &headers);
            if !is_head {
                wire.extend_from_slice(&body);
            }
            let _ = socket.write_all(&wire).await;
            let _ = socket.flush().await;
            let _ = socket.shutdown().await;
        }
        MockResponse::MidStreamInterrupt {
            status_line,
            headers,
            first_chunk,
            second_chunk,
            watch_path,
            expected_part_len,
            signal,
        } => {
            let mut wire = format_response_head(status_line, &headers);
            wire.extend_from_slice(&first_chunk);
            let _ = socket.write_all(&wire).await;
            let _ = socket.flush().await;

            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline && signal.is_running() {
                if fs::metadata(&watch_path)
                    .map(|m| m.len() >= expected_part_len)
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            signal.trigger();

            if let Some(chunk) = second_chunk {
                let _ = socket.write_all(&chunk).await;
                let _ = socket.flush().await;
            }
            let _ = socket.shutdown().await;
        }
    }
}
