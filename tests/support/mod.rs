#![allow(
    dead_code,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "shared integration-test harness: panics are the assertion mechanism and each test binary uses a subset"
)]

//! Minimal deterministic HTTP/1.1 mock server for integration tests.
//!
//! In-house on purpose (`dependency-logic`): no async runtime, no test
//! dependency. It records every request so tests can assert not only response
//! handling but the request shape we send upstream, and it can serve a sequence
//! of responses per route (needed to prove cookie rotation behaviour).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Artificial latency, so timeout handling can be exercised deterministically.
    pub delay: Option<Duration>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }

    pub fn ok(body: impl Into<String>) -> Self {
        Self::new(200).with_body(body)
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::ok(body).with_header("content-type", "application/json")
    }

    pub fn redirect(location: impl Into<String>) -> Self {
        Self::new(302).with_header("location", location)
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn with_cookie(self, cookie: impl Into<String>) -> Self {
        self.with_header("set-cookie", cookie)
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

/// A unique, cleaned-up-later temp directory for a test.
pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ao3i-test-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Serves a router on an ephemeral port; returns base URL and the task handle.
pub async fn serve(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind app");
    let addr = listener.local_addr().expect("app addr");
    let handle = tokio::spawn(async move {
        drop(axum::serve(listener, app).await);
    });
    (format!("http://{addr}"), handle)
}

pub mod harness;

type Routes = HashMap<String, Vec<Response>>;

pub struct Server {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    counters: Arc<Mutex<HashMap<String, usize>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    /// A server with no routes; every request gets 404.
    pub fn empty() -> Self {
        Self::start(Vec::<(String, Response)>::new())
    }

    /// One response per route key (`"METHOD /path"`).
    pub fn start<K: Into<String>>(routes: Vec<(K, Response)>) -> Self {
        let sequenced = routes
            .into_iter()
            .map(|(key, response)| (key.into(), vec![response]))
            .collect::<Vec<(String, Vec<Response>)>>();
        Self::start_sequence(sequenced)
    }

    /// A sequence of responses per route; the last one repeats.
    pub fn start_sequence<K: Into<String>>(routes: Vec<(K, Vec<Response>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let routes: Arc<Routes> = Arc::new(
            routes
                .into_iter()
                .map(|(key, responses)| (key.into(), responses))
                .collect(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let counters = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let requests = Arc::clone(&requests);
            let counters = Arc::clone(&counters);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                listener
                    .set_nonblocking(true)
                    .expect("non-blocking listener");
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            drop(serve_connection(stream, &routes, &counters, &requests));
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })
        };

        Self {
            addr,
            requests,
            counters,
            stop,
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("request log poisoned").clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop out of its non-blocking poll.
        drop(TcpStream::connect(self.addr));
        if let Some(handle) = self.handle.take() {
            drop(handle.join());
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    routes: &Routes,
    counters: &Mutex<HashMap<String, usize>>,
    requests: &Mutex<Vec<Request>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let Some(request) = read_request(&mut stream)? else {
        return Ok(());
    };
    let key = format!("{} {}", request.method, request.path);
    let response = next_response(routes, counters, &key).unwrap_or_else(|| Response::new(404));
    requests
        .lock()
        .map_err(|_| std::io::Error::other("request log poisoned"))?
        .push(request);
    if let Some(delay) = response.delay {
        thread::sleep(delay);
    }
    write_response(&mut stream, &response)
}

fn next_response(
    routes: &Routes,
    counters: &Mutex<HashMap<String, usize>>,
    key: &str,
) -> Option<Response> {
    let responses = routes.get(key)?;
    let mut counters = counters.lock().ok()?;
    let index = counters.entry(key.to_string()).or_insert(0);
    let response = responses
        .get(*index)
        .or_else(|| responses.last())
        .cloned()
        .unwrap_or_default();
    *index += 1;
    Some(response)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    let headers_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position;
        }
        if buffer.len() > 64 * 1024 {
            return Ok(None);
        }
    };

    let head = String::from_utf8_lossy(&buffer[..headers_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();

    let content_length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = headers_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body_end = (body_start + content_length).min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[body_start..body_end]).into_owned();

    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        302 => "Found",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", response.status);
    head.push_str("connection: close\r\n");
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let announced = response
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    if !announced {
        head.push_str(&format!("content-length: {}\r\n", response.body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(response.body.as_bytes())?;
    stream.flush()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
