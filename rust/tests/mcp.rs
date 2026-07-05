//! The MCP binding, end to end: spawn the kernel with --mcp --apps-dir over
//! the fixture registry (tests/fixtures/apps, one app per subdirectory
//! carrying its <name>.store.json sidecar), speak newline-delimited JSON-RPC
//! 2.0 over stdio, and prove the daily-driver surface: initialize echoes the
//! client's protocol version, a notification answers nothing, tools/list
//! names the tool table, apps_use boots the fixture store through the serve
//! ingestion path, and the read verbs answer over the retained store. One
//! JSON object per line each way, as the resident protocol runs.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

struct Mcp {
    child: Child,
    rx: Receiver<String>,
}

impl Mcp {
    fn spawn() -> Mcp {
        let apps_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/apps");
        let mut child = Command::new(env!("CARGO_BIN_EXE_arestlam"))
            .arg("--mcp")
            .arg("--apps-dir")
            .arg(apps_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arestlam --mcp");
        // A reader thread feeds a channel so a missing reply fails the test
        // by timeout instead of hanging it.
        let out = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in out.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Mcp { child, rx }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self) -> String {
        self.rx
            .recv_timeout(Duration::from_secs(30))
            .expect("no MCP reply within 30s (the mode must answer one line per request)")
    }

    fn rpc(&mut self, line: &str) -> String {
        self.send(line);
        self.recv()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take()); // EOF ends the loop
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_mode_serves_the_apps_registry_over_stdio() {
    let mut c = Mcp::spawn();

    // ---- initialize: echo the client's protocolVersion, name the server.
    //      The params carry a boolean, which a real MCP client always sends
    //      and the case protocol never does. ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"#,
            r#""protocolVersion":"2025-06-18","#,
            r#""capabilities":{"roots":{"listChanged":true}},"#,
            r#""clientInfo":{"name":"itest","version":"0"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","#,
            r#""capabilities":{"tools":{}},"#,
            r#""serverInfo":{"name":"arestlam","version":"0.1.0"}}}"#
        )
    );

    // ---- a notification (no id) answers nothing: the next line on the wire
    //      must answer the next request ----
    c.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let r = c.rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(r.contains(r#""id":2"#), "the notification must produce no line: {r}");
    for tool in ["orient", "apps_list", "apps_current", "apps_use", "query", "cells", "synthesize"] {
        assert!(r.contains(&format!(r#""name":"{tool}""#)), "missing tool {tool}: {r}");
    }
    assert!(r.contains(r#""inputSchema":{"type":"object""#), "{r}");
    assert!(r.contains(r#""required":["fact_type"]"#), "{r}");

    // ---- before apps_use the read verbs answer a clear error ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":"#,
            r#"{"name":"apps_current","arguments":{}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","#,
            r#""text":"{\"current\":null}"}]}}"#
        )
    );
    // A malformed line is skipped (it carries no recoverable id); the loop
    // lives to answer the next request.
    c.send("{this is not json");
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":"#,
            r#"{"name":"query","arguments":{"fact_type":"Ticket_status"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32602,"#,
            r#""message":"no app loaded; call apps_use before query"}}"#
        )
    );

    // ---- the apps registry: list, use, current, orient ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":"#,
            r#"{"name":"apps_list","arguments":{}}}"#
        )),
        r#"{"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"[\"flow\"]"}]}}"#
    );
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":"#,
            r#"{"name":"apps_use","arguments":{"name":"flow"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":6,"result":{"content":[{"type":"text","#,
            r#""text":"{\"app\":\"flow\",\"ok\":true}"}]}}"#
        )
    );
    // apps_current answers without an arguments key at all.
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":"#,
            r#"{"name":"apps_current"}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","#,
            r#""text":"{\"current\":\"flow\"}"}]}}"#
        )
    );
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":"#,
            r#"{"name":"orient","arguments":{}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"text","#,
            r#""text":"{\"apps\":[\"flow\"],\"current\":\"flow\"}"}]}}"#
        )
    );

    // ---- the read verbs over the retained store (the fixture's real rows) ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":"#,
            r#"{"name":"cells","arguments":{"pattern":"ticket"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":9,"result":{"content":[{"type":"text","#,
            r#""text":"{\"cells\":[{\"name\":\"Ticket\",\"rows\":1},"#,
            r#"{\"name\":\"Ticket:t1\",\"rows\":3},"#,
            r#"{\"name\":\"Ticket_has_Note_uc\",\"rows\":5},"#,
            r#"{\"name\":\"Ticket_has_Status\",\"rows\":1},"#,
            r#"{\"name\":\"Ticket_has_Status_uc\",\"rows\":5}]}"}]}}"#
        )
    );
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":"#,
            r#"{"name":"query","arguments":{"fact_type":"Ticket_has_Status"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":10,"result":{"content":[{"type":"text","#,
            r#""text":"{\"fact_type\":\"Ticket_has_Status\",\"rows\":[[\"t1\",\"open\"]]}"}]}}"#
        )
    );
    // synthesize routes to the resident synthesize_pairs op. The canonical
    // verbalize dispatches populations through the rmapColumns layout cell,
    // so the absorbed fact type's row answers as a real reading pair.
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":"#,
            r#"{"name":"synthesize","arguments":{"id":"t1"}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":11,"result":{"content":[{"type":"text","#,
            r#""text":"{\"id\":\"t1\",\"pairs\":[[\"{0} has {1}\",[\"t1\",\"open\"]]]}"}]}}"#
        )
    );

    // ---- an unknown tool is a JSON-RPC error, not a crash ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":"#,
            r#"{"name":"frobnicate","arguments":{}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":12,"error":{"code":-32601,"#,
            r#""message":"unknown tool \"frobnicate\""}}"#
        )
    );

    // ---- initialize without a client protocolVersion answers the default ----
    let r = c.rpc(r#"{"jsonrpc":"2.0","id":13,"method":"initialize","params":{}}"#);
    assert!(r.contains(r#""protocolVersion":"2024-11-05""#), "{r}");
}
