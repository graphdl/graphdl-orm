//! The MCP binding, end to end: spawn the kernel with --mcp --apps-dir over
//! the fixture registry (tests/fixtures/apps, one app per subdirectory
//! carrying its <name>.store.json sidecar), speak newline-delimited JSON-RPC
//! 2.0 over stdio, and prove the daily-driver surface: initialize echoes the
//! client's protocol version, a notification answers nothing, tools/list
//! names the tool table, apps_use boots the fixture store through the serve
//! ingestion path, and the read verbs answer over the retained store. One
//! JSON object per line each way, as the resident protocol runs. The write
//! flow drives the delegated verbs over a temp apps directory: apps_compile
//! materializes a real app through the Python CLI, apply commits and then
//! refuses, retract removes, and every write reloads the sidecar into the
//! retained store. The same flow then drives the delegated read long tail
//! (get, schema, sql, explain, validate, verify, actions), which scopes to
//! the retained app and reloads nothing.

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
        Mcp::spawn_over(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/apps"))
    }

    // spawn_over boots the server against any apps directory; the write-flow
    // test points it at a temp registry it materializes itself.
    fn spawn_over(apps_dir: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arest"))
            .arg("--mcp")
            .arg("--apps-dir")
            .arg(apps_dir)
            // This suite tests the DELEGATION machinery (its write flow gates on
            // Python + cli.py being present), so pin apps_compile to the Python
            // oracle: the 2026-07-13 default flip made apps_compile native, and
            // the native base-atop compile in a DEBUG test binary exceeds the 60s
            // rpc timeout. The native path's own gate is the release-binary
            // parity harness (tools/apps_compile_parity.py), not this suite.
            .env("AREST_PYTHON_COMPILE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arest --mcp");
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

    // spawn_native_over: the PYTHON-FREE posture — no AREST_PYTHON_COMPILE pin
    // (apps_compile runs its native default) and AREST_NATIVE_RETRACT=1 (the
    // opt-in native retract this suite certifies). The write path then never
    // names Python.
    fn spawn_native_over(apps_dir: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arest"))
            .arg("--mcp")
            .arg("--apps-dir")
            .arg(apps_dir)
            .env("AREST_NATIVE_RETRACT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arest --mcp");
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

    // rpc_slow: for calls that embed a full native base-atop compile in a
    // DEBUG binary (native apps_compile; a committed retract's rebuild).
    fn rpc_slow(&mut self, line: &str) -> String {
        self.send(line);
        self.rx
            .recv_timeout(Duration::from_secs(300))
            .expect("no MCP reply within 300s (a debug native compile hung)")
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
            r#""serverInfo":{"name":"arest","version":"0.1.0"}}}"#
        )
    );

    // ---- a notification (no id) answers nothing: the next line on the wire
    //      must answer the next request ----
    c.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let r = c.rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(r.contains(r#""id":2"#), "the notification must produce no line: {r}");
    for tool in ["orient", "apps_list", "apps_current", "apps_use", "query", "cells",
                 "synthesize", "derive", "apply", "retract", "apps_compile",
                 "get", "schema", "sql", "explain", "validate", "verify", "actions"] {
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
    // synthesize now DELEGATES like the read tail (the canonical Rust path
    // reduces in minutes at daily-driver scale where the Python host's
    // native twins answer in seconds), so its answer assertion lives in the
    // python-guarded write-flow test; here it needs only the no-crash gate,
    // which the delegation machinery's own errors cover.

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

#[test]
fn mcp_write_verbs_delegate_to_the_cli_and_reload_the_sidecar() {
    // The write verbs shell out to the repository's one-shot Python CLI, so
    // the flow needs a python on PATH and cli.py above the server executable
    // (the same walk-up the binding performs at startup). Absent either, the
    // flow skips with a clear line, the way the pytest host gates skip when
    // a toolchain is missing.
    let python_ok = Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        println!("skipping the write flow: python --version failed to run");
        return;
    }
    let cli_found = std::path::Path::new(env!("CARGO_BIN_EXE_arest"))
        .ancestors()
        .skip(1)
        .any(|d| d.join("cli.py").is_file());
    if !cli_found {
        println!("skipping the write flow: no cli.py above the server executable");
        return;
    }

    // A fresh temp apps directory materializes a REAL app. The fixture ships
    // only the sidecar and a README, and the Python Registry's apply loads an
    // app from its .db, so a bare sidecar copy could not take a write. The
    // readings carry the fixture README's seven-line model, and apps_compile
    // builds the .db and the sidecar through the CLI.
    let tmp = std::env::temp_dir().join(format!(
        "arest-mcp-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(tmp.join("flow").join("readings")).unwrap();
    std::fs::write(
        tmp.join("flow").join("readings").join("app.md"),
        concat!(
            "Status is a value type.\n",
            "Note is a value type.\n",
            "Ticket is an entity type.\n",
            "Ticket has Status.\n",
            "Ticket has Note.\n",
            "Each Ticket has at most one Status.\n",
            "Each Ticket has at most one Note.\n"
        ),
    )
    .unwrap();

    let mut c = Mcp::spawn_over(&tmp.to_string_lossy());
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"#,
        r#""protocolVersion":"2025-06-18","capabilities":{},"#,
        r#""clientInfo":{"name":"itest","version":"0"}}}"#
    ));
    assert!(r.contains(r#""serverInfo""#), "{r}");

    // ---- apps_compile delegates the readings compile; the compile report
    //      (the CLI's stdout receipt) is the tool result ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":"#,
        r#"{"name":"apps_compile","arguments":{"app":"flow"}}}"#
    ));
    assert!(r.contains(r#"\"app\":\"flow\""#), "compile must answer the report: {r}");
    assert!(r.contains(r#"\"unparsed\":[]"#), "the model must parse clean: {r}");

    // ---- apps_use boots the compiled sidecar as the retained store ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":"#,
        r#"{"name":"apps_use","arguments":{"name":"flow"}}}"#
    ));
    assert!(r.contains(r#"\"ok\":true"#), "{r}");

    // ---- two committed applies; each receipt rides as the tool result and
    //      each commit reloads the rewritten sidecar ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":"#,
        r#"{"name":"apply","arguments":{"app":"flow","#,
        r#""fact_type":"Ticket_has_Status","fact":["t1","open"]}}}"#
    ));
    assert!(r.contains(r#"\"committed\":true"#), "{r}");
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":"#,
        r#"{"name":"apply","arguments":{"app":"flow","#,
        r#""fact_type":"Ticket_has_Status","fact":["t2","open"]}}}"#
    ));
    assert!(r.contains(r#"\"committed\":true"#), "{r}");

    // ---- the reload proof: query reads ONLY the retained store, so both
    //      written rows must appear without any further apps_use ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":"#,
        r#"{"name":"query","arguments":{"fact_type":"Ticket_has_Status"}}}"#
    ));
    assert!(r.contains(r#"[\"t1\",\"open\"]"#), "t1 must appear after the reload: {r}");
    assert!(r.contains(r#"[\"t2\",\"open\"]"#), "t2 must appear after the reload: {r}");

    // ---- a second Status on t2 refuses (the at-most-one constraint); the
    //      refusal is a RESULT the caller reads, never a protocol error ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":"#,
        r#"{"name":"apply","arguments":{"app":"flow","#,
        r#""fact_type":"Ticket_has_Status","fact":["t2","closed"]}}}"#
    ));
    assert!(r.contains(r#""result""#), "a refusal must ride as a result: {r}");
    assert!(r.contains(r#"\"committed\":false"#), "{r}");

    // ---- retract removes the t2 row and reloads; the population keeps t1 ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":"#,
        r#"{"name":"retract","arguments":{"app":"flow","#,
        r#""fact_type":"Ticket_has_Status","fact":["t2","open"]}}}"#
    ));
    assert!(r.contains(r#"\"committed\":true"#), "{r}");
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":"#,
        r#"{"name":"query","arguments":{"fact_type":"Ticket_has_Status"}}}"#
    ));
    assert!(r.contains(r#"[\"t1\",\"open\"]"#), "t1 must survive the retract: {r}");
    assert!(!r.contains(r#"[\"t2\",\"open\"]"#), "t2 must be gone after the retract: {r}");

    // ---- the read long tail delegates through the same CLI, scoped to the
    //      retained app, so no argument names an app and nothing reloads ----
    let r = c.rpc(r#"{"jsonrpc":"2.0","id":10,"method":"tools/list"}"#);
    for tool in ["get", "schema", "sql", "explain", "validate", "verify", "actions"] {
        assert!(r.contains(&format!(r#""name":"{tool}""#)), "missing read tool {tool}: {r}");
    }
    // get answers the per-entity view; the receipt's own key says exists.
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":"#,
        r#"{"name":"get","arguments":{"noun":"Ticket","id":"t1"}}}"#
    ));
    assert!(r.contains(r#"\"exists\":true"#), "get must answer the entity view: {r}");
    // schema answers the model surface, which names the fact type.
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":"#,
        r#"{"name":"schema","arguments":{}}}"#
    ));
    assert!(r.contains("Ticket_has_Status"), "schema must name the fact type: {r}");
    // This model has no state machine, so actions only has to SUCCEED as a
    // result; its shape stays the CLI's business.
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":"#,
        r#"{"name":"actions","arguments":{"noun":"Ticket","id":"t1"}}}"#
    ));
    assert!(r.contains(r#""result""#), "actions must succeed as a result: {r}");
    // sql answers rows of rows (a bare array, not an object envelope); the
    // count is some digit right after the opening brackets.
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":"#,
        r#"{"name":"sql","arguments":{"statement":"SELECT COUNT(*) FROM sqlite_master"}}}"#
    ));
    let open = match r.find("[[") {
        Some(i) => i,
        None => panic!("sql must answer rows of rows: {r}"),
    };
    assert!(r.as_bytes()[open + 2].is_ascii_digit(), "sql must answer a count: {r}");
    // synthesize delegates too (measured 2026-07-05: the canonical Rust path
    // reduces in minutes at daily-driver scale where the Python host's
    // native twins answer in seconds), so the entity's facts answer in the
    // Registry's rendered shape.
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":"#,
        r#"{"name":"synthesize","arguments":{"id":"t1"}}}"#
    ));
    assert!(r.contains("t1 has open"), "synthesize must render the fact: {r}");

    drop(c);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_write_path_retracts_with_no_python() {
    // The directive's Rust-configured-environment case, end to end, with NO
    // Python named anywhere: compile natively (the default), boot the store,
    // retract natively (AREST_NATIVE_RETRACT). Covers all three receipt
    // shapes: no-such-fact, the mandatory refusal (Def. Violation is
    // direction-blind, so removing the last Name violates the lower bound
    // while the Person still exists through its Age row), and the committed
    // retraction with its rebuild and reload.
    let tmp = std::env::temp_dir().join(format!(
        "arest-native-retract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(tmp.join("board").join("readings")).unwrap();
    std::fs::write(
        tmp.join("board").join("readings").join("app.md"),
        concat!(
            "Name is a value type.\n",
            "Age is a value type.\n",
            "Person is an entity type.\n",
            "Person has Name.\n",
            "Each Person has some Name.\n",
            "Person has Age.\n",
            "\n",
            "Person 'p1' has Name 'A'.\n",
            "Person 'p1' has Age '30'.\n",
            "Person 'p2' has Name 'B'.\n",
            "Person 'p2' has Age '40'.\n"
        ),
    )
    .unwrap();

    let mut c = Mcp::spawn_native_over(&tmp.to_string_lossy());
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"#,
        r#""protocolVersion":"2025-06-18","capabilities":{},"#,
        r#""clientInfo":{"name":"itest","version":"0"}}}"#
    ));
    assert!(r.contains(r#""serverInfo""#), "{r}");

    // ---- native compile (the default; nothing names Python) ----
    let r = c.rpc_slow(concat!(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":"#,
        r#"{"name":"apps_compile","arguments":{"app":"board"}}}"#
    ));
    assert!(r.contains(r#"\"unparsed\":[]"#), "the model must parse clean: {r}");
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":"#,
        r#"{"name":"apps_use","arguments":{"name":"board"}}}"#
    ));
    assert!(r.contains(r#"\"ok\":true"#), "{r}");

    // ---- no such fact: refused without touching the store ----
    // (Age literals coerce to integers at the cook boundary, so the wire
    // fact speaks the store's types: 99, not "99")
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":"#,
        r#"{"name":"retract","arguments":{"app":"board","#,
        r#""fact_type":"Person_has_Age","fact":["p9",99]}}}"#
    ));
    assert!(
        r.contains(r#"\"committed\": false"#) && r.contains("no such fact"),
        "{r}"
    );

    // ---- the mandatory lower bound refuses: p1's only Name ----
    let r = c.rpc_slow(concat!(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":"#,
        r#"{"name":"retract","arguments":{"app":"board","#,
        r#""fact_type":"Person_has_Name","fact":["p1","A"]}}}"#
    ));
    assert!(
        r.contains(r#"\"committed\": false"#) && !r.contains("no such fact"),
        "the shrunk population must refuse the mandatory violation: {r}"
    );
    assert!(!r.contains(r#"\"violations\": []"#), "the refusal names offenders: {r}");

    // ---- a legal retraction commits, rebuilds, and the row is gone ----
    let r = c.rpc_slow(concat!(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":"#,
        r#"{"name":"retract","arguments":{"app":"board","#,
        r#""fact_type":"Person_has_Age","fact":["p2",40]}}}"#
    ));
    assert!(r.contains(r#"\"committed\": true"#), "{r}");
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":"#,
        r#"{"name":"query","arguments":{"fact_type":"Person_has_Age"}}}"#
    ));
    assert!(
        r.contains(r#"[\"p1\",30]"#) && !r.contains(r#"[\"p2\",40]"#),
        "the retracted row must be gone and the sibling kept: {r}"
    );

    // ---- the event log carries the retract entry (the durable stream) ----
    let log = std::fs::read_to_string(tmp.join("board").join("board.events.jsonl")).unwrap();
    assert!(
        log.contains(r#""op": "retract""#) && log.contains(r#""ft": "Person_has_Age""#)
            && log.contains(r#"["p2", 40]"#),
        "{log}"
    );

    // ---- retracting the same row again: no such fact ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":"#,
        r#"{"name":"retract","arguments":{"app":"board","#,
        r#""fact_type":"Person_has_Age","fact":["p2",40]}}}"#
    ));
    assert!(
        r.contains(r#"\"committed\": false"#) && r.contains("no such fact"),
        "{r}"
    );

    drop(c);
    let _ = std::fs::remove_dir_all(&tmp);
}
