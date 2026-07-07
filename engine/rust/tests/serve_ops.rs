//! The serve loop's verb surface, end to end: spawn the kernel with --serve,
//! build a store OVER THE EXISTING SERVE OPS (set d, then evolve it with
//! retain cases through the canonical ast:Store — no new machinery), and prove
//! the verb ops (verbs, query, cells, synthesize_pairs) answer over that
//! resident store. One JSON per line each way, as the resident protocol runs.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

struct Serve {
    child: Child,
    out: BufReader<ChildStdout>,
}

impl Serve {
    fn spawn() -> Serve {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arest"))
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn arest --serve");
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
        drop(self.child.stdin.take());                        // EOF ends the loop
        let _ = self.child.wait();
    }
}

/// A store step from EXISTING ops only: f = ⟨CONS, ⟨CONST,"ok"⟩, apply∘⟨H, id⟩⟩
/// with H = apply∘⟨⟨CONST,"ast:Store"⟩, ⟨CONST,name⟩⟩, so f : ⟨rows, D⟩ =
/// ⟨"ok", (ast:Store : name) : ⟨rows, D⟩⟩ — the retain protocol's pair shape,
/// the canonical ast:Store doing the writing. rows ride the xd field (the
/// resident ⟨fact, D⟩ pairing); retain commits D' as the retained store.
fn store_case(name: &str, rows: &str) -> String {
    format!(
        r#"{{"cases":[{{"f":["CONS",["CONST","ok"],["COMP","apply",["CONS",["COMP","apply",["CONS",["CONST","ast:Store"],["CONST","{name}"]]],"id"]]],"xd":{rows},"fuel":0,"retain":1}}]}}"#
    )
}

#[test]
fn verb_ops_answer_over_a_store_built_via_serve_ops() {
    let mut s = Serve::spawn();

    // ---- build the resident store via the EXISTING serve-loop ops ----
    // 1. set the resident store to the empty store
    assert_eq!(s.rpc(r#"{"d": []}"#), "[]");
    // 2. evolve it: three retained steps through the canonical ast:Store
    let r = s.rpc(&store_case(
        "factType",
        r#"[["Person_has_Name","{0} has {1}"],["Person_keeps_Pet","{0} keeps {1}"]]"#,
    ));
    assert!(r.starts_with(r#"[["ok","#), "factType store step: {r}");
    let r = s.rpc(&store_case(
        "Person_has_Name",
        r#"[["p1","Ada"],["p2","Bo"]]"#,
    ));
    assert!(r.starts_with(r#"[["ok","#), "Person_has_Name store step: {r}");
    let r = s.rpc(&store_case("Person_keeps_Pet", r#"[["p1","rex"]]"#));
    assert!(r.starts_with(r#"[["ok","#), "Person_keeps_Pet store step: {r}");

    // ---- verbs: the surface-agnostic table + the resident subset — the
    //      FULL first-class table (rust-primary serving: one verb surface,
    //      every binding advertises exactly what it dispatches) ----
    assert_eq!(
        s.rpc(r#"{"op":"verbs"}"#),
        concat!(
            r#"{"op":"verbs","result":{"verbs":["#,
            r#""apps_check","apps_compile","apps_create","apps_current","apps_list","#,
            r#""apps_register","apps_status","apps_use","context","engine_version","orient","#,
            r#""apply","ask","cells","compile","explain","get","induce","propose","#,
            r#""query","retract","schema","sql","synthesize"],"#,
            r#""session":["apps_check","apps_compile","apps_create","apps_current","#,
            r#""apps_list","apps_register","apps_status","apps_use","context","#,
            r#""engine_version","orient"],"#,
            r#""app":["apply","ask","cells","compile","explain","get","induce","#,
            r#""propose","query","retract","schema","sql","synthesize"],"#,
            r#""resident":["cells","query","run_rules","synthesize_pairs","verbs"]}}"#
        )
    );

    // ---- cells: names + row counts over the resident store, sorted;
    //      pattern filters case-insensitively ----
    assert_eq!(
        s.rpc(r#"{"op":"cells"}"#),
        concat!(
            r#"{"op":"cells","result":{"cells":["#,
            r#"{"name":"Person_has_Name","rows":2},"#,
            r#"{"name":"Person_keeps_Pet","rows":1},"#,
            r#"{"name":"factType","rows":2}]}}"#
        )
    );
    assert_eq!(
        s.rpc(r#"{"op":"cells","pattern":"person"}"#),
        concat!(
            r#"{"op":"cells","result":{"cells":["#,
            r#"{"name":"Person_has_Name","rows":2},"#,
            r#"{"name":"Person_keeps_Pet","rows":1}]}}"#
        )
    );

    // ---- query: FetchPop of a named cell over the resident store ----
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Person_has_Name"}"#),
        r#"{"op":"query","result":{"fact_type":"Person_has_Name","rows":[["p1","Ada"],["p2","Bo"]]}}"#
    );
    // an absent cell answers the empty population (FetchPop's PHI), never ⊥
    assert_eq!(
        s.rpc(r#"{"op":"query","fact_type":"Nope"}"#),
        r#"{"op":"query","result":{"fact_type":"Nope","rows":[]}}"#
    );

    // ---- synthesize_pairs: (system:verbalize : id) : D — the entity's facts
    //      paired with their fact types' reading templates ----
    let syn = s.rpc(r#"{"op":"synthesize_pairs","id":"p1"}"#);
    assert!(
        syn.starts_with(r#"{"op":"synthesize_pairs","result":{"id":"p1","pairs":["#),
        "{syn}"
    );
    assert!(syn.contains(r#"["{0} has {1}",["p1","Ada"]]"#), "{syn}");
    assert!(syn.contains(r#"["{0} keeps {1}",["p1","rex"]]"#), "{syn}");
    assert!(!syn.contains("Bo"), "p2's facts must not appear: {syn}");

    // ---- errors are answers, not crashes ----
    assert_eq!(
        s.rpc(r#"{"op":"query"}"#),
        r#"{"op":"query","error":"query needs a scalar fact_type"}"#
    );
    let registry_backed = s.rpc(r#"{"op":"sql","statement":"SELECT 1"}"#);
    assert!(
        registry_backed.contains(r#""error""#)
            && registry_backed.contains("apps registry"),
        "{registry_backed}"
    );
    let unknown = s.rpc(r#"{"op":"frobnicate"}"#);
    assert!(unknown.contains("unknown op"), "{unknown}");

    // ---- the case protocol still serves beside the ops, unchanged ----
    assert_eq!(
        s.rpc(r#"{"cases":[{"f":"length","x":["a","b","c"],"fuel":0}]}"#),
        "[3]"
    );
}
