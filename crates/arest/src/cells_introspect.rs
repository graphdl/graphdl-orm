// crates/arest/src/cells_introspect.rs
//
// Read-only cell introspection verb (#870).
//
// Today, agents and humans both drop to `sqlite3 cells "select name,
// length(contents) from cells …"` to inspect what compile / apply
// wrote — find malformed cells, check derivation rule outputs,
// look up the latest contents of a single cell. This module lifts
// that workflow into the engine so MCP callers (and the CLI shell-
// out the MCP shim uses) get a structured envelope instead of
// reaching for a sidecar SQLite tool.
//
// Three modes:
//   * list pattern=<glob>     → {cells: [{name, size_bytes}, ...]}
//   * get name=<cell>         → {name, contents: <parsed-tuples>, size_bytes}
//   * trace rule_id=<id>      → {rule_text, consequent_cell, materialized_count}
//   * trace rule_pattern=<s>  → same shape, first matching rule
//
// The verb accepts a JSON envelope:
//   {"mode": "list", "pattern": "Task_*"}
//   {"mode": "get",  "name":    "FactType"}
//   {"mode": "trace","rule_id": "rule_…"}
//   {"mode": "trace","rule_pattern": "Task is overdue"}
//
// Cells ARE relations (RMAP / whitepaper §3) but `cells` is the
// flat view — it doesn't materialize per-FT SQL tables, it just
// surfaces what's actually stored in the cell graph (size +
// optional structured contents). Use `sql` (#864) when you need
// to JOIN; use `cells` when you need to know what cells exist
// and how big they are.
//
// Mirrors the wiring shape of the `sql` verb (#864): engine-level
// intercept under the shared lock, pure projection, JSON envelope,
// no_std builds get a structured "feature unavailable" diagnostic
// because the verb leans on serde_json for envelope shaping.

#![cfg(feature = "std-deps")]

use crate::ast::{self, Object};
use serde_json::{Map, Value};

// ── Public entry point ─────────────────────────────────────────────

/// Run a read-only cells introspection query against the cell graph.
///
/// `state` is the live cell store (typically `tenant.read().snapshot_d()`
/// in the engine path or the loaded `D` in the CLI path).
///
/// Returns a JSON envelope sized to the requested mode:
///
///   {"cells":[{"name":"…","size_bytes":N}, ...]}            list
///   {"name":"…","contents":[…parsed tuples…],"size_bytes":N} get
///   {"rule_text":"…","consequent_cell":"…","materialized_count":N} trace
///   {"error":"<message>"}                                    parse / lookup failure
pub fn cells_query(state: &Object, input: &str) -> String {
    let parsed: Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => return error_envelope(&format!("input must be JSON: {}", e)),
    };
    let mode = parsed.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    match mode {
        "list" => {
            let pattern = parsed.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
            list_cells(state, pattern)
        }
        "get" => {
            let Some(name) = parsed.get("name").and_then(|v| v.as_str()) else {
                return error_envelope("get mode requires `name`");
            };
            get_cell(state, name)
        }
        "trace" => {
            let rule_id = parsed.get("rule_id").and_then(|v| v.as_str());
            let rule_pat = parsed.get("rule_pattern").and_then(|v| v.as_str());
            match (rule_id, rule_pat) {
                (Some(id), _) => trace_rule_by_id(state, id),
                (None, Some(p)) => trace_rule_by_pattern(state, p),
                (None, None) => error_envelope("trace mode requires `rule_id` or `rule_pattern`"),
            }
        }
        "" => error_envelope("input must include `mode`: one of \"list\", \"get\", \"trace\""),
        other => error_envelope(&format!("unknown mode: {}", other)),
    }
}

// ── list ───────────────────────────────────────────────────────────

/// List cells matching a glob-style pattern (`*` and `?` wildcards).
/// Empty pattern or `*` returns every cell. Each entry carries the
/// FFP-encoded size on disk so callers can find oversized cells.
fn list_cells(state: &Object, pattern: &str) -> String {
    let mut entries: Vec<(String, usize)> = Vec::new();
    for (name, contents) in ast::cells_iter(state) {
        if !glob_match(pattern, name) {
            continue;
        }
        let size = render_contents(contents).len();
        entries.push((name.to_string(), size));
    }
    // Stable sort by cell name for deterministic envelopes — the
    // BTreeMap-backed Object::Map already iterates in name order, so
    // this only matters for legacy Seq-shaped stores.
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let cells_arr: Vec<Value> = entries.into_iter().map(|(n, sz)| {
        let mut m = Map::with_capacity(2);
        m.insert("name".into(), Value::String(n));
        m.insert("size_bytes".into(), Value::from(sz as u64));
        Value::Object(m)
    }).collect();
    let mut env = Map::with_capacity(1);
    env.insert("cells".into(), Value::Array(cells_arr));
    Value::Object(env).to_string()
}

// ── get ────────────────────────────────────────────────────────────

/// Fetch a single cell by name. Returns an error envelope when the
/// cell is absent rather than `{contents:[]}` so callers can
/// distinguish "no such cell" from "empty cell". Contents are
/// rendered as a JSON array of parsed tuples — each fact becomes
/// either an object (when every entry is a `<key,value>` pair, the
/// common FactType-cell shape) or a tagged array (when the fact
/// isn't pair-shaped, e.g. defs cells).
fn get_cell(state: &Object, name: &str) -> String {
    let contents = ast::fetch(name, state);
    if matches!(contents, Object::Bottom) {
        return error_envelope(&format!("no such cell: {}", name));
    }
    let rendered = render_contents(&contents);
    let size = rendered.len();
    let parsed_contents = render_as_json(&contents);

    let mut env = Map::with_capacity(3);
    env.insert("name".into(), Value::String(name.to_string()));
    env.insert("contents".into(), parsed_contents);
    env.insert("size_bytes".into(), Value::from(size as u64));
    Value::Object(env).to_string()
}

// ── trace ──────────────────────────────────────────────────────────

/// Trace a derivation rule by its `id` field on the `DerivationRule`
/// cell. Returns the rule text, the FT-id of the consequent cell, and
/// the current row count of that cell (so callers can verify the rule
/// actually fired during the last forward-chain pass).
fn trace_rule_by_id(state: &Object, rule_id: &str) -> String {
    let dr = ast::fetch_or_phi("DerivationRule", state);
    let rule = dr.as_seq()
        .and_then(|facts| facts.iter().find(|f| ast::binding(f, "id") == Some(rule_id)).cloned());
    match rule {
        Some(r) => trace_envelope(state, &r),
        None => error_envelope(&format!("no derivation rule with id={}", rule_id)),
    }
}

/// Trace by substring match on the rule text. Returns the FIRST match
/// (text matching is approximate by design — patterns like "is
/// overdue" pick whichever rule exposes that phrase).
fn trace_rule_by_pattern(state: &Object, pattern: &str) -> String {
    let dr = ast::fetch_or_phi("DerivationRule", state);
    let rule = dr.as_seq()
        .and_then(|facts| facts.iter().find(|f| {
            ast::binding(f, "text").map_or(false, |t| t.contains(pattern))
        }).cloned());
    match rule {
        Some(r) => trace_envelope(state, &r),
        None => error_envelope(&format!("no derivation rule matches pattern: {}", pattern)),
    }
}

fn trace_envelope(state: &Object, rule: &Object) -> String {
    let text = ast::binding(rule, "text").unwrap_or("").to_string();
    let consequent = ast::binding(rule, "consequentFactTypeId").unwrap_or("").to_string();
    let materialized = if consequent.is_empty() {
        0
    } else {
        ast::fetch_or_phi(&consequent, state)
            .as_seq().map(|s| s.len()).unwrap_or(0)
    };
    let mut env = Map::with_capacity(3);
    env.insert("rule_text".into(), Value::String(text));
    env.insert("consequent_cell".into(), Value::String(consequent));
    env.insert("materialized_count".into(), Value::from(materialized as u64));
    Value::Object(env).to_string()
}

// ── Helpers ────────────────────────────────────────────────────────

/// Glob match supporting `*` (any sequence, including empty) and `?`
/// (single char). No bracket classes — the verb only needs the simple
/// CLI-style globbing that `Task_*` and `*derivation*` cover. Anchored
/// at both ends (no implicit start/end wildcards).
fn glob_match(pattern: &str, candidate: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    glob_match_inner(&p, 0, &c, 0)
}

fn glob_match_inner(p: &[char], pi: usize, c: &[char], ci: usize) -> bool {
    if pi == p.len() {
        return ci == c.len();
    }
    match p[pi] {
        '*' => {
            // Match zero or more chars. Try increasing match lengths.
            for skip in ci..=c.len() {
                if glob_match_inner(p, pi + 1, c, skip) {
                    return true;
                }
            }
            false
        }
        '?' => ci < c.len() && glob_match_inner(p, pi + 1, c, ci + 1),
        ch => ci < c.len() && c[ci] == ch && glob_match_inner(p, pi + 1, c, ci + 1),
    }
}

/// Render a cell's contents using the existing FFP `Display` impl so
/// the size we report matches what a `sqlite3 cells` query would
/// return for `length(contents)`.
fn render_contents(contents: &Object) -> String {
    contents.to_string()
}

/// Convert a cell's contents into a JSON value friendly for MCP
/// callers. Pair-shaped facts become objects ({"role": "value", ...});
/// other shapes (atoms, deeper nesting) fall back to the FFP string
/// representation so nothing is lost.
fn render_as_json(contents: &Object) -> Value {
    match contents {
        Object::Seq(items) => {
            let arr: Vec<Value> = items.iter().map(render_fact).collect();
            Value::Array(arr)
        }
        Object::Atom(s) => Value::String(s.clone()),
        Object::Map(_) | Object::Bottom => Value::String(contents.to_string()),
    }
}

fn render_fact(fact: &Object) -> Value {
    let Some(pairs) = fact.as_seq() else {
        return Value::String(fact.to_string());
    };
    let mut all_pairs = true;
    let mut map = Map::with_capacity(pairs.len());
    for p in pairs {
        let Some(kv) = p.as_seq() else { all_pairs = false; break; };
        if kv.len() != 2 { all_pairs = false; break; }
        let Some(k) = kv[0].as_atom() else { all_pairs = false; break; };
        // Value can be atom or nested seq — stringify either way so
        // callers get something legible without losing structure.
        let v = match &kv[1] {
            Object::Atom(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        };
        map.insert(k.to_string(), v);
    }
    if all_pairs {
        Value::Object(map)
    } else {
        // Mixed shape (defs cell, function rep, etc.) — fall back to
        // the raw FFP string so nothing is lost.
        Value::String(fact.to_string())
    }
}

fn error_envelope(message: &str) -> String {
    let mut m = Map::with_capacity(1);
    m.insert("error".into(), Value::String(message.to_string()));
    Value::Object(m).to_string()
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell_push, fact_from_pairs};

    fn parse_envelope(envelope: &str) -> Value {
        serde_json::from_str(envelope)
            .unwrap_or_else(|_| panic!("envelope must be JSON, got: {}", envelope))
    }

    fn parse_error(envelope: &str) -> String {
        let v = parse_envelope(envelope);
        v.get("error").and_then(|r| r.as_str()).map(String::from)
            .unwrap_or_else(|| panic!("envelope must have error, got: {}", envelope))
    }

    fn state_with_tasks() -> Object {
        let mut state = Object::phi();
        state = cell_push("FactType",
            fact_from_pairs(&[("id", "Task_has_Task_Priority")]), &state);
        state = cell_push("FactType",
            fact_from_pairs(&[("id", "Task_has_Task_Status")]), &state);
        state = cell_push("Task_has_Task_Priority",
            fact_from_pairs(&[("Task", "1"), ("Task Priority", "p0")]), &state);
        state = cell_push("Task_has_Task_Priority",
            fact_from_pairs(&[("Task", "2"), ("Task Priority", "p1")]), &state);
        state = cell_push("Task_has_Task_Status",
            fact_from_pairs(&[("Task", "1"), ("Task Status", "ready")]), &state);
        state
    }

    // ── input shape ────────────────────────────────────────────────

    #[test]
    fn rejects_non_json_input() {
        let env = cells_query(&Object::phi(), "this is not json");
        assert!(parse_error(&env).contains("JSON"));
    }

    #[test]
    fn rejects_missing_mode() {
        let env = cells_query(&Object::phi(), r#"{"pattern": "*"}"#);
        assert!(parse_error(&env).contains("mode"));
    }

    #[test]
    fn rejects_unknown_mode() {
        let env = cells_query(&Object::phi(), r#"{"mode": "frobnicate"}"#);
        assert!(parse_error(&env).contains("unknown mode"));
    }

    // ── list mode ──────────────────────────────────────────────────

    #[test]
    fn list_returns_every_cell_when_pattern_is_star() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"list","pattern":"*"}"#);
        let v = parse_envelope(&env);
        let cells = v.get("cells").and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("expected cells array, got: {}", env));
        // FactType + Task_has_Task_Priority + Task_has_Task_Status
        assert_eq!(cells.len(), 3, "envelope: {}", env);
    }

    #[test]
    fn list_filters_by_glob_prefix() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"list","pattern":"Task_*"}"#);
        let v = parse_envelope(&env);
        let cells = v.get("cells").and_then(|c| c.as_array()).expect("cells array");
        let names: Vec<String> = cells.iter()
            .map(|c| c.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string())
            .collect();
        assert_eq!(names, vec!["Task_has_Task_Priority", "Task_has_Task_Status"],
            "expected only Task_-prefixed cells; got: {:?}", names);
    }

    #[test]
    fn list_filters_by_glob_substring() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"list","pattern":"*Priority*"}"#);
        let v = parse_envelope(&env);
        let cells = v.get("cells").and_then(|c| c.as_array()).expect("cells array");
        assert_eq!(cells.len(), 1, "expected 1 cell with Priority substring; got: {}", env);
        assert_eq!(cells[0].get("name").and_then(|n| n.as_str()),
            Some("Task_has_Task_Priority"));
    }

    #[test]
    fn list_returns_size_bytes_per_cell() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"list","pattern":"Task_has_Task_Priority"}"#);
        let v = parse_envelope(&env);
        let cells = v.get("cells").and_then(|c| c.as_array()).expect("cells array");
        let sz = cells[0].get("size_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
        assert!(sz > 0, "size_bytes should be non-zero for a populated cell; got: {}", env);
    }

    #[test]
    fn list_empty_for_no_match() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"list","pattern":"NoSuchPrefix*"}"#);
        let v = parse_envelope(&env);
        let cells = v.get("cells").and_then(|c| c.as_array()).expect("cells array");
        assert!(cells.is_empty(), "expected empty list, got: {}", env);
    }

    // ── get mode ───────────────────────────────────────────────────

    #[test]
    fn get_returns_parsed_tuple_list() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"get","name":"Task_has_Task_Priority"}"#);
        let v = parse_envelope(&env);
        assert_eq!(v.get("name").and_then(|n| n.as_str()),
            Some("Task_has_Task_Priority"));
        let contents = v.get("contents").and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("expected contents array, got: {}", env));
        assert_eq!(contents.len(), 2);
        // Each fact becomes an object keyed by role name.
        assert_eq!(contents[0].get("Task").and_then(|t| t.as_str()), Some("1"));
        assert_eq!(contents[0].get("Task Priority").and_then(|t| t.as_str()), Some("p0"));
    }

    #[test]
    fn get_includes_size_bytes() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"get","name":"Task_has_Task_Status"}"#);
        let v = parse_envelope(&env);
        let sz = v.get("size_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
        assert!(sz > 0, "size_bytes missing or zero, envelope: {}", env);
    }

    #[test]
    fn get_returns_error_for_missing_cell() {
        let state = state_with_tasks();
        let env = cells_query(&state, r#"{"mode":"get","name":"NoSuchCell"}"#);
        assert!(parse_error(&env).contains("no such cell"));
    }

    #[test]
    fn get_requires_name() {
        let env = cells_query(&Object::phi(), r#"{"mode":"get"}"#);
        assert!(parse_error(&env).contains("name"));
    }

    // ── trace mode ─────────────────────────────────────────────────

    fn state_with_derivation_rule() -> Object {
        let mut state = Object::phi();
        state = cell_push("DerivationRule", fact_from_pairs(&[
            ("id", "rule_overdue_001"),
            ("text", "Task is overdue iff Task has Due Date before today"),
            ("consequentFactTypeId", "Task_is_overdue"),
        ]), &state);
        state = cell_push("DerivationRule", fact_from_pairs(&[
            ("id", "rule_blocked_002"),
            ("text", "Task is blocked iff Task depends on incomplete Task"),
            ("consequentFactTypeId", "Task_is_blocked"),
        ]), &state);
        // Materialise some facts for the consequent cell so
        // materialized_count is observable.
        state = cell_push("Task_is_overdue",
            fact_from_pairs(&[("Task", "5")]), &state);
        state = cell_push("Task_is_overdue",
            fact_from_pairs(&[("Task", "6")]), &state);
        state = cell_push("Task_is_overdue",
            fact_from_pairs(&[("Task", "7")]), &state);
        state
    }

    #[test]
    fn trace_by_rule_id_returns_text_and_consequent() {
        let state = state_with_derivation_rule();
        let env = cells_query(&state, r#"{"mode":"trace","rule_id":"rule_overdue_001"}"#);
        let v = parse_envelope(&env);
        assert_eq!(v.get("rule_text").and_then(|t| t.as_str()),
            Some("Task is overdue iff Task has Due Date before today"));
        assert_eq!(v.get("consequent_cell").and_then(|c| c.as_str()),
            Some("Task_is_overdue"));
        assert_eq!(v.get("materialized_count").and_then(|n| n.as_u64()), Some(3));
    }

    #[test]
    fn trace_by_pattern_returns_first_match() {
        let state = state_with_derivation_rule();
        let env = cells_query(&state, r#"{"mode":"trace","rule_pattern":"depends on incomplete"}"#);
        let v = parse_envelope(&env);
        assert_eq!(v.get("consequent_cell").and_then(|c| c.as_str()),
            Some("Task_is_blocked"));
    }

    #[test]
    fn trace_unknown_rule_id_returns_error() {
        let state = state_with_derivation_rule();
        let env = cells_query(&state, r#"{"mode":"trace","rule_id":"rule_does_not_exist"}"#);
        assert!(parse_error(&env).contains("no derivation rule"));
    }

    #[test]
    fn trace_requires_rule_id_or_pattern() {
        let env = cells_query(&Object::phi(), r#"{"mode":"trace"}"#);
        assert!(parse_error(&env).contains("rule_id"));
    }

    // ── glob_match unit coverage ───────────────────────────────────

    #[test]
    fn glob_anchored_at_both_ends() {
        assert!(glob_match("foo", "foo"));
        assert!(!glob_match("foo", "foobar"));
        assert!(!glob_match("foo", "barfoo"));
    }

    #[test]
    fn glob_star_matches_any_sequence() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("Task_*", "Task_has_Status"));
        assert!(glob_match("*Status", "Task_has_Status"));
        assert!(glob_match("*", "x"));
    }

    #[test]
    fn glob_question_matches_single_char() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "abbc"));
    }
}
