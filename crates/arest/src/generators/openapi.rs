// crates/arest/src/generators/openapi.rs
//
// OpenAPI 3.1 generator: compile FFP state to an OpenAPI document.
//
// AREST.tex §4.4 is the source of truth:
//   "RMAP determines which facts belong to which cell from the schema's
//    uniqueness constraints: the result is a 3NF row, the complete set
//    of facts that depend on one entity's key. Each entity is a cell."
//
// This generator therefore CONSUMES rmap::rmap(domain) as the primary
// source of component schemas and does not re-derive attributes from
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
//   - `/{plural}/{id}/transition` POST (event in body)
//   - related-collection per binary fact type the noun participates in
//
// No DELETE — per §4.1 and Corollary 2, deletion is a transition to a
// terminal status. The list endpoint filters out terminal entities via
// `Filter(p_live) : P` (server-side, documented in the description).
//
// Response envelope per Theorems 3 + 5 and Corollary 1:
//   `{ data, _links, violations }` — `data` is the 3NF row plus derived
//   values; `_links` is `links_full(e, n, status(e, P))`; each violation
//   carries the original reading text (Cor 1 Verbalization).
//
// Design constraints (project rules):
//   - Pure FP style: iterator combinators, no for loops, no control-flow ifs.
//   - The function is total: missing cells yield a valid empty document.
//   - Output parses as valid JSON conforming to OpenAPI 3.1.

use std::collections::HashMap;

use crate::ast::Object;
use crate::rmap::{self, TableColumn, TableDef};
use crate::types::{Domain, StateMachineDef};

/// Compile state into an OpenAPI 3.1 JSON document.
///
/// Public entry point matching the solidity/fpga generator signature.
/// Reconstructs the domain from state, runs RMAP, and composes the
/// OpenAPI document from the resulting TableDefs.
pub fn compile_to_openapi(state: &Object) -> String {
    let domain = crate::compile::state_to_domain(state);
    openapi_for_domain(&domain).to_string()
}

/// Build the OpenAPI 3.1 document as a `serde_json::Value`.
///
/// `pub(crate)` so `compile.rs` can register the document cell under the
/// `openapi` generator opt-in without round-tripping through state.
pub(crate) fn openapi_for_domain(domain: &Domain) -> serde_json::Value {
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

    let schemas: serde_json::Map<String, serde_json::Value> = domain.nouns.iter()
        .filter(|(_, n)| n.object_type == "entity")
        .filter_map(|(name, _)| {
            let table_name = rmap::to_snake(name);
            tables_by_entity.get(&table_name)
                .map(|table| (name.clone(), component_schema(domain, name, table, &noun_by_snake)))
        })
        .collect();

    // Paths per Theorem 4 (HATEOAS as Projection). For each entity with a
    // RMAP-derived table, emit the canonical CRUD routes. Follow-up work
    // adds transition routes (Theorem 4a) and navigation links.
    let paths: serde_json::Map<String, serde_json::Value> = domain.nouns.iter()
        .filter(|(_, n)| n.object_type == "entity")
        .filter(|(name, _)| {
            let table_name = rmap::to_snake(name);
            tables_by_entity.contains_key(&table_name)
        })
        .flat_map(|(name, _)| {
            let plural = plural_for_noun(domain, name);
            let sm = domain.state_machines.get(name);
            paths_for_noun(name, &plural, sm)
        })
        .collect();

    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "AREST",
            "version": "1.0.0",
            "description": "Compiled from FORML2 readings by the AREST engine.",
        },
        "paths": paths,
        "components": {
            "schemas": schemas,
        },
    })
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
) -> Vec<(String, serde_json::Value)> {
    let schema_ref = serde_json::json!({
        "$ref": format!("#/components/schemas/{}", noun_name),
    });
    let list_response = serde_json::json!({
        "200": {
            "description": format!("List of {}.", noun_name),
            "content": {
                "application/json": {
                    "schema": { "type": "array", "items": schema_ref },
                },
            },
        },
    });
    let item_response = serde_json::json!({
        "200": {
            "description": format!("One {}.", noun_name),
            "content": {
                "application/json": { "schema": schema_ref },
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

    crud.into_iter().chain(transitions).collect()
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
        let doc = openapi_for_domain(&Domain::default());

        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], "1.0.0");
        assert!(doc["info"]["title"].is_string());
        assert!(doc["paths"].is_object());
        assert!(doc["components"]["schemas"].is_object());
        assert_eq!(doc["components"]["schemas"].as_object().unwrap().len(), 0);
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

        let doc = openapi_for_domain(&domain);
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

        let doc = openapi_for_domain(&domain);
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

        let doc = openapi_for_domain(&domain);
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

        let doc = openapi_for_domain(&domain);
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

        let doc = openapi_for_domain(&domain);
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
    fn openapi_generator_is_gated_by_opt_in() {
        use std::collections::HashSet;

        let mut domain = organization_with_slug();
        domain.domain = "test".into();
        let state = crate::parse_forml2::domain_to_state(&domain);

        crate::compile::set_active_generators(HashSet::new());
        let defs_without = crate::compile::compile_to_defs_state(&state);
        assert!(
            !defs_without.iter().any(|(k, _)| k.starts_with("openapi:")),
            "openapi:* cells must not appear without opt-in"
        );

        let active: HashSet<String> = std::iter::once("openapi".to_string()).collect();
        crate::compile::set_active_generators(active);
        let defs_with = crate::compile::compile_to_defs_state(&state);
        assert!(
            defs_with.iter().any(|(k, _)| k == "openapi:document"),
            "openapi:document cell must exist when 'openapi' is opted in"
        );

        crate::compile::set_active_generators(HashSet::new());
    }
}
