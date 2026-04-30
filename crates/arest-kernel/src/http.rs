// crates/arest-kernel/src/http.rs
//
// Minimal HTTP/1.1 server layered on smoltcp TCP sockets (#264).
//
// Not a general-purpose server. Not hyper. The contract is:
//
//   1. `register(port, handler)` binds a TCP listen socket at boot.
//   2. `poll()` runs inside the kernel's idle / timer loop. When a
//      complete request has arrived on the socket, the handler is
//      invoked with (method, path, body) and its response is
//      serialised straight onto the socket.
//   3. Only one connection is served at a time — the listen socket
//      is re-armed after the response closes. Good enough for a
//      bare-metal "hello world" website; multi-connection support
//      is a follow-up (#264 extension).
//
// HTTP/1.1 semantics supported:
//   • Request line: `METHOD SP path SP HTTP/1.1 CRLF`
//   • Headers until CRLF CRLF. Parsed but mostly ignored; only
//     Content-Length is honoured for request-body framing.
//   • Response: `HTTP/1.1 {status} {reason} CRLF Content-Type: ... CRLF
//     Content-Length: N CRLF Connection: close CRLF CRLF {body}`
//   • Connection: close — we close after every response so smoltcp's
//     socket half-closes cleanly and the listen socket re-arms.
//
// All parsing allocates (Vec<u8> buffers) because smoltcp's TCP
// sockets deliver bytes in arbitrary chunks.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// HTTP status code + default reason phrase. Extend as needed.
#[derive(Debug, Clone, Copy)]
pub struct Status(pub u16);

impl Status {
    pub fn reason(self) -> &'static str {
        match self.0 {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            503 => "Service Unavailable",
            _ => "Unknown",
        }
    }
}

/// A decoded HTTP/1.1 request. Buffered in full before the handler
/// runs — no streaming body access.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    /// Parsed Accept header value if present (so handlers can do
    /// content negotiation without re-parsing). None when absent.
    pub accept: Option<String>,
}

/// Response the handler returns. Content-Type is always emitted;
/// `Cache-Control` is optional so API responses (no sensible cache
/// policy at this layer) stay header-light while static assets
/// (#266) emit the right directive per path.
///
/// `retry_after` (#620 / HATEOAS-6b) carries an opaque pointer string
/// for the `Retry-After` header. Per RFC 7231 §7.1.3 this is normally
/// a delay-seconds or HTTP-date, but the kernel's 503-on-no-LLM-body
/// envelope re-purposes it as a worker URL the caller can re-issue
/// the request against — same intent ("come back here when this
/// request can succeed"), wider target. Optional so legacy responses
/// stay header-light.
#[derive(Debug)]
pub struct Response {
    pub status: Status,
    pub content_type: &'static str,
    pub cache_control: Option<&'static str>,
    pub retry_after: Option<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: Status(200),
            content_type,
            cache_control: None,
            retry_after: None,
            body,
        }
    }

    /// `ok` with an explicit Cache-Control directive — used by the
    /// static-asset handler (#266) to mark immutable bundles and
    /// no-cache the HTML shell.
    pub fn ok_cached(
        content_type: &'static str,
        cache_control: &'static str,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status: Status(200),
            content_type,
            cache_control: Some(cache_control),
            retry_after: None,
            body,
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: Status(404),
            content_type: "text/plain",
            cache_control: None,
            retry_after: None,
            body: b"Not Found\n".to_vec(),
        }
    }

    pub fn bad_request(msg: &str) -> Self {
        Self {
            status: Status(400),
            content_type: "text/plain",
            cache_control: None,
            retry_after: None,
            body: msg.as_bytes().to_vec(),
        }
    }

    pub fn internal_error(msg: &str) -> Self {
        Self {
            status: Status(500),
            content_type: "text/plain",
            cache_control: None,
            retry_after: None,
            body: msg.as_bytes().to_vec(),
        }
    }

    /// 503 Service Unavailable with a `Retry-After` header carrying an
    /// opaque pointer (#620 / HATEOAS-6b). Used by the
    /// `POST /arest/extract` handler to surface "the verb is
    /// registered but no LLM body is installed in this profile;
    /// re-issue against `<retry_after>`". Body is JSON envelope bytes
    /// the caller renders out of the introspectable Agent Definition
    /// metadata.
    pub fn service_unavailable_with_retry_after(
        content_type: &'static str,
        retry_after: String,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status: Status(503),
            content_type,
            cache_control: None,
            retry_after: Some(retry_after),
            body,
        }
    }

    /// Serialise `status + headers + body` into the wire format
    /// smoltcp's TCP send ring expects. Allocates once and returns
    /// the full buffer so the caller can hand it straight to
    /// `TcpSocket::send_slice`.
    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(192 + self.body.len());
        write_fmt(&mut out, format_args!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n",
            self.status.0,
            self.status.reason(),
            self.content_type,
            self.body.len(),
        ));
        if let Some(cache) = self.cache_control {
            write_fmt(&mut out, format_args!("Cache-Control: {}\r\n", cache));
        }
        if let Some(retry_after) = &self.retry_after {
            write_fmt(&mut out, format_args!("Retry-After: {}\r\n", retry_after));
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

/// Internal fmt helper — writes into a Vec<u8> via core::fmt.
fn write_fmt(buf: &mut Vec<u8>, args: core::fmt::Arguments<'_>) {
    struct VecWriter<'a>(&'a mut Vec<u8>);
    impl core::fmt::Write for VecWriter<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0.extend_from_slice(s.as_bytes());
            Ok(())
        }
    }
    let _ = core::fmt::Write::write_fmt(&mut VecWriter(buf), args);
}

/// Parse a buffered HTTP/1.1 request. Returns `Ok(Some(req))` when
/// the full request (headers + body per Content-Length) is present,
/// `Ok(None)` when more bytes are needed, `Err(msg)` on malformed
/// input.
pub fn parse_request(buf: &[u8]) -> Result<Option<Request>, &'static str> {
    // Find the end of the header block (CRLF CRLF).
    let header_end = match find_double_crlf(buf) {
        Some(i) => i,
        None => return Ok(None),
    };
    let head = &buf[..header_end];
    let head_str = core::str::from_utf8(head).map_err(|_| "non-utf8 headers")?;

    let mut lines = head_str.split("\r\n");
    let request_line = lines.next().ok_or("missing request line")?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or("missing method")?.to_string();
    let path = parts.next().ok_or("missing path")?.to_string();
    let version = parts.next().ok_or("missing version")?;
    if !version.starts_with("HTTP/1.") {
        return Err("not HTTP/1.x");
    }

    // Parse headers we care about.
    let mut content_length: usize = 0;
    let mut accept: Option<String> = None;
    for line in lines {
        if line.is_empty() { continue; }
        let colon = line.find(':').ok_or("header missing ':'")?;
        let name = line[..colon].trim();
        let value = line[colon + 1..].trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().map_err(|_| "bad Content-Length")?;
        } else if name.eq_ignore_ascii_case("accept") {
            accept = Some(value.to_string());
        }
    }

    let body_start = header_end + 4;
    if buf.len() < body_start + content_length {
        return Ok(None); // need more bytes
    }
    let body = buf[body_start..body_start + content_length].to_vec();

    Ok(Some(Request { method, path, body, accept }))
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    // Look for `\r\n\r\n` anywhere in buf, return index of first `\r`.
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Handler signature: request in, response out. No state — all
/// state reaches the handler through `crate::system_impl` (wired in
/// #265).
pub type Handler = fn(&Request) -> Response;

/// Sanity test of the parser + response formatter for a GET without
/// a body. Not a real unit test (no std test harness on bare metal),
/// but the function is reachable from the REPL for manual checks.
#[allow(dead_code)]
pub fn self_test() -> Result<(), &'static str> {
    let wire = b"GET /api/Noun HTTP/1.1\r\nHost: arest\r\nAccept: application/json\r\n\r\n";
    let req = parse_request(wire)?.ok_or("parser returned None")?;
    if req.method != "GET" || req.path != "/api/Noun" {
        return Err("wrong method or path");
    }
    if req.accept.as_deref() != Some("application/json") {
        return Err("Accept header not captured");
    }
    let resp = Response::ok("text/plain", b"hi\n".to_vec());
    let wire = resp.to_wire();
    if !wire.starts_with(b"HTTP/1.1 200 OK\r\n") {
        return Err("response status line malformed");
    }
    Ok(())
}
