// schema.org External System connector (#343).
//
// The vocabulary is 1000+ classes and 1600+ properties — eagerly
// expanding it into FactType / Role / Subtype cells would bloat every
// tenant's state and slow compile/RMAP for no benefit, since most
// tenants will only browse a handful of types. Instead this module:
//
//   mount(state)                → pushes the External System row plus
//                                 the six core-type Nouns (so
//                                 generators/openapi + UI can
//                                 enumerate the mount). Minimal cells
//                                 only. No walking the catalog.
//
//   browse(state, system, path) → answers type questions by reading
//                                 the OnceLock-cached parsed graph
//                                 directly. Stateless — does not
//                                 mutate state. Inherits properties
//                                 up the rdfs:subClassOf chain so
//                                 `Person.properties` includes `name`
//                                 (declared on Thing).
//
// Cache layers:
//   1. The compressed JSON-LD is baked into the binary (include_bytes!).
//   2. A OnceLock<ParsedGraph> decodes and parses once per process; all
//      subsequent browse calls read from it in O(#types + #props).
//   3. No per-state cache. State only holds the mount metadata; browse
//      is pure on (state, path, parsed_graph).
//
// A follow-up lane can layer a per-state cache on top (populate cells
// for the types users actually browse, so compile can see them). The
// shape defined here is the foundation that layer would sit on.

use crate::ast::{Object, cell_push_unique, fact_from_pairs, fetch_or_phi};
use crate::sync::OnceLock;
use alloc::{string::{String, ToString}, vec::Vec, vec, format, borrow::ToOwned};
use hashbrown::{HashMap, HashSet};
use serde_json::Value;

/// The External System name this connector produces. Callers filter by
/// this in UI `ResourceDefinition`s and OpenAPI route emission.
pub const SYSTEM_NAME: &str = "schema.org";

/// Canonical catalog URL — used verbatim as the External System's URL
/// field so downstream citation (#305) can cite the vocabulary.
pub const SYSTEM_URL: &str = "https://schema.org/";

/// Six core Noun rows emitted at mount time. Anchors the mount so the
/// UI's External Systems section has a non-empty list on first load;
/// every other type is resolved on demand by `browse`.
pub const CORE_TYPES: &[&str] = &[
    "Thing", "Person", "Organization", "Event", "Place", "Action",
];

const RAW_GZ: &[u8] = include_bytes!("schema_org.jsonld.gz");

static GRAPH: OnceLock<ParsedGraph> = OnceLock::new();

struct ParsedGraph {
    /// Local name → direct supertypes (local names; non-schema parents
    /// like rdfs:Class are preserved so users can see the cross-vocab
    /// edge, but the UI filters them out of nav).
    supers_by_name: HashMap<String, Vec<String>>,
    /// Local name → direct subtypes (inverted supers map, built once).
    subs_by_name: HashMap<String, Vec<String>>,
    /// Every property whose domain includes this type (direct only —
    /// inheritance is resolved at browse time).
    props_by_domain: HashMap<String, Vec<PropertyEdge>>,
    /// Every declared type (includes DataType, Enumeration, etc.).
    known_types: HashSet<String>,
    /// DataType + all descendants. Rendered as objectType = "value".
    value_types: HashSet<String>,
}

#[derive(Clone)]
struct PropertyEdge {
    name: String,
    ranges: Vec<String>,
}

fn parsed_graph() -> &'static ParsedGraph {
    GRAPH.get_or_init(|| {
        let text = decompress_gz(RAW_GZ)
            .expect("schema.org JSON-LD: gzip decompress must not fail in-tree");
        let root: Value = serde_json::from_str(&text)
            .expect("schema.org JSON-LD: parse must not fail in-tree");
        build_graph(&root)
    })
}

fn decompress_gz(bytes: &[u8]) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut gz = flate2::read::GzDecoder::new(bytes);
    let mut out = String::with_capacity(bytes.len() * 6);
    gz.read_to_string(&mut out)?;
    Ok(out)
}

fn build_graph(root: &Value) -> ParsedGraph {
    let graph = root.get("@graph").and_then(Value::as_array)
        .cloned().unwrap_or_default();

    let mut supers_by_name: HashMap<String, Vec<String>> = HashMap::new();
    let mut props_by_domain: HashMap<String, Vec<PropertyEdge>> = HashMap::new();
    let mut known_types: HashSet<String> = HashSet::new();
    // schema.org marks value types by adding "schema:DataType" to @type,
    // not via rdfs:subClassOf. Nodes like schema:Text carry
    // @type: ["schema:DataType","rdfs:Class"] — the subClassOf chain is
    // empty. Collect these directly so DataType descendants surface as
    // value types regardless of subClassOf shape.
    let mut explicit_value_types: HashSet<String> = HashSet::new();

    for node in graph.iter() {
        let Some(id) = node.get("@id").and_then(Value::as_str) else { continue };
        let Some(local) = strip_schema_prefix(id) else { continue };
        let types = type_set(node);

        if types.iter().any(|t| *t == "rdfs:Class") {
            let supers = id_values(node.get("rdfs:subClassOf"))
                .into_iter()
                .map(|raw| strip_schema_prefix(&raw)
                    .map(str::to_owned)
                    .unwrap_or(raw))
                .collect::<Vec<_>>();
            supers_by_name.insert(local.to_string(), supers);
            known_types.insert(local.to_string());
            if types.iter().any(|t| *t == "schema:DataType") {
                explicit_value_types.insert(local.to_string());
            }
            continue;
        }

        if types.iter().any(|t| *t == "rdf:Property") {
            let domains = id_values(node.get("schema:domainIncludes"))
                .into_iter()
                .filter_map(|raw| strip_schema_prefix(&raw).map(str::to_owned))
                .collect::<Vec<String>>();
            let ranges = id_values(node.get("schema:rangeIncludes"))
                .into_iter()
                .filter_map(|raw| strip_schema_prefix(&raw).map(str::to_owned))
                .collect::<Vec<String>>();
            if domains.is_empty() || ranges.is_empty() {
                continue;
            }
            let edge = PropertyEdge { name: local.to_string(), ranges };
            for dom in domains.iter() {
                props_by_domain.entry(dom.clone()).or_default().push(edge.clone());
            }
        }
    }

    // Invert supers → subs.
    let mut subs_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parents) in supers_by_name.iter() {
        for p in parents.iter() {
            subs_by_name.entry(p.clone()).or_default().push(child.clone());
        }
    }

    // DataType's transitive subclasses + explicit @type: schema:DataType
    // nodes. Union because the two sources overlap (DataType appears in
    // both) but neither subsumes the other.
    let mut value_types = descendants_of("DataType", &supers_by_name);
    value_types.extend(explicit_value_types.into_iter());
    // Each explicit value-type node's subtree should also count as
    // values, so e.g. schema:Integer (subClassOf schema:Number) is a
    // value even though only Number carries the schema:DataType marker.
    let snapshot: Vec<String> = value_types.iter().cloned().collect();
    for root in snapshot.iter() {
        value_types.extend(descendants_of(root, &supers_by_name).into_iter());
    }

    ParsedGraph {
        supers_by_name,
        subs_by_name,
        props_by_domain,
        known_types,
        value_types,
    }
}

/// BFS over the inverted super graph to collect every transitive
/// subclass of `root`. Used to mark DataType's subtree as value types.
fn descendants_of(root: &str, supers_by_name: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (child, parents) in supers_by_name.iter() {
        for p in parents.iter() {
            children.entry(p.as_str()).or_default().push(child.as_str());
        }
    }
    let mut out = HashSet::new();
    out.insert(root.to_string());
    let mut frontier: Vec<&str> = vec![root];
    while let Some(cur) = frontier.pop() {
        let Some(kids) = children.get(cur) else { continue };
        for k in kids.iter() {
            if out.insert(k.to_string()) {
                frontier.push(*k);
            }
        }
    }
    out
}

fn strip_schema_prefix(id: &str) -> Option<&str> {
    id.strip_prefix("schema:")
}

fn type_set(node: &Value) -> Vec<&str> {
    match node.get("@type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(xs)) => xs.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn id_values(v: Option<&Value>) -> Vec<String> {
    let Some(v) = v else { return Vec::new(); };
    match v {
        Value::Object(_) => v.get("@id").and_then(Value::as_str)
            .map(|s| vec![s.to_string()]).unwrap_or_default(),
        Value::Array(xs) => xs.iter().filter_map(|x|
            x.get("@id").and_then(Value::as_str).map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

// ── Mount (minimal metadata, no catalog walk) ────────────────────────

/// Mount schema.org into `state`. Emits:
///   - External System { name: "schema.org" } row
///   - InstanceFact rows for URL / Kind / Prefix
///   - Noun rows for the six core types (CORE_TYPES)
///   - InstanceFact backed-by rows for each core type
///
/// Does NOT walk the full catalog. Users discover other types via
/// `browse`, which reads the parsed-graph cache directly.
///
/// Idempotent: every push uses `cell_push_unique`.
pub fn mount(state: &Object) -> Object {
    let state = mount_external_system(state);
    mount_core_types(state)
}

fn mount_external_system(state: &Object) -> Object {
    let state = cell_push_unique(
        "External System",
        fact_from_pairs(&[("name", SYSTEM_NAME)]),
        state,
    );

    let rows: [(&str, &str, &str); 3] = [
        ("URL",    "URL",    SYSTEM_URL),
        ("Kind",   "Kind",   "federated"),
        ("Prefix", "Prefix", SYSTEM_NAME),
    ];
    rows.iter().fold(state, |acc, (field, object_noun, value)| {
        cell_push_unique(
            "InstanceFact",
            fact_from_pairs(&[
                ("subjectNoun",  "External System"),
                ("subjectValue", SYSTEM_NAME),
                ("fieldName",    field),
                ("objectNoun",   object_noun),
                ("objectValue",  value),
            ]),
            &acc,
        )
    })
}

fn mount_core_types(state: Object) -> Object {
    let graph = parsed_graph();
    CORE_TYPES.iter().fold(state, |acc, name| {
        let obj_type = if graph.value_types.contains(*name) { "value" } else { "entity" };
        let with_noun = cell_push_unique(
            "Noun",
            fact_from_pairs(&[
                ("name",                         name),
                ("objectType",                   obj_type),
                ("is_backed_by_external_system", SYSTEM_NAME),
            ]),
            &acc,
        );
        cell_push_unique(
            "InstanceFact",
            fact_from_pairs(&[
                ("subjectNoun",  "Noun"),
                ("subjectValue", name),
                ("fieldName",    "is_backed_by_External_System"),
                ("objectNoun",   "External System"),
                ("objectValue",  SYSTEM_NAME),
            ]),
            &with_noun,
        )
    })
}

/// Returns true if schema.org has already been mounted into `state`.
/// Used by callers that want to mount-on-demand without racing.
pub fn is_mounted(state: &Object) -> bool {
    let cell = fetch_or_phi("External System", state);
    cell.as_seq()
        .map(|items| items.iter()
            .any(|f| crate::ast::binding(f, "name") == Some(SYSTEM_NAME)))
        .unwrap_or(false)
}

// ── Browse (populate-on-call, reads parsed graph) ────────────────────

/// Does the parsed graph know about `type_name`?
pub fn has_type(type_name: &str) -> bool {
    parsed_graph().known_types.contains(type_name)
}

/// Every rdfs:Class local-name in the schema.org graph, in HashMap
/// iteration order (stable within a process). Used by
/// `GET /external/schema.org/types` and the top-level UI listing.
pub fn all_known_types() -> Vec<String> {
    parsed_graph().known_types.iter().cloned().collect()
}

/// Direct supertypes from the parsed graph.
pub fn supertypes(type_name: &str) -> Vec<String> {
    parsed_graph().supers_by_name.get(type_name)
        .cloned().unwrap_or_default()
}

/// Direct subtypes from the parsed graph.
pub fn subtypes(type_name: &str) -> Vec<String> {
    parsed_graph().subs_by_name.get(type_name)
        .cloned().unwrap_or_default()
}

/// Properties declared on `type_name` OR any ancestor, with the first
/// range from each property's rangeIncludes list. Walks the supertype
/// chain within schema.org only (stops at cross-vocab parents) so a
/// schema:Thing sub-property like `name` surfaces on Person, Place, etc.
///
/// Returns `(property_name, range)` pairs in declaration order within
/// each class, ancestors first (root → leaf), deduped by property name.
pub fn inherited_properties(type_name: &str) -> Vec<(String, String)> {
    let graph = parsed_graph();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();

    // Ancestor chain, root → self. Walks only schema: parents; stops
    // at the first non-schema parent (e.g. rdfs:Class) so Enumeration
    // doesn't pick up rdfs-level properties.
    let chain = ancestor_chain_root_first(type_name, &graph.supers_by_name,
        &graph.known_types);
    for cls in chain.iter() {
        let Some(edges) = graph.props_by_domain.get(cls) else { continue };
        for e in edges.iter() {
            if seen.insert(e.name.clone()) {
                let range = e.ranges.first().cloned().unwrap_or_default();
                out.push((e.name.clone(), range));
            }
        }
    }
    out
}

fn ancestor_chain_root_first(
    start: &str,
    supers: &HashMap<String, Vec<String>>,
    known: &HashSet<String>,
) -> Vec<String> {
    // Iterative: walk supers, pick the first schema: parent at each
    // step. For multiple inheritance (rare in schema.org) we traverse
    // the first chain only — the repeated-property dedupe in
    // `inherited_properties` catches overlap.
    let mut chain: Vec<String> = Vec::new();
    let mut cur = start.to_string();
    let mut guard: HashSet<String> = HashSet::new();
    loop {
        if !guard.insert(cur.clone()) { break; }
        chain.push(cur.clone());
        let Some(parents) = supers.get(&cur) else { break };
        let next = parents.iter().find(|p| known.contains(p.as_str()));
        match next {
            Some(p) => { cur = p.clone(); }
            None    => break,
        }
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::binding;

    #[test]
    fn parsed_graph_loads_person_with_thing_supertype() {
        let g = parsed_graph();
        assert!(g.known_types.contains("Person"));
        assert!(g.known_types.contains("Thing"));
        let supers = g.supers_by_name.get("Person").expect("Person has supers");
        assert!(supers.iter().any(|s| s == "Thing"),
            "Person's direct supertype must include Thing");
    }

    #[test]
    fn parsed_graph_marks_text_as_value_type() {
        let g = parsed_graph();
        assert!(g.value_types.contains("Text"),
            "schema:Text descends from DataType and must be a value type");
        assert!(!g.value_types.contains("Person"),
            "Person must not be a value type");
    }

    #[test]
    fn mount_produces_external_system_cell() {
        let state = mount(&Object::phi());
        let cell = fetch_or_phi("External System", &state);
        let facts = cell.as_seq().expect("External System cell must exist");
        assert!(facts.iter().any(|f| binding(f, "name") == Some(SYSTEM_NAME)));
    }

    #[test]
    fn mount_is_minimal_six_core_nouns_not_thousands() {
        let state = mount(&Object::phi());
        let cell = fetch_or_phi("Noun", &state);
        let facts = cell.as_seq().expect("Noun cell must exist");
        let backed: Vec<&str> = facts.iter()
            .filter(|f| binding(f, "is_backed_by_external_system") == Some(SYSTEM_NAME))
            .filter_map(|f| binding(f, "name"))
            .collect();
        // Populate-on-browse: mount only seeds the core-type anchors.
        assert_eq!(backed.len(), CORE_TYPES.len(),
            "mount must NOT walk the catalog; expected {} core types, got {} backed Nouns",
            CORE_TYPES.len(), backed.len());
        for core in CORE_TYPES {
            assert!(backed.iter().any(|n| n == core),
                "core type '{core}' missing from mounted Nouns");
        }
    }

    #[test]
    fn mount_adds_backed_by_instance_facts_for_core_types() {
        let state = mount(&Object::phi());
        let inst = fetch_or_phi("InstanceFact", &state);
        let facts = inst.as_seq().expect("InstanceFact cell must exist");
        // compile.rs:1444 filter shape — if this breaks, federation
        // discovery breaks silently.
        let backed: Vec<&str> = facts.iter()
            .filter(|f| binding(f, "fieldName")
                .map(|s| s.contains("backed")).unwrap_or(false))
            .filter(|f| binding(f, "objectNoun") == Some("External System"))
            .filter(|f| binding(f, "objectValue") == Some(SYSTEM_NAME))
            .filter_map(|f| binding(f, "subjectValue"))
            .collect();
        assert_eq!(backed.len(), CORE_TYPES.len());
    }

    #[test]
    fn mount_is_idempotent() {
        let a = mount(&Object::phi());
        let b = mount(&a);
        for cell in ["External System", "Noun", "InstanceFact"] {
            let la = fetch_or_phi(cell, &a).as_seq().map(|s| s.len()).unwrap_or(0);
            let lb = fetch_or_phi(cell, &b).as_seq().map(|s| s.len()).unwrap_or(0);
            assert_eq!(la, lb, "mount({cell}) must be idempotent");
        }
    }

    #[test]
    fn supertypes_of_person_includes_thing() {
        let sups = supertypes("Person");
        assert!(sups.iter().any(|s| s == "Thing"));
    }

    #[test]
    fn inherited_properties_of_person_include_name_from_thing() {
        let props = inherited_properties("Person");
        let names: Vec<&str> = props.iter().map(|p| p.0.as_str()).collect();
        assert!(names.iter().any(|n| *n == "name"),
            "Person inherits 'name' from Thing; props = {} entries, missing 'name'", names.len());
        assert!(names.iter().any(|n| *n == "birthDate"),
            "Person has 'birthDate' directly");
    }

    #[test]
    fn inherited_properties_deduplicate_across_ancestors() {
        // Every ancestor walk must dedupe: re-asking Person's props
        // should never produce two entries with the same name.
        let props = inherited_properties("Person");
        let mut names: Vec<String> = props.iter().map(|p| p.0.clone()).collect();
        names.sort();
        let unique_count = names.iter().enumerate()
            .filter(|(i, n)| *i == 0 || &names[*i - 1] != *n)
            .count();
        assert_eq!(unique_count, names.len(),
            "inherited_properties must dedupe by property name");
    }

    #[test]
    fn has_type_discriminates_known_from_unknown() {
        assert!(has_type("Person"));
        assert!(has_type("Thing"));
        assert!(!has_type("ZzNonexistentType"));
    }

    #[test]
    fn is_mounted_false_before_true_after() {
        assert!(!is_mounted(&Object::phi()));
        assert!(is_mounted(&mount(&Object::phi())));
    }
}
