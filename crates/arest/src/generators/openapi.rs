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
// AREST.tex §4.4 is the source of truth for what a document contains:
//   "RMAP determines which facts belong to which cell from the schema's
//    uniqueness constraints: the result is a 3NF row, the complete set
//    of facts that depend on one entity's key. Each entity is a cell."
//
// This generator CONSUMES rmap::rmap(domain) as the primary source of
// component schemas and does not re-derive attributes from
// fact_types/constraints/ref_schemes independently. Columns → properties.
// `!nullable` → `required`. `references` → `$ref`. That is the whole
// schema side.
//
// State-machine status is orthogonal to RMAP (storage ≠ behavior) and
// contributes a separate `status` property with the status enum.
//
// Paths per entity are derived from Theorem 4 (HATEOAS as Projection):
//   - `/{plural}`          GET (list), POST (create)
//   - `/{plural}/{id}`     GET (read), PATCH (update)
//   - `/{plural}/{id}/transition` POST (event in body) — only if SM
//   - related-collection per binary fact type the noun participates in
//     (follow-up scope)
//
// No DELETE — per §4.1 and Corollary 2, deletion is a transition to a
// terminal status. The list endpoint filters out terminal entities via
// `Filter(p_live) : P` (server-side).
//
// Response envelope per Theorems 3 + 5 and Corollary 1:
//   `{ data, derived, violations, _links }` — follow-up scope.
//
// Design constraints (project rules):
//   - Pure FP style: iterator combinators, no for loops, no control-flow ifs.
//   - The function is total: missing cells yield a valid empty document.
//   - Output parses as valid JSON conforming to OpenAPI 3.1.

use hashbrown::HashMap;

use crate::ast::Object;
use crate::rmap::{self, TableColumn, TableDef};
use crate::types::{Domain, StateMachineDef};

/// Compile state into an OpenAPI 3.1 JSON document for one App.
///
/// Public entry point matching the solidity/fpga generator signature.
/// Reconstructs the domain from state, runs RMAP, and composes the
/// OpenAPI document from the resulting TableDefs, with the App's
/// identity baked into the document's `info` section.
pub fn compile_to_openapi(state: &Object, app_name: &str) -> String {
    let domain = crate::compile::state_to_domain(state);
    openapi_for_app(&domain, app_name).to_string()
}

/// Build the OpenAPI 3.1 document for one App as a `serde_json::Value`.
///
/// An App is the unit of API product identity — the `info.title` is the
/// App, the `info.description` comes from the App's instance facts when
/// declared. Nouns and paths are drawn from the full compile: today
/// there is no structured noun→domain mapping, so every entity in the
/// compile contributes to every App's document. Future work can narrow
/// this via `Domain belongs to App` + a noun→domain trace, at which
/// point the per-App cell will specialize further.
///
/// `pub(crate)` so `compile.rs` can register the document cell without
/// round-tripping through state for every App.
pub(crate) fn openapi_for_app(domain: &Domain, app_name: &str) -> serde_json::Value {
    let tables = rmap::rmap(domain);
    let tables_by_entity: HashMap<String, &TableDef> = tables.iter()
        .map(|t| (t.name.clone(), t))
        .collect();

    // For each value-type column name (snake_case), recover the source noun
    // so we can consult `domain.enum_values`. RMAP does not carry the source
    // noun name on TableColumn; this side-map bridges the gap without
    // changing the RMAP type surface.
    let noun_by_snake: HashMap<String, String> = domain.nouns.keys()
        .map(|n| (rmap::to_snake(n), n.clone()))
        .collect();

    let mut schemas: serde_json::Map<String, serde_json::Value> = domain.nouns.iter()
        .filter(|(_, n)| n.object_type == "entity")
        .filter_map(|(name, _)| {
            let table_name = rmap::to_snake(name);
            tables_by_entity.get(&table_name)
                .map(|table| (name.clone(), component_schema(domain, name, table, &noun_by_snake)))
        })
        .collect();
    // Violation component is the wire representation of a failed constraint
    // (Theorem 3 violations + Corollary 1 verbalization). Declared
    // unconditionally so the envelope reference always resolves even in
    // compiles that do not load outcomes.md. A real Violation entity from
    // outcomes.md, when loaded, overrides this default via the user's own
    // RMAP-derived schema.
    schemas.entry("Violation".to_string())
        .or_insert_with(violation_component_schema);

    // Paths per Theorem 4 (HATEOAS as Projection). For each entity with a
    // RMAP-derived table, emit the canonical CRUD routes. Follow-up work
    // adds related-collection routes (Theorem 4b navigation) and the
    // `{data, derived, violations, _links}` response envelope.
    let paths: serde_json::Map<String, serde_json::Value> = domain.nouns.iter()
        .filter(|(_, n)| n.object_type == "entity")
        .filter(|(name, _)| {
            let table_name = rmap::to_snake(name);
            tables_by_entity.contains_key(&table_name)
        })
        .flat_map(|(name, _)| {
            let plural = plural_for_noun(domain, name);
            let sm = domain.state_machines.get(name);
            paths_for_noun(name, &plural, sm, domain)
        })
        .collect();

    let app_description = app_description(domain, app_name)
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

/// Look up an App's description from `App has Description` instance
/// facts. Returns `None` when no description fact is present; the
/// caller chooses its own fallback sentence.
fn app_description(domain: &Domain, app_name: &str) -> Option<String> {
    domain.general_instance_facts.iter()
        .find(|f| f.subject_noun == "App"
            && f.subject_value == app_name
            && f.field_name == "Description")
        .map(|f| f.object_value.clone())
}

/// Resolve the plural slug for a noun.
///
/// First consults `Noun has Plural` instance facts — facts-all-the-way-
/// down, no dedicated struct field. Falls back to `snake(noun) + "s"`
/// when no plural was declared. Users override the fallback by writing
/// `Noun 'Entity' has Plural 'entities'.` in their readings.
fn plural_for_noun(domain: &Domain, noun_name: &str) -> String {
    domain.general_instance_facts.iter()
        .find(|f| f.subject_noun == "Noun"
            && f.subject_value == noun_name
            && f.field_name == "Plural")
        .map(|f| f.object_value.clone())
        .unwrap_or_else(|| format!("{}s", rmap::to_snake(noun_name)))
}

/// Default Violation component schema — the wire shape of a failed
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
                                this violation reports. Round-trips parse ∘ compile.",
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
                                command (paper §4.1).",
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
/// references as θ₁ projections). Clients drive action from this
/// sub-structure; the envelope always carries it.
fn links_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "HATEOAS links per Theorem 4 — all are θ₁ projections over P and S.",
        "properties": {
            "transitions": {
                "type": "array",
                "description": "Events valid from the entity's current status. \
                                Theorem 4a: π_event(Filter(s_from ∈ {current} ∪ \
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
/// `repr(e, P, S) = {ρ(s):facts} ∪ {ρ(r):P} ∪ {ρ(c):P} ∪ links_full`.
/// Four keys: `data` (the 3NF row or list), `derived` (rule outputs —
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
            "description": "Derivation-rule outputs for this entity — every value is a \
                            ρ-application of a derivation rule over P (Theorem 5).",
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

/// Emit the canonical path items for one entity noun per Theorem 4.
///
/// Always (navigation + Cor 2 soft-delete):
///   `/{plural}`          GET (list Filter(p_live):P), POST (create)
///   `/{plural}/{id}`     GET (read), PATCH (update)
///
/// Only when the noun has a State Machine Definition (Theorem 4a —
/// transition links are a projection over transitions filtered to
/// `from ∈ {current} ∪ supertypes(current)`):
///   `/{plural}/{id}/transition`   POST (fire event)
///   `/{plural}/{id}/transitions`  GET  (events valid from current status)
///
/// Related-collection routes (Theorem 4b navigation) and the full
/// `{data, derived, violations, _links}` response envelope (Thm 5 repr +
/// Cor 1 violation verbalization) are follow-up scope.
fn paths_for_noun(
    noun_name: &str,
    plural: &str,
    sm: Option<&StateMachineDef>,
    domain: &Domain,
) -> Vec<(String, serde_json::Value)> {
    let schema_ref = serde_json::json!({
        "$ref": format!("#/components/schemas/{}", noun_name),
    });
    let list_envelope = envelope_schema(
        serde_json::json!({ "type": "array", "items": schema_ref }),
        false, // no per-entity `derived` on a list response
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

    let crud = vec![
        (format!("/{}", plural), serde_json::json!({
            "get":  { "summary": format!("List {}.", noun_name),   "responses": list_response },
            "post": { "summary": format!("Create {}.", noun_name), "requestBody": request_body, "responses": item_response },
        })),
        (format!("/{}/{{id}}", plural), serde_json::json!({
            "parameters": [id_param.clone()],
            "get":   { "summary": format!("Read {}.", noun_name),   "responses": item_response },
            "patch": { "summary": format!("Update {}.", noun_name), "requestBody": request_body, "responses": item_response },
        })),
    ];

    let transitions = sm.into_iter().flat_map(|sm| {
        let events: Vec<&str> = sm.transitions.iter().map(|t| t.event.as_str()).collect();
        let fire_request = serde_json::json!({
            "required": true,
            "description": "Fire a transition by event name. The event is \
                            a no-op when it is not valid from the entity's \
                            current status.",
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
    });

    // Theorem 4b navigation: every binary fact type the noun participates
    // in contributes a related-collection endpoint on the noun's path
    // space. The projection is
    //   π_noun(other) ∘ Filter(eq ∘ [noun(r_c), n̄]) : f
    // which at the OpenAPI surface materializes as GET
    // `/{plural}/{id}/{slug}`. Only entity-typed co-participants
    // contribute — value types are absorbed into the noun row per RMAP.
    //
    // Slug rules (ring + multi-FT disambiguation):
    //   - Single FT for a (this_noun, other_noun) pair, non-ring:
    //     `/{plural}/{id}/{other-plural}`.
    //   - Multiple FTs for the same (this_noun, other_noun) pair:
    //     `/{plural}/{id}/{verb-slug}-{other-plural}` so parallel
    //     relationships (Customer owns Account, Customer bills Account)
    //     each get their own route.
    //   - Ring (same noun on both roles, e.g. Employee reports to
    //     Employee): `/{plural}/{id}/{verb-slug}` — the verb alone,
    //     since the other side is this plural and would collide.
    //
    // `verb_slug_from_reading` lowercases and kebab-cases the
    // non-noun tokens in the reading.

    // Collect this noun's binary-FT participations into (other_noun,
    // reading) tuples. Rings list twice (once per role) but we
    // dedupe below via reading identity.
    let participations: Vec<(String, String)> = domain.fact_types.values()
        .filter(|ft| ft.roles.len() == 2)
        .filter(|ft| ft.roles.iter().any(|r| r.noun_name == noun_name))
        .filter(|ft| {
            // The "other" side must be an entity. For rings, both sides
            // are the same noun which IS an entity — so rings pass.
            let other_role = ft.roles.iter().find(|r| r.noun_name != noun_name)
                .unwrap_or(&ft.roles[0]);
            domain.nouns.get(&other_role.noun_name)
                .map(|n| n.object_type == "entity")
                .unwrap_or(false)
        })
        .map(|ft| {
            let other_name = ft.roles.iter()
                .find(|r| r.noun_name != noun_name)
                .map(|r| r.noun_name.clone())
                .unwrap_or_else(|| ft.roles[0].noun_name.clone()); // ring case
            (other_name, ft.reading.clone())
        })
        .collect();

    // Group by other_noun so we can tell single vs multi.
    let mut by_other: HashMap<String, Vec<String>> = HashMap::new();
    participations.into_iter().for_each(|(other, reading)| {
        by_other.entry(other).or_default().push(reading);
    });

    let noun_names: Vec<&str> = domain.nouns.keys().map(|s| s.as_str()).collect();
    // Rebind id_param to a fresh clone so the closure below owns it
    // independently of the transitions closure above (both are lazy
    // and composed in a chain; the borrow checker otherwise sees
    // overlapping uses of the outer binding).
    let id_param_for_related = id_param.clone();

    let related_routes: Vec<(String, serde_json::Value)> = by_other.iter()
        .flat_map(|(other_noun, readings)| {
            let other_plural = plural_for_noun(domain, other_noun);
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

    // Introspection routes per Theorem 5.
    //
    // /{plural}/{id}/actions  — events valid from current status.
    //   Mirrors /{plural}/{id}/transitions; emitted only when the noun
    //   has an SM. The name matches the `actions` MCP verb so clients
    //   can navigate between the two surfaces with one mental model.
    //
    // /{plural}/{id}/explain  — derivation chain for all derived facts
    //   on this entity. Always emitted; returns the Thm 5 provenance
    //   trace (which rules fired, which facts they consumed, whether
    //   each antecedent was asserted or derived).
    let actions_route = sm.into_iter().map(|sm| {
        let events: Vec<&str> = sm.transitions.iter().map(|t| t.event.as_str()).collect();
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
        (
            format!("/{}/{{id}}/actions", plural),
            serde_json::json!({
                "parameters": [id_param.clone()],
                "get": {
                    "summary": format!("List valid actions (SM events) for a {}.", noun_name),
                    "description": "Alias of /transitions; named to match the MCP `actions` verb.",
                    "responses": events_response,
                },
            }),
        )
    }).collect::<Vec<_>>();

    let explain_response = serde_json::json!({
        "200": {
            "description": format!(
                "Derivation chain for all derived facts on this {}. \
                 Theorem 5: every value in the representation is a ρ-application \
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
                "description": "Returns the derivation chain per Theorem 5 — rule name, \
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
        .chain(std::iter::once(explain_route))
        .collect()
}

/// Extract a kebab-case verb slug from a binary fact type's reading.
///
/// Strategy: tokenize the reading, drop the longest-first noun matches,
/// keep what's left, lowercase-kebab-case the residue. Handles
/// compound nouns ("State Machine Definition") via longest-match.
///
/// "Customer owns Account"        → "owns"
/// "Order was placed by Customer" → "was-placed-by"
/// "Employee reports to Employee" → "reports-to"
fn verb_slug_from_reading(reading: &str, noun_names: &[&str]) -> String {
    // Sort noun_names descending by whitespace-token count so longer
    // names match before shorter prefixes of themselves.
    let mut sorted: Vec<&str> = noun_names.to_vec();
    sorted.sort_by_key(|n| std::cmp::Reverse(n.split_whitespace().count()));

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

/// Build one entity's component schema from its RMAP TableDef.
///
/// Columns contribute properties. Non-nullable columns contribute to
/// `required`. FK columns emit `$ref`. The state machine, if any, adds a
/// `status` property whose enum is the declared status set.
fn component_schema(
    domain: &Domain,
    noun_name: &str,
    table: &TableDef,
    noun_by_snake: &HashMap<String, String>,
) -> serde_json::Value {
    let column_props = table.columns.iter()
        .map(|col| (col.name.clone(), column_property(col, domain, noun_by_snake)));

    // State machines for this noun contribute a `status` property whose
    // enum is the declared status set. Transitions drive behavior; this
    // property is the read-side projection of the current status.
    let sm_props = domain.state_machines.values()
        .filter(|sm| sm.noun_name == noun_name)
        .map(|sm| (
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": &sm.statuses,
            }),
        ));

    let properties: serde_json::Map<String, serde_json::Value> =
        column_props.chain(sm_props).collect();

    let required: Vec<String> = table.columns.iter()
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
fn column_property(
    col: &TableColumn,
    domain: &Domain,
    noun_by_snake: &HashMap<String, String>,
) -> serde_json::Value {
    col.references.as_ref()
        .map(|target| serde_json::json!({
            "$ref": format!("#/components/schemas/{}", target),
        }))
        .unwrap_or_else(|| {
            let source_noun = noun_by_snake.get(&col.name);
            let enum_vals = source_noun.and_then(|n| domain.enum_values.get(n));
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
        let doc = openapi_for_app(&Domain::default(), "test-app");

        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], "1.0.0");
        assert!(doc["info"]["title"].is_string());
        assert!(doc["paths"].is_object());
        let schemas = doc["components"]["schemas"].as_object()
            .expect("components.schemas must be an object");
        // Violation is unconditional — every envelope references it.
        assert_eq!(schemas.keys().cloned().collect::<Vec<_>>(), vec!["Violation"],
            "empty domain emits only the Violation envelope type; got: {:?}",
            schemas.keys().collect::<Vec<_>>());
    }

    /// Parse a FORML2 snippet into a Domain for tests.
    /// Using the real parser guarantees the resulting Domain has all the
    /// invariants RMAP expects (ref schemes, backing fact types, implicit
    /// uniqueness on reference-scheme roles).
    fn parse(src: &str) -> Domain {
        crate::parse_forml2::parse_markdown(src)
            .expect("test FORML2 must parse")
    }

    fn organization_with_slug() -> Domain {
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
        let domain = organization_with_slug();

        let doc = openapi_for_app(&domain, "test-app");
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
        let domain = organization_with_slug();

        let doc = openapi_for_app(&domain, "test-app");
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

    #[test]
    fn plural_instance_fact_overrides_fallback() {
        // `Noun 'X' has Plural 'ys'` is how irregular plurals ("policies",
        // "categories", "children") reach the path surface. Without this
        // override, snake(noun) + "s" mangles most non-regular nouns.
        // The instance fact lives as a GeneralInstanceFact against the
        // metamodel's `Noun has Plural` binary — facts all the way down.
        let mut domain = organization_with_slug();
        domain.general_instance_facts.push(crate::types::GeneralInstanceFact {
            subject_noun: "Noun".into(),
            subject_value: "Organization".into(),
            field_name: "Plural".into(),
            object_noun: "Plural".into(),
            object_value: "orgs".into(),
        });

        let doc = openapi_for_app(&domain, "test-app");
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
             declared — the declaration wins; got: {:?}",
            paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn noun_with_state_machine_has_transition_routes() {
        // Theorem 4a: transition links are a projection over the transition
        // fact set filtered to `from ∈ {current} ∪ supertypes(current)`.
        // At the OpenAPI surface that projection materializes as two
        // routes on the entity: POST /transition to fire an event, and
        // GET /transitions to list the events valid from the current
        // status. They only exist when the noun has a State Machine
        // Definition; a status-less noun has no transitions to project.
        use crate::types::{StateMachineDef, TransitionDef};
        let mut domain = organization_with_slug();
        domain.state_machines.insert("Organization".into(), StateMachineDef {
            noun_name: "Organization".into(),
            statuses: vec!["active".into(), "archived".into()],
            transitions: vec![TransitionDef {
                from: "active".into(),
                to: "archived".into(),
                event: "archive".into(),
                guard: None,
            }],
        });

        let doc = openapi_for_app(&domain, "test-app");
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
        // that cannot be fulfilled — the handler would 404 on every call.
        let domain = organization_with_slug();

        let doc = openapi_for_app(&domain, "test-app");
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
        // reading (by the injectivity of parse ∘ compile). The OpenAPI
        // document must therefore declare a Violation component schema
        // that exposes the reading text as a field so tools generate
        // clients capable of surfacing the original sentence.
        let domain = organization_with_slug();
        let doc = openapi_for_app(&domain, "test-app");
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
        // Theorem 5 repr(e, P, S) = {ρ(s):f | facts} ∪ {ρ(r):P | rules}
        //                        ∪ {ρ(c):P | constraints} ∪ links_full.
        // Four top-level keys: data, derived, violations, _links.
        // Not three, not collapsed. This matches the Backus §13.3.2
        // representation function and preserves provenance.
        let domain = organization_with_slug();
        let doc = openapi_for_app(&domain, "test-app");
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
        // data is the 3NF row — a ref to the noun schema
        let data = &item_schema["properties"]["data"];
        assert!(data.get("$ref").is_some() || data["type"] == "object",
            "envelope.data must be the noun row (schema $ref or inline object); got: {}", data);
    }

    #[test]
    fn list_response_wraps_array_in_envelope_per_theorem_5() {
        // List responses carry the same envelope; `data` is an array.
        // Pagination + query-level violations are reported alongside.
        let domain = organization_with_slug();
        let doc = openapi_for_app(&domain, "test-app");
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
        // `Customer owns Account` — Customer and Account each get a
        // navigation toward the other in its path space.
        let domain = parse("\
            Customer(.Slug) is an entity type.\n\
            Account(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Customer has Slug.\n\
              Each Customer has exactly one Slug.\n\
            Account has Slug.\n\
              Each Account has exactly one Slug.\n\
            Customer owns Account.\n\
        ");
        let doc = openapi_for_app(&domain, "test-app");
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
        // `Employee reports to Employee` — both roles on Employee.
        // The forward direction gets a verb-slug path because the
        // other-plural would collide with this plural.
        let domain = parse("\
            Employee(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Employee has Slug.\n\
              Each Employee has exactly one Slug.\n\
            Employee reports to Employee.\n\
        ");
        let doc = openapi_for_app(&domain, "test-app");
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
        let domain = parse("\
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
        let doc = openapi_for_app(&domain, "test-app");
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
        use crate::types::{StateMachineDef, TransitionDef};
        let mut domain = organization_with_slug();
        domain.state_machines.insert("Organization".into(), StateMachineDef {
            noun_name: "Organization".into(),
            statuses: vec!["active".into(), "archived".into()],
            transitions: vec![TransitionDef {
                from: "active".into(),
                to: "archived".into(),
                event: "archive".into(),
                guard: None,
            }],
        });

        let doc = openapi_for_app(&domain, "test-app");
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
        let domain = organization_with_slug();
        let doc = openapi_for_app(&domain, "test-app");
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
        let mut domain = organization_with_slug();
        domain.domain = "test".into();
        let base_state = crate::parse_forml2::domain_to_state(&domain);

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
