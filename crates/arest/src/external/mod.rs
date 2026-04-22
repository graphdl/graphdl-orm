// External System catalogs (#343).
//
// External System is a first-class noun in core.md: (Name, URL, Header,
// Prefix, Kind) plus `Noun is backed by External System`. This module
// mounts vocabularies — schema.org first — as live External System
// instances, and answers browse requests populate-on-demand from each
// connector's parsed-source cache.
//
// Design:
//   - `mount(state)` pushes minimal metadata + a handful of anchor
//     Nouns so the UI shell and generators/openapi can enumerate the
//     mount.
//   - `browse(state, system, path)` answers type questions by dispatching
//     to the connector module for the named system. Reads that
//     module's cache (a OnceLock'd parsed graph); does NOT walk cells
//     or mutate state.
//   - Follow-up lanes add more vocabularies (DCMI, FOAF, Wikidata,
//     GoodRelations) as sibling modules with the same shape.

pub mod schema_org;

use crate::ast::{Object, binding, fetch_or_phi};
use alloc::{string::{String, ToString}, vec::Vec, format};

/// Response shape for `external_browse` / `/external/{system}/types/{name}`.
///
/// Matches the handoff contract verbatim:
///     type, supertypes[], subtypes[], properties[{name, range}]
///
/// Kept explicit (not `serde_json::Value`) so the same type round-trips
/// to both JSON (MCP / OpenAPI body) and the engine's `Object` without
/// re-encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseResponse {
    pub type_name: String,
    pub supertypes: Vec<String>,
    pub subtypes: Vec<String>,
    pub properties: Vec<BrowseProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseProperty {
    pub name: String,
    pub range: String,
}

impl BrowseResponse {
    /// Render as JSON text so callers can pipe the response straight
    /// through the MCP `{content:[{type:"text", text:…}]}` envelope or
    /// the OpenAPI `application/json` body.
    pub fn to_json(&self) -> String {
        let supertypes = json_string_array(&self.supertypes);
        let subtypes = json_string_array(&self.subtypes);
        let properties: Vec<String> = self.properties.iter()
            .map(|p| format!(
                "{{\"name\":{},\"range\":{}}}",
                json_escape(&p.name),
                json_escape(&p.range),
            ))
            .collect();
        format!(
            "{{\"type\":{},\"supertypes\":{},\"subtypes\":{},\"properties\":[{}]}}",
            json_escape(&self.type_name),
            supertypes,
            subtypes,
            properties.join(","),
        )
    }
}

fn json_string_array(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| json_escape(s)).collect();
    format!("[{}]", parts.join(","))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// List every mounted External System's name (as seen in the "External
/// System" cell). Used by `generators/openapi` to decide which
/// `/external/{system}/...` routes to emit.
pub fn mounted_systems(state: &Object) -> Vec<String> {
    let cell = fetch_or_phi("External System", state);
    cell.as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "name").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// Types exposed by `system`, in the order the connector surfaces them.
///
/// Source by system:
///   - "schema.org": every rdfs:Class in the parsed graph. Covers the
///     whole vocabulary so `GET /external/schema.org/types` lists
///     everything, not just the six mount anchors.
///   - other / unknown: empty (no connector dispatched).
pub fn types_for_system(system: &str) -> Vec<String> {
    match system {
        schema_org::SYSTEM_NAME => {
            let g = schema_org_graph_snapshot();
            g.known_types
        }
        _ => Vec::new(),
    }
}

/// MCP verb `external_browse` handler. Accepts the raw JSON request
/// body — `{"system":"schema.org","path":["Person"]}` — and returns
/// either a JSON-encoded `BrowseResponse` or the single glyph `"⊥"` on
/// malformed input / unmounted system / unknown type.
///
/// Kept here (not in lib.rs) so the engine-side intercept is a thin
/// string-in / string-out adapter. The acceptance path in the handoff
/// reads exactly one line in system_impl.
pub fn external_browse_json(input: &str, state: &Object) -> String {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(input) else {
        return "⊥".into();
    };
    let Some(system) = parsed.get("system").and_then(|v| v.as_str()) else {
        return "⊥".into();
    };
    let path: Vec<String> = parsed.get("path")
        .and_then(|v| v.as_array())
        .map(|xs| xs.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();

    match browse(state, system, &path) {
        Some(resp) => resp.to_json(),
        None => "⊥".into(),
    }
}

/// Browse one type in `system`. Returns `None` if the system is unknown
/// or the named type isn't exposed by that system.
///
/// `path` is a dotted breadcrumb — e.g. `["Thing","Person"]`. Only the
/// last segment picks the type; the earlier segments are reserved for
/// future tree-view navigation and are currently ignored beyond bounds.
/// When `path` is empty the response is a system-root overview whose
/// `subtypes` are the top-level types the connector exposes.
pub fn browse(state: &Object, system: &str, path: &[String]) -> Option<BrowseResponse> {
    // Gate on the mount: unmounted systems never answer browse requests
    // so a mis-spelled tenant config doesn't silently succeed.
    if !mounted_systems(state).iter().any(|s| s == system) {
        return None;
    }

    match system {
        schema_org::SYSTEM_NAME => browse_schema_org(path),
        _ => None,
    }
}

/// schema.org browse — reads the parsed-graph cache directly.
/// Stateless; does not touch `state`.
fn browse_schema_org(path: &[String]) -> Option<BrowseResponse> {
    let type_name = match path.last() {
        Some(t) => t.clone(),
        None => return Some(BrowseResponse {
            type_name: schema_org::SYSTEM_NAME.to_string(),
            supertypes: Vec::new(),
            subtypes: schema_org_top_level(),
            properties: Vec::new(),
        }),
    };

    if !schema_org::has_type(&type_name) { return None; }

    let properties = schema_org::inherited_properties(&type_name).into_iter()
        .map(|(name, range)| BrowseProperty { name, range })
        .collect();

    Some(BrowseResponse {
        type_name: type_name.clone(),
        supertypes: schema_org::supertypes(&type_name),
        subtypes: schema_org::subtypes(&type_name),
        properties,
    })
}

/// Lightweight projection of schema.org's top-level types: the types
/// whose only supertypes live outside the schema: namespace (so Thing,
/// DataType, and Enumeration surface at the root).
fn schema_org_top_level() -> Vec<String> {
    let all = schema_org_graph_snapshot().known_types;
    let known: hashbrown::HashSet<&str> = all.iter().map(|s| s.as_str()).collect();
    all.iter()
        .filter(|t| {
            let sups = schema_org::supertypes(t);
            sups.iter().all(|s| !known.contains(s.as_str()))
        })
        .cloned()
        .collect()
}

/// Cheap snapshot of the schema.org connector's browse-relevant state.
/// Used by `types_for_system` and `schema_org_top_level` so they stay
/// decoupled from the connector's internal representation.
struct SchemaOrgSnapshot {
    known_types: Vec<String>,
}

fn schema_org_graph_snapshot() -> SchemaOrgSnapshot {
    // Eager flatten of the parsed graph's known types so callers don't
    // need to pay the HashMap → Vec conversion every call. The parsed
    // graph itself is OnceLock'd inside the connector; this helper
    // just exposes a slice-friendly projection.
    let mut known_types: Vec<String> = (0..schema_org::CORE_TYPES.len())
        .map(|i| schema_org::CORE_TYPES[i].to_string())
        .collect();
    known_types.extend(schema_org::all_known_types().into_iter()
        .filter(|t| !schema_org::CORE_TYPES.iter().any(|c| t == *c)));
    SchemaOrgSnapshot { known_types }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Object, cell_push, fact_from_pairs};

    fn state_with_mounted_system(name: &str) -> Object {
        cell_push(
            "External System",
            fact_from_pairs(&[("name", name)]),
            &Object::phi(),
        )
    }

    #[test]
    fn mounted_systems_reads_external_system_cell() {
        let state = cell_push(
            "External System",
            fact_from_pairs(&[("name", "schema.org")]),
            &state_with_mounted_system("auth.vin"),
        );
        let systems = mounted_systems(&state);
        assert!(systems.iter().any(|s| s == "schema.org"));
        assert!(systems.iter().any(|s| s == "auth.vin"));
    }

    #[test]
    fn browse_unmounted_system_returns_none() {
        let resp = browse(&Object::phi(), "schema.org", &["Person".to_string()]);
        assert!(resp.is_none(),
            "browse must refuse unmounted systems (cell gate)");
    }

    #[test]
    fn browse_schema_org_person_after_mount_returns_inherited_name_and_birthdate() {
        let state = schema_org::mount(&Object::phi());
        let resp = browse(&state, "schema.org", &["Person".to_string()])
            .expect("Person is a schema.org type after mount");
        assert_eq!(resp.type_name, "Person");
        assert!(resp.properties.iter().any(|p| p.name == "name"),
            "Person inherits 'name' from Thing via populate-on-browse");
        assert!(resp.properties.iter().any(|p| p.name == "birthDate"),
            "Person declares 'birthDate' directly");
    }

    #[test]
    fn browse_schema_org_person_supertypes_include_thing() {
        let state = schema_org::mount(&Object::phi());
        let resp = browse(&state, "schema.org", &["Person".to_string()]).unwrap();
        assert!(resp.supertypes.iter().any(|s| s == "Thing"));
    }

    #[test]
    fn browse_is_stateless_no_cells_added() {
        let mounted = schema_org::mount(&Object::phi());
        let noun_len = |s: &Object| fetch_or_phi("Noun", s).as_seq().map(|x| x.len()).unwrap_or(0);
        let before = noun_len(&mounted);
        let _ = browse(&mounted, "schema.org", &["Patient".to_string()]);
        let _ = browse(&mounted, "schema.org", &["Organization".to_string()]);
        let after = noun_len(&mounted);
        assert_eq!(before, after,
            "populate-on-browse must not mutate state — cache is the parsed graph");
    }

    #[test]
    fn browse_unknown_type_returns_none() {
        let state = schema_org::mount(&Object::phi());
        let resp = browse(&state, "schema.org", &["ZzNonexistent".to_string()]);
        assert!(resp.is_none(),
            "unknown types must not fabricate a BrowseResponse");
    }

    #[test]
    fn browse_empty_path_returns_system_root_with_subtypes() {
        let state = schema_org::mount(&Object::phi());
        let resp = browse(&state, "schema.org", &[]).expect("root browse");
        assert_eq!(resp.type_name, "schema.org");
        assert!(resp.subtypes.iter().any(|s| s == "Thing"),
            "root subtypes must include Thing");
    }

    #[test]
    fn types_for_system_lists_schema_org_classes() {
        let all = types_for_system("schema.org");
        assert!(all.iter().any(|t| t == "Person"));
        assert!(all.iter().any(|t| t == "Thing"));
        assert!(all.len() >= 800,
            "schema.org has 1000+ classes; snapshot returned {}", all.len());
    }

    #[test]
    fn types_for_system_unknown_returns_empty() {
        assert_eq!(types_for_system("bogus.vocab"), Vec::<String>::new());
    }

    #[test]
    fn external_browse_json_returns_person_response_after_mount() {
        let state = schema_org::mount(&Object::phi());
        let resp = external_browse_json(
            r#"{"system":"schema.org","path":["Person"]}"#,
            &state,
        );
        assert!(resp.starts_with('{'),
            "response must be JSON, got: {resp}");
        assert!(resp.contains("\"type\":\"Person\""));
        assert!(resp.contains("\"name\":\"name\""));
        assert!(resp.contains("\"name\":\"birthDate\""));
    }

    #[test]
    fn external_browse_json_rejects_malformed_input() {
        assert_eq!(
            external_browse_json("not json", &schema_org::mount(&Object::phi())),
            "⊥"
        );
    }

    #[test]
    fn external_browse_json_rejects_unmounted_system() {
        let resp = external_browse_json(
            r#"{"system":"schema.org","path":["Person"]}"#,
            &Object::phi(),
        );
        assert_eq!(resp, "⊥",
            "browse on unmounted system must return ⊥");
    }

    #[test]
    fn external_browse_json_rejects_unknown_type() {
        let state = schema_org::mount(&Object::phi());
        let resp = external_browse_json(
            r#"{"system":"schema.org","path":["ZzNope"]}"#,
            &state,
        );
        assert_eq!(resp, "⊥");
    }

    #[test]
    fn external_browse_json_empty_path_returns_system_root() {
        let state = schema_org::mount(&Object::phi());
        let resp = external_browse_json(
            r#"{"system":"schema.org","path":[]}"#,
            &state,
        );
        assert!(resp.contains("\"type\":\"schema.org\""));
    }

    #[test]
    fn to_json_shape_matches_handoff_contract() {
        let resp = BrowseResponse {
            type_name: "Person".into(),
            supertypes: vec!["Thing".into()],
            subtypes: vec!["Patient".into()],
            properties: vec![
                BrowseProperty { name: "name".into(), range: "Text".into() },
            ],
        };
        let json = resp.to_json();
        assert!(json.contains("\"type\":\"Person\""));
        assert!(json.contains("\"supertypes\":[\"Thing\"]"));
        assert!(json.contains("\"subtypes\":[\"Patient\"]"));
        assert!(json.contains("\"name\":\"name\""));
        assert!(json.contains("\"range\":\"Text\""));
    }
}
