// crates/arest/src/generators/openapi.rs
//
// OpenAPI 3.1 generator: compile FFP state to an OpenAPI document.
//
// Scope is App-keyed, not Domain-keyed. An App lassos one or more
// Domains (organizations.md: `Domain belongs to App`). The FORML 2
// opt-in `App 'X' uses Generator 'openapi'.` is an assertion ON the
// App; a single compile may contain multiple Apps, each with its own
// opt-in decision. The generator therefore emits one document per App
// that opted in, keyed `openapi:{snake(app-slug)}`.
//
// AREST.tex Â§4.4 is the source of truth for what a document contains:
//   "RMAP determines which facts belong to which cell from the schema's
//    uniqueness constraints: the result is a 3NF row, the complete set
//    of facts that depend on one entity's key. Each entity is a cell."
//
// This generator CONSUMES rmap::rmap(domain) as the primary source of
// component schemas and does not re-derive attributes from
// fact_types/constraints/ref_schemes independently. Columns â†’ properties.
// `!nullable` â†’ `required`. `references` â†’ `$ref`. That is the whole
// schema side.
//
// State-machine status is orthogonal to RMAP (storage â‰  behavior) and
// contributes a separate `status` property with the status enum.
//
// Paths per entity are derived from Theorem 4 (HATEOAS as Projection):
//   - `/{plural}`          GET (list), POST (create)
//   - `/{plural}/{id}`     GET (read), PATCH (update)
//   - `/{plural}/{id}/transition` POST (event in body) â€” only if SM
//   - related-collection per binary fact type the noun participates in
//     (follow-up scope)
//
// No DELETE â€” per Â§4.1 and Corollary 2, deletion is a transition to a
// terminal status. The list endpoint filters out terminal entities via
// `Filter(p_live) : P` (server-side).
//
// Response envelope per Theorems 3 + 5 and Corollary 1:
//   `{ data, derived, violations, _links }` â€” follow-up scope.
//
// Design constraints (project rules):
//   - Pure FP style: iterator combinators, no for loops, no control-flow ifs.
//   - The function is total: missing cells yield a valid empty document.
//   - Output parses as valid JSON conforming to OpenAPI 3.1.

use hashbrown::HashMap;

use crate::ast::{Object, binding, fetch_or_phi};
use crate::rmap::{self, ColumnView};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// State-machine cell readers (#325). Replaces the earlier
// `state_machines_from_state -> HashMap<String, StateMachineDef>`
// typed-IR materialisation. Consumers read per-noun SM info via these
// three helpers directly, no typed struct in flight.

/// Resolve the SM name attached to a noun, if any.
fn sm_name_for_noun(state: &Object, noun_name: &str) -> Option<String> {
    fetch_or_phi("InstanceFact", state).as_seq()?
        .iter()
        .find(|f| binding(f, "subjectNoun") == Some("State Machine Definition")
            && binding(f, "fieldName").map(|s| s.contains("is for")).unwrap_or(false)
            && binding(f, "objectValue") == Some(noun_name))
        .and_then(|f| binding(f, "subjectValue").map(String::from))
}

/// Statuses for the SM attached to `noun_name`, in declaration order.
fn sm_statuses(state: &Object, noun_name: &str) -> Vec<String> {
    let Some(sm_name) = sm_name_for_noun(state, noun_name) else { return vec![]; };
    let inst = fetch_or_phi("InstanceFact", state);
    let Some(facts) = inst.as_seq() else { return vec![]; };
    let mut out: Vec<String> = Vec::new();
    for f in facts.iter().filter(|f|
        binding(f, "subjectNoun") == Some("Status")
        && binding(f, "fieldName").map(|s| s.contains("defined in")).unwrap_or(false)
        && binding(f, "objectValue") == Some(sm_name.as_str()))
    {
        if let Some(s) = binding(f, "subjectValue") {
            let s = s.to_string();
            if !out.contains(&s) { out.push(s); }
        }
    }
    out
}

/// Transitions for the SM attached to `noun_name` as
/// `(event, from, to)` tuples.
fn sm_transitions(state: &Object, noun_name: &str) -> Vec<(String, String, String)> {
    if sm_name_for_noun(state, noun_name).is_none() { return vec![]; }
    let inst = fetch_or_phi("InstanceFact", state);
    let Some(facts) = inst.as_seq() else { return vec![]; };
    let mut by_event: Vec<(String, String, String)> = Vec::new();
    for f in facts.iter().filter(|f| binding(f, "subjectNoun") == Some("Transition")) {
        let Some(event) = binding(f, "subjectValue").map(String::from) else { continue };
        let field = binding(f, "fieldName").unwrap_or("");
        let value = binding(f, "objectValue").unwrap_or("").to_string();
        let slot = by_event.iter_mut().find(|(e, _, _)| *e == event);
        match slot {
            Some((_, from, to)) => {
                if field.contains("from") { *from = value; }
                else if field.contains("to") { *to = value; }
            }
            None => {
                let mut from = String::new();
                let mut to = String::new();
                let mut ev = event.clone();
                if field.contains("from") { from = value; }
                else if field.contains("to") { to = value; }
                else if field.contains("triggered") { ev = value; }
                by_event.push((ev, from, to));
            }
        }
    }
    by_event
}

/// Compile state into an OpenAPI 3.1 JSON document for one App.
///
/// Public entry point matching the solidity/fpga generator signature.
/// Reads directly from state cells via `rmap_cells_from_state` and the
/// SM cell-reader helpers â€” no `state_to_domain` round-trip, no typed
/// IR struct in flight.
pub fn compile_to_openapi(state: &Object, app_name: &str) -> String {
    openapi_from_state(state, app_name).to_string()
}

/// Build the OpenAPI 3.1 document for one App from raw state (no Domain).
///
/// Used by `compile_to_openapi`. Reads nouns, fact types, instance facts,
/// enum values, and state machines directly from state cells.
fn openapi_from_state(state: &Object, app_name: &str) -> serde_json::Value {
    // RMAP as cells (#325): per-noun columns + PK come from the
    // `RMAPTable` / `RMAPColumn` cell readers. No typed-IR struct
    // crosses the generator boundary.
    let cells = rmap::rmap_cells_from_state(state);

    let nouns_cell = fetch_or_phi("Noun", state);
    let nouns_seq = nouns_cell.as_seq().unwrap_or(&[]);

    // noun_name -> objectType map
    let noun_types: HashMap<String, String> = nouns_seq.iter()
        .filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let obj_type = binding(n, "objectType").unwrap_or("entity").to_string();
            Some((name, obj_type))
        })
        .collect();

    // noun_name -> enum values (from "enumValues" binding on Noun cell)
    let enum_values: HashMap<String, Vec<String>> = nouns_seq.iter()
        .filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let vals = binding(n, "enumValues")?;
            let v: Vec<String> = vals.split(',').map(|s| s.to_string()).collect();
            Some((name, v))
        })
        .collect();

    // snake(noun_name) -> noun_name for enum lookup in column_property
    let noun_by_snake: HashMap<String, String> = noun_types.keys()
        .map(|n| (rmap::to_snake(n), n.clone()))
        .collect();

    // InstanceFact cell for general_instance_facts (plural / app description)
    let inst_cell = fetch_or_phi("InstanceFact", state);
    let inst_seq = inst_cell.as_seq().unwrap_or(&[]);

    let mut schemas: serde_json::Map<String, serde_json::Value> = noun_types.iter()
        .filter(|(_, obj_type)| obj_type.as_str() == "entity")
        .filter_map(|(name, _)| {
            let table_name = rmap::to_snake(name);
            let cols = rmap::columns_for_table(&cells, &table_name);
            if cols.is_empty() { return None; }
            Some((name.clone(), component_schema_from_state(name, &cols, &noun_by_snake, &enum_values, state)))
        })
        .collect();

    schemas.entry("Violation".to_string())
        .or_insert_with(violation_component_schema);

    // FactType + Role cells for Theorem 4b navigation
    let ft_cell = fetch_or_phi("FactType", state);
    let ft_seq = ft_cell.as_seq().unwrap_or(&[]);
    let role_cell = fetch_or_phi("Role", state);
    let role_seq = role_cell.as_seq().unwrap_or(&[]);

    let paths: serde_json::Map<String, serde_json::Value> = noun_types.iter()
        .filter(|(_, obj_type)| obj_type.as_str() == "entity")
        .flat_map(|(name, _)| {
            let table_name = rmap::to_snake(name);
            let cols = rmap::columns_for_table(&cells, &table_name);
            if cols.is_empty() { return Vec::new(); }
            let plural = plural_for_noun_from_state(name, inst_seq);
            paths_for_noun_from_state(name, &plural, state, &noun_types, inst_seq, ft_seq, role_seq, &cols)
        })
        .collect();

    let app_description = app_description_from_state(inst_seq, app_name)
        .unwrap_or_else(|| format!("Compiled from FORML2 readings for App '{}'.", app_name));

    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": app_name,
            "version": "1.0.0",
            "description": app_description,
        },
        "paths": paths,
        "components": {
            "schemas": schemas,
        },
    })
}

/// Build the OpenAPI 3.1 document for one App as a `serde_json::Value`.
///
/// An App is the unit of API product identity â€” the `info.title` is the
/// App, the `info.description` comes from the App's instance facts when
/// declared. Nouns and paths are drawn from the full compile: today
/// there is no structured nounâ†’domain mapping, so every entity in the
/// compile contributes to every App's document. Future work can narrow
/// this via `Domain belongs to App` + a nounâ†’domain trace, at which
/// point the per-App cell will specialize further.
///
/// `pub(crate)` so `compile.rs` can register the document cell without
/// round-tripping through state for every App.
#[cfg(test)]
pub(crate) fn openapi_for_app(state: &Object, app_name: &str) -> serde_json::Value {
    openapi_from_state(state, app_name)
}

/// State-based variant of `app_description`. Reads from the InstanceFact
/// cell slice directly â€” no Domain round-trip.
fn app_description_from_state(inst_seq: &[Object], app_name: &str) -> Option<String> {
    inst_seq.iter()
        .find(|f| binding(f, "subjectNoun") == Some("App")
            && binding(f, "subjectValue") == Some(app_name)
            && binding(f, "fieldName") == Some("Description"))
        .and_then(|f| binding(f, "objectValue").map(|s| s.to_string()))
}

/// Resolve the plural slug for a noun by reading `Noun has Plural`
/// instance facts directly. Falls back to `snake(noun) + "s"` when no
/// plural was declared.
fn plural_for_noun_from_state(noun_name: &str, inst_seq: &[Object]) -> String {
    inst_seq.iter()
        .find(|f| binding(f, "subjectNoun") == Some("Noun")
            && binding(f, "subjectValue") == Some(noun_name)
            && binding(f, "fieldName") == Some("Plural"))
        .and_then(|f| binding(f, "objectValue").map(|s| s.to_string()))
        .unwrap_or_else(|| format!("{}s", rmap::to_snake(noun_name)))
}

/// Default Violation component schema â€” the wire shape of a failed
/// constraint. Corollary 1 guarantees that `reading` carries the
/// original FORML 2 sentence verbatim. A loaded `readings/outcomes.md`
/// produces its own Violation schema via RMAP; that one wins when the
/// user's app lassos outcomes.
fn violation_component_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "title": "Violation",
        "description": "A constraint violation. The `reading` field is the original \
                        FORML 2 sentence per Corollary 1 (Violation Verbalization).",
        "properties": {
            "reading": {
                "type": "string",
                "description": "The original FORML 2 reading whose compiled constraint \
                                this violation reports. Round-trips parse âˆ˜ compile.",
            },
            "constraintId": {
                "type": "string",
                "description": "The compiled constraint identifier.",
            },
            "modality": {
                "type": "string",
                "enum": ["alethic", "deontic"],
                "description": "Alethic violations reject the command; deontic \
                                violations are reported alongside the accepted \
                                command (paper Â§4.1).",
            },
            "detail": {
                "type": "string",
                "description": "Optional tuple-level detail: which instance triggered the \
                                violation. Empty when the constraint is over the \
                                schema rather than a specific fact.",
            },
        },
        "required": ["reading", "constraintId", "modality"],
    })
}

/// Shared `_links` sub-schema for response envelopes.
///
/// Theorem 4 projects two link sets: transitions (SM events valid from
/// the current status) and navigation (related/parent/child/peer
/// references as Î¸â‚ projections). Clients drive action from this
/// sub-structure; the envelope always carries it.
fn links_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "HATEOAS links per Theorem 4 â€” all are Î¸â‚ projections over P and S.",
        "properties": {
            "transitions": {
                "type": "array",
                "description": "Events valid from the entity's current status. \
                                Theorem 4a: Ï€_event(Filter(s_from âˆˆ {current} âˆª \
                                supertypes):T).",
                "items": {
                    "type": "object",
                    "properties": {
                        "event": { "type": "string" },
                        "href":  { "type": "string", "format": "uri-reference" },
                        "method": { "type": "string", "enum": ["POST"] },
                    },
                    "required": ["event", "href", "method"],
                },
            },
            "navigation": {
                "type": "object",
                "description": "Related/parent/child/peer URIs per Theorem 4b.",
                "additionalProperties": {
                    "type": "string",
                    "format": "uri-reference",
                },
            },
        },
    })
}

/// Wrap a data schema in the Theorem 5 representation envelope.
///
/// `repr(e, P, S) = {Ï(s):facts} âˆª {Ï(r):P} âˆª {Ï(c):P} âˆª links_full`.
/// Four keys: `data` (the 3NF row or list), `derived` (rule outputs â€”
/// only for single-entity reads), `violations` (Cor 1-verbalized),
/// `_links` (Theorem 4). `_links` and `data` are required; `derived`
/// and `violations` are optional because not every response carries
/// them (pagination pages, for instance, may have neither).
fn envelope_schema(data_schema: serde_json::Value, include_derived: bool) -> serde_json::Value {
    let violation_ref = serde_json::json!({
        "type": "array",
        "items": { "$ref": "#/components/schemas/Violation" },
    });
    let mut props = serde_json::Map::new();
    props.insert("data".to_string(), data_schema);
    if include_derived {
        props.insert("derived".to_string(), serde_json::json!({
            "type": "object",
            "description": "Derivation-rule outputs for this entity â€” every value is a \
                            Ï-application of a derivation rule over P (Theorem 5).",
            "additionalProperties": true,
        }));
    }
    props.insert("violations".to_string(), violation_ref);
    props.insert("_links".to_string(), links_schema());
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": ["data", "_links"],
    })
}

/// State-based variant of `paths_for_noun`. Takes noun_types, inst_seq,
/// ft_seq, and role_seq slices directly â€” no Domain round-trip.
fn paths_for_noun_from_state(
    noun_name: &str,
    plural: &str,
    state: &Object,
    noun_types: &HashMap<String, String>,
    inst_seq: &[Object],
    ft_seq: &[Object],
    role_seq: &[Object],
    columns: &[ColumnView],
) -> Vec<(String, serde_json::Value)> {
    let schema_ref = serde_json::json!({
        "$ref": format!("#/components/schemas/{}", noun_name),
    });
    let list_envelope = envelope_schema(
        serde_json::json!({ "type": "array", "items": schema_ref }),
        false,
    );
    let item_envelope = envelope_schema(schema_ref.clone(), true);
    let list_response = serde_json::json!({
        "200": {
            "description": format!("List of {}. Envelope per Theorem 5.", noun_name),
            "content": {
                "application/json": { "schema": list_envelope },
            },
        },
    });
    let item_response = serde_json::json!({
        "200": {
            "description": format!("One {}. Envelope per Theorem 5.", noun_name),
            "content": {
                "application/json": { "schema": item_envelope },
            },
        },
    });
    let request_body = serde_json::json!({
        "required": true,
        "content": {
            "application/json": { "schema": schema_ref },
        },
    });
    let id_param = serde_json::json!({
        "name": "id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
    });

    // #218: list-endpoint sort/order query parameters enumerated
    // over the noun's RMAP-derived columns. Each noun's list route
    // documents exactly which fields a client may sort on — the
    // cross-product of schema fields × {asc, desc} — so tooling
    // (OpenAPI code-gen, ui.do table widgets) can render valid
    // sort UI without guessing. Sorting is bound by Halpin's 3NF
    // row shape: only scalar columns of the entity's own table
    // qualify, never joined FTs (those get their own list route).
    let sort_fields: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let list_params: Vec<serde_json::Value> = if sort_fields.is_empty() {
        Vec::new()
    } else {
        vec![
            serde_json::json!({
                "name": "sort",
                "in": "query",
                "required": false,
                "description": format!(
                    "Field to sort {} by. Enumerates the noun's RMAP columns (§5.4).",
                    noun_name,
                ),
                "schema": {
                    "type": "string",
                    "enum": sort_fields,
                },
            }),
            serde_json::json!({
                "name": "order",
                "in": "query",
                "required": false,
                "description": "Sort direction. Ignored when `sort` is omitted.",
                "schema": {
                    "type": "string",
                    "enum": ["asc", "desc"],
                    "default": "asc",
                },
            }),
        ]
    };

    let mut list_get = serde_json::json!({
        "summary": format!("List {}.", noun_name),
        "responses": list_response,
    });
    if !list_params.is_empty() {
        list_get.as_object_mut().unwrap()
            .insert("parameters".to_string(), serde_json::Value::Array(list_params));
    }

    let crud = vec![
        (format!("/{}", plural), serde_json::json!({
            "get":  list_get,
            "post": { "summary": format!("Create {}.", noun_name), "requestBody": request_body, "responses": item_response },
        })),
        (format!("/{}/{{id}}", plural), serde_json::json!({
            "parameters": [id_param.clone()],
            "get":   { "summary": format!("Read {}.", noun_name),   "responses": item_response },
            "patch": { "summary": format!("Update {}.", noun_name), "requestBody": request_body, "responses": item_response },
        })),
    ];

    let sm_trans = sm_transitions(state, noun_name);
    let transitions: Vec<(String, serde_json::Value)> = if sm_trans.is_empty() {
        vec![]
    } else {
        let events: Vec<String> = sm_trans.iter().map(|(e, _, _)| e.clone()).collect();
        let events: Vec<&str> = events.iter().map(|s| s.as_str()).collect();
        let fire_request = serde_json::json!({
            "required": true,
            "description": "Fire a transition by event name.",
            "content": {
                "application/json": {
                    "schema": {
                        "type": "object",
                        "required": ["event"],
                        "properties": {
                            "event": { "type": "string", "enum": events },
                        },
                    },
                },
            },
        });
        let events_response = serde_json::json!({
            "200": {
                "description": format!("Events valid from the current status of this {}.", noun_name),
                "content": {
                    "application/json": {
                        "schema": { "type": "array", "items": { "type": "string" } },
                    },
                },
            },
        });
        vec![
            (format!("/{}/{{id}}/transition", plural), serde_json::json!({
                "parameters": [id_param.clone()],
                "post": {
                    "summary": format!("Fire a transition on a {}.", noun_name),
                    "requestBody": fire_request,
                    "responses": item_response,
                },
            })),
            (format!("/{}/{{id}}/transitions", plural), serde_json::json!({
                "parameters": [id_param.clone()],
                "get": {
                    "summary": format!("Transitions available from the current status of a {}.", noun_name),
                    "responses": events_response,
                },
            })),
        ]
    };

    // Theorem 4b navigation from state cells
    let participations: Vec<(String, String)> = ft_seq.iter().filter_map(|f| {
        let ft_id = binding(f, "id")?;
        let reading = binding(f, "reading").unwrap_or("").to_string();
        let ft_roles: Vec<&str> = role_seq.iter()
            .filter(|r| binding(r, "factType") == Some(ft_id))
            .filter_map(|r| binding(r, "nounName"))
            .collect();
        if ft_roles.len() != 2 { return None; }
        if !ft_roles.iter().any(|r| *r == noun_name) { return None; }
        let other_name = ft_roles.iter()
            .find(|r| **r != noun_name)
            .map(|r| r.to_string())
            .unwrap_or_else(|| ft_roles[0].to_string());
        // other must be an entity
        if noun_types.get(&other_name).map(|t| t.as_str()) != Some("entity") { return None; }
        Some((other_name, reading))
    }).collect();

    let mut by_other: HashMap<String, Vec<String>> = HashMap::new();
    participations.into_iter().for_each(|(other, reading)| {
        by_other.entry(other).or_default().push(reading);
    });

    let noun_names: Vec<&str> = noun_types.keys().map(|s| s.as_str()).collect();
    let id_param_for_related = id_param.clone();

    let related_routes: Vec<(String, serde_json::Value)> = by_other.iter()
        .flat_map(|(other_noun, readings)| {
            let other_plural = plural_for_noun_from_state(other_noun, inst_seq);
            let is_ring = other_noun == noun_name;
            let multiple = readings.len() > 1;
            readings.iter().map(|reading| {
                let slug = if is_ring {
                    verb_slug_from_reading(reading, &noun_names)
                } else if multiple {
                    format!("{}-{}",
                        verb_slug_from_reading(reading, &noun_names),
                        other_plural)
                } else {
                    other_plural.clone()
                };
                let other_ref = serde_json::json!({
                    "$ref": format!("#/components/schemas/{}", other_noun),
                });
                let list_env = envelope_schema(
                    serde_json::json!({ "type": "array", "items": other_ref }),
                    false,
                );
                (
                    format!("/{}/{{id}}/{}", plural, slug),
                    serde_json::json!({
                        "parameters": [id_param_for_related.clone()],
                        "get": {
                            "summary": format!("{} (Theorem 4b).", reading),
                            "responses": {
                                "200": {
                                    "description": format!(
                                        "{} entities reached via `{}`. Envelope per Theorem 5.",
                                        other_noun, reading),
                                    "content": {
                                        "application/json": { "schema": list_env },
                                    },
                                },
                            },
                        },
                    }),
                )
            }).collect::<Vec<_>>()
        })
        .collect();

    let actions_route: Vec<(String, serde_json::Value)> = if sm_trans.is_empty() {
        vec![]
    } else {
        let events: Vec<String> = sm_trans.iter().map(|(e, _, _)| e.clone()).collect();
        let events: Vec<&str> = events.iter().map(|s| s.as_str()).collect();
        let events_response = serde_json::json!({
            "200": {
                "description": format!("Events (actions) valid from the current status of this {}.", noun_name),
                "content": {
                    "application/json": {
                        "schema": { "type": "array", "items": { "type": "string", "enum": &events } },
                    },
                },
            },
        });
        vec![(
            format!("/{}/{{id}}/actions", plural),
            serde_json::json!({
                "parameters": [id_param.clone()],
                "get": {
                    "summary": format!("List valid actions (SM events) for a {}.", noun_name),
                    "description": "Alias of /transitions; named to match the MCP `actions` verb.",
                    "responses": events_response,
                },
            }),
        )]
    };

    let explain_response = serde_json::json!({
        "200": {
            "description": format!(
                "Derivation chain for all derived facts on this {}. \
                 Theorem 5: every value in the representation is a Ï-application \
                 over P; /explain surfaces the chain of rules and antecedents \
                 that produced each derived fact.",
                noun_name),
            "content": {
                "application/json": {
                    "schema": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "factTypeId": { "type": "string" },
                                "rule":       { "type": "string" },
                                "bindings":   { "type": "object", "additionalProperties": true },
                                "antecedents": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "factTypeId": { "type": "string" },
                                            "bindings":   { "type": "object", "additionalProperties": true },
                                            "source":     { "type": "string", "enum": ["asserted", "derived"] },
                                        },
                                    },
                                },
                            },
                            "required": ["factTypeId", "rule"],
                        },
                    },
                },
            },
        },
    });
    let explain_route = (
        format!("/{}/{{id}}/explain", plural),
        serde_json::json!({
            "parameters": [id_param.clone()],
            "get": {
                "summary": format!("Explain derived facts on a {}.", noun_name),
                "description": "Returns the derivation chain per Theorem 5 â€” rule name, \
                                bindings, and antecedents (asserted or derived) for every \
                                derived fact the entity participates in.",
                "responses": explain_response,
            },
        }),
    );

    crud.into_iter()
        .chain(transitions)
        .chain(related_routes)
        .chain(actions_route)
        .chain(core::iter::once(explain_route))
        .collect()
}


/// Extract a kebab-case verb slug from a binary fact type's reading.
///
/// Strategy: tokenize the reading, drop the longest-first noun matches,
/// keep what's left, lowercase-kebab-case the residue. Handles
/// compound nouns ("State Machine Definition") via longest-match.
///
/// "Customer owns Account"        â†’ "owns"
/// "Order was placed by Customer" â†’ "was-placed-by"
/// "Employee reports to Employee" â†’ "reports-to"
fn verb_slug_from_reading(reading: &str, noun_names: &[&str]) -> String {
    // Sort noun_names descending by whitespace-token count so longer
    // names match before shorter prefixes of themselves.
    let mut sorted: Vec<&str> = noun_names.to_vec();
    sorted.sort_by_key(|n| core::cmp::Reverse(n.split_whitespace().count()));

    let tokens: Vec<&str> = reading.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let matched = sorted.iter().find(|noun| {
            let noun_tokens: Vec<&str> = noun.split_whitespace().collect();
            i + noun_tokens.len() <= tokens.len()
                && tokens[i..i + noun_tokens.len()].iter()
                    .zip(noun_tokens.iter()).all(|(a, b)| a == b)
        });
        match matched {
            Some(noun) => { i += noun.split_whitespace().count(); }
            None => {
                out.push(tokens[i].trim_end_matches('.').to_lowercase());
                i += 1;
            }
        }
    }
    out.join("-")
}

/// State-based variant of `component_schema`. Uses enum_values and sms
/// HashMaps derived from state cells rather than `&Domain`.
fn component_schema_from_state(
    noun_name: &str,
    columns: &[ColumnView],
    noun_by_snake: &HashMap<String, String>,
    enum_values: &HashMap<String, Vec<String>>,
    state: &Object,
) -> serde_json::Value {
    let column_props = columns.iter()
        .map(|col| (col.name.clone(), column_property_from_state(col, noun_by_snake, enum_values)));

    // SM-derived "status" property, if this noun has a state machine.
    let statuses = sm_statuses(state, noun_name);
    let sm_props: Box<dyn Iterator<Item = (String, serde_json::Value)>> = if statuses.is_empty() {
        Box::new(core::iter::empty())
    } else {
        Box::new(core::iter::once((
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": statuses,
            }),
        )))
    };

    let properties: serde_json::Map<String, serde_json::Value> =
        column_props.chain(sm_props).collect();

    let required: Vec<String> = columns.iter()
        .filter(|c| !c.nullable)
        .map(|c| c.name.clone())
        .collect();

    serde_json::json!({
        "type": "object",
        "title": noun_name,
        "properties": properties,
        "required": required,
    })
}

/// Map a RMAP column to a JSON Schema property.
///
/// FK columns emit `$ref` into `components.schemas.{Target}`. Value-type
/// columns with declared enum values emit `{type, enum}`. Other value
/// columns emit a scalar type derived from the SQL `col_type`.
/// State-based variant of `column_property`. Uses `enum_values` HashMap
/// derived directly from the Noun cell rather than `domain.enum_values`.
fn column_property_from_state(
    col: &ColumnView,
    noun_by_snake: &HashMap<String, String>,
    enum_values: &HashMap<String, Vec<String>>,
) -> serde_json::Value {
    col.references.as_ref()
        .map(|target| serde_json::json!({
            "$ref": format!("#/components/schemas/{}", target),
        }))
        .unwrap_or_else(|| {
            let source_noun = noun_by_snake.get(&col.name);
            let enum_vals = source_noun.and_then(|n| enum_values.get(n));
            match enum_vals {
                Some(vals) => serde_json::json!({
                    "type": sql_type_to_json(&col.col_type),
                    "enum": vals,
                }),
                None => serde_json::json!({
                    "type": sql_type_to_json(&col.col_type),
                }),
            }
        })
}

/// Map a SQL type string to a JSON Schema scalar type.
///
/// Coarse mapping covering the common RMAP outputs. Unknown types fall
/// back to "string" so the function remains total.
fn sql_type_to_json(sql_type: &str) -> &'static str {
    match sql_type.to_uppercase().as_str() {
        "INTEGER" | "BIGINT" | "SMALLINT" => "integer",
        "REAL" | "NUMERIC" | "DECIMAL" | "DOUBLE" | "FLOAT" => "number",
        "BOOLEAN" | "BOOL" => "boolean",
        _ => "string",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_domain_emits_valid_openapi_3_1_document() {
        let doc = openapi_for_app(&Object::phi(), "test-app");

        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], "1.0.0");
        assert!(doc["info"]["title"].is_string());
        assert!(doc["paths"].is_object());
        let schemas = doc["components"]["schemas"].as_object()
            .expect("components.schemas must be an object");
        // Violation is unconditional â€” every envelope references it.
        assert_eq!(schemas.keys().cloned().collect::<Vec<_>>(), vec!["Violation"],
            "empty domain emits only the Violation envelope type; got: {:?}",
            schemas.keys().collect::<Vec<_>>());
    }

    use crate::ast::fact_from_pairs;

    /// Parse a FORML2 snippet into Object state for tests.
    fn parse(src: &str) -> Object {
        crate::parse_forml2::parse_to_state(src)
            .expect("test FORML2 must parse")
    }

    fn push_instance_fact(
        mut state: Object, subject_noun: &str, subject_value: &str,
        field_name: &str, object_noun: &str, object_value: &str,
    ) -> Object {
        let inst = fact_from_pairs(&[
            ("subjectNoun", subject_noun), ("subjectValue", subject_value),
            ("fieldName", field_name), ("objectNoun", object_noun),
            ("objectValue", object_value),
        ]);
        if let Object::Map(ref mut m) = state {
            let mut v: Vec<Object> = m.get("InstanceFact")
                .and_then(|o| o.as_seq())
                .map(|s| s.to_vec())
                .unwrap_or_default();
            v.push(inst);
            m.insert("InstanceFact".into(), Object::Seq(v.into()));
        }
        state
    }

    /// Push SM instance-fact rows onto `state`: one "State Machine
    /// Definition … is for …" row, one "Status … defined in …" per
    /// status, two rows ("from" / "to") per transition. Matches the
    /// shape the parser produces and that `sm_*` helpers read.
    fn push_state_machine(
        state: Object, sm_name: &str, noun_name: &str,
        statuses: &[&str], transitions: &[(&str, &str, &str)], // (from, to, event)
    ) -> Object {
        let mut rows: Vec<Object> = Vec::new();
        rows.push(fact_from_pairs(&[
            ("subjectNoun", "State Machine Definition"),
            ("subjectValue", sm_name),
            ("fieldName", "is for"),
            ("objectValue", noun_name),
        ]));
        for s in statuses {
            rows.push(fact_from_pairs(&[
                ("subjectNoun", "Status"),
                ("subjectValue", s),
                ("fieldName", "defined in"),
                ("objectValue", sm_name),
            ]));
        }
        for (from, to, event) in transitions {
            rows.push(fact_from_pairs(&[
                ("subjectNoun", "Transition"),
                ("subjectValue", event),
                ("fieldName", "from"),
                ("objectValue", from),
            ]));
            rows.push(fact_from_pairs(&[
                ("subjectNoun", "Transition"),
                ("subjectValue", event),
                ("fieldName", "to"),
                ("objectValue", to),
            ]));
        }
        let mut state = state;
        if let Object::Map(ref mut m) = state {
            let mut v: Vec<Object> = m.get("InstanceFact")
                .and_then(|o| o.as_seq())
                .map(|s| s.to_vec())
                .unwrap_or_default();
            v.extend(rows);
            m.insert("InstanceFact".into(), Object::Seq(v.into()));
        }
        state
    }

    fn organization_with_slug() -> Object {
        // RMAP needs the fact type backing the ref scheme to materialize
        // a column. The Organization(.Slug) declaration is the
        // reference-scheme shorthand; the binary fact + UC is the
        // explicit form RMAP folds into a single-column table.
        parse("\
            Organization(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Organization has Slug.\n\
              Each Organization has exactly one Slug.\n\
        ")
    }

    #[test]
    fn entity_schema_properties_come_from_rmap_table_columns() {
        let state = organization_with_slug();

        let doc = openapi_for_app(&state, "test-app");
        let schema = &doc["components"]["schemas"]["Organization"];

        assert_eq!(schema["type"], "object");
        assert_eq!(schema["title"], "Organization");
        // RMAP absorbs the single-value reference scheme (Slug) into the
        // entity's primary key column (`id` by convention). The generator
        // surfaces whatever columns RMAP produced as schema properties.
        let props = schema["properties"].as_object()
            .expect("properties must be an object");
        assert!(!props.is_empty(),
            "schema must have at least one property derived from RMAP; got: {}",
            schema);
        assert!(props.contains_key("id"),
            "RMAP-produced primary key column 'id' must be a property; got: {:?}",
            props.keys().collect::<Vec<_>>());

        let required = schema["required"].as_array()
            .expect("required must be an array");
        assert!(required.iter().any(|v| v == "id"),
            "'id' must be required (non-nullable primary key); got required: {:?}",
            required);
    }

    #[test]
    fn entity_produces_list_and_item_paths() {
        // Theorem 4 (HATEOAS as Projection) mandates per-entity CRUD routes.
        // The plural slug falls back to snake(noun) + "s" when no
        // `Noun has Plural` instance fact overrides it.
        let state = organization_with_slug();

        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object()
            .expect("paths must be an object");

        let list_key = "/organizations";
        assert!(paths.contains_key(list_key),
            "list path {:?} must exist; got: {:?}",
            list_key, paths.keys().collect::<Vec<_>>());
        assert!(paths[list_key]["get"].is_object(),
            "GET {} (list) must be defined", list_key);
        assert!(paths[list_key]["post"].is_object(),
            "POST {} (create) must be defined", list_key);

        let item_key = "/organizations/{id}";
        assert!(paths.contains_key(item_key),
            "item path {:?} must exist; got: {:?}",
            item_key, paths.keys().collect::<Vec<_>>());
        assert!(paths[item_key]["get"].is_object(),
            "GET {} (read) must be defined", item_key);
        assert!(paths[item_key]["patch"].is_object(),
            "PATCH {} (update) must be defined", item_key);
    }

    /// #218: the list-GET endpoint advertises sort + order query
    /// parameters, with `sort` enumerated over the noun's RMAP
    /// columns and `order` enumerated over {asc, desc}. Tooling
    /// can then render valid sort UI without extra introspection.
    #[test]
    fn list_endpoint_emits_sort_and_order_params() {
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let list_get = &doc["paths"]["/organizations"]["get"];
        let params = list_get["parameters"].as_array()
            .expect("list GET must carry a `parameters` array");

        let sort = params.iter()
            .find(|p| p["name"] == "sort")
            .expect("sort parameter must be present on list GET");
        assert_eq!(sort["in"], "query");
        let sort_enum = sort["schema"]["enum"].as_array()
            .expect("sort schema must be an enum of RMAP columns");
        // Organization's RMAP table always has at least the `id`
        // column (entity key) — stronger assertions live in the
        // richer fixtures; here we just pin the contract.
        assert!(sort_enum.iter().any(|v| v == "id"),
            "sort enum must include the primary-key column; got {:?}", sort_enum);

        let order = params.iter()
            .find(|p| p["name"] == "order")
            .expect("order parameter must be present on list GET");
        let order_enum = order["schema"]["enum"].as_array()
            .expect("order must enumerate direction values");
        assert_eq!(order_enum.len(), 2);
        assert!(order_enum.iter().any(|v| v == "asc"));
        assert!(order_enum.iter().any(|v| v == "desc"));
    }

    #[test]
    fn plural_instance_fact_overrides_fallback() {
        // `Noun 'X' has Plural 'ys'` is how irregular plurals ("policies",
        // "categories", "children") reach the path surface. Without this
        // override, snake(noun) + "s" mangles most non-regular nouns.
        // The instance fact lives as a GeneralInstanceFact against the
        // metamodel's `Noun has Plural` binary â€” facts all the way down.
        let state = push_instance_fact(
            organization_with_slug(),
            "Noun", "Organization", "Plural", "Plural", "orgs",
        );

        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object()
            .expect("paths must be an object");

        assert!(paths.contains_key("/orgs"),
            "plural-fact path /orgs must exist when 'Noun has Plural orgs' is \
             declared; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths.contains_key("/orgs/{id}"),
            "plural-fact item path /orgs/{{id}} must exist; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(!paths.contains_key("/organizations"),
            "fallback path /organizations must not exist once Plural is \
             declared â€” the declaration wins; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn noun_with_state_machine_has_transition_routes() {
        // Theorem 4a: transition links are a projection over the transition
        // fact set filtered to `from âˆˆ {current} âˆª supertypes(current)`.
        // At the OpenAPI surface that projection materializes as two
        // routes on the entity: POST /transition to fire an event, and
        // GET /transitions to list the events valid from the current
        // status. They only exist when the noun has a State Machine
        // Definition; a status-less noun has no transitions to project.
        let state = push_state_machine(
            organization_with_slug(),
            "Organization Lifecycle", "Organization",
            &["active", "archived"],
            &[("active", "archived", "archive")],
        );

        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object()
            .expect("paths must be an object");

        let fire_key = "/organizations/{id}/transition";
        assert!(paths.contains_key(fire_key),
            "POST transition path must exist for SM-bearing noun; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths[fire_key]["post"].is_object(),
            "POST {} (fire transition) must be defined", fire_key);

        let list_key = "/organizations/{id}/transitions";
        assert!(paths.contains_key(list_key),
            "GET transitions path must exist for SM-bearing noun; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths[list_key]["get"].is_object(),
            "GET {} (available transitions) must be defined", list_key);
    }

    #[test]
    fn noun_without_state_machine_has_no_transition_routes() {
        // A status-less noun has no transition fact set to project (Thm 4a).
        // Emitting transition routes in that case would advertise an API
        // that cannot be fulfilled â€” the handler would 404 on every call.
        let state = organization_with_slug();

        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object()
            .expect("paths must be an object");

        assert!(!paths.contains_key("/organizations/{id}/transition"),
            "transition route must be absent without an SM; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(!paths.contains_key("/organizations/{id}/transitions"),
            "transitions route must be absent without an SM; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn doc_includes_violation_component_with_reading_text() {
        // Theorem 5 / Corollary 1: every operation response may carry
        // violations, and each violation's body IS the original FORML 2
        // reading (by the injectivity of parse âˆ˜ compile). The OpenAPI
        // document must therefore declare a Violation component schema
        // that exposes the reading text as a field so tools generate
        // clients capable of surfacing the original sentence.
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let schemas = doc["components"]["schemas"].as_object()
            .expect("components.schemas must be an object");
        assert!(schemas.contains_key("Violation"),
            "Violation component schema must be declared; got: {:?}",
            schemas.keys().collect::<Vec<_>>());
        let violation = &schemas["Violation"];
        assert_eq!(violation["type"], "object");
        let props = violation["properties"].as_object()
            .expect("Violation.properties must be an object");
        assert!(props.contains_key("reading"),
            "Violation must carry a 'reading' field per Cor 1; got: {:?}",
            props.keys().collect::<Vec<_>>());
        assert!(props.contains_key("constraintId"),
            "Violation must carry 'constraintId' so clients can correlate; \
             got: {:?}", props.keys().collect::<Vec<_>>());
        assert!(props.contains_key("modality"),
            "Violation must carry 'modality' (alethic|deontic) so clients \
             know whether the violation rejected the command or merely \
             warned; got: {:?}", props.keys().collect::<Vec<_>>());
    }

    #[test]
    fn item_response_wraps_entity_in_envelope_per_theorem_5() {
        // Theorem 5 repr(e, P, S) = {Ï(s):f | facts} âˆª {Ï(r):P | rules}
        //                        âˆª {Ï(c):P | constraints} âˆª links_full.
        // Four top-level keys: data, derived, violations, _links.
        // Not three, not collapsed. This matches the Backus Â§13.3.2
        // representation function and preserves provenance.
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let item_schema = &doc["paths"]["/organizations/{id}"]["get"]
            ["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(item_schema["type"], "object",
            "item response envelope must be an object, got: {}", item_schema);
        let props = item_schema["properties"].as_object()
            .expect("envelope must have properties");
        ["data", "derived", "violations", "_links"].iter().for_each(|k| {
            assert!(props.contains_key(*k),
                "envelope must carry '{}' per Theorem 5; got: {:?}",
                k, props.keys().collect::<Vec<_>>());
        });
        // data is the 3NF row â€” a ref to the noun schema
        let data = &item_schema["properties"]["data"];
        assert!(data.get("$ref").is_some() || data["type"] == "object",
            "envelope.data must be the noun row (schema $ref or inline object); got: {}", data);
    }

    #[test]
    fn list_response_wraps_array_in_envelope_per_theorem_5() {
        // List responses carry the same envelope; `data` is an array.
        // Pagination + query-level violations are reported alongside.
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let list_schema = &doc["paths"]["/organizations"]["get"]
            ["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(list_schema["type"], "object");
        let props = list_schema["properties"].as_object()
            .expect("list envelope must have properties");
        assert!(props.contains_key("data"));
        assert!(props.contains_key("violations"));
        assert!(props.contains_key("_links"));
        assert_eq!(list_schema["properties"]["data"]["type"], "array",
            "list envelope's data must be an array of entity rows; got: {}",
            list_schema);
    }

    #[test]
    fn binary_fact_types_emit_related_collection_routes_per_theorem_4b() {
        // Theorem 4b: for each binary fact type f that noun n participates
        // in, f contributes a "related collection on n, filtered by n"
        // (always applies). The OpenAPI surface is
        // `/{plural-n}/{id}/{plural-other}` GET listing the other-side
        // entities participating with the given n instance.
        //
        // `Customer owns Account` â€” Customer and Account each get a
        // navigation toward the other in its path space.
        let state = parse("\
            Customer(.Slug) is an entity type.\n\
            Account(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Customer has Slug.\n\
              Each Customer has exactly one Slug.\n\
            Account has Slug.\n\
              Each Account has exactly one Slug.\n\
            Customer owns Account.\n\
        ");
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().expect("paths must be object");

        let c_to_a = "/customers/{id}/accounts";
        let a_to_c = "/accounts/{id}/customers";
        assert!(paths.contains_key(c_to_a),
            "Customer's related-collection for Account must exist; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths[c_to_a]["get"].is_object(),
            "GET {} must be defined", c_to_a);
        assert!(paths.contains_key(a_to_c),
            "Account's related-collection for Customer must exist; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths[a_to_c]["get"].is_object(),
            "GET {} must be defined", a_to_c);
    }

    #[test]
    fn ring_fact_type_emits_verb_slug_path_per_theorem_4b() {
        // `Employee reports to Employee` â€” both roles on Employee.
        // The forward direction gets a verb-slug path because the
        // other-plural would collide with this plural.
        let state = parse("\
            Employee(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Employee has Slug.\n\
              Each Employee has exactly one Slug.\n\
            Employee reports to Employee.\n\
        ");
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().expect("paths must be object");
        let ring_key = "/employees/{id}/reports-to";
        assert!(paths.contains_key(ring_key),
            "ring FT must emit verb-slug path; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn multiple_fts_same_pair_disambiguate_via_verb_slug() {
        // Two binary FTs between Customer and Account:
        //   Customer owns Account
        //   Customer bills Account
        // Each must emit its own route; the dedupe trap would have
        // dropped one. Verb slug distinguishes them.
        let state = parse("\
            Customer(.Slug) is an entity type.\n\
            Account(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Customer has Slug.\n\
              Each Customer has exactly one Slug.\n\
            Account has Slug.\n\
              Each Account has exactly one Slug.\n\
            Customer owns Account.\n\
            Customer bills Account.\n\
        ");
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().expect("paths must be object");
        assert!(paths.contains_key("/customers/{id}/owns-accounts"),
            "verb-slugged route for 'owns' must exist; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths.contains_key("/customers/{id}/bills-accounts"),
            "verb-slugged route for 'bills' must exist; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn introspection_routes_emit_explain_always_and_actions_when_sm_present(){
        // /explain always. /actions only when the noun has an SM.
        let state = push_state_machine(
            organization_with_slug(),
            "Organization Lifecycle", "Organization",
            &["active", "archived"],
            &[("active", "archived", "archive")],
        );

        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().unwrap();
        assert!(paths.contains_key("/organizations/{id}/explain"),
            "GET /explain must exist per Thm 5; got: {:?}",
            paths.keys().collect::<Vec<_>>());
        assert!(paths.contains_key("/organizations/{id}/actions"),
            "GET /actions must exist for SM-bearing noun; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn explain_route_exists_for_noun_without_state_machine() {
        // No SM: /actions is absent, /explain still present because
        // derivations can exist on any entity regardless of SM.
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().unwrap();
        assert!(paths.contains_key("/organizations/{id}/explain"));
        assert!(!paths.contains_key("/organizations/{id}/actions"),
            "/actions must be absent without an SM");
    }

    #[test]
    fn openapi_generator_is_app_scoped_opt_in() {
        // Generators are App-scoped: `App 'X' uses Generator 'openapi'.`
        // The opt-in is an instance fact on the App, carried through the
        // compile as a fact in the `App_uses_Generator` cell. Without
        // that fact, no openapi:* cells are emitted. With it, exactly one
        // `openapi:{snake(app-slug)}` cell is emitted per opted-in App.
        let base_state = organization_with_slug();

        let defs_without = crate::compile::compile_to_defs_state(&base_state);
        assert!(
            !defs_without.iter().any(|(k, _)| k.starts_with("openapi:")),
            "openapi:* cells must not appear without an App opt-in fact; got keys: {:?}",
            defs_without.iter().filter(|(k, _)| k.starts_with("openapi:")).map(|(k, _)| k).collect::<Vec<_>>()
        );

        // Opt in: push `{App: 'sherlock', Generator: 'openapi'}` into
        // the `App_uses_Generator` cell that main.rs populates from the
        // raw `App 'X' uses Generator 'Y'` regex capture.
        let opt_in_state = crate::ast::cell_push(
            "App_uses_Generator",
            crate::ast::fact_from_pairs(&[("App", "sherlock"), ("Generator", "openapi")]),
            &base_state,
        );

        let defs_with = crate::compile::compile_to_defs_state(&opt_in_state);
        assert!(
            defs_with.iter().any(|(k, _)| k == "openapi:sherlock"),
            "openapi:sherlock cell must exist when 'App sherlock uses Generator openapi' \
             is asserted; got openapi:* keys: {:?}",
            defs_with.iter().filter(|(k, _)| k.starts_with("openapi:")).map(|(k, _)| k).collect::<Vec<_>>()
        );
    }
}
