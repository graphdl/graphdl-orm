// crates/arest/src/platform/http_fetch.rs
//
// `http_fetch` — the canonical §5.2 httpFetch effect (pb-effect-fns-
// canonical): (ρ fact):httpFetch → response, as a Platform fn body.
//
// The DEFS binding has existed since `register:http_fetch` landed
// (lib.rs system_register tests); this module supplies the body that
// was noted there as "a callback-layer concern for a follow-up". The
// transport is the SAME synchronous rustls/TcpStream HTTP/1.1 client
// the task-919 SM-dispatch callback branch uses
// (`command::http_request`) — 5 s deadlines, 64 KB response cap,
// http:// and https:// only.
//
// Operand shapes (either):
//   <url-atom>                                      → GET url
//   < <'url', u>, <'method', m>?, <'body', b>?,
//     <'headers', <<name, value>, ...>>? >          → full form
//
// Result: < <'status', '200'>, <'body', '...'> > — the §5.2 "response".
// Object::Bottom on malformed operand, unsupported scheme, or network
// failure (apply() totality; a Verb-dispatched fetch that bottoms rolls
// the SM transition back, per the transition_via_defs discipline).

#![cfg(all(not(target_arch = "wasm32"), not(target_os = "uefi")))]

use crate::ast::{self, Object};
use crate::sync::Arc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Register the `http_fetch` body. Pre-approved in
/// `ast::APPROVED_PLATFORM_FN_NAMES` (sec-2: outbound network reach,
/// bounded by the transport's deadlines + caps). Installed at CLI boot
/// beside the renderer; the cloudflare worker installs its own
/// fetch()-backed body under the same name instead.
pub fn install() {
    let f: ast::PlatformFn = Arc::new(|x: &Object, d: &Object| http_fetch_apply(x, d));
    ast::install_platform_fn("http_fetch", f);
}

fn http_fetch_apply(x: &Object, _d: &Object) -> Object {
    let Some((method, url, body, headers)) = decode_operand(x) else {
        return Object::Bottom;
    };
    match crate::command::http_request(&method, &url, body.as_bytes(), &headers) {
        Ok((status, resp_body)) => Object::seq(alloc::vec![
            Object::seq(alloc::vec![
                Object::atom("status"), Object::atom(&status.to_string()),
            ]),
            Object::seq(alloc::vec![
                Object::atom("body"), Object::atom(&resp_body),
            ]),
        ]),
        Err(_) => Object::Bottom,
    }
}

/// Decode the operand into (method, url, body, headers). None ⇒ Bottom.
fn decode_operand(x: &Object) -> Option<(String, String, String, Vec<(String, String)>)> {
    // Bare atom: GET <url>.
    if let Some(u) = x.as_atom() {
        return Some(("GET".to_string(), u.to_string(), String::new(), Vec::new()));
    }
    // Tagged form.
    let sections = x.as_seq()?;
    let field = |tag: &str| -> Option<Object> {
        sections.iter().find_map(|s| {
            let pair = s.as_seq()?;
            (pair.first()?.as_atom()? == tag).then(|| pair.get(1).cloned())?
        })
    };
    let url = field("url")?.as_atom()?.to_string();
    let method = field("method").and_then(|m| m.as_atom().map(str::to_string))
        .unwrap_or_else(|| "GET".to_string());
    let body = field("body").and_then(|b| b.as_atom().map(str::to_string))
        .unwrap_or_default();
    let headers = field("headers").and_then(|h| h.as_seq().map(|rows| {
        rows.iter().filter_map(|r| {
            let pair = r.as_seq()?;
            Some((pair.first()?.as_atom()?.to_string(),
                  pair.get(1)?.as_atom()?.to_string()))
        }).collect()
    })).unwrap_or_default();
    Some((method, url, body, headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Spin a one-shot local HTTP server returning a canned response;
    /// prove the effect round-trips method, status, AND body — the
    /// part `http_post_callback` discards.
    #[test]
    fn fetch_returns_status_and_body_from_live_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                  Connection: close\r\n\r\n{\"ok\":true}").ok();
            req
        });

        let operand = Object::seq(alloc::vec![
            Object::seq(alloc::vec![
                Object::atom("url"),
                Object::atom(&format!("http://127.0.0.1:{}/probe", port)),
            ]),
            Object::seq(alloc::vec![
                Object::atom("method"), Object::atom("POST"),
            ]),
            Object::seq(alloc::vec![
                Object::atom("body"), Object::atom("{\"q\":1}"),
            ]),
        ]);
        let out = http_fetch_apply(&operand, &Object::Bottom);
        let req_seen = server.join().expect("server thread");

        assert!(req_seen.starts_with("POST /probe HTTP/1.1"),
            "method+path must reach the wire; saw: {}", req_seen.lines().next().unwrap_or(""));
        let sections = out.as_seq().expect("tagged response seq");
        let get = |tag: &str| sections.iter().find_map(|s| {
            let p = s.as_seq()?;
            (p.first()?.as_atom()? == tag)
                .then(|| p.get(1)?.as_atom().map(str::to_string))?
        });
        assert_eq!(get("status").as_deref(), Some("200"));
        assert_eq!(get("body").as_deref(), Some("{\"ok\":true}"),
            "response body must be captured, not discarded");
    }

    /// Malformed operands and unreachable schemes bottom out.
    #[test]
    fn malformed_or_unsupported_returns_bottom() {
        assert_eq!(http_fetch_apply(&Object::seq(alloc::vec![]), &Object::Bottom),
            Object::Bottom, "no url field");
        assert_eq!(http_fetch_apply(&Object::atom("ftp://nope"), &Object::Bottom),
            Object::Bottom, "unsupported scheme");
    }

    /// install() + dispatch through the real Platform-apply path.
    #[test]
    fn installed_body_dispatches_via_platform_apply() {
        install();
        let out = ast::apply(
            &ast::Func::Platform("http_fetch".to_string()),
            &Object::atom("not-a-url"),
            &Object::Bottom,
        );
        assert_eq!(out, Object::Bottom, "bad url bottoms through dispatch too");
    }
}
