//! Phase one of the derivation-engine port ({"op":"run_rules"} and the MCP
//! "derive" tool): the NAIVE positive-rule fixpoint over the retained store.
//! The serve tests build a store through the EXISTING ops (set d, evolve it
//! with retained ast:Store cases) carrying hand-written COPY rules (a body of
//! one canonical FetchPop, so a rule's rows are its source cell's rows) plus
//! the M-fact cells run_rules reads, and prove: the fixpoint crosses rounds
//! to the least fixed point, the retained store is REPLACED so query answers
//! the derived rows, asserted rows survive the union, aggregate-marked and
//! uncompiled rule ids are skipped, a second run adds nothing (idempotence),
//! and the two mirror blocks fill exactly the empty cells. The MCP test
//! drives the python-gated write flow over a real compiled app (the anaphoric
//! state-machine model from tests/test_rule_anaphora.py) and proves derive
//! answers idempotently over a store Python already derived.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

// ---- the serve harness (serve_ops.rs's pattern) ----
struct Serve {
    child: Child,
    out: BufReader<ChildStdout>,
}

impl Serve {
    fn spawn() -> Serve {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arestlam"))
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arestlam --serve");
        let out = BufReader::new(child.stdout.take().unwrap());
        Serve { child, out }
    }

    fn rpc(&mut self, line: &str) -> String {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        reply.trim_end().to_string()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        drop(self.child.stdin.take()); // EOF ends the loop
        let _ = self.child.wait();
    }
}

/// A store step from EXISTING ops only (serve_ops.rs's builder): f : ⟨x, D⟩ =
/// ⟨"ok", (ast:Store : name) : ⟨x, D⟩⟩, with x riding the xd field and retain
/// committing D' as the retained store. x is ANY value, so the same step
/// stores row populations and compiled rule objects alike.
fn store_case(name: &str, contents: &str) -> String {
    format!(
        r#"{{"cases":[{{"f":["CONS",["CONST","ok"],["COMP","apply",["CONS",["COMP","apply",["CONS",["CONST","ast:Store"],["CONST","{name}"]]],"id"]]],"xd":{contents},"fuel":0,"retain":1}}]}}"#
    )
}

/// A COPY rule object: applied to D it answers the source cell's rows,
/// obj : D = (ast:FetchPop : src) : D. This is exactly the shape of a
/// compiled one-atom identity rule (compiler.py's copy-rule case), written
/// out of canonical parts so the test owes Python nothing.
fn copy_rule(src: &str) -> String {
    format!(
        r#"["COMP","apply",["CONS",["COMP","apply",["CONS",["CONST","ast:FetchPop"],["CONST","{src}"]]],"id"]]"#
    )
}

#[test]
fn run_rules_reaches_the_least_fixed_point_and_replaces_the_retained_store() {
    let mut s = Serve::spawn();
    assert_eq!(s.rpc(r#"{"d": []}"#), "[]");

    // ---- the base population, the rule table, and the rule objects ----
    let r = s.rpc(&store_case("Src", r#"[["a","x"],["b","y"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "Src store step: {r}");
    // Dst carries an ASSERTED row the rules never derive; the union must
    // keep it (derive adds, never removes).
    let r = s.rpc(&store_case("Dst", r#"[["z","w"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "Dst store step: {r}");
    // chain_rule sits FIRST so its source (Dst, fed by copy_rule) is behind
    // it in every round: reaching Chain's fixpoint takes a second productive
    // round, proving the loop iterates rather than sweeping once.
    let r = s.rpc(&store_case(
        "ruleDerives",
        concat!(
            r#"[["chain_rule","Chain"],["copy_rule","Dst"],"#,
            r#"["agg_rule","AggHead"],["ghost_rule","Ghost"]]"#
        ),
    ));
    assert!(r.starts_with(r#"[["ok","#), "ruleDerives store step: {r}");
    // agg_rule is marked aggregate: phase one must SKIP it entirely.
    let r = s.rpc(&store_case("ruleAgg", r#"[["agg_rule"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "ruleAgg store step: {r}");
    let r = s.rpc(&store_case("copy_rule", &copy_rule("Src")));
    assert!(r.starts_with(r#"[["ok","#), "copy_rule store step: {r}");
    let r = s.rpc(&store_case("chain_rule", &copy_rule("Dst")));
    assert!(r.starts_with(r#"[["ok","#), "chain_rule store step: {r}");
    let r = s.rpc(&store_case("agg_rule", &copy_rule("Src")));
    assert!(r.starts_with(r#"[["ok","#), "agg_rule store step: {r}");
    // ghost_rule stays an M-facts-only rule: no compiled object cell at all.

    // ---- the fixpoint: round one fills Dst (and Chain with the asserted
    //      row), round two chains Dst's derived rows into Chain, round three
    //      is quiet ----
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        r#"{"op":"run_rules","result":{"rounds":3,"changed":["Chain","Dst"]}}"#
    );

    // ---- the retained store is REPLACED: query answers the derived rows ----
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Dst"}"#),
        r#"{"op":"query","result":{"fact_type":"Dst","rows":[["a","x"],["b","y"],["z","w"]]}}"#
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Chain"}"#),
        r#"{"op":"query","result":{"fact_type":"Chain","rows":[["a","x"],["b","y"],["z","w"]]}}"#
    );
    // the aggregate rule was skipped and the uncompiled rule fired nothing
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"AggHead"}"#),
        r#"{"op":"query","result":{"fact_type":"AggHead","rows":[]}}"#
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Ghost"}"#),
        r#"{"op":"query","result":{"fact_type":"Ghost","rows":[]}}"#
    );

    // ---- idempotence: the fixpoint of a fixpoint adds nothing ----
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        r#"{"op":"run_rules","result":{"rounds":1,"changed":[]}}"#
    );
}

#[test]
fn the_mirror_blocks_fill_exactly_the_empty_cells() {
    let mut s = Serve::spawn();
    assert_eq!(s.rpc(r#"{"d": []}"#), "[]");

    // role rows are ⟨role id, fact type, position, player⟩; Person is a noun
    // (instanceOf ObjectType) and Name is not, so only position 1 mirrors.
    let r = s.rpc(&store_case(
        "role",
        r#"[["r1","Person_has_Name",1,"Person"],["r2","Person_has_Name",2,"Name"]]"#,
    ));
    assert!(r.starts_with(r#"[["ok","#), "role store step: {r}");
    let r = s.rpc(&store_case(
        "instanceOf",
        r#"[["Person","ObjectType"],["Name","ValueType"]]"#,
    ));
    assert!(r.starts_with(r#"[["ok","#), "instanceOf store step: {r}");
    let r = s.rpc(&store_case("Person_has_Name", r#"[["p1","Ada"],["p2","Bo"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "population store step: {r}");
    // the mirrors run when ANY rule reads them; the reader needs no compiled
    // object (the M-facts alone put it in the reads map, exactly as Python's)
    let r = s.rpc(&store_case(
        "ruleReads",
        concat!(
            r#"[["some_rule","Resource_is_instance_of_Noun"],"#,
            r#"["some_rule","Fact_Type_has_Role"]]"#
        ),
    ));
    assert!(r.starts_with(r#"[["ok","#), "ruleReads store step: {r}");

    // no ruleDerives at all: the loop is one quiet round; only mirrors fire
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        concat!(
            r#"{"op":"run_rules","result":{"rounds":1,"#,
            r#""changed":["Fact_Type_has_Role","Resource_is_instance_of_Noun"]}}"#
        )
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Resource_is_instance_of_Noun"}"#),
        concat!(
            r#"{"op":"query","result":{"fact_type":"Resource_is_instance_of_Noun","#,
            r#""rows":[["p1","Person"],["p2","Person"]]}}"#
        )
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Fact_Type_has_Role"}"#),
        concat!(
            r#"{"op":"query","result":{"fact_type":"Fact_Type_has_Role","#,
            r#""rows":[["Person_has_Name","r1"],["Person_has_Name","r2"]]}}"#
        )
    );

    // the filled mirrors now count as asserted: a second run changes nothing
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        r#"{"op":"run_rules","result":{"rounds":1,"changed":[]}}"#
    );
}

#[test]
fn asserted_mirror_rows_win_over_the_derivation() {
    let mut s = Serve::spawn();
    assert_eq!(s.rpc(r#"{"d": []}"#), "[]");
    let r = s.rpc(&store_case("role", r#"[["r1","Person_has_Name",1,"Person"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "role store step: {r}");
    let r = s.rpc(&store_case("instanceOf", r#"[["Person","ObjectType"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "instanceOf store step: {r}");
    let r = s.rpc(&store_case("Person_has_Name", r#"[["p1","Ada"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "population store step: {r}");
    let r = s.rpc(&store_case(
        "ruleReads",
        concat!(
            r#"[["some_rule","Resource_is_instance_of_Noun"],"#,
            r#"["some_rule","Fact_Type_has_Role"]]"#
        ),
    ));
    assert!(r.starts_with(r#"[["ok","#), "ruleReads store step: {r}");
    // both mirror cells already carry asserted rows: the mirrors must leave
    // them untouched (they serve only the EMPTY cell)
    let r = s.rpc(&store_case(
        "Resource_is_instance_of_Noun",
        r#"[["custom","Row"]]"#,
    ));
    assert!(r.starts_with(r#"[["ok","#), "asserted mirror store step: {r}");
    let r = s.rpc(&store_case("Fact_Type_has_Role", r#"[["kept","asserted"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "asserted role mirror store step: {r}");

    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        r#"{"op":"run_rules","result":{"rounds":1,"changed":[]}}"#
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Resource_is_instance_of_Noun"}"#),
        concat!(
            r#"{"op":"query","result":{"fact_type":"Resource_is_instance_of_Noun","#,
            r#""rows":[["custom","Row"]]}}"#
        )
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Fact_Type_has_Role"}"#),
        concat!(
            r#"{"op":"query","result":{"fact_type":"Fact_Type_has_Role","#,
            r#""rows":[["kept","asserted"]]}}"#
        )
    );
}

#[test]
fn the_rust_fixpoint_rederives_an_emptied_head_from_python_compiled_rules() {
    // Idempotence alone cannot tell evaluation from silent skipping (both
    // answer changed []), so this test EMPTIES a derived head cell on a store
    // the Python compiler built and proves the Rust loop re-derives the row
    // through the compiled rule object riding in D's own DEFS cells. Gated on
    // the Python host exactly as mcp.rs gates the write flow.
    let python_ok = Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        println!("skipping the rederive flow: python --version failed to run");
        return;
    }
    let cli = std::path::Path::new(env!("CARGO_BIN_EXE_arestlam"))
        .ancestors()
        .skip(1)
        .map(|d| d.join("cli.py"))
        .find(|p| p.is_file());
    let cli = match cli {
        Some(p) => p,
        None => {
            println!("skipping the rederive flow: no cli.py above the server executable");
            return;
        }
    };

    // the SAME model as the MCP flow below (tests/test_rule_anaphora.py's
    // SM_MODEL), compiled directly through the one-shot CLI
    let tmp = std::env::temp_dir().join(format!(
        "arestlam-serve-rederive-{}-{}",
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
            "Status is a value type.\n",
            "Resource is an entity type.\n",
            "State Machine is an entity type.\n",
            "Task is an entity type.\n",
            "Task Status is a value type.\n",
            "State Machine is for Resource.\n",
            "State Machine is currently in Status.\n",
            "Task has Task Status.\n",
            "\n",
            "* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.\n",
            "\n",
            "* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.\n",
            "\n",
            "State Machine 'sm1' is for Resource 't1'.\n",
            "State Machine 'sm1' is currently in Status 'in_progress'.\n",
            "State Machine 'sm2' is for Resource 't2'.\n"
        ),
    )
    .unwrap();
    let out = Command::new("python")
        .arg(&cli)
        .arg("compile")
        .arg("--apps-dir")
        .arg(&tmp)
        .arg("board")
        .output()
        .expect("spawn python cli.py compile");
    assert!(
        out.status.success(),
        "cli.py compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // the sidecar IS one serve set_store line; feed it through the same
    // ingestion a --serve stdin line takes
    let payload =
        std::fs::read_to_string(tmp.join("board").join("board.store.json")).unwrap();
    let mut s = Serve::spawn();
    assert_eq!(s.rpc(payload.trim()), "[]");

    // empty the derived head behind the rules' back, then derive: the
    // compiled rule must rebuild the row from the state-machine base facts
    let r = s.rpc(&store_case("Resource_is_currently_in_Status", "[]"));
    assert!(r.starts_with(r#"[["ok","#), "empty-the-head store step: {r}");
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        concat!(
            r#"{"op":"run_rules","result":{"rounds":2,"#,
            r#""changed":["Resource_is_currently_in_Status"]}}"#
        )
    );
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Resource_is_currently_in_Status"}"#),
        concat!(
            r#"{"op":"query","result":{"fact_type":"Resource_is_currently_in_Status","#,
            r#""rows":[["t1","in_progress"]]}}"#
        )
    );
    // and the rebuilt store is again a fixpoint
    assert_eq!(
        s.rpc(r#"{"op":"run_rules"}"#),
        r#"{"op":"run_rules","result":{"rounds":1,"changed":[]}}"#
    );

    drop(s);
    let _ = std::fs::remove_dir_all(&tmp);
}

// ---- the MCP harness (mcp.rs's pattern) ----
struct Mcp {
    child: Child,
    rx: Receiver<String>,
}

impl Mcp {
    fn spawn_over(apps_dir: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arestlam"))
            .arg("--mcp")
            .arg("--apps-dir")
            .arg(apps_dir)
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
        Mcp { child, rx }
    }

    fn rpc(&mut self, line: &str) -> String {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        self.rx
            .recv_timeout(Duration::from_secs(60))
            .expect("no MCP reply within 60s (the mode must answer one line per request)")
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take()); // EOF ends the loop
        let _ = self.child.wait();
    }
}

#[test]
fn the_derive_tool_is_idempotent_over_a_python_derived_app() {
    // The flow needs the Python compiler host, gated exactly as mcp.rs gates
    // the write flow: a python on PATH and cli.py above the executable.
    let python_ok = Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_ok {
        println!("skipping the derive flow: python --version failed to run");
        return;
    }
    let cli_found = std::path::Path::new(env!("CARGO_BIN_EXE_arestlam"))
        .ancestors()
        .skip(1)
        .any(|d| d.join("cli.py").is_file());
    if !cli_found {
        println!("skipping the derive flow: no cli.py above the server executable");
        return;
    }

    // The model is EXACTLY tests/test_rule_anaphora.py's SM_MODEL: two
    // anaphoric derivation rules the Python compiler compiles, with instance
    // facts that derive one row into each head.
    let tmp = std::env::temp_dir().join(format!(
        "arestlam-mcp-derive-{}-{}",
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
            "Status is a value type.\n",
            "Resource is an entity type.\n",
            "State Machine is an entity type.\n",
            "Task is an entity type.\n",
            "Task Status is a value type.\n",
            "State Machine is for Resource.\n",
            "State Machine is currently in Status.\n",
            "Task has Task Status.\n",
            "\n",
            "* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.\n",
            "\n",
            "* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.\n",
            "\n",
            "State Machine 'sm1' is for Resource 't1'.\n",
            "State Machine 'sm1' is currently in Status 'in_progress'.\n",
            "State Machine 'sm2' is for Resource 't2'.\n"
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

    // ---- the tool table names derive ----
    let r = c.rpc(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(r.contains(r#""name":"derive""#), "missing tool derive: {r}");

    // ---- derive needs a current app ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":"#,
            r#"{"name":"derive","arguments":{}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"#,
            r#""message":"no app loaded; call apps_use before derive"}}"#
        )
    );

    // ---- compile through the CLI (Python compiles AND derives), then boot ----
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":"#,
        r#"{"name":"apps_compile","arguments":{"app":"board"}}}"#
    ));
    assert!(r.contains(r#"\"unparsed\":[]"#), "the model must parse clean: {r}");
    assert!(r.contains(r#"\"rule_diagnostics\":[]"#), "the rules must compile: {r}");
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":"#,
        r#"{"name":"apps_use","arguments":{"name":"board"}}}"#
    ));
    assert!(r.contains(r#"\"ok\":true"#), "{r}");

    // ---- the phase-one bar: over a store Python already derived, the Rust
    //      fixpoint terminates in one quiet round and changes NOTHING (the
    //      fixpoint of a fixpoint), and the derived rows still answer ----
    assert_eq!(
        c.rpc(concat!(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":"#,
            r#"{"name":"derive","arguments":{}}}"#
        )),
        concat!(
            r#"{"jsonrpc":"2.0","id":6,"result":{"content":[{"type":"text","#,
            r#""text":"{\"rounds\":1,\"changed\":[]}"}]}}"#
        )
    );
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":"#,
        r#"{"name":"query","arguments":{"fact_type":"Resource_is_currently_in_Status"}}}"#
    ));
    assert!(
        r.contains(r#"[\"t1\",\"in_progress\"]"#),
        "the derived row must survive the Rust fixpoint: {r}"
    );
    let r = c.rpc(concat!(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":"#,
        r#"{"name":"query","arguments":{"fact_type":"Task_has_Task_Status"}}}"#
    ));
    assert!(
        r.contains(r#"[\"t1\",\"in_progress\"]"#),
        "the re-keyed derived row must survive the Rust fixpoint: {r}"
    );

    drop(c);
    let _ = std::fs::remove_dir_all(&tmp);
}
