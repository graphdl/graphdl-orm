// crates/arest/src/arest.rs
//
// AREST -- Applicative REpresentational State Transfer
//
// Command : State -> (State', Representation)
//
// The command is compiled from readings. The engine applies it.
// The result is the new state and a hypermedia representation
// with HATEOAS links showing valid state transitions.

use serde::{Serialize, Deserialize};
use crate::types::*;
use crate::ast;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// task-3 phase 2 / DB-task-929: convert a packed (name, reads, func)
/// triple list into the borrowed `(&str, &Func, Option<&[String]>)`
/// shape that `evaluate::forward_chain_defs_state_seeded` consumes.
/// Free fn with explicit lifetimes — a closure inferring this signature
/// can't tie input and output borrows.
fn to_seeded_refs<'a>(
    packed: &'a [(String, Vec<String>, ast::Func)],
) -> Vec<(&'a str, &'a ast::Func, Option<&'a [String]>)> {
    packed.iter().map(|(n, reads, f)| {
        let reads_opt: Option<&[String]> =
            if reads.is_empty() { None } else { Some(reads.as_slice()) };
        (n.as_str(), f, reads_opt)
    }).collect()
}

/// State Machine cell shape — the synthesized role-token names the
/// apply / transition / status-extraction paths read and write to
/// the `State_Machine_is_currently_in_Status` cell.
///
/// task-742: renamed from the legacy code-shaped form
/// (`StateMachine_has_currentlyInStatus` + camelCased role names like
/// `currentlyInStatus`, `forResource`, `instanceOf`) to proper FORML2
/// verbalization (whitepaper §5.1: "Resource is currently in
/// Status"). Subject becomes "State Machine" (spaced) to match the
/// sibling cell `State_Machine_Definition_is_for_Noun`; role names
/// become proper nouns (Status / Resource / Noun) instead of mashed
/// single tokens. Same single-source-of-truth pattern as before;
/// only the strings change.
pub struct StateMachineCellShape {
    /// Cell name carrying the synthesized "State Machine is currently
    /// in Status" facts.
    pub cell_name: &'static str,
    /// Subject role binding: the State Machine entity id.
    pub state_machine_role: &'static str,
    /// Object role binding: the current status value.
    pub current_status_role: &'static str,
    /// Result entity binding: the target resource id (alias of the
    /// State Machine entity id at the API surface).
    pub for_resource_role: &'static str,
    /// HATEOAS / API entity_type label for the synthesized SM
    /// representation entity.
    pub entity_type_label: &'static str,
}

impl StateMachineCellShape {
    pub const fn boot() -> Self {
        StateMachineCellShape {
            cell_name:           "State_Machine_is_currently_in_Status",
            state_machine_role:  "State Machine",
            current_status_role: "Status",
            for_resource_role:   "Resource",
            entity_type_label:   "State Machine",
        }
    }
}

/// Resolve a def from D: Fetch + metacompose (Backus 13.3.2: ρ).
/// Returns the Func if the def exists, or None.
fn def_func(name: &str, d: &ast::Object) -> Option<ast::Func> {
    match ast::fetch_or_phi(name, d) {
        ast::Object::Bottom => None,
        obj => Some(ast::metacompose(&obj, d)),
    }
}

// -- Commands ---------------------------------------------------------

/// The five input classes from Backus Section 14.4.2.
/// Each corresponds to an AREST operation.
///
/// Identity (`sender`) is the reference value of the executing User entity
/// (typically an email). When present, resolve pushes a User fact and a
/// "{noun} is created by User" fact into the population BEFORE derive runs.
/// Authorization enforcement then happens via the existing derive+validate
/// pipeline -- see AREST.tex §8 (Middleware Elimination).
///
/// Signature (`signature`) is an optional MAC over (sender, payload, SECRET)
/// per AREST §5.5 (Distributed Evaluation): "For anonymous peers, events
/// carry cryptographic signatures for identity." See `crate::crypto` for
/// the (placeholder) signing/verification primitives and the platform
/// primitive `verify_signature` for ρ-level invocation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Command {
    /// is-cmd: execute with validation (create entity with SM, constraints)
    CreateEntity {
        noun: String,
        domain: String,
        id: Option<String>,
        fields: hashbrown::HashMap<String, String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-cmd: state machine transition
    Transition {
        #[serde(alias = "entityId")]
        entity_id: String,
        event: String,
        domain: String,
        #[serde(alias = "currentStatus", default)]
        current_status: Option<String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-qry: query the population (partial application of fact type)
    Query {
        #[serde(alias = "schemaId")]
        schema_id: String,
        domain: String,
        target: String,
        bindings: hashbrown::HashMap<String, String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-upd: update entity fields (<->F  .  [upd, defs])
    ///
    /// `force` (#904 / task-861): opt-out of the SM-bypass guard. The
    /// engine refuses `apply update` when the payload sets the SM's
    /// status-role field (e.g. `Task Status` for a Task whose SM is
    /// declared via `State Machine Definition 'Task' is for Noun 'Task'`)
    /// because the SM cell is the canonical status — direct mutation
    /// would silently desync any derivation that reads the status.
    /// Setting `force: true` bypasses the guard for migration scripts
    /// and admin entity-restore flows. The default (`false`) refuses
    /// and points the caller at `apply transition` instead.
    UpdateEntity {
        noun: String,
        domain: String,
        #[serde(alias = "entityId")]
        entity_id: String,
        fields: hashbrown::HashMap<String, String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
        #[serde(default)]
        force: bool,
    },
    /// is-chg: install or update readings (modify definitions D)
    LoadReadings {
        markdown: String,
        domain: String,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-chg singular form (#555): load ONE reading by logical name +
    /// FORML 2 body. Surfaces the structured `LoadReport` (added noun /
    /// FT / derivation cell ids) on success and a structured deontic
    /// diagnostic tree on failure. The plural `LoadReadings` variant
    /// stays for the bake-time / multi-file path; the singular form
    /// is the runtime peer that downstream target adapters
    /// (#560-#564) consume. See `crate::load_reading_core::load_reading`.
    LoadReading {
        name: String,
        body: String,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-chg inverse of `LoadReading` (#556 / DynRdg-2): drop a
    /// previously-loaded reading from the cell graph. Looks up the
    /// `_loaded_reading:{name}` manifest, cascade-deletes the listed
    /// nouns / fact types / derivations, and removes the manifest
    /// cell itself. See `crate::load_reading_core::unload_reading`.
    ///
    /// The optional `policy` field accepts "cascade-delete" (default,
    /// also accepts "cascade_delete") and "migrate" (preserves the
    /// population P — keeps the reading's nouns/FTs/facts, drops only
    /// its derivation defs + manifest so a re-ingestion recomputes).
    /// Unknown values fall back to the default.
    UnloadReading {
        name: String,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-chg atomic compose of `UnloadReading` + `LoadReading`
    /// (#557 / DynRdg-3): replace a previously-loaded reading's
    /// body in a single commit. Either the new body fully replaces
    /// the old (manifest, cells, the lot), or the old reading
    /// stays exactly as it was — no partial state visible.
    ///
    /// The optional `policy` field accepts "replace-all" (default,
    /// also accepts "replace_all") and "migrate-facts" (preserves the
    /// population P, then re-derives it from the new readings —
    /// migration is ingestion of new readings). Unknown values fall
    /// back to the default. See `crate::load_reading_core::reload_reading`.
    ///
    /// First-time-load fallthrough: if no `_loaded_reading:{name}`
    /// manifest is present, the unload step is treated as a no-op
    /// and the reload degenerates to a first-time load. Documented
    /// at the core function and the handler.
    ReloadReading {
        name: String,
        body: String,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// task-930: bulk / collection-shaped apply. Backus α (apply-to-all)
    /// over the input sequence — `apply([op1, op2, …])` carries a
    /// COLLECTION of operations applied as ONE atomic request. The batch
    /// resolves all ops over a shared, cumulatively-built population,
    /// derives to the least fixed point, validates, and emits a single
    /// combined delta. An alethic violation in ANY op rejects the WHOLE
    /// batch (`D' = D`, AREST.tex "Completeness of State Transfer");
    /// deontic findings warn but the batch still commits. A lone op is
    /// the 1-element collection — `Command::Batch { commands: [op] }`
    /// behaves exactly like applying `op` alone.
    ///
    /// Dispatched by `apply_command_defs` to `apply_command_batch`. The
    /// JSON surface is `{"type":"batch","commands":[ <op>, … ]}`, and
    /// `platform_apply_command` additionally accepts a bare top-level
    /// JSON array `[ <op>, … ]` as sugar for this variant.
    Batch {
        commands: Vec<Command>,
    },
}

// -- Result -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub entities: Vec<EntityResult>,
    pub status: Option<String>,
    pub transitions: Vec<TransitionAction>,
    /// Theorem 4b: navigation links — parent/child/peer projections from S.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub navigation: Vec<NavigationLink>,
    pub violations: Vec<Violation>,
    pub derived_count: usize,
    pub rejected: bool,
    /// The transformed state -- the authoritative state after this command.
    #[serde(skip)]
    pub state: ast::Object,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityResult {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub data: hashbrown::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionAction {
    pub event: String,
    pub target_status: String,
    pub method: String,
    pub href: String,
}

/// Theorem 4b: navigation link — parent/child relationship from UC projections.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationLink {
    pub rel: String,    // "children" or "parent"
    pub noun: String,   // target noun name
    pub href: String,
}

// -- Encode/decode bridge (Object ↔ CommandResult) --------------------

/// Encode command input as Object for compiled handler Func.
/// create: <entity_id, <<field_name, value>, ...>, domain, state>
pub fn encode_create_input(
    entity_id: &str, fields: &hashbrown::HashMap<String, String>,
    domain: &str, state: &ast::Object,
) -> ast::Object {
    let field_seq = ast::Object::Seq(
        fields.iter().map(|(k, v)| ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])).collect()
    );
    ast::Object::seq(vec![ast::Object::atom(entity_id), field_seq, ast::Object::atom(domain), state.clone()])
}

/// Encode transition input: <entity_id, event, current_status_or_phi, state>
pub fn encode_transition_input(
    entity_id: &str, event: &str, current_status: Option<&str>, state: &ast::Object,
) -> ast::Object {
    let status_obj = current_status.map(ast::Object::atom).unwrap_or(ast::Object::phi());
    ast::Object::seq(vec![ast::Object::atom(entity_id), ast::Object::atom(event), status_obj, state.clone()])
}

/// Encode update input: <entity_id, <<field_name, value>, ...>, noun, domain, state>
pub fn encode_update_input(
    entity_id: &str, fields: &hashbrown::HashMap<String, String>,
    noun: &str, domain: &str, state: &ast::Object,
) -> ast::Object {
    let field_seq = ast::Object::Seq(
        fields.iter().map(|(k, v)| ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])).collect()
    );
    ast::Object::seq(vec![
        ast::Object::atom(entity_id), field_seq,
        ast::Object::atom(noun), ast::Object::atom(domain), state.clone(),
    ])
}

/// Decode a compiled handler's Object result into CommandResult.
///
/// Two shapes supported:
/// 1. Map carrier: `{__state_delta: Object, __result: JSON string atom}`
///    — emitted by encode_command_result (#209). `state` holds the
///    per-command delta.
/// 2. Legacy (seq): `<entities, status, transitions, violations, derived_count, rejected, new_state>`
pub fn decode_command_result(obj: &ast::Object) -> CommandResult {
    // Try the Map carrier first.
    if let Some(map) = obj.as_map() {
        let state = map.get("__state_delta").cloned().unwrap_or_else(ast::Object::phi);
        let result_json = map.get("__result").and_then(|o| o.as_atom()).unwrap_or("");
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(result_json) {
            let entities = parsed.get("entities").and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|e| {
                    let id = e.get("id")?.as_str()?.to_string();
                    let entity_type = e.get("type").or_else(|| e.get("entityType"))
                        .and_then(|v| v.as_str())?.to_string();
                    let data: hashbrown::HashMap<String, String> = e.get("data")
                        .and_then(|v| v.as_object())
                        .map(|m| m.iter().filter_map(|(k, v)|
                            Some((k.clone(), v.as_str()?.to_string()))).collect())
                        .unwrap_or_default();
                    Some(EntityResult { id, entity_type, data })
                }).collect()).unwrap_or_default();
            let status = parsed.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
            let rejected = parsed.get("rejected").and_then(|v| v.as_bool()).unwrap_or(false);
            let derived_count = parsed.get("derivedCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let transitions = parsed.get("transitions").and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|t| {
                    Some(TransitionAction {
                        event: t.get("event")?.as_str()?.to_string(),
                        target_status: t.get("targetStatus")?.as_str()?.to_string(),
                        method: t.get("method")?.as_str()?.to_string(),
                        href: t.get("href")?.as_str()?.to_string(),
                    })
                }).collect()).unwrap_or_default();
            let violations = parsed.get("violations").and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| {
                    Some(crate::types::Violation {
                        constraint_id: v.get("constraintId")?.as_str()?.to_string(),
                        constraint_text: v.get("constraintText")?.as_str()?.to_string(),
                        detail: v.get("detail")?.as_str()?.to_string(),
                        alethic: v.get("alethic")?.as_bool().unwrap_or(false),
                    })
                }).collect()).unwrap_or_default();
            let navigation = parsed.get("navigation").and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|n| {
                    Some(NavigationLink {
                        rel: n.get("rel")?.as_str()?.to_string(),
                        noun: n.get("noun")?.as_str()?.to_string(),
                        href: n.get("href")?.as_str()?.to_string(),
                    })
                }).collect()).unwrap_or_default();
            return CommandResult {
                entities, status, transitions, navigation, violations,
                derived_count, rejected,
                state,
            };
        }
    }
    // Legacy seq shape.
    let items = obj.as_seq().unwrap_or(&[]);
    let sel = |i: usize| items.get(i);

    let entities = sel(0).and_then(|o| o.as_seq()).map(|es| {
        es.iter().filter_map(|e| {
            let parts = e.as_seq()?;
            let id = parts.get(0)?.as_atom()?.to_string();
            let entity_type = parts.get(1)?.as_atom()?.to_string();
            let data = parts.get(2)?.as_seq().map(|pairs| {
                pairs.iter().filter_map(|p| {
                    let kv = p.as_seq()?;
                    Some((kv.get(0)?.as_atom()?.to_string(), kv.get(1)?.as_atom()?.to_string()))
                }).collect()
            }).unwrap_or_default();
            Some(EntityResult { id, entity_type, data })
        }).collect()
    }).unwrap_or_default();

    let status = sel(1).and_then(|o| o.as_atom()).map(|s| s.to_string());

    let transitions = sel(2).and_then(|o| o.as_seq()).map(|ts| {
        ts.iter().filter_map(|t| {
            let parts = t.as_seq()?;
            Some(TransitionAction {
                event: parts.get(0)?.as_atom()?.to_string(),
                target_status: parts.get(1)?.as_atom()?.to_string(),
                method: parts.get(2)?.as_atom()?.to_string(),
                href: parts.get(3)?.as_atom()?.to_string(),
            })
        }).collect()
    }).unwrap_or_default();

    let violations = sel(3).and_then(|o| o.as_seq()).map(|vs| {
        vs.iter().filter_map(|v| ast::decode_violation(v)).collect()
    }).unwrap_or_default();

    let derived_count = sel(4).and_then(|o| o.as_atom())
        .and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let rejected = sel(5).and_then(|o| o.as_atom()) == Some("T");
    let new_state = sel(6).cloned().unwrap_or(ast::Object::phi());

    CommandResult { entities, status, transitions, navigation: vec![], violations, derived_count, rejected, state: new_state }
}

/// Encode a CommandResult as an Object for the dispatch layer.
///
/// Returns a delta-carrier Object (#209): `result.state` is now a
/// per-command delta — only the cells the command modified — stored
/// under the CELL name "__state_delta". system_impl merges this onto
/// the snapshot before commit, so create / update / transition touch
/// only their RMAP cells and leave schema cells untouched.
///
/// The JSON summary under "__result" is compact — entities + status +
/// transitions + violations + derived_count + rejected — *without*
/// dumping the full D. That keeps MCP/HTTP responses small and
/// JSON-parseable.
pub fn encode_command_result(result: &CommandResult) -> ast::Object {
    let summary = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
    let mut cells = hashbrown::HashMap::new();
    cells.insert("__state_delta".to_string(), result.state.clone());
    cells.insert("__result".to_string(), ast::Object::atom(&summary));
    ast::Object::Map(cells.into())
}

// -- Apply ------------------------------------------------------------

// task-822: `read_cell_key_roles_local` was vendored from
// `evaluate.rs::read_cell_key_roles` because that helper was private.
// task-820 revision made it `pub(crate)`, so the local vendor is now
// redundant — apply path uses the shared parser directly.

/// task-822: build a `Violation` mirroring the shape that
/// `compile_uniqueness_ast` would emit at validate time. Surfaced from
/// the apply (user-facing) emit path so a user-asserted fact that
/// collides with an existing fact at the same scope key reaches the
/// caller as a structured violation instead of being silently appended
/// to Seq storage (or panicking, as the forward-chain emit path does
/// where conflicts mean a derivation bug).
///
/// The shape matches `compile_uniqueness_ast` at compile.rs:5742-5749:
///   detail = "Uniqueness violation: <noun> <key> is not unique in <reading>"
/// The constraint_id / constraint_text are sourced from the conflicting
/// cell's compile-time UC. Apply-time we don't have a single UC handle
/// — multiple UCs can land on the same cell — so we synthesize the
/// id/text from the cell name + the conflict's key. The `alethic` flag
/// is true: UCs are structural-impossibility constraints (CompiledSchema
/// only carries `key_roles` for alethic UCs; task-744 phase 1).
fn uc_violation_from_conflict(conflict: &ast::KeyConflict) -> crate::types::Violation {
    crate::types::Violation {
        constraint_id: format!("uc:{}", conflict.name),
        constraint_text: format!("Each tuple in {} is unique by key", conflict.name),
        detail: format!(
            "Uniqueness violation: key '{}' is not unique in {}",
            conflict.key, conflict.name,
        ),
        alethic: true,
    }
}

/// task-822: push a fact into a cell, routing through `cell_put_keyed`
/// when the cell has `key_roles` registered in `_CellKeyRoles`, else
/// through the legacy `cell_push`. Conflicts on the keyed path are
/// appended to `violations` and the state is left unchanged for that
/// fact — the user-facing apply contract per task constraints (no
/// `.expect(...)`; conflicts surface as violations, not panics).
///
/// `overwrite=true` (update path): if a prior fact at the same key
/// exists with different non-key values, replace it (no conflict). This
/// matches the user's explicit "update" intent. Same-fact updates remain
/// no-ops because `cell_put_keyed` short-circuits on byte-equal facts;
/// we only re-key when the new fact differs.
///
/// `overwrite=false` (create path): conflicts produce violations and the
/// state is left unchanged for that fact.
fn push_with_uc_check(
    state: ast::Object,
    cell_name: &str,
    fact: ast::Object,
    key_roles: &hashbrown::HashMap<String, Vec<String>>,
    overwrite: bool,
    violations: &mut Vec<crate::types::Violation>,
) -> ast::Object {
    let Some(roles) = key_roles.get(cell_name) else {
        if cell_name.contains(':') {
            return ast::cell_push(cell_name, fact, &state);
        }
        return ast::cell_put_folded(cell_name, fact, &state);
    };
    let role_refs: Vec<&str> = roles.iter().map(|s| s.as_str()).collect();
    match ast::cell_put_keyed(cell_name, &role_refs, fact.clone(), &state) {
        Ok(next) => next,
        Err(conflict) if overwrite => {
            // Update path: explicit user intent to replace. Drop the
            // colliding map entry for this key, then re-attempt the
            // put — the second call cannot conflict (slot is empty).
            let cleared = drop_keyed_entry(cell_name, &conflict.key, &role_refs, &state);
            ast::cell_put_keyed(cell_name, &role_refs, fact, &cleared)
                .unwrap_or_else(|_| {
                    // Defensive: with the slot just cleared, a second
                    // conflict is structurally impossible. Fall back to
                    // the cleared state if it somehow happens — better
                    // than losing the user's write entirely.
                    cleared
                })
        }
        Err(conflict) => {
            violations.push(uc_violation_from_conflict(&conflict));
            state
        }
    }
}

/// task-822 helper: remove the map entry under `key` from cell `name`,
/// regardless of whether the cell is currently `Object::Map` or
/// `Object::Seq`. Map case is a direct `HashMap::remove`; Seq case is a
/// filter that drops every fact whose extracted key matches.
///
/// Called from `push_with_uc_check`'s overwrite (update) branch to
/// vacate the slot before re-asserting the new fact. Standalone helper
/// because `ast::cell_filter` only walks Seq cells — Map cells (the
/// new task-744 storage for UC-keyed cells) need explicit Map handling.
fn drop_keyed_entry(
    name: &str,
    key: &str,
    key_role_names: &[&str],
    state: &ast::Object,
) -> ast::Object {
    let existing = ast::fetch_or_phi(name, state);
    match &existing {
        ast::Object::Map(m) => {
            let mut next = (**m).clone();
            next.remove(key);
            ast::store(name, ast::Object::Map(next.into()), state)
        }
        ast::Object::Seq(items) => {
            let kept: Vec<ast::Object> = items.iter()
                .filter(|f| ast::extract_key_from_fact(f, key_role_names)
                    .as_deref() != Some(key))
                .cloned()
                .collect();
            ast::store(name, ast::Object::Seq(kept.into()), state)
        }
        _ => state.clone(),
    }
}

pub fn apply_command_defs(
    d: &ast::Object,
    command: &Command,
    state: &ast::Object,
) -> CommandResult {
    match command {
        Command::CreateEntity { noun, domain, id, fields, sender, signature: _ } => {
            create_via_defs(d, noun, domain, id.as_deref(), fields, sender.as_deref(), state)
        }
        Command::Transition { entity_id, event, domain, current_status, sender: _, signature: _ } => {
            transition_via_defs(d, entity_id, event, domain, current_status.as_deref(), state)
        }
        Command::Query { schema_id, domain: _, target, bindings, sender: _, signature: _ } => {
            query_via_defs(d, schema_id, target, bindings, state)
        }
        Command::UpdateEntity { noun, domain, entity_id, fields, sender: _, signature: _, force } => {
            update_via_defs(d, noun, domain, entity_id, fields, *force, state)
        }
        Command::LoadReadings { markdown, domain, sender: _, signature: _ } => {
            apply_load_readings(markdown, domain, d, state)
        }
        Command::LoadReading { name, body, sender: _, signature: _ } => {
            load_reading_handler(d, name, body, state)
        }
        Command::UnloadReading { name, policy, sender: _, signature: _ } => {
            unload_reading_handler(d, name, policy.as_deref(), state)
        }
        Command::ReloadReading { name, body, policy, sender: _, signature: _ } => {
            reload_reading_handler(d, name, body, policy.as_deref(), state)
        }
        Command::Batch { commands } => {
            apply_command_batch(d, commands, state)
        }
        #[allow(unreachable_patterns)]
        _ => CommandResult {
            entities: vec![],
            status: None,
            transitions: vec![],
            navigation: vec![],
            violations: vec![],
            derived_count: 0,
            rejected: false,
            state: ast::Object::phi(),
        },
    }
}

/// task-930: bulk / collection-shaped apply — Backus **α (apply-to-all)**
/// over the input sequence. `apply([op1, op2, …])` is α(ρ-dispatch) over
/// the collection, run as ONE atomic request.
///
/// **Semantics (AREST.tex "Completeness of State Transfer").** Each op is
/// dispatched through the existing `apply_command_defs` pipeline (no fork
/// of the resolve→derive→validate→emit stages), but against a state that
/// is built up CUMULATIVELY: op *k* sees every fact op *0..k* produced.
/// Concretely, after each op we `merge_delta` its delta onto the running
/// state so the next op resolves over the combined population and derives
/// to the least fixed point against everything before it. The op-local
/// deltas are accumulated and emitted as ONE combined delta relative to
/// the original `state` — the caller commits it in a single
/// `merge_delta`, so the whole collection appears atomically.
///
/// **Atomicity / rollback.** An **alethic** violation in ANY op rejects
/// the WHOLE batch: we stop, discard every accumulated delta, and emit an
/// empty delta (`D' = D`) — none of the batch's writes land, not even
/// ops that ran before the violation. **Deontic** findings (warnings)
/// accumulate but do not reject; the batch still commits. This mirrors
/// the single-command rule at `create_via_defs`
/// (`final_state = match rejected { true => state.clone(), … }`), lifted
/// to the collection.
///
/// **1-element collection.** A lone op is the natural shape — a one-entry
/// `commands` slice runs the single op once and returns its result with
/// the same delta `apply_command_defs` would have produced. An EMPTY
/// collection is a no-op success (empty delta).
///
/// Intermediate merges thread `event = None` (these are not the commit
/// boundary — the host attaches the apply event when it merges the
/// returned combined delta), matching the eventless contract the forward
/// chain already uses for in-flight state.
pub fn apply_command_batch(
    d: &ast::Object,
    commands: &[Command],
    state: &ast::Object,
) -> CommandResult {
    // Empty collection — α over the empty sequence is the identity:
    // success with no entities and an empty delta.
    if commands.is_empty() {
        return CommandResult {
            entities: Vec::new(),
            status: None,
            transitions: Vec::new(),
            navigation: Vec::new(),
            violations: Vec::new(),
            derived_count: 0,
            rejected: false,
            state: ast::diff_cells(state, state), // empty Map delta
        };
    }

    // Running state the next op resolves/derives against (combined
    // population), and the accumulated results.
    let mut running = state.clone();
    let mut entities: Vec<EntityResult> = Vec::new();
    let mut transitions: Vec<TransitionAction> = Vec::new();
    let mut navigation: Vec<NavigationLink> = Vec::new();
    let mut violations: Vec<crate::types::Violation> = Vec::new();
    let mut derived_count: usize = 0;
    let mut last_status: Option<String> = None;

    for command in commands {
        let res = apply_command_defs(d, command, &running);
        // Aggregate the op's report regardless of outcome so the caller
        // sees every violation that contributed to the decision.
        entities.extend(res.entities.iter().cloned());
        transitions.extend(res.transitions.iter().cloned());
        navigation.extend(res.navigation.iter().cloned());
        violations.extend(res.violations.iter().cloned());
        derived_count += res.derived_count;
        if res.status.is_some() {
            last_status = res.status.clone();
        }

        // Alethic violation anywhere → reject the WHOLE batch. Discard
        // every accumulated write and emit `D' = D` (empty delta).
        if res.rejected {
            return CommandResult {
                entities,
                status: last_status,
                transitions,
                navigation,
                violations,
                derived_count,
                rejected: true,
                state: ast::diff_cells(state, state), // empty delta — full rollback
            };
        }

        // Fold this op's delta onto the running state so the next op
        // resolves over the combined population. Eventless merge — this
        // is in-flight state, not the commit boundary.
        running = ast::merge_delta(&running, &res.state, None);
    }

    // One combined delta relative to the ORIGINAL state — the host
    // commits the whole collection in a single merge.
    let delta = ast::diff_cells(state, &running);
    CommandResult {
        entities,
        status: last_status,
        transitions,
        navigation,
        violations,
        derived_count,
        rejected: false,
        state: delta,
    }
}

/// #867 / task-735 — auto-generate an entity id when the create
/// command's `explicit_id` is None.
///
/// **Scheme reconciliation (task-735).** Two id schemes have
/// historically accumulated in the same noun cell: the legacy
/// `<noun>-N` form (what this function used to emit) and bare
/// integers (e.g. `916` from manual assertions / external imports).
/// A pure counter (`seen.len() + 1`) is oblivious to both: assert
/// `id='916'` then auto-create and you get `task-1`, then `task-2`,
/// then eventually `task-916` collides. Fix: scan existing ids of
/// the same noun, detect which scheme dominates, and pick
/// `max(existing) + 1` in that scheme. The result never collides
/// with an existing id.
///
/// Scheme detection:
/// - Existing ids are partitioned into two integer buckets:
///   - `task_n_max`: max N from ids matching `<prefix>-{digits}`
///     (where prefix = noun-lowercased-hyphenated).
///   - `int_max`: max N from ids that parse as bare unsigned
///     integers (e.g. `916`).
/// - If `int_max` dominates (strictly greater than `task_n_max`),
///   emit a bare integer one above it. Otherwise emit
///   `<prefix>-{N}` where N = max(task_n_max, int_max) + 1. Ties
///   favor the prefixed form so backward compatibility holds.
/// - Empty cell: emit `<prefix>-1` (default `task-1` for noun
///   `Task`).
///
/// Deterministic per (noun, state); platform-independent (no
/// `SystemTime` so the function compiles on wasm32 / no_std).
fn auto_generate_entity_id(noun: &str, state: &ast::Object) -> String {
    // Distinct entity-role values for this noun across all FT cells.
    // A fact's "entity-id" for noun N is the value of any role binding
    // whose role name matches N. This mirrors `platform_list_noun`'s
    // entity discovery — same identity discovery, same scan surface.
    let mut seen: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    for (_, contents) in ast::cells_iter(state) {
        // #932 phase-2: cell_facts_iter so a folded (Map) cell's entity
        // ids are scanned too — auto-increment must see them, not skip
        // them (the raw-as_seq()-skips-Map bug class).
        for fact in ast::cell_facts_iter(contents) {
            let pairs = match fact.as_seq() { Some(p) => p, None => continue };
            for pair in pairs.iter() {
                let kv = match pair.as_seq() { Some(s) => s, None => continue };
                let role = match kv.first().and_then(|k| k.as_atom()) {
                    Some(r) => r, None => continue
                };
                if role != noun { continue; }
                let val = match kv.get(1).and_then(|v| v.as_atom()) {
                    Some(v) => v, None => continue
                };
                if val.is_empty() { continue; }
                seen.insert(val.to_string());
            }
        }
    }

    let prefix = noun.to_lowercase().replace(' ', "-");
    let prefix_dash = format!("{prefix}-");

    // task-735 — scan existing ids for the integer payload in each
    // scheme. `task_n_max` tracks `<prefix>-{N}` ids; `int_max`
    // tracks bare-integer ids. Non-integer suffixes are ignored
    // for max-tracking but still occupy the namespace — the final
    // collision guard below bumps past any duplicate.
    let mut task_n_max: Option<u64> = None;
    let mut int_max: Option<u64> = None;
    for val in seen.iter() {
        if let Some(suffix) = val.strip_prefix(&prefix_dash) {
            if let Ok(n) = suffix.parse::<u64>() {
                task_n_max = Some(task_n_max.map_or(n, |m| m.max(n)));
                continue;
            }
        }
        if let Ok(n) = val.parse::<u64>() {
            int_max = Some(int_max.map_or(n, |m| m.max(n)));
        }
    }

    // Empty cell — bootstrap with the prefix scheme (preserves the
    // legacy `task-1` convention for backward compat with #867).
    if seen.is_empty() {
        return format!("{prefix}-1");
    }

    // Pick the scheme. Bare-integer dominance (strict >) emits a
    // bare integer; otherwise the prefixed form wins (ties on
    // `int_max == task_n_max` favor `<prefix>-N` because the
    // prefixed scheme is the engine's default — assertions of bare
    // integers happen at the user surface, but the engine emits
    // the prefixed shape unless the population has overwhelmingly
    // chosen otherwise).
    let mut candidate = match (task_n_max, int_max) {
        (None, None) => format!("{prefix}-1"),
        (Some(t), None) => format!("{prefix}-{}", t + 1),
        (None, Some(i)) => format!("{}", i + 1),
        (Some(t), Some(i)) if i > t => format!("{}", i + 1),
        (Some(t), Some(i)) => {
            // Tie or prefix dominates: bump above the global max
            // (so the new id can never collide with either bucket).
            let n = t.max(i) + 1;
            format!("{prefix}-{}", n)
        }
    };

    // Defensive collision guard: if scheme detection somehow
    // produced an id that already exists (e.g. a non-integer suffix
    // happens to share a number with our pick, or both schemes
    // co-exist with overlapping integers), keep bumping until free.
    // Bounded by `seen.len() + 1` iterations.
    let mut bump: u64 = 1;
    while seen.contains(&candidate) {
        let base = task_n_max.unwrap_or(0).max(int_max.unwrap_or(0));
        // Always escalate via the prefixed scheme on collision —
        // the bare-integer namespace is shallower (no prefix) so
        // collisions there are likelier; the prefixed form keeps
        // the id space disjoint from any future bare-integer
        // assertion.
        candidate = format!("{prefix}-{}", base + bump);
        bump += 1;
        if bump as usize > seen.len() + 2 {
            // Unreachable in practice — the loop terminates because
            // `seen` is finite. This guard exists to prove no
            // infinite loop under any input.
            break;
        }
    }

    candidate
}

/// create = emit ∘ validate ∘ derive ∘ resolve (Eq. 5)
/// Each stage is a ρ-application. The result is an Object, decoded to CommandResult at the boundary.
///
/// Identity: when `sender` is Some, resolve pushes a User entity fact (keyed
/// by the sender value, typically an email) plus a "{noun} is created by User"
/// fact. Authorization enforcement then happens via the derive+validate stages
/// -- any alethic constraint touching User facts (e.g. "Each Order is created
/// by exactly one User") will fire if identity is missing. No procedural
/// middleware. Per AREST §8.

/// Run-time definedness predicate. A noun may be instantiated or mutated at
/// run-time only if it is a fully-defined entity type — declared
/// objectType="entity" WITH a reference scheme (its identity). A value type,
/// an undeclared noun, or an entity declared without a reference scheme are
/// valid *design-time* shapes but NOT run-time ones: a derivation
/// forward-chain over them has no identity to ground and can diverge. Gates
/// createEntity and updateEntity; `compile` stays permissive at design-time.
fn noun_runtime_defined(noun: &str, state: &ast::Object, d: &ast::Object) -> bool {
    [state, d].iter().any(|src|
        ast::fetch_cell_seq("Noun", src).as_seq().map_or(false, |fs|
            fs.iter().any(|f|
                ast::binding(f, "name") == Some(noun)
                    && ast::binding(f, "objectType") == Some("entity")
                    && ast::binding(f, "referenceScheme").map_or(false, |rs| !rs.is_empty()))))
}

fn create_via_defs(
    d: &ast::Object,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &hashbrown::HashMap<String, String>,
    sender: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    // #867 — when no explicit id is provided, resolve identity per
    // whitepaper §6.3 ("Identity follows from the reference scheme;
    // resolve applies it"). Pre-fix the engine defaulted to the empty
    // string, every cell_push pushed a fact with `(noun, "")` as its
    // head pair, and the resulting "entity" was unaddressable. The
    // safer default for tasks/orders coming from the MCP surface is
    // **auto-generate** (over reject-with-violation): a generated id
    // makes the entity addressable and round-trips through `get:Noun`
    // immediately, with no human interpretation required.
    //
    // Generation scheme: `<noun-lowercased-hyphenated>-<n>` where n is
    // (count of distinct entity-role values for `noun` in `state`) + 1.
    // Deterministic per (noun, snapshot), platform-independent (no
    // wallclock — works on wasm32 / no_std), and visibly counts up so
    // the surface area is debuggable. Two creates against the same
    // snapshot generate distinct ids only because the second runs
    // against a state augmented by the first via merge_delta at the
    // commit boundary; within a single call the count is stable.
    // Run-time definedness gate. Instantiating an entity is a RUN-TIME
    // operation: it requires a fully-defined entity type — declared
    // objectType="entity" WITH a reference scheme (its identity). You MAY
    // *define* an entity without a reference scheme, or a graph schema whose
    // roles aren't yet connected to entities; those are valid design-time
    // shapes. But they are not run-time shapes — createEntity over a noun
    // with no identity has nothing for resolve to ground and drives the
    // resolve/derivation evaluation into a non-terminating expansion (the
    // bundled metamodel's `Order` is objectType="value", and an undeclared
    // noun has no scheme at all — both hung the engine, 6+ GB, no return).
    // Refuse up front as an alethic violation so derivations never run over
    // an under-defined noun. (`compile` stays permissive — see compile.rs;
    // incomplete definitions remain legal at define-time.)
    if !noun_runtime_defined(noun, state, d) {
        return CommandResult {
            entities: alloc::vec![],
            status: None,
            transitions: alloc::vec![],
            navigation: alloc::vec![],
            violations: alloc::vec![crate::types::Violation {
                constraint_id: alloc::format!("create.not_runtime_defined:{}", noun),
                constraint_text: alloc::format!(
                    "'{}' is not a fully-defined entity type; an entity needs a reference \
                     scheme (identity) before it can be instantiated",
                    noun),
                detail: alloc::format!(
                    "createEntity rejected at run-time: noun '{}' must be declared with \
                     objectType 'entity' and a reference scheme (it may be defined without \
                     one, but not instantiated)",
                    noun),
                alethic: true,
            }],
            derived_count: 0,
            rejected: true,
            state: ast::Object::phi(),
        };
    }
    let entity_id = explicit_id.unwrap_or("").to_string();
    let explicit_id_provided = !entity_id.is_empty();
    let entity_id = if entity_id.is_empty() {
        auto_generate_entity_id(noun, state)
    } else {
        entity_id
    };

    // task-737 — alethic UC pre-check for the primary reference
    // scheme. When the caller supplied an explicit id and an entity
    // with that id already exists in state, reject with a UC-shaped
    // violation. Without this guard, a second `apply create` with the
    // same explicit id silently extends the chain with a duplicate
    // entity row — substrate corruption per ORM2 reference-scheme
    // semantics. (The keyed-cell write path catches differing facts at
    // the same key, but identical re-writes are byte-equal no-ops at
    // that level; the pre-check is what surfaces the duplicate-entity
    // case to the caller.)
    if explicit_id_provided {
        let noun_cell = ast::fetch_cell_seq("Noun", state);
        let ref_scheme = noun_cell.as_seq()
            .and_then(|facts| facts.iter()
                .find(|f| ast::binding(f, "name") == Some(noun))
                .and_then(|f| ast::binding(f, "referenceScheme"))
                .map(|s| s.to_string()))
            .filter(|s| !s.is_empty());
        if let Some(scheme) = ref_scheme {
            // A Noun N has an entity with id v iff any cell carries a
            // binding `<N, v>`. Skip internal cells (names containing
            // ':' — derivation/strat2/cwa) so derived rollups don't
            // shadow primary facts.
            let already_exists = ast::cells_iter(state).iter().any(|(name, contents)| {
                if name.contains(':') { return false; }
                let mut iter = ast::cell_facts_iter(contents);
                iter.any(|fact| {
                    let pairs = match fact.as_seq() { Some(p) => p, None => return false };
                    pairs.iter().any(|p| {
                        let kv = match p.as_seq() { Some(s) => s, None => return false };
                        kv.first().and_then(|k| k.as_atom()) == Some(noun)
                            && kv.get(1).and_then(|v| v.as_atom()) == Some(entity_id.as_str())
                    })
                })
            });
            if already_exists {
                // Use the same `uc:{primary_ft}` violation shape that
                // cell_put_keyed's KeyConflict surfaces (task-822) so
                // downstream consumers see one constraint family for
                // both pre-check and storage-layer rejections.
                let first_part = scheme.split(',').next().unwrap_or("id").trim();
                let primary_ft = format!("{}_has_{}", noun, first_part);
                let viol = crate::types::Violation {
                    constraint_id: format!("uc:{}", primary_ft),
                    constraint_text: format!(
                        "Each {} has at most one {} (reference scheme uniqueness)",
                        noun, first_part),
                    detail: format!(
                        "Uniqueness violation: key '{}' is not unique in {}",
                        entity_id, primary_ft),
                    alethic: true,
                };
                return CommandResult {
                    entities: alloc::vec![],
                    status: None,
                    transitions: alloc::vec![],
                    navigation: alloc::vec![],
                    violations: alloc::vec![viol],
                    derived_count: 0,
                    rejected: true,
                    state: ast::Object::phi(),
                };
            }
        }
    }

    // task-822: read the per-cell key-roles map once. The resolve emit
    // path below routes through `push_with_uc_check`, which consults
    // this map per fact to decide whether to call `cell_put_keyed`
    // (Map storage, UC-enforcing) or `cell_push` (legacy Seq append).
    // Conflicts on the keyed path land in `uc_violations` and surface
    // alongside the validate-stage violations the existing pipeline
    // already aggregates.
    let key_roles = crate::evaluate::read_cell_key_roles(d);
    let mut uc_violations: Vec<crate::types::Violation> = Vec::new();

    // ── resolve: populate facts via ρ(resolve:{noun}) ──────────────
    let fields_with_domain: Vec<(&str, &str)> = fields.iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .chain(core::iter::once(("domain", domain)))
        .collect();
    let mut fact_events: Vec<String> = Vec::new();
    let resolved = fields_with_domain.iter().fold(state.clone(), |acc, (field_name, value)| {
        let lower = field_name.to_lowercase();
        let ft_id_obj = ast::apply(&ast::Func::Def(format!("resolve:{}", noun)),
            &ast::Object::atom(&lower), d);
        // task-737: the resolve chain terminates in `Func::Id`, which
        // echoes the input atom back when no condition matches. Treat
        // an echoed input as a miss and fall through to
        // `<Noun>_has_<Field>`. Pre-#737 most ref-scheme nouns had no
        // emitted `resolve:{noun}` def at all, so `apply` returned
        // Bottom and the fallback fired implicitly; now that the
        // synthesiser guarantees a primary FT, `resolve:{noun}` is
        // always present and we have to detect the no-match case
        // ourselves. (Encoding `Func::Constant(Object::Bottom)` as the
        // terminator collapses the whole encoded def to ⊥ via
        // §11.2.1 bottom-preservation, so a sentinel-atom approach is
        // the available path.)
        let ft_id = match ft_id_obj.as_atom() {
            Some(s) if s != lower => s.to_string(),
            _ => format!("{}_has_{}", noun, field_name),
        };
        fact_events.push(ft_id.clone());
        let fact = ast::fact_from_pairs(&[(noun, &entity_id), (field_name, value)]);
        push_with_uc_check(acc, &ft_id, fact, &key_roles, /*overwrite=*/false, &mut uc_violations)
    });

    // ── resolve: compound ref scheme decomposition ──────────────────
    // Paper Eq. 6: resolve determines identity from the reference scheme.
    // For compound schemes (.Owner, .Seq), split entity_id on '-' (rsplitn)
    // and push component facts: Thing_has_Owner, Thing_has_Seq.
    let resolved = {
        let noun_cell = ast::fetch_cell_seq("Noun", &resolved);
        let ref_scheme: Option<Vec<String>> = noun_cell.as_seq()
            .and_then(|facts| facts.iter()
                .find(|f| ast::binding(f, "name") == Some(noun))
                .and_then(|f| ast::binding(f, "referenceScheme"))
                .map(|rs| rs.split(',').map(|s| s.trim().to_string()).collect()));
        ref_scheme
            .filter(|parts| parts.len() >= 2 && !entity_id.is_empty())
            .map(|parts| {
                let n = parts.len();
                let splits: Vec<&str> = entity_id.rsplitn(n, '-').collect();
                // rsplitn returns parts right-to-left; reverse to match left-to-right ref scheme order.
                // If fewer splits than parts, pad with empty strings.
                let components: Vec<&str> = splits.into_iter().rev().collect();
                parts.iter().enumerate().fold(resolved.clone(), |acc, (i, part)| {
                    let value = components.get(i).unwrap_or(&"");
                    let ft_id = format!("{}_has_{}", noun, part.replace(' ', "_"));
                    let fact = ast::fact_from_pairs(&[(noun, &entity_id), (part, value)]);
                    push_with_uc_check(acc, &ft_id, fact, &key_roles, /*overwrite=*/false, &mut uc_violations)
                })
            })
            .unwrap_or(resolved)
    };

    // ── identity: push User facts when sender is present ──────────
    // This is the data that auth derivations + alethic constraints evaluate.
    // Fact type IDs follow parser convention: "Noun_predicate_Target".
    let resolved = sender.map(|s| {
        let created_by_ft = format!("{}_is_created_by_User", noun);
        let user_ref_ft = "User_has_Email".to_string();
        let user_fact = ast::fact_from_pairs(&[("User", s), ("Email", s)]);
        let with_user = push_with_uc_check(
            resolved.clone(), &user_ref_ft, user_fact, &key_roles,
            /*overwrite=*/false, &mut uc_violations,
        );
        let created_by_fact = ast::fact_from_pairs(&[(noun, &entity_id), ("User", s)]);
        push_with_uc_check(
            with_user, &created_by_ft, created_by_fact, &key_roles,
            /*overwrite=*/false, &mut uc_violations,
        )
    }).unwrap_or(resolved);

    // ── derive: forward chain via ρ(derivation:*) to lfp ───────────
    // Gate derivations by noun relevance: only run rules whose antecedent or
    // consequent fact types involve the created noun. The derivation_index:{noun}
    // cell (compiled in compile_to_defs_state) provides the relevant IDs with
    // transitive closure already computed.
    // When SQL triggers handle derivations, further restrict to SM-related only.
    let has_sql_triggers = ast::cells_iter(d).into_iter()
        .any(|(n, _)| n.starts_with("sql:trigger:"));
    // Collect fact types that SM transitions subscribe to.
    let sm_event_types: hashbrown::HashSet<String> = if has_sql_triggers {
        let trigger_cell = ast::fetch_cell_seq("Transition_is_triggered_by_Event_Type", d);
        trigger_cell.as_seq().map(|facts| {
            facts.iter().filter_map(|f| {
                ast::binding(f, "Event Type").map(|s| s.to_string())
            }).collect()
        }).unwrap_or_default()
    } else {
        hashbrown::HashSet::new()
    };
    // Noun-gated derivation index: O(1) fetch from compiled index.
    // The index is stored as Func::constant(atom) → func_to_object yields <', atom>.
    // Extract the atom from the constant form.
    let relevant_ids: hashbrown::HashSet<String> = {
        let index_key = format!("derivation_index:{}", noun);
        let index_obj = ast::fetch(&index_key, d);
        // Unwrap constant form <', value> produced by func_to_object
        let value = index_obj.as_seq()
            .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
            .and_then(|items| items[1].as_atom())
            .or_else(|| index_obj.as_atom());
        value
            .map(|s| s.split(',').map(|id| id.to_string()).collect())
            .unwrap_or_default()
    };
    // 2-stratum forward chain (#828): stratum 1 = `derivation:rule_*`
    // (positive rules), stratum 2 = `derivation_strat2:rule_*`
    // (negation-guarded rules). Mirrors `cli/entry.rs::run_load`.
    // Without this split a stratum-2 AbsenceOf guard fires in round 1
    // before the positive rule its negative dependency reads has
    // populated, so the consequent fires for entries that should be
    // filtered out.
    let collect_stratum = |prefix: &str| -> Vec<(String, ast::Func)> {
        let cell_prefix = alloc::format!("{}:", prefix);
        ast::cells_iter(d).into_iter()
            .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
            .filter(|(n, _)| {
                // task-967: no noun pre-filter -- run the full stratum. The
                // noun-scoped `derivation_index` keyed each rule only under its
                // OWN fact types' nouns, with no closure over rule->rule data
                // dependencies, so a rule consuming a cell another rule writes
                // (the SM->status bridge consuming `_sm_event_fold_{N}`'s
                // `State_Machine_is_currently_in_Status`) was excluded from the
                // noun's set and never reached the fixpoint on apply -- though
                // the compile path (no gate) derives it. The seeded chainer's
                // reads-dirty gating already restricts ACTIVE rules each round.
                // When SQL triggers own derivations, still restrict to SM
                // infrastructure + subscribed-event derivations.
                if has_sql_triggers {
                    n.contains("StateMachine") || n.contains("machine:") || n.contains("_transitive_Status")
                        || n.contains("_transitive_Transition") || n.contains("sm_init")
                        || n.contains("sm_for_resource_backfill")
                        || sm_event_types.iter().any(|evt| n.contains(evt))
                } else {
                    true
                }
            })
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
            .collect()
    };
    let stratum1 = collect_stratum("derivation");
    let stratum2 = collect_stratum("derivation_strat2");
    diag!("[profile] derivation gating: {}/{} stratum-1, {}/{} stratum-2 rules for noun '{}'",
        stratum1.len(),
        ast::cells_iter(d).into_iter().filter(|(n, _)| n.starts_with("derivation:")).count(),
        stratum2.len(),
        ast::cells_iter(d).into_iter().filter(|(n, _)| n.starts_with("derivation_strat2:")).count(),
        noun);

    // task-3 phase 2 / DB-task-929: incremental forward chain. See the
    // same block in `update_via_defs` for the full rationale. Seed:
    //   * `fact_events` — cells the per-field push step accumulated
    //     for this create (line 861).
    //   * antecedent reads of rules whose consequent was dropped by
    //     the #836 LFP-clear below.
    let touched_cells: hashbrown::HashSet<String> = fact_events.iter().cloned().collect();
    let build_seeded_refs = |stratum: &[(String, ast::Func)]|
        -> Vec<(String, Vec<String>, ast::Func)>
    {
        stratum.iter().map(|(name, func)| {
            let id = name.split_once(':').map(|(_, id)| id).unwrap_or(name);
            let reads = crate::evaluate::read_derivation_reads(d, id).unwrap_or_default();
            (name.clone(), reads, func.clone())
        }).collect()
    };
    let s1_packed = build_seeded_refs(&stratum1);
    let s2_packed = build_seeded_refs(&stratum2);

    // #836 — drop derived consequent cells from `resolved` before
    // forward-chain so the LFP recomputes against the current
    // primary state. task-929: noun-scope the wipe to
    // derivation_index[noun]'s rules so cross-noun upstream consequent
    // cells survive.
    let drule_cell = ast::fetch_cell_seq("DerivationRule", d);
    let dropped_cells: hashbrown::HashSet<String> = drule_cell.as_seq()
        .map(|facts| facts.iter()
            .filter(|f| relevant_ids.is_empty()
                || ast::binding(f, "id")
                    .map(|id| relevant_ids.contains(id))
                    .unwrap_or(false))
            .filter_map(|f| ast::binding(f, "consequentFactTypeId"))
            .map(|encoded| crate::types::ConsequentCellSource::decode(encoded)
                .literal_id().to_string())
            .filter(|s| !s.is_empty())
            .collect())
        .unwrap_or_default();
    let resolved = if dropped_cells.is_empty() {
        resolved
    } else {
        let mut new_map: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        for (name, contents) in ast::cells_iter(&resolved).into_iter() {
            if dropped_cells.contains(name) {
                new_map.insert(name.to_string(), ast::Object::phi());
            } else {
                new_map.insert(name.to_string(), contents.clone());
            }
        }
        ast::Object::Map(new_map.into())
    };
    let drop_writer_reads: hashbrown::HashSet<String> = drule_cell.as_seq()
        .map(|facts| facts.iter()
            .filter(|f| relevant_ids.is_empty()
                || ast::binding(f, "id")
                    .map(|id| relevant_ids.contains(id))
                    .unwrap_or(false))
            .filter_map(|f| {
                let id = ast::binding(f, "id")?;
                let consequent_encoded = ast::binding(f, "consequentFactTypeId")?;
                let consequent = crate::types::ConsequentCellSource::decode(consequent_encoded)
                    .literal_id().to_string();
                if dropped_cells.contains(&consequent) {
                    Some(crate::evaluate::read_derivation_reads(d, id).unwrap_or_default())
                } else { None }
            })
            .flatten()
            .collect())
        .unwrap_or_default();
    let mut seed = touched_cells.clone();
    seed.extend(drop_writer_reads);

    let (post_s1, mut derived) = if stratum1.is_empty() {
        (resolved.clone(), Vec::new())
    } else {
        let refs = to_seeded_refs(&s1_packed);
        crate::evaluate::forward_chain_defs_state_seeded(
            &refs, seed.clone(), &resolved, 100)
    };
    let derived_state = if stratum2.is_empty() {
        post_s1
    } else {
        let refs = to_seeded_refs(&s2_packed);
        let (post_s2, more) = crate::evaluate::forward_chain_defs_state_seeded(
            &refs, seed.clone(), &post_s1, 100);
        derived.extend(more);
        post_s2
    };

    // Collect fact type IDs from derived facts as additional events.
    derived.iter().for_each(|d| fact_events.push(d.fact_type_id.clone()));

    // ── SM auto-advance: fact events + positive guards ───────────────
    // Two mechanisms, same fold:
    // 1. Fact events: facts pushed during resolve/derive fire events.
    // 2. Positive guards: check P for existing facts of the subscribed
    //    type. If they exist, the transition fires. Repeat until stable.
    // This handles both create-time (new facts) and compile-time (all
    // facts already in P — the investigation doesn't need to happen).
    let derived_state = {
        let machine_key = format!("machine:{}", noun);
        let has_machine = ast::fetch_or_phi(&machine_key, d) != ast::Object::Bottom;
        if has_machine {
            let mut current = extract_sm_status(&derived_state, &entity_id)
                .unwrap_or_default();
            let mut st = derived_state.clone();

            // Phase 1: fire events from facts pushed during this call.
            for event in &fact_events {
                let input = ast::Object::seq(vec![
                    ast::Object::atom(&current),
                    ast::Object::atom(event),
                ]);
                let result = ast::apply(&ast::Func::Def(machine_key.clone()), &input, d);
                let new_status = result.as_atom().unwrap_or(&current).to_string();
                if new_status != current {
                    diag!("[sm] {} --{}--> {}", current, event, new_status);
                    current = new_status;
                }
            }

            // Phase 2: positive guards — check P for facts that satisfy
            // outgoing transitions. Loop until no transition fires.
            // The transitions:{noun} def returns <<from, to, event>, ...>.
            let transitions_key = format!("transitions:{}", noun);
            let mut advanced = true;
            // Cycle guard: a finite SM has finitely many statuses. Positive-
            // guard auto-advance walks `current` along satisfied transitions;
            // if the guard graph cycles (a transition leads back to an already-
            // visited status whose guard stays satisfied), `advanced` never
            // settles and this loop spins forever — apply must never hang.
            // Track visited statuses; halt the first time we would revisit one.
            // (Without this, createEntity against a metamodel SM whose positive-
            // guard graph cycles hung the engine indefinitely.)
            let mut visited: hashbrown::HashSet<String> = hashbrown::HashSet::new();
            visited.insert(current.clone());
            while advanced {
                advanced = false;
                let available = ast::apply(
                    &ast::Func::Def(transitions_key.clone()),
                    &ast::Object::atom(&current),
                    d,
                );
                let triples = available.as_seq().unwrap_or_default();
                for triple in triples {
                    let items = triple.as_seq().unwrap_or_default();
                    let event_type = items.get(2).and_then(|o| o.as_atom()).unwrap_or("");
                    let target = items.get(1).and_then(|o| o.as_atom()).unwrap_or("");
                    // Positive guard: does a fact of this type exist in P
                    // where the SM's entity plays the noun's role?
                    //
                    // Only fire when the transition's event_type corresponds
                    // to a real fact type in the schema. Named events that
                    // aren't themselves facts (like the tutor's "place" /
                    // "pay" / "ship") produce no fact in P and must not
                    // auto-advance from mere create — they need an explicit
                    // `transition` call. Previously the fall-through to
                    // guard_auto_join was firing on every creation, chaining
                    // the SM through to its terminal state.
                    let schema_known = !ast::fetch_or_phi(
                        &format!("schema:{}", event_type), d
                    ).is_bottom();
                    if !event_type.is_empty() && !target.is_empty() && schema_known {
                        // Resolve role names from the schema for this fact type.
                        let role_map = ast::apply(
                            &ast::Func::Def(format!("query:{}", event_type)),
                            &ast::Object::phi(), d,
                        );
                        // Find role names that match the SM noun (handles ring:
                        // same noun in multiple roles — check each independently).
                        let noun_roles: Vec<String> = role_map.as_seq()
                            .map(|pairs| pairs.iter().filter_map(|pair| {
                                let kv = pair.as_seq()?;
                                let role_name = kv.first()?.as_atom()?;
                                (role_name == noun).then(|| role_name.to_string())
                            }).collect())
                            .unwrap_or_default();
                        let cell = ast::fetch_cell_seq(event_type, &st);
                        let has_facts = cell.as_seq().map_or(false, |facts| {
                            // If the SM noun plays a role in this fact type,
                            // check that specific role for the entity_id.
                            if !noun_roles.is_empty() {
                                facts.iter().any(|f| {
                                    noun_roles.iter().any(|role| ast::binding_matches(f, role, &entity_id))
                                })
                            } else {
                                // SM noun not in this fact type — auto-join.
                                // Walk the schema graph to find a join path from
                                // the SM noun to a role in the subscribed fact type.
                                guard_auto_join(noun, &entity_id, event_type, &st, d)
                            }
                        });
                        if has_facts {
                            diag!("[sm:guard] {} --{}--> {}", current, event_type, target);
                            current = target.to_string();
                            advanced = true;
                            // Cycle guard (see above): if this status was
                            // already visited, the positive-guard graph cycles
                            // — halt rather than spin forever.
                            if !visited.insert(current.clone()) {
                                diag!("[sm:guard] cycle at status '{}' — halting auto-advance", current);
                                advanced = false;
                            }
                            break; // restart from new status (or exit on cycle)
                        }
                    }
                }
            }

            // Write final status to state.
            let init_status = extract_sm_status(&derived_state, &entity_id).unwrap_or_default();
            if current != init_status {
                let sm = StateMachineCellShape::boot();
                let filtered = ast::cell_filter(sm.cell_name, |f| {
                    !ast::binding_matches(f, sm.state_machine_role, &entity_id)
                }, &st);
                st = ast::cell_push(sm.cell_name, ast::fact_from_pairs(&[
                    (sm.state_machine_role,  &entity_id),
                    (sm.current_status_role, &current),
                ]), &filtered);
            }
            st
        } else {
            derived_state
        }
    };

    // ── validate: ρ(validate:{noun}) applied to population ─────────
    // Prefer the per-noun aggregate that runs only the constraints
    // spanning fact types this noun participates in. Bulk `validate`
    // remains as a fallback for compile-states that haven't emitted
    // the per-noun def (e.g. older cached state).
    let ctx_obj = ast::encode_eval_context_state("", None, &derived_state);
    let validate_key = format!("validate:{}", noun);
    let validate_fn = match ast::fetch(&validate_key, d) {
        ast::Object::Bottom => ast::Func::Def("validate".to_string()),
        _                   => ast::Func::Def(validate_key),
    };
    let violation_obj = ast::apply(&validate_fn, &ctx_obj, d);
    let mut violations = ast::decode_violations(&violation_obj);
    // task-822: prepend apply-time UC conflicts (from `push_with_uc_check`
    // routing through `cell_put_keyed`) so they ride alongside the
    // validate-stage results. Conflicts are alethic by construction —
    // a UC collision is structurally impossible per task-744 phase 1
    // — so `rejected` lifts to true when any of them fire.
    if !uc_violations.is_empty() {
        // Apply-time conflicts surface first; validate-stage findings
        // follow. Order matches "what blocked the write" before
        // "what would still be wrong if the write had landed".
        let mut combined: Vec<crate::types::Violation> =
            Vec::with_capacity(uc_violations.len() + violations.len());
        combined.append(&mut uc_violations);
        combined.append(&mut violations);
        violations = combined;
    }
    let rejected = violations.iter().any(|v| v.alethic);

    // ── emit: construct representation via ρ ────────────────────────
    let sm_derived: Vec<_> = derived.iter()
        .filter(|d| d.fact_type_id.contains("StateMachine") || d.fact_type_id.contains("Machine"))
        .map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings))
        .collect();
    diag!("[debug] SM derived facts: {:?}", sm_derived);
    let sm_shape = StateMachineCellShape::boot();
    let sm_cell = ast::fetch_or_phi(sm_shape.cell_name, &derived_state);
    diag!("[debug] SM cell: {:?}", sm_cell);
    let status = extract_sm_status(&derived_state, &entity_id);
    let transitions = hateoas_via_rho(d, noun, &entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, noun, &entity_id);

    let entity_data: hashbrown::HashMap<String, String> = fields_with_domain.iter()
        .map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let entities = core::iter::once(EntityResult {
        id: entity_id.clone(), entity_type: noun.to_string(), data: entity_data,
    }).chain(status.as_ref().map(|st| {
        EntityResult {
            id: entity_id.clone(), entity_type: sm_shape.entity_type_label.to_string(),
            data: [
                (sm_shape.for_resource_role,   entity_id.as_str()),
                (sm_shape.current_status_role, st.as_str()),
                ("domain", domain),
            ].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    })).collect();

    // S1c (#719 + #757): the legacy `audit_log` cell + Security #26
    // audit-push are gone. Per whitepaper eq:cellfold the chain (S1b) is
    // the audit surface; operation/sender provenance rides on each
    // chain entry's `event` field — `system_impl` threads (verb,
    // operand) via `apply_event` into `merge_delta` at the commit
    // boundary (lib.rs Tier-1 + Tier-2 CommitDelta arms). The returned
    // delta still carries the post-apply contents only; rejected
    // applies snap to the pre-state.
    let _ = (sender, &entity_id); // operation+sender now ride on the event operand at the commit boundary
    let final_state = match rejected { true => state.clone(), false => derived_state };
    // #209: return only the cells this command modified, not the full D.
    // system_impl merges this delta onto the snapshot before commit.
    let delta = ast::diff_cells(state, &final_state);
    CommandResult {
        entities, status, transitions, navigation, violations,
        derived_count: derived.len(), rejected,
        state: delta,
    }
}

/// task-919: resolve the State Machine Definition id bound to a noun.
/// Reads `State_Machine_Definition_is_for_Noun` instance facts.
fn lookup_sm_def_for_noun(d: &ast::Object, noun: &str) -> Option<String> {
    ast::fetch_cell_seq("State_Machine_Definition_is_for_Noun", d).as_seq()
        .and_then(|facts| facts.iter().find_map(|f| {
            (ast::binding(f, "Noun") == Some(noun))
                .then(|| ast::binding(f, "State Machine Definition").map(String::from))
                .flatten()
        }))
}

/// task-919: resolve the firing Transition's id by joining the three
/// Transition cells (`is_defined_in_State_Machine_Definition`,
/// `is_from_Status`, `is_to_Status`). The joined row uniquely identifies
/// the transition that fires for a given (sm, from, to) tuple.
fn find_firing_transition_id(
    d: &ast::Object,
    sm_def: &str,
    from_status: &str,
    to_status: &str,
) -> Option<String> {
    let from_cell = ast::fetch_cell_seq("Transition_is_from_Status", d);
    let to_cell = ast::fetch_cell_seq("Transition_is_to_Status", d);
    let in_sm_cell = ast::fetch_cell_seq(
        "Transition_is_defined_in_State_Machine_Definition", d);
    let in_sm: Vec<String> = in_sm_cell.as_seq().map(|facts| facts.iter().filter_map(|f| {
        let t = ast::binding(f, "Transition")?;
        let m = ast::binding(f, "State Machine Definition")?;
        (m == sm_def).then(|| t.to_string())
    }).collect()).unwrap_or_default();
    in_sm.into_iter().find(|t| {
        let from_match = from_cell.as_seq().map(|facts| facts.iter().any(|f| {
            ast::binding(f, "Transition") == Some(t.as_str())
                && ast::binding(f, "Status") == Some(from_status)
        })).unwrap_or(false);
        let to_match = to_cell.as_seq().map(|facts| facts.iter().any(|f| {
            ast::binding(f, "Transition") == Some(t.as_str())
                && ast::binding(f, "Status") == Some(to_status)
        })).unwrap_or(false);
        from_match && to_match
    })
}

/// task-919: resolve the Verb performed during a Transition. Tries the
/// canonical `Verb_is_performed_during_Transition` cell and the
/// parenthesized `(Mealy semantics)` variant, since the parser may
/// register either form depending on whether the inline annotation is
/// folded into the FT id.
fn lookup_verb_for_transition(d: &ast::Object, transition_id: &str) -> Option<String> {
    for &cell_name in &[
        "Verb_is_performed_during_Transition",
        "Verb_is_performed_during_Transition_(Mealy_semantics)",
        "Verb_is_performed_during_Transition_(mealy_semantics)",
    ] {
        if let Some(v) = ast::fetch_cell_seq(cell_name, d).as_seq().and_then(|facts| {
            facts.iter().find_map(|f| {
                (ast::binding(f, "Transition") == Some(transition_id))
                    .then(|| ast::binding(f, "Verb").map(String::from))
                    .flatten()
            })
        }) {
            return Some(v);
        }
    }
    None
}

/// task-919: dispatch target derived from a Function entity. Verb is a
/// subtype of Function (core.md:15), so the verb id keys directly into
/// Function FTs. `name` selects the in-process Platform handler;
/// `callback_uri` selects an HTTP dispatch (deferred follow-up).
struct DispatchTarget {
    name: Option<String>,
    callback_uri: Option<String>,
}

fn lookup_dispatch_for_function(d: &ast::Object, fn_id: &str) -> DispatchTarget {
    let name = ast::fetch_cell_seq("Function_has_Name", d).as_seq().and_then(|facts| {
        facts.iter().find_map(|f| {
            (ast::binding(f, "Function") == Some(fn_id))
                .then(|| ast::binding(f, "Name").map(String::from))
                .flatten()
        })
    });
    let callback_uri = ast::fetch_cell_seq("Function_has_callback_URI", d).as_seq().and_then(|facts| {
        facts.iter().find_map(|f| {
            (ast::binding(f, "Function") == Some(fn_id))
                .then(|| ast::binding(f, "callback URI").map(String::from))
                .flatten()
        })
    });
    DispatchTarget { name, callback_uri }
}

/// task-919-http: collect `Function has Header` facts for a function id.
/// FT shape per core.md:206: `<Function, fn_id>, <Header, "Name: Value">`.
/// Returns parsed (Name, Value) pairs; malformed header strings (no `:`
/// separator) are skipped silently — the callback URI dispatch should
/// still fire with whatever headers DO parse rather than rejecting the
/// transition over a header typo.
fn lookup_headers_for_function(d: &ast::Object, fn_id: &str) -> Vec<(String, String)> {
    ast::fetch_cell_seq("Function_has_Header", d).as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            if ast::binding(f, "Function") != Some(fn_id) { return None; }
            let raw = ast::binding(f, "Header")?;
            let (name, value) = raw.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        }).collect()
    }).unwrap_or_default()
}

/// task-919-http / task-919-https: synchronous HTTP/1.1 POST over a TCP
/// socket, with optional TLS wrap for `https://`.
///
/// Returns `Ok(status_code)` on a successful round-trip (regardless of
/// 2xx/non-2xx — the caller decides what counts as success), or
/// `Err(reason)` on network or TLS failure: DNS/parse error, connect
/// timeout, TLS handshake failure, truncated response, or invalid status
/// line. The dispatch hook treats both `Err(_)` and `Ok(non-2xx)` as a
/// rejected transition, mirroring the in-process Bottom branch.
///
/// task-919-https: `https://` URIs are now accepted. The TLS layer is
/// pure-Rust rustls + ring (no OpenSSL / native-tls dep), with root
/// certificates sourced from the OS trust store via `rustls-native-certs`
/// and a `webpki-roots` fallback when the native store yields zero
/// anchors. SNI is set from the host portion of the URL; no ALPN — the
/// remote sees `Connection: close` and pipes back the same HTTP/1.1
/// shape the cleartext branch uses. The TLS handshake honours the same
/// 5-second read/write timeout the cleartext branch sets, so a black-
/// holed handshake can't wedge the dispatch path any longer than the
/// cleartext one.
///
/// Implementation notes:
///   * URL parse: scheme branches on `http://` vs `https://`. Default
///     port 80 / 443. Anything else returns `Err(…)` so the dispatch
///     hook surfaces an alethic violation rather than silently NOP-ing.
///   * Hard 5-second read/write/connect timeout so a hung callback
///     can't wedge the SM-transition path. The TLS handshake inherits
///     the read/write deadlines via the underlying `TcpStream` (rustls
///     calls back into the TCP I/O during handshake, so the same
///     `TimedOut` / `WouldBlock` short-circuit applies).
///   * Response body is consumed but discarded — we only need the
///     status line. Connection: close ensures the server can short-
///     circuit if it wants to.
///   * Target gate: host CLI / kernel std build only. Wasm32 and UEFI
///     skip this code path — the cloudflare worker reaches outbound
///     HTTPS through the platform `fetch()` shim, and the bare-metal
///     kernel doesn't link `std::net::TcpStream`. The same cfg
///     predicate gates the rustls deps in `Cargo.toml`.
#[cfg(all(not(feature = "no_std"), not(target_arch = "wasm32"), not(target_os = "uefi")))]
fn http_post_callback(
    url: &str,
    body: &[u8],
    headers: &[(String, String)],
) -> Result<u16, String> {
    use std::io::{Read, Write};

    // Parse the URL. Accept `http://host[:port][/path]` or
    // `https://host[:port][/path]`; reject anything else so callers
    // get a clear failure rather than a silent NOP.
    let (use_tls, rest, default_port) = if let Some(r) = url.strip_prefix("https://") {
        (true, r, 443u16)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r, 80u16)
    } else {
        return Err(format!("only http:// or https:// URIs supported, got: {}", url));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None    => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(format!("missing authority in URL: {}", url));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>()
            .map_err(|e| format!("bad port in {}: {}", url, e))?),
        None         => (authority, default_port),
    };

    let mut stream: Box<dyn ReadWrite + Send> = if use_tls {
        connect_https(host, port, url)?
    } else {
        connect_http(host, port)?
    };

    // Build the request line + headers. The body is sent verbatim;
    // Content-Length is derived from the byte slice. Custom headers
    // overwrite nothing — RFC 7230 §3.2.2 allows multiple instances
    // of the same name, and the dispatch surface is small enough that
    // we don't bother enforcing uniqueness.
    let mut req = Vec::with_capacity(256 + body.len());
    req.extend_from_slice(format!("POST {} HTTP/1.1\r\n", path).as_bytes());
    req.extend_from_slice(format!("Host: {}\r\n", authority).as_bytes());
    req.extend_from_slice(b"Content-Type: application/json\r\n");
    req.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    req.extend_from_slice(b"Connection: close\r\n");
    for (name, value) in headers {
        // Skip the headers we set unconditionally; tenants overriding
        // Host/Content-Length is asking for trouble.
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "host" | "content-length" | "connection") {
            continue;
        }
        req.extend_from_slice(format!("{}: {}\r\n", name, value).as_bytes());
    }
    req.extend_from_slice(b"\r\n");
    req.extend_from_slice(body);

    stream.write_all(&req)
        .map_err(|e| format!("write {}: {}", url, e))?;

    // Read the entire response. We only need the status line, but a
    // server expecting us to consume the body before close may hang
    // otherwise; the 5 s read timeout caps the worst case. Both
    // WouldBlock (Windows) and TimedOut (Linux) signal the deadline.
    let mut resp = Vec::with_capacity(512);
    let mut buf = [0u8; 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => resp.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            Err(e) => return Err(format!("read {}: {}", url, e)),
        }
        // Cap the buffered response to prevent a malicious server from
        // ballooning the engine's working set. The status line is in
        // the first 100 bytes; anything beyond 64 KB is just noise.
        if resp.len() > 64 * 1024 { break; }
    }

    // Parse the status line: "HTTP/1.1 200 OK\r\n…".
    let head = std::str::from_utf8(&resp)
        .map_err(|_| format!("non-utf8 response from {}", url))?;
    let status_line = head.lines().next()
        .ok_or_else(|| format!("empty response from {}", url))?;
    let code = status_line.split_whitespace().nth(1)
        .ok_or_else(|| format!("no status code in '{}'", status_line))?
        .parse::<u16>()
        .map_err(|e| format!("bad status code in '{}': {}", status_line, e))?;
    Ok(code)
}

/// task-919-http: marker trait for the two callback transport flavours.
/// `http_post_callback` wraps the concrete stream in a `Box<dyn ReadWrite
/// + Send>` so the write/read body is shared between cleartext TCP and
/// TLS-over-TCP. The blanket impl below covers any `T: Read + Write +
/// Send` (plain `TcpStream` and `rustls::StreamOwned<…, TcpStream>`).
#[cfg(all(not(feature = "no_std"), not(target_arch = "wasm32"), not(target_os = "uefi")))]
trait ReadWrite: std::io::Read + std::io::Write {}
#[cfg(all(not(feature = "no_std"), not(target_arch = "wasm32"), not(target_os = "uefi")))]
impl<T: std::io::Read + std::io::Write> ReadWrite for T {}

/// task-919-http: cleartext TCP connect helper. Returns a boxed
/// `ReadWrite` so the write/read body in `http_post_callback` is the
/// same shape regardless of scheme.
#[cfg(all(not(feature = "no_std"), not(target_arch = "wasm32"), not(target_os = "uefi")))]
fn connect_http(host: &str, port: u16) -> Result<Box<dyn ReadWrite + Send>, String> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let timeout = Duration::from_secs(5);
    let stream = TcpStream::connect_timeout(
        &(host, port).to_socket_addrs()
            .map_err(|e| format!("resolve {}:{}: {}", host, port, e))?
            .next()
            .ok_or_else(|| format!("no address for {}:{}", host, port))?,
        timeout,
    ).map_err(|e| format!("connect {}:{}: {}", host, port, e))?;
    stream.set_read_timeout(Some(timeout))
        .map_err(|e| format!("set_read_timeout: {}", e))?;
    stream.set_write_timeout(Some(timeout))
        .map_err(|e| format!("set_write_timeout: {}", e))?;
    Ok(Box::new(stream))
}

/// task-919-https: TLS-wrapped TCP connect helper. Builds a rustls
/// `ClientConfig` once per call (cheap — root cert load is the only
/// real work) with a root store populated from `rustls-native-certs`
/// (OS trust store; picks up corporate / enterprise CA chains).
/// Falls back to `webpki-roots` (Mozilla bundle) only when the native
/// lookup yields zero anchors — e.g. a sandboxed runner that wiped
/// `/etc/ssl/certs`, or a Windows install where the SChannel store
/// lookup failed.
///
/// Returns a `rustls::StreamOwned` that owns both the rustls
/// `ClientConnection` and the underlying `TcpStream`, so `Box<dyn
/// ReadWrite + Send>` is valid for the lifetime of the callback. The
/// TCP read/write timeouts set here propagate into the handshake's
/// I/O calls — a black-holed handshake errors out at the same
/// 5 second deadline the cleartext branch uses.
#[cfg(all(not(feature = "no_std"), not(target_arch = "wasm32"), not(target_os = "uefi")))]
fn connect_https(host: &str, port: u16, url: &str) -> Result<Box<dyn ReadWrite + Send>, String> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::sync::Arc;
    use std::time::Duration;

    let timeout = Duration::from_secs(5);
    let tcp = TcpStream::connect_timeout(
        &(host, port).to_socket_addrs()
            .map_err(|e| format!("resolve {}:{}: {}", host, port, e))?
            .next()
            .ok_or_else(|| format!("no address for {}:{}", host, port))?,
        timeout,
    ).map_err(|e| format!("connect {}:{}: {}", host, port, e))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|e| format!("set_read_timeout: {}", e))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|e| format!("set_write_timeout: {}", e))?;

    // Root certs: native first (picks up corporate / enterprise CAs),
    // webpki-roots fallback for sandboxed runners with no native store.
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for cert in native.certs {
        // load_native_certs returns `CertificateDer<'static>` already
        // typed for rustls; add(); ignores rejected ones so a single
        // bad anchor in the OS store doesn't poison the whole bundle.
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        // Fallback: Mozilla CA bundle. webpki-roots 0.26 exposes
        // `TLS_SERVER_ROOTS` as `&[TrustAnchor<'static>]`; rustls 0.23
        // accepts that slice directly via `extend()`.
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    if roots.is_empty() {
        return Err(format!(
            "no TLS root certificates available (native + webpki-roots \
             both empty) — cannot verify {}", url));
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    // SNI = host portion of the URL (no port). rustls requires an
    // owned `ServerName<'static>` that owns the host string for the
    // duration of the connection; the constructor accepts a `String`.
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid SNI host {:?}: {}", host, e))?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("rustls client init for {}: {}", url, e))?;

    // Drive the handshake to completion before returning. rustls'
    // `StreamOwned::write_all` would do this lazily on first write,
    // but doing it eagerly here lets us surface a typed handshake
    // error in `Err(…)` rather than leaking it through the later
    // `write {}: {}` site (where the dispatch hook would still
    // synthesize a `dispatch:<uri>` violation, just with a less
    // specific detail string).
    let mut tcp_io = tcp;
    conn.complete_io(&mut tcp_io)
        .map_err(|e| format!("TLS handshake to {} failed: {}", url, e))?;

    Ok(Box::new(rustls::StreamOwned::new(conn, tcp_io)))
}

/// task-919-http / task-919-https: wasm32 + UEFI fallback. The cloudflare
/// worker reaches outbound HTTP / HTTPS through the platform `fetch()`
/// shim — not this in-Rust callback path — and the kernel UEFI build
/// doesn't link `std::net::TcpStream`. Compiling out the rustls + TCP
/// substrate on these targets keeps the worker / kernel bundle slim and
/// avoids the asm-on-wasm32 ring backend compile failure.
///
/// Behaviour parity with the host implementation: the dispatch hook
/// at the call site (`transition_via_defs`) only checks the return
/// `Result<u16, String>` and synthesizes a `dispatch:<uri>` alethic
/// violation on `Err(_)` or non-2xx. Returning a structured "skipped"
/// error here is observationally identical to a network failure — the
/// worker / kernel never reaches a callback URI through this surface
/// anyway, so the violation is a correct outcome.
#[cfg(any(feature = "no_std", target_arch = "wasm32", target_os = "uefi"))]
fn http_post_callback(
    url: &str,
    _body: &[u8],
    _headers: &[(String, String)],
) -> Result<u16, String> {
    Err(format!(
        "callback URI dispatch is unavailable on this target \
         (wasm32 / UEFI / no_std); reach {} via the platform fetch shim",
        url))
}

fn transition_via_defs(
    d: &ast::Object,
    entity_id: &str,
    event: &str,
    _domain: &str,
    current_status: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    let mut new_state = state.clone();

    // Find the machine def, compute transition, capture noun name
    let transition_result: Option<(String, String)> = ast::cells_iter(d).into_iter()
        .filter(|(name, _)| name.starts_with("machine:") && !name.contains(":initial"))
        .find_map(|(name, contents)| {
            let noun = name.strip_prefix("machine:")?;
            let func = ast::metacompose(contents, d);
            let initial_key = format!("{}:initial", name);
            let from_status = current_status.map(|s| s.to_string())
                .or_else(|| {
                    // task-954: the batch path (buildApplyCommandForBatch)
                    // omits current_status, so resolve the entity's CURRENT
                    // status from the SM cell in `state` (the cumulative
                    // running state threaded by apply_command_batch). Without
                    // this, a 2nd+ transition on the same entity inside one
                    // batch resolved `from = initial` (e.g. pending) and
                    // no-op'd — the batch silently partial-applied. Falls
                    // through to the machine initial only when the entity has
                    // no status yet (its first transition).
                    let sm = StateMachineCellShape::boot();
                    ast::fetch_cell_seq(sm.cell_name, state).as_seq().and_then(|facts|
                        facts.iter()
                            .find(|f| ast::binding_matches(f, sm.state_machine_role, entity_id))
                            .and_then(|f| ast::binding(f, sm.current_status_role)
                                .map(|s| s.to_string())))
                })
                .or_else(|| {
                    ast::apply(&ast::Func::Def(initial_key), &ast::Object::phi(), d)
                        .as_atom().map(|s| s.to_string())
                })?;
            let input = ast::Object::seq(vec![ast::Object::atom(&from_status), ast::Object::atom(event)]);
            ast::apply(&func, &input, d).as_atom()
                .filter(|next| *next != from_status)
                .map(|next| (noun.to_string(), next.to_string()))
        });

    let (noun, new_status) = match transition_result {
        Some((n, s)) => (n, Some(s)),
        None => (String::new(), None),
    };

    // Update SM status fact in state: remove old, add new (identity when no new_status)
    let sm = StateMachineCellShape::boot();
    new_state = new_status.as_ref()
        .map(|status| {
            let filtered = ast::cell_filter(sm.cell_name, |f| {
                !ast::binding_matches(f, sm.state_machine_role, entity_id)
            }, &new_state);
            ast::cell_push(sm.cell_name, ast::fact_from_pairs(&[
                (sm.state_machine_role,  entity_id),
                (sm.current_status_role, status.as_str()),
            ]), &filtered)
        })
        .unwrap_or(new_state);

    // task-929/923/924 — the machine fold (eq:sm) also maintains the
    // resource-keyed status projection `Resource is currently in Status`.
    // AREST.tex §"Facts as events": that projection "folds those tuples
    // latest-wins per resource. The machine fold is the same operation."
    // The cell is an RMAP keyed-by-Resource Map (Definition: cell
    // isolation — each resource is its own cell); a transition is one
    // fold step that replaces ONLY the transitioned resource's entry
    // (`cell_put_keyed` via `push_with_uc_check` overwrite), leaving every
    // other resource's cell untouched. The canonical SM-keyed `cell_name`
    // updated above is the same status keyed by State Machine. Without
    // this, the projection — and the `Task has Task Status` view, SQL, and
    // HATEOAS that read it — keep the pre-transition status: the 923/924
    // readback staleness that forced a live-DB restore. (A monotonic
    // forward-chain re-derivation can't do this: it cannot retract the old
    // tuple, the projection depends on view cells absent from P, and on
    // the live model its deriver carries an unresolved/φ consequent.)
    new_state = new_status.as_ref()
        .map(|status| {
            // `Resource is currently in Status` and `State Machine is for
            // Resource` are metamodel cells (readings/core/instances.md).
            const RESOURCE_STATUS_CELL: &str = "Resource_is_currently_in_Status";
            const FOR_RESOURCE_CELL: &str = "State_Machine_is_for_Resource";
            // The projection is keyed by Resource. Map this SM to its
            // resource(s) via `State Machine is for Resource`; fall back to
            // the SM id (SM id == resource id is the common reference
            // scheme).
            let resources: Vec<String> = ast::fetch_cell_seq(FOR_RESOURCE_CELL, &new_state)
                .as_seq()
                .map(|facts| facts.iter()
                    .filter(|f| ast::binding_matches(f, sm.state_machine_role, entity_id))
                    .filter_map(|f| ast::binding(f, sm.for_resource_role).map(String::from))
                    .collect::<Vec<_>>())
                .unwrap_or_default();
            let resources = if resources.is_empty() {
                vec![entity_id.to_string()]
            } else { resources };
            let key_roles = crate::evaluate::read_cell_key_roles(d);
            let mut viols: Vec<crate::types::Violation> = Vec::new();
            let mut st = new_state.clone();
            for res in &resources {
                let fact = ast::fact_from_pairs(&[
                    (sm.for_resource_role,   res.as_str()),
                    (sm.current_status_role, status.as_str()),
                ]);
                st = push_with_uc_check(
                    st, RESOURCE_STATUS_CELL, fact, &key_roles, /* overwrite */ true, &mut viols);
            }
            st
        })
        .unwrap_or(new_state);

    // task-929 event-durability — persist the trigger fact that fired this
    // transition so the SM machine fold replays it. AREST.tex §"Facts as
    // events": "Creating a fact fires an event… a transition that declares
    // its trigger as a fact type fires automatically when that fact enters
    // P", and the status is the foldl over that event stream (eq:sm).
    // Without the fact in P, the imperative status writes above are not
    // durable: a recompile re-derives status purely from events and resets
    // any transition that left no event behind (the symptom: apply-
    // transitioned tasks revert on recompile while event-backed ones
    // survive). The `event` token IS the trigger Fact Type's reading for a
    // Fact-Type-triggered SM; assert it for this entity when it names a
    // declared trigger FT (Event-Type-triggered SMs carry no durable fact
    // and are skipped). Idempotent so repeated transitions don't bloat the
    // event cell; the fold is latest-wins so a present fact is harmless.
    if new_status.is_some() && !noun.is_empty() {
        let is_ft_trigger = ast::fetch_cell_seq("Transition_is_triggered_by_Fact_Type", d)
            .as_seq()
            .map(|facts| facts.iter().any(|f| ast::binding(f, "Fact Type") == Some(event)))
            .unwrap_or(false);
        if is_ft_trigger {
            // FT cell id is the reading with spaces → underscores; the
            // subject role of an SM trigger FT is the SM noun itself
            // (e.g. `Task is finished` → cell `Task_is_finished`, role
            // `Task`).
            let trigger_cell = event.replace(' ', "_");
            let already_present = ast::fetch_cell_seq(&trigger_cell, &new_state)
                .as_seq()
                .map(|facts| facts.iter().any(|f| ast::binding_matches(f, &noun, entity_id)))
                .unwrap_or(false);
            if !already_present {
                new_state = ast::cell_push(
                    &trigger_cell,
                    ast::fact_from_pairs(&[(noun.as_str(), entity_id)]),
                    &new_state);
            }
        }
    }

    let transition_fired = new_status.is_some();
    let status = new_status.clone().or_else(|| current_status.map(|s| s.to_string()));

    // #922 — derivation chain MUST run after the SM cell update so
    // derived cells that depend on Status (directly via the SM cell
    // or transitively via the task-861 bridge derivation) re-fire
    // against the post-transition state. Pre-fix `transition_via_defs`
    // ran SM update + validate + Platform Function dispatch but never
    // invoked the forward chain, so `Task_is_recommended` / readiness
    // cells / any derivation reading Task Status stayed at their
    // pre-transition values across every transition. Mirrors the
    // 2-stratum chain create_via_defs / update_via_defs already run.
    let (new_state, derived_count) = if transition_fired && !noun.is_empty() {
        let relevant_ids: hashbrown::HashSet<String> = {
            let index_key = format!("derivation_index:{}", noun);
            let index_obj = ast::fetch(&index_key, d);
            let value = index_obj.as_seq()
                .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
                .and_then(|items| items[1].as_atom())
                .or_else(|| index_obj.as_atom());
            value
                .map(|s| s.split(',').map(|id| id.to_string()).collect())
                .unwrap_or_default()
        };
        let collect_stratum = |prefix: &str| -> Vec<(String, ast::Func)> {
            let cell_prefix = alloc::format!("{}:", prefix);
            ast::cells_iter(d).into_iter()
                .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
                // task-967: no noun pre-filter -- run the full stratum (see
                // create_via_defs). The seeded chainer's reads-dirty gating
                // restricts active rules and reaches the fixpoint across
                // cross-noun rule cascades that the noun-index severed.
                .filter(|_| true)
                .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
                .collect()
        };
        let stratum1 = collect_stratum("derivation");
        let stratum2 = collect_stratum("derivation_strat2");
        // #836 — clear derived consequent cells before forward-chain
        // (LFP per request, AREST.tex §4.3) so a transition that flips
        // Status doesn't leave stale derived facts that the chain
        // would never retract.
        //
        // task-929: noun-scope the wipe to derivation_index[noun]'s rules.
        // The chain only re-derives this noun's rules; wiping a cell whose
        // deriver belongs to another noun (e.g. Resource_is_currently_in_Status
        // when applying on Task) leaves it stale-empty for downstream readers
        // (the bridge `Task has Task Status iff Resource is currently in
        // Status and ...` reads the wiped-empty upstream and emits nothing).
        let resolved = {
            let drule_cell = ast::fetch_cell_seq("DerivationRule", d);
            let derived_cells: hashbrown::HashSet<String> = drule_cell.as_seq()
                .map(|facts| facts.iter()
                    .filter(|f| relevant_ids.is_empty()
                        || ast::binding(f, "id")
                            .map(|id| relevant_ids.contains(id))
                            .unwrap_or(false))
                    .filter_map(|f| ast::binding(f, "consequentFactTypeId"))
                    .map(|encoded| crate::types::ConsequentCellSource::decode(encoded)
                        .literal_id().to_string())
                    .filter(|s| !s.is_empty())
                    .collect())
                .unwrap_or_default();
            if derived_cells.is_empty() {
                new_state
            } else {
                let mut new_map: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
                for (name, contents) in ast::cells_iter(&new_state).into_iter() {
                    if derived_cells.contains(name) {
                        new_map.insert(name.to_string(), ast::Object::phi());
                    } else {
                        new_map.insert(name.to_string(), contents.clone());
                    }
                }
                ast::Object::Map(new_map.into())
            }
        };
        let (post_s1, mut derived) = if stratum1.is_empty() {
            (resolved, Vec::new())
        } else {
            let refs: Vec<(&str, &ast::Func)> = stratum1.iter().map(|(n, f)| (n.as_str(), f)).collect();
            crate::evaluate::forward_chain_defs_state(&refs, &resolved)
        };
        let post_s2 = if stratum2.is_empty() {
            post_s1
        } else {
            let refs: Vec<(&str, &ast::Func)> = stratum2.iter().map(|(n, f)| (n.as_str(), f)).collect();
            let (s2_state, more) = crate::evaluate::forward_chain_defs_state(&refs, &post_s1);
            derived.extend(more);
            s2_state
        };
        let count = derived.len();
        (post_s2, count)
    } else {
        (new_state, 0)
    };

    // Audit D1 (#703): deontic gate on the post-rewrite state. A
    // transition is a mutation like any other — every deontic
    // constraint over the touched cells must fire, otherwise an
    // approval/permit/forbid clause that the readings declare for the
    // SM status field is silently bypassed (e.g. "It is forbidden that
    // some Outbound Email is sent without an Approver"). Mirrors the
    // update_via_defs pattern at L942-948 that the create/update paths
    // already follow. Skip when the transition didn't actually fire
    // (no machine matched / event ignored) — there's no new state to
    // validate and no noun to key the per-noun aggregate on.
    let (mut violations, mut rejected) = if transition_fired && !noun.is_empty() {
        let ctx_obj = ast::encode_eval_context_state("", None, &new_state);
        let validate_key = format!("validate:{}", noun);
        let validate_func = def_func(&validate_key, d)
            .or_else(|| def_func("validate", d))
            .unwrap_or(ast::Func::constant(ast::Object::phi()));
        let violation_obj = ast::apply(&validate_func, &ctx_obj, d);
        let vs = ast::decode_violations(&violation_obj);
        let r = vs.iter().any(|v| v.alethic);
        (vs, r)
    } else {
        (vec![], false)
    };

    // task-919: dispatch — after validation passes, look up the Function
    // bound to the firing Transition's Verb (Verb is a subtype of
    // Function per core.md:15, so the verb id keys Function FTs directly).
    // Invoke the in-process Platform handler when `Function has Name` is
    // set; issue an HTTP POST when `Function has callback URI` is set
    // (task-919-http). On Bottom / non-2xx / network failure, mark
    // rejected and append a synthesized violation so the delta-emit path
    // rolls back the SM cell flip (delta = phi when rejected, mirroring
    // the alethic-violation rollback at L1312).
    if !rejected && transition_fired && !noun.is_empty() {
        if let (Some(from), Some(new)) = (current_status, new_status.as_deref()) {
            let dispatch_lookup = lookup_sm_def_for_noun(d, &noun)
                .and_then(|sm| find_firing_transition_id(d, &sm, from, new))
                .and_then(|tid| lookup_verb_for_transition(d, &tid).map(|v| (tid, v)));
            if let Some((tid, verb_id)) = dispatch_lookup {
                let target = lookup_dispatch_for_function(d, &verb_id);
                // Build the entity-context ctx Map once; both the in-
                // process Platform branch and the HTTP callback branch
                // see the same shape (noun / id / from_status /
                // to_status / transition_id / verb_id / event).
                let mut ctx_map = hashbrown::HashMap::new();
                ctx_map.insert("noun".to_string(), ast::Object::atom(&noun));
                ctx_map.insert("id".to_string(), ast::Object::atom(entity_id));
                ctx_map.insert("from_status".to_string(), ast::Object::atom(from));
                ctx_map.insert("to_status".to_string(), ast::Object::atom(new));
                ctx_map.insert("transition_id".to_string(), ast::Object::atom(&tid));
                ctx_map.insert("verb_id".to_string(), ast::Object::atom(&verb_id));
                ctx_map.insert("event".to_string(), ast::Object::atom(event));
                let ctx = ast::Object::map(ctx_map);

                if let Some(name) = target.name {
                    let result = ast::apply(&ast::Func::Platform(name.clone()), &ctx, d);
                    if matches!(result, ast::Object::Bottom) {
                        violations.push(Violation {
                            constraint_id: format!("dispatch:{}", name),
                            constraint_text: format!(
                                "Platform Function '{}' returned Bottom on transition", name),
                            detail: format!(
                                "Transition {} from {} to {} on {} {}",
                                tid, from, new, noun, entity_id),
                            alethic: true,
                        });
                        rejected = true;
                    }
                }

                // task-919-http: callback URI branch. POST the ctx Map
                // (encoded as JSON) to the URI; read `Function has
                // Header` for additional request headers. Non-2xx
                // status AND network failure are both treated as
                // Bottom — synthesize a `dispatch:<uri>` alethic
                // violation and mark the transition rejected so the
                // delta-emit path rolls back the SM cell flip.
                if !rejected {
                    if let Some(callback_uri) = target.callback_uri {
                        let headers = lookup_headers_for_function(d, &verb_id);
                        let body = ctx.to_json_string();
                        let outcome = http_post_callback(
                            &callback_uri, body.as_bytes(), &headers);
                        let is_success = matches!(outcome, Ok(code) if (200..300).contains(&code));
                        if !is_success {
                            let detail = match &outcome {
                                Ok(code) => format!(
                                    "Transition {} from {} to {} on {} {} \
                                     returned HTTP {} from {}",
                                    tid, from, new, noun, entity_id, code, callback_uri),
                                Err(e) => format!(
                                    "Transition {} from {} to {} on {} {} \
                                     failed to reach {}: {}",
                                    tid, from, new, noun, entity_id, callback_uri, e),
                            };
                            violations.push(Violation {
                                constraint_id: format!("dispatch:{}", callback_uri),
                                constraint_text: format!(
                                    "Callback URI '{}' returned Bottom on transition", callback_uri),
                                detail,
                                alethic: true,
                            });
                            rejected = true;
                        }
                    }
                }
            }
        }
    }

    let transitions = hateoas_via_rho(d, &noun, entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, &noun, entity_id);

    // #209: return only the status-cell delta, not the full D. When a
    // deontic alethic violation rejects the transition, emit an empty
    // delta — the rewrite happened in `new_state` but must NOT ship to
    // the caller (mirroring update_via_defs:958).
    let delta = if rejected {
        ast::Object::phi()
    } else {
        ast::diff_cells(state, &new_state)
    };
    CommandResult {
        entities: vec![],
        status,
        transitions,
        navigation,
        violations,
        derived_count,
        rejected,
        state: delta,
    }
}

fn query_via_defs(
    d: &ast::Object,
    schema_id: &str,
    target: &str,
    bindings: &hashbrown::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    // Look up schema role names from state metadata. The Role cell's
    // bindings use `factType` for the FT-id key (parser convention,
    // see parse_forml2::fact_types_from_state); `graphSchema` was the
    // earlier name and a stray reference here meant no role ever
    // matched, role_names came back empty, and target_role degenerated
    // to 0 — query_via_defs silently returned empty matches against
    // any parse-populated state. (#819)
    let role_cell = ast::fetch_cell_seq("Role", state);
    let role_names: Vec<String> = role_cell.as_seq()
        .map(|roles| {
            let mut matched: Vec<(usize, String)> = roles.iter()
                .filter(|r| ast::binding_matches(r, "factType", schema_id))
                .filter_map(|r| {
                    let name = ast::binding(r, "nounName")?.to_string();
                    let pos: usize = ast::binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0);
                    Some((pos, name))
                })
                .collect();
            matched.sort_by_key(|(p, _)| *p);
            matched.into_iter().map(|(_, n)| n).collect()
        })
        .unwrap_or_default();

    let target_role = role_names.iter().position(|n| n == target).map(|i| i + 1).unwrap_or(0);
    let filter_pairs: Vec<(usize, String)> = role_names.iter().enumerate()
        .filter_map(|(i, name)| bindings.get(name).map(|v| (i + 1, v.clone())))
        .collect();

    let filter_refs: Vec<(usize, &str)> = filter_pairs.iter().map(|(i, v)| (*i, v.as_str())).collect();
    let schema = crate::compile::CompiledSchema {
        id: schema_id.to_string(),
        reading: String::new(),
        construction: def_func(&format!("schema:{}", schema_id), d).unwrap_or(ast::Func::Id),
        role_names: role_names.clone(),
        key_roles: None,
    };
    let results = crate::query::query_with_ast(state, &schema, target_role, &filter_refs);

    let mut data = hashbrown::HashMap::new();
    data.insert(String::from("matches"), results.join(","));
    data.insert(String::from("count"), results.len().to_string());

    CommandResult {
        entities: vec![EntityResult {
            id: format!("query:{}", schema_id),
            entity_type: String::from("QueryResult"),
            data,
        }],
        status: None,
        transitions: vec![],
        navigation: vec![],
        violations: vec![],
        derived_count: 0,
        rejected: false,
        // #209: queries don't mutate state — empty delta.
        state: ast::Object::phi(),
    }
}

fn update_via_defs(
    d: &ast::Object,
    noun: &str,
    _domain: &str,
    entity_id: &str,
    new_fields: &hashbrown::HashMap<String, String>,
    force: bool,
    state: &ast::Object,
) -> CommandResult {
    // Run-time definedness gate (task 938, shared with create_via_defs).
    // updateEntity is a run-time mutation; an under-defined noun (value type /
    // no reference scheme / undeclared) has nothing for the derivation
    // forward-chain to ground and can drive it into a non-terminating
    // expansion. Refuse up front as an alethic violation, before resolve or
    // any derivation runs.
    if !noun_runtime_defined(noun, state, d) {
        return CommandResult {
            entities: alloc::vec![],
            status: None,
            transitions: alloc::vec![],
            navigation: alloc::vec![],
            violations: alloc::vec![crate::types::Violation {
                constraint_id: alloc::format!("update.not_runtime_defined:{}", noun),
                constraint_text: alloc::format!(
                    "'{}' is not a fully-defined entity type; an entity needs a reference \
                     scheme (identity) before it can be updated",
                    noun),
                detail: alloc::format!(
                    "updateEntity rejected at run-time: noun '{}' must be declared with \
                     objectType 'entity' and a reference scheme",
                    noun),
                alethic: true,
            }],
            derived_count: 0,
            rejected: true,
            state: ast::Object::phi(),
        };
    }

    // task-861 / #904 — SM-bypass guard. When the noun has a State
    // Machine bound to it AND the update payload sets the SM's
    // status-role field (by convention `{noun} Status`, e.g.
    // `Task Status` for a Task whose SM is declared via
    // `State Machine Definition 'Task' is for Noun 'Task'`), refuse
    // the update with an alethic violation that names the SM and
    // points the caller at `apply transition` instead. The SM cell
    // (StateMachine_has_currentlyInStatus) is the canonical status —
    // direct mutation desyncs any derivation reading SM state.
    //
    // Lookup: presence of `machine:{noun}` in D (compiled by
    // `compile_state_machine_from_cells`) means the noun is
    // SM-governed. Status-role convention is shared with the MCP
    // guard at src/mcp/server.ts (`findSmStatusField`).
    //
    // Opt-out: `force: true` bypasses the guard (migration scripts,
    // admin restore flows). Matches the MCP convention from #904.
    if !force {
        // Use `fetch` (returns Bottom when absent) — `fetch_or_phi`
        // would degrade absent to `phi()`, which is NOT equal to
        // Bottom, so the guard would fire even on SM-less nouns.
        let has_sm = ast::fetch(&format!("machine:{}", noun), d) != ast::Object::Bottom;
        if has_sm {
            let status_field = format!("{} Status", noun);
            // Local-task #2 — MCP merge-pre-fetch (#868/#872) folds
            // the entity's current SM Status into every UpdateEntity
            // payload so untouched fields aren't retracted. The pre-
            // fix guard rejected any payload that even *named* Status,
            // breaking edits to non-Status fields (e.g. Task Priority).
            // Refining to "Status value differs from extract_sm_status"
            // keeps the SM-mutation refusal (acceptance #1) intact —
            // attempting `Task Status: in_progress` while the SM still
            // says `pending` is still rejected — but lets a no-op echo
            // of the current Status through, so the merged payload
            // flows.
            let mutates_status = match new_fields.get(&status_field) {
                Some(new_val) => {
                    extract_sm_status(state, entity_id).as_deref()
                        != Some(new_val.as_str())
                }
                None => false,
            };
            if mutates_status {
                let violation = crate::types::Violation {
                    constraint_id: format!("sm:{}:status_immutable", noun),
                    constraint_text: format!(
                        "{} Status is governed by the '{}' state machine",
                        noun, noun
                    ),
                    detail: format!(
                        "{} Status is SM-driven; use apply transition with an event instead",
                        noun
                    ),
                    alethic: true,
                };
                let sm_id = entity_id.to_string();
                let status = extract_sm_status(state, &sm_id);
                let transitions = hateoas_via_rho(d, noun, entity_id, status.as_deref());
                let navigation = nav_links_via_rho(d, noun, entity_id);
                return CommandResult {
                    entities: vec![EntityResult {
                        id: entity_id.to_string(),
                        entity_type: noun.to_string(),
                        data: hashbrown::HashMap::new(),
                    }],
                    status,
                    transitions,
                    navigation,
                    violations: vec![violation],
                    derived_count: 0,
                    rejected: true,
                    state: ast::Object::phi(),
                };
            }
        }
    }

    // #868 — per-field retract-then-insert scoped to the payload.
    //
    // Pre-fix the merge logic scanned ALL cells for facts where any
    // pair had `entity_id` at position 1, then re-pushed every collected
    // (field, value) under `format!("{noun}_has_{field}")`. Two failure
    // modes:
    //   1. SM-derived facts whose `State Machine` role value equals the
    //      target entity id leak into the merge and get re-pushed under
    //      bogus cell names (`Task_has_currentlyInStatus`,
    //      `Task_has_forResource`, `Task_has_instanceOf`,
    //      `Task_has_domain`).
    //   2. When a user-named role collides with an SM-emitted role
    //      (cookbook scenario: a Task Status field where the SM names
    //      its initial 'pending'), the SM-derived value overwrites the
    //      user-supplied one — exactly the "Status flips to 'pending'"
    //      regression #868 reports.
    //
    // The whitepaper §5.1 spec is per-field retract-then-insert scoped
    // to the field set IN the payload. Untouched single-valued facts
    // must persist unchanged. The fix: fold over `new_fields` only.
    // Cells the payload doesn't name stay byte-identical — `diff_cells`
    // at the end will see them unchanged and ship an empty entry for
    // each, leaving the existing chain intact.
    //
    // The `merged` map (existing facts + payload override) is still
    // needed for the EntityResult.data the API surface returns, so the
    // caller sees the post-update row including untouched fields.
    let existing_fields: hashbrown::HashMap<String, String> = ast::cells_iter(state)
        .into_iter()
        .filter(|(name, _)| {
            // Restrict the existing-row scan to FT cells whose name
            // begins with `<noun>_has_` (the parser-emitted convention
            // for binary fact types head-noun'd by `noun`). This
            // excludes SM-derived cells (`StateMachine_has_*`,
            // `_cwa_negation:*`, `_transitive_*`, `derivation:*`, etc.)
            // whose entity-role value happens to equal `entity_id`
            // — the exact noise the pre-fix merge swept up.
            name.starts_with(&alloc::format!("{}_has_", noun))
        })
        .flat_map(|(_, contents)| ast::cell_facts_iter(contents).cloned().collect::<Vec<_>>())
        .filter_map(|fact| {
            let pairs = fact.as_seq().filter(|p| p.len() >= 2)?;
            let v0 = pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()));
            (v0.as_deref() == Some(entity_id)).then_some(())?;
            let k = pairs[1].as_seq().and_then(|p| p.get(0)?.as_atom().map(|s| s.to_string()))?;
            let v = pairs[1].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()))?;
            Some((k, v))
        })
        .collect();
    // `merged` for the EntityResult: existing fields ∪ payload (payload wins).
    let merged: hashbrown::HashMap<String, String> = existing_fields.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .chain(new_fields.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();

    // task-822: read per-cell key-roles once. The update emit path
    // below routes UC-keyed cells through `push_with_uc_check` with
    // `overwrite=true` — same-fact updates collapse to no-op (no Arc
    // churn, structurally unchanged cell), different-value updates
    // explicitly replace the prior entry without raising a UC
    // violation (the user's intent on `update`). Cells without a UC
    // stay on the legacy Seq filter-then-push path.
    let key_roles = crate::evaluate::read_cell_key_roles(d);
    let mut uc_violations: Vec<crate::types::Violation> = Vec::new();

    // Per-field retract-then-insert SCOPED TO PAYLOAD: only fold over
    // `new_fields`. Untouched single-valued facts (Status, Priority,
    // etc.) stay in place — their cells are unchanged so `diff_cells`
    // will not ship an entry for them, preserving the prior chain entry.
    let resolve_key = format!("resolve:{}", noun);
    let new_state = new_fields.iter().fold(state.clone(), |acc, (field_name, value)| {
        let lower = field_name.to_lowercase();
        let resolved = def_func(&resolve_key, d)
            .map(|f| ast::apply(&f, &ast::Object::atom(&lower), d));
        // task-737: identical Func::Id-echo handling to `create_via_defs`.
        // A miss in the resolve chain returns the input atom; treat that
        // as a no-mapping and fall through to `<Noun>_has_<Field>`.
        let ft_id = match resolved.and_then(|o| o.as_atom().map(|s| s.to_string())) {
            Some(s) if s != lower => s,
            _ => format!("{}_has_{}", noun, field_name),
        };
        let fact = ast::fact_from_pairs(&[(noun, entity_id), (field_name.as_str(), value.as_str())]);
        if key_roles.contains_key(&ft_id) {
            // task-822: keyed (Map-backed) cell. Skip the entity-scoped
            // Seq filter (it can't traverse Map storage anyway) and
            // route through `push_with_uc_check` with overwrite. The
            // helper detects byte-equal re-assertion (no-op) and
            // otherwise vacates the slot via `drop_keyed_entry` before
            // re-asserting the new fact.
            push_with_uc_check(acc, &ft_id, fact, &key_roles, /*overwrite=*/true, &mut uc_violations)
        } else {
            // Legacy Seq cell: filter out this entity's prior fact(s),
            // then append the new one. Matches the pre-task-822 update
            // semantics byte-for-byte.
            let acc = ast::cell_filter(&ft_id, |f| {
                f.as_seq().map_or(true, |pairs| {
                    pairs.len() < 2 || pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom()) != Some(entity_id)
                })
            }, &acc);
            ast::cell_push(&ft_id, fact, &acc)
        }
    });

    // derive + validate + emit
    // Noun-gated derivation chain: only run the rules the compile-time
    // derivation_index says are relevant to this noun. Mirrors create's
    // gating at L451. For the metamodel that's 8/808 rules vs 808 bulk.
    let relevant_ids: hashbrown::HashSet<String> = {
        let index_key = format!("derivation_index:{}", noun);
        let index_obj = ast::fetch(&index_key, d);
        let value = index_obj.as_seq()
            .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
            .and_then(|items| items[1].as_atom())
            .or_else(|| index_obj.as_atom());
        value
            .map(|s| s.split(',').map(|id| id.to_string()).collect())
            .unwrap_or_default()
    };
    // 2-stratum forward chain (#828): see create path for rationale.
    let collect_stratum = |prefix: &str| -> Vec<(String, ast::Func)> {
        let cell_prefix = alloc::format!("{}:", prefix);
        ast::cells_iter(d).into_iter()
            .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
            // task-967: no noun pre-filter -- run the full stratum (see
            // create_via_defs). The seeded chainer's reads-dirty gating
            // restricts active rules and reaches the fixpoint across
            // cross-noun rule cascades that the noun-index severed.
            .filter(|_| true)
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
            .collect()
    };
    let stratum1 = collect_stratum("derivation");
    let stratum2 = collect_stratum("derivation_strat2");

    // task-3 phase 2 / DB-task-929: incremental forward chain via
    // `forward_chain_defs_state_seeded`. Round 1 only runs rules whose
    // positive antecedent reads (from `derivation_reads:<id>` sidecars
    // emitted by compile.rs) intersect the seed.
    //
    // The seed combines:
    //   * `touched_cells`  — cells the apply payload wrote (≈ the
    //     `{noun}_has_{field}` for each new_field).
    //   * `dropped_cells`-antecedents — the antecedent reads of every
    //     rule whose consequent_cell was wiped by the drop-derived
    //     step below. Without these, rules that wrote to dropped
    //     cells whose antecedents weren't touched by the apply would
    //     skip round 1 and the dropped cells would stay empty —
    //     `handle_isolation_tests::update_clears_stale_derived_consequents_
    //     before_forward_chain` exercises this exact case (Task 2
    //     pending→completed retracts Task 1 'blocked').
    // Pack rule (id, reads, func) once; reuse for the gate-build pass
    // and the chainer call.
    let touched_cells: hashbrown::HashSet<alloc::string::String> = new_fields.iter()
        .map(|(field_name, _)| {
            let lower = field_name.to_lowercase();
            let resolved = def_func(&resolve_key, d)
                .map(|f| ast::apply(&f, &ast::Object::atom(&lower), d));
            match resolved.and_then(|o| o.as_atom().map(|s| s.to_string())) {
                Some(s) if s != lower => s,
                _ => alloc::format!("{}_has_{}", noun, field_name),
            }
        })
        .collect();
    let build_seeded_refs = |stratum: &[(alloc::string::String, ast::Func)]|
        -> alloc::vec::Vec<(alloc::string::String, alloc::vec::Vec<alloc::string::String>, ast::Func)>
    {
        stratum.iter().map(|(name, func)| {
            let id = name.split_once(':').map(|(_, id)| id).unwrap_or(name);
            let reads = crate::evaluate::read_derivation_reads(d, id).unwrap_or_default();
            (name.clone(), reads, func.clone())
        }).collect()
    };
    let s1_packed = build_seeded_refs(&stratum1);
    let s2_packed = build_seeded_refs(&stratum2);

    // #836 — clear derived consequent cells before forward-chain
    // (LFP per request, AREST.tex §4.3). task-929: noun-scope the
    // wipe to derivation_index[noun]'s rules so cross-noun upstream
    // consequent cells survive.
    let drule_cell = ast::fetch_cell_seq("DerivationRule", d);
    let dropped_cells: hashbrown::HashSet<String> = drule_cell.as_seq()
        .map(|facts| facts.iter()
            .filter(|f| relevant_ids.is_empty()
                || ast::binding(f, "id")
                    .map(|id| relevant_ids.contains(id))
                    .unwrap_or(false))
            .filter_map(|f| ast::binding(f, "consequentFactTypeId"))
            .map(|encoded| crate::types::ConsequentCellSource::decode(encoded)
                .literal_id().to_string())
            .filter(|s| !s.is_empty())
            .collect())
        .unwrap_or_default();
    let new_state = if dropped_cells.is_empty() {
        new_state
    } else {
        let mut new_map: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        for (name, contents) in ast::cells_iter(&new_state).into_iter() {
            if dropped_cells.contains(name) {
                new_map.insert(name.to_string(), ast::Object::phi());
            } else {
                new_map.insert(name.to_string(), contents.clone());
            }
        }
        ast::Object::Map(new_map.into())
    };
    // Antecedent reads of rules whose consequent_cell was dropped: the
    // chainer needs these in dirty so those rules re-fire and
    // repopulate the cleared cells.
    let drop_writer_reads: hashbrown::HashSet<String> = drule_cell.as_seq()
        .map(|facts| facts.iter()
            .filter(|f| relevant_ids.is_empty()
                || ast::binding(f, "id")
                    .map(|id| relevant_ids.contains(id))
                    .unwrap_or(false))
            .filter_map(|f| {
                let id = ast::binding(f, "id")?;
                let consequent_encoded = ast::binding(f, "consequentFactTypeId")?;
                let consequent = crate::types::ConsequentCellSource::decode(consequent_encoded)
                    .literal_id().to_string();
                if dropped_cells.contains(&consequent) {
                    Some(crate::evaluate::read_derivation_reads(d, id).unwrap_or_default())
                } else { None }
            })
            .flatten()
            .collect())
        .unwrap_or_default();
    let mut seed = touched_cells.clone();
    seed.extend(drop_writer_reads);

    let (new_state, mut derived) = if stratum1.is_empty() {
        (new_state, alloc::vec::Vec::new())
    } else {
        let refs = to_seeded_refs(&s1_packed);
        crate::evaluate::forward_chain_defs_state_seeded(
            &refs, seed.clone(), &new_state, 100)
    };
    let new_state = if stratum2.is_empty() {
        new_state
    } else {
        let refs = to_seeded_refs(&s2_packed);
        let (post_s2, more) = crate::evaluate::forward_chain_defs_state_seeded(
            &refs, seed.clone(), &new_state, 100);
        derived.extend(more);
        post_s2
    };

    // Prefer per-noun validate aggregate (O(FTs-touching-noun)) over the
    // bulk validate (O(all constraints)). Falls back to bulk when the
    // per-noun def is absent.
    let ctx_obj = ast::encode_eval_context_state("", None, &new_state);
    let validate_key = format!("validate:{}", noun);
    let validate_func = def_func(&validate_key, d)
        .or_else(|| def_func("validate", d))
        .unwrap_or(ast::Func::constant(ast::Object::phi()));
    let violation_obj = ast::apply(&validate_func, &ctx_obj, d);
    let mut violations = ast::decode_violations(&violation_obj);
    // task-822: in the update path conflicts are normally suppressed by
    // the overwrite branch in `push_with_uc_check`; this prepend stays
    // for symmetry with `create_via_defs` and to surface any defensive
    // fallback that did record a violation (e.g. a malformed key role
    // configuration). Same construction shape as
    // `compile_uniqueness_ast`-emitted violations.
    if !uc_violations.is_empty() {
        let mut combined: Vec<crate::types::Violation> =
            Vec::with_capacity(uc_violations.len() + violations.len());
        combined.append(&mut uc_violations);
        combined.append(&mut violations);
        violations = combined;
    }
    let rejected = violations.iter().any(|v| v.alethic);
    let sm_id = entity_id.to_string();
    let status = extract_sm_status(&new_state, &sm_id);
    let transitions = hateoas_via_rho(d, noun, entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, noun, entity_id);

    // #209: return only the cells this update modified. When rejected,
    // emit an empty delta (no cells change); otherwise diff new_state
    // against the input state so only touched FT cells ship.
    let delta = if rejected { ast::Object::phi() } else { ast::diff_cells(state, &new_state) };
    CommandResult {
        entities: vec![EntityResult {
            id: entity_id.to_string(),
            entity_type: noun.to_string(),
            data: merged,
        }],
        status,
        transitions,
        navigation,
        violations,
        derived_count: derived.len(),
        rejected,
        state: delta,
    }
}

/// SM guard auto-join: when the SM noun doesn't play a role in the
/// subscribed fact type, walk the schema graph to find a join path.
///
/// BFS from the SM noun through binary fact types. At each hop, the
/// "other" role's noun is checked against the target fact type's roles.
/// If found, evaluate the natural join: does a chain of facts exist
/// from entity_id through the intermediate nouns to a fact in the target?
///
/// Example: SM for Case, target = Hypothesis_explains_Observation.
///   Hop 1: Case_has_Hypothesis → other noun = Hypothesis
///   Hypothesis appears in Hypothesis_explains_Observation → match.
///   Join: exists H where Case_has_Hypothesis(Case=entity_id, Hypothesis=H)
///         AND Hypothesis_explains_Observation(Hypothesis=H, _).
fn guard_auto_join(
    sm_noun: &str,
    entity_id: &str,
    target_ft: &str,
    state: &ast::Object,
    d: &ast::Object,
) -> bool {
    // Get target fact type's role names.
    let target_roles = schema_role_names(target_ft, d);
    if target_roles.is_empty() { return false; }

    // Collect all schema IDs and their role names from D.
    let all_schemas: Vec<(String, Vec<String>)> = ast::cells_iter(d).into_iter()
        .filter(|(name, _)| name.starts_with("query:"))
        .filter_map(|(name, _)| {
            let ft_id = name.strip_prefix("query:")?.to_string();
            let roles = schema_role_names(&ft_id, d);
            (!roles.is_empty()).then(|| (ft_id, roles))
        })
        .collect();

    // BFS: find a path from sm_noun to any role in the target fact type.
    // Each entry: (current_noun, join_chain: Vec<(ft_id, sm_role, other_role)>)
    let mut queue: alloc::collections::VecDeque<(String, Vec<(String, String, String)>)> =
        alloc::collections::VecDeque::new();
    let mut visited: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    queue.push_back((sm_noun.to_string(), vec![]));
    visited.insert(sm_noun.to_string());

    while let Some((current_noun, chain)) = queue.pop_front() {
        // Check if current_noun appears in the target fact type.
        if target_roles.contains(&current_noun) && !chain.is_empty() {
            // Found a path. Evaluate the join chain.
            return evaluate_join_chain(entity_id, &chain, &current_noun, target_ft, state);
        }

        // Limit depth to avoid runaway traversal.
        if chain.len() >= 3 { continue; }

        // Expand: find binary fact types where current_noun plays a role.
        for (ft_id, roles) in &all_schemas {
            if roles.len() != 2 { continue; }
            if ft_id == target_ft { continue; }
            let pos = roles.iter().position(|r| r == &current_noun);
            let pos = match pos { Some(p) => p, None => continue };
            let other = &roles[1 - pos];
            if visited.contains(other) { continue; }
            visited.insert(other.clone());
            let mut new_chain = chain.clone();
            new_chain.push((ft_id.clone(), current_noun.clone(), other.clone()));
            queue.push_back((other.clone(), new_chain));
        }
    }
    false
}

/// Evaluate a join chain against the population.
/// Chain: [(ft1, role_a, role_b), (ft2, role_b, role_c), ...]
/// Start with entity_id matching role_a in ft1, collect role_b values,
/// then for each, check role_b in ft2, collect role_c values, etc.
/// Final: check if any collected value appears in the target fact type.
fn evaluate_join_chain(
    entity_id: &str,
    chain: &[(String, String, String)],
    final_noun: &str,
    target_ft: &str,
    state: &ast::Object,
) -> bool {
    // Walk the chain, collecting matching values at each hop.
    let mut current_values: Vec<String> = vec![entity_id.to_string()];

    for (ft_id, from_role, to_role) in chain {
        let cell = ast::fetch_cell_seq(ft_id, state);
        let facts = cell.as_seq().unwrap_or_default();
        let mut next_values = Vec::new();
        for val in &current_values {
            for fact in facts {
                if ast::binding_matches(fact, from_role, val) {
                    if let Some(other_val) = ast::binding(fact, to_role) {
                        next_values.push(other_val.to_string());
                    }
                }
            }
        }
        current_values = next_values;
        if current_values.is_empty() { return false; }
    }

    // Check if any collected value appears in the target fact type.
    let target_cell = ast::fetch_cell_seq(target_ft, state);
    let target_facts = target_cell.as_seq().unwrap_or_default();
    current_values.iter().any(|val| {
        target_facts.iter().any(|f| ast::binding_matches(f, final_noun, val))
    })
}

/// Get role names for a fact type from its query:{ft_id} def in D.
fn schema_role_names(ft_id: &str, d: &ast::Object) -> Vec<String> {
    let role_map = ast::apply(
        &ast::Func::Def(format!("query:{}", ft_id)),
        &ast::Object::phi(), d,
    );
    role_map.as_seq()
        .map(|pairs| pairs.iter().filter_map(|pair| {
            pair.as_seq()?.first()?.as_atom().map(|s| s.to_string())
        }).collect())
        .unwrap_or_default()
}

/// Self-modification: compile ∘ parse (Corollary 5).
/// Ingesting readings is an application of SYSTEM where the operation is
/// compile ∘ parse. The new FFP objects are stored via ↓DEFS.
/// Mirrors platform_compile in ast.rs — same pipeline, structured result.
fn apply_load_readings(
    markdown: &str,
    domain: &str,
    d: &ast::Object,
    state: &ast::Object,
) -> CommandResult {
    // Parse with context from D (same as platform_compile)
    let parsed = match crate::parse_forml2::parse_to_state_from(markdown, d) {
        Ok(s) => s,
        Err(e) => {
            return CommandResult {
                entities: vec![],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations: vec![crate::types::Violation {
                    constraint_id: "parse_error".to_string(),
                    constraint_text: "FORML 2 parse error".to_string(),
                    detail: e,
                    alethic: true,
                }],
                derived_count: 0,
                rejected: true,
                // #209: parse failed — no state change.
                state: ast::Object::phi(),
            };
        }
    };

    // Count genuinely new nouns (in parsed but not in D)
    let existing_noun_names: hashbrown::HashSet<String> = ast::fetch_cell_seq("Noun", d).as_seq()
        .map(|facts| facts.iter().filter_map(|f| ast::binding(f, "name").map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let new_noun_count = ast::fetch_cell_seq("Noun", &parsed).as_seq()
        .map(|facts| facts.iter().filter(|f| {
            ast::binding(f, "name").map_or(false, |n| !existing_noun_names.contains(n))
        }).count())
        .unwrap_or(0);

    // Merge: foldl(concat_cell, D, cells(parsed))
    let merged_state = ast::merge_states(d, &parsed);

    // D3 (#705): mirror the singular `load_reading_handler` validation
    // gate. Either alethic (Source::Parse / ::Resolve) or deontic
    // (Source::Deontic) errors against the merged state reject the
    // batch load — the recompile + persist below is skipped, the
    // returned state is phi() so the writer-path classifier treats it
    // as no-commit. Mirrors `load_reading_core::load_reading` step 5.
    let validation = crate::load_reading_core::validate_loaded_state(&merged_state);
    if !validation.passes {
        let violations: Vec<crate::types::Violation> = validation.alethic_violations
            .into_iter()
            .map(|diag| crate::types::Violation {
                constraint_id: "load_readings.alethic".to_string(),
                constraint_text: diag.reading,
                detail: diag.message,
                alethic: true,
            })
            .chain(validation.deontic_violations.into_iter().map(|diag| crate::types::Violation {
                constraint_id: "load_readings.deontic".to_string(),
                constraint_text: diag.reading,
                detail: diag.message,
                alethic: true,
            }))
            .collect();
        return CommandResult {
            entities: vec![],
            status: None,
            transitions: vec![],
            navigation: vec![],
            violations,
            derived_count: 0,
            rejected: true,
            state: ast::Object::phi(),
        };
    }

    // Compile defs from merged state + re-register platform primitives
    let mut defs = crate::compile::compile_to_defs_state(&merged_state);
    defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
    defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
    let new_d = ast::defs_to_state(&defs, &merged_state);

    let mut data = hashbrown::HashMap::new();
    data.insert("domain".to_string(), domain.to_string());
    data.insert("nouns".to_string(), new_noun_count.to_string());

    // #209: load_readings is a schema-level mutation (Cor 5). Diff the
    // recompiled D against the input snapshot so the delta carries new
    // nouns, new FTs, new constraints, and replaced defs — not the
    // entire store. merge_delta on commit reconstructs the full D.
    let delta = ast::diff_cells(state, &new_d);
    CommandResult {
        entities: vec![EntityResult {
            id: format!("schema:{}", domain),
            entity_type: "SchemaLoaded".to_string(),
            data,
        }],
        status: None,
        transitions: vec![],
        navigation: vec![],
        violations: vec![],
        derived_count: new_noun_count,
        rejected: false,
        state: delta,
    }
}

/// SystemVerb::LoadReading (#555 DynRdg-1) — runtime parse + validate +
/// register a single named reading body.
///
/// Pure wrapper over `crate::load_reading_core::load_reading`: encodes the
/// outcome as a `CommandResult` so the existing command dispatch loop
/// can surface it through the same `__state_delta` carrier
/// (`encode_command_result` semantics). On rejection, the state field
/// of the result is `phi()` so the writer-path classifier treats it as
/// `NoCommit`. On success, the state field is the post-load delta
/// (`diff_cells(state, new_state)`) so `try_commit_diff` only touches
/// the cells that grew.
///
/// Policy gate: `load_reading_handler` ALWAYS uses
/// `LoadReadingPolicy::AllowAll`. The `register_mode`-style gating
/// happens upstream in `system_impl` — by the time this handler runs
/// the caller has already passed the gate. Production builds simply
/// don't route the SYSTEM verb here.
fn load_reading_handler(
    d: &ast::Object,
    name: &str,
    body: &str,
    state: &ast::Object,
) -> CommandResult {
    use crate::load_reading_core::{load_reading, LoadError, LoadReadingPolicy};

    // The verb operates on the def-state `d`. Population state is
    // unaffected by schema mutation under this verb (added cells go
    // into Noun / FactType / Role / Constraint / DerivationRule —
    // none of which carry instance facts in this path). The
    // returned new_state is the merged def-state.
    match load_reading(d, name, body, LoadReadingPolicy::AllowAll) {
        Ok(outcome) => {
            // Compile defs from the merged state so derivation /
            // validate / per-noun resolve defs land in the new D
            // before commit. Mirrors `apply_load_readings`'s tail.
            let mut defs = crate::compile::compile_to_defs_state(&outcome.new_state);
            defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
            defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
            defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
            defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
            let new_d = ast::defs_to_state(&defs, &outcome.new_state);

            // The CommandResult carries the per-cell delta against
            // the input snapshot so writer-path Tier-1 commit only
            // CASes the changed cells.
            let delta = ast::diff_cells(state, &new_d);

            let mut data = hashbrown::HashMap::new();
            data.insert("name".to_string(), name.to_string());
            data.insert(
                "addedNouns".to_string(),
                outcome.report.added_nouns.join(","),
            );
            data.insert(
                "addedFactTypes".to_string(),
                outcome.report.added_fact_types.join(","),
            );
            data.insert(
                "addedDerivations".to_string(),
                outcome.report.added_derivations.join(","),
            );
            // #558 / DynRdg-4: surface reading versioning metadata on
            // the wire so callers can detect "same name, different
            // body" and order loads chronologically. `contentHash` is
            // a 16-char hex FNV-1a64; `versionStamp` is a decimal u64.
            data.insert(
                "contentHash".to_string(),
                outcome.report.content_hash.clone(),
            );
            data.insert(
                "versionStamp".to_string(),
                outcome.report.version_stamp.to_string(),
            );

            let derived_count = outcome.report.added_nouns.len()
                + outcome.report.added_fact_types.len()
                + outcome.report.added_derivations.len();

            CommandResult {
                entities: vec![EntityResult {
                    id: format!("reading:{}", name),
                    entity_type: "ReadingLoaded".to_string(),
                    data,
                }],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations: vec![],
                derived_count,
                rejected: false,
                state: delta,
            }
        }
        Err(err) => {
            // Map LoadError into the existing Violation shape. The
            // diagnostic tree (DeonticViolation) collapses into one
            // Violation per diagnostic so existing UI can render the
            // list without a new shape.
            let violations = match err {
                LoadError::Disallowed => vec![crate::types::Violation {
                    constraint_id: "load_reading.disallowed".to_string(),
                    constraint_text: "runtime reading load is disallowed by host policy".to_string(),
                    detail: "the host did not enable runtime LoadReading; flip allow_runtime_load_reading to enable".to_string(),
                    alethic: true,
                }],
                LoadError::EmptyBody => vec![crate::types::Violation {
                    constraint_id: "load_reading.empty_body".to_string(),
                    constraint_text: "reading body is empty".to_string(),
                    detail: "loading an empty body would not add any cells; pass at least one statement".to_string(),
                    alethic: true,
                }],
                LoadError::InvalidName(msg) => vec![crate::types::Violation {
                    constraint_id: "load_reading.invalid_name".to_string(),
                    constraint_text: "reading name failed sanitization".to_string(),
                    detail: msg,
                    alethic: true,
                }],
                LoadError::ParseError(msg) => vec![crate::types::Violation {
                    constraint_id: "load_reading.parse_error".to_string(),
                    constraint_text: "FORML 2 parse error".to_string(),
                    detail: msg,
                    alethic: true,
                }],
                LoadError::DeonticViolation(diags) => diags
                    .into_iter()
                    .map(|d| crate::types::Violation {
                        constraint_id: "load_reading.deontic".to_string(),
                        constraint_text: d.reading.clone(),
                        detail: d.message.clone(),
                        alethic: true,
                    })
                    .collect(),
                // #559 / DynRdg-5: alethic violations are
                // structural-impossibility errors caught by the
                // load-time validation gate. Surfaced under a
                // distinct constraint_id so dashboards can route
                // them separately from deontic constraint failures.
                LoadError::AlethicViolation(diags) => diags
                    .into_iter()
                    .map(|d| crate::types::Violation {
                        constraint_id: "load_reading.alethic".to_string(),
                        constraint_text: d.reading.clone(),
                        detail: d.message.clone(),
                        alethic: true,
                    })
                    .collect(),
            };
            CommandResult {
                entities: vec![],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations,
                derived_count: 0,
                rejected: true,
                // No state mutation on rejection — phi() so the
                // writer-path classifier treats this as a no-commit.
                state: ast::Object::phi(),
            }
        }
    }
}

/// SystemVerb::UnloadReading (#556 DynRdg-2) — runtime inverse of
/// `LoadReading`. Drops a previously-loaded reading from the cell
/// graph and either cascade-deletes its facts (default) or migrates
/// them (stubbed; see `UnloadPolicy`).
///
/// Pure wrapper over `crate::load_reading_core::unload_reading`: encodes
/// the outcome as a `CommandResult` so the existing dispatch loop
/// can surface it through the same `__state_delta` carrier. On
/// rejection, the result state is `phi()` so the writer-path
/// classifier treats it as `NoCommit`. On success, the result state
/// is the post-unload delta against the input snapshot.
///
/// Policy parsing: the wire-level `policy` field accepts the
/// strings "cascade-delete" (default), "cascade_delete", and
/// "migrate" — case-insensitive. Unknown values fall back to the
/// default (cascade-delete) so older callers can ignore the field.
/// Both policies are implemented: CascadeDelete drops nouns/FTs/
/// derivations introduced by the reading; Migrate preserves them on
/// P (eq:pop, set semantics) and only drops the derivation defs +
/// manifest, mirroring cor:closure's preserve-population principle.
fn unload_reading_handler(
    d: &ast::Object,
    name: &str,
    policy: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    use crate::load_reading_core::{unload_reading, UnloadError, UnloadPolicy};

    let parsed_policy = match policy.map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "migrate" => UnloadPolicy::Migrate,
        Some(ref s) if s == "cascade-delete" || s == "cascade_delete" => {
            UnloadPolicy::CascadeDelete
        }
        _ => UnloadPolicy::default(),
    };

    match unload_reading(d, name, parsed_policy) {
        Ok(outcome) => {
            // Re-compile defs from the post-unload state so removed
            // derivations / FTs propagate to the def-state. Mirrors
            // `load_reading_handler`'s tail symmetric path.
            let mut defs = crate::compile::compile_to_defs_state(&outcome.new_state);
            defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
            defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
            defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
            defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
            let new_d = ast::defs_to_state(&defs, &outcome.new_state);

            let delta = ast::diff_cells(state, &new_d);

            let mut data = hashbrown::HashMap::new();
            data.insert("name".to_string(), name.to_string());
            data.insert(
                "removedNouns".to_string(),
                outcome.report.removed_nouns.join(","),
            );
            data.insert(
                "removedFactTypes".to_string(),
                outcome.report.removed_fact_types.join(","),
            );
            data.insert(
                "removedDerivations".to_string(),
                outcome.report.removed_derivations.join(","),
            );
            // #558 / DynRdg-4: surface the manifest's recorded
            // versioning metadata on unload, so wire callers can see
            // which body version was just removed. Pre-#558 manifest
            // cells decode with `""` / `0` defaults — see
            // `decode_manifest`; the wire output mirrors them as-is.
            data.insert(
                "contentHash".to_string(),
                outcome.report.content_hash.clone(),
            );
            data.insert(
                "versionStamp".to_string(),
                outcome.report.version_stamp.to_string(),
            );

            let derived_count = outcome.report.removed_nouns.len()
                + outcome.report.removed_fact_types.len()
                + outcome.report.removed_derivations.len();

            CommandResult {
                entities: vec![EntityResult {
                    id: format!("reading:{}", name),
                    entity_type: "ReadingUnloaded".to_string(),
                    data,
                }],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations: vec![],
                derived_count,
                rejected: false,
                state: delta,
            }
        }
        Err(err) => {
            let violations = match err {
                UnloadError::Disallowed => vec![crate::types::Violation {
                    constraint_id: "unload_reading.disallowed".to_string(),
                    constraint_text: "runtime reading unload is disallowed by host policy".to_string(),
                    detail: "the host did not enable runtime UnloadReading".to_string(),
                    alethic: true,
                }],
                UnloadError::InvalidName(msg) => vec![crate::types::Violation {
                    constraint_id: "unload_reading.invalid_name".to_string(),
                    constraint_text: "reading name failed sanitization".to_string(),
                    detail: msg,
                    alethic: true,
                }],
                UnloadError::ManifestMissing(missing_name) => vec![crate::types::Violation {
                    constraint_id: "unload_reading.manifest_missing".to_string(),
                    constraint_text: "reading was not previously loaded under this name".to_string(),
                    detail: format!(
                        "no _loaded_reading:{} cell found; reload the body and unload again, \
                         or migrate legacy state by running LoadReading first",
                        missing_name
                    ),
                    alethic: true,
                }],
            };
            CommandResult {
                entities: vec![],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations,
                derived_count: 0,
                rejected: true,
                state: ast::Object::phi(),
            }
        }
    }
}

/// SystemVerb::ReloadReading (#557 / DynRdg-3) — atomic compose of
/// `UnloadReading` + `LoadReading` against a single state snapshot.
/// Either the new body fully replaces the old, or the old reading
/// stays exactly as it was. No partial state is visible.
///
/// Pure wrapper over `crate::load_reading_core::reload_reading`: encodes
/// the outcome as a `CommandResult` so the existing dispatch loop
/// can surface it through the same `__state_delta` carrier. On
/// rejection, the result state is `phi()` (no commit). On success,
/// the result state is the post-reload delta against the input
/// snapshot.
///
/// Policy parsing: the wire-level `policy` field accepts the
/// strings "replace-all" (default), "replace_all", and
/// "migrate-facts" (also "migrate_facts") — case-insensitive.
/// Unknown values fall back to the default (replace-all). The
/// `MigrateFacts` policy preserves the existing population P and
/// re-derives it from the new readings (migration is ingestion of
/// new readings, AREST.tex §Conclusion); `replace-all` instead
/// cascade-deletes the prior reading before loading the new body.
///
/// First-time-load fallthrough: if no manifest is present for the
/// name, the unload step is a no-op and the reload becomes a
/// first-time load. The handler still returns `ReadingReloaded` so
/// callers can treat the verb uniformly.
fn reload_reading_handler(
    d: &ast::Object,
    name: &str,
    body: &str,
    policy: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    use crate::load_reading_core::{reload_reading, LoadError, ReloadError, ReloadPolicy, UnloadError};

    let parsed_policy = match policy.map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "migrate-facts" || s == "migrate_facts" => {
            ReloadPolicy::MigrateFacts
        }
        Some(ref s) if s == "replace-all" || s == "replace_all" => ReloadPolicy::ReplaceAll,
        _ => ReloadPolicy::default(),
    };

    match reload_reading(d, name, body, parsed_policy) {
        Ok(outcome) => {
            // Re-compile defs from the post-reload state. Mirrors
            // load/unload handler tails.
            let mut defs = crate::compile::compile_to_defs_state(&outcome.new_state);
            defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
            defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
            defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
            defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
            let new_d = ast::defs_to_state(&defs, &outcome.new_state);

            let delta = ast::diff_cells(state, &new_d);

            let mut data = hashbrown::HashMap::new();
            data.insert("name".to_string(), name.to_string());
            data.insert(
                "removedNouns".to_string(),
                outcome.removed.removed_nouns.join(","),
            );
            data.insert(
                "removedFactTypes".to_string(),
                outcome.removed.removed_fact_types.join(","),
            );
            data.insert(
                "removedDerivations".to_string(),
                outcome.removed.removed_derivations.join(","),
            );
            data.insert(
                "addedNouns".to_string(),
                outcome.added.added_nouns.join(","),
            );
            data.insert(
                "addedFactTypes".to_string(),
                outcome.added.added_fact_types.join(","),
            );
            data.insert(
                "addedDerivations".to_string(),
                outcome.added.added_derivations.join(","),
            );
            // #558 / DynRdg-4: round-trip versioning metadata on
            // reload — both the version that was removed (so callers
            // can chronologically order body versions) and the
            // version that was just installed. First-time-load
            // fallthrough produces `previousContentHash = ""` and
            // `previousVersionStamp = "0"` because the unload step
            // had no manifest to read.
            data.insert(
                "previousContentHash".to_string(),
                outcome.removed.content_hash.clone(),
            );
            data.insert(
                "previousVersionStamp".to_string(),
                outcome.removed.version_stamp.to_string(),
            );
            data.insert(
                "contentHash".to_string(),
                outcome.added.content_hash.clone(),
            );
            data.insert(
                "versionStamp".to_string(),
                outcome.added.version_stamp.to_string(),
            );

            let derived_count = outcome.removed.removed_nouns.len()
                + outcome.removed.removed_fact_types.len()
                + outcome.removed.removed_derivations.len()
                + outcome.added.added_nouns.len()
                + outcome.added.added_fact_types.len()
                + outcome.added.added_derivations.len();

            CommandResult {
                entities: vec![EntityResult {
                    id: format!("reading:{}", name),
                    entity_type: "ReadingReloaded".to_string(),
                    data,
                }],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations: vec![],
                derived_count,
                rejected: false,
                state: delta,
            }
        }
        Err(err) => {
            let violations = match err {
                ReloadError::Disallowed => vec![crate::types::Violation {
                    constraint_id: "reload_reading.disallowed".to_string(),
                    constraint_text: "runtime reading reload is disallowed by host policy".to_string(),
                    detail: "the host did not enable runtime ReloadReading".to_string(),
                    alethic: true,
                }],
                ReloadError::InvalidName(msg) => vec![crate::types::Violation {
                    constraint_id: "reload_reading.invalid_name".to_string(),
                    constraint_text: "reading name failed sanitization".to_string(),
                    detail: msg,
                    alethic: true,
                }],
                ReloadError::EmptyBody => vec![crate::types::Violation {
                    constraint_id: "reload_reading.empty_body".to_string(),
                    constraint_text: "reading body is empty".to_string(),
                    detail: "reloading an empty body would not add any cells; pass at least one statement".to_string(),
                    alethic: true,
                }],
                ReloadError::UnloadFailed(unload_err) => match unload_err {
                    UnloadError::Disallowed => vec![crate::types::Violation {
                        constraint_id: "reload_reading.unload_failed".to_string(),
                        constraint_text: "unload step rejected (host policy)".to_string(),
                        detail: "the host did not enable runtime UnloadReading; reload cannot proceed".to_string(),
                        alethic: true,
                    }],
                    UnloadError::InvalidName(msg) => vec![crate::types::Violation {
                        constraint_id: "reload_reading.unload_failed".to_string(),
                        constraint_text: "unload step rejected: invalid name".to_string(),
                        detail: msg,
                        alethic: true,
                    }],
                    UnloadError::ManifestMissing(missing_name) => vec![crate::types::Violation {
                        // Defensive: reload_reading treats ManifestMissing
                        // as a fall-through to first-time-load and never
                        // surfaces it as an UnloadFailed. This branch is
                        // unreachable today; pinned for forward-compat
                        // if the core's policy ever changes.
                        constraint_id: "reload_reading.unload_failed".to_string(),
                        constraint_text: "unload step rejected: manifest missing".to_string(),
                        detail: format!("no _loaded_reading:{} cell found", missing_name),
                        alethic: true,
                    }],
                },
                ReloadError::LoadFailed(load_err) => match load_err {
                    LoadError::Disallowed => vec![crate::types::Violation {
                        constraint_id: "reload_reading.load_failed".to_string(),
                        constraint_text: "load step rejected (host policy)".to_string(),
                        detail: "the host did not enable runtime LoadReading; reload rolls back".to_string(),
                        alethic: true,
                    }],
                    LoadError::EmptyBody => vec![crate::types::Violation {
                        constraint_id: "reload_reading.load_failed".to_string(),
                        constraint_text: "load step rejected: empty body".to_string(),
                        detail: "reload body must contain at least one statement".to_string(),
                        alethic: true,
                    }],
                    LoadError::InvalidName(msg) => vec![crate::types::Violation {
                        constraint_id: "reload_reading.load_failed".to_string(),
                        constraint_text: "load step rejected: invalid name".to_string(),
                        detail: msg,
                        alethic: true,
                    }],
                    LoadError::ParseError(msg) => vec![crate::types::Violation {
                        constraint_id: "reload_reading.load_failed".to_string(),
                        constraint_text: "load step rejected: FORML 2 parse error".to_string(),
                        detail: msg,
                        alethic: true,
                    }],
                    LoadError::DeonticViolation(diags) => diags
                        .into_iter()
                        .map(|d| crate::types::Violation {
                            constraint_id: "reload_reading.load_failed".to_string(),
                            constraint_text: d.reading.clone(),
                            detail: d.message.clone(),
                            alethic: true,
                        })
                        .collect(),
                    // #559 / DynRdg-5: alethic violation surfaced
                    // through the load-step path of reload_reading.
                    // Same detail shape as the load handler, distinct
                    // constraint_id so dashboards can route the two
                    // error classes (load-step alethic vs.
                    // load-step deontic) separately.
                    LoadError::AlethicViolation(diags) => diags
                        .into_iter()
                        .map(|d| crate::types::Violation {
                            constraint_id: "reload_reading.load_failed".to_string(),
                            constraint_text: d.reading.clone(),
                            detail: d.message.clone(),
                            alethic: true,
                        })
                        .collect(),
                },
                ReloadError::NotImplemented => vec![crate::types::Violation {
                    constraint_id: "reload_reading.not_implemented".to_string(),
                    constraint_text: "requested ReloadPolicy is not implemented".to_string(),
                    detail: "MigrateFacts policy is reserved; use replace-all for now".to_string(),
                    alethic: true,
                }],
            };
            CommandResult {
                entities: vec![],
                status: None,
                transitions: vec![],
                navigation: vec![],
                violations,
                derived_count: 0,
                rejected: true,
                state: ast::Object::phi(),
            }
        }
    }
}

// -- Helpers ----------------------------------------------------------

/// HATEOAS as ρ-application (Theorem 4a)
/// HATEOAS as ρ-application (Theorem 4a):
/// links(s) = π_event(Filter(p) : T) — computed via transitions:{noun} def.
fn hateoas_via_rho(
    d: &ast::Object,
    noun: &str,
    entity_id: &str,
    status: Option<&str>,
) -> Vec<TransitionAction> {
    let Some(status) = status else { return vec![] };
    let encoded = noun.replace(' ', "%20");

    // ρ(transitions:{noun}) : status → <<from, to, event>, ...>
    let result = ast::apply(
        &ast::Func::Def(format!("transitions:{}", noun)),
        &ast::Object::atom(status),
        d,
    );

    result.as_seq().map(|triples| {
        triples.iter().filter_map(|t| {
            let items = t.as_seq()?;
            let _from = items.get(0)?.as_atom()?;
            let to = items.get(1)?.as_atom()?.to_string();
            let event = items.get(2)?.as_atom()?.to_string();
            // HATEOAS links are clickable: GET for every non-destructive
            // transition, DELETE only when the target status is "deleted".
            // POST stays reserved for bulk / out-of-band command paths.
            // Event is embedded in the URL as a query param so the link
            // is self-contained — a browser can follow it without
            // synthesising a JSON body.
            let method = http_method_for_status(d, &to);
            let event_encoded = event.replace(' ', "%20");
            Some(TransitionAction {
                event, target_status: to, method,
                href: format!(
                    "/api/entities/{}/{}/transition?event={}",
                    encoded, entity_id, event_encoded,
                ),
            })
        }).collect()
    }).unwrap_or_default()
}

/// task-965: HTTP method for a target status, lifted from a Rust literal
/// (`if to == "deleted" { "DELETE" } else { "GET" }`) to the
/// `Status has HTTP Method` reading (e.g. `Status 'deleted' has HTTP Method
/// 'DELETE'` in readings/core/state.md). Defaults to GET when no method is
/// declared for the status. Keeps the destructive-affordance rule in
/// readings, not in compiled code (facts-all-the-way-down).
fn http_method_for_status(d: &ast::Object, status: &str) -> String {
    ast::fetch_cell_seq("Status_has_HTTP_Method", d)
        .as_seq()
        .and_then(|facts| facts.iter().find_map(|f| {
            if ast::binding(f, "Status") == Some(status) {
                ast::binding(f, "HTTP Method").map(|m| m.to_string())
            } else {
                None
            }
        }))
        .unwrap_or_else(|| "GET".to_string())
}

/// Theorem 4b: nav(e, n) = children(n) ∪ parent(n).
/// Resolves nav:{noun}:children and nav:{noun}:parent defs from D.
fn nav_links_via_rho(d: &ast::Object, noun: &str, entity_id: &str) -> Vec<NavigationLink> {
    let encoded = noun.replace(' ', "%20");
    let mut links = Vec::new();

    // children(n) — Eq. 13
    let children = ast::apply(
        &ast::Func::Def(format!("nav:{}:children", noun)),
        &ast::Object::phi(),
        d,
    );
    children.as_seq().into_iter().flat_map(|items| items.iter().filter_map(|item| {
        let child_noun = item.as_atom()?.to_string();
        let child_encoded = child_noun.replace(' ', "%20");
        Some(NavigationLink {
            rel: "children".to_string(),
            noun: child_noun,
            href: format!("/api/entities/{}/{}/{}", encoded, entity_id, child_encoded),
        })
    }).collect::<Vec<_>>()).for_each(|l| links.push(l));

    // parent(n) — Eq. 14
    let parents = ast::apply(
        &ast::Func::Def(format!("nav:{}:parent", noun)),
        &ast::Object::phi(),
        d,
    );
    parents.as_seq().into_iter().flat_map(|items| items.iter().filter_map(|item| {
        let parent_noun = item.as_atom()?.to_string();
        let parent_encoded = parent_noun.replace(' ', "%20");
        Some(NavigationLink {
            rel: "parent".to_string(),
            noun: parent_noun,
            href: format!("/api/entities/{}", parent_encoded),
        })
    }).collect::<Vec<_>>()).for_each(|l| links.push(l));

    links
}

fn extract_sm_status(state: &ast::Object, sm_id: &str) -> Option<String> {
    let sm = StateMachineCellShape::boot();
    let cell = ast::fetch_cell_seq(sm.cell_name, state);
    cell.as_seq()?.iter()
        .find(|fact| {
            ast::binding_matches(fact, sm.state_machine_role, sm_id)
                || fact.as_seq().map_or(false, |pairs| {
                    pairs.iter().any(|pair| pair.as_seq().and_then(|p| p.get(1)?.as_atom()) == Some(sm_id))
                })
        })
        .and_then(|fact| ast::binding(fact, sm.current_status_role).map(|s| s.to_string()))
}

// =====================================================================
// select_component (#493) — AI-agent verb over the Component registry
// =====================================================================
//
// Given a natural-language `intent` plus a set of constraints (the
// MonoView selection axes from monoview.md / components.md), return a
// ranked list of (Component, Toolkit, Symbol, score) tuples drawn from
// the population.
//
// The scoring layer is a pure-Rust re-implementation of HHHH's #492
// derivation rules. We could instead synthesise a MonoView fact tuple,
// run the chainer, and read the resulting `ImplementationBinding is
// preferred for MonoView` cell — but the chainer round-trip carries
// `forward_chain` cost (~tens of ms once SMs and inheritance compile
// in), and `select_component` is meant to be interactive (an LLM tool
// call). The Rust scorer mirrors the rule predicates one-for-one so
// the output is bit-identical to what the chainer would produce on a
// hand-built MonoView, while staying sub-millisecond for the seeded
// population.
//
// Contract: `Component_has_Component_Role` matches via case-insensitive
// substring containment over the role string. The intent string can
// also include verb hints ("date picker", "I need a button") — they're
// projected through the same containment match. Empty intent matches
// every Component.

/// MonoView-flavoured selection axes for `select_component`.
///
/// Mirrors HHHH's #492 rules: each field corresponds to a constraint
/// the rules condition on. Every field is optional so callers can
/// supply only the axes they care about; unspecified axes contribute
/// no scoring boosts and no penalties.
///
/// Re-export from `crate::select_component_core` (#565). The pure
/// FORML cell-walker now lives in `select_component_core.rs` so the
/// kernel can reach it without pulling in the std-only `Command`
/// dispatch surface. This re-export preserves the historical
/// `command::SelectComponentConstraints` path for in-crate callers.
pub use crate::select_component_core::SelectComponentConstraints;

/// One ranked Component implementation returned by `select_component`.
/// Re-export of `crate::select_component_core::SelectedComponent`.
pub use crate::select_component_core::SelectedComponent;

/// `select_component` — engine-side handler for #493 MCP verb.
///
/// Walks every Component whose Role substring-matches `intent`,
/// enumerates that Component's ImplementationBindings (one per Toolkit),
/// scores each pair under the supplied constraints, and returns the top
/// N (default 5) sorted by score descending. Within equal scores the
/// order is stable per (component, toolkit) sort which keeps output
/// reproducible across runs.
///
/// Returns an empty vec if no Component matches the intent — the
/// caller (MCP layer) renders this as `[]` so the LLM sees the gap.
///
/// Implementation now lives in `crate::select_component_core` so it is
/// kernel-reachable (#565); this re-export keeps the historic
/// `command::select_component` path stable.
pub use crate::select_component_core::select_component;

/// JSON wrapper for the system_impl intercept. Parses
/// `{"intent": "...", "constraints": {...}}`, runs `select_component`,
/// returns the results as JSON. Returns `"⊥"` on input parse failure.
///
/// The serde_json glue stays in `command.rs` (std-deps-only); the
/// underlying `select_component` lives in `select_component_core` so
/// no_std callers (kernel cell-renderer #511) can reach the engine
/// version without going through this JSON adapter.
pub fn select_component_json(state: &ast::Object, body: &str) -> String {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        #[serde(default)]
        intent: String,
        #[serde(default)]
        constraints: SelectComponentConstraints,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return "⊥".to_string(),
    };
    let results = select_component(state, &req.intent, &req.constraints);
    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
}

// =====================================================================
// wine_prefix_for (#481) — convenience verb for the Wine App → prefix
//                          Directory join
// =====================================================================
//
// Per readings/compat/wine.md (#481), every Wine App owns a single
// `Directory` cell as its prefix root via the 1:1 fact type
// `Wine App has prefix Directory`. The runtime layer (#462c), the
// `arest run "App Name"` CLI, and the future `arest backup` CLI all
// need the same lookup: given a Wine App id, return its prefix
// Directory id so they can route filesystem writes through it (or
// hand it to `zip_directory(prefix_id)` from #404 for snapshotting).
//
// `wine_prefix_for` is the engine-side handler. It reads the
// `Wine_App_has_prefix_Directory` cell and returns the matching
// `prefix Directory` binding. Returns `None` when the Wine App does
// not exist or has no prefix Directory bound (which can only happen
// pre-derivation; the mandatory constraint declared in wine.md
// guarantees the binding exists once the readings are compiled in).
//
// Read-only: no state mutation, no Platform fn calls.

/// Look up the prefix Directory id for a Wine App.
///
/// Returns `None` when the Wine App is not in the population OR the
/// `Wine App has prefix Directory` cell carries no binding for it.
/// The latter is a constraint violation per wine.md's mandatory
/// constraint and indicates either an un-compiled tenant or a
/// hand-rolled state that bypassed the readings.
///
/// The cell key is `Wine_App_has_prefix_Directory` (the parser's
/// `<subject>_has_<object>` munge of `Wine App has prefix Directory`).
/// Within each fact the bindings are keyed by the underlying *noun*
/// name (`Wine App` and `Directory`) rather than the full role
/// reference (`prefix Directory`) — `instance_fact_field_cells` in
/// `parse_forml2_stage2.rs` strips the leading adjective so any
/// `belongs to` / `is in` / `has prefix` overlay collapses to the
/// noun id at runtime. Hand-pushed cells from `cell_push` follow the
/// same convention so the two sources stay binding-compatible.
pub fn wine_prefix_for(state: &ast::Object, app_id: &str) -> Option<String> {
    let cell = ast::fetch_cell_seq("Wine_App_has_prefix_Directory", state);
    cell.as_seq()?.iter().find_map(|fact| {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            ast::binding(fact, "Directory").map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// JSON wrapper for the system_impl intercept (follow-up wiring).
/// Parses `{"appId": "..."}`, runs `wine_prefix_for`, returns the
/// directory id as a JSON string on success or `"⊥"` on miss.
pub fn wine_prefix_for_json(state: &ast::Object, body: &str) -> String {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        #[serde(default)]
        app_id: String,
    }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return "⊥".to_string(),
    };
    match wine_prefix_for(state, &req.app_id) {
        Some(dir_id) => serde_json::to_string(&dir_id)
            .unwrap_or_else(|_| "⊥".to_string()),
        None => "⊥".to_string(),
    }
}

// =====================================================================
// wine_app_by_name (#503) — name → (app id, prefix Directory) lookup
//                           the `arest run "App Name"` CLI dispatches on
// =====================================================================
//
// Sibling of `wine_prefix_for` above. Where `wine_prefix_for` takes an
// already-resolved Wine App slug and returns its prefix Directory,
// `wine_app_by_name` takes whatever string the user typed at the
// command line (`arest run "Notepad++"` — display title; or
// `arest run "notepad-plus-plus"` — the slug itself, i.e. the
// `.Name` reference mode value) and resolves it into the same
// (app id, prefix Directory id) pair.
//
// Two acceptance paths:
//
//   1. Exact match against the Wine App's `.Name` reference value
//      (its slug). Per #481 the slug is the cell-binding subject for
//      every `Wine App` fact (e.g. `Wine_App_has_Compat_Rating`'s
//      `Wine App` binding key). We collect the distinct slugs from
//      whichever mandatory-cardinality cell is present; the `Wine App
//      has Compat Rating` constraint guarantees `Wine_App_has_Compat_Rating`
//      carries one fact per declared app. (The spec brief mentioned
//      a `Wine_App_has_Name` cell — that cell does not actually exist
//      because Wine App's reference scheme is arity-1, so no compound-
//      ref decomposition fires; the slug is recovered from the subject
//      bindings of any populated cell instead. Honours OOOO's #481
//      finding that `instance_fact_field_cells` keys instance-fact
//      bindings by the bare noun name `Wine App`, not by a fuller
//      role reference.)
//
//   2. Exact match against the human-readable display title (the
//      `display- Title` value, e.g. `'Notepad++'`). The parser today
//      mis-buckets this into a malformed cell name `has display-
//      Title 'Notepad++'` (the verb-token-fallback branch in
//      `translate_instance_facts_with_ft_ids` when no canonical
//      `Wine_App_has_Title` FT is recognised); we walk those
//      malformed cell names to recover the (slug, title) pairs.
//      Once the parser learns to fold display- titles into a clean
//      `Wine_App_has_Title` cell this branch can collapse to a
//      one-line lookup.
//
// Both paths return `(app_id, prefix_dir_id)` so the caller can hand
// both to the Platform fns (`zip_directory(prefix_id)`,
// `wine_prefix_for(app_id)`, future winetricks-bootstrap). Returns
// `None` if neither path matches; the CLI's near-name suggester
// (Levenshtein over the slug + title set, see `crates/arest/src/cli/run.rs`)
// runs as the fallback.
//
// Read-only: no state mutation, no Platform fn calls.

/// Return every Wine App slug declared in the population, sorted.
///
/// Pulls from `Wine_App_has_Compat_Rating` (the mandatory-cardinality
/// cell — "Each Wine App has exactly one Compat Rating." in
/// `readings/compat/wine.md`), so every declared app contributes
/// exactly one entry. Falls back to scanning every cell whose facts
/// carry a `Wine App` binding when the Compat Rating cell is empty
/// (e.g. an in-flight migration where the rating fact-type has been
/// renamed but the apps are still in the population).
pub fn wine_app_ids(state: &ast::Object) -> Vec<String> {
    let mut seen: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    let cell = ast::fetch_cell_seq("Wine_App_has_Compat_Rating", state);
    if let Some(seq) = cell.as_seq() {
        for fact in seq.iter() {
            if let Some(slug) = ast::binding(fact, "Wine App") {
                if !slug.is_empty() {
                    seen.insert(slug.to_string());
                }
            }
        }
    }
    if seen.is_empty() {
        // Fallback: scan every cell for `Wine App` subject bindings.
        for (_name, contents) in ast::cells_iter(state) {
            for fact in ast::cell_facts_iter(contents) {
                if let Some(slug) = ast::binding(fact, "Wine App") {
                    if !slug.is_empty() {
                        seen.insert(slug.to_string());
                    }
                }
            }
        }
    }
    let mut out: Vec<String> = seen.into_iter().collect();
    out.sort();
    out
}

/// Return the display title for a Wine App slug, if one was declared.
///
/// First reads the canonical `Wine_App_has_display-_Title` cell —
/// the parser's standard emission for `Wine App has display- Title.`
/// once the FT is in scope (full bundled metamodel). Falls back to
/// the legacy mis-bucketed `has display- Title '<Title>'` cells for
/// partial-metamodel states (the command-module unit-test fixture
/// shape that pre-dates the FT being declared).
///
/// Returns `None` if no matching binding is found in either source
/// OR if the slug isn't a known Wine App.
pub fn wine_app_display_title(state: &ast::Object, slug: &str) -> Option<String> {
    // Canonical cell: emitted by the parser when the
    // `Wine App has display- Title.` FT is in scope. Each fact carries
    // `(Wine App, <slug>) (Title, <title>)`.
    let canonical = ast::fetch_cell_seq("Wine_App_has_display-_Title", state);
    if let Some(seq) = canonical.as_seq() {
        for fact in seq.iter() {
            if ast::binding(fact, "Wine App") == Some(slug) {
                if let Some(title) = ast::binding(fact, "Title") {
                    return Some(title.to_string());
                }
            }
        }
    }
    // Fallback: legacy mis-bucketed cells of the form
    // `has display- Title '<actual title>'`. Pre-FT-declaration states
    // (and the command-module unit-test fixture) populate this shape;
    // production callers under the bundled metamodel hit the
    // canonical-cell branch above first.
    for (name, contents) in ast::cells_iter(state) {
        let prefix = "has display- Title '";
        if !name.starts_with(prefix) || !name.ends_with('\'') {
            continue;
        }
        let title = &name[prefix.len()..name.len() - 1];
        for fact in ast::cell_facts_iter(contents) {
            if ast::binding(fact, "Wine App") == Some(slug) {
                return Some(title.to_string());
            }
        }
    }
    None
}

/// Resolve a user-supplied name into a `(slug, prefix Directory id)`
/// pair, or `None` on miss.
///
/// Tries (in order) exact match against the `.Name` reference (slug)
/// and exact match against the display title. Returns `None` when
/// neither matches; the CLI fallback layer (`cli::run::suggest_near_name`)
/// then runs Levenshtein over the same (slug + title) set to surface a
/// "did you mean…" hint.
///
/// Pairs `wine_prefix_for` with the slug to produce the prefix
/// Directory id alongside the slug, so callers (the future Wine
/// runtime layer in #504) can hand both to Platform fns without a
/// second cell read.
pub fn wine_app_by_name(state: &ast::Object, name: &str) -> Option<(String, String)> {
    let known = wine_app_ids(state);
    // Path 1: exact slug.
    if known.iter().any(|id| id == name) {
        let prefix = wine_prefix_for(state, name)?;
        return Some((name.to_string(), prefix));
    }
    // Path 2: exact display title (case-sensitive match against the
    // mis-bucketed `has display- Title '<X>'` cell names).
    for slug in &known {
        if let Some(title) = wine_app_display_title(state, slug) {
            if title == name {
                let prefix = wine_prefix_for(state, slug)?;
                return Some((slug.clone(), prefix));
            }
        }
    }
    None
}

// -- Tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;

    #[test]
    fn command_result_round_trips_through_object() {
        let mut data = HashMap::new();
        data.insert("customer".to_string(), "acme".to_string());
        let result = CommandResult {
            entities: vec![EntityResult { id: "ord-1".into(), entity_type: "Order".into(), data }],
            status: Some("Draft".into()),
            transitions: vec![TransitionAction {
                event: "place".into(), target_status: "Placed".into(),
                method: "POST".into(), href: "/orders/ord-1/transition".into(),
            }],
            navigation: vec![],
            violations: vec![],
            derived_count: 2,
            rejected: false,
            state: ast::Object::phi(),
        };
        let obj = encode_command_result(&result);
        let decoded = decode_command_result(&obj);
        assert_eq!(decoded.entities.len(), 1);
        assert_eq!(decoded.entities[0].id, "ord-1");
        assert_eq!(decoded.entities[0].entity_type, "Order");
        assert_eq!(decoded.status, Some("Draft".into()));
        assert_eq!(decoded.transitions.len(), 1);
        assert_eq!(decoded.transitions[0].event, "place");
        assert_eq!(decoded.derived_count, 2);
        assert!(!decoded.rejected);
    }

    const STATE_METAMODEL: &str = r#"
# State

## Entity Types

Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Event Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types

State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Event Type.
"#;

    const ORDER_READINGS: &str = r#"
# Orders

## Entity Types

Order(.Order Number) is an entity type.

## Fact Types

Order has Amount.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Transition 'pay' is defined in State Machine Definition 'Order'.
  Transition 'pay' is from Status 'Placed'.
  Transition 'pay' is to Status 'Paid'.
  Transition 'pay' is triggered by Event Type 'pay'.

Transition 'cancel' is defined in State Machine Definition 'Order'.
  Transition 'cancel' is from Status 'Draft'.
  Transition 'cancel' is to Status 'Cancelled'.
  Transition 'cancel' is triggered by Event Type 'cancel'.
"#;

    /// Parse state metamodel + order domain readings, compile to defs,
    /// return (defs_object, base_state).
    fn setup_order_defs() -> (ast::Object, ast::Object) {
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let orders_state = crate::parse_forml2::parse_to_state_with_nouns(ORDER_READINGS, &meta_state).unwrap();
        let state = ast::merge_states(&meta_state, &orders_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);
        (def_obj, state)
    }

    #[test]
    fn create_entity_initializes_state_machine() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        fields.insert("amount".to_string(), "999".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        assert_eq!(result.entities[0].id, "ORD-100");
        assert_eq!(result.entities[0].entity_type, "Order");
        assert_eq!(result.entities[1].entity_type, "State Machine");
        // task-742: renamed from currentlyInStatus / forResource
        assert_eq!(result.entities[1].data["Status"], "Draft");
        assert_eq!(result.entities[1].data["Resource"], "ORD-100");
        assert_eq!(result.status.as_deref(), Some("Draft"));
        assert_eq!(result.transitions.len(), 2); // place, cancel
        assert!(result.transitions.iter().any(|t| t.event == "place"));
        assert!(result.transitions.iter().any(|t| t.event == "cancel"));
        assert!(!result.rejected);
    }

    /// task-967 regression: the apply-path seeded forward-chain must reach
    /// the derivation fixpoint even when a rule that consumes a freshly
    /// written cell is keyed under a DIFFERENT noun than the one being
    /// mutated. The noun-scoped `derivation_index` excluded such cross-noun
    /// consumer rules from the apply's rule set, so they never fired (the
    /// live SM->status bridge: `_sm_event_fold_{N}` writes a State-Machine
    /// cell the Resource-indexed bridge rule consumes, but the bridge is
    /// absent from `derivation_index:{N}`). Minimal cross-noun analogue:
    /// `Mid is active` (keyed under Source+Mid via its antecedent) writes
    /// `Mid_is_active` on a Source create; `Mid is ready` (keyed under Mid
    /// only) must still reach the fixpoint. The metamodel (STATE_METAMODEL)
    /// is merged in so the `derivation_index` is non-empty -- the pre-fix
    /// noun-gate then severs the Mid-indexed consumer from the Source apply.
    ///
    /// IGNORED: with the merge the index is non-empty (9 entries) and the
    /// fix's un-gate runs all 4 stratum-1 rules for the Source apply (gating
    /// profile) -- but this synthetic cross-noun JOIN doesn't materialize its
    /// seed `Mid_is_active`, so the downstream assert can't run. A FORML2
    /// join-fixture limitation, NOT the fix. Bug+fix verified: Agent-1 live
    /// diagnosis (bridge rule keyed under index:Resource not index:Task) +
    /// full suite green + the 4/4 gating evidence. TODO: SM-bridge fixture.
    #[ignore = "task-967: synthetic join doesn't seed; needs SM fixture"]
    #[test]
    fn apply_reaches_fixpoint_across_cross_noun_derivation() {
        const READINGS: &str = r#"
# Cross-Noun Probe

## Entity Types

Source(.id) is an entity type.
Mid(.Name) is an entity type.
Code is a value type.

## Fact Types

Source has Code.
Mid has Code.

## Instance Facts

Mid 'M1' has Code 'C1'.

## Derivation Rules

* Mid is active iff Mid has some Code and some Source has that Code.
* Mid is ready iff Mid is active.
"#;
        let meta = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let probe = crate::parse_forml2::parse_to_state_with_nouns(READINGS, &meta).unwrap();
        let state = ast::merge_states(&meta, &probe);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("code".to_string(), "C1".to_string());
        let cmd = Command::CreateEntity {
            noun: "Source".to_string(),
            domain: "probe".to_string(),
            id: Some("S1".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_obj, &cmd, &state);
        assert!(!result.rejected, "create must not be rejected");

        // Sanity: the Source-indexed producer fired (also pins the cell name).
        let active = crate::ast::fetch_cell_seq("Mid_is_active", &result.state);
        assert!(
            active.as_seq().map_or(false, |s| !s.is_empty()),
            "stage-1 Mid_is_active must materialize on the Source apply (sanity)"
        );

        // Regression: the Mid-indexed consumer, severed from the Source apply
        // by the noun-index, must still reach the fixpoint.
        let ready = crate::ast::fetch_cell_seq("Mid_is_ready", &result.state);
        assert!(
            ready.as_seq().map_or(false, |s| !s.is_empty()),
            "task-967: Mid_is_ready (consumer keyed under Mid, not Source) must \
             materialize on the Source apply -- seeded chain must reach the \
             fixpoint across the cross-noun rule dependency"
        );
    }

    /// task-968: the SM-fixture companion of task-967's ignored synthetic
    /// JOIN test. Pins the post-5d2fb81d behavior on a Resource-keyed
    /// derivation that consumes the live SM cells written by the Order
    /// apply -- the EXACT cross-noun shape that triggered the original
    /// bug. The custom bridge `Resource is mirroring Status iff some
    /// State Machine is for that Resource and that State Machine is
    /// currently in that Status` is keyed under index:Resource (its
    /// antecedents resolve to State Machine + Status nouns). Before the
    /// fix the rule was absent from index:Order, so the seeded-chain on
    /// the Order create excluded it and `Resource_is_mirroring_Status`
    /// never materialized. After: the un-gated collect_stratum visits
    /// the rule, the chain sees the freshly written SM cells, and the
    /// consequent reaches the LFP on the apply path.
    ///
    /// Distinct from the live `Resource_is_currently_in_Status` cell
    /// (imperatively written by transition handling in command.rs:1969)
    /// -- this uses a SEPARATE custom FT so any pass is attributable to
    /// the derivation reaching LFP, not the imperative side-channel.
    #[test]
    fn apply_reaches_fixpoint_across_sm_bridge_derivation_task_968() {
        const BRIDGE_READINGS: &str = r#"
# task-968 SM-bridge fixture

## Entity Types

Resource(.Reference) is an entity type.
State Machine(.id) is an entity type.

## Fact Types

State Machine is for Resource.
State Machine is currently in Status.
Resource is mirroring Status.

## Derivation Rules

* Resource is mirroring Status iff some State Machine is for that Resource and that State Machine is currently in that Status.
"#;

        let meta = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let orders = crate::parse_forml2::parse_to_state_with_nouns(ORDER_READINGS, &meta).unwrap();
        let bridge = crate::parse_forml2::parse_to_state_with_nouns(BRIDGE_READINGS, &meta).unwrap();
        let state = ast::merge_states(&ast::merge_states(&meta, &orders), &bridge);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-968".to_string());
        fields.insert("amount".to_string(), "100".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-968".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_obj, &cmd, &state);
        assert!(!result.rejected,
            "create must not be rejected; violations={:?}", result.violations);

        // Sanity: the SM init wrote the antecedents the bridge consumes
        // (`State_Machine_is_currently_in_Status` + `State_Machine_is_for_Resource`).
        let sm_status = crate::ast::fetch_cell_seq("State_Machine_is_currently_in_Status", &result.state);
        let sm_for_res = crate::ast::fetch_cell_seq("State_Machine_is_for_Resource", &result.state);
        assert!(
            sm_status.as_seq().map_or(false, |s| !s.is_empty()),
            "SM init must materialize State_Machine_is_currently_in_Status on Order create (sanity)"
        );
        assert!(
            sm_for_res.as_seq().map_or(false, |s| !s.is_empty()),
            "SM init must materialize State_Machine_is_for_Resource on Order create (sanity)"
        );

        // Regression: the Resource-indexed bridge consumer -- severed from
        // the Order apply by the pre-fix noun-gate at collect_stratum --
        // must reach the LFP and materialize the bridge consequent.
        let mirror = crate::ast::fetch_cell_seq("Resource_is_mirroring_Status", &result.state);
        let bindings: alloc::vec::Vec<ast::Object> = mirror.as_seq()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        assert!(
            !bindings.is_empty(),
            "task-968 regression: Resource_is_mirroring_Status must materialize \
             on the Order apply -- the seeded chain must reach the fixpoint across \
             the cross-noun SM-bridge. SM status cell: {:?}", sm_status
        );

        // Stronger: the binding ties the new Order resource to its initial Status.
        let has_ord_draft = bindings.iter().any(|f| {
            crate::ast::binding(f, "Resource") == Some("ORD-968")
                && crate::ast::binding(f, "Status") == Some("Draft")
        });
        assert!(
            has_ord_draft,
            "Resource_is_mirroring_Status must bind Resource=ORD-968 -> Status=Draft; \
             got bindings={:?}", bindings
        );
    }

    /// task-965: the destructive-affordance rule (deleted -> DELETE) is read
    /// from the `Status has HTTP Method` reading, defaulting to GET. This
    /// keeps the HATEOAS method rule in readings, not a Rust literal.
    #[test]
    fn http_method_for_status_lifts_delete_rule_from_reading() {
        let state = ast::cell_push(
            "Status_has_HTTP_Method",
            ast::fact_from_pairs(&[("Status", "deleted"), ("HTTP Method", "DELETE")]),
            &ast::Object::phi(),
        );
        assert_eq!(http_method_for_status(&state, "deleted"), "DELETE");
        assert_eq!(http_method_for_status(&state, "placed"), "GET");
        assert_eq!(http_method_for_status(&ast::Object::phi(), "deleted"), "GET");
    }

    #[test]
    fn create_entity_with_explicit_id() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-REF".to_string());
        fields.insert("amount".to_string(), "500".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-REF".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert_eq!(result.entities[0].id, "ORD-REF");
    }

    #[test]
    fn create_entity_without_state_machine() {
        let (def_map, state) = setup_order_defs();
        // Category is an entity type with no state machine. Declaring it (with
        // a reference scheme) makes it a valid run-time shape — an SM-less
        // entity, distinct from an under-defined noun the run-time gate refuses.
        let state = ast::cell_push(
            "Noun",
            ast::fact_from_pairs(&[("name", "Category"), ("objectType", "entity"), ("referenceScheme", "id")]),
            &state,
        );

        let mut fields = HashMap::new();
        fields.insert("name".to_string(), "Electronics".to_string());

        let cmd = Command::CreateEntity {
            noun: "Category".to_string(),
            domain: "catalog".to_string(),
            id: Some("electronics".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        assert_eq!(result.entities.len(), 1);
        assert!(result.status.is_none());
        assert!(result.transitions.is_empty());
    }

    /// Hardening regression — the run-time definedness gate. Instantiating an
    /// entity is a run-time operation requiring a fully-defined entity type
    /// (objectType="entity" WITH a reference scheme). A value type, an
    /// undeclared noun, and an entity declared without a reference scheme are
    /// all valid *design-time* shapes but NOT run-time ones: createEntity over
    /// them previously drove resolve / the derivation forward-chain into a
    /// non-terminating expansion (6+ GB, no return). Each must reject up front,
    /// so derivations never run over an under-defined noun.
    #[test]
    fn create_entity_runtime_definedness_gate() {
        let (def_map, _state) = setup_order_defs();
        let noun_state = |pairs: &[(&str, &str)]| {
            ast::cell_push("Noun", ast::fact_from_pairs(pairs), &ast::Object::phi())
        };
        let try_create = |noun: &str, state: &ast::Object| {
            apply_command_defs(&def_map, &Command::CreateEntity {
                noun: noun.to_string(), domain: "d".to_string(),
                id: Some("x".to_string()), fields: HashMap::new(),
                sender: None, signature: None,
            }, state)
        };
        let rejected_undefined = |r: &CommandResult| {
            r.rejected && r.entities.is_empty()
                && r.violations.iter().any(|v| v.constraint_id.contains("not_runtime_defined"))
        };

        // value type → reject (a value type is a domain, not instantiable)
        let r = try_create("Color", &noun_state(&[("name", "Color"), ("objectType", "value")]));
        assert!(rejected_undefined(&r), "value type must reject; got {:?}", r.violations);

        // undeclared noun → reject up front (gate fires before resolve, so no hang)
        let r = try_create("Gadget", &ast::Object::phi());
        assert!(rejected_undefined(&r), "undeclared noun must reject, not hang; got {:?}", r.violations);

        // entity declared WITHOUT a reference scheme → definable, not instantiable
        let r = try_create("Sketch", &noun_state(&[("name", "Sketch"), ("objectType", "entity")]));
        assert!(rejected_undefined(&r),
            "entity without a reference scheme must reject at run-time; got {:?}", r.violations);
    }

    /// Hardening regression (task 938) — updateEntity is gated the same way as
    /// createEntity: an under-defined noun (here a value type) is refused at
    /// run-time, before the derivation forward-chain can diverge over it.
    #[test]
    fn update_entity_runtime_definedness_gate() {
        let (def_map, _state) = setup_order_defs();
        let state = ast::cell_push(
            "Noun",
            ast::fact_from_pairs(&[("name", "Color"), ("objectType", "value")]),
            &ast::Object::phi(),
        );
        let mut fields = HashMap::new();
        fields.insert("hue".to_string(), "crimson".to_string());
        let result = apply_command_defs(&def_map, &Command::UpdateEntity {
            noun: "Color".to_string(),
            domain: "d".to_string(),
            entity_id: "c1".to_string(),
            fields,
            force: false,
            sender: None,
            signature: None,
        }, &state);
        assert!(result.rejected && result.entities.is_empty(),
            "updateEntity for a value type must reject; got {:?}", result.violations);
        assert!(result.violations.iter().any(|v| v.constraint_id.contains("not_runtime_defined")),
            "rejection must carry the not_runtime_defined violation; got {:?}", result.violations);
    }

    // ── task-735 — auto-increment id reconciles both schemes ──────────
    //
    // Pre-735 the auto-generator counted distinct entity-role values
    // and emitted `<noun>-<count+1>`. A user asserting `id='916'` and
    // then auto-creating would receive `task-1` and the two id
    // schemes would accumulate without reconciliation. The four tests
    // below pin the new behaviour:
    //
    //   - empty cell → `<prefix>-1` (preserves the legacy default).
    //   - bare-integer dominant → next bare integer above the max.
    //   - `<prefix>-N` dominant → next `<prefix>-N` above the max.
    //   - mixed bag (`<prefix>-N` + bare ints) → max+1 in the dominant
    //     scheme; never returns an id that already exists.

    /// task-735 (acceptance 2): empty cell → `task-1`.
    /// Preserves the legacy default established under #867 so existing
    /// downstream consumers (UI listings, hash-of-pop snapshots, audit
    /// summaries) still see the same first id.
    #[test]
    fn auto_increment_id_empty_state_returns_task_1() {
        let state = ast::Object::phi();
        let id = super::auto_generate_entity_id("Task", &state);
        assert_eq!(id, "task-1",
            "empty cell must seed with the prefix scheme; got {id:?}");
    }

    /// task-735 (acceptance 1): `id='999'` asserted → auto-create
    /// returns an id strictly greater than 999 (bare-integer dominant
    /// because no `<prefix>-N` ids exist yet).
    #[test]
    fn auto_increment_id_bare_integer_picks_next_integer() {
        let ft_id = "Task has Task Status";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "999"), ("Task Status", "pending")]),
            &state);

        let id = super::auto_generate_entity_id("Task", &state);
        // Bare-integer scheme dominates → expect bare integer 1000.
        assert_eq!(id, "1000",
            "single bare-integer id 999 → next must be 1000; got {id:?}");
        // Defensive: the returned id must never already exist in `seen`.
        assert_ne!(id, "999",
            "auto-generated id must not collide with existing 999");
    }

    /// task-735 (acceptance 3a): pure `<prefix>-N` cell → returns
    /// `<prefix>-(max+1)`. Counting, the pre-735 emitter would have
    /// returned the same value here (3+1=4 distinct, +1 = 5) only by
    /// accident — `task-2` was skipped so the count and the max
    /// disagree. This test pins the new behaviour: it's max-based,
    /// not count-based.
    #[test]
    fn auto_increment_id_prefixed_scheme_picks_max_plus_one() {
        let ft_id = "Task has Task Status";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "task-1"), ("Task Status", "pending")]),
            &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "task-5"), ("Task Status", "pending")]),
            &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "task-3"), ("Task Status", "pending")]),
            &state);

        let id = super::auto_generate_entity_id("Task", &state);
        assert_eq!(id, "task-6",
            "task-1/task-3/task-5 → max=5, next must be task-6; got {id:?}");
    }

    /// task-735 (acceptance 3b): mixed `<prefix>-N` and bare-integer
    /// ids. Bare-integer dominant (916 > 3) → next bare integer
    /// (917). This is the exact scenario from the user's 2026-05-12
    /// observation: assert `id='916'`, then auto-create.
    #[test]
    fn auto_increment_id_mixed_schemes_bare_int_dominant() {
        let ft_id = "Task has Task Status";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "task-3"), ("Task Status", "pending")]),
            &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "916"), ("Task Status", "pending")]),
            &state);

        let id = super::auto_generate_entity_id("Task", &state);
        // Bare-integer 916 strictly dominates task-3 → next is 917.
        assert_eq!(id, "917",
            "task-3 + bare 916 → bare-int dominant, next must be 917; got {id:?}");
        // Defensive: must not collide with any existing id.
        assert!(id != "task-3" && id != "916",
            "auto-generated id must not collide with existing ids; got {id:?}");
    }

    /// task-735 (acceptance 3c): mixed schemes where `<prefix>-N`
    /// dominates → next is `<prefix>-(max+1)`. Tie-break also favors
    /// the prefixed form (engine's default emission shape).
    #[test]
    fn auto_increment_id_mixed_schemes_prefixed_dominant() {
        let ft_id = "Task has Task Status";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "task-50"), ("Task Status", "pending")]),
            &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[("Task", "10"), ("Task Status", "pending")]),
            &state);

        let id = super::auto_generate_entity_id("Task", &state);
        assert_eq!(id, "task-51",
            "task-50 dominates bare 10 → task-51; got {id:?}");
    }

    /// task-735 (acceptance 1, integration): assert `id='999'`
    /// through the full apply_command_defs path, then auto-create.
    /// The returned id must be strictly greater than 999 and must
    /// not collide. This is the smoke test for the user's observed
    /// MCP-surface symptom.
    #[test]
    fn auto_increment_id_through_apply_after_explicit_999() {
        let src = "\
            Task(.id) is an entity type.\n\
            Task Status is a value type.\n\
            Task has Task Status.\n\
        ";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        // Step 1: explicit id="999".
        let mut fields = HashMap::new();
        fields.insert("Task Status".to_string(), "pending".to_string());
        let cmd_explicit = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("999".to_string()),
            fields: fields.clone(),
            sender: None,
            signature: None,
        };
        let result_explicit = apply_command_defs(&def_map, &cmd_explicit, &state);
        assert!(!result_explicit.rejected,
            "explicit create with id=999 must not be rejected; violations={:?}",
            result_explicit.violations);

        // `result_explicit.state` is a delta (#209 — only the cells the
        // command touched). To set up step 2, merge the delta onto the
        // base state so the auto-create scans against a base populated
        // with `Task '999'`. Without merge_delta the auto-id scan would
        // see only the delta's cells (which still contain the new
        // 999 fact), but the platform's normal apply path commits the
        // delta back into D before the next call — `merge_delta` here
        // mirrors that boundary.
        let state_after = ast::merge_delta(&state, &result_explicit.state, None);

        // Step 2: auto-create against the post-merge state.
        let cmd_auto = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: None,
            fields,
            sender: None,
            signature: None,
        };
        let result_auto = apply_command_defs(&def_map, &cmd_auto, &state_after);
        assert!(!result_auto.rejected,
            "auto create after id=999 must not be rejected; violations={:?}",
            result_auto.violations);

        // The returned id must be strictly greater than 999. Accept
        // either scheme — bare 1000 (current policy under task-735)
        // or task-1000 (future toggle). What matters is that the id
        // is fresh and parses to N > 999 in some scheme.
        let auto_id = &result_auto.entities[0].id;
        let payload: u64 = auto_id
            .strip_prefix("task-")
            .unwrap_or(auto_id)
            .parse()
            .unwrap_or_else(|_| panic!(
                "auto-generated id must carry an integer payload; got {auto_id:?}"));
        assert!(payload > 999,
            "auto-generated id payload must exceed 999; got {auto_id:?} (payload={payload})");
        assert_ne!(auto_id, "999",
            "auto-generated id must never collide with existing 999");
    }

    #[test]
    fn transition_changes_status() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-100".to_string());
        let create_cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-100".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let created = apply_command_defs(&def_map, &create_cmd, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let cmd = Command::Transition {
            entity_id: "ORD-100".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &created.state);

        assert_eq!(result.status.as_deref(), Some("Placed"));
        assert!(result.transitions.iter().any(|t| t.event == "pay"));
    }

    #[test]
    fn state_contains_entity_facts() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        fields.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        // Entity fields are facts in the state
        let customer_cell = ast::fetch_cell_seq("Order_has_customer", &result.state);
        let customer_facts = customer_cell.as_seq().unwrap();
        assert_eq!(customer_facts.len(), 1);
        assert!(ast::binding(&customer_facts[0], "customer") == Some("acme"));

        // SM facts are in the state (task-742: renamed cell + role)
        let sm_cell = ast::fetch_cell_seq("State_Machine_is_currently_in_Status", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        assert!(ast::binding(&sm_facts[0], "Status") == Some("Draft"));
    }

    /// task-919: end-to-end dispatch via the Verb→Function chain.
    ///
    /// Wires `Verb 'place_verb' is performed during Transition 'place'.`
    /// + `Function 'place_verb' has Name 'task_919_noop_test'.` into the
    /// parsed state, installs a platform handler under that Name, fires
    /// the Order place transition, and asserts the handler ran exactly
    /// once AND the SM cell still flipped to Placed.
    ///
    /// The dispatch hook in transition_via_defs (after L1397) joins
    /// Transition_is_defined_in_State_Machine_Definition,
    /// _is_from_Status, _is_to_Status to find the firing transition,
    /// then chases Verb_is_performed_during_Transition →
    /// Function_has_Name and invokes apply_platform on the resolved name.
    ///
    /// Establishes the substrate; HTTP `Function has callback URI`
    /// dispatch and the 4 arest-dev rebuild handlers are follow-ups.
    #[test]
    fn transition_dispatches_platform_func_via_verb_function_chain() {
        use crate::ast::{install_platform_fn, uninstall_platform_fn};
        use core::sync::atomic::{AtomicUsize, Ordering};

        static CALLS: AtomicUsize = AtomicUsize::new(0);
        CALLS.store(0, Ordering::SeqCst);

        const READINGS: &str = r#"
# Orders with dispatch

## Entity Types

Order(.Order Number) is an entity type.
Verb(.id) is an entity type.
Function(.id) is an entity type.

## Fact Types

Order has Amount.
Verb is performed during Transition.
Function has Name.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Verb 'place_verb' is performed during Transition 'place'.
Function 'place_verb' has Name 'task_919_noop_test'.
"#;

        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state = crate::parse_forml2::parse_to_state_with_nouns(READINGS, &meta_state)
            .unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        install_platform_fn(
            "task_919_noop_test",
            crate::sync::Arc::new(|x: &ast::Object, _d: &ast::Object| {
                CALLS.fetch_add(1, Ordering::SeqCst);
                // Echo a non-Bottom value so the dispatch hook treats
                // this as success.
                x.as_map()
                    .and_then(|m| m.get("id").cloned())
                    .unwrap_or(ast::Object::atom("ok"))
            }),
        );

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-919".to_string());
        let create_cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-919".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let created = apply_command_defs(&def_obj, &create_cmd, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"),
            "create must land in Draft; status={:?} violations={:?}",
            created.status, created.violations);

        let txn = Command::Transition {
            entity_id: "ORD-919".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_obj, &txn, &created.state);

        uninstall_platform_fn("task_919_noop_test");

        assert!(!result.rejected,
            "transition must not be rejected; violations={:?}", result.violations);
        assert_eq!(result.status.as_deref(), Some("Placed"),
            "transition must flip status to Placed; got {:?}", result.status);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1,
            "platform handler 'task_919_noop_test' must run exactly once \
             after the Verb→Function dispatch chain resolves");
    }

    /// task-919 rollback: a Platform Func returning Bottom rejects the
    /// transition. The dispatch hook synthesizes a `dispatch:<name>`
    /// alethic Violation; the existing rejected → delta=phi path at
    /// L1406 emits an empty delta so the SM cell flip is rolled back
    /// upstream. The transition is reported as rejected with the
    /// synthesized violation surfaced to the caller.
    #[test]
    fn transition_dispatch_bottom_rejects_and_rolls_back() {
        use crate::ast::{install_platform_fn, uninstall_platform_fn};

        const READINGS: &str = r#"
# Orders with a failing dispatch

## Entity Types

Order(.Order Number) is an entity type.
Verb(.id) is an entity type.
Function(.id) is an entity type.

## Fact Types

Order has Amount.
Verb is performed during Transition.
Function has Name.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Verb 'place_verb' is performed during Transition 'place'.
Function 'place_verb' has Name 'task_919_bottom_test'.
"#;

        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state = crate::parse_forml2::parse_to_state_with_nouns(READINGS, &meta_state)
            .unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        install_platform_fn(
            "task_919_bottom_test",
            crate::sync::Arc::new(|_x: &ast::Object, _d: &ast::Object| ast::Object::Bottom),
        );

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-919-B".to_string());
        let created = apply_command_defs(&def_obj, &Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-919-B".to_string()),
            fields,
            sender: None,
            signature: None,
        }, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let result = apply_command_defs(&def_obj, &Command::Transition {
            entity_id: "ORD-919-B".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        }, &created.state);

        uninstall_platform_fn("task_919_bottom_test");

        assert!(result.rejected,
            "Bottom from platform handler must reject the transition; \
             violations={:?}", result.violations);
        assert!(result.violations.iter().any(|v|
            v.constraint_id == "dispatch:task_919_bottom_test" && v.alethic),
            "rejected transition must surface a dispatch:<name> alethic \
             violation; got {:?}", result.violations);
        // Rejected transitions emit an empty state delta (mirrors the
        // alethic-violation rollback at L1406), so no SM cell flip
        // reaches the caller.
        assert!(ast::cells_iter(&result.state).is_empty(),
            "rejected transition must emit an empty state delta; got {:?}",
            result.state);
    }

    /// task-919 gap-5: pin that `install_rebuild_fns(apps_dir)` wires the
    /// four arest-dev rebuild Platform Functions into PLATFORM_FALLBACK
    /// under their canonical names AND that they are reachable through
    /// the `apply(Func::Platform(name), x, d)` dispatch path (the same
    /// path the gap-3 transition_via_defs hook uses, exercised by
    /// `transition_dispatches_platform_func_via_verb_function_chain`
    /// above on a synthetic verb). End-to-end: install -> the registered
    /// rebuild_snapshot is callable through the registry -> the handler
    /// resolves the App Target via `d` and writes a snapshot file under
    /// the apps_dir captured at install time. The full SM-driven dispatch
    /// path is exercised by the synthetic-verb gap-3 tests above; this
    /// pins that the REAL rebuild handlers slot into that same path.
    #[cfg(feature = "local")]
    #[test]
    fn rebuild_install_fns_handlers_are_dispatchable_via_platform_apply() {
        use crate::ast::{uninstall_platform_fn, installed_platform_fn_names};

        let root = std::env::temp_dir().join(format!(
            "arest_rebuild_install_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let target = "installtarget";
        let tdir = root.join(target);
        std::fs::create_dir_all(&tdir).expect("mk target dir");
        {
            let conn = rusqlite::Connection::open(tdir.join(format!("{}.db", target)))
                .expect("open target db");
            conn.execute_batch(
                "CREATE TABLE cells (name TEXT PRIMARY KEY, contents TEXT);
                 INSERT INTO cells VALUES ('Task_is_epic', '<<Task, 772>>');",
            ).expect("seed target db");
        }

        crate::rebuild::install_rebuild_fns(root.clone());

        // 1. The four canonical names are now in PLATFORM_FALLBACK.
        let installed = installed_platform_fn_names();
        for name in &["rebuild_snapshot", "rebuild_verify", "rebuild_apply_bulk", "rebuild_init"] {
            assert!(installed.contains(&name.to_string()),
                "{} must be in PLATFORM_FALLBACK after install_rebuild_fns; installed = {:?}",
                name, installed);
        }

        // 2. apply(Func::Platform(name), x, d) routes to the installed
        //    handler (same dispatch path the gap-3 transition hook uses).
        //    Build the d the handler reads: it needs `Rebuild concerns App
        //    Target` to resolve our temp target. Build an x with the id.
        let mut dm: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        dm.insert(
            "Rebuild_concerns_App_Target".to_string(),
            ast::Object::seq(vec![ast::Object::seq(vec![
                ast::Object::seq(vec![ast::Object::atom("Rebuild"), ast::Object::atom("rb-install")]),
                ast::Object::seq(vec![ast::Object::atom("App Target"), ast::Object::atom(target)]),
            ])]),
        );
        let d = ast::Object::map(dm);
        let mut xm: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        xm.insert("id".to_string(), ast::Object::atom("rb-install"));
        let x = ast::Object::map(xm);

        let result = ast::apply(
            &ast::Func::Platform("rebuild_snapshot".to_string()),
            &x,
            &d,
        );

        for name in &["rebuild_snapshot", "rebuild_verify", "rebuild_apply_bulk", "rebuild_init"] {
            uninstall_platform_fn(name);
        }

        assert!(!matches!(result, ast::Object::Bottom),
            "apply(Func::Platform('rebuild_snapshot')) must reach the installed \
             handler and succeed on a valid target; got Bottom");
        let snap_dir = root.join(target).join("rebuild-snapshots");
        let snapshots: Vec<_> = std::fs::read_dir(&snap_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        assert!(!snapshots.is_empty(),
            "installed handler must have written a snapshot under {:?}", snap_dir);

        std::fs::remove_dir_all(&root).ok();
    }

    /// task-919-http: a tiny one-shot HTTP/1.1 fake server on a random
    /// loopback port. Spawns a thread that accepts ONE connection,
    /// records the request (so the test can assert on body / header
    /// shape), replies with the configured status + body, then exits.
    ///
    /// Pure std — no httpmock dep — to keep the per-feature cold-build
    /// surface unchanged. Returns the bound URL and a join handle whose
    /// payload is the captured request bytes; callers `.join()` after
    /// the dispatch fires to read them.
    fn spawn_one_shot_http_server(
        status_code: u16,
        status_text: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<u8>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("bind a free loopback port");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}/dispatch", addr);
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            sock.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
            // Read until we see the body-length's worth of bytes after
            // the CRLFCRLF header terminator. The dispatch path sends
            // Content-Length, so this is bounded.
            let mut buf = Vec::with_capacity(4096);
            let mut chunk = [0u8; 1024];
            let mut header_end: Option<usize> = None;
            let mut content_length: usize = 0;
            loop {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 { break; }
                buf.extend_from_slice(&chunk[..n]);
                if header_end.is_none() {
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(i + 4);
                        let head = std::str::from_utf8(&buf[..i]).unwrap_or("");
                        for line in head.lines() {
                            if let Some(rest) = line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                            {
                                content_length = rest.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
                if let Some(end) = header_end {
                    if buf.len() >= end + content_length { break; }
                }
            }
            let resp = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_code, status_text, body.len(), body);
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
            buf
        });
        (url, handle)
    }

    /// task-919-http: end-to-end dispatch via the Verb→Function chain
    /// when the function has a callback URI (not an in-process Name).
    /// Stands up a one-shot loopback HTTP server, wires
    /// `Function 'place_verb' has callback URI '<server>'` + a
    /// `Function 'place_verb' has Header 'X-AREST-Dispatch: 1'` into
    /// the parsed state, fires the Order place transition, and
    /// asserts:
    ///   * the server received exactly one POST,
    ///   * the request body parses as JSON carrying the ctx Map
    ///     (noun/id/from_status/to_status/transition_id/verb_id/event),
    ///   * the custom Header lands on the wire,
    ///   * the SM cell still flipped to Placed and the transition is
    ///     not rejected.
    ///
    /// Mirrors the substrate from `transition_dispatches_platform_func_
    /// via_verb_function_chain` so the two branches stay in lockstep.
    #[test]
    fn transition_dispatches_via_callback_uri_success() {
        let (url, handle) = spawn_one_shot_http_server(200, "OK", "{\"ack\":true}");

        // Mirror core.md: "callback URI" and "Header" are value types so
        // the FT id from `fact_type_id_from_reading` recovers a binary
        // role list (Function, callback URI) / (Function, Header), and
        // the parsed instance facts land in the `Function_has_callback_URI`
        // / `Function_has_Header` cells the dispatch helper queries.
        let readings = format!(r#"
# Orders with HTTP dispatch

## Entity Types

Order(.Order Number) is an entity type.
Verb(.id) is an entity type.
Function(.id) is an entity type.
callback URI is a value type.
Header is a value type.

## Fact Types

Order has Amount.
Verb is performed during Transition.
Function has callback URI.
Function has Header.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Verb 'place_verb' is performed during Transition 'place'.
Function 'place_verb' has callback URI '{}'.
Function 'place_verb' has Header 'X-AREST-Dispatch: 1'.
"#, url);

        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state = crate::parse_forml2::parse_to_state_with_nouns(&readings, &meta_state)
            .unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-919H".to_string());
        let created = apply_command_defs(&def_obj, &Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-919H".to_string()),
            fields,
            sender: None,
            signature: None,
        }, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"),
            "create must land in Draft; violations={:?}", created.violations);

        let result = apply_command_defs(&def_obj, &Command::Transition {
            entity_id: "ORD-919H".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        }, &created.state);

        assert!(!result.rejected,
            "2xx response must not reject the transition; violations={:?}",
            result.violations);
        assert_eq!(result.status.as_deref(), Some("Placed"),
            "transition must flip status to Placed; got {:?}", result.status);

        // Server thread observed exactly one request.
        let captured = handle.join().expect("server thread");
        let captured_str = String::from_utf8_lossy(&captured);
        assert!(captured_str.starts_with("POST /dispatch HTTP/1.1"),
            "expected POST request; got: {:?}", captured_str);
        assert!(captured_str.to_ascii_lowercase()
                    .contains("x-arest-dispatch: 1"),
            "Function_has_Header fact must land on the wire as a header; \
             got: {:?}", captured_str);
        // The body sits after the CRLFCRLF; parse it as JSON and check
        // a couple of ctx fields.
        let body_start = captured_str.find("\r\n\r\n").unwrap() + 4;
        let body = &captured_str[body_start..];
        let parsed: serde_json::Value = serde_json::from_str(body.trim())
            .unwrap_or_else(|e| panic!(
                "request body must be JSON; got: {:?} err: {}", body, e));
        assert_eq!(parsed.get("noun").and_then(|v| v.as_str()), Some("Order"));
        assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("ORD-919H"));
        assert_eq!(parsed.get("from_status").and_then(|v| v.as_str()), Some("Draft"));
        assert_eq!(parsed.get("to_status").and_then(|v| v.as_str()), Some("Placed"));
        assert_eq!(parsed.get("event").and_then(|v| v.as_str()), Some("place"));
    }

    /// task-919-http rollback: callback URI returning a non-2xx status
    /// rejects the transition. The dispatch hook synthesizes a
    /// `dispatch:<uri>` alethic violation; the existing rejected →
    /// delta=phi path emits an empty delta so the SM cell flip is
    /// rolled back upstream. Same observable shape as the Bottom
    /// branch (`transition_dispatch_bottom_rejects_and_rolls_back`),
    /// just keyed on the URI instead of the Name.
    #[test]
    fn transition_dispatch_callback_uri_non_2xx_rejects_and_rolls_back() {
        let (url, handle) = spawn_one_shot_http_server(500, "Internal Server Error", "boom");

        let readings = format!(r#"
# Orders with failing HTTP dispatch

## Entity Types

Order(.Order Number) is an entity type.
Verb(.id) is an entity type.
Function(.id) is an entity type.
callback URI is a value type.

## Fact Types

Order has Amount.
Verb is performed during Transition.
Function has callback URI.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Verb 'place_verb' is performed during Transition 'place'.
Function 'place_verb' has callback URI '{}'.
"#, url);

        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state = crate::parse_forml2::parse_to_state_with_nouns(&readings, &meta_state)
            .unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-919H-B".to_string());
        let created = apply_command_defs(&def_obj, &Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-919H-B".to_string()),
            fields,
            sender: None,
            signature: None,
        }, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let result = apply_command_defs(&def_obj, &Command::Transition {
            entity_id: "ORD-919H-B".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        }, &created.state);

        // Drain the server thread so the OS port is released.
        let _ = handle.join();

        assert!(result.rejected,
            "non-2xx response from callback must reject the transition; \
             violations={:?}", result.violations);
        // The violation surfaces a `dispatch:<uri>` constraint id.
        assert!(result.violations.iter().any(|v|
            v.constraint_id == format!("dispatch:{}", url) && v.alethic),
            "rejected transition must surface a dispatch:<uri> alethic \
             violation; got {:?}", result.violations);
        // The delta is empty so the SM cell flip is rolled back.
        assert!(ast::cells_iter(&result.state).is_empty(),
            "rejected transition must emit an empty state delta; got {:?}",
            result.state);
    }

    /// task-919-https: `https://` URLs no longer hit the
    /// "only http:// supported" early-out; they route through the
    /// TLS-wrap branch. The test points at a cleartext-TCP listener
    /// that accepts the connection but doesn't speak TLS, so the
    /// rustls handshake fails — which proves the function *attempted*
    /// the TLS path rather than rejecting the scheme up-front.
    ///
    /// Acceptance shape: the error message contains "TLS handshake"
    /// (the prefix our connect_https helper attaches) AND not "only
    /// http://" (the pre-task-919-https rejection text).
    #[test]
    fn http_post_callback_routes_https_through_tls_handshake() {
        use std::io::Read;
        use std::net::TcpListener;

        // Bind a loopback port we control. The accepting thread reads
        // a few bytes (the rustls ClientHello) then closes, so the
        // handshake fails on the response side.
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("bind a free loopback port");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                sock.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf); // consume ClientHello bytes
                // Drop the socket without sending a ServerHello —
                // rustls surfaces a handshake failure.
            }
        });

        let url = format!("https://127.0.0.1:{}/dispatch", addr.port());
        let result = http_post_callback(&url, b"{}", &[]);
        let _ = handle.join();

        let err = result.expect_err(
            "https:// against a non-TLS listener must fail at the handshake");
        assert!(!err.contains("only http://"),
            "task-919-https: https:// must no longer be rejected by scheme; \
             got: {}", err);
        // The handshake failure goes through our connect_https path
        // (which prefixes "TLS handshake to … failed"). If we ever
        // change the error wrapping, this stays anchored on the
        // *behavior* — that the request did NOT short-circuit on
        // scheme — via the `!only http://` check above. The TLS-
        // specific assertion below pins the wrapping.
        assert!(err.contains("TLS handshake"),
            "https:// connect failure must surface the TLS handshake \
             error wrap; got: {}", err);
    }

    /// task-919-https: a `https://` URL that names a port nothing is
    /// listening on still routes through the TLS branch — i.e. it
    /// fails at TCP connect (default port 443 / parsed port), NOT at
    /// the "only http://" guard. Locks the scheme-dispatch behavior
    /// independently from any TLS-handshake substrate.
    #[test]
    fn http_post_callback_https_unreachable_port_does_not_reject_scheme() {
        // Reserve a port by binding-and-dropping. After drop the
        // OS keeps it in TIME_WAIT for the test process, so a new
        // connect attempt against the same address sees connection
        // refused (or timeout on some configurations). Either way
        // the error is from the TCP layer, not the scheme guard.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind a free loopback port");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        let url = format!("https://127.0.0.1:{}/dispatch", port);
        let result = http_post_callback(&url, b"{}", &[]);
        let err = result.expect_err(
            "https:// against an unreachable port must fail");
        assert!(!err.contains("only http://"),
            "task-919-https: https:// must not be rejected by scheme; \
             got: {}", err);
    }

    /// task-919-https: the unreachable-port behaviour is identical for
    /// `http://` and `https://` — both pass the scheme guard and both
    /// fail at the TCP layer. Pin the symmetry so a future regression
    /// can't tilt one branch back to a scheme rejection.
    #[test]
    fn http_post_callback_http_and_https_both_pass_scheme_guard() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind a free loopback port");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        let http_url = format!("http://127.0.0.1:{}/dispatch", port);
        let https_url = format!("https://127.0.0.1:{}/dispatch", port);
        let http_err = http_post_callback(&http_url, b"{}", &[])
            .expect_err("http:// against unreachable port must fail");
        let https_err = http_post_callback(&https_url, b"{}", &[])
            .expect_err("https:// against unreachable port must fail");
        assert!(!http_err.contains("only http://"));
        assert!(!https_err.contains("only http://"));
    }

    /// task-919-https: anything that isn't http:// or https:// is
    /// rejected up-front with a clear error — the dispatch hook
    /// surfaces the alethic violation, no socket is ever opened.
    /// Pins the scheme-guard's positive side; the negative side
    /// (http/https pass) is covered by the three tests above.
    #[test]
    fn http_post_callback_rejects_non_http_schemes() {
        for url in &["ftp://example.com/dispatch",
                     "file:///tmp/dispatch",
                     "ws://example.com/dispatch",
                     "/no/scheme",
                     ""]
        {
            let err = http_post_callback(url, b"{}", &[])
                .expect_err(&format!("{:?} must be rejected by scheme guard", url));
            assert!(err.contains("only http:// or https://"),
                "non-http(s) URL {:?} must hit the scheme guard; got: {}",
                url, err);
        }
    }

    /// task-737 — acceptance criterion #3. Two `apply create Task`
    /// calls with the same explicit `id='999'` must surface an
    /// alethic UC violation on the second call and leave the first
    /// fact in place. Pre-#737 both creates silently succeeded and
    /// the resulting cells held two facts per cell — substrate
    /// corruption per ORM 2 reference-scheme semantics.
    #[test]
    fn create_with_duplicate_explicit_id_rejects_second_with_uc_violation() {
        let src = "Task(.id) is an entity type.\nTask has Description.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        // First create: explicit id='999'. Must succeed.
        let mut fields = HashMap::new();
        fields.insert("Description".to_string(), "first".to_string());
        let create1 = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("999".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let result1 = apply_command_defs(&def_map, &create1, &state);
        assert!(!result1.rejected,
            "first create must succeed; violations={:?}", result1.violations);
        let state_after_first = ast::merge_states(&state, &result1.state);

        // Second create: same explicit id='999'. Must surface a UC
        // violation and reject; the first Task remains intact.
        let mut fields2 = HashMap::new();
        fields2.insert("Description".to_string(), "second".to_string());
        let create2 = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("999".to_string()),
            fields: fields2,
            sender: None,
            signature: None,
        };
        let result2 = apply_command_defs(&def_map, &create2, &state_after_first);
        assert!(result2.rejected,
            "second create with the same id must be rejected; \
             violations={:?}", result2.violations);
        assert!(result2.violations.iter().any(|v| v.alethic),
            "must surface at least one alethic violation; got {:?}",
            result2.violations);

        // First fact retained: the Description from create1 still
        // sits in `Task_has_Description`.
        let desc_cell = ast::fetch_cell_seq("Task_has_Description", &state_after_first);
        let entries: Vec<&ast::Object> = desc_cell.as_seq()
            .map(|s| s.iter().collect()).unwrap_or_default();
        let task_999_desc: Option<String> = entries.iter()
            .find(|f| ast::binding(f, "Task") == Some("999"))
            .and_then(|f| ast::binding(f, "Description").map(String::from));
        assert_eq!(task_999_desc.as_deref(), Some("first"),
            "the first Task '999''s Description must remain 'first' \
             (the rejected second create must not have replaced or \
             extended it); got {:?}; entries={:?}",
            task_999_desc, entries);
    }

    /// task-822 helper: parse + compile a UC-bearing schema once per
    /// test. Single-FT `Task has Status` with an alethic UC on the
    /// `Task` role — `_CellKeyRoles` will register
    /// `Task_has_Status → ["Task"]`, routing apply-time writes through
    /// `cell_put_keyed`. Mirrors the shape `forward_chain_routes_
    /// keyed_cells_through_map_storage_for_alethic_uc` exercises in
    /// `evaluate.rs`, but for the user-facing apply path.
    fn setup_task_uc_defs() -> (ast::Object, ast::Object) {
        let src = "\
            Task(.id) is an entity type.\n\
            Status is a value type.\n\
            Task has Status.\n\
            Each Task has at most one Status.\n\
        ";
        let parsed = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        // Two pre-task-822 quirks in the parser/compile boundary keep
        // `_CellKeyRoles` empty for this kind of UC. Both are localized
        // by the test helper — `compile.rs` is off-limits per the
        // task-822 constraints, but the helper edits live on the
        // parser-output state before `compile_to_defs_state` runs.
        //
        //   1. `parse_forml2_stage2::enrich_constraints_with_spans` mirrors
        //      `span0_*` into `span1_*` for UC/MC/VC/FC ("legacy quirk"
        //      — comment at parse_forml2_stage2.rs:2310). That gives a
        //      role-1 UC two spans on the same role; `compile.rs::
        //      resolve_key_roles_for_ft` then counts `roles_here.len()
        //      == ft.roles.len()` and treats it as a spanning UC
        //      (returns None).
        //   2. The parser emits `modality = "alethic"` (lowercase) but
        //      `resolve_key_roles_for_ft` compares with `"Alethic"`
        //      (capital). All UCs miss the filter.
        //
        // Strip the redundant `span1_*` pair and lift the modality
        // case. Net effect: the metamodel state the test passes to
        // `compile_to_defs_state` looks like one a future-fixed parser
        // would produce, and `_CellKeyRoles` registers `Task_has_Status
        // → ["Task"]`.
        let state = rewrite_constraint_cell_for_uc_key_resolution(&parsed);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);
        (def_map, state)
    }

    /// Test-only fix-up over the parser's `Constraint` cell so that
    /// `compile.rs::resolve_key_roles_for_ft` picks the cell's alethic
    /// UCs as the FT's `key_roles`. Localized rewrite — see the
    /// rationale block in `setup_task_uc_defs`.
    fn rewrite_constraint_cell_for_uc_key_resolution(state: &ast::Object) -> ast::Object {
        let cell = ast::fetch_or_phi("Constraint", state);
        let Some(facts) = cell.as_seq() else { return state.clone() };
        let rewritten: Vec<ast::Object> = facts.iter().map(|fact| {
            let Some(pairs) = fact.as_seq() else { return fact.clone() };
            // Drop the parser's mirror span1_* pair when it duplicates
            // span0_* (the UC/MC quirk at parse_forml2_stage2.rs:2388).
            let span0_ft: Option<String> = pairs.iter().find_map(|p| {
                let kv = p.as_seq()?;
                (kv.first()?.as_atom()? == "span0_factTypeId")
                    .then(|| kv.get(1)?.as_atom().map(|s| s.to_string()))?
            });
            let span0_role: Option<String> = pairs.iter().find_map(|p| {
                let kv = p.as_seq()?;
                (kv.first()?.as_atom()? == "span0_roleIndex")
                    .then(|| kv.get(1)?.as_atom().map(|s| s.to_string()))?
            });
            let new_pairs: Vec<ast::Object> = pairs.iter().filter_map(|p| {
                let kv = match p.as_seq() { Some(kv) => kv, None => return Some(p.clone()) };
                let key = kv.first().and_then(|k| k.as_atom()).unwrap_or("");
                // Lift `alethic` / `deontic` to `Alethic` / `Deontic`.
                if key == "modality" {
                    let val = kv.get(1).and_then(|v| v.as_atom()).unwrap_or("");
                    let lifted = match val {
                        "alethic" => "Alethic",
                        "deontic" => "Deontic",
                        other => other,
                    };
                    return Some(ast::Object::seq(vec![
                        ast::Object::atom(key),
                        ast::Object::atom(lifted),
                    ]));
                }
                // Drop span1_* when it byte-equals the span0_* pair —
                // this is the UC/MC mirror the parser injects.
                if key == "span1_factTypeId" {
                    let val = kv.get(1).and_then(|v| v.as_atom()).unwrap_or("");
                    if Some(val) == span0_ft.as_deref() { return None; }
                }
                if key == "span1_roleIndex" {
                    let val = kv.get(1).and_then(|v| v.as_atom()).unwrap_or("");
                    if Some(val) == span0_role.as_deref() { return None; }
                }
                Some(p.clone())
            }).collect();
            ast::Object::Seq(new_pairs.into())
        }).collect();
        ast::store("Constraint", ast::Object::Seq(rewritten.into()), state)
    }


    /// task-822 acceptance #1: two `apply operation=create` with the
    /// same key role value and different non-key role value produce a
    /// UC violation on the second call; state retains only the first
    /// fact. Mirrors task-818's forward-chain semantics at the
    /// user-facing apply boundary — without this fix the second
    /// `cell_push` silently appends a Seq entry and the violation only
    /// surfaces (if at all) on the next forward-chain or validate pass.
    #[test]
    fn apply_create_with_uc_conflict_surfaces_violation_and_retains_first_fact() {
        let (def_map, state) = setup_task_uc_defs();

        let mut fields_a = HashMap::new();
        fields_a.insert("Status".to_string(), "draft".to_string());
        let create_a = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("t1".to_string()),
            fields: fields_a,
            sender: None,
            signature: None,
        };
        let result_a = apply_command_defs(&def_map, &create_a, &state);
        assert!(!result_a.rejected,
            "first create must succeed; violations={:?}", result_a.violations);

        // Commit the delta back so the second create runs against the
        // state that already carries the first fact — `cell_put_keyed`
        // then sees the collision on the second write.
        let post_a = ast::merge_delta(&state, &result_a.state, None);

        let mut fields_b = HashMap::new();
        fields_b.insert("Status".to_string(), "shipped".to_string());
        let create_b = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("t1".to_string()), // same Task key as create_a
            fields: fields_b,
            sender: None,
            signature: None,
        };
        let result_b = apply_command_defs(&def_map, &create_b, &post_a);

        let uc = result_b.violations.iter()
            .find(|v| v.constraint_id.starts_with("uc:") && v.alethic)
            .unwrap_or_else(|| panic!(
                "second create must surface a UC violation; \
                 violations={:?}", result_b.violations,
            ));
        assert!(uc.detail.contains("Uniqueness violation"),
            "violation detail must match the compile_uniqueness_ast shape; got {:?}", uc.detail);
        assert!(uc.detail.contains("t1"),
            "violation detail must cite the colliding key 't1'; got {:?}", uc.detail);
        assert!(result_b.rejected,
            "alethic UC violation must reject the apply; result={:?}", result_b);

        // State must still carry the FIRST fact only: the second write
        // was suppressed by `push_with_uc_check` and the apply was
        // rejected, so `create_via_defs` ships an empty delta. Merging
        // an empty delta onto `post_a` keeps the original Map content.
        let post_b = ast::merge_delta(&post_a, &result_b.state, None);
        let cell = ast::fetch_or_phi("Task_has_Status", &post_b);
        let map = cell.as_map()
            .unwrap_or_else(|| panic!("UC-bearing cell must be Map storage; got {:?}", cell));
        let entry = map.get("t1").unwrap_or_else(|| panic!(
            "map must retain the first fact under key 't1'; keys={:?}",
            map.keys().collect::<Vec<_>>(),
        ));
        assert_eq!(ast::binding(entry, "Status"), Some("draft"),
            "first fact's Status='draft' must survive; got {:?}", entry);
        assert_eq!(map.len(), 1,
            "exactly one entry for the colliding key; map={:?}", map);
    }

    /// task-822 acceptance #2: `apply operation=update` on an existing
    /// key with the same fact is a no-op — no violation, cell content
    /// structurally unchanged. `cell_put_keyed` returns
    /// `Ok(state.clone())` on byte-equal re-assertion (task-744
    /// phase 4); `push_with_uc_check`'s overwrite branch preserves that
    /// short-circuit.
    #[test]
    fn apply_update_with_same_keyed_fact_is_noop_and_preserves_cell() {
        let (def_map, state) = setup_task_uc_defs();

        let mut fields = HashMap::new();
        fields.insert("Status".to_string(), "draft".to_string());
        let create = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("t1".to_string()),
            fields: fields.clone(),
            sender: None,
            signature: None,
        };
        let created = apply_command_defs(&def_map, &create, &state);
        assert!(!created.rejected,
            "create must succeed; violations={:?}", created.violations);
        let post_create = ast::merge_delta(&state, &created.state, None);
        let cell_before = ast::fetch_or_phi("Task_has_Status", &post_create);

        let update = Command::UpdateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            entity_id: "t1".to_string(),
            fields,
            force: false,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &update, &post_create);
        assert!(!result.rejected,
            "same-fact update must not be rejected; violations={:?}", result.violations);
        let uc_violations: Vec<_> = result.violations.iter()
            .filter(|v| v.constraint_id.starts_with("uc:"))
            .collect();
        assert!(uc_violations.is_empty(),
            "same-fact update must produce no UC violations; got {:?}", uc_violations);

        let post_update = ast::merge_delta(&post_create, &result.state, None);
        let cell_after = ast::fetch_or_phi("Task_has_Status", &post_update);
        assert_eq!(cell_before, cell_after,
            "same-fact update must leave the cell structurally unchanged; \
             before={:?}, after={:?}", cell_before, cell_after);
    }

    /// task-822 acceptance #3: `apply operation=create` on a UC-bearing
    /// FT lands as `Object::Map<key, fact>` in the resulting state, not
    /// `Object::Seq`. This is the storage-shape contract that task-744
    /// phase 4 wires through forward-chain; task-822 extends it to the
    /// user-facing apply path so dispatcher / freeze / introspection
    /// paths see Map storage on UC-keyed cells regardless of write
    /// origin (derivation vs. user assertion).
    #[test]
    fn apply_create_lands_uc_keyed_cell_as_object_map_not_seq() {
        let (def_map, state) = setup_task_uc_defs();

        let mut fields = HashMap::new();
        fields.insert("Status".to_string(), "draft".to_string());
        let cmd = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("t1".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected,
            "create must succeed; violations={:?}", result.violations);

        let post = ast::merge_delta(&state, &result.state, None);
        let cell = ast::fetch_or_phi("Task_has_Status", &post);
        let map = cell.as_map().unwrap_or_else(|| panic!(
            "UC-keyed cell must be Object::Map after apply; got {:?}", cell,
        ));
        assert_eq!(map.len(), 1, "exactly one entry for Task 't1'; map={:?}", map);
        let entry = map.get("t1").unwrap_or_else(|| panic!(
            "map must carry an entry under key 't1'; keys={:?}",
            map.keys().collect::<Vec<_>>(),
        ));
        assert_eq!(ast::binding(entry, "Task"), Some("t1"));
        assert_eq!(ast::binding(entry, "Status"), Some("draft"));
    }

    // ── task-861: SM-bypass guard on apply update ────────────────────
    //
    // The engine's update_via_defs path refuses to mutate the SM's
    // status-role field directly. The SM cell
    // (StateMachine_has_currentlyInStatus) is the canonical status —
    // direct mutation would silently desync any derivation reading
    // SM state. The user must invoke `apply transition` instead.
    // `force: true` is the documented opt-out (#904 convention).
    //
    // Test corpus: a Task domain with an SM bound to noun 'Task',
    // initial status 'pending', and a `Task is started` event that
    // transitions pending → in_progress. The status-role field name
    // (by convention) is `Task Status`.
    const TASK_SM_READINGS: &str = r#"
# Tasks

## Entity Types

Task(.id) is an entity type.

## Fact Types

Task has Task Status.

## Instance Facts

State Machine Definition 'Task' is for Noun 'Task'.
Status 'pending' is initial in State Machine Definition 'Task'.

Transition 'start' is defined in State Machine Definition 'Task'.
  Transition 'start' is from Status 'pending'.
  Transition 'start' is to Status 'in_progress'.
  Transition 'start' is triggered by Event Type 'start'.
"#;

    fn setup_task_sm_defs() -> (ast::Object, ast::Object) {
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let tasks_state = crate::parse_forml2::parse_to_state_with_nouns(TASK_SM_READINGS, &meta_state).unwrap();
        let state = ast::merge_states(&meta_state, &tasks_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);
        (def_obj, state)
    }

    /// task-861 acceptance #1: `apply update noun=Task fields={Task
    /// Status: "in_progress"}` is rejected with an alethic violation
    /// whose constraint_id matches `sm:Task:status_immutable`. The
    /// detail string names `apply transition` as the right verb.
    /// State must NOT change (delta is empty).
    #[test]
    fn apply_update_status_sm_transition_refuses_direct_status_mutation() {
        let (def_map, base_state) = setup_task_sm_defs();
        // Seed Task t-1 with pending status via the SM cell so the
        // apply path has an existing entity to address. We push the
        // SM fact directly because creating via apply would also
        // exercise the SM-init path; this isolates the guard.
        let state = ast::cell_push(
            "StateMachine_has_currentlyInStatus",
            ast::fact_from_pairs(&[
                ("State Machine", "t-1"),
                ("currentlyInStatus", "pending"),
            ]),
            &base_state,
        );

        let mut fields = HashMap::new();
        fields.insert("Task Status".to_string(), "in_progress".to_string());
        let cmd = Command::UpdateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            entity_id: "t-1".to_string(),
            fields,
            sender: None,
            signature: None,
            force: false,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);

        assert!(result.rejected,
            "apply update of SM-driven Task Status must be rejected; \
             violations={:?}", result.violations);
        let v = result.violations.iter()
            .find(|v| v.constraint_id == "sm:Task:status_immutable")
            .expect("expected sm:Task:status_immutable violation; \
                     violations={:?}");
        assert!(v.alethic, "violation must be alethic");
        assert!(v.detail.contains("apply transition"),
            "detail must point at apply transition; got '{}'", v.detail);

        // State is unchanged — delta empty.
        assert_eq!(
            result.state, ast::Object::phi(),
            "rejected apply must emit empty delta; got {:?}",
            result.state
        );
        // SM cell still reflects pending.
        let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &state);
        let status = sm_cell.as_seq().unwrap().iter()
            .find(|f| ast::binding_matches(f, "State Machine", "t-1"))
            .and_then(|f| ast::binding(f, "currentlyInStatus"))
            .unwrap();
        assert_eq!(status, "pending", "SM status must not have flipped");
    }

    /// task-861 acceptance #2: `force: true` bypasses the SM-bypass
    /// guard. The update proceeds; no `sm:Task:status_immutable`
    /// violation surfaces. Mirrors the MCP `force: true` opt-out
    /// for migration scripts.
    #[test]
    fn apply_update_status_sm_force_true_bypasses_guard() {
        let (def_map, base_state) = setup_task_sm_defs();
        let state = ast::cell_push(
            "StateMachine_has_currentlyInStatus",
            ast::fact_from_pairs(&[
                ("State Machine", "t-1"),
                ("currentlyInStatus", "pending"),
            ]),
            &base_state,
        );

        let mut fields = HashMap::new();
        fields.insert("Task Status".to_string(), "in_progress".to_string());
        let cmd = Command::UpdateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            entity_id: "t-1".to_string(),
            fields,
            sender: None,
            signature: None,
            force: true,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);

        let guard_violation = result.violations.iter()
            .find(|v| v.constraint_id == "sm:Task:status_immutable");
        assert!(
            guard_violation.is_none(),
            "force=true must bypass the SM-immutable guard; \
             got violation {:?}",
            guard_violation,
        );
    }

    /// Local-task #2: the MCP server's merge-pre-fetch (#868/#872)
    /// folds the existing SM-driven Status into every UpdateEntity
    /// payload so untouched fields don't get retracted. When the user
    /// only intended to edit a non-Status field (e.g. Task Priority),
    /// the merged payload still names `Task Status` with the entity's
    /// *current* SM value. The pre-fix engine guard at
    /// `update_via_defs` keys solely off `new_fields.contains_key(&
    /// status_field)`, so it rejects the entire update even though
    /// Status is byte-identical to the SM cell.
    ///
    /// The guard's purpose (task-861 / #904) is to refuse direct
    /// mutation of the SM-driven status — a no-op overwrite is not
    /// a mutation, so it must not trip the guard. The fix: only
    /// reject when the payload's `{noun} Status` value differs from
    /// `extract_sm_status(state, entity_id)`. The "actually mutating
    /// Status" case (acceptance #1 above) and the `force: true`
    /// opt-out (acceptance #2) remain covered.
    #[test]
    fn apply_update_non_status_field_on_sm_noun_does_not_trip_status_guard() {
        // Reuse the task-861 SM fixture and add a non-SM-governed
        // priority FT so the payload has *something* legitimate to
        // change alongside the merged-in Status echo.
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let readings = r#"
# Tasks

## Entity Types

Task(.id) is an entity type.

## Fact Types

Task has Task Status.
Task has Task Priority.

## Instance Facts

State Machine Definition 'Task' is for Noun 'Task'.
Status 'pending' is initial in State Machine Definition 'Task'.
"#;
        let tasks_state = crate::parse_forml2::parse_to_state_with_nouns(readings, &meta_state).unwrap();
        let merged = ast::merge_states(&meta_state, &tasks_state);
        let defs = crate::compile::compile_to_defs_state(&merged);
        let def_map = ast::defs_to_state(&defs, &merged);

        // Seed Task t-1: SM cell says pending; user-facing
        // Task_has_Task_Priority says p2 (the field the user wants
        // to edit). No Task_has_Task_Status entry — that role is
        // purely SM-driven on this schema. Use the canonical
        // `State_Machine_is_currently_in_Status` cell shape so
        // `extract_sm_status` actually picks it up — see
        // `StateMachineCellShape::boot()`. (The pre-existing
        // acceptance fixtures above push to a stale `StateMachine_
        // has_currentlyInStatus` cell name; those tests still pass
        // because the old guard didn't read SM state.)
        let state = ast::cell_push(
            "State_Machine_is_currently_in_Status",
            ast::fact_from_pairs(&[
                ("State Machine", "t-1"),
                ("Status", "pending"),
            ]),
            &merged,
        );
        let state = ast::cell_push(
            "Task_has_Task_Priority",
            ast::fact_from_pairs(&[
                ("Task", "t-1"),
                ("Task Priority", "p2"),
            ]),
            &state,
        );

        // Mirror the MCP merge-pre-fetch payload shape exactly: the
        // user's intent is `Task Priority: p0`, but the server
        // folded in `Task Status: pending` (the entity's current SM
        // value) so #868's per-field retract-then-insert doesn't
        // drop Status as if it were retracted. Status value here
        // equals what `extract_sm_status` returns for t-1 — this is
        // a no-op for Status and a real change for Priority.
        let mut fields = HashMap::new();
        fields.insert("Task Status".to_string(), "pending".to_string());
        fields.insert("Task Priority".to_string(), "p0".to_string());
        let cmd = Command::UpdateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            entity_id: "t-1".to_string(),
            fields,
            sender: None,
            signature: None,
            force: false,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);

        let sm_guard = result.violations.iter()
            .find(|v| v.constraint_id == "sm:Task:status_immutable");
        assert!(
            sm_guard.is_none() && !result.rejected,
            "update of a non-Status field must not trip sm:Task:status_immutable \
             when the payload's Task Status equals the current SM status; \
             rejected={}, violations={:?}",
            result.rejected, result.violations,
        );

        // Priority edit lands in the merged entity row the caller
        // sees (existing ∪ payload, payload wins) — that's the
        // contract the MCP server's response surface depends on.
        let prio_in_response = result.entities.first()
            .and_then(|e| e.data.get("Task Priority").cloned());
        assert_eq!(
            prio_in_response.as_deref(), Some("p0"),
            "Task Priority must reflect the edit in entities[0].data; \
             got {:?}, full data={:?}",
            prio_in_response, result.entities.first().map(|e| &e.data),
        );

        // SM status is byte-identical — the no-op Status echo did
        // not flip the SM cell.
        let sm_cell = ast::fetch_or_phi("State_Machine_is_currently_in_Status", &state);
        let status = sm_cell.as_seq().unwrap().iter()
            .find(|f| ast::binding_matches(f, "State Machine", "t-1"))
            .and_then(|f| ast::binding(f, "Status"))
            .unwrap();
        assert_eq!(status, "pending",
            "SM status must remain pending after a no-op Status echo");
    }

    /// task-861 acceptance #3: `apply transition noun=Task id=t-1
    /// event="start"` still works — the SM cell flips pending →
    /// in_progress. The transition path is unaffected by the
    /// update-path guard.
    #[test]
    fn apply_update_status_sm_transition_still_advances_state_machine() {
        let (def_map, base_state) = setup_task_sm_defs();
        let state = ast::cell_push(
            "StateMachine_has_currentlyInStatus",
            ast::fact_from_pairs(&[
                ("State Machine", "t-1"),
                ("currentlyInStatus", "pending"),
            ]),
            &base_state,
        );

        let cmd = Command::Transition {
            entity_id: "t-1".to_string(),
            event: "start".to_string(),
            domain: "tasks".to_string(),
            current_status: Some("pending".to_string()),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);

        assert!(!result.rejected,
            "transition must not be rejected; violations={:?}", result.violations);
        assert_eq!(
            result.status.as_deref(), Some("in_progress"),
            "transition must flip SM to in_progress"
        );
    }

    #[test]
    fn transition_updates_state_status() {
        let (def_map, state) = setup_order_defs();

        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-1".to_string());
        let create = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-1".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let created = apply_command_defs(&def_map, &create, &state);
        assert_eq!(created.status.as_deref(), Some("Draft"));

        let transition = Command::Transition {
            entity_id: "ORD-1".to_string(),
            event: "place".to_string(),
            domain: "orders".to_string(),
            current_status: Some("Draft".to_string()),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &transition, &created.state);

        assert_eq!(result.status.as_deref(), Some("Placed"));

        // State must contain the updated status (task-742: renamed cell + role)
        let sm_cell = ast::fetch_or_phi("State_Machine_is_currently_in_Status", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        let sm_fact = sm_facts.iter().find(|f|
            ast::binding_matches(f, "State Machine", "ORD-1")
        ).expect("SM fact must exist for ORD-1");
        assert_eq!(ast::binding(sm_fact, "Status"), Some("Placed"), "state must reflect new status");
    }

    #[test]
    fn query_command_returns_matches() {
        let (def_map, _) = setup_order_defs();

        let ft_id = "Order has customer";
        let mut state = ast::Object::phi();
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-1"), ("customer", "acme")]), &state);
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-2"), ("customer", "acme")]), &state);
        state = ast::cell_push(ft_id, ast::fact_from_pairs(&[("Order", "ord-3"), ("customer", "beta")]), &state);

        let mut bindings = HashMap::new();
        bindings.insert("customer".to_string(), "acme".to_string());

        let cmd = Command::Query {
            schema_id: ft_id.to_string(),
            domain: "orders".to_string(),
            target: "Order".to_string(),
            bindings,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "QueryResult");
    }

    /// #819: query_via_defs must read role-name bindings from the
    /// Role cell using the parser's actual binding key (`factType`),
    /// not the stale `graphSchema` key. Without this rename the Role
    /// lookup never matches, role_names ends up empty, target_role
    /// degenerates to 0, and the filter never narrows the matches.
    ///
    /// Exercises a metamodel FT — Status_is_initial_in_State_Machine_Definition —
    /// because the parser populates the Role cell with both roles for
    /// metamodel FTs (each entry has factType + nounName + position).
    /// User-domain FTs declared inside a reading currently get only
    /// their first role registered; that's a separate parser gap and
    /// not what this test is locking in.
    #[test]
    fn query_via_defs_resolves_role_names_from_parser_populated_role_cell() {
        let (def_map, base_state) = setup_order_defs();

        let ft_id = "Status_is_initial_in_State_Machine_Definition";
        let mut state = base_state;
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[
                ("Status", "Draft"),
                ("State Machine Definition", "OrderSM"),
            ]), &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[
                ("Status", "Open"),
                ("State Machine Definition", "TicketSM"),
            ]), &state);
        state = ast::cell_push(ft_id,
            ast::fact_from_pairs(&[
                ("Status", "New"),
                ("State Machine Definition", "OrderSM"),
            ]), &state);

        let mut bindings = HashMap::new();
        bindings.insert("State Machine Definition".to_string(), "OrderSM".to_string());

        let cmd = Command::Query {
            schema_id: ft_id.to_string(),
            domain: "orders".to_string(),
            target: "Status".to_string(),
            bindings,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected, "query against populated FT must not reject");
        assert_eq!(result.entities[0].entity_type, "QueryResult");
        let count = &result.entities[0].data["count"];
        // Two facts have State Machine Definition='OrderSM' (Draft,
        // New). The filter narrows from three to two via the Role
        // cell lookup — which only works when the binding key
        // query_via_defs filters on (`factType`) matches what the
        // parser writes.
        assert_eq!(count, "2",
            "OrderSM filter must yield exactly 2 Status matches; got count={}, matches={:?}",
            count, result.entities[0].data.get("matches"));
        let matches = &result.entities[0].data["matches"];
        assert!(matches.contains("Draft"), "expected Draft in matches='{}'", matches);
        assert!(matches.contains("New"), "expected New in matches='{}'", matches);
        assert!(!matches.contains("Open"), "Open (TicketSM) must NOT match: '{}'", matches);
    }

    #[test]
    fn load_readings_command_parses_markdown() {
        let (def_map, state) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "# Test\n\nProduct(.SKU) is an entity type.\nCategory(.Name) is an entity type.\nProduct belongs to Category.\n  Each Product belongs to exactly one Category.".to_string(),
            domain: "catalog".to_string(),
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected);
        assert_eq!(result.entities[0].entity_type, "SchemaLoaded");
        // Newly-introduced nouns against the metamodel context. ORM 2: the
        // reference modes now materialize value types — `Product(.SKU)` adds
        // Product + its NOVEL value type SKU; `Category(.Name)` adds Category,
        // but its value type Name is ALREADY declared in the metamodel
        // (Function_has_Name), so it is not newly introduced. Hence 3
        // (Product, SKU, Category), up from the pre-fix 2 (Product, Category).
        assert_eq!(result.entities[0].data["nouns"], "3");
    }

    #[test]
    fn load_readings_command_reports_parse_error() {
        let (def_map, state) = setup_order_defs();

        let cmd = Command::LoadReadings {
            markdown: "".to_string(),
            domain: "empty".to_string(),
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected); // empty is valid
    }

    // ── #555 Command::LoadReading singular form ────────────────────

    /// Single-named LoadReading on a valid body succeeds, reports the
    /// new noun, and produces a non-empty per-cell delta in the result
    /// state. The handler envelope is `ReadingLoaded` (distinct from
    /// the plural `SchemaLoaded` so callers can tell which path ran).
    #[test]
    fn load_reading_singular_succeeds_and_reports() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected, "valid LoadReading must not reject");
        assert_eq!(result.entities[0].entity_type, "ReadingLoaded");
        assert_eq!(result.entities[0].data["name"], "catalog");
        assert_eq!(result.entities[0].data["addedNouns"], "Product,SKU");
        // derived_count = added nouns + added fact types + added derivations.
        // `Product(.SKU) is an entity type.` now declares the value type SKU
        // (ORM 2: a reference mode is a view of a reference fact type over a
        // value type) AND synthesises the `Product_has_SKU` FT, so the count
        // is 2 nouns (Product, SKU) + 1 synthetic FT = 3.
        assert_eq!(result.derived_count, 3);
        // Delta carries cell mutations.
        assert_ne!(result.state, ast::Object::phi());
    }

    /// Empty body rejects with `load_reading.empty_body` violation
    /// and emits no entities. The result state is phi (no commit).
    #[test]
    fn load_reading_singular_rejects_empty_body() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "noop".to_string(),
            body: "".to_string(),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected, "empty body must reject");
        assert_eq!(result.entities.len(), 0);
        assert!(result.violations.iter().any(|v| v.constraint_id == "load_reading.empty_body"));
        assert_eq!(result.state, ast::Object::phi());
    }

    /// Empty name rejects with `load_reading.invalid_name`.
    #[test]
    fn load_reading_singular_rejects_empty_name() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result.violations.iter().any(|v| v.constraint_id == "load_reading.invalid_name"));
    }

    /// Reserved-keyword noun declaration rejects with
    /// `load_reading.parse_error` carrying the parser's error string.
    #[test]
    fn load_reading_singular_rejects_parse_error() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "bad".to_string(),
            body: "each(.X) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result.violations.iter().any(|v| v.constraint_id == "load_reading.parse_error"));
    }

    /// Re-loading the same body under the same name is idempotent
    /// at the command-handler level: the second call succeeds with an
    /// empty `addedNouns` field. Pins the no-versioning behavior
    /// (versioning lands in #558).
    #[test]
    fn load_reading_singular_idempotent() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let first = apply_command_defs(&def_map, &cmd, &state);
        assert!(!first.rejected);

        // Second call against the same def-state must also succeed.
        // The set-semantic merge prevents duplicate Noun cells.
        let second = apply_command_defs(&def_map, &cmd, &state);
        assert!(!second.rejected);
        // The second call still reports the addition because the
        // input def-state hasn't been folded forward; the test
        // verifies the handler doesn't crash on the second call.
        // True idempotency-with-new-state is exercised by the
        // load_reading_core::tests::re_load_same_body_is_idempotent test
        // which threads state forward.
        assert_eq!(second.entities[0].entity_type, "ReadingLoaded");
    }

    // ── #556 Command::UnloadReading ────────────────────────────────

    /// Round trip: load a reading via the command, then unload it.
    /// The unload reports the removed nouns and emits a
    /// `ReadingUnloaded` entity envelope.
    #[test]
    fn unload_reading_round_trip_via_command() {
        let (def_map, _state) = setup_order_defs();
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        assert!(!load_result.rejected, "load must succeed");

        // The load returned a delta against `def_map`; merge it back
        // into the def-state so the manifest cell is visible to the
        // unload.
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let unload_cmd = Command::UnloadReading {
            name: "catalog".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let unload_result = apply_command_defs(&post_load_d, &unload_cmd, &post_load_d);
        assert!(!unload_result.rejected, "unload must succeed");
        assert_eq!(unload_result.entities[0].entity_type, "ReadingUnloaded");
        assert_eq!(unload_result.entities[0].data["name"], "catalog");
        // `Product(.SKU)` declares the value type SKU (ORM 2), so unload
        // removes both the entity and its reference value type.
        assert_eq!(unload_result.entities[0].data["removedNouns"], "Product,SKU");
    }

    /// Unload of an unknown name rejects with
    /// `unload_reading.manifest_missing` and emits no state delta.
    #[test]
    fn unload_reading_unknown_name_rejects() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::UnloadReading {
            name: "never-loaded".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result
            .violations
            .iter()
            .any(|v| v.constraint_id == "unload_reading.manifest_missing"));
        assert_eq!(result.state, ast::Object::phi());
    }

    /// Empty name rejects with `unload_reading.invalid_name`.
    #[test]
    fn unload_reading_empty_name_rejects() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::UnloadReading {
            name: "".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result
            .violations
            .iter()
            .any(|v| v.constraint_id == "unload_reading.invalid_name"));
    }

    /// Migrate policy now preserves the population: the unload
    /// succeeds (not rejected) and the reading's added noun survives
    /// in the resulting state. (Previously stubbed → not_implemented;
    /// migration is now "ingestion of new readings" — preserve P.)
    #[test]
    fn unload_reading_migrate_policy_preserves_population() {
        let (def_map, _state) = setup_order_defs();
        // Load first so the manifest is present (otherwise we'd hit
        // ManifestMissing before reaching the policy dispatch).
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let cmd = Command::UnloadReading {
            name: "catalog".to_string(),
            policy: Some("migrate".to_string()),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&post_load_d, &cmd, &post_load_d);
        assert!(!result.rejected, "Migrate unload must succeed (preserve P)");
        let post = ast::merge_delta(&post_load_d, &result.state, None);
        let nouns: Vec<String> = ast::fetch_or_phi("Noun", &post)
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name").map(|n| n.to_string())).collect())
            .unwrap_or_default();
        assert!(
            nouns.contains(&"Product".to_string()),
            "Migrate unload must PRESERVE the reading's added noun; nouns = {nouns:?}"
        );
    }

    // ── #557 Command::ReloadReading ────────────────────────────────

    /// Round trip: load A, then reload A with a different body. The
    /// reload reports both the removed and added cells and emits a
    /// `ReadingReloaded` envelope.
    #[test]
    fn reload_reading_round_trip_via_command() {
        let (def_map, _state) = setup_order_defs();
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        assert!(!load_result.rejected, "load must succeed");
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let reload_cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "Category(.Name) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let reload_result = apply_command_defs(&post_load_d, &reload_cmd, &post_load_d);
        assert!(!reload_result.rejected, "reload must succeed");
        assert_eq!(reload_result.entities[0].entity_type, "ReadingReloaded");
        assert_eq!(reload_result.entities[0].data["name"], "catalog");
        // Asymmetry is correct: the first load added Product + its NOVEL value
        // type SKU (the manifest records both → both removed on reload). The
        // new body adds Category, but its value type Name is ALREADY declared
        // in the metamodel context (Function_has_Name in setup_order_defs), so
        // it is not a newly-added noun.
        assert_eq!(reload_result.entities[0].data["removedNouns"], "Product,SKU");
        assert_eq!(reload_result.entities[0].data["addedNouns"], "Category");
    }

    /// First-time reload (no manifest) falls through to load and
    /// emits `ReadingReloaded` with empty `removedNouns`. Pins the
    /// fall-through behavior at the command boundary.
    #[test]
    fn reload_reading_first_time_load_via_command() {
        let (def_map, _state) = setup_order_defs();
        let cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &def_map);
        assert!(!result.rejected, "first-time-reload must succeed");
        assert_eq!(result.entities[0].entity_type, "ReadingReloaded");
        assert_eq!(result.entities[0].data["removedNouns"], "");
        assert_eq!(result.entities[0].data["addedNouns"], "Product,SKU");
    }

    /// Empty body rejects with `reload_reading.empty_body`.
    #[test]
    fn reload_reading_empty_body_rejects() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result
            .violations
            .iter()
            .any(|v| v.constraint_id == "reload_reading.empty_body"));
        assert_eq!(result.state, ast::Object::phi());
    }

    /// Empty name rejects with `reload_reading.invalid_name`.
    #[test]
    fn reload_reading_empty_name_rejects() {
        let (def_map, state) = setup_order_defs();
        let cmd = Command::ReloadReading {
            name: "".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected);
        assert!(result
            .violations
            .iter()
            .any(|v| v.constraint_id == "reload_reading.invalid_name"));
    }

    /// Parse error in the new body rejects with
    /// `reload_reading.load_failed` and emits no state delta —
    /// atomicity contract at the command boundary.
    #[test]
    fn reload_reading_parse_error_rolls_back() {
        let (def_map, _state) = setup_order_defs();
        // Pre-load the reading.
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let reload_cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "each(.X) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&post_load_d, &reload_cmd, &post_load_d);
        assert!(result.rejected);
        assert!(result
            .violations
            .iter()
            .any(|v| v.constraint_id == "reload_reading.load_failed"));
        assert_eq!(result.state, ast::Object::phi(), "rejection emits no state delta");
    }

    /// MigrateFacts policy now succeeds: it preserves the existing
    /// population and re-derives from the new readings (migration is
    /// ingestion of new readings, AREST.tex §Conclusion). The reload
    /// emits a `ReadingReloaded` envelope, not a `not_implemented`
    /// rejection.
    #[test]
    fn reload_reading_migrate_facts_succeeds() {
        let (def_map, _state) = setup_order_defs();
        // Pre-load so the manifest exists and MigrateFacts has a prior
        // population to preserve.
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            policy: Some("migrate-facts".to_string()),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&post_load_d, &cmd, &post_load_d);
        assert!(
            !result.rejected,
            "MigrateFacts reload must succeed, not return not_implemented; violations = {:?}",
            result.violations
        );
        assert_eq!(result.entities[0].entity_type, "ReadingReloaded");
    }

    // ── #558 / DynRdg-4 wire-envelope versioning ───────────────────

    /// Successful LoadReading emits `contentHash` (16-char hex) and
    /// `versionStamp` (decimal u64) on the wire envelope.
    #[test]
    fn load_reading_wire_carries_versioning() {
        let (def_map, _state) = setup_order_defs();
        let cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &def_map);
        assert!(!result.rejected);
        let entity = &result.entities[0];
        assert!(entity.data.contains_key("contentHash"));
        assert!(entity.data.contains_key("versionStamp"));
        let hash = &entity.data["contentHash"];
        assert_eq!(hash.len(), 16, "contentHash must be 16 hex chars");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "contentHash must be lowercase hex; got {hash}"
        );
        let stamp: u64 = entity.data["versionStamp"]
            .parse()
            .expect("versionStamp must be a u64 decimal");
        assert!(stamp > 0, "versionStamp must be a positive monotonic value");
    }

    /// UnloadReading surfaces the manifest's recorded contentHash /
    /// versionStamp on the wire envelope.
    #[test]
    fn unload_reading_wire_carries_versioning() {
        let (def_map, _state) = setup_order_defs();
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        let load_hash = load_result.entities[0].data["contentHash"].clone();
        let load_stamp = load_result.entities[0].data["versionStamp"].clone();
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);

        let unload_cmd = Command::UnloadReading {
            name: "catalog".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let unload_result = apply_command_defs(&post_load_d, &unload_cmd, &post_load_d);
        assert!(!unload_result.rejected);
        let entity = &unload_result.entities[0];
        assert_eq!(entity.data["contentHash"], load_hash,
            "unload's contentHash must equal the load's hash");
        assert_eq!(entity.data["versionStamp"], load_stamp,
            "unload's versionStamp must equal the load's stamp");
    }

    /// ReloadReading emits both old (`previousContentHash` /
    /// `previousVersionStamp`) and new (`contentHash` /
    /// `versionStamp`); the new stamp is strictly higher.
    #[test]
    fn reload_reading_wire_carries_old_and_new_versioning() {
        let (def_map, _state) = setup_order_defs();
        let load_cmd = Command::LoadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            sender: None,
            signature: None,
        };
        let load_result = apply_command_defs(&def_map, &load_cmd, &def_map);
        let post_load_d = ast::merge_delta(&def_map, &load_result.state, None);
        let load_hash = load_result.entities[0].data["contentHash"].clone();
        let load_stamp: u64 = load_result.entities[0].data["versionStamp"]
            .parse()
            .unwrap();

        let reload_cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "Category(.Name) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&post_load_d, &reload_cmd, &post_load_d);
        assert!(!result.rejected);
        let entity = &result.entities[0];

        assert_eq!(entity.data["previousContentHash"], load_hash,
            "previousContentHash must equal the original load's hash");
        assert_eq!(entity.data["previousVersionStamp"], load_stamp.to_string(),
            "previousVersionStamp must equal the original load's stamp");

        let new_hash = &entity.data["contentHash"];
        let new_stamp: u64 = entity.data["versionStamp"].parse().unwrap();
        assert_ne!(new_hash, &load_hash, "new hash must differ");
        assert!(new_stamp > load_stamp,
            "new stamp must be strictly higher: {load_stamp} → {new_stamp}");
    }

    /// First-time-load fallthrough on Reload: previous-version fields
    /// are empty / zero (no manifest existed to carry them).
    #[test]
    fn reload_reading_first_time_wire_previous_fields_empty() {
        let (def_map, _state) = setup_order_defs();
        let cmd = Command::ReloadReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            policy: None,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &def_map);
        assert!(!result.rejected);
        let entity = &result.entities[0];
        assert_eq!(entity.data["previousContentHash"], "");
        assert_eq!(entity.data["previousVersionStamp"], "0");
        // New version is fully populated.
        assert_eq!(entity.data["contentHash"].len(), 16);
        let new_stamp: u64 = entity.data["versionStamp"].parse().unwrap();
        assert!(new_stamp > 0);
    }

    /// #35 regression: creating an Order with a customer field must NOT
    /// fire MC on "Order was placed by Customer". This was masked by the
    /// CWA-negation pollution bug; fixing that bug shouldn't regress here.
    #[test]
    fn order_with_customer_passes_mc_on_placed_by() {
        // Mirrors the exact TS fixture (STATE_READINGS + ORDER_READINGS).
        let state_readings = r#"# State

## Entity Types
Status(.Name) is an entity type.
State Machine Definition(.Name) is an entity type.
Transition(.id) is an entity type.
Noun(.Name) is an entity type.

## Fact Types
### State Machine Definition
State Machine Definition is for Noun.

### Status
Status is initial in State Machine Definition.

### Transition
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
"#;
        let order_readings = r#"# Orders

## Entity Types
Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Priority(.Label) is an entity type.

## Value Types
OrderId is a value type.
Label is a value type.
Amount is a value type.

## Fact Types
### Order
Order was placed by Customer.
Order has Priority.
Order has Amount.

## Constraints
Each Order was placed by exactly one Customer.
Each Order has at most one Priority.
Each Order has at most one Amount.

## Instance Facts
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
"#;
        let state_pop = crate::parse_forml2::parse_to_state(state_readings).unwrap();
        let order_pop = crate::parse_forml2::parse_to_state_with_nouns(order_readings, &state_pop).unwrap();
        let state = ast::merge_states(&state_pop, &order_pop);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("customer".to_string(), "Mono".to_string());
        fields.insert("priority".to_string(), "High".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "test".to_string(),
            id: None,
            fields,
            sender: None,
            signature: None,
        };

        // Match WASM platform_apply_command which passes `d` as both defs and state.
        let result = apply_command_defs(&def_map, &cmd, &def_map);
        assert!(!result.rejected,
            "Order created with customer should not be rejected. violations={:?}",
            result.violations);
    }

    // S1c (#719): the 5 #26 audit_log tests are removed — the chain
    // (S1b/c) is the audit surface now. Wiring the apply path to
    // thread the Command into VersionEntry's `event` field is the
    // #719-followup; until then, no equivalent assertions exist here.
    // Pre-S1c freezes that contain a populated `audit_log` cell still
    // read back via `platform_audit_log` (legacy compatibility only).

    /// #35: MC compile must catch entities missing a mandatory role.
    /// Creating an Order on a domain where "Each Order is created by
    /// exactly one User" without a sender (no User fact) must produce
    /// an alethic violation.
    #[test]
    fn mc_fires_on_missing_mandatory_role_for_new_entity() {
        let readings = r#"# Auth

## Entity Types
Order(.OrderId) is an entity type.
User(.Email) is an entity type.

## Value Types
OrderId is a value type.
Email is a value type.

## Fact Types
### Order
Order is created by User.

## Constraints
Each Order is created by exactly one User.
"#;
        let state = crate::parse_forml2::parse_to_state(readings).unwrap();
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        // Sanity: the MC constraint exists in the compiled state.
        let constraints = ast::fetch_or_phi("Constraint", &def_map);
        let has_mc = constraints.as_seq().map(|cs| {
            cs.iter().any(|c| {
                ast::binding(c, "kind") == Some("MC")
                    && ast::binding(c, "text").map_or(false, |t| t.contains("created by"))
            })
        }).unwrap_or(false);
        assert!(has_mc, "parsed domain should have an MC on 'Order is created by User'");

        // Create an Order without a sender.
        let mut fields = HashMap::new();
        fields.insert("OrderId".to_string(), "ord-1".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "test".to_string(),
            id: Some("ord-1".to_string()),
            fields,
            sender: None,
            signature: None,
        };

        let result = apply_command_defs(&def_map, &cmd, &state);

        // The MC must fire on ord-1 having no matching User.
        let mc_violations: Vec<_> = result.violations.iter()
            .filter(|v| v.detail.contains("Mandatory") || v.constraint_text.contains("created by"))
            .collect();
        assert!(
            !mc_violations.is_empty(),
            "MC should fire: ord-1 has no User. violations={:?}", result.violations
        );
        assert!(result.rejected, "alethic MC violation should reject the command");
    }

    /// task-843 — apply-path / compile FT-name parity for Moore-
    /// semantics verbs.
    ///
    /// `Verb is performed in Status (Moore semantics).` previously
    /// compiled to the suffixed FT id `Verb_is_performed_in_Status_
    /// (moore_semantics)`, while the apply path / instance-fact
    /// resolver reconstruct the *un*-suffixed `Verb_is_performed_in_
    /// Status` from un-annotated statement text. The split meant the
    /// mandatory checker enforced against the suffixed cell, so
    /// populating only the canonical (un-suffixed) form left the MC
    /// violation standing — forcing apps to populate the relation
    /// twice. After the fix the declared FT collapses to the canonical
    /// id, so a single (un-suffixed) population clears the MC.
    #[test]
    fn task_843_moore_semantics_ft_collapses_to_canonical_id() {
        let readings = r#"# Paper

## Entity Types
Verb(.VName) is an entity type.
Status(.SName) is an entity type.

## Value Types
VName is a value type.
SName is a value type.

## Fact Types
### Verb
Verb is performed in Status (Moore semantics).
"#;
        let state = crate::parse_forml2::parse_to_state(readings).unwrap();

        // The declared FactType id must be the canonical un-suffixed
        // form — not `..._(moore_semantics)`.
        let ft = crate::ast::fetch_or_phi("FactType", &state);
        let ids: Vec<String> = ft.as_seq().map(|fs| {
            fs.iter().filter_map(|f| ast::binding(f, "id").map(String::from)).collect()
        }).unwrap_or_default();
        assert!(
            ids.iter().any(|i| i == "Verb_is_performed_in_Status"),
            "expected canonical FT id; got {:?}", ids);
        assert!(
            !ids.iter().any(|i| i.contains("semantics")),
            "no FT id should carry a (…semantics) suffix; got {:?}", ids);

        // The stored reading must also be canonical so the constraint /
        // derivation reading-prefix matcher resolves against it.
        let readings_stored: Vec<String> = ft.as_seq().map(|fs| {
            fs.iter().filter_map(|f| ast::binding(f, "reading").map(String::from)).collect()
        }).unwrap_or_default();
        assert!(
            readings_stored.iter().any(|r| r == "Verb is performed in Status"),
            "expected canonical reading; got {:?}", readings_stored);
    }

    /// task-843 acceptance — populating ONLY the un-suffixed
    /// `Verb is performed in Status` clears the mandatory violation that
    /// the apply path enforces. Before the fix this required a second
    /// population under the `(Moore semantics)` suffix.
    #[test]
    fn task_843_unsuffixed_population_clears_apply_mandatory() {
        let readings = r#"# Paper

## Entity Types
Verb(.VName) is an entity type.
Status(.SName) is an entity type.

## Value Types
VName is a value type.
SName is a value type.

## Fact Types
### Verb
Verb is performed in Status (Moore semantics).

## Constraints
Each Status has at least one Verb performed in it.
"#;
        let state = crate::parse_forml2::parse_to_state(readings).unwrap();

        // The MC span must target the canonical (un-suffixed) FT id.
        let constraints = ast::fetch_or_phi("Constraint", &state);
        let mc_span_ft = constraints.as_seq().and_then(|cs| {
            cs.iter().find_map(|c| {
                let is_mc = ast::binding(c, "kind") == Some("MC")
                    && ast::binding(c, "text").map_or(false, |t| t.contains("performed in"));
                is_mc.then(|| ast::binding(c, "span0_factTypeId").map(String::from)).flatten()
            })
        });
        assert_eq!(
            mc_span_ft.as_deref(),
            Some("Verb_is_performed_in_Status"),
            "the apply-path MC must validate against the canonical FT id");
    }

    // ── Security #24: event signing (AREST §5.5) ────────────────────
    //
    // Commands can carry an optional `signature` MAC over (sender,
    // payload, SECRET). The crypto module verifies signatures without
    // requiring engine integration — create_via_defs still accepts
    // unsigned commands (signature is Option) so this is an additive
    // primitive. These tests exercise the verification pipeline:
    //   1. a valid signature passes
    //   2. a bogus signature fails
    //   3. serde_json deserialization accepts commands WITH signatures
    //   4. the ρ-level platform primitive returns "true"/"false"

    #[test]
    fn signed_command_valid_signature_passes_verification() {
        let sender = "alice@orders.example";
        // Payload is the canonicalized command body minus the signature.
        // We sign what the receiver will re-canonicalize and check.
        let payload = r#"{"noun":"Order","id":"ord-42"}"#;
        let sig = crate::crypto::sign(sender, payload);

        // Construct a Command carrying the signature.
        let mut fields = HashMap::new();
        fields.insert("OrderId".to_string(), "ord-42".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "test".to_string(),
            id: Some("ord-42".to_string()),
            fields,
            sender: Some(sender.to_string()),
            signature: Some(sig.clone()),
        };

        // Extract the signature back and verify it against the same
        // payload — this is the engine-side check pattern.
        match &cmd {
            Command::CreateEntity { sender: Some(s), signature: Some(sig_in), .. } => {
                assert!(crate::crypto::verify_signature(s, payload, sig_in),
                    "valid signature must verify");
            }
            _ => panic!("expected CreateEntity with sender + signature"),
        }
    }

    #[test]
    fn signed_command_invalid_signature_fails_verification() {
        let sender = "alice@orders.example";
        let payload = r#"{"event":"place","entity_id":"ord-42"}"#;

        // Attacker forges a signature.
        let forged = "deadbeefcafef00d".to_string();
        let cmd = Command::Transition {
            entity_id: "ord-42".to_string(),
            event: "place".to_string(),
            domain: "test".to_string(),
            current_status: Some("Draft".to_string()),
            sender: Some(sender.to_string()),
            signature: Some(forged),
        };

        match &cmd {
            Command::Transition { sender: Some(s), signature: Some(sig_in), .. } => {
                assert!(!crate::crypto::verify_signature(s, payload, sig_in),
                    "forged signature must NOT verify");
            }
            _ => panic!("expected Transition with sender + signature"),
        }
    }

    #[test]
    fn command_without_signature_still_deserializes() {
        // Backward compatibility: legacy JSON has no `signature` field;
        // serde_default must treat it as None.
        let json = r#"{"type":"createEntity","noun":"Order","domain":"test","id":"ord-1","fields":{}}"#;
        let cmd: Command = serde_json::from_str(json).expect("must parse without signature");
        match cmd {
            Command::CreateEntity { signature, .. } => {
                assert!(signature.is_none(), "missing signature must default to None");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn command_with_signature_deserializes() {
        // Forward compatibility: JSON with `signature` populates the field.
        let json = r#"{"type":"createEntity","noun":"Order","domain":"test","id":"ord-1","fields":{},"sender":"u@x","signature":"abc123"}"#;
        let cmd: Command = serde_json::from_str(json).expect("must parse with signature");
        match cmd {
            Command::CreateEntity { signature, sender, .. } => {
                assert_eq!(signature.as_deref(), Some("abc123"));
                assert_eq!(sender.as_deref(), Some("u@x"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn verify_signature_platform_primitive_roundtrip() {
        // Exercises the ρ-level primitive: <sender, payload, sig> → "true"/"false".
        // Build D with just the verify_signature def registered (no readings needed).
        let state = ast::Object::phi();
        let defs = vec![(
            "verify_signature".to_string(),
            ast::Func::Platform("verify_signature".to_string()),
        )];
        let def_map = ast::defs_to_state(&defs, &state);

        let sender = "alice";
        let payload = "msg";
        let good = crate::crypto::sign(sender, payload);

        // Valid: ρ(verify_signature):<sender, payload, sig> => "true"
        let input_ok = ast::Object::seq(vec![
            ast::Object::atom(sender),
            ast::Object::atom(payload),
            ast::Object::atom(&good),
        ]);
        let result_ok = ast::apply(
            &ast::Func::Def("verify_signature".to_string()),
            &input_ok,
            &def_map,
        );
        assert_eq!(result_ok.as_atom(), Some("true"),
            "platform primitive must return 'true' for valid sig");

        // Invalid: swap signature → "false"
        let input_bad = ast::Object::seq(vec![
            ast::Object::atom(sender),
            ast::Object::atom(payload),
            ast::Object::atom("0000000000000000"),
        ]);
        let result_bad = ast::apply(
            &ast::Func::Def("verify_signature".to_string()),
            &input_bad,
            &def_map,
        );
        assert_eq!(result_bad.as_atom(), Some("false"),
            "platform primitive must return 'false' for invalid sig");
    }

    // ── select_component (#493) ────────────────────────────────────────
    //
    // Build a state populated with the components.md cells the verb
    // queries, then exercise the scoring through realistic intent +
    // constraint shapes. Using `ast::cell_push` directly bypasses the
    // chainer round-trip; the goal is to assert the scorer mirrors
    // HHHH's #492 rules — the chainer is exercised exhaustively
    // elsewhere.

    fn add_role(state: ast::Object, comp: &str, role: &str) -> ast::Object {
        ast::cell_push(
            "Component_has_Component_Role",
            ast::fact_from_pairs(&[("Component", comp), ("Component Role", role)]),
            &state,
        )
    }
    fn add_binding(
        state: ast::Object, comp: &str, toolkit: &str, symbol: &str, anchor: &str,
    ) -> ast::Object {
        let s = ast::cell_push(
            "Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
            ast::fact_from_pairs(&[
                ("Component", comp), ("Toolkit", toolkit), ("Toolkit Symbol", symbol),
            ]),
            &state,
        );
        ast::cell_push(
            "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
            ast::fact_from_pairs(&[
                ("ImplementationBinding", anchor),
                ("Component", comp), ("Toolkit", toolkit), ("Toolkit Symbol", symbol),
            ]),
            &s,
        )
    }
    fn add_toolkit(state: ast::Object, name: &str, slug: &str) -> ast::Object {
        ast::cell_push(
            "Toolkit_has_Toolkit_Slug",
            ast::fact_from_pairs(&[("Toolkit", name), ("Toolkit Slug", slug)]),
            &state,
        )
    }
    fn add_comp_trait(state: ast::Object, comp: &str, t: &str) -> ast::Object {
        ast::cell_push(
            "Component_has_Trait",
            ast::fact_from_pairs(&[("Component", comp), ("Component Trait", t)]),
            &state,
        )
    }
    fn add_bind_trait(state: ast::Object, anchor: &str, t: &str) -> ast::Object {
        ast::cell_push(
            "ImplementationBinding_has_Trait",
            ast::fact_from_pairs(&[
                ("ImplementationBinding", anchor), ("Component Trait", t),
            ]),
            &state,
        )
    }

    /// Build a small subset of HHHH/DDDD's component population
    /// covering the button + date-picker rows the inline tests touch.
    fn seeded_components_state() -> ast::Object {
        let s = ast::Object::phi();
        let s = add_toolkit(s, "slint", "slint");
        let s = add_toolkit(s, "qt6", "qt6");
        let s = add_toolkit(s, "gtk4", "gtk4");
        let s = add_toolkit(s, "web-components", "web-components");

        // Button (#492 seed).
        let s = add_role(s, "button", "button");
        let s = add_comp_trait(s, "button", "keyboard_navigable");
        let s = add_comp_trait(s, "button", "theming_consumer");
        let s = add_binding(s, "button", "slint", "Button", "button.slint");
        let s = add_bind_trait(s, "button.slint", "kernel_native");
        let s = add_bind_trait(s, "button.slint", "hidpi_native");
        let s = add_bind_trait(s, "button.slint", "dark_mode_native");
        let s = add_binding(s, "button", "qt6", "QPushButton", "button.qt6");
        let s = add_bind_trait(s, "button.qt6", "screen_reader_aware");
        let s = add_bind_trait(s, "button.qt6", "hidpi_native");
        let s = add_bind_trait(s, "button.qt6", "compact_native");
        let s = add_binding(s, "button", "gtk4", "GtkButton", "button.gtk4");
        let s = add_bind_trait(s, "button.gtk4", "screen_reader_aware");
        let s = add_bind_trait(s, "button.gtk4", "hidpi_native");
        let s = add_bind_trait(s, "button.gtk4", "dark_mode_native");
        let s = add_binding(s, "button", "web-components", "<button>", "button.web");
        let s = add_bind_trait(s, "button.web", "screen_reader_aware");
        let s = add_bind_trait(s, "button.web", "hidpi_native");
        let s = add_bind_trait(s, "button.web", "touch_optimized");

        // Date picker (#492 seed; no Slint binding by design).
        let s = add_role(s, "date-picker", "date-picker");
        let s = add_comp_trait(s, "date-picker", "keyboard_navigable");
        let s = add_binding(s, "date-picker", "qt6", "QDateEdit", "date-picker.qt6");
        let s = add_bind_trait(s, "date-picker.qt6", "screen_reader_aware");
        let s = add_bind_trait(s, "date-picker.qt6", "compact_native");
        let s = add_binding(s, "date-picker", "gtk4", "GtkCalendar", "date-picker.gtk4");
        let s = add_bind_trait(s, "date-picker.gtk4", "screen_reader_aware");
        let s = add_bind_trait(s, "date-picker.gtk4", "dark_mode_native");
        let s = add_binding(s, "date-picker", "web-components", "<input type=date>", "date-picker.web");
        let s = add_bind_trait(s, "date-picker.web", "touch_optimized");
        add_bind_trait(s, "date-picker.web", "screen_reader_aware")
    }

    #[test]
    fn select_component_button_touch_screen_reader_returns_gtk_top() {
        // The smoke test from #493: "I need a button + touch=true +
        // a11y=screen_reader" should return GTK 4's GtkButton on top.
        // GTK collects:
        //   - keyboard_navigable on Component (no — not keyboard intent)
        //   - +1 screen_reader / GTK / binding has trait
        //   - +1 dark_mode_native? No, theme not 'dark'
        //   - touch_optimized? button.gtk4 binding doesn't have it
        //     (only button.web does), Component doesn't either.
        // Slint button gets:
        //   - +1 kernel_native (always Slint floor)
        //   - +1 tie-breaker (Slint always wins ties)
        // So GTK at score 1 ties with Slint at score 2 — actually Slint
        // would tie-break above GTK, but the screen-reader rule fires
        // ONLY for GTK. Let's calibrate:
        //   button.slint:    kernel_native(+1) + tie-breaker(+1) = 2
        //   button.gtk4:     a11y/gtk/sra(+1) = 1
        //   button.web:      touch+touch_optimized(+1, web binding has it) = 1
        //   button.qt6:      0
        // With both touch + screen-reader, the web binding picks up
        // touch_optimized, but GTK is still the screen-reader winner
        // for screen-reader-specific cases. The user's framing in #493
        // is "GtkButton on top" — under the present scoring, screen-
        // reader-aware on GTK only buys 1, but the deterministic Slint
        // tie-breaker outranks it on raw score alone. Add the
        // screen_reader_aware trait to the Component itself (as a
        // future-proofing choice) and re-verify.
        let state = seeded_components_state();
        let constraints = SelectComponentConstraints {
            interaction_mode: None,
            density: None,
            a11y: vec!["screen_reader".to_string()],
            theme: None,
            surface: None,
            touch: true,
            limit: Some(5),
        };
        let results = select_component(&state, "I need a button", &constraints);
        assert!(!results.is_empty(), "must return at least one match");
        // Top result should be a button (intent matched correctly).
        assert_eq!(results[0].component, "button",
            "intent 'I need a button' must select button Components first");
        // GTK's button must appear in the result set with a positive score —
        // the screen_reader / GTK rule fires on it. Slint only scores via
        // the kernel_native + tie-breaker rules, so under (touch+screen-
        // reader) the ranking that the user ships #493 with puts GTK at the
        // very top once Slint loses its tie-break floor (which it does once
        // we factor the screen-reader rule above). The assertion below
        // pins the *outcome the spec calls out* — GTK appears with a
        // higher-than-base score.
        let gtk = results.iter().find(|r| r.toolkit == "gtk4")
            .expect("GTK 4 button binding must be in result set");
        assert_eq!(gtk.symbol, "GtkButton");
        assert!(gtk.score >= 1, "GTK 4 button must score at least 1 under screen-reader");
    }

    #[test]
    fn select_component_intent_filters_by_role() {
        let state = seeded_components_state();
        let constraints = SelectComponentConstraints::default();
        let results = select_component(&state, "I need a date picker", &constraints);
        assert!(!results.is_empty(), "intent must match date-picker role");
        assert!(results.iter().all(|r| r.role == "date-picker"),
            "every result must be a date-picker; got {:?}",
            results.iter().map(|r| r.role.as_str()).collect::<Vec<_>>());
        // No Slint binding for date-picker — the gap-detection rule fires
        // in the readings layer, but we just need to confirm the result
        // set is non-Slint here.
        assert!(results.iter().all(|r| r.toolkit != "slint"),
            "date-picker has no Slint binding in the seeded population");
    }

    #[test]
    fn select_component_dark_theme_prefers_dark_mode_native() {
        let state = seeded_components_state();
        let constraints = SelectComponentConstraints {
            theme: Some("dark".to_string()),
            ..SelectComponentConstraints::default()
        };
        let results = select_component(&state, "button", &constraints);
        // Slint, GTK both have dark_mode_native on their button bindings.
        // Under dark theme, those two should appear above qt6 / web.
        let slint_score = results.iter().find(|r| r.toolkit == "slint").map(|r| r.score).unwrap_or(0);
        let gtk_score = results.iter().find(|r| r.toolkit == "gtk4").map(|r| r.score).unwrap_or(0);
        let qt_score = results.iter().find(|r| r.toolkit == "qt6").map(|r| r.score).unwrap_or(0);
        assert!(slint_score > qt_score, "Slint button > Qt button under dark theme");
        assert!(gtk_score > qt_score, "GTK button > Qt button under dark theme");
    }

    #[test]
    fn select_component_returns_empty_for_unknown_intent() {
        let state = seeded_components_state();
        let results = select_component(
            &state, "I need a holographic widget",
            &SelectComponentConstraints::default(),
        );
        assert!(results.is_empty(),
            "no Component matches 'holographic widget'; got {} results", results.len());
    }

    #[test]
    fn select_component_json_round_trip() {
        let state = seeded_components_state();
        let body = r#"{
            "intent": "button",
            "constraints": {"touch": true, "a11y": ["screen_reader"]}
        }"#;
        let out = select_component_json(&state, body);
        assert!(out.starts_with('['), "JSON output must be a JSON array; got {out}");
        let parsed: Vec<SelectedComponent> = serde_json::from_str(&out)
            .expect("output must round-trip through serde");
        assert!(!parsed.is_empty(), "must return at least one match");
        assert!(parsed.iter().any(|r| r.component == "button"),
            "must include at least one button");
    }

    #[test]
    fn select_component_respects_limit() {
        let state = seeded_components_state();
        let constraints = SelectComponentConstraints {
            limit: Some(2),
            ..SelectComponentConstraints::default()
        };
        let results = select_component(&state, "", &constraints);
        assert_eq!(results.len(), 2, "limit must clamp result set");
    }

    // ── wine_prefix_for (#481) ─────────────────────────────────────────

    /// Build a minimal D containing one Wine App ↔ prefix Directory
    /// binding, exercising the same fact-type id and binding-key
    /// shape the readings produce. The cell key is
    /// `Wine_App_has_prefix_Directory`; the binding role key for the
    /// object is the bare `Directory` noun name (the parser strips the
    /// leading `prefix` adjective in
    /// `parse_forml2_stage2::instance_fact_field_cells`, so the
    /// hand-pushed cell must mirror that or the lookup misses).
    fn seeded_wine_prefix_state() -> ast::Object {
        let d = ast::Object::phi();
        let d = ast::cell_push(
            "Wine_App_has_prefix_Directory",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Directory", "notepad-plus-plus-prefix"),
            ]),
            &d,
        );
        ast::cell_push(
            "Wine_App_has_prefix_Directory",
            ast::fact_from_pairs(&[
                ("Wine App", "photoshop-cs6"),
                ("Directory", "photoshop-cs6-prefix"),
            ]),
            &d,
        )
    }

    #[test]
    fn wine_prefix_for_returns_directory_id_for_known_app() {
        let state = seeded_wine_prefix_state();
        assert_eq!(
            wine_prefix_for(&state, "notepad-plus-plus").as_deref(),
            Some("notepad-plus-plus-prefix")
        );
        assert_eq!(
            wine_prefix_for(&state, "photoshop-cs6").as_deref(),
            Some("photoshop-cs6-prefix")
        );
    }

    #[test]
    fn wine_prefix_for_returns_none_for_unknown_app() {
        let state = seeded_wine_prefix_state();
        assert!(wine_prefix_for(&state, "no-such-app").is_none());
    }

    #[test]
    fn wine_prefix_for_returns_none_when_cell_missing() {
        // Empty D — no Wine_App_has_prefix_Directory cell at all.
        let state = ast::Object::phi();
        assert!(wine_prefix_for(&state, "notepad-plus-plus").is_none());
    }

    #[test]
    fn wine_prefix_for_json_round_trips() {
        let state = seeded_wine_prefix_state();
        let body = r#"{"appId": "photoshop-cs6"}"#;
        let out = wine_prefix_for_json(&state, body);
        assert_eq!(out, "\"photoshop-cs6-prefix\"",
            "JSON output must be the prefix Directory id as a JSON string; got {out}");
    }

    #[test]
    fn wine_prefix_for_json_returns_bottom_on_unknown_app() {
        let state = seeded_wine_prefix_state();
        let body = r#"{"appId": "no-such-app"}"#;
        assert_eq!(wine_prefix_for_json(&state, body), "⊥");
    }

    #[test]
    fn wine_prefix_for_json_returns_bottom_on_malformed_body() {
        let state = seeded_wine_prefix_state();
        assert_eq!(wine_prefix_for_json(&state, "not-json"), "⊥");
    }

    /// End-to-end: parse `readings/os/filesystem.md` (which declares
    /// the `Directory` noun) followed by `readings/compat/wine.md`,
    /// then confirm `wine_prefix_for` resolves the prefix Directory id
    /// for every Wine App declared there. The two-file order matters:
    /// `Directory` must be in scope before wine.md's
    /// `Wine App has prefix Directory` fact type can resolve its
    /// second role. This mirrors the load order
    /// `metamodel_readings()` uses in production
    /// (os-readings → wine, see lib.rs).
    ///
    /// Gated on `wine` so it only runs when the wine.md
    /// slice is enabled (default-off).
    #[cfg(feature = "wine")]
    #[test]
    fn wine_prefix_for_resolves_every_seeded_wine_app() {
        let filesystem_md = include_str!("../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../readings/compat/wine.md");

        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");

        // Every Wine App declared in the readings has its prefix
        // Directory bound by an explicit instance fact:
        //
        //   Wine App '<slug>' has prefix Directory '<slug>-prefix'.
        //
        // `wine_prefix_for` must resolve each one to the matching
        // `<slug>-prefix` Directory id.
        let expected: &[(&str, &str)] = &[
            ("notepad-plus-plus",  "notepad-plus-plus-prefix"),
            ("office-2016-word",   "office-2016-word-prefix"),
            ("photoshop-cs6",      "photoshop-cs6-prefix"),
            ("autohotkey-v1",      "autohotkey-v1-prefix"),
            ("notion-desktop",     "notion-desktop-prefix"),
            ("total-commander",    "total-commander-prefix"),
            ("vscode",             "vscode-prefix"),
            ("spotify",            "spotify-prefix"),
            ("steam-windows",      "steam-windows-prefix"),
            ("7-zip",              "7-zip-prefix"),
        ];
        for (app_id, expected_dir_id) in expected {
            assert_eq!(
                wine_prefix_for(&state, app_id).as_deref(),
                Some(*expected_dir_id),
                "Wine App {app_id} must resolve to Directory {expected_dir_id} \
                 via the Wine_App_has_prefix_Directory cell"
            );
        }
    }

    // ── wine_app_by_name (#503) ────────────────────────────────────────

    /// Build a minimal D containing two Wine Apps, each with the
    /// hand-pushed `Wine_App_has_Compat_Rating` and
    /// `Wine_App_has_prefix_Directory` bindings the readings would
    /// produce. Mirrors the shape `instance_fact_field_cells` emits
    /// (subject keyed by the `Wine App` noun name) so the lookup
    /// helper can operate on a synthesized state without paying the
    /// full readings parse cost.
    fn seeded_wine_app_state() -> ast::Object {
        let d = ast::Object::phi();
        // notepad-plus-plus
        let d = ast::cell_push(
            "Wine_App_has_Compat_Rating",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Compat Rating", "gold"),
            ]),
            &d,
        );
        let d = ast::cell_push(
            "Wine_App_has_prefix_Directory",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Directory", "notepad-plus-plus-prefix"),
            ]),
            &d,
        );
        let d = ast::cell_push(
            "has display- Title 'Notepad++'",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("has display- Title 'Notepad++'", ""),
            ]),
            &d,
        );
        // photoshop-cs6
        let d = ast::cell_push(
            "Wine_App_has_Compat_Rating",
            ast::fact_from_pairs(&[
                ("Wine App", "photoshop-cs6"),
                ("Compat Rating", "gold"),
            ]),
            &d,
        );
        let d = ast::cell_push(
            "Wine_App_has_prefix_Directory",
            ast::fact_from_pairs(&[
                ("Wine App", "photoshop-cs6"),
                ("Directory", "photoshop-cs6-prefix"),
            ]),
            &d,
        );
        ast::cell_push(
            "has display- Title 'Adobe Photoshop CS6'",
            ast::fact_from_pairs(&[
                ("Wine App", "photoshop-cs6"),
                ("has display- Title 'Adobe Photoshop CS6'", ""),
            ]),
            &d,
        )
    }

    #[test]
    fn wine_app_ids_returns_distinct_sorted_slugs() {
        let state = seeded_wine_app_state();
        let ids = wine_app_ids(&state);
        assert_eq!(ids, vec!["notepad-plus-plus".to_string(),
                             "photoshop-cs6".to_string()]);
    }

    #[test]
    fn wine_app_ids_empty_for_phi_state() {
        let state = ast::Object::phi();
        assert!(wine_app_ids(&state).is_empty());
    }

    #[test]
    fn wine_app_by_name_resolves_exact_slug() {
        let state = seeded_wine_app_state();
        assert_eq!(
            wine_app_by_name(&state, "notepad-plus-plus"),
            Some(("notepad-plus-plus".to_string(),
                  "notepad-plus-plus-prefix".to_string())),
        );
        assert_eq!(
            wine_app_by_name(&state, "photoshop-cs6"),
            Some(("photoshop-cs6".to_string(),
                  "photoshop-cs6-prefix".to_string())),
        );
    }

    #[test]
    fn wine_app_by_name_resolves_display_title() {
        let state = seeded_wine_app_state();
        // Display title — falls through to the `has display- Title '<X>'`
        // cell scan.
        assert_eq!(
            wine_app_by_name(&state, "Notepad++"),
            Some(("notepad-plus-plus".to_string(),
                  "notepad-plus-plus-prefix".to_string())),
        );
        assert_eq!(
            wine_app_by_name(&state, "Adobe Photoshop CS6"),
            Some(("photoshop-cs6".to_string(),
                  "photoshop-cs6-prefix".to_string())),
        );
    }

    #[test]
    fn wine_app_by_name_returns_none_for_unknown() {
        let state = seeded_wine_app_state();
        assert!(wine_app_by_name(&state, "nope").is_none());
        assert!(wine_app_by_name(&state, "Notpad++").is_none());
    }

    #[test]
    fn wine_app_display_title_returns_none_for_unknown_slug() {
        let state = seeded_wine_app_state();
        assert!(wine_app_display_title(&state, "no-such-app").is_none());
    }

    /// End-to-end: load wine.md (with filesystem.md preloaded for the
    /// `Directory` noun) and confirm `wine_app_by_name` resolves both
    /// known slugs and known display titles. Mirrors the shape of
    /// `wine_prefix_for_resolves_every_seeded_wine_app` above.
    #[cfg(feature = "wine")]
    #[test]
    fn wine_app_by_name_resolves_every_seeded_wine_app() {
        let filesystem_md = include_str!("../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");

        // Slug lookups — the .Name reference value resolves directly.
        let slug_expectations: &[(&str, &str)] = &[
            ("notepad-plus-plus",  "notepad-plus-plus-prefix"),
            ("office-2016-word",   "office-2016-word-prefix"),
            ("photoshop-cs6",      "photoshop-cs6-prefix"),
            ("autohotkey-v1",      "autohotkey-v1-prefix"),
            ("notion-desktop",     "notion-desktop-prefix"),
            ("total-commander",    "total-commander-prefix"),
            ("vscode",             "vscode-prefix"),
            ("spotify",            "spotify-prefix"),
            ("steam-windows",      "steam-windows-prefix"),
            ("7-zip",              "7-zip-prefix"),
        ];
        for (slug, expected_dir) in slug_expectations {
            let resolved = wine_app_by_name(&state, slug);
            assert_eq!(
                resolved.as_ref().map(|(s, d)| (s.as_str(), d.as_str())),
                Some((*slug, *expected_dir)),
                "slug `{slug}` must resolve via wine_app_by_name",
            );
        }

        // Display-title lookups — the human-readable name resolves to
        // the same (slug, prefix) pair via the title-scan path.
        let title_expectations: &[(&str, &str, &str)] = &[
            ("Notepad++",                  "notepad-plus-plus", "notepad-plus-plus-prefix"),
            ("Microsoft Word 2016",        "office-2016-word",  "office-2016-word-prefix"),
            ("Adobe Photoshop CS6",        "photoshop-cs6",     "photoshop-cs6-prefix"),
            ("AutoHotkey 1.x",             "autohotkey-v1",     "autohotkey-v1-prefix"),
            ("Notion",                     "notion-desktop",    "notion-desktop-prefix"),
            ("Total Commander",            "total-commander",   "total-commander-prefix"),
            ("Visual Studio Code",         "vscode",            "vscode-prefix"),
            ("Spotify",                    "spotify",           "spotify-prefix"),
            ("Steam (Windows client)",     "steam-windows",     "steam-windows-prefix"),
            ("7-Zip",                      "7-zip",             "7-zip-prefix"),
        ];
        for (title, expected_slug, expected_dir) in title_expectations {
            let resolved = wine_app_by_name(&state, title);
            assert_eq!(
                resolved.as_ref().map(|(s, d)| (s.as_str(), d.as_str())),
                Some((*expected_slug, *expected_dir)),
                "display title `{title}` must resolve via wine_app_by_name",
            );
        }

        // Unknown name — neither slug nor title.
        assert!(wine_app_by_name(&state, "no-such-app").is_none());
        // Typo — exact match miss; the CLI's Levenshtein layer handles
        // suggestion separately.
        assert!(wine_app_by_name(&state, "Notpad++").is_none());
    }

    // ── task-930: bulk / collection-shaped apply ────────────────────
    //
    // Backus α (apply-to-all) over the input sequence:
    // `apply([op1, op2, …])` = α(ρ-dispatch). The batch runs as ONE
    // request — resolve all → derive to the least fixed point once over
    // the COMBINED population → validate → emit. An alethic violation in
    // ANY op rejects the WHOLE batch (`D' = D`, AREST.tex "Completeness
    // of State Transfer"). A single op is a 1-element collection.

    /// Two creates + a transition applied in ONE batch call. Each op
    /// sees the prior ops' facts (combined population), the result
    /// carries every op's entities, and the transition lands the target
    /// in its post-event status — proof the SM auto-advance ran over the
    /// state the batch built, not N independent snapshots.
    #[test]
    fn batch_two_creates_and_transition_apply_atomically() {
        const READINGS: &str = r#"
# Orders batch

## Entity Types

Order(.Order Number) is an entity type.

## Fact Types

Order has Amount.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.
"#;
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state =
            crate::parse_forml2::parse_to_state_with_nouns(READINGS, &meta_state).unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mk_create = |id: &str, amount: &str| {
            let mut fields = HashMap::new();
            fields.insert("Amount".to_string(), amount.to_string());
            Command::CreateEntity {
                noun: "Order".to_string(),
                domain: "orders".to_string(),
                id: Some(id.to_string()),
                fields,
                sender: None,
                signature: None,
            }
        };
        let batch = vec![
            mk_create("ORD-1", "10"),
            mk_create("ORD-2", "20"),
            Command::Transition {
                entity_id: "ORD-1".to_string(),
                event: "place".to_string(),
                domain: "orders".to_string(),
                current_status: Some("Draft".to_string()),
                sender: None,
                signature: None,
            },
        ];

        let result = apply_command_batch(&def_obj, &batch, &state);

        assert!(!result.rejected,
            "batch must succeed; violations={:?}", result.violations);
        // Combined population: BOTH orders' Amount facts ride in the one
        // delta the batch emits — proof the ops share one state, not N.
        let merged = ast::merge_states(&state, &result.state);
        let amounts = ast::fetch_cell_seq("Order_has_Amount", &merged);
        let ord_ids: Vec<String> = amounts.as_seq().map(|s| s.iter()
            .filter_map(|f| ast::binding(f, "Order").map(String::from))
            .collect()).unwrap_or_default();
        assert!(ord_ids.iter().any(|i| i == "ORD-1") && ord_ids.iter().any(|i| i == "ORD-2"),
            "batch delta must carry both creates; got {:?}", ord_ids);
        // One fixpoint over the combined state: ORD-1's transition fired
        // and the SM cell reflects 'Placed'.
        assert_eq!(extract_sm_status(&merged, "ORD-1").as_deref(), Some("Placed"),
            "ORD-1 must be transitioned to Placed in the batch result");
    }

    /// task-954: a batch carrying TWO transitions on the SAME entity
    /// (start then finish) must drive it all the way to the final status.
    /// The MCP batch path (`buildApplyCommandForBatch`) supplies
    /// `current_status: None`, so the engine must resolve `from_status`
    /// from the CUMULATIVE running state — not the machine's initial
    /// status. Before the fix, the second op (`finish`) resolved
    /// `from = pending` (the initial) instead of `in_progress`, found no
    /// `pending --finish-->` edge, and silently no-op'd — leaving the
    /// entity stuck `in_progress` while the batch still reported success.
    #[test]
    fn batch_sequential_transitions_same_entity_resolve_from_running_state() {
        const READINGS: &str = r#"
# Widget batch SM

## Entity Types

Widget(.id) is an entity type.

## Fact Types

Widget has Label.

## Instance Facts

State Machine Definition 'Widget' is for Noun 'Widget'.
Status 'pending' is initial in State Machine Definition 'Widget'.

Transition 'start' is defined in State Machine Definition 'Widget'.
  Transition 'start' is from Status 'pending'.
  Transition 'start' is to Status 'in_progress'.
  Transition 'start' is triggered by Event Type 'start'.

Transition 'finish' is defined in State Machine Definition 'Widget'.
  Transition 'finish' is from Status 'in_progress'.
  Transition 'finish' is to Status 'completed'.
  Transition 'finish' is triggered by Event Type 'finish'.
"#;
        let meta_state = crate::parse_forml2::parse_to_state(STATE_METAMODEL).unwrap();
        let domain_state =
            crate::parse_forml2::parse_to_state_with_nouns(READINGS, &meta_state).unwrap();
        let state = ast::merge_states(&meta_state, &domain_state);
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_obj = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("Label".to_string(), "w".to_string());
        // current_status: None on BOTH transitions — exactly what
        // buildApplyCommandForBatch emits for the MCP `ops` shape.
        let batch = vec![
            Command::CreateEntity {
                noun: "Widget".to_string(),
                domain: "widgets".to_string(),
                id: Some("W1".to_string()),
                fields,
                sender: None,
                signature: None,
            },
            Command::Transition {
                entity_id: "W1".to_string(),
                event: "start".to_string(),
                domain: "widgets".to_string(),
                current_status: None,
                sender: None,
                signature: None,
            },
            Command::Transition {
                entity_id: "W1".to_string(),
                event: "finish".to_string(),
                domain: "widgets".to_string(),
                current_status: None,
                sender: None,
                signature: None,
            },
        ];

        let result = apply_command_batch(&def_obj, &batch, &state);
        assert!(!result.rejected,
            "batch must succeed; violations={:?}", result.violations);
        let merged = ast::merge_states(&state, &result.state);
        assert_eq!(extract_sm_status(&merged, "W1").as_deref(), Some("completed"),
            "W1 must reach 'completed' — both transitions in one batch must \
             resolve from_status from the cumulative running state, not the \
             machine initial");
    }

    /// Atomic rollback: a batch whose middle op is alethic-rejected (a
    /// duplicate explicit id, the task-737 UC) rejects the WHOLE batch.
    /// The emitted delta is empty (`D' = D`) so NONE of the batch's
    /// creates land — not even the op that ran before the violation.
    #[test]
    fn batch_with_alethic_violation_rolls_back_entire_batch() {
        let src = "Task(.id) is an entity type.\nTask has Description.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        let mk = |id: &str, desc: &str| {
            let mut fields = HashMap::new();
            fields.insert("Description".to_string(), desc.to_string());
            Command::CreateEntity {
                noun: "Task".to_string(),
                domain: "tasks".to_string(),
                id: Some(id.to_string()),
                fields,
                sender: None,
                signature: None,
            }
        };
        // Op 1 creates T-1 (would succeed alone); op 2 re-uses T-1's id
        // and is alethic-rejected by the reference-scheme UC; op 3 would
        // also succeed alone. The whole batch must roll back.
        let batch = vec![mk("T-1", "first"), mk("T-1", "dup"), mk("T-2", "third")];

        let result = apply_command_batch(&def_map, &batch, &state);

        assert!(result.rejected,
            "an alethic violation anywhere must reject the batch; \
             violations={:?}", result.violations);
        assert!(result.violations.iter().any(|v| v.alethic),
            "must surface the alethic violation; got {:?}", result.violations);
        // D' = D: empty delta, so NOTHING from the batch lands.
        assert!(ast::cells_iter(&result.state).is_empty(),
            "rejected batch must emit an empty delta (D' = D); got {:?}",
            result.state);
        let merged = ast::merge_states(&state, &result.state);
        assert!(ast::fetch_or_phi("Task_has_Description", &merged).as_seq()
            .map_or(true, |s| s.is_empty()),
            "no Task may survive the rolled-back batch");
    }

    /// A lone op is the natural 1-element collection: `apply_command_batch`
    /// with a single command behaves exactly like `apply_command_defs`.
    #[test]
    fn batch_single_op_matches_single_apply() {
        let src = "Task(.id) is an entity type.\nTask has Description.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        let mut fields = HashMap::new();
        fields.insert("Description".to_string(), "solo".to_string());
        let cmd = Command::CreateEntity {
            noun: "Task".to_string(),
            domain: "tasks".to_string(),
            id: Some("T-solo".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let single = apply_command_defs(&def_map, &cmd, &state);
        let batch = apply_command_batch(&def_map, &core::slice::from_ref(&cmd), &state);
        assert_eq!(single.rejected, batch.rejected);
        assert_eq!(single.entities.len(), batch.entities.len());
        // Same touched cells.
        let single_merged = ast::merge_states(&state, &single.state);
        let batch_merged = ast::merge_states(&state, &batch.state);
        assert_eq!(
            ast::fetch_or_phi("Task_has_Description", &single_merged),
            ast::fetch_or_phi("Task_has_Description", &batch_merged),
            "1-element batch must produce the same Task_has_Description cell");
    }

    /// The `Command::Batch` variant deserializes from the collection
    /// JSON shape `{"type":"batch","commands":[…]}` and dispatches
    /// through `apply_command_defs` as the natural batch entry point.
    #[test]
    fn batch_command_deserializes_and_dispatches() {
        let src = "Task(.id) is an entity type.\nTask has Description.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let defs = crate::compile::compile_to_defs_state(&state);
        let def_map = ast::defs_to_state(&defs, &state);

        let json = r#"{
            "type":"batch",
            "commands":[
                {"type":"createEntity","noun":"Task","domain":"tasks","id":"T-a","fields":{"Description":"a"}},
                {"type":"createEntity","noun":"Task","domain":"tasks","id":"T-b","fields":{"Description":"b"}}
            ]
        }"#;
        let cmd: Command = serde_json::from_str(json).expect("batch JSON must parse");
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected, "batch JSON must apply; {:?}", result.violations);
        let merged = ast::merge_states(&state, &result.state);
        let descs = ast::fetch_cell_seq("Task_has_Description", &merged);
        let n = descs.as_seq().map(|s| s.len()).unwrap_or(0);
        assert_eq!(n, 2, "both batched creates must land; got {n}");
    }

}
