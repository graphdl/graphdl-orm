// crates/arest/src/orient.rs
//
// Read-only session re-orientation verb (#871).
//
// Per #869 / north-star: a fresh agent session re-discovers the
// landscape by issuing 5-6 separate verbs (which apps exist, which
// is active, what's in flight, what derivation rules just fired).
// `orient` collapses that into one envelope so the second LLM turn
// can be productive instead of probing.
//
// Sister to `cells` (#870) and `sql` (#864): all three are pure
// projections over the cell graph. Where `cells` answers "what's in
// this cell" and `sql` answers "join across these cells", `orient`
// answers "what's the situation right now".
//
// Input envelope (all fields optional):
//   {
//     "apps_dir":   "/abs/path/to/apps",   // when present + local feature
//                                            // → enumerate sibling apps
//     "active_app": "tasks"                  // hint for the suggested_next
//                                            // template
//   }
//
// Output envelope:
//   {
//     "apps":            [{name, root, last_compile, ready_count,
//                          in_progress_count, completed_count}, ...],
//     "active_app":      "tasks" | null,
//     "recent_changes":  [{kind, ...}, ...],
//     "suggested_next":  "Try: ..."
//   }
//
// Counts come from the core `Resource_is_currently_in_Status` cell —
// the SM-derived latest-wins status, NOT the slop `Task_has_Task_Status`
// bridge — in the active app's snapshot (the only DB the engine has
// loaded). Other apps in
// `apps_dir` surface as bare entries with `last_compile` from their
// .db file mtime — no row counts (the engine has one snapshot at a
// time). This keeps the verb cheap and predictable; agents that need
// per-app task counts can `apps_use` then `orient` again.
//
// std-deps gated for serde_json envelope shaping. no_std builds get
// a structured "feature unavailable" diagnostic from the lib.rs
// intercept.

#![cfg(feature = "std-deps")]

use crate::ast::{self, Object};
use serde_json::{Map, Value};

// ── Public entry point ─────────────────────────────────────────────

/// Build a one-screen re-orientation envelope from the engine snapshot
/// and an optional sibling-apps directory.
///
/// `state` is the live cell store (typically `tenant.read().snapshot_d()`
/// in the engine path or the loaded `D` in the CLI path).
///
/// `input` is a JSON envelope; missing fields fall back to defaults.
/// Malformed JSON returns `{"error": "..."}` rather than panicking
/// — the verb is the agent's recovery path when the rest of the
/// session is in a confusing state, so it must never throw.
pub fn orient(state: &Object, input: &str) -> String {
    let parsed: Value = if input.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        match serde_json::from_str(input) {
            Ok(v) => v,
            Err(e) => return error_envelope(&format!("input must be JSON: {}", e)),
        }
    };
    let apps_dir = parsed.get("apps_dir").and_then(|v| v.as_str());
    let active_app = parsed.get("active_app").and_then(|v| v.as_str());

    let task_counts = compute_task_counts(state);
    let mut apps_arr: Vec<Value> = Vec::new();

    // Always surface the active app first (counts available because
    // the engine has its snapshot loaded).
    if let Some(name) = active_app {
        let mut entry = Map::with_capacity(6);
        entry.insert("name".into(), Value::String(name.to_string()));
        entry.insert("root".into(), Value::String(
            apps_dir.map(|d| join_path(d, name)).unwrap_or_default()));
        entry.insert("last_compile".into(), Value::Null);
        entry.insert("ready_count".into(), Value::from(task_counts.ready));
        entry.insert("in_progress_count".into(), Value::from(task_counts.in_progress));
        entry.insert("completed_count".into(), Value::from(task_counts.completed));
        apps_arr.push(Value::Object(entry));
    }

    // Sibling apps via filesystem scan (only when the host gave us
    // an apps_dir AND the local feature is on so we can stat files).
    if let Some(dir) = apps_dir {
        for sibling in scan_sibling_apps(dir, active_app) {
            apps_arr.push(sibling);
        }
    }

    let recent = recent_changes(state);
    // claude-stack: a stack-bearing app re-points the suggestion at its OWN
    // ledger (what/why/how is in the `stack` field) instead of the generic
    // tasks-board stub — `tasks` is a SEPARATE app, not part of every app.
    let stack = stack_overview(state, active_app);
    let lessons = lessons_overview(state);
    let suggestion = match &stack {
        Some(_) => "This app is a cognitive stack — the `stack` field gives what/why/how. \
            Next: query and OBEY the operational ledger — Engineering_Lever_has_Lever_Status \
            (current work), Operating_Rule_has_Rule_Statement (standing rules), Engine_Lesson \
            (durable technical facts).".to_string(),
        None => suggested_next(active_app, &task_counts),
    };

    let mut env = Map::with_capacity(6);
    env.insert("apps".into(), Value::Array(apps_arr));
    env.insert("active_app".into(), match active_app {
        Some(s) => Value::String(s.to_string()),
        None => Value::Null,
    });
    env.insert("recent_changes".into(), Value::Array(recent));
    env.insert("suggested_next".into(), Value::String(suggestion));
    if let Some(s) = stack { env.insert("stack".into(), s); }
    if let Some(l) = lessons { env.insert("lessons".into(), l); }
    Value::Object(env).to_string()
}

/// stack-self-description: surface the active app's Purpose / Rationale /
/// Usage and its Stack Layers, so `orient` tells a re-entering session WHAT
/// the stack does, WHY it is built this way, and HOW to wield it — not a
/// generic task pointer. Reads the facts the app declares (App has
/// Purpose/Rationale/Usage, App includes Stack Layer, Stack Layer has Layer
/// Description / is sourced from App). Returns None for apps declaring none.
fn stack_overview(state: &Object, active_app: Option<&str>) -> Option<Value> {
    let app = active_app?;
    let one = |cell: &str, val_role: &str| -> Option<String> {
        let c = crate::ast::fetch_or_phi(cell, state);
        for f in crate::ast::cell_facts_iter(&c) {
            if crate::ast::binding(f, "App") == Some(app) {
                if let Some(v) = crate::ast::binding(f, val_role) { return Some(v.to_string()); }
            }
        }
        None
    };
    let field = |cell: &Object, layer: &str, role: &str| -> Option<String> {
        for f in crate::ast::cell_facts_iter(cell) {
            if crate::ast::binding(f, "Stack Layer") == Some(layer) {
                if let Some(v) = crate::ast::binding(f, role) { return Some(v.to_string()); }
            }
        }
        None
    };
    let inc = crate::ast::fetch_or_phi("App_includes_Stack_Layer", state);
    let mut layer_names: Vec<String> = Vec::new();
    for f in crate::ast::cell_facts_iter(&inc) {
        if crate::ast::binding(f, "App") == Some(app) {
            if let Some(l) = crate::ast::binding(f, "Stack Layer") { layer_names.push(l.to_string()); }
        }
    }
    let desc = crate::ast::fetch_or_phi("Stack_Layer_has_Layer_Description", state);
    let src = crate::ast::fetch_or_phi("Stack_Layer_is_sourced_from_App", state);
    let layers: Vec<Value> = layer_names.iter().map(|n| {
        let mut m = Map::new();
        m.insert("layer".into(), Value::String(n.clone()));
        if let Some(d) = field(&desc, n, "Layer Description") { m.insert("description".into(), Value::String(d)); }
        if let Some(s) = field(&src, n, "App") { m.insert("from".into(), Value::String(s)); }
        Value::Object(m)
    }).collect();
    let purpose = one("App_has_Purpose", "Purpose");
    if purpose.is_none() && layers.is_empty() { return None; }
    let mut m = Map::new();
    if let Some(p) = purpose { m.insert("purpose".into(), Value::String(p)); }
    if let Some(r) = one("App_has_Rationale", "Rationale") { m.insert("rationale".into(), Value::String(r)); }
    if let Some(u) = one("App_has_Usage", "Usage") { m.insert("usage".into(), Value::String(u)); }
    if !layers.is_empty() { m.insert("layers".into(), Value::Array(layers)); }
    Some(Value::Object(m))
}

/// lessons-surfacing: surface the active app's Engine Lessons so a re-entering
/// session sees the durable do's (prescribed Constructions) and don'ts (Reading
/// Traps with their trigger phrases) WITHOUT re-deriving them from a compaction
/// summary. Reads Engine_Lesson_has_Lesson_Kind (grouped), prescribes
/// Construction, and Reading_Trap_is_triggered_by_Trigger_Phrase. None when the
/// app declares no lessons (so non-ledger apps stay unaffected).
fn lessons_overview(state: &Object) -> Option<Value> {
    use std::collections::BTreeMap;
    let kinds = crate::ast::fetch_or_phi("Engine_Lesson_has_Lesson_Kind", state);
    let mut by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in crate::ast::cell_facts_iter(&kinds) {
        if let (Some(id), Some(kind)) =
            (crate::ast::binding(f, "Engine Lesson"), crate::ast::binding(f, "Lesson Kind"))
        {
            by_kind.entry(kind.to_string()).or_default().push(id.to_string());
        }
    }
    if by_kind.is_empty() { return None; }
    let count: u64 = by_kind.values().map(|v| v.len() as u64).sum();
    let mut kinds_obj = Map::new();
    for (k, mut v) in by_kind {
        v.sort();
        v.dedup();
        kinds_obj.insert(k, Value::Array(v.into_iter().map(Value::String).collect()));
    }

    let cons = crate::ast::fetch_or_phi("Engine_Lesson_prescribes_Construction", state);
    let mut prescribes: Vec<String> = Vec::new();
    for f in crate::ast::cell_facts_iter(&cons) {
        if let Some(c) = crate::ast::binding(f, "Construction") { prescribes.push(c.to_string()); }
    }
    prescribes.sort();
    prescribes.dedup();

    let traps = crate::ast::fetch_or_phi("Reading_Trap_is_triggered_by_Trigger_Phrase", state);
    let mut avoid: Vec<Value> = Vec::new();
    for f in crate::ast::cell_facts_iter(&traps) {
        if let (Some(t), Some(p)) =
            (crate::ast::binding(f, "Reading Trap"), crate::ast::binding(f, "Trigger Phrase"))
        {
            let mut tm = Map::new();
            tm.insert("trap".into(), Value::String(t.to_string()));
            tm.insert("trigger".into(), Value::String(p.to_string()));
            avoid.push(Value::Object(tm));
        }
    }

    let mut m = Map::new();
    m.insert("count".into(), Value::from(count));
    m.insert("by_kind".into(), Value::Object(kinds_obj));
    if !prescribes.is_empty() {
        m.insert("prescribes".into(), Value::Array(prescribes.into_iter().map(Value::String).collect()));
    }
    if !avoid.is_empty() { m.insert("avoid".into(), Value::Array(avoid)); }
    Some(Value::Object(m))
}

// ── Task counts ────────────────────────────────────────────────────

#[derive(Default, Clone, Copy)]
struct TaskCounts {
    ready: u64,
    in_progress: u64,
    completed: u64,
}

/// Count tasks by their CURRENT state-machine status, read from the
/// CORE object `Resource_is_currently_in_Status` (the SM event-fold,
/// latest-wins) — NOT the denormalized `Task_has_Task_Status` bridge,
/// which is a "slop" cell that accumulates un-retracted direct facts
/// and drifts from SM truth (orient-reorient-snapshot). The tasks
/// domain's SM statuses are pending / in_progress / blocked /
/// completed / deleted; `ready` here means "actionable" = pending.
/// blocked / deleted fall into none of the three counters — the
/// suggested_next template handles the "no tasks at all" path.
fn compute_task_counts(state: &Object) -> TaskCounts {
    // `Resource_is_currently_in_Status` (roles: Resource, Status) can be
    // folded to `Object::Map` by the materializer; `fetch_cell_seq`
    // flattens Map -> key-sorted Seq (and passes a legacy Seq through)
    // so a raw `as_seq()` does not silently zero the counts.
    let cell = ast::fetch_cell_seq("Resource_is_currently_in_Status", state);
    let Some(facts) = cell.as_seq() else { return TaskCounts::default() };
    let mut counts = TaskCounts::default();
    for fact in facts {
        let Some(status) = ast::binding(fact, "Status") else { continue };
        match status {
            "pending" => counts.ready += 1,
            "in_progress" | "in-progress" => counts.in_progress += 1,
            "completed" => counts.completed += 1,
            _ => {}
        }
    }
    counts
}

// ── Sibling apps ───────────────────────────────────────────────────

/// Scan `apps_dir` for sub-directories that look like AREST apps
/// (carry a `readings/` directory and a `*.db` file). Returns one
/// entry per non-active sibling. Active app is skipped here because
/// the caller already pushed it with live counts.
///
/// Rust-side filesystem scan is gated on the `local` feature — same
/// gate as `cli::entry`'s `db::open`. Builds without the feature
/// (Cloudflare worker, kernel) return an empty list rather than
/// failing; the engine has no filesystem there to scan.
fn scan_sibling_apps(apps_dir: &str, active_app: Option<&str>) -> Vec<Value> {
    #[cfg(feature = "local")]
    {
        scan_sibling_apps_local(apps_dir, active_app)
    }
    #[cfg(not(feature = "local"))]
    {
        let _ = (apps_dir, active_app);
        Vec::new()
    }
}

#[cfg(feature = "local")]
fn scan_sibling_apps_local(apps_dir: &str, active_app: Option<&str>) -> Vec<Value> {
    use std::fs;
    let mut out = Vec::new();
    let entries = match fs::read_dir(apps_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    let mut dirs: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if name.starts_with('.') {
            continue;
        }
        if Some(name) == active_app {
            continue;
        }
        let readings = path.join("readings");
        if !readings.is_dir() {
            continue;
        }
        let db = discover_db(&path);
        if db.is_none() {
            continue;
        }
        dirs.push((name.to_string(), path));
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, root) in dirs {
        let mut entry = Map::with_capacity(6);
        let last_compile = discover_db(&root)
            .and_then(|db| fs::metadata(&db).ok())
            .and_then(|m| m.modified().ok())
            .map(format_systemtime)
            .unwrap_or_default();
        entry.insert("name".into(), Value::String(name));
        entry.insert("root".into(), Value::String(root.to_string_lossy().into_owned()));
        entry.insert("last_compile".into(), if last_compile.is_empty() {
            Value::Null
        } else {
            Value::String(last_compile)
        });
        // No counts for sibling apps — the engine only has one
        // snapshot loaded. Agents call `apps_use` to switch.
        entry.insert("ready_count".into(), Value::Null);
        entry.insert("in_progress_count".into(), Value::Null);
        entry.insert("completed_count".into(), Value::Null);
        out.push(Value::Object(entry));
    }
    out
}

#[cfg(feature = "local")]
fn discover_db(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = root.file_name().and_then(|n| n.to_str())?;
    let candidates = [
        root.join(format!("{}.db", name)),
        root.join(format!("{}-arest.db", name)),
        root.join("app.db"),
        root.join("arest.db"),
    ];
    for c in &candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    None
}

#[cfg(feature = "local")]
fn format_systemtime(t: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => format_iso8601(d.as_secs()),
        Err(_) => String::new(),
    }
}

#[cfg(feature = "local")]
fn format_iso8601(unix_secs: u64) -> String {
    // Hand-rolled UTC formatter — avoids pulling in chrono. Civil
    // calendar conversion via Howard Hinnant's algorithm
    // (date.html#civil_from_days). Adequate for last_compile labels;
    // not for sub-second precision or timezone-aware reasoning.
    let days = (unix_secs / 86400) as i64;
    let secs_in_day = unix_secs % 86400;
    let h = secs_in_day / 3600;
    let m = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

#[cfg(feature = "local")]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // From http://howardhinnant.github.io/date_algorithms.html
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = y + (if m <= 2 { 1 } else { 0 });
    (y, m, d)
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

// ── Recent changes ─────────────────────────────────────────────────

/// Surface the most recently active cells as a recent-changes list.
/// Reads `cells_iter` (current cell graph), keeps the last few cells
/// that look like instance fact types (rather than schema scaffolding),
/// and emits one entry per cell with a `kind=apply` envelope so the
/// suggested_next template can fall back when none are present.
///
/// This is a deliberate simplification: the spec called out "skip git
/// tail if too involved; surface what's easy". Cell-level recency is
/// what the snapshot exposes without a separate apply log.
fn recent_changes(state: &Object) -> Vec<Value> {
    let mut out = Vec::new();
    // Look for instance fact cells (named like `Task_has_Task_Status`,
    // `Task_has_Task_Priority`) — they carry the freshly-applied facts.
    // Schema cells, derivation indices, and generator scaffolding are
    // not session activity and would crowd the envelope.
    const SCHEMA_CELLS: &[&str] = &[
        "FactType", "NounType", "DerivationRule", "Domain", "Verb",
        "DefinitionalRule", "AlethicConstraint", "DeonticRule",
        "ScoringRule", "JoinClause", "RingPattern", "EnumValues",
        "Reading", "Role", "Subtype", "ConceptualPredicate",
    ];
    // Generator + scaffolding cells namespace themselves with a `:`
    // (sql:sqlite:Task, derivation:..., html:Max Length, plix:Grid Cell,
    // linq:..., shard:..., nav:..., openapi:..., wsdl:..., dsl:..., …).
    // Canonical instance fact cells are FactType IDs, which by design
    // contain only ASCII letters, digits, and `_` (`Task_has_Task_Status`).
    // The single colon discriminator catches every generator emit
    // without us having to enumerate every generator namespace.
    for (name, contents) in ast::cells_iter(state) {
        if SCHEMA_CELLS.contains(&name) {
            continue;
        }
        if name.contains(':') {
            continue;
        }
        // orient-reorient-snapshot: skip MIS-FILED cells whose names are not
        // canonical FactType ids. Unresolved / multi-fact-line tuples get
        // mis-filed under a raw-verb fragment ("maps agent-", "has auditTask
        // 'ft-", "are for the same", "maps to 'Arousal positive'"), which then
        // surfaced as garbage recent-activity nouns. A canonical FT id is ASCII
        // letters, digits, and '_' only; a name with a space, quote, or other
        // punctuation is mis-filed scaffolding, not session activity.
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        // #932 phase-2: collect via cell_facts_iter so a folded (Map)
        // cell is counted too, not skipped as a non-Seq.
        let facts: Vec<_> = crate::ast::cell_facts_iter(contents).collect();
        if facts.is_empty() {
            continue;
        }
        let mut entry = Map::with_capacity(3);
        entry.insert("kind".into(), Value::String("apply".into()));
        entry.insert("noun".into(), Value::String(name.to_string()));
        entry.insert("count".into(), Value::from(facts.len() as u64));
        out.push(Value::Object(entry));
    }
    // Cap at a manageable number — agents read the envelope in one
    // glance, not 200 entries deep.
    out.truncate(10);
    out
}

// ── Suggested next ─────────────────────────────────────────────────

/// One-line pointer for the agent's next move. Templates intentionally
/// pick the highest-leverage suggestion based on what the snapshot
/// shows; agents still have full discretion to ignore it.
fn suggested_next(active_app: Option<&str>, counts: &TaskCounts) -> String {
    match active_app {
        None => {
            "Try: mcp__arest__apps_list to enumerate available apps, then \
             mcp__arest__apps_use <name> to activate one.".to_string()
        }
        Some(app) if counts.ready > 0 => {
            format!(
                "Try: mcp__arest__query Task_is_recommended in app '{}' for \
                 the launch-candidate set; consult readings/app.md for schema.",
                app
            )
        }
        Some(app) if counts.in_progress > 0 => {
            format!(
                "Try: mcp__arest__query Task_has_Task_Status in app '{}' \
                 with filter Task_Status=in_progress to see what's already \
                 underway before starting new work.",
                app
            )
        }
        Some(app) => {
            format!(
                "Try: mcp__arest__query Task_has_Task_Status in app '{}' to \
                 inspect the population; consult readings/app.md for schema.",
                app
            )
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

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

    fn state_with_tasks() -> Object {
        let mut state = Object::phi();
        // Seed FactType so the schema cell exists.
        state = cell_push("FactType",
            fact_from_pairs(&[("id", "Task_has_Task_Status")]), &state);
        // 3 pending (=ready), 2 in_progress, 4 completed — on the CORE
        // `Resource_is_currently_in_Status` cell (the new count source).
        for i in 1..=3 {
            state = cell_push("Resource_is_currently_in_Status",
                fact_from_pairs(&[
                    ("Resource", &format!("ready-{}", i)),
                    ("Status", "pending"),
                ]), &state);
        }
        for i in 1..=2 {
            state = cell_push("Resource_is_currently_in_Status",
                fact_from_pairs(&[
                    ("Resource", &format!("inprog-{}", i)),
                    ("Status", "in_progress"),
                ]), &state);
        }
        for i in 1..=4 {
            state = cell_push("Resource_is_currently_in_Status",
                fact_from_pairs(&[
                    ("Resource", &format!("done-{}", i)),
                    ("Status", "completed"),
                ]), &state);
        }
        // Stale slop row the counts MUST ignore: if compute_task_counts
        // regressed to reading `Task_has_Task_Status`, this would wrongly
        // bump in_progress to 3 (regression guard for orient-reorient-snapshot).
        state = cell_push("Task_has_Task_Status",
            fact_from_pairs(&[("Task", "stale-1"), ("Task Status", "in_progress")]), &state);
        state
    }

    // ── input shape ────────────────────────────────────────────────

    #[test]
    fn empty_input_returns_an_envelope_with_apps_active_recent_suggested() {
        let env = orient(&Object::phi(), "");
        let v = parse_envelope(&env);
        // All four top-level keys present even when the snapshot is empty.
        assert!(v.get("apps").is_some(), "missing apps: {}", env);
        assert!(v.get("active_app").is_some(), "missing active_app: {}", env);
        assert!(v.get("recent_changes").is_some(), "missing recent_changes: {}", env);
        assert!(v.get("suggested_next").is_some(), "missing suggested_next: {}", env);
    }

    #[test]
    fn rejects_non_json_input() {
        let env = orient(&Object::phi(), "this is not json");
        let v = parse_envelope(&env);
        assert!(v.get("error").is_some(), "expected error envelope, got: {}", env);
    }

    // ── task counts ────────────────────────────────────────────────

    #[test]
    fn counts_ready_in_progress_and_completed_for_active_app() {
        let state = state_with_tasks();
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let apps = v.get("apps").and_then(|a| a.as_array())
            .unwrap_or_else(|| panic!("expected apps array, got: {}", env));
        assert_eq!(apps.len(), 1, "expected single (active) app entry, got: {}", env);
        let active = &apps[0];
        assert_eq!(active.get("name").and_then(|n| n.as_str()), Some("tasks"));
        assert_eq!(active.get("ready_count").and_then(|n| n.as_u64()), Some(3));
        assert_eq!(active.get("in_progress_count").and_then(|n| n.as_u64()), Some(2));
        assert_eq!(active.get("completed_count").and_then(|n| n.as_u64()), Some(4));
    }

    #[test]
    fn empty_snapshot_returns_zero_counts_for_active_app() {
        let env = orient(&Object::phi(), r#"{"active_app":"empty"}"#);
        let v = parse_envelope(&env);
        let apps = v.get("apps").and_then(|a| a.as_array()).expect("apps array");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].get("ready_count").and_then(|n| n.as_u64()), Some(0));
        assert_eq!(apps[0].get("in_progress_count").and_then(|n| n.as_u64()), Some(0));
        assert_eq!(apps[0].get("completed_count").and_then(|n| n.as_u64()), Some(0));
    }

    #[test]
    fn accepts_alternate_in_progress_spelling_with_dash() {
        let mut state = Object::phi();
        state = cell_push("Resource_is_currently_in_Status",
            fact_from_pairs(&[("Resource", "1"), ("Status", "in-progress")]), &state);
        state = cell_push("Resource_is_currently_in_Status",
            fact_from_pairs(&[("Resource", "2"), ("Status", "in_progress")]), &state);
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let apps = v.get("apps").and_then(|a| a.as_array()).expect("apps");
        // Both spellings counted.
        assert_eq!(apps[0].get("in_progress_count").and_then(|n| n.as_u64()), Some(2),
            "envelope: {}", env);
    }

    // ── suggested_next ──────────────────────────────────────────────

    #[test]
    fn suggested_next_points_at_recommended_when_ready_tasks_present() {
        let state = state_with_tasks();
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let suggestion = v.get("suggested_next").and_then(|s| s.as_str()).unwrap_or("");
        assert!(suggestion.contains("Task_is_recommended") || suggestion.contains("recommended"),
            "expected ready-tasks suggestion to mention recommended set; got: {}", suggestion);
        assert!(suggestion.contains("'tasks'"),
            "expected suggestion to name the active app; got: {}", suggestion);
    }

    #[test]
    fn suggested_next_falls_back_to_apps_list_when_no_active_app() {
        let env = orient(&Object::phi(), "{}");
        let v = parse_envelope(&env);
        let suggestion = v.get("suggested_next").and_then(|s| s.as_str()).unwrap_or("");
        assert!(suggestion.contains("apps_list") || suggestion.contains("apps_use"),
            "expected no-active-app suggestion to point at apps_list/apps_use; got: {}",
            suggestion);
    }

    // ── recent_changes ──────────────────────────────────────────────

    #[test]
    fn recent_changes_lists_instance_cells_not_schema_cells() {
        let state = state_with_tasks();
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let changes = v.get("recent_changes").and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("expected recent_changes array, got: {}", env));
        // Should have at least one entry (Task_has_Task_Status).
        assert!(!changes.is_empty(), "expected at least one recent change, got: {}", env);
        // FactType is a schema cell — must not appear.
        for c in changes {
            let noun = c.get("noun").and_then(|n| n.as_str()).unwrap_or("");
            assert_ne!(noun, "FactType",
                "schema cell FactType leaked into recent_changes: {}", env);
        }
    }

    #[test]
    fn recent_changes_skips_generator_scaffolding_prefixes() {
        // Generator output cells (sql:, ddl:, derivation:, edm:, ilayer:,
        // …) are how the engine *talks to itself* — agents don't
        // care about them in a re-orientation snapshot.
        let mut state = Object::phi();
        state = cell_push("sql:sqlite:Task",
            fact_from_pairs(&[("ddl", "CREATE TABLE Task ...")]), &state);
        state = cell_push("derivation:Task_has_Task_Priority",
            fact_from_pairs(&[("rule", "...")]), &state);
        state = cell_push("derivation_index:Task",
            fact_from_pairs(&[("ids", "rule1,rule2")]), &state);
        state = cell_push("ilayer:Element",
            fact_from_pairs(&[("kind", "Button")]), &state);
        state = cell_push("nav:IconToken:parent",
            fact_from_pairs(&[("k", "v")]), &state);
        state = cell_push("Task_has_Task_Status",
            fact_from_pairs(&[("Task", "1"), ("Task Status", "ready")]), &state);
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let changes = v.get("recent_changes").and_then(|c| c.as_array()).expect("changes");
        let nouns: Vec<&str> = changes.iter()
            .map(|c| c.get("noun").and_then(|n| n.as_str()).unwrap_or("")).collect();
        for prefix in &["sql:", "derivation:", "derivation_index:", "ilayer:", "nav:"] {
            assert!(!nouns.iter().any(|n| n.starts_with(prefix)),
                "scaffolding cell with prefix {} leaked into recent_changes: {:?}",
                prefix, nouns);
        }
        // The genuine instance cell must still be present.
        assert!(nouns.contains(&"Task_has_Task_Status"),
            "expected Task_has_Task_Status in recent_changes; got: {:?}", nouns);
    }

    #[test]
    fn recent_changes_skips_misfiled_non_factid_cell_names() {
        // orient-reorient-snapshot: mis-filed cells (raw-verb fragments with
        // spaces/quotes/punctuation from unresolved or multi-fact-line tuples)
        // must NOT surface as recent-activity nouns -- they were the "garbage
        // recent-activity nouns" symptom. Canonical FT-id cells still surface.
        let mut state = Object::phi();
        state = cell_push("maps agent-",
            fact_from_pairs(&[("subjectValue", "x")]), &state);
        state = cell_push("has auditTask 'ft-",
            fact_from_pairs(&[("subjectValue", "y")]), &state);
        state = cell_push("maps to 'Arousal positive'",
            fact_from_pairs(&[("subjectValue", "z")]), &state);
        state = cell_push("Task_has_Task_Status",
            fact_from_pairs(&[("Task", "1"), ("Task Status", "ready")]), &state);
        let env = orient(&state, r#"{"active_app":"tasks"}"#);
        let v = parse_envelope(&env);
        let changes = v.get("recent_changes").and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("expected recent_changes array, got: {}", env));
        let nouns: Vec<&str> = changes.iter()
            .map(|c| c.get("noun").and_then(|n| n.as_str()).unwrap_or("")).collect();
        for n in &nouns {
            assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "mis-filed non-FactId cell leaked into recent_changes: {:?}", nouns);
        }
        assert!(nouns.contains(&"Task_has_Task_Status"),
            "canonical FT cell dropped from recent_changes: {:?}", nouns);
    }

    // ── glob join_path / civil_from_days unit coverage ────────────

    #[test]
    fn join_path_handles_trailing_slash_or_not() {
        assert_eq!(join_path("/apps", "tasks"), "/apps/tasks");
        assert_eq!(join_path("/apps/", "tasks"), "/apps/tasks");
        assert_eq!(join_path("C:\\apps\\", "tasks"), "C:\\apps\\tasks");
    }

    #[cfg(feature = "local")]
    #[test]
    fn civil_from_days_round_trips_known_dates() {
        // 2026-05-09 → unix days = 20583. 1970-01-01 → days = 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // Sanity check: a known recent date.
        let (y, m, _) = civil_from_days(20583);
        assert_eq!(y, 2026);
        assert_eq!(m, 5);
    }
}
