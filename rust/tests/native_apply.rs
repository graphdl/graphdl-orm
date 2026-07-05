//! The resident's NATIVE write path for OWN-TABLE fact types, end to end. A
//! fact type whose RMAP mapping keeps it its own table carries a create:<ft>
//! handler cell (engine.py create_handlers stores one per own-table fact type;
//! an absorbed fact type has none, phase two), so the resident computes and
//! persists the create in process — fetch the handler, reduce it over the pair
//! of the fact and the retained store, extract the receipt exactly as
//! protocol.py Registry.apply does, retain D', run the native derive, and
//! persist natively (the event line to <app>.events.jsonl, the store sidecar
//! refreshed). An absorbed fact type has no handler cell and FALLS BACK to the
//! Python CLI delegation.
//!
//! The discriminator that keeps the test honest: the own-table flow runs
//! against a resident spawned with a BOGUS --python, so any delegation fails to
//! spawn. An own-table apply that commits there committed WITHOUT the host — it
//! went native. The absorbed apply against the same broken-delegate resident
//! errors, proving it took the delegate path. A second resident with a real
//! python then proves the absorbed fallback commits end to end, and supplies
//! the Python parity reference for the differential (the native store's queried
//! rows against a Python-written app's, over the same own-table fact).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

struct Mcp {
    child: Child,
    rx: Receiver<String>,
}

impl Mcp {
    // spawn_over boots the MCP server against an apps directory; python names
    // the interpreter the delegate spawns (a bogus name breaks delegation so a
    // committed own-table apply proves it went native).
    fn spawn_over(apps_dir: &str, python: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arestlam"))
            .arg("--mcp")
            .arg("--apps-dir")
            .arg(apps_dir)
            .arg("--python")
            .arg(python)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arestlam --mcp");
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
        let mut c = Mcp { child, rx };
        // initialize once so the loop is live before the first tool call
        let r = c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"#,
            r#""protocolVersion":"2025-06-18","capabilities":{},"#,
            r#""clientInfo":{"name":"itest","version":"0"}}}"#
        ));
        assert!(r.contains(r#""serverInfo""#), "{r}");
        c
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
            .expect("no MCP reply within 30s")
    }

    fn rpc(&mut self, line: &str) -> String {
        self.send(line);
        self.recv()
    }

    // call is one tools/call, the arguments object supplied verbatim.
    fn call(&mut self, id: u32, name: &str, arguments: &str) -> String {
        self.rpc(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
        ))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

fn repo_root() -> std::path::PathBuf {
    // the crate manifest is <root>/rust; cli.py lives at <root>/cli.py
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// compile_app materializes a real app through the Python CLI (the same
// one-shot the resident's delegate spawns), building <app>.db and the
// <app>.store.json sidecar the resident boots from.
fn compile_app(apps_dir: &std::path::Path, name: &str) {
    let cli = repo_root().join("cli.py");
    let out = Command::new("python")
        .arg(&cli)
        .arg("compile")
        .arg("--apps-dir")
        .arg(apps_dir)
        .arg(name)
        .output()
        .expect("spawn python cli.py compile");
    assert!(
        out.status.success(),
        "compile {name} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// py_apply drives one Python Registry.apply through the CLI (the parity
// reference: the delegated write path the resident is replacing for own-table).
fn py_apply(apps_dir: &std::path::Path, name: &str, ft: &str, row_json: &str) {
    let cli = repo_root().join("cli.py");
    let out = Command::new("python")
        .arg(&cli)
        .arg("apply")
        .arg("--apps-dir")
        .arg(apps_dir)
        .arg(name)
        .arg(ft)
        .arg(row_json)
        .output()
        .expect("spawn python cli.py apply");
    // apply exits 0 on a committed write; the receipt rides stdout
    assert!(
        out.status.success(),
        "py_apply {name} {ft} refused or failed: {} / {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_readings(apps_dir: &std::path::Path, name: &str) {
    let dir = apps_dir.join(name).join("readings");
    std::fs::create_dir_all(&dir).unwrap();
    // Task blocks Task is an m:n binary fact type: no uniqueness, so RMAP keeps
    // it its OWN table and create_handlers stores create:Task_blocks_Task.
    // Task has Status is functional (at most one), so RMAP ABSORBS it into the
    // Task table and it carries no create cell — the phase-two, delegate case.
    std::fs::write(
        dir.join("app.md"),
        concat!(
            "Status is a value type.\n",
            "Task is an entity type.\n",
            "Task blocks Task.\n",
            "Task has Status.\n",
            "Each Task has at most one Status.\n"
        ),
    )
    .unwrap();
}

// the escaped rows tail of a query answer, for the native-versus-Python
// differential: the MCP content wraps {"fact_type":..,"rows":..} as an escaped
// text string, so the substring from "rows" onward is a stable, resident-only
// projection of the stored population.
fn rows_tail(resp: &str) -> String {
    let i = resp.find("rows").expect("a query answer names rows");
    resp[i..].to_string()
}

#[test]
fn native_apply_commits_own_table_in_process_and_delegates_absorbed() {
    // Gate exactly as the delegated write-flow test does: the flow needs a
    // python on PATH and cli.py above the crate. Absent either, skip clean.
    let python_ok = Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        println!("skipping native-apply: python --version failed to run");
        return;
    }
    if !repo_root().join("cli.py").is_file() {
        println!("skipping native-apply: no cli.py at the repository root");
        return;
    }

    let tmp = std::env::temp_dir().join(format!(
        "arestlam-native-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    write_readings(&tmp, "ring");
    write_readings(&tmp, "ringpy");
    compile_app(&tmp, "ring");
    compile_app(&tmp, "ringpy");

    // A bogus interpreter: any delegation this resident attempts fails to
    // spawn, so a committed own-table apply committed WITHOUT the host.
    let bogus = "arestlam_no_such_python_interpreter";

    // ---- Phase 1: the own-table apply computes natively, no host ----
    let ring_rows;
    {
        let mut c = Mcp::spawn_over(&tmp.to_string_lossy(), bogus);
        let r = c.call(1, "apps_use", r#"{"name":"ring"}"#);
        assert!(r.contains(r#"\"ok\":true"#), "apps_use ring: {r}");

        // the structural proof of which path each fact type takes: the
        // own-table fact type carries a create:<ft> handler cell, the absorbed
        // one does not.
        let r = c.call(2, "cells", r#"{"pattern":"create:"}"#);
        assert!(
            r.contains(r#"\"name\":\"create:Task_blocks_Task\""#),
            "own-table must carry a create cell: {r}"
        );
        assert!(
            !r.contains("create:Task_has_Status"),
            "absorbed must carry NO create cell: {r}"
        );

        // the own-table apply: committed in process, the receipt protocol.py's
        // shape. On the delegating code this line delegates to the bogus python
        // and errors, so committed:true is the red-to-green gate.
        let r = c.call(
            3,
            "apply",
            r#"{"app":"ring","fact_type":"Task_blocks_Task","fact":["t1","t2"]}"#,
        );
        assert!(
            r.contains(r#"\"committed\":true"#),
            "own-table apply must commit natively (no host): {r}"
        );
        assert!(r.contains(r#"\"violations\":[]"#), "no violations expected: {r}");
        assert!(
            r.contains(r#"\"fact_type\":\"Task_blocks_Task\""#),
            "the receipt names the fact type: {r}"
        );

        // the row is in the retained store immediately.
        let r = c.call(4, "query", r#"{"fact_type":"Task_blocks_Task"}"#);
        assert!(
            r.contains(r#"[[\"t1\",\"t2\"]]"#),
            "query must show the written row: {r}"
        );
        ring_rows = rows_tail(&r);

        // the event line landed in FileEventSink's format, byte for byte the
        // json.dumps(entry) the Python sink writes (default separators, spaces).
        let ev = std::fs::read_to_string(tmp.join("ring").join("ring.events.jsonl"))
            .expect("the event log must exist after a commit");
        assert!(
            ev.contains(r#"{"ft": "Task_blocks_Task", "fact": ["t1", "t2"]}"#),
            "the event line must match FileEventSink: {ev:?}"
        );

        // the absorbed apply has no create cell, so it FALLS BACK to
        // delegation — which fails against the bogus python, proving the
        // fallback routes to the CLI (never silently native).
        let r = c.call(
            5,
            "apply",
            r#"{"app":"ring","fact_type":"Task_has_Status","fact":["t1","open"]}"#,
        );
        assert!(
            r.contains(r#""error""#) && !r.contains(r#"\"committed\":true"#),
            "absorbed must delegate (and fail against the bogus python): {r}"
        );
    }

    // ---- Phase 2: durability + the native-versus-Python differential ----
    // The Python parity reference: the SAME own-table fact through Python's
    // Registry.apply against the twin app.
    py_apply(&tmp, "ringpy", "Task_blocks_Task", r#"["t1","t2"]"#);
    {
        // A FRESH resident (real python) boots each app from its refreshed
        // sidecar and reads the population back through the native query op.
        let mut c = Mcp::spawn_over(&tmp.to_string_lossy(), "python");

        // durability: the native write survived to the sidecar a new resident
        // boots from.
        let r = c.call(1, "apps_use", r#"{"name":"ring"}"#);
        assert!(r.contains(r#"\"ok\":true"#), "apps_use ring: {r}");
        let r = c.call(2, "query", r#"{"fact_type":"Task_blocks_Task"}"#);
        assert!(
            r.contains(r#"[[\"t1\",\"t2\"]]"#),
            "a fresh resident must see the native write (durable sidecar): {r}"
        );
        let ring_rows_reboot = rows_tail(&r);
        assert_eq!(ring_rows, ring_rows_reboot, "the reboot rows must match");

        // the differential: the native store's rows equal Python's for the
        // same own-table fact.
        let r = c.call(3, "apps_use", r#"{"name":"ringpy"}"#);
        assert!(r.contains(r#"\"ok\":true"#), "apps_use ringpy: {r}");
        let r = c.call(4, "query", r#"{"fact_type":"Task_blocks_Task"}"#);
        let py_rows = rows_tail(&r);
        assert_eq!(
            ring_rows, py_rows,
            "native apply's rows must match Python Registry.apply's rows"
        );

        // the absorbed fallback commits end to end through the real delegate.
        let r = c.call(
            5,
            "apply",
            r#"{"app":"ringpy","fact_type":"Task_has_Status","fact":["t9","open"]}"#,
        );
        assert!(
            r.contains(r#"\"committed\":true"#),
            "absorbed apply must commit through delegation: {r}"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
