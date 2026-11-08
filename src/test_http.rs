//! A minimal HTTP/1.1 server for exercising the real network paths in tests.
//!
//! `download_to_cache` and `verify_gpg_signature` are only meaningfully tested
//! against a server that actually speaks the protocol: the resume logic turns on
//! `Range`/`If-Range`/`206`/`416` handling, and the signature path turns on
//! what an HTTP error on `SHASUMS256.txt.sig` does. Neither can be reached
//! through a mock without re-implementing the thing under test.
//!
//! Hand-rolled rather than added as a dev-dependency: `deny.toml` allow-lists
//! licenses explicitly (16 entries, no GPL family), so pulling in a test HTTP
//! crate would mean a policy change that costs more than this file.
//!
//! Every test using this must hold `crate::system::ENV_TESTS_MUTEX` — see
//! [`NoProxyGuard`].

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// What the server saw, reduced to the parts tests assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenRequest {
    pub path: String,
    /// Verbatim `Range` header, e.g. `bytes=12-`.
    pub range: Option<String>,
    /// Verbatim `If-Range` header — the validator the client captured earlier.
    pub if_range: Option<String>,
}

/// How the server answers a request carrying `Range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeMode {
    /// Reply `200 OK` with the whole body, ignoring `Range`/`If-Range`. This is
    /// what a CDN that does not implement ranges does, and what the client must
    /// survive without appending the body to a stale `.part`.
    Ignore,
    /// Honour `Range` with `206`, but fall back to `200` with the whole body
    /// when the request's `If-Range` does not match `etag` — the RFC 9110
    /// behaviour that detects an upstream file swap.
    Honor,
    /// Reply `416 Range Not Satisfiable` to a request that carries `Range`,
    /// and serve the whole body to one that does not. That is what a server does
    /// when the resume offset is past the end of its current copy, and it is
    /// what lets `download_to_cache`'s delete-and-retry-from-zero path complete.
    Unsatisfiable,
}

/// How the server answers any request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Range(RangeMode),
    /// Always `404 Not Found`, with no body.
    NotFound,
}

/// Keeps `127.0.0.1` out of any proxy the environment happens to configure.
///
/// reqwest reads `HTTP_PROXY`/`HTTPS_PROXY` by default, so on a machine that
/// exports a proxy for its own network access a loopback request would be
/// handed to that proxy and fail in a way that has nothing to do with the code
/// under test. Restores the previous values on drop.
struct NoProxyGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl NoProxyGuard {
    fn bypass_loopback() -> Self {
        const VARS: [&str; 2] = ["NO_PROXY", "no_proxy"];
        let saved = VARS
            .iter()
            .map(|var| (*var, std::env::var_os(var)))
            .collect();
        for var in VARS {
            std::env::set_var(var, "127.0.0.1,localhost,::1");
        }
        Self { saved }
    }
}

impl Drop for NoProxyGuard {
    fn drop(&mut self) {
        for (var, old) in self.saved.drain(..) {
            match old {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
    }
}

/// A running loopback server. Shuts down when dropped.
pub(crate) struct TestServer {
    origin: String,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    // Held for the server's lifetime so the bypass is in effect for every
    // request made against it, and restored only once it is gone.
    _no_proxy: NoProxyGuard,
}

impl TestServer {
    /// Start a server that answers every path with `body`.
    ///
    /// `etag` is sent on `200` responses so the client captures it into its
    /// `If-Range` sidecar; pass a value that differs from the one a test
    /// pre-seeded to exercise the mismatch path.
    ///
    /// Must be called while holding `ENV_TESTS_MUTEX` (it mutates `NO_PROXY`).
    pub(crate) fn start(body: &'static [u8], etag: &'static str, mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        // Port 0 lets the OS pick, so parallel test runs cannot collide.
        let port = listener.local_addr().expect("local_addr").port();
        listener
            .set_nonblocking(true)
            .expect("listener nonblocking");

        let seen: Arc<Mutex<Vec<SeenRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("test-http-server".into())
                .spawn(move || {
                    for attempt in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        match attempt {
                            Ok(stream) => {
                                let _ = stream.set_nonblocking(false);
                                // Bound the blocking read. A connection that
                                // never sends a request would otherwise stall
                                // `serve` forever, and `Drop` joins this thread
                                // — one stray connection would hang the whole
                                // suite until the CI timeout.
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                                serve(stream, body, etag, mode, &seen);
                            }
                            // A nonblocking listener yields WouldBlock when no
                            // client is waiting; poll instead of blocking so the
                            // stop flag is noticed promptly.
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(_) => break,
                        }
                    }
                })
                .expect("spawn server thread")
        };

        Self {
            origin: format!("http://127.0.0.1:{port}"),
            seen,
            stop,
            worker: Some(worker),
            _no_proxy: NoProxyGuard::bypass_loopback(),
        }
    }

    /// Absolute URL for `path`, which must start with `/`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    /// Every request the server has handled so far, in arrival order.
    pub(crate) fn requests(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Read one request, record it, and write the response.
fn serve(
    stream: TcpStream,
    body: &'static [u8],
    etag: &'static str,
    mode: Mode,
    seen: &Arc<Mutex<Vec<SeenRequest>>>,
) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut range = None;
    let mut if_range = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            // Header names are case-insensitive; compare lowercased.
            match name.trim().to_ascii_lowercase().as_str() {
                "range" => range = Some(value.trim().to_string()),
                "if-range" => if_range = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }

    seen.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(SeenRequest {
            path,
            range: range.clone(),
            if_range: if_range.clone(),
        });

    let mut out = stream;
    let _ = match mode {
        Mode::NotFound => write_empty(&mut out, "404 Not Found"),
        Mode::Range(range_mode) => {
            // A server that cannot satisfy the offset answers 416. The client's
            // retry arrives without a Range header and must be served normally,
            // otherwise the recovery path could never complete.
            if range_mode == RangeMode::Unsatisfiable && range.is_some() {
                write_empty(&mut out, "416 Range Not Satisfiable")
            } else {
                // A `Range` of `bytes=N-` asks for the tail from offset N.
                let offset = range
                    .as_deref()
                    .and_then(parse_range_start)
                    .filter(|&n| n > 0 && (n as usize) < body.len());

                let validator_matches = match (&if_range, range_mode) {
                    // Without If-Range the client cannot detect a swap, and
                    // neither can we — plain Range is honoured, matching the
                    // pre-fix behaviour.
                    (None, _) => true,
                    (Some(v), RangeMode::Honor) => v.trim() == etag,
                    // `Ignore` and `Unsatisfiable` never consult the validator.
                    (Some(_), _) => true,
                };

                match (offset, validator_matches, range_mode) {
                    (Some(n), true, RangeMode::Honor) => {
                        let start = n as usize;
                        let last = body.len() - 1;
                        write_body(
                            &mut out,
                            "206 Partial Content",
                            &body[start..],
                            Some(&format!("bytes {start}-{last}/{}", body.len())),
                            Some(etag),
                        )
                    }
                    // Either no usable Range, or the validator mismatched: the
                    // whole body, which is what tells the client its `.part` is
                    // stale and must be discarded rather than appended to.
                    _ => write_body(&mut out, "200 OK", body, None, Some(etag)),
                }
            }
        }
    };
}

/// Parse the start offset out of `bytes=N-` / `bytes=N-M`.
fn parse_range_start(header: &str) -> Option<u64> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let first = spec.split(['-', ',']).next()?;
    first.trim().parse::<u64>().ok()
}

fn write_empty(out: &mut TcpStream, status: &str) -> std::io::Result<()> {
    write_body(out, status, &[], None, None)
}

fn write_body(
    out: &mut TcpStream,
    status: &str,
    body: &[u8],
    content_range: Option<&str>,
    etag: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    if let Some(cr) = content_range {
        head.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    if let Some(etag) = etag {
        head.push_str(&format!("ETag: {etag}\r\n"));
    }
    // Closing after each response keeps the server single-shot per connection,
    // so a client that pipelines or reuses connections cannot desynchronise it.
    head.push_str("Connection: close\r\n\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(body)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ENV_TESTS_MUTEX;

    /// The server is the fixture every other network test depends on, so it gets
    /// its own smoke test: a plain GET must return the exact bytes with the
    /// status, `Content-Length` and `ETag` the resume logic reads.
    #[test]
    fn server_returns_body_and_records_the_request() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let server = TestServer::start(b"hello-body", "\"v1\"", Mode::Range(RangeMode::Honor));

        let body = fetch(&server.url("/v20.0.0/node.tar.gz"));
        assert_eq!(body, b"hello-body");

        let seen = server.requests();
        assert_eq!(seen.len(), 1, "exactly one request: {seen:?}");
        assert_eq!(seen[0].path, "/v20.0.0/node.tar.gz");
        assert_eq!(seen[0].range, None);
        assert_eq!(seen[0].if_range, None);
    }

    /// `Range`/`If-Range` must be captured verbatim: the resume tests assert on
    /// exactly what the client sent, so a server that normalised or dropped
    /// them would make those assertions vacuous.
    #[test]
    fn server_records_range_and_if_range_headers() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let server = TestServer::start(b"0123456789", "\"v1\"", Mode::Range(RangeMode::Honor));

        assert_eq!(
            partial(&server.url("/f"), Some(4), Some("\"v1\"")),
            b"456789"
        );
        assert_eq!(
            partial(&server.url("/f"), Some(4), Some("\"stale\"")),
            b"0123456789"
        );
        assert_eq!(partial(&server.url("/f"), None, None), b"0123456789");

        let seen = server.requests();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].range.as_deref(), Some("bytes=4-"));
        assert_eq!(seen[0].if_range.as_deref(), Some("\"v1\""));
        assert_eq!(seen[1].if_range.as_deref(), Some("\"stale\""));
        assert_eq!(seen[2].range, None);
    }

    /// A 404 must reach the client as a 404, not as an empty success — the
    /// signature path treats an unreachable `.sig` as a hard error precisely
    /// because a MITM could otherwise strip it.
    #[test]
    fn server_can_return_not_found() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let server = TestServer::start(b"ignored", "\"v1\"", Mode::NotFound);
        assert_eq!(status_of(&server.url("/missing.sig")), 404);
    }

    /// `416` is its own status, not an error: `download_to_cache` catches it,
    /// deletes the stale `.part` and retries from byte 0. The server therefore
    /// has to answer 416 to the ranged request *and* serve the retry.
    #[test]
    fn unsatisfiable_mode_answers_416_only_to_ranged_requests() {
        let _lock = ENV_TESTS_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let server = TestServer::start(b"short", "\"v1\"", Mode::Range(RangeMode::Unsatisfiable));
        let url = server.url("/f");

        let (ranged, _, _) = raw_request(&url, &["Range: bytes=99-"]);
        assert_eq!(ranged, 416, "an offset past the end must be 416");

        let (plain, _, body) = raw_request(&url, &[]);
        assert_eq!(plain, 200, "the retry without Range must be served");
        assert_eq!(body, b"short");
    }

    // --- tiny client helpers -------------------------------------------------
    //
    // Deliberately not reqwest: these tests are about what the server does, and
    // using the crate under test to drive them would hide a client-side bug
    // behind a matching server-side one.

    fn raw_request(url: &str, extra_headers: &[&str]) -> (u16, Vec<String>, Vec<u8>) {
        let stripped = url.strip_prefix("http://").expect("http url");
        let (authority, path) = stripped.split_once('/').expect("host/path");
        let mut stream = TcpStream::connect(authority).expect("connect");
        let mut req = format!("GET /{path} HTTP/1.1\r\nHost: {authority}\r\n");
        for h in extra_headers {
            req.push_str(h);
            req.push_str("\r\n");
        }
        req.push_str("Connection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).expect("write request");
        stream.flush().expect("flush");

        let mut all = Vec::new();
        std::io::Read::read_to_end(&mut stream, &mut all).expect("read response");
        let split = all
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header terminator");
        let head = String::from_utf8_lossy(&all[..split]).to_string();
        let body = all[split + 4..].to_vec();
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .expect("status code");
        (status, lines.map(String::from).collect(), body)
    }

    fn fetch(url: &str) -> Vec<u8> {
        let (status, _, body) = raw_request(url, &[]);
        assert_eq!(status, 200, "expected 200 from {url}");
        body
    }

    fn status_of(url: &str) -> u16 {
        raw_request(url, &[]).0
    }

    /// GET with an optional `Range: bytes=<from>-` and `If-Range` validator.
    fn partial(url: &str, from: Option<u64>, if_range: Option<&str>) -> Vec<u8> {
        let range;
        let mut headers: Vec<&str> = Vec::new();
        if let Some(n) = from {
            range = format!("Range: bytes={n}-");
            headers.push(&range);
        }
        let ifr;
        if let Some(v) = if_range {
            ifr = format!("If-Range: {v}");
            headers.push(&ifr);
        }
        let (status, _, body) = raw_request(url, &headers);
        assert!(
            status == 200 || status == 206,
            "expected 200/206 from {url}, got {status}"
        );
        body
    }
}
