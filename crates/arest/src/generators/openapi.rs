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

use crate::ast::{Object, binding, fetch_cell_seq};
use crate::rmap::{self, ColumnView};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// State-machine cell readers (#325). Replaces the earlier
// `state_machines_from_state -> HashMap<String, StateMachineDef>`
// typed-IR materialisation. Consumers read per-noun SM info via these
// three helpers directly, no typed struct in flight.

/// Resolve the SM name attached to a noun, if any.
fn sm_name_for_noun(state: &Object, noun_name: &str) -> Option<String> {
    fetch_cell_seq("InstanceFact", state).as_seq()?
        .iter()
        .find(|f| binding(f, "subjectNoun") == Some("State Machine Definition")
            && binding(f, "fieldName").map(|s| s.contains("is for")).unwrap_or(false)
            && binding(f, "objectValue") == Some(noun_name))
        .and_then(|f| binding(f, "subjectValue").map(String::from))
}

/// Statuses for the SM attached to `noun_name`, in declaration order.
fn sm_statuses(state: &Object, noun_name: &str) -> Vec<String> {
    let Some(sm_name) = sm_name_for_noun(state, noun_name) else { return vec![]; };
    let inst = fetch_cell_seq("InstanceFact", state);
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
    let inst = fetch_cell_seq("InstanceFact", state);
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

    let nouns_cell = fetch_cell_seq("Noun", state);
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

    // #279 P2a: noun_name -> Conceptual Data Type code (from the
    // "conceptualDataType" binding on the Noun cell, absorbed by P1's
    // `The data type of <VT> is <code>.`). Mirrors the objectType /
    // enumValues maps above. A value-type property whose source noun has
    // a code is typed from the catalog instead of the SQL-derived scalar.
    let noun_data_types: HashMap<String, String> = nouns_seq.iter()
        .filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let code = binding(n, "conceptualDataType")?.to_string();
            Some((name, code))
        })
        .collect();

    // #279 P4: noun_name -> maxLength, from the `maxLength` binding the
    // parser absorbs for a `text with length <n>` data-type assignment
    // (reusing the existing Max Length value attribute). Surfaces as JSON
    // Schema `maxLength` on the value-type property. precision / scale
    // have no JSON Schema representation and are intentionally omitted.
    let noun_max_lengths: HashMap<String, u64> = nouns_seq.iter()
        .filter_map(|n| {
            let name = binding(n, "name")?.to_string();
            let len: u64 = binding(n, "maxLength")?.parse().ok()?;
            Some((name, len))
        })
        .collect();

    // #279 P2a: code -> (JSON Type, JSON Format?) catalog projection.
    // Read from the `Conceptual_Data_Type_has_JSON_Type` / `...Format`
    // cells when present (full compile), else the boot fallback — same
    // dual-path discipline as `compile::SqlTypeMappingTable`.
    let json_type_map = JsonTypeMappingTable::from_readings_state(state);

    // audit-entity-datatype Phase 2(d): the Format refinement layer.
    // Format is an extensible refinement built ON TOP of a Conceptual
    // Data Type (user design; the override is ordinary most-specific-
    // subtype resolution, ratified 2026-06-04). A noun with a Format
    // whose `Format has JSON Format` / `Format has Pattern` facts are
    // declared refines the CDT-derived property: the Format's JSON
    // Format WINS over the CDT catalog's, and the Pattern emits as
    // JSON Schema `pattern`. Read via cell_facts_iter — these cells
    // are Map-keyed (hash-keyed fold storage), so as_seq() would
    // silently yield nothing.
    let noun_formats: HashMap<String, String> = {
        let cell = crate::ast::fetch_or_phi("Noun_has_Format", state);
        crate::ast::cell_facts_iter(&cell)
            .filter_map(|f| Some((
                binding(f, "Noun")?.to_string(),
                binding(f, "Format")?.to_string(),
            )))
            .collect()
    };
    let format_json_formats: HashMap<String, String> = {
        let cell = crate::ast::fetch_or_phi("Format_has_JSON_Format", state);
        crate::ast::cell_facts_iter(&cell)
            .filter_map(|f| Some((
                binding(f, "Format")?.to_string(),
                binding(f, "JSON Format")?.to_string(),
            )))
            .collect()
    };
    let format_patterns: HashMap<String, String> = {
        let cell = crate::ast::fetch_or_phi("Format_has_Pattern", state);
        crate::ast::cell_facts_iter(&cell)
            .filter_map(|f| Some((
                binding(f, "Format")?.to_string(),
                binding(f, "Pattern")?.to_string(),
            )))
            .collect()
    };
    let format_layer = FormatLayer {
        noun_formats,
        format_json_formats,
        format_patterns,
    };

    // InstanceFact cell for general_instance_facts (plural / app description)
    let inst_cell = fetch_cell_seq("InstanceFact", state);
    let inst_seq = inst_cell.as_seq().unwrap_or(&[]);

    let mut schemas: serde_json::Map<String, serde_json::Value> = noun_types.iter()
        .filter(|(_, obj_type)| obj_type.as_str() == "entity")
        .filter_map(|(name, _)| {
            let table_name = rmap::to_snake(name);
            let cols = rmap::columns_for_table(&cells, &table_name);
            if cols.is_empty() { return None; }
            Some((name.clone(), component_schema_from_state(
                name, &cols, &noun_by_snake, &enum_values,
                &noun_data_types, &noun_max_lengths, &json_type_map,
                &format_layer, state)))
        })
        .collect();

    schemas.entry("Violation".to_string())
        .or_insert_with(violation_component_schema);

    // FactType + Role cells for Theorem 4b navigation
    let ft_cell = fetch_cell_seq("FactType", state);
    let ft_seq = ft_cell.as_seq().unwrap_or(&[]);
    let role_cell = fetch_cell_seq("Role", state);
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

    // #343 External System browse routes. For every mounted system
    // (rows in the "External System" cell) emit:
    //   GET /external/{system}/types          → list of type names
    //   GET /external/{system}/types/{name}   → BrowseResponse shape
    // Schema component for BrowseResponse is inserted once into
    // `components.schemas` so both routes can $ref it.
    let mut paths = paths;
    let mut schemas = schemas;
    let mounted = crate::external::mounted_systems(state);
    if !mounted.is_empty() {
        schemas.entry("ExternalTypeList".to_string())
            .or_insert_with(external_type_list_component_schema);
        schemas.entry("ExternalTypeDescription".to_string())
            .or_insert_with(external_type_description_component_schema);
        for system in mounted.iter() {
            let (collection, item) = external_paths_for_system(system);
            paths.insert(format!("/external/{}/types", system), collection);
            paths.insert(format!("/external/{}/types/{{name}}", system), item);
        }
    }

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

/// Component schema for the `/external/{system}/types` list response —
/// a JSON array of strings (type names).
fn external_type_list_component_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Type names exposed by the External System.",
    })
}

/// Component schema for the `/external/{system}/types/{name}` item
/// response — mirrors `external::BrowseResponse` verbatim (handoff
/// §1. MCP verb shape).
fn external_type_description_component_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["type", "supertypes", "subtypes", "properties"],
        "properties": {
            "type":       { "type": "string" },
            "supertypes": { "type": "array", "items": { "type": "string" } },
            "subtypes":   { "type": "array", "items": { "type": "string" } },
            "properties": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["name", "range"],
                    "properties": {
                        "name":  { "type": "string" },
                        "range": { "type": "string" },
                    },
                },
            },
        },
    })
}

/// Build the (collection, item) path items for `system`. Matches the
/// handoff contract §2.
fn external_paths_for_system(system: &str) -> (serde_json::Value, serde_json::Value) {
    let op_id_list = format!("listExternalTypes_{}", rmap::to_snake(system));
    let op_id_get  = format!("getExternalType_{}",   rmap::to_snake(system));
    let tag        = format!("external:{}", system);

    let collection = serde_json::json!({
        "get": {
            "operationId": op_id_list,
            "summary":     format!("List types in External System '{}'", system),
            "tags":        [tag.clone()],
            "responses": {
                "200": {
                    "description": "Type names exposed by this External System.",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": "#/components/schemas/ExternalTypeList" },
                        },
                    },
                },
            },
        },
    });

    let item = serde_json::json!({
        "get": {
            "operationId": op_id_get,
            "summary":     format!("Browse a type in External System '{}'", system),
            "tags":        [tag],
            "parameters": [{
                "name":     "name",
                "in":       "path",
                "required": true,
                "schema":   { "type": "string" },
                "description": "Local type name (e.g. 'Person').",
            }],
            "responses": {
                "200": {
                    "description": "Type description (supertypes, subtypes, properties).",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": "#/components/schemas/ExternalTypeDescription" },
                        },
                    },
                },
                "404": {
                    "description": "Unknown type for this External System.",
                },
            },
        },
    });

    (collection, item)
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
                        "method": { "type": "string", "enum": ["GET", "DELETE", "POST"] },
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

    // Theorem 4b navigation from state cells. Generalised over arity:
    // for a binary FT we emit one nav route per other noun (one);
    // for a ternary FT, one per other noun (two); for an n-ary FT,
    // one per *unique* non-self role. Ring FTs (noun_name appearing
    // ≥ 2 times in the same FT) additionally get a single self-route
    // — the verb-slug disambiguator downstream avoids the
    // `/{plural}/{id}/{plural}` collision. Pre-#634 the binary-only
    // gate (`if ft_roles.len() != 2 { return None; }`) silently
    // dropped every ternary+ FT from the OpenAPI nav surface.
    let participations: Vec<(String, String)> = ft_seq.iter().flat_map(|f| {
        let ft_id = match binding(f, "id") {
            Some(id) => id,
            None => return Vec::new(),
        };
        let reading = binding(f, "reading").unwrap_or("").to_string();
        let ft_roles: Vec<&str> = role_seq.iter()
            .filter(|r| binding(r, "factType") == Some(ft_id))
            .filter_map(|r| binding(r, "nounName"))
            .collect();
        if ft_roles.len() < 2 { return Vec::new(); }
        let self_count = ft_roles.iter().filter(|r| **r == noun_name).count();
        if self_count == 0 { return Vec::new(); }

        let is_entity = |n: &str| {
            noun_types.get(n).map(|t| t.as_str()) == Some("entity")
        };

        let mut acc: Vec<(String, String)> = Vec::new();
        // Ring (noun appears 2+ times in this FT): emit a single self-
        // route. Only one — additional self-roles are the same target.
        if self_count >= 2 && is_entity(noun_name) {
            acc.push((noun_name.to_string(), reading.clone()));
        }
        // Then one route per unique non-self role (entities only —
        // value types like Status / Date aren't navigable).
        for other in ft_roles.iter().filter(|r| **r != noun_name) {
            if !is_entity(other) { continue; }
            let pair = (other.to_string(), reading.clone());
            if !acc.contains(&pair) {
                acc.push(pair);
            }
        }
        acc
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
    noun_data_types: &HashMap<String, String>,
    noun_max_lengths: &HashMap<String, u64>,
    json_type_map: &JsonTypeMappingTable,
    format_layer: &FormatLayer,
    state: &Object,
) -> serde_json::Value {
    let column_props = columns.iter()
        .map(|col| (col.name.clone(), column_property_from_state(
            col, noun_by_snake, enum_values, noun_data_types, noun_max_lengths,
            json_type_map, format_layer)));

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
/// columns type from their source noun's Conceptual Data Type via the
/// catalog (`json_type_map`) when one is declared (#279 P2a) — `{type}`
/// plus `format` for the temporal / binary / uuid leaves — and otherwise
/// fall back to the coarse scalar derived from the SQL `col_type`
/// (pre-P2a behavior; RMAP types every value column `TEXT`, so untyped
/// value columns stay `string`). Declared enum values still layer on
/// top as `{..., enum}`, regardless of which typing path produced the
/// base — matching the prior enum behavior.
///
/// State-based variant of `column_property`. Uses `enum_values` /
/// `noun_data_types` HashMaps derived directly from the Noun cell rather
/// than `domain.*`.
/// audit-entity-datatype Phase 2(d): the Format refinement maps —
/// noun → Format id, Format → JSON Format, Format → Pattern. Bundled
/// so the property generator takes one parameter, not three.
struct FormatLayer {
    noun_formats: HashMap<String, String>,
    format_json_formats: HashMap<String, String>,
    format_patterns: HashMap<String, String>,
}

impl FormatLayer {
    #[cfg(test)]
    fn empty() -> Self {
        FormatLayer {
            noun_formats: HashMap::new(),
            format_json_formats: HashMap::new(),
            format_patterns: HashMap::new(),
        }
    }
}

fn column_property_from_state(
    col: &ColumnView,
    noun_by_snake: &HashMap<String, String>,
    enum_values: &HashMap<String, Vec<String>>,
    noun_data_types: &HashMap<String, String>,
    noun_max_lengths: &HashMap<String, u64>,
    json_type_map: &JsonTypeMappingTable,
    format_layer: &FormatLayer,
) -> serde_json::Value {
    if let Some(target) = col.references.as_ref() {
        return serde_json::json!({
            "$ref": format!("#/components/schemas/{}", target),
        });
    }
    let source_noun = noun_by_snake.get(&col.name);

    // #279 P2a: prefer the Conceptual Data Type projection; fall back to
    // the legacy SQL-derived scalar when the source noun has no code or
    // the code isn't in the catalog (keeps untyped value types / future
    // codes on their pre-P2a schema).
    let cdt = source_noun
        .and_then(|n| noun_data_types.get(n))
        .and_then(|code| json_type_map.resolve(code));
    let (json_type, json_format): (&str, Option<&str>) = match cdt {
        Some((t, f)) => (t, f),
        None => (sql_type_to_json(&col.col_type), None),
    };

    // audit-entity-datatype Phase 2(d): the noun's FORMAT refines the
    // CDT projection — most-specific subtype wins (the ratified
    // framing: Format-typed value types are a SUBSET of CDT-typed
    // ones). The Format's declared JSON Format overrides the CDT
    // catalog's; the Format's Pattern emits as JSON Schema `pattern`.
    let noun_format = source_noun
        .and_then(|n| format_layer.noun_formats.get(n));
    let format_json = noun_format
        .and_then(|f| format_layer.format_json_formats.get(f));
    let format_pattern = noun_format
        .and_then(|f| format_layer.format_patterns.get(f));

    let mut prop = serde_json::Map::new();
    prop.insert("type".to_string(), serde_json::Value::from(json_type));
    match (format_json, json_format) {
        (Some(refined), _) => {
            prop.insert("format".to_string(), serde_json::Value::from(refined.as_str()));
        }
        (None, Some(fmt)) => {
            prop.insert("format".to_string(), serde_json::Value::from(fmt));
        }
        (None, None) => {}
    }
    if let Some(pat) = format_pattern {
        prop.insert("pattern".to_string(), serde_json::Value::from(pat.as_str()));
    }
    // Enum constraint layers on top of whichever base type was chosen.
    if let Some(vals) = source_noun.and_then(|n| enum_values.get(n)) {
        prop.insert("enum".to_string(), serde_json::json!(vals));
    }
    // #279 P4: a text `length` facet surfaces as JSON Schema `maxLength`
    // (only meaningful on string-typed properties). precision / scale
    // have no JSON Schema representation, so they are not emitted.
    if json_type == "string" {
        if let Some(len) = source_noun.and_then(|n| noun_max_lengths.get(n)) {
            prop.insert("maxLength".to_string(), serde_json::json!(len));
        }
    }
    serde_json::Value::Object(prop)
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

/// #279 P2a — the value-type → JSON Schema projection lifts from a
/// hardcoded match into a typed table reading the catalog at compile
/// time. Each row encodes one `(Conceptual Data Type code, JSON Type,
/// JSON Format?)` triple sourced from the `Conceptual Data Type has
/// JSON Type` / `... has JSON Format` instance facts in
/// `readings/core/core.md`; lookup is O(rows) — fine for a 31-leaf
/// catalog walked once per value-type property.
///
/// This MIRRORS `compile::SqlTypeMappingTable` (the #896 SQL-dialect
/// lift): a `boot()` fallback hand-mirrors the readings one-for-one so a
/// bare engine — or a snippet-parsed state that never merged core.md —
/// types identically, and `from_readings_state` reads the two catalog
/// fact-type cells when present (the full-compile path). The catalog's
/// binary single-valued fact types keep their own data cells
/// (`Conceptual_Data_Type_has_JSON_Type` / `...has_JSON_Format`), with
/// the role players as bindings — exactly the shape the SQL table reads.
#[derive(Debug, Clone)]
pub(crate) struct JsonTypeMappingTable {
    /// One row per leaf: `(code, jsonType, jsonFormat?)`. Order matches
    /// the `Conceptual Data Type has JSON Type` instance facts in
    /// `readings/core/core.md` (the #279 catalog area).
    rows: Vec<(String, String, Option<String>)>,
}

impl JsonTypeMappingTable {
    /// Boot table — must stay in sync with the `Conceptual Data Type has
    /// JSON Type` / `... has JSON Format` instance facts in
    /// `readings/core/core.md`. 31 leaf codes; the formatted leaves
    /// (temporal, raw, uuid) additionally carry a JSON format. A
    /// `core_md_declares_conceptual_data_type_json_projection` text pin
    /// plus `json_type_mapping_table_boot_matches_readings_state` guard
    /// this against drift.
    pub(crate) fn boot() -> Self {
        // (code, jsonType, jsonFormat?) in readings declaration order.
        let rows: Vec<(String, String, Option<String>)> = [
            ("text",          "string",  None),
            ("fixedText",     "string",  None),
            ("largeText",     "string",  None),
            ("smallInteger",  "integer", None),
            ("integer",       "integer", None),
            ("largeInteger",  "integer", None),
            ("unsignedTiny",  "integer", None),
            ("unsignedSmall", "integer", None),
            ("unsigned",      "integer", None),
            ("unsignedLarge", "integer", None),
            ("autoCounter",   "integer", None),
            ("singleFloat",   "number",  None),
            ("doubleFloat",   "number",  None),
            ("decimal",       "number",  None),
            ("money",         "number",  None),
            ("uuid",          "string",  Some("uuid")),
            ("date",          "string",  Some("date")),
            ("time",          "string",  Some("time")),
            ("dateTime",      "string",  Some("date-time")),
            ("autoTimestamp", "string",  Some("date-time")),
            ("boolean",       "boolean", None),
            ("yesNo",         "boolean", None),
            ("fixedRaw",      "string",  Some("byte")),
            ("raw",           "string",  Some("byte")),
            ("largeRaw",      "string",  Some("byte")),
            ("picture",       "string",  Some("byte")),
            ("oleObject",     "string",  Some("byte")),
            ("rowId",         "integer", None),
            ("objectId",      "integer", None),
            ("unspecified",   "string",  None),
            ("userDefined",   "string",  None),
        ].iter()
            .map(|(c, t, f)| (c.to_string(), t.to_string(), f.map(str::to_string)))
            .collect();
        JsonTypeMappingTable { rows }
    }

    /// Build the table from the runtime `Conceptual_Data_Type_has_JSON_Type`
    /// and `Conceptual_Data_Type_has_JSON_Format` cells in `state`. Falls
    /// back to `boot()` when the type cell is empty (bare engine, or a
    /// snippet-parsed state that never merged core.md).
    pub(crate) fn from_readings_state(state: &Object) -> Self {
        let type_cell = fetch_cell_seq("Conceptual_Data_Type_has_JSON_Type", state);
        let type_rows: Vec<(String, String)> = type_cell.as_seq()
            .map(|facts| facts.iter().filter_map(|f| {
                let code = binding(f, "Conceptual Data Type")?.to_string();
                let json_type = binding(f, "JSON Type")?.to_string();
                Some((code, json_type))
            }).collect())
            .unwrap_or_default();
        if type_rows.is_empty() {
            return Self::boot();
        }
        // Format is optional (at-most-one) — index it by code, join in.
        let fmt_cell = fetch_cell_seq("Conceptual_Data_Type_has_JSON_Format", state);
        let fmt_rows: Vec<(String, String)> = fmt_cell.as_seq()
            .map(|facts| facts.iter().filter_map(|f| {
                let code = binding(f, "Conceptual Data Type")?.to_string();
                let json_format = binding(f, "JSON Format")?.to_string();
                Some((code, json_format))
            }).collect())
            .unwrap_or_default();
        let rows: Vec<(String, String, Option<String>)> = type_rows.into_iter()
            .map(|(code, json_type)| {
                let fmt = fmt_rows.iter()
                    .find(|(c, _)| *c == code)
                    .map(|(_, f)| f.clone());
                (code, json_type, fmt)
            })
            .collect();
        JsonTypeMappingTable { rows }
    }

    /// Resolve a Conceptual Data Type `code` to `(jsonType, jsonFormat?)`.
    /// Returns `None` when the code is absent from the catalog — the
    /// caller then keeps the legacy SQL-derived typing (no regression
    /// for unknown / future codes).
    fn resolve<'a>(&'a self, code: &str) -> Option<(&'a str, Option<&'a str>)> {
        self.rows.iter()
            .find(|(c, _, _)| c == code)
            .map(|(_, t, f)| (t.as_str(), f.as_deref()))
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
        if let Object::Map(ref mut m_arc) = state { let m = alloc::sync::Arc::make_mut(m_arc);
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
        if let Object::Map(ref mut m_arc) = state { let m = alloc::sync::Arc::make_mut(m_arc);
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

    /// audit-entity-datatype Phase 2(d): a noun's Format refinement
    /// reaches the JSON Schema — the Format's JSON Format OVERRIDES
    /// the CDT catalog's (most-specific subtype wins, the ratified
    /// framing) and its Pattern emits as `pattern`. Fixture: Email is
    /// a text-CDT value type refined by Format 'email' carrying a
    /// JSON Format + Pattern; the absorbed user.email property must
    /// carry both.
    #[test]
    fn format_refinement_overrides_cdt_json_format_and_emits_pattern() {
        let mut state = parse("\
            User(.id) is an entity type.\n\
            Email is a value type.\n\
            The data type of Email is text.\n\
            User has Email.\n\
              Each User has exactly one Email.\n\
        ");
        // Format layer cells, the shape csdp.md's widget opt-in mints
        // (Map-keyed in live dbs; cell_push'd Seq here — the reader
        // walks both via cell_facts_iter).
        state = crate::ast::cell_push("Noun_has_Format",
            crate::ast::fact_from_pairs(&[("Noun", "Email"), ("Format", "email")]), &state);
        state = crate::ast::cell_push("Format_has_JSON_Format",
            crate::ast::fact_from_pairs(&[("Format", "email"), ("JSON Format", "email")]), &state);
        state = crate::ast::cell_push("Format_has_Pattern",
            crate::ast::fact_from_pairs(&[("Format", "email"), ("Pattern", "^[^@]+@[^@]+$")]), &state);

        let doc = openapi_for_app(&state, "test-app");
        let prop = &doc["components"]["schemas"]["User"]["properties"]["email"];
        assert_eq!(prop["type"], "string",
            "the CDT base type must survive the Format refinement; got {}", prop);
        assert_eq!(prop["format"], "email",
            "the Format's JSON Format must override the CDT catalog's; got {}", prop);
        assert_eq!(prop["pattern"], "^[^@]+@[^@]+$",
            "the Format's Pattern must emit as JSON Schema pattern; got {}", prop);
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
    fn ternary_fact_type_emits_nav_routes_for_each_other_role_634() {
        // Ternary FT — pre-#634, openapi.rs `if ft_roles.len() != 2 { return None; }`
        // dropped these from the Theorem 4b nav surface entirely, so a
        // ternary FT yielded *zero* `/{id}/related/...` routes. After
        // the fix, every other entity role of the FT contributes one
        // route per side. Three entity nouns A, B, C in one ternary FT
        // produce six routes (A→B, A→C, B→A, B→C, C→A, C→B).
        let state = parse("\
            Reviewer(.Slug) is an entity type.\n\
            Movie(.Slug) is an entity type.\n\
            Award(.Slug) is an entity type.\n\
            Slug is a value type.\n\
            Reviewer has Slug.\n\
              Each Reviewer has exactly one Slug.\n\
            Movie has Slug.\n\
              Each Movie has exactly one Slug.\n\
            Award has Slug.\n\
              Each Award has exactly one Slug.\n\
            Reviewer recommends Movie for Award.\n\
        ");
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().expect("paths must be object");

        // Each entity participating in the ternary FT must reach the
        // other two through nav routes. The slug shape under
        // `by_other` collapses to the plural of the *other* noun when
        // a single reading covers the pair (no disambiguation needed).
        let expected = [
            "/reviewers/{id}/movies",
            "/reviewers/{id}/awards",
            "/movies/{id}/reviewers",
            "/movies/{id}/awards",
            "/awards/{id}/reviewers",
            "/awards/{id}/movies",
        ];
        let actual: Vec<&String> = paths.keys().collect();
        for path in expected {
            assert!(
                paths.contains_key(path),
                "ternary FT must emit nav route '{}' (Theorem 4b); paths: {:?}",
                path, actual,
            );
        }
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

    // ── #343 /external/{system}/types + /types/{name} routes ────────

    /// Tenant with schema.org mounted via `external::schema_org::mount`
    /// — tests exercise the generator on the same minimal mount the
    /// production path uses. Avoids reaching into the parsed graph.
    fn state_with_schema_org_mounted() -> Object {
        let base = organization_with_slug();
        crate::external::schema_org::mount(&base)
    }

    #[test]
    fn openapi_emits_external_types_collection_path_when_system_mounted() {
        let state = state_with_schema_org_mounted();
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().expect("paths must be an object");
        assert!(paths.contains_key("/external/schema.org/types"),
            "mounted schema.org must emit GET /external/schema.org/types; \
             paths present: {:?}", paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn openapi_emits_external_type_item_path_when_system_mounted() {
        let state = state_with_schema_org_mounted();
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().unwrap();
        assert!(paths.contains_key("/external/schema.org/types/{name}"),
            "mounted schema.org must emit GET /external/schema.org/types/{{name}}; \
             paths present: {:?}", paths.keys().collect::<Vec<_>>());
    }

    #[test]
    fn openapi_external_routes_absent_without_mount() {
        let state = organization_with_slug();
        let doc = openapi_for_app(&state, "test-app");
        let paths = doc["paths"].as_object().unwrap();
        let external_keys: Vec<&String> = paths.keys()
            .filter(|k| k.starts_with("/external/"))
            .collect();
        assert!(external_keys.is_empty(),
            "no External System cell → no /external/* routes; got: {:?}",
            external_keys);
    }

    #[test]
    fn openapi_external_type_item_response_references_handoff_shape() {
        let state = state_with_schema_org_mounted();
        let doc = openapi_for_app(&state, "test-app");
        let op = &doc["paths"]["/external/schema.org/types/{name}"]["get"];
        let resp_schema = &op["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(resp_schema["$ref"], "#/components/schemas/ExternalTypeDescription",
            "item response must $ref the shared component so both systems reuse one shape");
        // And the referenced component describes the handoff's
        // BrowseResponse fields verbatim.
        let component = &doc["components"]["schemas"]["ExternalTypeDescription"];
        let props = component["properties"].as_object()
            .expect("ExternalTypeDescription must carry properties");
        for required in ["type", "supertypes", "subtypes", "properties"] {
            assert!(props.contains_key(required),
                "component must describe '{required}'; got props: {:?}",
                props.keys().collect::<Vec<_>>());
        }
    }

    // ── #279 P2a — Conceptual Data Type → JSON Schema projection ─────
    //
    // A value type that opts into a Conceptual Data Type
    // (`The data type of <VT> is <code>.`) types its property from the
    // catalog instead of defaulting to "string". The mapping is data
    // (readings/core.md), read via `JsonTypeMappingTable`; the boot
    // fallback covers states that don't carry the catalog cell — which
    // is the case for these snippet-parsed fixtures (no core.md merged).

    /// Build a compiled-shape state directly so a value attribute
    /// surfaces as a functional RMAP column on the entity table.
    ///
    /// The high-level FORML2 path can't drive this fixture: stage12's
    /// span-enrichment duplicates a single-role UC's span into a
    /// pseudo-compound `[role, role]` form, which `rmap()` then routes to
    /// a junction table — so `Product has <attr>` never absorbs as an
    /// entity column from a parsed snippet. Hand-built cells (the same
    /// approach `sql.rs` tests use) give a clean single-span UC so RMAP
    /// absorbs `<attr>` as a `product.<attr_snake>` column whose source
    /// noun carries the optional `conceptualDataType` under test.
    ///
    /// `extra` pushes additional pairs onto the value noun (e.g. an
    /// `enumValues` binding) so back-compat layering can be exercised.
    fn entity_state_with_value_attr(
        attr: &str, code: Option<&str>, extra: &[(&str, &str)],
    ) -> Object {
        use alloc::string::ToString;
        let ft_id = format!("Product_has_{}", attr.replace(' ', "_"));
        let reading = format!("Product has {}", attr);
        let mut attr_pairs: Vec<(&str, &str)> = vec![("name", attr), ("objectType", "value")];
        if let Some(c) = code { attr_pairs.push(("conceptualDataType", c)); }
        attr_pairs.extend_from_slice(extra);
        let nouns = Object::seq(vec![
            fact_from_pairs(&[("name", "Product"), ("objectType", "entity")]),
            fact_from_pairs(&attr_pairs),
        ]);
        let fts = Object::seq(vec![
            fact_from_pairs(&[("id", ft_id.as_str()), ("reading", reading.as_str())]),
        ]);
        let roles = Object::seq(vec![
            fact_from_pairs(&[("factType", ft_id.as_str()), ("nounName", "Product"), ("position", "0")]),
            fact_from_pairs(&[("factType", ft_id.as_str()), ("nounName", attr), ("position", "1")]),
        ]);
        // Single-span UC + MC on the Product role (role 0) → a mandatory
        // functional column, no spurious second span.
        let cons = Object::seq(vec![
            fact_from_pairs(&[("id", "uc0"), ("kind", "UC"),
                ("span0_factTypeId", ft_id.as_str()), ("span0_roleIndex", "0")]),
            fact_from_pairs(&[("id", "mc0"), ("kind", "MC"),
                ("span0_factTypeId", ft_id.as_str()), ("span0_roleIndex", "0")]),
        ]);
        let mut m: HashMap<alloc::string::String, Object> = HashMap::new();
        m.insert("Noun".to_string(), nouns);
        m.insert("FactType".to_string(), fts);
        m.insert("Role".to_string(), roles);
        m.insert("Constraint".to_string(), cons);
        Object::Map(m.into())
    }

    #[test]
    fn value_type_property_typed_integer_from_conceptual_data_type() {
        // `conceptualDataType == "integer"` → property `{ "type": "integer" }`.
        let state = entity_state_with_value_attr("Quantity", Some("integer"), &[]);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Product"]["properties"]
            .as_object().expect("Product.properties must be an object");
        let quantity = props.get("quantity")
            .unwrap_or_else(|| panic!("Product must carry a 'quantity' property; got: {:?}",
                props.keys().collect::<Vec<_>>()));
        assert_eq!(quantity["type"], "integer",
            "integer-typed value type must yield type:integer; got: {}", quantity);
        assert!(quantity.get("format").is_none(),
            "integer carries no JSON format; got: {}", quantity);
    }

    #[test]
    fn value_type_property_datetime_carries_date_time_format() {
        // `dateTime` → `{ "type": "string", "format": "date-time" }`.
        let state = entity_state_with_value_attr("Created", Some("dateTime"), &[]);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Product"]["properties"]
            .as_object().expect("Product.properties must be an object");
        let created = props.get("created")
            .unwrap_or_else(|| panic!("Product must carry a 'created' property; got: {:?}",
                props.keys().collect::<Vec<_>>()));
        assert_eq!(created["type"], "string",
            "dateTime maps to JSON type string; got: {}", created);
        assert_eq!(created["format"], "date-time",
            "dateTime carries format date-time; got: {}", created);
    }

    #[test]
    fn untyped_value_type_property_keeps_string_default() {
        // Backward-compat: a value type with NO Conceptual Data Type
        // keeps the pre-P2a schema (the coarse SQL-derived scalar, which
        // for an untyped value column is "string") and gains no format.
        let state = entity_state_with_value_attr("Nickname", None, &[]);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Product"]["properties"]
            .as_object().expect("Product.properties must be an object");
        let nickname = props.get("nickname")
            .unwrap_or_else(|| panic!("Product must carry a 'nickname' property; got: {:?}",
                props.keys().collect::<Vec<_>>()));
        assert_eq!(nickname["type"], "string",
            "untyped value type keeps the string default; got: {}", nickname);
        assert!(nickname.get("format").is_none(),
            "untyped value type gains no format; got: {}", nickname);
    }

    #[test]
    fn enum_value_type_still_layers_enum_when_untyped() {
        // Backward-compat: an enum-bearing value type with no Conceptual
        // Data Type keeps `{ type, enum }` exactly as before P2a.
        let state = entity_state_with_value_attr(
            "Size", None, &[("enumValues", "small,large")]);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Product"]["properties"]
            .as_object().expect("Product.properties must be an object");
        let size = props.get("size")
            .unwrap_or_else(|| panic!("Product must carry a 'size' property; got: {:?}",
                props.keys().collect::<Vec<_>>()));
        assert_eq!(size["type"], "string");
        let variants = size["enum"].as_array().expect("size must keep its enum");
        assert!(variants.iter().any(|v| v == "small")
            && variants.iter().any(|v| v == "large"),
            "enum variants must survive; got: {}", size);
    }

    // ── #279 P2b — JSON projection END-TO-END THROUGH THE REAL PARSER ──
    //
    // The P2a tests above drive hand-built cells because, pre-RMAP-fix,
    // a single-role functional value attribute did not absorb as an
    // entity column from a parsed snippet. Post-fix it does, so these
    // pins validate the SAME projection end-to-end: parse FORML2 → the
    // value column absorbs onto the entity table → the OpenAPI generator
    // types it from the catalog (boot fallback covers the snippet, which
    // carries no merged core.md). Regression pins for the P2a feature.

    #[test]
    fn openapi_value_column_typed_integer_end_to_end() {
        let src = "Order(.code) is an entity type.\n\
Quantity is a value type.\n\
The data type of Quantity is integer.\n\
\n\
## Fact Types\n\
Order has Quantity.\n\
\n\
## Constraints\n\
Each Order has at most one Quantity.\n";
        let state = parse(src);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Order"]["properties"]
            .as_object().expect("Order.properties must be an object");
        let quantity = props.get("quantity").unwrap_or_else(|| panic!(
            "Order must carry an absorbed 'quantity' column; got: {:?}",
            props.keys().collect::<Vec<_>>()));
        assert_eq!(quantity["type"], "integer",
            "integer CDT must yield type:integer end-to-end; got: {}", quantity);
        assert!(quantity.get("format").is_none(),
            "integer carries no JSON format; got: {}", quantity);
    }

    #[test]
    fn openapi_value_column_datetime_carries_format_end_to_end() {
        let src = "Order(.code) is an entity type.\n\
Placed At is a value type.\n\
The data type of Placed At is dateTime.\n\
\n\
## Fact Types\n\
Order has Placed At.\n\
\n\
## Constraints\n\
Each Order has at most one Placed At.\n";
        let state = parse(src);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Order"]["properties"]
            .as_object().expect("Order.properties must be an object");
        let placed = props.get("placed_at").unwrap_or_else(|| panic!(
            "Order must carry an absorbed 'placed_at' column; got: {:?}",
            props.keys().collect::<Vec<_>>()));
        assert_eq!(placed["type"], "string",
            "dateTime maps to JSON type string; got: {}", placed);
        assert_eq!(placed["format"], "date-time",
            "dateTime carries format date-time end-to-end; got: {}", placed);
    }

    /// #279 P4: a text value type's `maxLength` facet (absorbed onto the
    /// Noun cell) surfaces as JSON Schema `maxLength` on a string-typed
    /// property. precision / scale have no JSON representation, so a
    /// number-typed property gains no size keyword.
    #[test]
    fn value_type_property_text_carries_max_length() {
        let state = entity_state_with_value_attr(
            "Code", Some("text"), &[("maxLength", "50")]);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Product"]["properties"]
            .as_object().expect("Product.properties must be an object");
        let code = props.get("code")
            .unwrap_or_else(|| panic!("Product must carry a 'code' property; got: {:?}",
                props.keys().collect::<Vec<_>>()));
        assert_eq!(code["type"], "string");
        assert_eq!(code["maxLength"], 50,
            "text length facet must surface as JSON maxLength; got: {}", code);

        // A decimal property carrying precision/scale gains NO size keyword
        // (no JSON Schema representation for precision/scale).
        let dec = entity_state_with_value_attr(
            "Amount", Some("decimal"), &[("precision", "10"), ("scale", "2")]);
        let doc = openapi_for_app(&dec, "test-app");
        let amount = doc["components"]["schemas"]["Product"]["properties"]["amount"].clone();
        assert_eq!(amount["type"], "number");
        assert!(amount.get("maxLength").is_none(),
            "decimal precision/scale must not emit maxLength; got: {}", amount);
    }

    /// #279 P4 — END-TO-END THROUGH THE REAL PARSER. `text with length 50`
    /// surfaces as JSON Schema `maxLength: 50` on the absorbed column.
    #[test]
    fn openapi_text_length_carries_max_length_end_to_end() {
        let src = "Order(.code) is an entity type.\n\
Label is a value type.\n\
The data type of Label is text with length 50.\n\
\n\
## Fact Types\n\
Order has Label.\n\
\n\
## Constraints\n\
Each Order has at most one Label.\n";
        let state = parse(src);
        let doc = openapi_for_app(&state, "test-app");
        let props = doc["components"]["schemas"]["Order"]["properties"]
            .as_object().expect("Order.properties must be an object");
        let label = props.get("label").unwrap_or_else(|| panic!(
            "Order must carry an absorbed 'label' column; got: {:?}",
            props.keys().collect::<Vec<_>>()));
        assert_eq!(label["type"], "string");
        assert_eq!(label["maxLength"], 50,
            "text length facet must surface as maxLength end-to-end; got: {}", label);
    }

    #[test]
    fn json_type_mapping_table_boot_resolves_catalog() {
        // The boot fallback must mirror the readings one-for-one for the
        // representative leaves the projection mapping pins.
        let t = JsonTypeMappingTable::boot();
        assert_eq!(t.resolve("integer"), Some(("integer", None)));
        assert_eq!(t.resolve("decimal"), Some(("number", None)));
        assert_eq!(t.resolve("boolean"), Some(("boolean", None)));
        assert_eq!(t.resolve("dateTime"), Some(("string", Some("date-time"))));
        assert_eq!(t.resolve("uuid"), Some(("string", Some("uuid"))));
        assert_eq!(t.resolve("raw"), Some(("string", Some("byte"))));
        assert_eq!(t.resolve("text"), Some(("string", None)));
        // Unknown code → no mapping (caller falls back to legacy typing).
        assert_eq!(t.resolve("notACode"), None);
    }

    #[test]
    fn json_type_mapping_table_boot_matches_readings_state() {
        // Parity guard like the SQL table's
        // `from_readings_state_*_matches_boot` tests: the catalog cell
        // compiled from core.md must agree with the hand-maintained boot
        // table on every leaf code.
        let readings = JsonTypeMappingTable::from_readings_state(crate::metamodel_state());
        let boot = JsonTypeMappingTable::boot();
        assert_eq!(readings.rows.len(), 31,
            "catalog must expose all 31 leaf codes; got {}", readings.rows.len());
        for (code, ty, fmt) in boot.rows.iter() {
            assert_eq!(readings.resolve(code), Some((ty.as_str(), fmt.as_deref())),
                "readings disagree with boot for leaf '{}'", code);
        }
    }
}
