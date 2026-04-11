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
        fields: std::collections::HashMap<String, String>,
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
    /// is-qry: query the population (partial application of graph schema)
    Query {
        #[serde(alias = "schemaId")]
        schema_id: String,
        domain: String,
        target: String,
        bindings: std::collections::HashMap<String, String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
    /// is-upd: update entity fields (<->F  .  [upd, defs])
    UpdateEntity {
        noun: String,
        domain: String,
        #[serde(alias = "entityId")]
        entity_id: String,
        fields: std::collections::HashMap<String, String>,
        #[serde(default)]
        sender: Option<String>,
        #[serde(default)]
        signature: Option<String>,
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
    pub data: std::collections::HashMap<String, String>,
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
    entity_id: &str, fields: &std::collections::HashMap<String, String>,
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
    entity_id: &str, fields: &std::collections::HashMap<String, String>,
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
/// Expected: <entities, status, transitions, violations, derived_count, rejected, new_state>
pub fn decode_command_result(obj: &ast::Object) -> CommandResult {
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

/// Encode a CommandResult as an Object (inverse of decode_command_result).
pub fn encode_command_result(result: &CommandResult) -> ast::Object {
    let entities = ast::Object::Seq(result.entities.iter().map(|e| {
        let data = ast::Object::Seq(e.data.iter().map(|(k, v)| {
            ast::Object::seq(vec![ast::Object::atom(k), ast::Object::atom(v)])
        }).collect());
        ast::Object::seq(vec![ast::Object::atom(&e.id), ast::Object::atom(&e.entity_type), data])
    }).collect());

    let status = result.status.as_ref().map(|s| ast::Object::atom(s)).unwrap_or(ast::Object::phi());

    let transitions = ast::Object::Seq(result.transitions.iter().map(|t| {
        ast::Object::seq(vec![
            ast::Object::atom(&t.event), ast::Object::atom(&t.target_status),
            ast::Object::atom(&t.method), ast::Object::atom(&t.href),
        ])
    }).collect());

    let violations = ast::Object::Seq(result.violations.iter().map(|v| {
        ast::Object::seq(vec![
            ast::Object::atom(&v.constraint_id), ast::Object::atom(&v.constraint_text),
            ast::Object::atom(&v.detail), ast::Object::atom(if v.alethic { "T" } else { "F" }),
        ])
    }).collect());

    ast::Object::seq(vec![
        entities, status, transitions, violations,
        ast::Object::atom(&result.derived_count.to_string()),
        if result.rejected { ast::Object::t() } else { ast::Object::f() },
        result.state.clone(),
    ])
}

// -- Apply ------------------------------------------------------------

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
        Command::UpdateEntity { noun, domain, entity_id, fields, sender: _, signature: _ } => {
            update_via_defs(d, noun, domain, entity_id, fields, state)
        }
        Command::LoadReadings { markdown, domain, sender: _, signature: _ } => {
            apply_load_readings(markdown, domain, d, state)
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
            state: state.clone(),
        },
    }
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
fn create_via_defs(
    d: &ast::Object,
    noun: &str,
    domain: &str,
    explicit_id: Option<&str>,
    fields: &std::collections::HashMap<String, String>,
    sender: Option<&str>,
    state: &ast::Object,
) -> CommandResult {
    let entity_id = explicit_id.unwrap_or("").to_string();

    // ── resolve: populate facts via ρ(resolve:{noun}) ──────────────
    let fields_with_domain: Vec<(&str, &str)> = fields.iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .chain(std::iter::once(("domain", domain)))
        .collect();
    let mut fact_events: Vec<String> = Vec::new();
    let resolved = fields_with_domain.iter().fold(state.clone(), |acc, (field_name, value)| {
        let ft_id_obj = ast::apply(&ast::Func::Def(format!("resolve:{}", noun)),
            &ast::Object::atom(&field_name.to_lowercase()), d);
        let ft_id = ft_id_obj.as_atom().map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}_has_{}", noun, field_name));
        fact_events.push(ft_id.clone());
        ast::cell_push(&ft_id, ast::fact_from_pairs(&[(noun, &entity_id), (field_name, value)]), &acc)
    });

    // ── resolve: compound ref scheme decomposition ──────────────────
    // Paper Eq. 6: resolve determines identity from the reference scheme.
    // For compound schemes (.Owner, .Seq), split entity_id on '-' (rsplitn)
    // and push component facts: Thing_has_Owner, Thing_has_Seq.
    let resolved = {
        let noun_cell = ast::fetch_or_phi("Noun", &resolved);
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
                    ast::cell_push(&ft_id, ast::fact_from_pairs(&[(noun, &entity_id), (part, value)]), &acc)
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
        let with_user = ast::cell_push(
            &user_ref_ft,
            ast::fact_from_pairs(&[("User", s), ("Email", s)]),
            &resolved,
        );
        ast::cell_push(
            &created_by_ft,
            ast::fact_from_pairs(&[(noun, &entity_id), ("User", s)]),
            &with_user,
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
    let sm_event_types: std::collections::HashSet<String> = if has_sql_triggers {
        let trigger_cell = ast::fetch_or_phi("Transition_is_triggered_by_Event_Type", d);
        trigger_cell.as_seq().map(|facts| {
            facts.iter().filter_map(|f| {
                ast::binding(f, "Event Type").map(|s| s.to_string())
            }).collect()
        }).unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };
    // Noun-gated derivation index: O(1) fetch from compiled index.
    // The index is stored as Func::constant(atom) → func_to_object yields <', atom>.
    // Extract the atom from the constant form.
    let relevant_ids: std::collections::HashSet<String> = {
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
    let derivation_defs_owned: Vec<(String, ast::Func)> = ast::cells_iter(d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .filter(|(n, _)| {
            let def_id = n.strip_prefix("derivation:").unwrap_or(n);
            if has_sql_triggers {
                // SM infrastructure derivations
                n.contains("StateMachine") || n.contains("machine:") || n.contains("_transitive_Status")
                    || n.contains("_transitive_Transition") || n.contains("sm_init")
                // Derivations whose consequent is needed by the SM
                    || sm_event_types.iter().any(|evt| n.contains(evt))
            } else if !relevant_ids.is_empty() {
                // Noun-gated: only run derivations relevant to the created noun
                relevant_ids.contains(def_id)
                    // Always include SM infrastructure
                    || n.contains("StateMachine") || n.contains("machine:")
                    || n.contains("sm_init")
            } else {
                true // no index available, run all
            }
        })
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
        .collect();
    eprintln!("[profile] derivation gating: {}/{} rules for noun '{}'",
        derivation_defs_owned.len(),
        ast::cells_iter(d).into_iter().filter(|(n, _)| n.starts_with("derivation:")).count(),
        noun);
    let derivation_refs: Vec<(&str, &ast::Func)> = derivation_defs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (derived_state, derived) = crate::evaluate::forward_chain_defs_state(&derivation_refs, &resolved);

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
                    eprintln!("[sm] {} --{}--> {}", current, event, new_status);
                    current = new_status;
                }
            }

            // Phase 2: positive guards — check P for facts that satisfy
            // outgoing transitions. Loop until no transition fires.
            // The transitions:{noun} def returns <<from, to, event>, ...>.
            let transitions_key = format!("transitions:{}", noun);
            let mut advanced = true;
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
                    // Look up schema to find which role(s) the SM noun plays,
                    // then check those specific bindings for entity_id.
                    if !event_type.is_empty() && !target.is_empty() {
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
                        let cell = ast::fetch_or_phi(event_type, &st);
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
                            eprintln!("[sm:guard] {} --{}--> {}", current, event_type, target);
                            current = target.to_string();
                            advanced = true;
                            break; // restart from new status
                        }
                    }
                }
            }

            // Write final status to state.
            let init_status = extract_sm_status(&derived_state, &entity_id).unwrap_or_default();
            if current != init_status {
                let status_key = "StateMachine_has_currentlyInStatus";
                let filtered = ast::cell_filter(status_key, |f| {
                    !ast::binding_matches(f, "State Machine", &entity_id)
                }, &st);
                st = ast::cell_push(status_key, ast::fact_from_pairs(&[
                    ("State Machine", &entity_id),
                    ("currentlyInStatus", &current),
                ]), &filtered);
            }
            st
        } else {
            derived_state
        }
    };

    // ── validate: ρ(validate) applied to population ────────────────
    let ctx_obj = ast::encode_eval_context_state("", None, &derived_state);
    let violation_obj = ast::apply(&ast::Func::Def("validate".to_string()), &ctx_obj, d);
    let violations = ast::decode_violations(&violation_obj);
    let rejected = violations.iter().any(|v| v.alethic);

    // ── emit: construct representation via ρ ────────────────────────
    let sm_derived: Vec<_> = derived.iter()
        .filter(|d| d.fact_type_id.contains("StateMachine") || d.fact_type_id.contains("Machine"))
        .map(|d| format!("{}:{:?}", d.fact_type_id, d.bindings))
        .collect();
    eprintln!("[debug] SM derived facts: {:?}", sm_derived);
    let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &derived_state);
    eprintln!("[debug] SM cell: {:?}", sm_cell);
    let status = extract_sm_status(&derived_state, &entity_id);
    let transitions = hateoas_via_rho(d, noun, &entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, noun, &entity_id);

    let entity_data: std::collections::HashMap<String, String> = fields_with_domain.iter()
        .map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let entities = std::iter::once(EntityResult {
        id: entity_id.clone(), entity_type: noun.to_string(), data: entity_data,
    }).chain(status.as_ref().map(|st| {
        EntityResult {
            id: entity_id.clone(), entity_type: "State Machine".to_string(),
            data: [("forResource", entity_id.as_str()), ("currentlyInStatus", st.as_str()), ("domain", domain)]
                .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    })).collect();

    // Security #26: audit trail — append an audit_log fact to the returned
    // state so callers who persist the state see the trace. rejected applies
    // to the pre-audit state; the audit push itself is invisible to the
    // decision because it only touches the audit_log cell.
    let outcome = match rejected { true => "rejected", false => "ok" };
    let final_state = ast::record_audit(
        match rejected { true => state, false => &derived_state },
        "apply:create",
        outcome,
        sender,
    );
    CommandResult {
        entities, status, transitions, navigation, violations,
        derived_count: derived.len(), rejected,
        state: final_state,
    }
}

fn resolve_fact_type_id_defs(
    d: &ast::Object,
    noun: &str,
    field: &str,
) -> String {
    ast::cells_iter(d).into_iter()
        .filter_map(|(name, _)| name.strip_prefix("schema:").map(|s| s.to_string()))
        .find(|schema_id| schema_id.contains(noun) && schema_id.contains(field))
        .unwrap_or_else(|| format!("{}_has_{}", noun, field))
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
            let from_status = current_status.map(|s| s.to_string()).or_else(|| {
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
    let status_key = "StateMachine_has_currentlyInStatus";
    new_state = new_status.as_ref()
        .map(|status| {
            let filtered = ast::cell_filter(status_key, |f| {
                !ast::binding_matches(f, "State Machine", entity_id)
            }, &new_state);
            ast::cell_push(status_key, ast::fact_from_pairs(&[
                ("State Machine", entity_id),
                ("currentlyInStatus", status.as_str()),
            ]), &filtered)
        })
        .unwrap_or(new_state);

    let status = new_status.or_else(|| current_status.map(|s| s.to_string()));

    let transitions = hateoas_via_rho(d, &noun, entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, &noun, entity_id);

    CommandResult {
        entities: vec![],
        status,
        transitions,
        navigation,
        violations: vec![],
        derived_count: 0,
        rejected: false,
        state: new_state,
    }
}

fn query_via_defs(
    d: &ast::Object,
    schema_id: &str,
    target: &str,
    bindings: &std::collections::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    // Look up schema role names from state metadata
    let role_cell = ast::fetch_or_phi("Role", state);
    let role_names: Vec<String> = role_cell.as_seq()
        .map(|roles| {
            let mut matched: Vec<(usize, String)> = roles.iter()
                .filter(|r| ast::binding_matches(r, "graphSchema", schema_id))
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
    };
    let results = crate::query::query_with_ast(state, &schema, target_role, &filter_refs);

    let mut data = std::collections::HashMap::new();
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
        state: state.clone(),
    }
}

fn update_via_defs(
    d: &ast::Object,
    noun: &str,
    domain: &str,
    entity_id: &str,
    new_fields: &std::collections::HashMap<String, String>,
    state: &ast::Object,
) -> CommandResult {
    // Read current facts for this entity, merge with new fields
    let merged: std::collections::HashMap<String, String> = ast::cells_iter(state)
        .into_iter()
        .flat_map(|(_, contents)| contents.as_seq().into_iter().flat_map(|facts| facts.to_vec()))
        .filter_map(|fact| {
            let pairs = fact.as_seq().filter(|p| p.len() >= 2)?;
            let v0 = pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()));
            (v0.as_deref() == Some(entity_id)).then_some(())?;
            let k = pairs[1].as_seq().and_then(|p| p.get(0)?.as_atom().map(|s| s.to_string()))?;
            let v = pairs[1].as_seq().and_then(|p| p.get(1)?.as_atom().map(|s| s.to_string()))?;
            Some((k, v))
        })
        .chain(new_fields.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();

    // Remove old facts for this entity, insert merged (fold over fields)
    let resolve_key = format!("resolve:{}", noun);
    let new_state = merged.iter().fold(state.clone(), |acc, (field_name, value)| {
        let ft_id = def_func(&resolve_key, d)
            .map(|f| ast::apply(&f, &ast::Object::atom(&field_name.to_lowercase()), d))
            .and_then(|o| o.as_atom().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{}_has_{}", noun, field_name));
        let acc = ast::cell_filter(&ft_id, |f| {
            f.as_seq().map_or(true, |pairs| {
                pairs.len() < 2 || pairs[0].as_seq().and_then(|p| p.get(1)?.as_atom()) != Some(entity_id)
            })
        }, &acc);
        ast::cell_push(&ft_id, ast::fact_from_pairs(&[(noun, entity_id), (field_name.as_str(), value.as_str())]), &acc)
    });

    // derive + validate + emit
    let derivation_defs_owned: Vec<(String, ast::Func)> = ast::cells_iter(d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
        .collect();
    let derivation_defs: Vec<(&str, &ast::Func)> = derivation_defs_owned.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (new_state, derived) = crate::evaluate::forward_chain_defs_state(&derivation_defs, &new_state);

    let ctx_obj = ast::encode_eval_context_state("", None, &new_state);
    let validate_func = def_func("validate", d).unwrap_or(ast::Func::constant(ast::Object::phi()));
    let violation_obj = ast::apply(&validate_func, &ctx_obj, d);
    let violations = ast::decode_violations(&violation_obj);
    let rejected = violations.iter().any(|v| v.alethic);
    let sm_id = entity_id.to_string();
    let status = extract_sm_status(&new_state, &sm_id);
    let transitions = hateoas_via_rho(d, noun, entity_id, status.as_deref());
    let navigation = nav_links_via_rho(d, noun, entity_id);

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
        state: if rejected { state.clone() } else { new_state },
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
    let mut queue: std::collections::VecDeque<(String, Vec<(String, String, String)>)> =
        std::collections::VecDeque::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
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
        let cell = ast::fetch_or_phi(ft_id, state);
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
    let target_cell = ast::fetch_or_phi(target_ft, state);
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
                state: state.clone(),
            };
        }
    };

    // Count genuinely new nouns (in parsed but not in D)
    let existing_noun_names: std::collections::HashSet<String> = ast::fetch_or_phi("Noun", d).as_seq()
        .map(|facts| facts.iter().filter_map(|f| ast::binding(f, "name").map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let new_noun_count = ast::fetch_or_phi("Noun", &parsed).as_seq()
        .map(|facts| facts.iter().filter(|f| {
            ast::binding(f, "name").map_or(false, |n| !existing_noun_names.contains(n))
        }).count())
        .unwrap_or(0);

    // Merge: foldl(concat_cell, D, cells(parsed))
    let merged_state = ast::merge_states(d, &parsed);

    // Compile defs from merged state + re-register platform primitives
    let mut defs = crate::compile::compile_to_defs_state(&merged_state);
    defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
    let new_d = ast::defs_to_state(&defs, &merged_state);

    let mut data = std::collections::HashMap::new();
    data.insert("domain".to_string(), domain.to_string());
    data.insert("nouns".to_string(), new_noun_count.to_string());

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
        // Return full D (state + recompiled defs) — Corollary 5
        state: new_d,
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
            Some(TransitionAction {
                event, target_status: to, method: "POST".to_string(),
                href: format!("/api/entities/{}/{}/transition", encoded, entity_id),
            })
        }).collect()
    }).unwrap_or_default()
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
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    cell.as_seq()?.iter()
        .find(|fact| {
            ast::binding_matches(fact, "State Machine", sm_id)
                || fact.as_seq().map_or(false, |pairs| {
                    pairs.iter().any(|pair| pair.as_seq().and_then(|p| p.get(1)?.as_atom()) == Some(sm_id))
                })
        })
        .and_then(|fact| ast::binding(fact, "currentlyInStatus").map(|s| s.to_string()))
}

fn to_camel_case(s: &str) -> String {
    s.split(' ')
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.to_lowercase()
            } else {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            }
        })
        .collect()
}

// -- Tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
        assert_eq!(result.entities[1].data["currentlyInStatus"], "Draft");
        assert_eq!(result.entities[1].data["forResource"], "ORD-100");
        assert_eq!(result.status.as_deref(), Some("Draft"));
        assert_eq!(result.transitions.len(), 2); // place, cancel
        assert!(result.transitions.iter().any(|t| t.event == "place"));
        assert!(result.transitions.iter().any(|t| t.event == "cancel"));
        assert!(!result.rejected);
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
        let customer_cell = ast::fetch_or_phi("Order_has_customer", &result.state);
        let customer_facts = customer_cell.as_seq().unwrap();
        assert_eq!(customer_facts.len(), 1);
        assert!(ast::binding(&customer_facts[0], "customer") == Some("acme"));

        // SM facts are in the state
        let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        assert!(ast::binding(&sm_facts[0], "currentlyInStatus") == Some("Draft"));
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

        // State must contain the updated status
        let sm_cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &result.state);
        let sm_facts = sm_cell.as_seq().unwrap();
        let sm_fact = sm_facts.iter().find(|f|
            ast::binding_matches(f, "State Machine", "ORD-1")
        ).expect("SM fact must exist for ORD-1");
        assert_eq!(ast::binding(sm_fact, "currentlyInStatus"), Some("Placed"), "state must reflect new status");
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
        assert_eq!(result.entities[0].data["nouns"], "2");
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

    /// #26: audit trail — create command pushes an audit_log fact.
    #[test]
    fn create_command_appends_audit_log_entry() {
        let (def_map, state) = setup_order_defs();
        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-AUD".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-AUD".to_string()),
            fields,
            sender: Some("auditor@example.com".to_string()),
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        let log = ast::fetch_or_phi("audit_log", &result.state);
        let entries = log.as_seq().expect("audit_log cell must exist after apply");
        assert!(!entries.is_empty(), "audit_log must contain at least one entry");
        let first = &entries[0];
        assert_eq!(ast::binding(first, "operation"), Some("apply:create"));
        assert_eq!(ast::binding(first, "outcome"), Some("ok"));
        assert_eq!(ast::binding(first, "sender"), Some("auditor@example.com"));
    }

    /// #26: successful `platform_compile` must push an audit_log fact with
    /// operation=compile, outcome=compiled. Exercises the ρ-level compile
    /// primitive — same pattern as the compile_history #22 test.
    #[test]
    fn platform_compile_appends_audit_log_entry_on_success() {
        let readings = "Each Person has a name.";
        let initial_d = ast::defs_to_state(
            &vec![("compile".to_string(), ast::Func::Platform("compile".to_string()))],
            &ast::Object::phi(),
        );
        let result = ast::apply(
            &ast::Func::Platform("compile".to_string()),
            &ast::Object::atom(readings),
            &initial_d,
        );
        // Must be a successful state, not a ⊥ atom error.
        assert!(
            result.as_atom().map(|s| !s.starts_with("⊥")).unwrap_or(true),
            "compile should not return an error atom, got: {:?}",
            result
        );
        let log = ast::fetch_or_phi("audit_log", &result);
        let entries = log.as_seq().expect("audit_log cell should exist after successful compile");
        assert_eq!(entries.len(), 1, "expected exactly one audit_log entry after one compile");
        assert_eq!(ast::binding(&entries[0], "operation"), Some("compile"));
        assert_eq!(ast::binding(&entries[0], "outcome"), Some("compiled"));
        assert_eq!(ast::binding(&entries[0], "sequence"), Some("0"));
        // Compile has no sender — must render as empty string.
        assert_eq!(ast::binding(&entries[0], "sender"), Some(""));
    }

    /// #26: rejected apply (alethic MC violation) must still push an
    /// audit_log entry with operation=apply:create, outcome=rejected.
    /// Uses the same AUTH_DOMAIN pattern as
    /// `mc_fires_on_missing_mandatory_role_for_new_entity`.
    #[test]
    fn rejected_create_appends_audit_log_rejected_entry() {
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

        // Create without a User fact and without a sender — MC must
        // reject this command (same pattern as the mc_fires_on_... test).
        let mut fields = HashMap::new();
        fields.insert("OrderId".to_string(), "ord-rej".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "test".to_string(),
            id: Some("ord-rej".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(result.rejected, "MC violation must reject the command");

        // Even on rejection, the returned state must carry an audit entry
        // so a host that chooses to persist for audit-only purposes has
        // the rejection recorded.
        let log = ast::fetch_or_phi("audit_log", &result.state);
        let entries = log.as_seq().expect("audit_log cell must exist after rejected apply");
        assert_eq!(entries.len(), 1, "rejected apply should still push one audit entry");
        assert_eq!(ast::binding(&entries[0], "operation"), Some("apply:create"));
        assert_eq!(ast::binding(&entries[0], "outcome"), Some("rejected"));
        assert_eq!(ast::binding(&entries[0], "sequence"), Some("0"));
        // No sender supplied → empty string binding (None materializes as "").
        assert_eq!(ast::binding(&entries[0], "sender"), Some(""));
    }

    /// #26: multiple applied commands must yield monotonically increasing
    /// sequence numbers (0, 1, 2) — the audit trail is totally ordered.
    #[test]
    fn multiple_commands_yield_monotonic_audit_sequence() {
        let (def_map, state) = setup_order_defs();

        let make_create = |id: &str| {
            let mut fields = HashMap::new();
            fields.insert("orderNumber".to_string(), id.to_string());
            Command::CreateEntity {
                noun: "Order".to_string(),
                domain: "orders".to_string(),
                id: Some(id.to_string()),
                fields,
                sender: Some(format!("u-{}", id)),
                signature: None,
            }
        };

        // Thread state across three successive creates.
        let r1 = apply_command_defs(&def_map, &make_create("ORD-SEQ-1"), &state);
        assert!(!r1.rejected, "create 1 should succeed");
        let r2 = apply_command_defs(&def_map, &make_create("ORD-SEQ-2"), &r1.state);
        assert!(!r2.rejected, "create 2 should succeed");
        let r3 = apply_command_defs(&def_map, &make_create("ORD-SEQ-3"), &r2.state);
        assert!(!r3.rejected, "create 3 should succeed");

        let log = ast::fetch_or_phi("audit_log", &r3.state);
        let entries = log.as_seq().expect("audit_log cell must exist after three applies");
        assert_eq!(entries.len(), 3, "three creates should yield three audit entries");
        assert_eq!(ast::binding(&entries[0], "sequence"), Some("0"));
        assert_eq!(ast::binding(&entries[1], "sequence"), Some("1"));
        assert_eq!(ast::binding(&entries[2], "sequence"), Some("2"));
        // Each entry carries its per-command sender (shape sanity).
        assert_eq!(ast::binding(&entries[0], "sender"), Some("u-ORD-SEQ-1"));
        assert_eq!(ast::binding(&entries[1], "sender"), Some("u-ORD-SEQ-2"));
        assert_eq!(ast::binding(&entries[2], "sender"), Some("u-ORD-SEQ-3"));
    }

    /// #26: commands without a sender must still produce a well-formed
    /// audit entry; the `sender` binding is present as an empty string.
    #[test]
    fn create_without_sender_audit_entry_has_empty_sender_binding() {
        let (def_map, state) = setup_order_defs();
        let mut fields = HashMap::new();
        fields.insert("orderNumber".to_string(), "ORD-NOSND".to_string());
        let cmd = Command::CreateEntity {
            noun: "Order".to_string(),
            domain: "orders".to_string(),
            id: Some("ORD-NOSND".to_string()),
            fields,
            sender: None,
            signature: None,
        };
        let result = apply_command_defs(&def_map, &cmd, &state);
        assert!(!result.rejected, "create without sender should still succeed");
        let log = ast::fetch_or_phi("audit_log", &result.state);
        let entries = log.as_seq().expect("audit_log cell must exist after apply");
        assert_eq!(entries.len(), 1, "one create should yield one audit entry");
        // Missing sender renders as "" — binding is present, not absent.
        assert_eq!(ast::binding(&entries[0], "sender"), Some(""));
        assert_eq!(ast::binding(&entries[0], "operation"), Some("apply:create"));
        assert_eq!(ast::binding(&entries[0], "outcome"), Some("ok"));
    }

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
}
