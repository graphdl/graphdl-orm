// crates/arest/src/rmap.rs
//
// RMAP -- Relational Mapping Procedure (Halpin, Ch. 10)
//
// Pure function: Object state -> table definitions.
// No I/O, no mutable global state. The schema defines what exists;
// RMAP computes how it maps to relations.
//
// Steps:
//   0.1. Binarize exclusive unaries (XO -> status column)
//   0.3. Subtype absorption (partitioned if subtype has own facts, else single-table)
//   1.   Compound UC -> separate table (M:N, ternary+)
//   2.   Functional roles -> grouped into entity table
//   2.5. External UC -> UNIQUE constraint on cross-fact-type spans
//   3.   1:1 absorption (mandatory > entity-over-value > larger-table > reading-dir)
//   3.5. Compound reference scheme -> composite PK
//   4.   Independent entity -> single-column table
//   6.   Constraint mapping (UC -> keys, MC -> NOT NULL, VC -> CHECK, SS -> FK)

// Serde derives + per-field `#[serde(...)]` attrs gate on `std-deps`
// (#653 / #588). Under no_std (kernel build), `serde` is not in the
// crate graph at all, so every derive is wrapped in
// `cfg_attr(feature = "std-deps", ...)` to keep the type definitions
// no_std-clean. Round-tripping through serde continues to work in the
// std build.
#[cfg(feature = "std-deps")]
use serde::{Serialize, Deserialize};
use hashbrown::{HashMap, HashSet};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// -- Output types -----------------------------------------------------

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct TableColumn {
    pub name: String,
    #[cfg_attr(feature = "std-deps", serde(rename = "type"))]
    pub col_type: String,
    pub nullable: bool,
    #[cfg_attr(feature = "std-deps", serde(skip_serializing_if = "Option::is_none"))]
    pub references: Option<String>,
    /// rmap-3nf-tables Stage 1b — PROVENANCE for the row projection:
    /// the population cell this column's values come from. None for
    /// synthesized columns (the PK id, xo discriminators). Together
    /// with `source_subject_role` / `source_value_role` the persist
    /// path can replay exactly this column's data out of the cell
    /// graph without re-deriving (and drifting from) the final
    /// decorated name.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Option::is_none"))]
    pub source_cell: Option<String>,
    /// The role (noun name) whose binding keys the row — the absorbing
    /// entity's role in the source fact.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Option::is_none"))]
    pub source_subject_role: Option<String>,
    /// The role (noun name) whose binding supplies this column's value.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Option::is_none"))]
    pub source_value_role: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub primary_key: Vec<String>,
    #[cfg_attr(feature = "std-deps", serde(skip_serializing_if = "Option::is_none"))]
    pub checks: Option<Vec<String>>,
    /// Additional UNIQUE constraints (each inner Vec is a set of column names)
    #[cfg_attr(feature = "std-deps", serde(skip_serializing_if = "Option::is_none"))]
    pub unique_constraints: Option<Vec<Vec<String>>>,
    /// rmap-3nf-tables Stage 1b — the REFERENCE-SCHEME value column of
    /// an entity table (e.g. `reference` on `resource` from
    /// `Resource(.Reference)`), when the entity has a single-noun ref
    /// scheme and the column was absorbed. The row projection defaults
    /// it to the row's id when no explicit fact carries it: the
    /// synthetic id IS the reference-scheme value (Halpin — the
    /// reference mode identifies the entity), so populations that
    /// store bare ids still satisfy the NOT NULL refmode column.
    #[cfg_attr(feature = "std-deps", serde(default, skip_serializing_if = "Option::is_none"))]
    pub ref_value_column: Option<String>,
}

// -- Helpers ----------------------------------------------------------

pub fn to_snake(name: &str) -> String {
    name.chars().enumerate().fold(String::new(), |mut acc, (i, ch)| {
        (ch.is_uppercase() && i > 0 && name.chars().nth(i - 1).map_or(false, |p| p.is_lowercase()))
            .then(|| acc.push('_'));
        match ch {
            ' ' | '-' => acc.push('_'),
            _ => acc.push(ch.to_lowercase().next().unwrap_or(ch)),
        }
        acc
    })
}

fn fk_column_name(noun_name: &str) -> String {
    format!("{}_id", to_snake(noun_name))
}

fn value_column_name(noun_name: &str) -> String {
    to_snake(noun_name)
}

fn column_name_for_target(nouns: &HashMap<String, crate::types::NounDef>, noun_name: &str) -> String {
    match nouns.get(noun_name) {
        Some(noun) if noun.object_type == "value" => value_column_name(noun_name),
        _ => fk_column_name(noun_name),
    }
}

/// rmap-3nf-tables: NORMA's phase-1 column name — PREDICATE-TEXT
/// DECORATION (user-ratified: "the name just becomes fromStatus; use
/// the NORMA logic"; reference NameGeneration.cs `GenerateColumnName`
/// phase 0/1 + `decorateWithPredicateText`). When two absorbed columns
/// in one table collide on the phase-0 name (two functional FTs
/// absorbing the same target noun — `Transition is from Status` and
/// `Transition is to Status` both yielding `status_id`), the colliding
/// columns regenerate with the predicate text between the role players:
/// the reading minus the near player, minus the far player, minus
/// leading copula stop-words — `is from` → `from` → `from_status_id`.
///
/// Returns None when the reading doesn't contain both players in
/// order, or when nothing remains after the strip (a bare copula like
/// `has` keeps the stop-word so `has_url` still disambiguates against
/// a sibling like `pins_url`).
fn decorated_column_name(
    nouns: &HashMap<String, crate::types::NounDef>,
    reading: &str,
    near_noun: &str,
    far_noun: &str,
) -> Option<String> {
    let near_idx = reading.find(near_noun)?;
    let after_near = near_idx + near_noun.len();
    let far_rel = reading[after_near..].find(far_noun)?;
    let middle = reading[after_near..after_near + far_rel].trim();
    // NORMA also appends the post-placeholder reading text
    // (NameGeneration.cs ~728: the tail after the far role) — that's
    // where qualifier-tail readings carry their disambiguator:
    // `Material Touch Target has Dp as minimum width` → tail
    // `as minimum width` → dp_as_minimum_width.
    let tail = reading[after_near + far_rel + far_noun.len()..]
        .trim().trim_end_matches('.').trim();
    if middle.is_empty() && tail.is_empty() {
        return None;
    }
    const COPULAS: &[&str] = &["is", "has", "was", "are", "were", "does"];
    let words: Vec<&str> = middle.split_whitespace().collect();
    let mut kept: Vec<&str> = words.iter()
        .skip_while(|w| COPULAS.contains(&w.to_lowercase().as_str()))
        .copied()
        .collect();
    if kept.is_empty() && tail.is_empty() {
        // Pure copula predicate (`has`) with no tail — keep it rather
        // than emitting nothing: `has_url` vs `pins_url` still
        // disambiguates.
        kept = words;
    }
    let mut parts: Vec<String> = kept.iter().map(|w| to_snake(w)).collect();
    parts.push(column_name_for_target(nouns, far_noun));
    parts.extend(tail.split_whitespace().map(to_snake));
    Some(parts.join("_"))
}

fn compound_table_name(reading: &str, roles: &[crate::types::RoleDef], noun_names: &HashSet<String>) -> String {
    let words: Vec<&str> = reading.split_whitespace().collect();
    let has_verbs = words.iter().any(|w| !noun_names.contains(*w));
    if has_verbs {
        words.iter().map(|w| to_snake(w)).collect::<Vec<_>>().join("_")
    } else {
        to_snake(&roles.iter().map(|r| r.noun_name.as_str()).collect::<Vec<_>>().join("_"))
    }
}

// -- RMAP core --------------------------------------------------------

/// RMAP from Object state â€” reads cells directly. No Domain round-trip.
pub fn rmap_from_state(state: &crate::ast::Object) -> Vec<TableDef> {
    rmap(state)
}

/// RMAP as cells: `RMAPTable` + `RMAPColumn` rows covering the same
/// information `Vec<TableDef>` exposes today, but as an `Object::Map`
/// that downstream generators can read directly — no typed struct
/// boundary (#325).
///
/// `RMAPTable` rows carry one fact per table:
///   name          — snake_case table name
///   primaryKey    — comma-separated PK column names
///   uniqueConstraints (optional) — semicolon-separated groups of
///                   comma-separated columns (e.g. `a,b;c,d`)
///
/// `RMAPColumn` rows carry one fact per (table, column):
///   table         — owning table's name
///   name          — column name
///   colType       — SQL type string
///   nullable      — `true` / `false`
///   position      — zero-based declaration order
///   references (optional) — referenced table (FK target)
///
/// Column ordering is preserved via the `position` field; callers who
/// want columns in declaration order should sort by it (the helpers in
/// this module already do).
pub fn rmap_cells_from_state(state: &crate::ast::Object) -> crate::ast::Object {
    use crate::ast::{Object, fact_from_pairs};
    let tables = rmap(state);
    let mut table_rows: Vec<Object> = Vec::new();
    let mut column_rows: Vec<Object> = Vec::new();

    for t in &tables {
        let pk_joined = t.primary_key.join(",");
        let mut table_pairs: Vec<(&str, String)> = vec![
            ("name", t.name.clone()),
            ("primaryKey", pk_joined),
        ];
        let encoded_ucs = t.unique_constraints.as_ref().map(|ucs|
            ucs.iter().map(|uc| uc.join(",")).collect::<Vec<_>>().join(";"));
        if let Some(enc) = encoded_ucs.as_ref() {
            table_pairs.push(("uniqueConstraints", enc.clone()));
        }
        let pair_refs: Vec<(&str, &str)> = table_pairs.iter()
            .map(|(k, v)| (*k, v.as_str())).collect();
        table_rows.push(fact_from_pairs(&pair_refs));

        for (i, c) in t.columns.iter().enumerate() {
            let pos = i.to_string();
            let nullable = if c.nullable { "true" } else { "false" };
            let mut col_pairs: Vec<(&str, String)> = vec![
                ("table", t.name.clone()),
                ("name", c.name.clone()),
                ("colType", c.col_type.clone()),
                ("nullable", nullable.to_string()),
                ("position", pos),
            ];
            if let Some(r) = c.references.as_ref() {
                col_pairs.push(("references", r.clone()));
            }
            let pair_refs: Vec<(&str, &str)> = col_pairs.iter()
                .map(|(k, v)| (*k, v.as_str())).collect();
            column_rows.push(fact_from_pairs(&pair_refs));
        }
    }

    let mut map: HashMap<String, Object> = HashMap::new();
    map.insert("RMAPTable".to_string(), Object::Seq(table_rows.into()));
    map.insert("RMAPColumn".to_string(), Object::Seq(column_rows.into()));
    Object::Map(map.into())
}

// -- Cell reader helpers for downstream generators (#325) -------------
//
// Downstream generators consume RMAP output through these helpers so
// they never hold a `TableDef` / `TableColumn` value. The typed structs
// survive inside rmap.rs as working types; crate-internal callers may
// still use them (e.g. `compile.rs` for DDL emission). New consumers
// should prefer these cell-readers.

/// Cell-backed view of a column. Crate-internal by design — a thin
/// borrow struct that lets generators read the four fields they need
/// without importing the public `TableColumn` serialization IR.
#[derive(Debug, Clone)]
pub(crate) struct ColumnView {
    pub name: String,
    pub col_type: String,
    pub nullable: bool,
    pub references: Option<String>,
}

/// Return the RMAP table name for an entity noun, if one exists in
/// the rmap-cells view. Value types and unreferenced nouns map to
/// `None` so callers can skip them uniformly.
pub fn table_name_for_noun(cells: &crate::ast::Object, noun_name: &str) -> Option<String> {
    let snake = to_snake(noun_name);
    let rows = crate::ast::fetch_cell_seq("RMAPTable", cells);
    rows.as_seq()?.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(snake.as_str()))
        .and_then(|f| crate::ast::binding(f, "name").map(String::from))
}

/// Return every column of a table in declaration order (sorted by the
/// `position` field). Returns an empty vec if the table is unknown.
pub(crate) fn columns_for_table(cells: &crate::ast::Object, table_name: &str) -> Vec<ColumnView> {
    let rows = crate::ast::fetch_cell_seq("RMAPColumn", cells);
    let Some(seq) = rows.as_seq() else { return Vec::new(); };
    let mut with_pos: Vec<(usize, ColumnView)> = seq.iter()
        .filter(|f| crate::ast::binding(f, "table") == Some(table_name))
        .map(|f| {
            let pos: usize = crate::ast::binding(f, "position")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let view = ColumnView {
                name: crate::ast::binding(f, "name").unwrap_or("").to_string(),
                col_type: crate::ast::binding(f, "colType").unwrap_or("").to_string(),
                nullable: crate::ast::binding(f, "nullable") == Some("true"),
                references: crate::ast::binding(f, "references").map(String::from),
            };
            (pos, view)
        })
        .collect();
    with_pos.sort_by_key(|(p, _)| *p);
    with_pos.into_iter().map(|(_, v)| v).collect()
}

/// Return a table's UNIQUE constraints as a `Vec<Vec<String>>` — each
/// inner Vec is one UNIQUE set of column names. Empty when the table
/// has no extra UNIQUE beyond its primary key.
///
/// Decodes the `uniqueConstraints` binding on the RMAPTable row: the
/// wire form is `a,b;c,d` → `[[a,b],[c,d]]` (semicolons between UC
/// groups, commas within a group). Matches the encoding from
/// `rmap_cells_from_state`.
pub fn unique_constraints_of_table(cells: &crate::ast::Object, table_name: &str) -> Vec<Vec<String>> {
    let rows = crate::ast::fetch_cell_seq("RMAPTable", cells);
    let Some(seq) = rows.as_seq() else { return Vec::new(); };
    seq.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(table_name))
        .and_then(|f| crate::ast::binding(f, "uniqueConstraints"))
        .filter(|s| !s.is_empty())
        .map(|s| s.split(';')
            .map(|grp| grp.split(',').map(|c| c.to_string()).collect())
            .collect())
        .unwrap_or_default()
}

/// Every table name in the RMAP cells view, in declaration order.
pub fn table_names(cells: &crate::ast::Object) -> Vec<String> {
    let rows = crate::ast::fetch_cell_seq("RMAPTable", cells);
    rows.as_seq()
        .map(|seq| seq.iter()
            .filter_map(|f| crate::ast::binding(f, "name").map(String::from))
            .collect())
        .unwrap_or_default()
}

/// Return the primary-key columns of a table in order. Empty when the
/// table has no `RMAPTable` row or the `primaryKey` binding is empty.
pub fn primary_key_of_table(cells: &crate::ast::Object, table_name: &str) -> Vec<String> {
    let rows = crate::ast::fetch_cell_seq("RMAPTable", cells);
    let Some(seq) = rows.as_seq() else { return Vec::new(); };
    seq.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(table_name))
        .and_then(|f| crate::ast::binding(f, "primaryKey"))
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(|p| p.to_string()).collect())
        .unwrap_or_default()
}


/// #214: RMAP as a Func tree entry point.
///
/// Returns a `Func::Platform("rmap")` leaf that, when applied via
/// `ast::apply` against the state Object, produces an `Object::atom`
/// containing the JSON-serialized `Vec<TableDef>`. This is the
/// ρ-dispatchable form of RMAP — any caller that operates on Func
/// trees (including future lowered compile pipelines or the MCP
/// dispatch layer) can treat RMAP as a first-class ρ-application
/// instead of reaching in to the Rust procedure directly.
///
/// The platform body still wraps Halpin's procedural Ch. 10
/// algorithm. A deeper FFP rewrite would decompose the six RMAP
/// passes (binarize → absorb → classify-UC → one-to-one-absorb →
/// compound-ref-scheme → constraint-map) into a `FoldL` over per-pass
/// Funcs, each reading / augmenting an intermediate state Object.
/// That decomposition is tracked as a follow-up; routing through
/// Platform preserves every current behaviour while making the leaf
/// introspectable and per-runtime-routable.
///
/// std-deps gate: serializes through `serde_json`. Under no_std the
/// rmap procedure itself is reachable directly via `rmap(state)`; this
/// Func-tree entry exists for std-host MCP dispatch / lowered compile
/// pipelines where serde_json is already linked.
///
/// H4 (#692): this used to wrap an `Arc<dyn Fn>` in `Func::Native` —
/// the largest typed-IR-residue hazard remaining after H1-H3. RMAP
/// is now a named `Func::Platform("rmap")` op dispatched through
/// `apply_platform`; the closure body lives in `ast::platform_rmap`.
/// The leaf is now introspectable (kind = Platform, name = "rmap"),
/// freezable / replayable, and routable per runtime: each target
/// (server / FPGA / Solidity) can install its own table-mapping
/// strategy under the same name.
#[cfg(feature = "std-deps")]
pub fn rmap_func() -> crate::ast::Func {
    crate::ast::Func::Platform("rmap".to_string())
}

/// Decode the output of `apply(rmap_func(), state, state)` back into
/// `Vec<TableDef>`. The Func emits a JSON atom; this helper is the
/// inverse of that encoding.
#[cfg(feature = "std-deps")]
pub fn decode_rmap_result(obj: &crate::ast::Object) -> Vec<TableDef> {
    obj.as_atom()
        .and_then(|s| serde_json::from_str::<Vec<TableDef>>(s).ok())
        .unwrap_or_default()
}

pub fn rmap(state: &crate::ast::Object) -> Vec<TableDef> {
    use crate::ast::binding;
    use crate::types::*;

    // Build typed lookups from cells â€” same data state_to_domain
    // produced, without the Domain struct. Reads via cell_facts_iter
    // (NOT as_seq) so Map-keyed metamodel cells (#744 / #932 storage
    // duality) drive the projection identically to Seq cells.
    let noun_cell = crate::ast::fetch_or_phi("Noun", state);
    let mut nouns: HashMap<String, NounDef> = HashMap::new();
    let mut subtypes: HashMap<String, String> = HashMap::new();
    let mut ref_schemes: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();
    for f in crate::ast::cell_facts_iter(&noun_cell) {
        let name = binding(f, "name").unwrap_or("").to_string();
        let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
        nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
        if let Some(st) = binding(f, "superType") { subtypes.insert(name.clone(), st.to_string()); }
        if let Some(v) = binding(f, "referenceScheme") { ref_schemes.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
        if let Some(v) = binding(f, "enumValues") { enum_values.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
    }
    let role_cell = crate::ast::fetch_or_phi("Role", state);
    let role_facts: Vec<&crate::ast::Object> = crate::ast::cell_facts_iter(&role_cell).collect();
    let ft_cell = crate::ast::fetch_or_phi("FactType", state);
    let fact_types: HashMap<String, FactTypeDef> = crate::ast::cell_facts_iter(&ft_cell)
        .filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<RoleDef> = role_facts.iter()
                .filter(|r| binding(r, "factType") == Some(&id))
                .map(|r| RoleDef {
                    noun_name: binding(r, "nounName").unwrap_or("").to_string(),
                    role_index: binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                }).collect();
            Some((id, FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect();
    let constraint_cell = crate::ast::fetch_or_phi("Constraint", state);
    let constraints: Vec<ConstraintDef> = crate::ast::cell_facts_iter(&constraint_cell)
        .map(|f| {
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = crate::compile::decode_constraint_spans(&get);
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: get("setComparisonArgumentLength")
                    .and_then(|s| s.parse().ok()),
                clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
                predicate: None,
            }
        }).collect();

    let mut tables: Vec<TableDef> = Vec::new();
    let mut emitted: HashSet<String> = HashSet::new();

    // -- Step 0.1: Binarize exclusive unaries ------------------------
    let mut binarized_ft_ids: HashSet<String> = HashSet::new();
    let mut xo_columns: HashMap<String, Vec<(String, Vec<String>, bool)>> = HashMap::new();

    constraints.iter()
        .filter(|c| c.kind == "XO" && c.spans.len() >= 2)
        .filter_map(|constraint| {
            let ft_ids: Vec<&str> = constraint.spans.iter().map(|s| s.fact_type_id.as_str()).collect();
            let unary_fts: Vec<_> = ft_ids.iter()
                .filter_map(|id| fact_types.get(*id))
                .filter(|ft| ft.roles.len() == 1).collect();
            (unary_fts.len() >= 2).then_some((ft_ids, unary_fts))
        })
        .for_each(|(ft_ids, unary_fts)| {
            let entity_name = &unary_fts[0].roles[0].noun_name;
            let values: Vec<String> = unary_fts.iter().map(|ft|
                ft.reading.split(" is ").last().map(|s| s.trim_end_matches('.').to_string())
                    .unwrap_or_else(|| ft.reading.split_whitespace().last().unwrap_or("").to_string())
            ).collect();
            binarized_ft_ids.extend(ft_ids.iter().map(|id| id.to_string()));
            let is_mandatory = unary_fts.iter().any(|ft| {
                let ft_id_str = ft_ids.iter().find(|id| fact_types.get(**id).map(|f| core::ptr::eq(f, *ft)).unwrap_or(false));
                ft_id_str.map_or(false, |fid| constraints.iter().any(|c| c.kind == "MC" && c.spans.iter().any(|s| s.fact_type_id == *fid)))
            });
            let col_name = if values.iter().any(|v| v.to_lowercase() == "male" || v.to_lowercase() == "female") { "sex" } else { "status" }.to_string();
            xo_columns.entry(entity_name.clone()).or_default().push((col_name, values, !is_mandatory));
        });

    // -- Step 0.3: Subtype absorption --------------------------------
    // Determine which subtypes have their own fact types (partitioned strategy)
    // vs which should be absorbed into the supertype (single-table strategy).
    let mut parent_of: HashMap<String, String> = HashMap::new();
    subtypes.iter().for_each(|(name, st)| { parent_of.insert(name.clone(), st.clone()); });
    let subtype_to_root: HashMap<String, String> = parent_of.keys().map(|name| {
        let root = core::iter::successors(Some(name.clone()), |cur| parent_of.get(cur).cloned())
            .take(100) // cycle guard
            .last().unwrap_or_else(|| name.clone());
        (name.clone(), root)
    }).collect();

    // Detect subtypes that have their own fact types -> partitioned strategy
    let partitioned_subtypes: HashSet<String> = subtype_to_root.keys()
        .filter(|name| fact_types.values().any(|ft| ft.roles.iter().any(|r| &r.noun_name == *name)))
        .cloned()
        .collect();

    let subtype_names: HashSet<&String> = subtype_to_root.keys().collect();
    let resolve_entity = |name: &str| -> String {
        // Partitioned subtypes map to themselves, not the root (Backus cond).
        if partitioned_subtypes.contains(name) {
            name.to_string()
        } else {
            subtype_to_root.get(name).cloned().unwrap_or_else(|| name.to_string())
        }
    };

    // -- Index constraints -------------------------------------------
    let (ucs_by_ft, mc_set, vcs_by_ft_role): (
        HashMap<String, Vec<Vec<usize>>>,
        HashSet<String>,
        HashMap<String, Vec<String>>,
    ) = constraints.iter().fold(
        (HashMap::new(), HashSet::new(), HashMap::new()),
        |(mut ucs, mut mc, mut vcs), c| {
            // rmap-3nf-tables Stage 3: only ALETHIC constraints shape
            // the relational schema. A deontic UC/MC is advisory
            // ("ought", violations possible) — letting it mint PKs /
            // NOT NULLs was the material_* family: deontic
            // `It is obligatory that each Material Spacing Token has
            // some Dp …` lines made dp columns NOT NULL and every
            // token row warn-skipped. Modality is the policy (user
            // ruling): deontic never rejects, so it never constrains
            // DDL either. EMPTY modality = alethic (the historic
            // default — synth / hand-built ConstraintDefs omit it).
            if !(c.modality.is_empty() || c.modality.eq_ignore_ascii_case("alethic")) {
                return (ucs, mc, vcs);
            }
            match c.kind.as_str() {
                "UC" => {
                    c.spans.iter().for_each(|span| { ucs.entry(span.fact_type_id.clone()).or_default(); });
                    // Roles this UC spans inside its OWNING fact type. The
                    // parser now emits one real span per role (the former
                    // single-role `span0`→`span1` mirror in
                    // `enrich_constraints_with_spans` was removed at the
                    // source), so a single-role functional UC already
                    // carries one role here and absorbs as a column —
                    // Halpin §10.3 rule 2. The `dedup()` is retained purely
                    // as cheap belt-and-suspenders: if any future path ever
                    // re-introduces an identical-role duplicate it must not
                    // be misread by the `uc.len() >= 2` compound test below.
                    c.spans.first()
                        .map(|s| s.fact_type_id.clone())
                        .into_iter()
                        .for_each(|ft_id| {
                            let mut roles: Vec<usize> = c.spans.iter()
                                .filter(|s| s.fact_type_id == ft_id)
                                .map(|s| s.role_index)
                                .collect();
                            roles.sort_unstable();
                            roles.dedup();
                            ucs.entry(ft_id).or_default().push(roles);
                        });
                }
                "MC" => {
                    // rmap-3nf-tables (iii): a multi-span MC is the
                    // inclusive-or (disjunctive) mandatory — satisfied
                    // by participation in ANY span's fact type. Only a
                    // SINGLE-span MC makes the role itself total, so
                    // only that shape may mint NOT NULL on an absorbed
                    // column.
                    if c.spans.len() == 1 {
                        mc.extend(c.spans.iter().map(|s| format!("{}:{}", s.fact_type_id, s.role_index)));
                    }
                }
                "VC" => {
                    c.entity.as_ref()
                        .and_then(|e| enum_values.get(e))
                        .into_iter()
                        .for_each(|vals| {
                            c.spans.iter().for_each(|span| {
                                vcs.insert(format!("{}:{}", span.fact_type_id, span.role_index), vals.clone());
                            });
                        });
                }
                _ => {}
            }
            (ucs, mc, vcs)
        },
    );

    // -- Classify fact types -----------------------------------------
    // Classify: Filter(binary âˆ§ Â¬binarized) then partition by UC arity
    let classified: Vec<(&str, bool, bool)> = fact_types.iter()
        .filter(|(ft_id, ft)| !binarized_ft_ids.contains(*ft_id) && ft.roles.len() >= 2)
        .map(|(ft_id, _)| {
            let ucs = ucs_by_ft.get(ft_id).cloned().unwrap_or_default();
            // rmap-3nf-tables (iv): an FT with NO declared UC defaults
            // to the SPANNING UC (Halpin — every FT carries at least
            // the implicit whole-tuple uniqueness), i.e. m:n → its own
            // junction table. Rings (`Task blocks Task`) carry ring
            // constraints but usually no UC and previously fell through
            // BOTH classifications, producing no table at all.
            (ft_id.as_str(),
             ucs.is_empty() || ucs.iter().any(|uc| uc.len() >= 2),
             ucs.iter().any(|uc| uc.len() == 1))
        }).collect();
    let compound_facts: Vec<&str> = classified.iter().filter(|(_, c, _)| *c).map(|(id, _, _)| *id).collect();
    let functional_facts: Vec<&str> = classified.iter().filter(|(_, _, f)| *f).map(|(id, _, _)| *id).collect();

    // Detect 1:1: both roles have single-role UCs
    let one_to_one_ft_ids: HashSet<String> = functional_facts.iter()
        .filter(|ft_id| fact_types[**ft_id].roles.len() == 2)
        .filter(|ft_id| {
            let ucs = ucs_by_ft.get(**ft_id).cloned().unwrap_or_default();
            let singles: Vec<usize> = ucs.iter().filter(|uc| uc.len() == 1).map(|uc| uc[0]).collect();
            let ft = &fact_types[**ft_id];
            singles.contains(&ft.roles[0].role_index) && singles.contains(&ft.roles[1].role_index)
        })
        .map(|id| id.to_string())
        .collect();

    // -- Step 1: Compound UC -> separate table ------------------------
    let noun_name_set: HashSet<String> = nouns.keys().cloned().collect();

    let compound_tables: Vec<TableDef> = compound_facts.iter().map(|ft_id| {
        let ft = &fact_types[*ft_id];
        // rmap-3nf-tables (iv): UC-less FTs reach here via the implicit
        // whole-tuple uniqueness — default the spanning UC to ALL roles.
        let all_roles_uc: Vec<usize> = ft.roles.iter().map(|r| r.role_index).collect();
        let owned_ucs;
        let spanning_uc: &Vec<usize> = match ucs_by_ft.get(*ft_id) {
            Some(ucs) if !ucs.is_empty() => ucs.iter().max_by_key(|uc| uc.len()).unwrap(),
            _ => { owned_ucs = all_roles_uc; &owned_ucs }
        };

        // Per-role column names, REPEATED nouns disambiguated by
        // position (rings: `Task blocks Task` → task_id, task_id_2 —
        // both FK the same parent). Computed once, reused for the PK so
        // names always align.
        let mut seen: HashMap<String, usize> = HashMap::new();
        let role_col_names: Vec<String> = ft.roles.iter().map(|role| {
            let base = column_name_for_target(&nouns, &role.noun_name);
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { base } else { format!("{}_{}", base, n) }
        }).collect();

        let columns: Vec<TableColumn> = ft.roles.iter().zip(role_col_names.iter()).map(|(role, col_name)| {
            let is_entity = nouns.get(&role.noun_name).map_or(false, |n| n.object_type == "entity");
            TableColumn {
                name: col_name.clone(),
                col_type: "TEXT".to_string(),
                nullable: false,
                references: if is_entity { Some(to_snake(&role.noun_name)) } else { None },
                // Junction row projection: every column reads ITS role
                // from the same compound-FT cell; no subject key.
                source_cell: Some((*ft_id).to_string()),
                source_subject_role: None,
                source_value_role: Some(role.noun_name.clone()),
            }
        }).collect();
        let pk_cols: Vec<String> = ft.roles.iter().zip(role_col_names.iter())
            .filter(|(role, _)| spanning_uc.contains(&role.role_index))
            .map(|(_, col_name)| col_name.clone())
            .collect();

        let table_name = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
        TableDef { name: table_name, columns, primary_key: pk_cols, checks: None, unique_constraints: None, ref_value_column: None }
    }).collect();
    emitted.extend(compound_tables.iter().map(|t| t.name.clone()));
    tables.extend(compound_tables);

    // -- Step 1b: Unary fact types -> occurrence tables ----------------
    // rmap-3nf-tables Stage 2: Halpin §10.3 open-world unaries map to
    // their OWN table (the population set), not a boolean column —
    // AREST unary cells (`Task is started`) are open-world event
    // occurrences. Columns: the entity role + the SM occurred-at stamp
    // (`transition_via_defs` appends an UNDECLARED trailing
    // <Timestamp, …> pair to trigger-event facts; nullable — pre-stamp
    // historical facts project NULL). No PK: the cell is the log, the
    // projection wipes-and-reinserts, and bag semantics stay faithful.
    let unary_tables: Vec<TableDef> = fact_types.iter()
        .filter(|(ft_id, ft)| !binarized_ft_ids.contains(*ft_id) && ft.roles.len() == 1)
        .filter(|(_, ft)| nouns.get(&ft.roles[0].noun_name)
            .map_or(false, |n| n.object_type == "entity"))
        .map(|(ft_id, ft)| {
            let role = &ft.roles[0];
            let col_name = column_name_for_target(&nouns, &role.noun_name);
            let columns = alloc::vec![
                TableColumn {
                    name: col_name,
                    col_type: "TEXT".to_string(),
                    nullable: false,
                    references: Some(to_snake(&role.noun_name)),
                    source_cell: Some(ft_id.clone()),
                    source_subject_role: None,
                    source_value_role: Some(role.noun_name.clone()),
                },
                TableColumn {
                    name: "timestamp".to_string(),
                    col_type: "TEXT".to_string(),
                    nullable: true,
                    references: None,
                    source_cell: Some(ft_id.clone()),
                    source_subject_role: None,
                    source_value_role: Some("Timestamp".to_string()),
                },
            ];
            let table_name = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
            TableDef { name: table_name, columns, primary_key: Vec::new(),
                checks: None, unique_constraints: None, ref_value_column: None }
        })
        .filter(|t| !emitted.contains(&t.name))
        .collect();
    let mut unary_tables = unary_tables;
    unary_tables.sort_by(|a, b| a.name.cmp(&b.name));
    emitted.extend(unary_tables.iter().map(|t| t.name.clone()));
    tables.extend(unary_tables);

    // -- Step 2/3: Functional, 1:1 absorption, XO injection ----------
    //
    // Three pure data streams of (entity_key, column, Option<check>),
    // reduced into entity_columns via foldl (Backus insert combining form).
    // No external state mutation â€” each stream is computed from inputs only.

    let noun_ft_count: HashMap<&str, usize> = fact_types.values()
        .flat_map(|ft| ft.roles.iter().map(|r| r.noun_name.as_str()))
        .fold(HashMap::new(), |mut acc, name| { *acc.entry(name).or_insert(0) += 1; acc });

    // Addition tuples: (table, column, CHECK values, NORMA phase-1
    // alternate name). The phase-1 name engages only when phase-0
    // names collide within a table (see the fold below).
    let functional_additions: Vec<(String, TableColumn, Option<Vec<String>>, Option<String>)> = functional_facts.iter()
        .filter(|ft_id| !one_to_one_ft_ids.contains(**ft_id))
        .flat_map(|ft_id| {
            let ft = &fact_types[*ft_id];
            ucs_by_ft.get(*ft_id).cloned().unwrap_or_default().into_iter()
                .filter(|uc| uc.len() == 1)
                .filter_map(|uc| {
                    let source_role_idx = uc[0];
                    let source_role = ft.roles.iter().find(|r| r.role_index == source_role_idx)?;
                    nouns.get(&source_role.noun_name)
                        .filter(|n| n.object_type == "entity")?;
                    Some((*ft_id, source_role, source_role_idx))
                })
                .collect::<Vec<_>>()
        })
        .flat_map(|(ft_id, source_role, source_role_idx)| {
            let ft = &fact_types[ft_id];
            let entity_key = resolve_entity(&source_role.noun_name);
            let is_subtype = subtype_names.contains(&source_role.noun_name);
            let is_mandatory = mc_set.contains(&format!("{}:{}", ft_id, source_role_idx));
            ft.roles.iter()
                .filter(|role| role.role_index != source_role_idx)
                .map(|role| {
                    let col_name = column_name_for_target(&nouns, &role.noun_name);
                    let is_entity = nouns.get(&role.noun_name).map_or(false, |n| n.object_type == "entity");
                    let column = TableColumn {
                        name: col_name.clone(),
                        col_type: "TEXT".to_string(),
                        nullable: if is_subtype { true } else { !is_mandatory },
                        references: if is_entity { Some(to_snake(&role.noun_name)) } else { None },
                        source_cell: Some((*ft_id).to_string()),
                        source_subject_role: Some(source_role.noun_name.clone()),
                        source_value_role: Some(role.noun_name.clone()),
                    };
                    let vc_key = format!("{}:{}", ft_id, role.role_index);
                    let check_values = vcs_by_ft_role.get(&vc_key).cloned();
                    // NORMA phase-1 alternate, used only on collision.
                    let phase1 = decorated_column_name(
                        &nouns, &ft.reading, &source_role.noun_name, &role.noun_name);
                    (entity_key.clone(), column, check_values, phase1)
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // 1:1 absorption: direction bias via pure if-expression chain.
    // (Control flow has no side effects â€” returns a tuple, inputs â†’ output.)
    let one_to_one_additions: Vec<(String, TableColumn, Option<Vec<String>>, Option<String>)> = one_to_one_ft_ids.iter().filter_map(|ft_id| {
        let ft = &fact_types[ft_id];
        // 1:1 absorption is a binary-only collapse: pick whichever
        // entity role is mandatory (or has more participation) and
        // fold the other-side role into it as a column. Ternary+ FTs
        // can't be absorbed this way — they'd need to choose which
        // pair to collapse, and the answer is ambiguous. The upstream
        // `one_to_one_ft_ids` filter at line 462 (`roles.len() == 2`)
        // already guarantees binary, but the bounds-check below makes
        // the contract explicit at the use site so a future drift in
        // the upstream filter degrades to an empty additions list
        // instead of an out-of-bounds panic on `ft.roles[0]/[1]`.
        let role0 = ft.roles.get(0)?;
        let role1 = ft.roles.get(1)?;
        let mc0 = mc_set.contains(&format!("{}:{}", ft_id, role0.role_index));
        let mc1 = mc_set.contains(&format!("{}:{}", ft_id, role1.role_index));

        let (absorb_into, near_noun, fk_target, is_mandatory) = if mc0 && !mc1 {
            (resolve_entity(&role0.noun_name), &role0.noun_name, &role1.noun_name, true)
        } else if mc1 && !mc0 {
            (resolve_entity(&role1.noun_name), &role1.noun_name, &role0.noun_name, true)
        } else {
            let is_entity0 = nouns.get(&role0.noun_name).map_or(false, |n| n.object_type == "entity");
            let is_entity1 = nouns.get(&role1.noun_name).map_or(false, |n| n.object_type == "entity");
            let both_mandatory = mc0 && mc1;
            if is_entity0 && !is_entity1 {
                (resolve_entity(&role0.noun_name), &role0.noun_name, &role1.noun_name, both_mandatory)
            } else if is_entity1 && !is_entity0 {
                (resolve_entity(&role1.noun_name), &role1.noun_name, &role0.noun_name, both_mandatory)
            } else {
                let count0 = noun_ft_count.get(role0.noun_name.as_str()).copied().unwrap_or(0);
                let count1 = noun_ft_count.get(role1.noun_name.as_str()).copied().unwrap_or(0);
                if count1 > count0 {
                    (resolve_entity(&role1.noun_name), &role1.noun_name, &role0.noun_name, both_mandatory)
                } else {
                    // count0 >= count1 -- default to role0 (reading direction)
                    (resolve_entity(&role0.noun_name), &role0.noun_name, &role1.noun_name, both_mandatory)
                }
            }
        };
        let is_target_entity = nouns.get(fk_target.as_str()).map_or(false, |n| n.object_type == "entity");
        let column = TableColumn {
            name: column_name_for_target(&nouns, fk_target),
            col_type: "TEXT".to_string(),
            nullable: !is_mandatory,
            references: if is_target_entity { Some(to_snake(fk_target)) } else { None },
            source_cell: Some((*ft_id).to_string()),
            source_subject_role: Some(near_noun.clone()),
            source_value_role: Some(fk_target.clone()),
        };
        let phase1 = decorated_column_name(&nouns, &ft.reading, near_noun, fk_target);
        Some((absorb_into, column, None, phase1))
    }).collect();

    let xo_additions: Vec<(String, TableColumn, Option<Vec<String>>, Option<String>)> = xo_columns.iter()
        .flat_map(|(entity_name, xo_cols)| {
            let resolved = resolve_entity(entity_name);
            xo_cols.iter().map(move |(col_name, values, nullable)| {
                let column = TableColumn {
                    name: col_name.clone(),
                    col_type: "TEXT".to_string(),
                    nullable: *nullable,
                    references: None,
                    ..Default::default()
                };
                (resolved.clone(), column, Some(values.clone()), None)
            }).collect::<Vec<_>>()
        })
        .collect();

    // Foldl all additions into entity_columns — with NORMA's two-phase
    // collision resolution (rmap-3nf-tables): phase-0 names that
    // collide WITHIN a table all switch to their phase-1
    // predicate-decorated alternates (`Transition is from Status` +
    // `Transition is to Status` → from_status_id + to_status_id, never
    // a bare duplicated status_id). Residual collisions (duplicate FT
    // declarations yielding identical decorations) take a
    // deterministic numeric suffix so the CREATE TABLE always lands.
    // CHECK constraints format AFTER final naming so they track the
    // decorated column.
    let all_additions: Vec<(String, TableColumn, Option<Vec<String>>, Option<String>)> =
        functional_additions.into_iter()
            .chain(one_to_one_additions.into_iter())
            .chain(xo_additions.into_iter())
            .collect();
    // Pass 1: phase-0 name counts per table.
    let mut phase0_counts: HashMap<(String, String), usize> = HashMap::new();
    for (key, col, _, _) in all_additions.iter() {
        *phase0_counts.entry((key.clone(), col.name.clone())).or_insert(0) += 1;
    }
    #[cfg(feature = "std-deps")]
    if std::env::var("RMAP_DBG_COLLIDERS").is_ok() {
        for (key, col, _, p1) in all_additions.iter() {
            if phase0_counts.get(&(key.clone(), col.name.clone())).map_or(false, |n| *n > 1) {
                std::eprintln!("[rmap-collide] table={} col={} src_cell={:?} subj={:?} val={:?} phase1={:?}",
                    key, col.name, col.source_cell, col.source_subject_role,
                    col.source_value_role, p1);
            }
        }
    }
    // Pass 2: resolve final names + fold.
    let mut taken: HashMap<String, HashSet<String>> = HashMap::new();
    let entity_columns: HashMap<String, (Vec<TableColumn>, HashSet<String>, Vec<String>)> =
        all_additions.into_iter()
            .fold(HashMap::new(), |mut map, (key, mut col, check_values, phase1)| {
                let collides = phase0_counts
                    .get(&(key.clone(), col.name.clone()))
                    .map_or(false, |n| *n > 1);
                if collides {
                    if let Some(p1) = phase1 {
                        col.name = p1;
                    }
                }
                // Deterministic suffix for anything STILL colliding in
                // this table (identical decorations / duplicate FTs).
                let taken_for_table = taken.entry(key.clone()).or_default();
                if taken_for_table.contains(&col.name) {
                    let base = col.name.clone();
                    let mut n = 2usize;
                    while taken_for_table.contains(&format!("{}_{}", base, n)) {
                        n += 1;
                    }
                    col.name = format!("{}_{}", base, n);
                }
                taken_for_table.insert(col.name.clone());
                let check = check_values.map(|vals| {
                    let quoted = vals.iter().map(|v| format!("'{}'", v)).collect::<Vec<_>>().join(", ");
                    format!("{} IN ({})", col.name, quoted)
                });
                let entry = map.entry(key).or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new()));
                entry.0.push(col);
                check.into_iter().for_each(|chk| entry.2.push(chk));
                map
            });

    // -- Step 2.5: External UC -> UNIQUE constraints -------------------
    // External UCs span multiple fact types. Each span contributes a column
    // to the target entity's table. Pure iter chain; last span with a
    // determinable target wins (matches prior semantics).
    let external_ucs: HashMap<String, Vec<Vec<String>>> = constraints.iter()
        .filter(|c| c.kind == "UC" && c.spans.len() >= 2)
        .filter(|c| c.spans.iter().map(|s| s.fact_type_id.as_str()).collect::<HashSet<_>>().len() >= 2)
        .filter_map(|c| {
            let (uc_cols, target_entity): (Vec<String>, Option<String>) = c.spans.iter()
                .filter_map(|span| {
                    let ft = fact_types.get(&span.fact_type_id)?;
                    let uc_role = ft.roles.iter().find(|r| r.role_index == span.role_index)?;
                    let col_name = column_name_for_target(&nouns, &uc_role.noun_name);
                    let target = ft.roles.iter()
                        .filter(|role| role.role_index != span.role_index)
                        .last()
                        .map(|role| resolve_entity(&role.noun_name));
                    Some((col_name, target))
                })
                .fold((Vec::new(), None), |(mut cols, target), (col, t)| {
                    cols.push(col);
                    (cols, t.or(target))
                });
            (uc_cols.len() >= 2).then_some(())
                .and(target_entity.map(|e| (e, uc_cols)))
        })
        .fold(HashMap::new(), |mut m, (entity, uc_cols)| {
            m.entry(entity).or_insert_with(Vec::new).push(uc_cols);
            m
        });

    // -- Emit entity tables ------------------------------------------
    // Pure iter chain: Filter(absorbed-subtypes) then Map(build-TableDef).
    // Absorbed (non-partitioned) subtypes live in the root table and are
    // filtered upstream, eliminating the control-flow `continue`.
    let entity_tables: Vec<TableDef> = entity_columns.iter()
        .filter(|(entity_name, _)| {
            let name: &String = *entity_name;
            !(subtype_names.contains(&name) && !partitioned_subtypes.contains(name))
        })
        .map(|(entity_name, (columns, _, checks))| {
            let table_name = to_snake(entity_name);
            let is_partitioned_subtype = partitioned_subtypes.contains(entity_name);

            // Feature #59: Compound reference scheme -> composite PK
            let compound_ref = ref_schemes.get(entity_name)
                .filter(|parts| parts.len() >= 2);

            let (all_cols, pk) = if let Some(ref_parts) = compound_ref {
                // Compound reference scheme: use ref parts as composite PK
                let pk_cols: Vec<String> = ref_parts.iter()
                    .map(|part| column_name_for_target(&nouns, part))
                    .collect();
                // No synthetic "id" column; columns are already present from functional absorption
                (columns.iter().cloned().collect::<Vec<_>>(), pk_cols)
            } else if is_partitioned_subtype {
                // Partitioned subtype: id column references parent table
                let parent_name = subtype_to_root.get(entity_name).unwrap();
                let id_col = TableColumn {
                    name: "id".to_string(),
                    col_type: "TEXT".to_string(),
                    nullable: false,
                    references: Some(to_snake(parent_name)),
                    ..Default::default()
                };
                let mut all = vec![id_col];
                all.extend(columns.iter().cloned());
                (all, vec!["id".to_string()])
            } else {
                // Normal entity: synthetic id PK
                let id_col = TableColumn {
                    name: "id".to_string(),
                    col_type: "TEXT".to_string(),
                    nullable: false,
                    references: None,
                    ..Default::default()
                };
                let mut all = vec![id_col];
                all.extend(columns.iter().cloned());
                (all, vec!["id".to_string()])
            };

            // Feature #57: Attach external UC as UNIQUE constraints
            let ext_ucs = external_ucs.get(entity_name).cloned();

            // Stage 1b: the absorbed reference-scheme value column (the
            // row projection defaults it to the id when no explicit
            // fact carries it — the id IS the refmode value).
            let ref_value_column = ref_schemes.get(entity_name)
                .filter(|schemes| schemes.len() == 1)
                .map(|schemes| column_name_for_target(&nouns, &schemes[0]))
                .filter(|col| all_cols.iter().any(|c| &c.name == col));

            TableDef {
                name: table_name,
                columns: all_cols,
                primary_key: pk,
                checks: if checks.is_empty() { None } else { Some(checks.clone()) },
                unique_constraints: ext_ucs,
                ref_value_column,
            }
        })
        .collect();
    emitted.extend(entity_tables.iter().map(|t| t.name.clone()));
    tables.extend(entity_tables);

    // -- Step 4: Independent entity -> single-column table ------------
    let referenced: HashSet<String> = tables.iter()
        .flat_map(|t| t.columns.iter().filter_map(|col| col.references.clone()))
        .collect();
    referenced.iter()
        .filter(|ref_table| !emitted.contains(*ref_table))
        .filter_map(|ref_table| {
            let (name, _) = nouns.iter().find(|(name, def)| to_snake(name) == *ref_table && def.object_type == "entity")?;
            (!(subtype_names.contains(name) && !partitioned_subtypes.contains(name))).then_some(())?;
            Some(ref_table.clone())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .for_each(|ref_table| {
            tables.push(TableDef {
                name: ref_table.clone(),
                columns: vec![TableColumn { name: "id".to_string(), col_type: "TEXT".to_string(), nullable: false, references: None, ..Default::default() }],
                primary_key: vec!["id".to_string()], checks: None, unique_constraints: None, ref_value_column: None,
            });
            emitted.insert(ref_table);
        });

    tables
}

// -- WASM export -----------------------------------------------------


/// Cell assignment: fact_type_id â†’ owning cell name (paper Eq. demux).
///
/// RMAP determines which entity table absorbs each fact type:
/// - Compound UC (M:N, ternary+) â†’ own table/cell
/// - Single-role UC (functional) â†’ absorbed into the UC role's entity cell
/// - Unary facts â†’ entity cell
///
/// The returned map enables event demultiplexing:
///   E_n = Filter(eq âˆ˜ [RMAP, nÌ„]) : E
/// rmap_cell_map from Object state â€” no Domain round-trip.
pub fn rmap_cell_map_from_state(state: &crate::ast::Object) -> HashMap<String, String> {
    rmap_cell_map(state)
}

pub fn rmap_cell_map(state: &crate::ast::Object) -> HashMap<String, String> {
    use crate::ast::{fetch_cell_seq, binding};
    use crate::types::*;
    let mut nouns: HashMap<String, NounDef> = HashMap::new();
    let mut subtypes: HashMap<String, String> = HashMap::new();
    if let Some(ns) = fetch_cell_seq("Noun", state).as_seq() {
        for f in ns.iter() {
            let name = binding(f, "name").unwrap_or("").to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
            if let Some(st) = binding(f, "superType") { subtypes.insert(name.clone(), st.to_string()); }
        }
    }
    let role_cell = fetch_cell_seq("Role", state);
    let fact_types: HashMap<String, FactTypeDef> = fetch_cell_seq("FactType", state).as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            let roles: Vec<RoleDef> = role_cell.as_seq()
                .map(|rs| rs.iter()
                    .filter(|r| binding(r, "factType") == Some(&id))
                    .map(|r| RoleDef {
                        noun_name: binding(r, "nounName").unwrap_or("").to_string(),
                        role_index: binding(r, "position").and_then(|v| v.parse().ok()).unwrap_or(0),
                    }).collect())
                .unwrap_or_default();
            Some((id, FactTypeDef { schema_id: String::new(), reading, readings: vec![], roles }))
        }).collect())
        .unwrap_or_default();
    let constraints: Vec<ConstraintDef> = fetch_cell_seq("Constraint", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = crate::compile::decode_constraint_spans(&get);
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: get("setComparisonArgumentLength")
                    .and_then(|s| s.parse().ok()),
                clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
                predicate: None,
            }
        }).collect())
        .unwrap_or_default();
    let mut map = HashMap::new();
    let noun_name_set: HashSet<String> = nouns.keys().cloned().collect();

    // Index UCs by fact type (same as RMAP step classification).
    // The parser no longer emits the single-role-UC span mirror
    // (`enrich_constraints_with_spans` used to push span0 == span1), so a
    // single-role UC already presents one role. The `dedup()` below is
    // kept only as cheap belt-and-suspenders against any future
    // re-introduction of an identical-role duplicate that the
    // `uc.len() >= 2` compound test would otherwise misread.
    let ucs_by_ft: HashMap<String, Vec<Vec<usize>>> = constraints.iter()
        .filter(|c| c.kind == "UC")
        .fold(HashMap::new(), |mut acc, c| {
            c.spans.first().map(|s| s.fact_type_id.clone()).into_iter().for_each(|ft_id| {
                let mut roles: Vec<usize> = c.spans.iter()
                    .filter(|s| s.fact_type_id == ft_id)
                    .map(|s| s.role_index)
                    .collect();
                roles.sort_unstable();
                roles.dedup();
                acc.entry(ft_id).or_default().push(roles);
            });
            acc
        });

    // Subtype resolution. Backus's `while (p f)` combining form lifted
    // into Rust as iter::successors — walk the parent chain until fixed.
    // 100-step bound is a belt-and-braces cycle defence; the checker
    // (#199) rejects subtype cycles before we get here.
    let parent_of: HashMap<&str, &str> = subtypes.iter()
        .map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let resolve_root = |name: &str| -> String {
        core::iter::successors(
            Some(name.to_string()),
            |cur| parent_of.get(cur.as_str()).map(|p| p.to_string()),
        ).take(100).last().unwrap_or_default()
    };

    for (ft_id, ft) in &fact_types {
        if ft.roles.is_empty() { continue; }

        // Unary: entity cell
        if ft.roles.len() == 1 {
            let entity = resolve_root(&ft.roles[0].noun_name);
            map.insert(ft_id.clone(), to_snake(&entity));
            continue;
        }

        let ucs = ucs_by_ft.get(ft_id).cloned().unwrap_or_default();
        let has_compound = ucs.iter().any(|uc| uc.len() >= 2);
        let single_ucs: Vec<usize> = ucs.iter()
            .filter(|uc| uc.len() == 1).map(|uc| uc[0]).collect();

        if has_compound {
            // Compound UC â†’ own cell (M:N table)
            let cell = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
            map.insert(ft_id.clone(), cell);
        } else if !single_ucs.is_empty() {
            // Functional â†’ absorbed into the entity cell of the identifying role.
            // The UC constrains the dependent role; the other role identifies the row.
            // E.g. UC on Customer in "Order was placed by Customer" means
            // each Order has one Customer, so the fact is absorbed into Order's cell.
            let id_role = ft.roles.iter()
                .find(|r| !single_ucs.contains(&r.role_index))
                .unwrap_or(&ft.roles[0]);
            let entity = resolve_root(&id_role.noun_name);
            map.insert(ft_id.clone(), to_snake(&entity));
        } else {
            // No UC â†’ own cell (junction table)
            let cell = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
            map.insert(ft_id.clone(), cell);
        }
    }
    map
}

/// Inverse of `EntityCellRouter::route_fact` (task-962): given an absorbed
/// binary fact-type id that has no data cell of its own, project the
/// absorbing entity cell's facts back into elementary-fact tuples
/// `<<subjectRole, id>, <valueRole, value>>` so query / get / derivations
/// over the 5NF population P see the RMAP-absorbed column (the `up-FILE`
/// reconstitution of eq:pop). Presence-driven: a tuple is emitted only
/// where the value binding is actually present. Returns `None` for
/// self / junction cells or non-binary fact types.
///
/// Handles the metamodel entity-cell regime: `translate_nouns` stores the
/// subject under `name` and each functional property under
/// `lower_camel(valueRole)` (e.g. Noun's `objectType`, `referenceScheme`).
/// Runtime per-entity cells (`<Noun>:<id>`) are a follow-up.
pub fn reconstitute_absorbed_ft(
    state: &crate::ast::Object,
    ft_id: &str,
) -> Option<crate::ast::Object> {
    use crate::ast::{fetch_cell_seq, binding, fact_from_pairs, Object};

    // Absorbed FTs only: rmap routes them into an entity cell whose snake
    // name differs from the FT's own snake (self / junction cells route to
    // themselves and keep a real data cell — nothing to reconstitute).
    // FT roles, ordered by position.
    let role_cell = fetch_cell_seq("Role", state);
    let mut roles: Vec<(usize, String)> = role_cell.as_seq()
        .map(|rs| rs.iter()
            .filter(|r| binding(r, "factType") == Some(ft_id))
            .filter_map(|r| Some((
                binding(r, "position")?.parse::<usize>().ok()?,
                binding(r, "nounName")?.to_string(),
            )))
            .collect())
        .unwrap_or_default();
    roles.sort_by_key(|(p, _)| *p);
    if roles.len() != 2 { return None; }

    // task-962: classify absorption by ROLE OBJECT-TYPE, local to this
    // up-FILE projection (NOT via rmap_cell_map, whose classification drives
    // routing -- changing it there regresses MC/deontic/ring tests). A binary
    // FT is RMAP-absorbed into an entity cell (no own data cell) precisely when
    // exactly one role is VALUE-typed -- the dependent value, e.g. `Object
    // Type` in `Noun has Object Type` -- and the other is entity-typed (the
    // subject). Both-entity FTs are NOT folded this way: entity rings
    // (`Task blocks Task`) and entity-valued functional FTs (`Order was placed
    // by Customer`). Reconstituting those fabricates tuples that break
    // MC/deontic/ring checks, so return None. (A single-role-UC gate was
    // over-broad: it fired for entity-valued functional FTs too.)
    let value_nouns: HashSet<String> = fetch_cell_seq("Noun", state).as_seq()
        .map(|ns| ns.iter()
            .filter(|f| binding(f, "objectType") == Some("value"))
            .filter_map(|f| binding(f, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();
    let value_roles: Vec<&(usize, String)> =
        roles.iter().filter(|(_, n)| value_nouns.contains(n)).collect();
    let entity_roles: Vec<&(usize, String)> =
        roles.iter().filter(|(_, n)| !value_nouns.contains(n)).collect();
    if value_roles.len() != 1 || entity_roles.len() != 1 { return None; }
    let value_role = value_roles[0].1.clone();
    let subject_role = entity_roles[0].1.clone();

    // The absorbing cell is the subject entity type's registry cell, named
    // for the subject role's noun (metamodel regime — e.g. the `Noun` cell).
    // Project: subject under `name`, value under lower_camel(valueRole).
    // Presence-driven; sorted for replay determinism (D_n is a set).
    let value_key = crate::naming::lower_camel(&value_role);
    let cell = fetch_cell_seq(&subject_role, state);
    let mut rows: Vec<(String, String)> = cell.as_seq()
        .map(|fs| fs.iter()
            .filter_map(|f| Some((
                binding(f, "name")?.to_string(),
                binding(f, &value_key)?.to_string(),
            )))
            .collect())
        .unwrap_or_default();
    rows.sort();
    let facts: Vec<Object> = rows.into_iter()
        .map(|(subj, val)| fact_from_pairs(&[
            (subject_role.as_str(), subj.as_str()),
            (value_role.as_str(), val.as_str()),
        ]))
        .collect();
    // task-962: an empty projection means the FT is NOT actually folded into
    // the subject cell (e.g. a value-typed FT that keeps its OWN data cell) --
    // return None so callers fall back to the FT's own cell rather than
    // SHADOWING it with an empty reconstitution (which masked an SS subset
    // violation: `resolve_view` used the empty result instead of the real
    // ft_has_note cell). Folded metamodel FTs project a non-empty set.
    if facts.is_empty() { return None; }
    Some(Object::seq(facts))
}

// ── #770 — per-entity cell routing (paper §196 + §462 eq:cellfold) ──
//
// Given a fact stored in an FT cell, decide which per-entity cell
// `<Noun>:<entity-id>` should absorb its contribution and pull the
// (field, value) pair to merge into the entity's 3NF row.
//
// Routing decision is rmap-driven: `rmap_cell_map` already classifies
// every FT into either a per-entity table (snake_case) or its own
// junction cell (the original FT id). When the routing target is an
// entity table, the fact's first pair `(noun, entity_id)` identifies
// the entity instance and the remaining pair(s) become the row's
// (field, value) columns.
//
// Returns None when the FT routes into its own junction cell (compound
// UC / M:N / no UC) or when the fact's binding shape can't be projected.

/// Per-entity cell routing for a single fact contribution.
///
/// `cell_name` is `<Noun>:<entity-id>` — the per-entity cell that
/// absorbs this fact's contribution. `field_key` and `field_value`
/// are the (column, value) pair to merge into the entity's 3NF row
/// (e.g. `total = "100"` from `Order_has_total` for `Order:ord-1`).
#[derive(Debug, Clone)]
pub struct EntityCellRouting {
    pub cell_name: String,
    pub noun_name: String,
    pub entity_id: String,
    pub field_key: String,
    pub field_value: String,
}

/// Snapshot-level cache of state-derived data needed for entity-cell
/// routing. Build once per snapshot, route many facts cheaply.
///
/// `entity_cell_for_fact` and `entity_id_field_name` previously
/// recomputed `rmap_cell_map(state)` and walked the Noun cell on
/// every call — fine when called once per apply, regression when
/// called per fact across a multi-fact delta (see #771). The router
/// pulls those state-keyed reads up to one pass and caches the three
/// projections every per-fact call needs:
///
///   - shard_map      : `ft_id    -> snake_case absorbing target`
///   - noun_by_snake  : `to_snake(noun_name) -> noun_name` (PascalCase)
///   - id_field_by_noun : `noun_name -> referenceScheme field name`
///
/// Cache invalidation matches snapshot lifetime: when `merge_delta`
/// produces a new snapshot, build a new router. The snapshot is the
/// state, the router is its read view.
#[derive(Debug, Clone)]
pub struct EntityCellRouter {
    shard_map: HashMap<String, String>,
    noun_by_snake: HashMap<String, String>,
    id_field_by_noun: HashMap<String, String>,
}

impl EntityCellRouter {
    /// Build the router from a snapshot. Pre-computes the rmap shard
    /// map plus snake-case and reference-scheme indices over the Noun
    /// cell. One state pass; per-fact routing thereafter is HashMap
    /// lookups.
    pub fn new(state: &crate::ast::Object) -> Self {
        use crate::ast::{fetch_cell_seq, binding};

        let shard_map = rmap_cell_map(state);

        let mut noun_by_snake: HashMap<String, String> = HashMap::new();
        let mut id_field_by_noun: HashMap<String, String> = HashMap::new();
        if let Some(ns) = fetch_cell_seq("Noun", state).as_seq() {
            for n in ns.iter() {
                let Some(name) = binding(n, "name") else { continue };
                noun_by_snake.insert(to_snake(name), name.to_string());
                let scheme = binding(n, "referenceScheme")
                    .unwrap_or("id").to_string();
                id_field_by_noun.insert(name.to_string(), scheme);
            }
        }

        EntityCellRouter { shard_map, noun_by_snake, id_field_by_noun }
    }

    /// Route a single fact (under `ft_id`) to its absorbing entity
    /// cell, or `None` for self-cells / non-projectable shapes.
    ///
    /// Mirrors the per-call `entity_cell_for_fact` semantics exactly,
    /// but reads pre-computed state projections instead of walking
    /// the Noun cell and recomputing `rmap_cell_map` per call.
    pub fn route_fact(
        &self,
        ft_id: &str,
        fact: &crate::ast::Object,
    ) -> Option<EntityCellRouting> {
        use crate::ast::binding;

        let target = self.shard_map.get(ft_id)?;
        let ft_snake = to_snake(ft_id);
        if *target == ft_snake { return None; }

        let noun_name = self.noun_by_snake.get(target)?.clone();
        let entity_id = binding(fact, &noun_name)?.to_string();

        let pair = fact.as_seq()?.iter().find_map(|p| {
            let items = p.as_seq()?;
            if items.len() != 2 { return None; }
            let key = items[0].as_atom()?;
            if key == noun_name { return None; }
            let val = items[1].as_atom()?;
            Some((key.to_string(), val.to_string()))
        })?;

        Some(EntityCellRouting {
            cell_name: format!("{}:{}", noun_name, entity_id),
            noun_name,
            entity_id,
            field_key: pair.0,
            field_value: pair.1,
        })
    }

    /// Reference-scheme field name for a noun. Defaults to `"id"`.
    pub fn id_field_for(&self, noun_name: &str) -> &str {
        self.id_field_by_noun.get(noun_name).map(|s| s.as_str()).unwrap_or("id")
    }

    /// Borrowed view of the rmap shard map (FT id -> snake target).
    /// Exposed so callers that already hold a router can drive the
    /// row-projection loop without rebuilding the map.
    pub fn shard_map(&self) -> &HashMap<String, String> {
        &self.shard_map
    }
}

/// Compute per-entity cell routing for a fact under FT `ft_id`.
///
/// Consults `rmap_cell_map` to determine whether the FT absorbs into
/// an entity table. When it does, the absorbing entity's PascalCase
/// noun name is recovered from the Noun cell (matching by snake_case
/// equivalence to the rmap-emitted target), the entity_id is pulled
/// from the fact's binding for that noun, and the remaining pair(s)
/// supply the (field, value) contribution to the 3NF row.
///
/// Returns None when the FT routes to its own cell (compound /
/// junction) or when the fact bindings don't carry the expected
/// shape (defensive — the apply-path always emits the canonical
/// `<<noun, entity_id>, <field, value>>` shape).
///
/// One-shot helper for callers that route a single fact. For
/// multi-fact callers (e.g. `augment_delta_with_entity_cells`),
/// build an `EntityCellRouter` once and use `route_fact` per fact —
/// this function is implemented in those terms.
pub fn entity_cell_for_fact(
    state: &crate::ast::Object,
    ft_id: &str,
    fact: &crate::ast::Object,
) -> Option<EntityCellRouting> {
    EntityCellRouter::new(state).route_fact(ft_id, fact)
}

/// Reference-scheme field name for a noun (e.g. `"id"` for `Order`).
/// Reads the Noun cell's `referenceScheme` binding (set by the parser
/// from `Order(.id) is an entity type.`); defaults to `"id"`. Used by
/// the apply path to seed the 3NF row's primary-key column when
/// materializing per-entity cells.
///
/// One-shot helper. Multi-noun callers should build an
/// `EntityCellRouter` and use `id_field_for` instead.
pub fn entity_id_field_name(state: &crate::ast::Object, noun_name: &str) -> String {
    EntityCellRouter::new(state).id_field_for(noun_name).to_string()
}

// -- Population projection plan (rmap-3nf-tables Stage 2) -------------

/// SQLite identifier quoting WITH embedded-quote escaping. Junk
/// prose-minted FT names can carry literal `"` — a naive `"{name}"`
/// breaks out of the identifier; `""` is the SQL-standard escape.
pub fn qid(raw: &str) -> String {
    alloc::format!("\"{}\"", raw.replace('"', "\"\""))
}

/// CREATE TABLE from a `TableDef` — the SINGLE source of the 3NF
/// table shape, used by BOTH the persist path (cli/entry.rs apply_ddl
/// creates plan tables from this after its DROP pass, so the persisted
/// shape can never drift from the projection plan) and the sql verb's
/// :memory: materialization. TEXT affinity everywhere (cell values
/// are atoms), NOT NULL per column nullability, REFERENCES per FK,
/// PRIMARY KEY / UNIQUE / CHECK groups verbatim.
pub fn create_table_sql(t: &TableDef) -> String {
    let mut parts: Vec<String> = t.columns.iter().map(|c| {
        let mut s = alloc::format!("{} TEXT", qid(&c.name));
        if !c.nullable { s.push_str(" NOT NULL"); }
        if let Some(parent) = &c.references {
            s.push_str(&alloc::format!(" REFERENCES {}", qid(parent)));
        }
        s
    }).collect();
    if !t.primary_key.is_empty() {
        let cols = t.primary_key.iter()
            .map(|c| qid(c)).collect::<Vec<_>>().join(", ");
        parts.push(alloc::format!("PRIMARY KEY ({})", cols));
    }
    if let Some(ucs) = &t.unique_constraints {
        for uc in ucs {
            let cols = uc.iter()
                .map(|c| qid(c)).collect::<Vec<_>>().join(", ");
            parts.push(alloc::format!("UNIQUE ({})", cols));
        }
    }
    if let Some(checks) = &t.checks {
        for chk in checks {
            parts.push(alloc::format!("CHECK ({})", chk));
        }
    }
    alloc::format!("CREATE TABLE IF NOT EXISTS {} ({});", qid(&t.name), parts.join(", "))
}

/// One projected row: final column name → value.
pub type ProjectedRow = alloc::collections::BTreeMap<String, String>;

/// The PURE population-projection plan — Phases 1–3 of the 3NF row
/// projection (collect from cells via column provenance, derive
/// missing FK parents to fixpoint, Kahn-order parents-first). Shared
/// by the persist path (cli/entry.rs Phase 4 executes DELETE+INSERT
/// on the app db) and the `sql` verb (which materializes the plan
/// into a per-call `:memory:` database). One plan = "query the 3NF
/// substrate" means the same thing everywhere.
pub struct ProjectionPlan {
    pub tables: Vec<TableDef>,
    /// table name → insertion-ready rows.
    pub rows: HashMap<String, Vec<ProjectedRow>>,
    /// Table names parents-before-children (Kahn over the references
    /// graph; self-references pass; cycles append name-sorted).
    pub order: Vec<String>,
}

pub fn projection_plan(state: &crate::ast::Object) -> ProjectionPlan {
    use crate::ast;
    let tables = rmap(state);
    // Borrow-free lookups so `tables` can move into the returned plan:
    // name membership + the refscheme column per table.
    let table_names: HashSet<String> = tables.iter().map(|t| t.name.clone()).collect();
    let ref_value_by_name: HashMap<String, Option<String>> =
        tables.iter().map(|t| (t.name.clone(), t.ref_value_column.clone())).collect();

    // task-924 parity: a view (derived) fact type's population IS its
    // derivation's output, not the stored cell — but ONLY when the
    // stored cell is EMPTY (eagerly-materialized Stored cells that
    // also carry a `view:` def keep their stored truth).
    let effective_cell = |cell: &str| -> crate::ast::Object {
        let stored = ast::fetch_or_phi(cell, state);
        if ast::cell_fact_count(&stored) == 0 {
            if let Some(resolved) = ast::resolve_view(cell, state, state) {
                return resolved;
            }
        }
        stored
    };

    // ── Phase 1: collect every table's rows ────────────────────────
    let mut collected: HashMap<String, Vec<ProjectedRow>> = HashMap::new();
    for table in &tables {
        let is_entity_table = table.primary_key == ["id".to_string()];
        let mut out: Vec<ProjectedRow> = Vec::new();
        if is_entity_table {
            let mut rows: HashMap<String, ProjectedRow> = HashMap::new();
            for col in &table.columns {
                let (Some(cell), Some(subj), Some(val)) =
                    (&col.source_cell, &col.source_subject_role, &col.source_value_role)
                else { continue };
                let contents = effective_cell(cell);
                for fact in ast::cell_facts_iter(&contents) {
                    if let (Some(id), Some(v)) = (ast::binding(fact, subj), ast::binding(fact, val)) {
                        rows.entry(id.to_string()).or_default()
                            .insert(col.name.clone(), v.to_string());
                    }
                }
            }
            // Deterministic row order (HashMap iteration is not).
            let mut keyed: Vec<(String, ProjectedRow)> = rows.into_iter().collect();
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            for (id, mut cols) in keyed {
                // Refscheme defaulting: the synthetic id IS the
                // reference-mode value when no explicit fact carries it.
                if let Some(ref_col) = &table.ref_value_column {
                    cols.entry(ref_col.clone()).or_insert_with(|| id.clone());
                }
                cols.insert("id".to_string(), id);
                out.push(cols);
            }
        } else {
            // Junction/compound: one row per fact, positional
            // extraction (same-noun rings land both roles), by-name
            // fallback on arity mismatch.
            let Some(cell) = table.columns.iter()
                .find_map(|c| c.source_cell.clone()) else { continue };
            let proj_cols: Vec<&TableColumn> = table.columns.iter()
                .filter(|c| c.source_cell.is_some())
                .collect();
            let contents = effective_cell(&cell);
            for fact in ast::cell_facts_iter(&contents) {
                let pairs: Vec<(String, String)> = fact.as_seq()
                    .map(|items| items.iter().filter_map(|p| {
                        let kv = p.as_seq()?;
                        if kv.len() != 2 { return None; }
                        Some((kv[0].as_atom()?.to_string(), kv[1].as_atom()?.to_string()))
                    }).collect())
                    .unwrap_or_default();
                let mut row: ProjectedRow = ProjectedRow::new();
                if pairs.len() == proj_cols.len() {
                    for (col, (_, v)) in proj_cols.iter().zip(pairs.iter()) {
                        row.insert(col.name.clone(), v.clone());
                    }
                } else {
                    for col in proj_cols.iter() {
                        let Some(role) = &col.source_value_role else { continue };
                        if let Some((_, v)) = pairs.iter().find(|(k, _)| k == role) {
                            row.insert(col.name.clone(), v.clone());
                        }
                    }
                }
                if !row.is_empty() { out.push(row); }
            }
        }
        collected.insert(table.name.clone(), out);
    }

    // ── Phase 2: derive PARENT rows from FK values, to FIXPOINT ────
    // A parent row added here can itself carry a subtype id-FK that
    // needs ITS parent (status.id REFERENCES function via the
    // Status < Resource < Noun < Function chain). Terminates: each
    // round only adds ids not yet present, over a finite id universe.
    loop {
        let mut extra: HashMap<String, HashSet<String>> = HashMap::new();
        for table in &tables {
            let Some(rows) = collected.get(&table.name) else { continue };
            for col in &table.columns {
                let Some(parent) = &col.references else { continue };
                if !table_names.contains(parent) { continue; }
                for row in rows {
                    if let Some(v) = row.get(&col.name) {
                        extra.entry(parent.clone()).or_default().insert(v.clone());
                    }
                }
            }
        }
        // Deterministic fill order (HashMap iteration is not).
        let mut extra_sorted: Vec<(String, HashSet<String>)> = extra.into_iter().collect();
        extra_sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut grew = false;
        for (parent, ids) in extra_sorted {
            let ref_col = ref_value_by_name.get(&parent).cloned().flatten();
            let rows = collected.entry(parent.clone()).or_default();
            let existing: HashSet<String> = rows.iter()
                .filter_map(|r| r.get("id").cloned())
                .collect();
            let mut ids_sorted: Vec<String> = ids.into_iter().collect();
            ids_sorted.sort();
            for id in ids_sorted {
                if existing.contains(&id) { continue; }
                let mut row: ProjectedRow = ProjectedRow::new();
                if let Some(rc) = &ref_col {
                    row.insert(rc.clone(), id.clone());
                }
                row.insert("id".to_string(), id);
                rows.push(row);
                grew = true;
            }
        }
        if !grew { break; }
    }

    // ── Phase 3: dependency order (parents before children) ────────
    let mut order: Vec<String> = Vec::new();
    let mut placed: HashSet<String> = HashSet::new();
    let mut remaining: Vec<&TableDef> = tables.iter().collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        let (ready, rest): (Vec<_>, Vec<_>) = remaining.into_iter().partition(|t| {
            t.columns.iter()
                .filter_map(|c| c.references.as_ref())
                .all(|p| p == &t.name || placed.contains(p) || !table_names.contains(p))
        });
        let mut ready_sorted = ready;
        ready_sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for t in &ready_sorted { placed.insert(t.name.clone()); }
        order.extend(ready_sorted.iter().map(|t| t.name.clone()));
        remaining = rest;
        if remaining.len() == before {
            // Cycle: append the rest in name order (deterministic).
            let mut rest_sorted = remaining;
            rest_sorted.sort_by(|a, b| a.name.cmp(&b.name));
            order.extend(rest_sorted.iter().map(|t| t.name.clone()));
            break;
        }
    }

    ProjectionPlan { tables, rows: collected, order }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, Object, fact_from_pairs};
    use crate::types::*;

    /// rmap-3nf-tables, NORMA two-phase column naming: two functional
    /// FTs absorbing the SAME target into one table (`is from Status` /
    /// `is to Status`) must decorate BOTH colliding columns with their
    /// predicate text — from_stage_id + to_stage_id, never a bare
    /// duplicate and never one reading decorating both columns.
    #[test]
    fn colliding_absorbed_columns_decorate_with_their_own_predicate_text() {
        let src = "\
            Hop(.hid) is an entity type.\n\
            Stage(.sid) is an entity type.\n\
            hid is a value type.\n\
            sid is a value type.\n\
            \n\
            ## Fact Types\n\
            Hop is from Stage.\n\
              Each Hop is from exactly one Stage.\n\
            Hop is to Stage.\n\
              Each Hop is to exactly one Stage.\n\
        ";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let tables = rmap(&state);
        let t2 = tables.iter().find(|t| t.name == "hop")
            .expect("hop entity table must exist");
        let mut cols: Vec<&str> = t2.columns.iter().map(|c| c.name.as_str()).collect();
        cols.sort();
        assert!(cols.contains(&"from_stage_id"),
            "the is-from column must decorate with ITS predicate (from); got {:?}", cols);
        assert!(cols.contains(&"to_stage_id"),
            "the is-to column must decorate with ITS predicate (to); got {:?}", cols);
        assert!(!cols.iter().any(|c| c.ends_with("_id_2")),
            "no numeric-suffix fallback when predicate decoration disambiguates; got {:?}", cols);
    }

    /// rmap-3nf-tables (iv), ring fact types: a UC-less m:n FT carries
    /// Halpin's IMPLICIT whole-tuple spanning UC and must still emit a
    /// junction table (`Hop blocks Hop` → hop_blocks_hop). Repeated
    /// nouns disambiguate POSITIONALLY (hop_id, hop_id_2), both columns
    /// FK the same parent, and the PK spans both under the same names.
    #[test]
    fn uc_less_ring_fact_type_emits_junction_with_positional_columns() {
        let src = "\
            Hop(.hid) is an entity type.\n\
            hid is a value type.\n\
            \n\
            ## Fact Types\n\
            Hop blocks Hop.\n\
        ";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let tables = rmap(&state);
        let t = tables.iter().find(|t| t.name == "hop_blocks_hop")
            .expect("UC-less ring FT must emit a junction table (implicit spanning UC)");
        let cols: Vec<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, vec!["hop_id", "hop_id_2"],
            "repeated noun must disambiguate positionally; got {:?}", cols);
        assert!(t.columns.iter().all(|c| c.references.as_deref() == Some("hop")),
            "both ring columns must FK the hop parent");
        assert_eq!(t.primary_key, vec!["hop_id", "hop_id_2"],
            "implicit spanning UC -> PK over ALL role columns, names aligned");
        assert!(t.columns.iter().all(|c| c.source_cell.as_deref() == Some("Hop_blocks_Hop")),
            "junction provenance must point at the ring cell for projection");
    }

    /// rmap-3nf-tables (iii): a disjunctive ("For each Stage, … or …")
    /// mandatory constraint is the inclusive-or — it makes NO single
    /// role total, so it must not mint NOT NULL on any absorbed
    /// column. The live bug attached the unresolvable disjunction to
    /// an arbitrary FT sharing the entity's role
    /// (Verb_is_performed_in_Status), and status.verb_id NOT NULL
    /// zeroed the status/resource projection family.
    #[test]
    fn disjunctive_mc_does_not_mint_not_null_on_unrelated_absorption() {
        let src = "\
            Hop(.hid) is an entity type.\n\
            Stage(.sid) is an entity type.\n\
            Gate(.gid) is an entity type.\n\
            hid is a value type.\n\
            sid is a value type.\n\
            gid is a value type.\n\
            \n\
            ## Fact Types\n\
            Hop is from Stage.\n\
              Each Hop is from exactly one Stage.\n\
            Hop is to Stage.\n\
              Each Hop is to exactly one Stage.\n\
            Gate is performed in Stage.\n\
              For each Stage, at most one Gate is performed in that Stage.\n\
            \n\
            ## Constraints\n\
            For each Stage, some Hop is from that Stage or some Hop is to that Stage.\n\
        ";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let tables = rmap(&state);
        let stage = tables.iter().find(|t| t.name == "stage")
            .expect("stage entity table must exist");
        let gate_col = stage.columns.iter().find(|c| c.name == "gate_id")
            .unwrap_or_else(|| panic!(
                "per-Stage functional Gate must absorb as stage.gate_id; got {:?}",
                stage.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()));
        assert!(gate_col.nullable,
            "the disjunctive MC constrains Hop-participation, not Gate \
             absorption — gate_id must stay nullable");
    }

    /// RED (bug repro through the PARSER): a single-role functional
    /// value attribute MUST absorb as a COLUMN on the entity's table
    /// (Halpin §10.3 rule 2 / Fig 10.11 horse example: `Horse(horseName,
    /// sex, weight)`), NOT become a junction table.
    ///
    /// The rmap/sql unit tests above all hand-build the Constraint cell
    /// with a single span, bypassing the parser. The parser's
    /// `enrich_constraints_with_spans` MIRRORS span0 into span1 for
    /// single-role UC/MC (the "legacy quirk"), so a real parse yields a
    /// UC with `spans == [(ft,0),(ft,0)]`. rmap then classifies any UC
    /// with `spans.len() >= 2` as COMPOUND and routes the fact to its
    /// own (junction) table. This test goes through the parser to pin
    /// the canonical column-absorption result.
    #[test]
    fn parser_path_single_role_uc_value_absorbs_as_column_not_junction() {
        let src = "\
            Product(.code) is an entity type.\n\
            Quantity is a value type.\n\
            \n\
            ## Fact Types\n\
            Product has Quantity.\n\
            \n\
            ## Constraints\n\
            Each Product has at most one Quantity.\n\
        ";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let tables = rmap(&state);
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();

        // No junction table for an absorbed functional value attribute.
        assert!(!names.iter().any(|n| n.contains("product_has_quantity")
                || (n.contains("product") && n.contains("quantity"))),
            "single-role UC value attribute must NOT produce a junction table; got tables {:?}",
            names);

        // Product table carries an absorbed `quantity` column.
        let product = tables.iter().find(|t| t.name == "product")
            .unwrap_or_else(|| panic!("expected a `product` table; got {:?}", names));
        assert!(product.columns.iter().any(|c| c.name == "quantity"),
            "Product must absorb a `quantity` column (Halpin §10.3 rule 2); got columns {:?}",
            product.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
        // Value type => no FK reference.
        let q = product.columns.iter().find(|c| c.name == "quantity").unwrap();
        assert!(q.references.is_none(), "value attribute column must not be a FK");
    }

    /// Build Object state for rmap input. Emits Noun, FactType, Role,
    /// Constraint cells directly — no Domain intermediate (#211).
    fn make_state(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
    ) -> ast::Object {
        let mut cells: HashMap<String, Vec<Object>> = HashMap::new();
        for (name, obj_type) in &nouns {
            let ref_scheme = (*obj_type == "entity").then(|| "id");
            let mut pairs: Vec<(&str, &str)> = vec![
                ("name", *name), ("objectType", *obj_type), ("worldAssumption", "closed"),
            ];
            if let Some(rs) = ref_scheme { pairs.push(("referenceScheme", rs)); }
            cells.entry("Noun".into()).or_default().push(fact_from_pairs(&pairs));
        }
        for (id, reading, roles) in &fact_types {
            let arity = roles.len().to_string();
            cells.entry("FactType".into()).or_default().push(fact_from_pairs(&[
                ("id", *id), ("reading", *reading), ("arity", arity.as_str()),
            ]));
            for (name, idx) in roles {
                let pos = idx.to_string();
                cells.entry("Role".into()).or_default().push(fact_from_pairs(&[
                    ("factType", *id), ("nounName", *name), ("position", pos.as_str()),
                ]));
            }
        }
        for (i, (kind, spans)) in constraints.iter().enumerate() {
            let cdef = ConstraintDef {
                id: format!("c_{}", i),
                kind: (*kind).to_string(),
                modality: "Alethic".to_string(),
                text: String::new(),
                spans: spans.iter().map(|(ft_id, role_idx)| SpanDef {
                    fact_type_id: ft_id.to_string(),
                    role_index: *role_idx,
                    subset_autofill: None,
                }).collect(),
                ..Default::default()
            };
            cells.entry("Constraint".into()).or_default()
                .push(crate::parse_forml2::constraint_to_fact_test(&cdef));
        }
        Object::Map(cells.into_iter().map(|(k, v)| (k, Object::Seq(v.into()))).collect::<hashbrown::HashMap<_, _>>().into())
    }

    #[test]
    fn reconstitutes_absorbed_metamodel_object_type() {
        // task-962: `Noun has Object Type` is functional, so RMAP absorbs
        // Object Type into the Noun cell (UC on the dependent role, like
        // "Order was placed by Customer"). reconstitute_absorbed_ft must
        // project it back out of the Noun registry into elementary
        // <<Noun,id>,<Object Type,val>> tuples (the up-FILE direction).
        let state = make_state(
            vec![("Task", "entity"), ("Order", "entity"), ("Object Type", "value")],
            vec![("Noun_has_Object_Type", "Noun has Object Type",
                  vec![("Noun", 0), ("Object Type", 1)])],
            vec![("UC", vec![("Noun_has_Object_Type", 1)])],
        );
        let out = reconstitute_absorbed_ft(&state, "Noun_has_Object_Type")
            .expect("absorbed FT should reconstitute");
        let facts = out.as_seq().expect("seq of tuples");
        // One tuple per Noun (every Noun has an Object Type), sorted by
        // subject: Object Type (value), Order (entity), Task (entity).
        assert_eq!(facts.len(), 3);
        assert_eq!(ast::binding(&facts[0], "Noun"), Some("Object Type"));
        assert_eq!(ast::binding(&facts[0], "Object Type"), Some("value"));
        assert_eq!(ast::binding(&facts[1], "Noun"), Some("Order"));
        assert_eq!(ast::binding(&facts[1], "Object Type"), Some("entity"));
        assert_eq!(ast::binding(&facts[2], "Noun"), Some("Task"));
        assert_eq!(ast::binding(&facts[2], "Object Type"), Some("entity"));
    }

    #[test]
    fn reconstitute_skips_entity_valued_functional_ft() {
        // task-962: an entity-valued functional FT (both roles entity, e.g.
        // `Order was placed by Customer`) is NOT folded into an entity cell the
        // way a value-typed property is; reconstituting it would fabricate
        // tuples that break MC/deontic/ring checks. The value-typed-value-role
        // gate must return None (no value-typed role).
        let state = make_state(
            vec![("Order", "entity"), ("Customer", "entity")],
            vec![("ft1", "Order was placed by Customer",
                  vec![("Order", 0), ("Customer", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        assert!(reconstitute_absorbed_ft(&state, "ft1").is_none(),
            "entity-valued functional FT must not reconstitute");
    }

    #[test]
    fn functional_binary_produces_entity_table() {
        // Person has Name (UC on Person role -> Name absorbed into Person table)
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])], // UC on Person -> each Person has at most one Name
        );
        let tables = rmap(&state);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "person");
        assert_eq!(tables[0].columns.len(), 2); // id + name
        assert_eq!(tables[0].columns[1].name, "name");
        assert!(tables[0].columns[1].references.is_none()); // value type, no FK
    }

    #[test]
    fn compound_uc_produces_junction_table() {
        // Person teaches Course (UC spanning both roles -> junction table)
        let state = make_state(
            vec![("Person", "entity"), ("Course", "entity")],
            vec![("ft1", "Person teaches Course", vec![("Person", 0), ("Course", 1)])],
            vec![("UC", vec![("ft1", 0), ("ft1", 1)])], // compound UC
        );
        let tables = rmap(&state);
        assert!(tables.iter().any(|t| t.name == "person_teaches_course"));
        let jt = tables.iter().find(|t| t.name == "person_teaches_course").unwrap();
        assert_eq!(jt.primary_key.len(), 2);
    }

    #[test]
    fn mandatory_constraint_produces_not_null() {
        // Person has Name (UC on Person + MC on Person -> Name is NOT NULL)
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("MC", vec![("ft1", 0)]),
            ],
        );
        let tables = rmap(&state);
        let person = tables.iter().find(|t| t.name == "person").unwrap();
        let name_col = person.columns.iter().find(|c| c.name == "name").unwrap();
        assert!(!name_col.nullable); // MC -> NOT NULL
    }

    #[test]
    fn entity_fk_gets_references() {
        // Order belongs to Customer (UC on Order)
        let state = make_state(
            vec![("Order", "entity"), ("Customer", "entity")],
            vec![("ft1", "Order belongs to Customer", vec![("Order", 0), ("Customer", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let tables = rmap(&state);
        let order = tables.iter().find(|t| t.name == "order").unwrap();
        let cust_col = order.columns.iter().find(|c| c.name == "customer_id").unwrap();
        assert_eq!(cust_col.references.as_deref(), Some("customer"));
    }

    #[test]
    fn independent_entity_gets_id_table() {
        // Customer referenced by Order but has no own fact types with UC
        let state = make_state(
            vec![("Order", "entity"), ("Customer", "entity")],
            vec![("ft1", "Order belongs to Customer", vec![("Order", 0), ("Customer", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let tables = rmap(&state);
        let customer = tables.iter().find(|t| t.name == "customer").unwrap();
        assert_eq!(customer.columns.len(), 1); // just id
        assert_eq!(customer.primary_key, vec!["id"]);
    }

    // â”€â”€ Feature #57: External Uniqueness Constraint â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn external_uc_produces_unique_constraint() {
        // Room is in Building (UC on Room role -> functional)
        // Room has RoomNr (UC on Room role -> functional)
        // External UC spans both fact types on Room roles -> UNIQUE(building_id, room_nr)
        let state = make_state(
            vec![
                ("Room", "entity"),
                ("Building", "entity"),
                ("RoomNr", "value"),
            ],
            vec![
                ("ft1", "Room is in Building", vec![("Room", 0), ("Building", 1)]),
                ("ft2", "Room has RoomNr", vec![("Room", 0), ("RoomNr", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),   // each Room is in at most one Building
                ("UC", vec![("ft2", 0)]),   // each Room has at most one RoomNr
                // External UC: the combination of Building and RoomNr uniquely identifies Room
                ("UC", vec![("ft1", 1), ("ft2", 1)]),
            ],
        );
        let tables = rmap(&state);
        let room = tables.iter().find(|t| t.name == "room").unwrap();
        // Room table should have columns: id, building_id, room_nr
        assert!(room.columns.iter().any(|c| c.name == "building_id"));
        assert!(room.columns.iter().any(|c| c.name == "room_nr"));
        // Should have a UNIQUE constraint on (building_id, room_nr)
        let ucs = room.unique_constraints.as_ref().expect("should have unique constraints");
        assert!(ucs.iter().any(|uc| {
            uc.len() == 2
            && uc.contains(&"building_id".to_string())
            && uc.contains(&"room_nr".to_string())
        }), "Expected UNIQUE(building_id, room_nr), got {:?}", ucs);
    }

    // â”€â”€ Feature #58: Partitioned Subtype Absorption â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn make_state_with_subtypes(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
        subtypes: Vec<(&str, &str)>,
    ) -> ast::Object {
        let mut state = make_state(nouns, fact_types, constraints);
        // Patch existing Noun facts with superType where applicable.
        let sub_map: HashMap<&str, &str> = subtypes.iter().copied().collect();
        if let Object::Map(ref mut m_arc) = state { let m = alloc::sync::Arc::make_mut(m_arc);
            if let Some(Object::Seq(ref mut arc)) = m.get_mut("Noun") {
                let updated: Vec<Object> = arc.iter().map(|f| {
                    let name = ast::binding(f, "name").unwrap_or("").to_string();
                    match sub_map.get(name.as_str()) {
                        Some(parent) => {
                            // Re-emit this Noun fact with superType appended.
                            let obj_type = ast::binding(f, "objectType").unwrap_or("entity").to_string();
                            let wa = ast::binding(f, "worldAssumption").unwrap_or("closed").to_string();
                            let mut pairs: Vec<(&str, &str)> = vec![
                                ("name", name.as_str()), ("objectType", obj_type.as_str()),
                                ("worldAssumption", wa.as_str()), ("superType", *parent),
                            ];
                            if let Some(rs) = ast::binding(f, "referenceScheme") { pairs.push(("referenceScheme", rs)); }
                            fact_from_pairs(&pairs)
                        }
                        None => f.clone(),
                    }
                }).collect();
                *arc = updated.into();
            }
        }
        state
    }

    #[test]
    fn partitioned_subtype_gets_own_table() {
        // Person is the supertype. Employee is a subtype of Person.
        // Person has Name (functional on Person).
        // Employee has Salary (functional on Employee -- subtype-specific).
        // Because Employee has its own fact type, it should get a partitioned table.
        let state = make_state_with_subtypes(
            vec![
                ("Person", "entity"),
                ("Employee", "entity"),
                ("Name", "value"),
                ("Salary", "value"),
            ],
            vec![
                ("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)]),
                ("ft2", "Employee has Salary", vec![("Employee", 0), ("Salary", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft2", 0)]),
            ],
            vec![("Employee", "Person")],
        );
        let tables = rmap(&state);
        let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        // Person table should exist with name column but NOT salary
        let person = tables.iter().find(|t| t.name == "person").unwrap();
        assert!(person.columns.iter().any(|c| c.name == "name"), "Person should have name");
        assert!(!person.columns.iter().any(|c| c.name == "salary"),
            "Person should NOT have salary (partitioned)");
        // Employee table should exist with its own PK referencing person
        assert!(table_names.contains(&"employee"),
            "Employee should get its own table, got: {:?}", table_names);
        let employee = tables.iter().find(|t| t.name == "employee").unwrap();
        assert!(employee.columns.iter().any(|c| c.name == "salary"),
            "Employee table should have salary column");
        // Employee PK should reference Person
        let id_col = employee.columns.iter().find(|c| c.name == "id").unwrap();
        assert_eq!(id_col.references.as_deref(), Some("person"),
            "Employee id should FK to person");
    }

    #[test]
    fn absorbed_subtype_stays_in_supertype_table() {
        // Person is the supertype. VIPCustomer is a subtype but has no own fact types.
        // VIPCustomer should stay absorbed into Person (single-table).
        let state = make_state_with_subtypes(
            vec![
                ("Person", "entity"),
                ("VIPCustomer", "entity"),
                ("Name", "value"),
            ],
            vec![
                ("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
            ],
            vec![("VIPCustomer", "Person")],
        );
        let tables = rmap(&state);
        let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        // VIPCustomer should NOT get its own table (no fact types of its own)
        assert!(!table_names.contains(&"v_i_p_customer") && !table_names.contains(&"vip_customer"),
            "VIPCustomer should not get its own table: {:?}", table_names);
        // Person table should still exist
        assert!(table_names.contains(&"person"));
    }

    // â”€â”€ Feature #59: Compound Reference Scheme â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn make_state_with_ref_schemes(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
        ref_schemes: Vec<(&str, Vec<&str>)>,
    ) -> ast::Object {
        let mut state = make_state(nouns, fact_types, constraints);
        let rs_map: HashMap<&str, String> = ref_schemes.iter()
            .map(|(n, p)| (*n, p.join(",")))
            .collect();
        if let Object::Map(ref mut m_arc) = state { let m = alloc::sync::Arc::make_mut(m_arc);
            if let Some(Object::Seq(ref mut arc)) = m.get_mut("Noun") {
                let updated: Vec<Object> = arc.iter().map(|f| {
                    let name = ast::binding(f, "name").unwrap_or("").to_string();
                    match rs_map.get(name.as_str()) {
                        Some(rs_joined) => {
                            let obj_type = ast::binding(f, "objectType").unwrap_or("entity").to_string();
                            let wa = ast::binding(f, "worldAssumption").unwrap_or("closed").to_string();
                            fact_from_pairs(&[
                                ("name", name.as_str()), ("objectType", obj_type.as_str()),
                                ("worldAssumption", wa.as_str()),
                                ("referenceScheme", rs_joined.as_str()),
                            ])
                        }
                        None => f.clone(),
                    }
                }).collect();
                *arc = updated.into();
            }
        }
        state
    }

    #[test]
    fn compound_ref_scheme_produces_composite_pk() {
        // Room is in Building (UC on Room), Room has RoomNr (UC on Room)
        // Compound reference scheme: Room is identified by (Building, RoomNr)
        let state = make_state_with_ref_schemes(
            vec![
                ("Room", "entity"),
                ("Building", "entity"),
                ("RoomNr", "value"),
            ],
            vec![
                ("ft1", "Room is in Building", vec![("Room", 0), ("Building", 1)]),
                ("ft2", "Room has RoomNr", vec![("Room", 0), ("RoomNr", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft2", 0)]),
            ],
            vec![("Room", vec!["Building", "RoomNr"])],
        );
        let tables = rmap(&state);
        let room = tables.iter().find(|t| t.name == "room").unwrap();
        // PK should be composite: (building_id, room_nr)
        assert_eq!(room.primary_key.len(), 2,
            "Expected composite PK, got {:?}", room.primary_key);
        assert!(room.primary_key.contains(&"building_id".to_string()));
        assert!(room.primary_key.contains(&"room_nr".to_string()));
        // Should NOT have an "id" column
        assert!(!room.columns.iter().any(|c| c.name == "id"),
            "Should not have synthetic id column with compound ref scheme");
    }

    // â”€â”€ Feature #60: Fact Type Direction Bias â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn one_to_one_absorbs_toward_entity_not_value() {
        // Country has CountryCode (1:1, both UC).
        // Should absorb CountryCode into Country (entity over value).
        let state = make_state(
            vec![("Country", "entity"), ("CountryCode", "value")],
            vec![("ft1", "Country has CountryCode", vec![("Country", 0), ("CountryCode", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft1", 1)]),
            ],
        );
        let tables = rmap(&state);
        // Country table should absorb country_code
        let country = tables.iter().find(|t| t.name == "country").unwrap();
        assert!(country.columns.iter().any(|c| c.name == "country_code"),
            "Country should absorb country_code, columns: {:?}",
            country.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
    }

    #[test]
    fn one_to_one_absorbs_toward_larger_table() {
        // Person has SSN (1:1), Person has Name (functional on Person)
        // Person already has more columns -> SSN should be absorbed into Person
        let state = make_state(
            vec![
                ("Person", "entity"),
                ("SSN", "entity"),
                ("Name", "value"),
            ],
            vec![
                ("ft1", "Person has SSN", vec![("Person", 0), ("SSN", 1)]),
                ("ft2", "Person has Name", vec![("Person", 0), ("Name", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft1", 1)]),
                ("UC", vec![("ft2", 0)]),
            ],
        );
        let tables = rmap(&state);
        let person = tables.iter().find(|t| t.name == "person").unwrap();
        assert!(person.columns.iter().any(|c| c.name == "ssn_id"),
            "Person should absorb ssn_id, columns: {:?}",
            person.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
    }

    #[test]
    fn one_to_one_absorbs_using_reading_direction() {
        // Husband is married to Wife (1:1, both entities, same number of fact types)
        // Reading direction: Husband is first -> absorb into Husband
        let state = make_state(
            vec![("Husband", "entity"), ("Wife", "entity")],
            vec![("ft1", "Husband is married to Wife", vec![("Husband", 0), ("Wife", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft1", 1)]),
            ],
        );
        let tables = rmap(&state);
        let husband = tables.iter().find(|t| t.name == "husband").unwrap();
        assert!(husband.columns.iter().any(|c| c.name == "wife_id"),
            "Husband should absorb wife_id (reading direction), columns: {:?}",
            husband.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
        // Wife should NOT have husband_id
        let wife = tables.iter().find(|t| t.name == "wife");
        if let Some(w) = wife {
            assert!(!w.columns.iter().any(|c| c.name == "husband_id"),
                "Wife should NOT have husband_id");
        }
    }

    // ── #325: cell-based RMAP output for downstream generators ────────

    #[test]
    fn rmap_cells_emits_rmaptable_and_rmapcolumn_for_simple_entity() {
        // Person has Name (UC on Person). The typed API produces a
        // single `person` table with id + name columns. The cell API
        // must carry the same information via RMAPTable / RMAPColumn
        // rows keyed by table name — no typed struct crosses the
        // boundary.
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let cells = rmap_cells_from_state(&state);

        let tables = crate::ast::fetch_or_phi("RMAPTable", &cells);
        let table_rows = tables.as_seq().expect("RMAPTable cell must be a Seq");
        let person_row = table_rows.iter()
            .find(|f| crate::ast::binding(f, "name") == Some("person"))
            .expect("RMAPTable must carry a `person` row");
        assert_eq!(crate::ast::binding(person_row, "primaryKey"), Some("id"));

        let columns = crate::ast::fetch_or_phi("RMAPColumn", &cells);
        let col_rows = columns.as_seq().expect("RMAPColumn cell must be a Seq");
        let person_cols: Vec<&Object> = col_rows.iter()
            .filter(|f| crate::ast::binding(f, "table") == Some("person"))
            .collect();
        assert_eq!(person_cols.len(), 2,
            "person table has id + name — 2 column rows expected");

        // id column: nullable=false, no references
        let id_col = person_cols.iter()
            .find(|f| crate::ast::binding(f, "name") == Some("id"))
            .expect("id column must exist");
        assert_eq!(crate::ast::binding(id_col, "nullable"), Some("false"));
        assert_eq!(crate::ast::binding(id_col, "colType"), Some("TEXT"));
        assert_eq!(crate::ast::binding(id_col, "references"), None);

        // name column: references a value type => no reference target
        let name_col = person_cols.iter()
            .find(|f| crate::ast::binding(f, "name") == Some("name"))
            .expect("name column must exist");
        assert_eq!(crate::ast::binding(name_col, "references"), None);
    }

    #[test]
    fn table_name_for_noun_returns_snake_name_for_entity() {
        // Person(.name) entity => `person` table. Helper answers from
        // the RMAPTable cell, no typed-IR lookup.
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let cells = rmap_cells_from_state(&state);
        assert_eq!(table_name_for_noun(&cells, "Person"), Some("person".to_string()));
    }

    #[test]
    fn table_name_for_noun_returns_none_for_value_type() {
        // Value types don't produce their own RMAP table — helper
        // must return None so callers can skip them uniformly.
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let cells = rmap_cells_from_state(&state);
        assert_eq!(table_name_for_noun(&cells, "Name"), None);
    }

    #[test]
    fn columns_for_table_returns_columns_in_position_order() {
        // Columns must come back in declaration order — the cell
        // layout doesn't guarantee insertion order, so the helper
        // sorts by `position`. This is load-bearing for generators
        // that emit struct fields / schema properties in a fixed
        // order.
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let cells = rmap_cells_from_state(&state);
        let cols = columns_for_table(&cells, "person");
        assert_eq!(cols.len(), 2, "person table has id + name");
        assert_eq!(cols[0].name, "id", "id (PK) must come first");
        assert_eq!(cols[1].name, "name");
        assert_eq!(cols[0].nullable, false);
        assert_eq!(cols[0].col_type, "TEXT");
        assert!(cols[0].references.is_none());
    }

    #[test]
    fn columns_for_table_empty_for_unknown_table() {
        let cells = rmap_cells_from_state(&make_state(vec![], vec![], vec![]));
        assert!(columns_for_table(&cells, "nonexistent").is_empty());
    }

    #[test]
    fn primary_key_of_table_returns_columns_in_order() {
        // Single-PK `person` table returns `vec!["id"]`.
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let cells = rmap_cells_from_state(&state);
        assert_eq!(primary_key_of_table(&cells, "person"), vec!["id"]);
    }

    #[test]
    fn primary_key_of_table_preserves_composite_key_order() {
        // Compound ref scheme => composite PK. Helper must preserve
        // the order so generators can use it for SQL DDL / routing.
        let state = make_state_with_ref_schemes(
            vec![
                ("Room", "entity"),
                ("Building", "entity"),
                ("RoomNr", "value"),
            ],
            vec![
                ("ft1", "Room is in Building", vec![("Room", 0), ("Building", 1)]),
                ("ft2", "Room has RoomNr", vec![("Room", 0), ("RoomNr", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft2", 0)]),
            ],
            vec![("Room", vec!["Building", "RoomNr"])],
        );
        let cells = rmap_cells_from_state(&state);
        let pk = primary_key_of_table(&cells, "room");
        assert_eq!(pk.len(), 2);
        assert!(pk.contains(&"building_id".to_string()));
        assert!(pk.contains(&"room_nr".to_string()));
    }

    #[test]
    fn rmap_cells_encode_fk_references_and_composite_pk() {
        // Room(Building, RoomNr) — compound reference scheme produces
        // composite PK on (building_id, room_nr), with building_id
        // pointing to the building table via `references`.
        let state = make_state_with_ref_schemes(
            vec![
                ("Room", "entity"),
                ("Building", "entity"),
                ("RoomNr", "value"),
            ],
            vec![
                ("ft1", "Room is in Building", vec![("Room", 0), ("Building", 1)]),
                ("ft2", "Room has RoomNr", vec![("Room", 0), ("RoomNr", 1)]),
            ],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft2", 0)]),
            ],
            vec![("Room", vec!["Building", "RoomNr"])],
        );
        let cells = rmap_cells_from_state(&state);

        let tables = crate::ast::fetch_or_phi("RMAPTable", &cells);
        let table_rows = tables.as_seq().expect("RMAPTable cell must be a Seq");
        let room_row = table_rows.iter()
            .find(|f| crate::ast::binding(f, "name") == Some("room"))
            .expect("RMAPTable must carry a `room` row");
        let pk = crate::ast::binding(room_row, "primaryKey")
            .expect("room has a primaryKey binding");
        let pk_parts: Vec<&str> = pk.split(',').collect();
        assert_eq!(pk_parts.len(), 2,
            "compound ref scheme => composite PK, got {:?}", pk_parts);
        assert!(pk_parts.contains(&"building_id"));
        assert!(pk_parts.contains(&"room_nr"));

        // building_id column on room should carry `references=building`
        let columns = crate::ast::fetch_or_phi("RMAPColumn", &cells);
        let col_rows = columns.as_seq().unwrap();
        let building_fk = col_rows.iter()
            .find(|f|
                crate::ast::binding(f, "table") == Some("room")
                && crate::ast::binding(f, "name") == Some("building_id"))
            .expect("room.building_id column must exist");
        assert_eq!(crate::ast::binding(building_fk, "references"), Some("building"));
    }

    /// #214: rmap_func applied via ast::apply produces the same
    /// Vec<TableDef> as the direct Rust call. Pins the FFP entry
    /// point so future callers can ρ-dispatch to RMAP without
    /// reaching into the Rust procedure.
    #[test]
    fn rmap_func_round_trip_matches_direct_call() {
        let state = make_state(
            vec![("Person", "entity"), ("Name", "value")],
            vec![
                ("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)]),
            ],
            vec![("UC", vec![("ft1", 0)])],
        );
        let direct = rmap(&state);
        let via_apply = decode_rmap_result(
            &crate::ast::apply(&rmap_func(), &state, &state));

        assert_eq!(direct.len(), via_apply.len(),
            "Func-apply must produce the same number of tables as the direct call");
        for (a, b) in direct.iter().zip(via_apply.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.primary_key, b.primary_key);
            assert_eq!(
                a.columns.iter().map(|c| &c.name).collect::<Vec<_>>(),
                b.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
        }
    }

    /// H4 (#692): rmap_func MUST return a Platform variant — not a
    /// Native closure. Migrating the escape hatch into Platform makes
    /// the leaf introspectable (the audit / freeze / replay surfaces
    /// can see "this is rmap" instead of "<native>") and routable per
    /// runtime (server / FPGA / Solidity each pick their own body for
    /// the same name).
    #[test]
    fn rmap_func_is_platform_not_native() {
        match rmap_func() {
            crate::ast::Func::Platform(name) => assert_eq!(name, "rmap",
                "rmap_func must dispatch via the named platform op `rmap`"),
            other => panic!(
                "rmap_func must be Func::Platform(\"rmap\"), got {:?}", other),
        }
    }

    /// H4 (#692): the Func tree returned by rmap_func() must be
    /// fully introspectable — no Native escape hatches anywhere
    /// in the tree. has_native is the canonical purity probe.
    #[test]
    fn rmap_func_has_no_native_leaves() {
        assert!(!rmap_func().has_native(),
            "rmap_func must be free of Native leaves; the migration to \
             Platform is the whole point of H4");
    }

    /// #771: EntityCellRouter::route_fact must produce the same
    /// routing as the per-call `entity_cell_for_fact` for every
    /// (ft_id, fact) pair we throw at both. Pins parity so the perf
    /// fix is invisible to behaviour. The actual absorption direction
    /// (which role becomes the absorber under a single-role UC) is
    /// rmap_cell_map's choice; the parity test only requires that
    /// direct and router agree.
    #[test]
    fn router_route_fact_matches_per_call_entity_cell_for_fact() {
        let state = make_state(
            vec![
                ("Order", "entity"),
                ("Customer", "entity"),
                ("Course", "entity"),
                ("Person", "entity"),
                ("Total", "value"),
            ],
            vec![
                ("ft_total", "Order has Total", vec![("Order", 0), ("Total", 1)]),
                ("ft_cust", "Order belongs to Customer",
                    vec![("Order", 0), ("Customer", 1)]),
                // Compound UC -> own junction cell (route_fact returns None)
                ("ft_teach", "Person teaches Course",
                    vec![("Person", 0), ("Course", 1)]),
            ],
            vec![
                ("UC", vec![("ft_total", 0)]),
                ("UC", vec![("ft_cust", 0)]),
                ("UC", vec![("ft_teach", 0), ("ft_teach", 1)]),
            ],
        );

        let router = EntityCellRouter::new(&state);

        let cases: Vec<(&str, ast::Object)> = vec![
            ("ft_total",
                ast::fact_from_pairs(&[("Order", "ord-1"), ("Total", "100")])),
            ("ft_cust",
                ast::fact_from_pairs(&[("Order", "ord-2"), ("Customer", "cust-1")])),
            ("ft_teach",
                ast::fact_from_pairs(&[("Person", "alice"), ("Course", "cs101")])),
            // Unknown FT id — both must miss the shard map identically.
            ("ft_unknown",
                ast::fact_from_pairs(&[("Order", "ord-3"), ("Total", "200")])),
        ];

        for (ft_id, fact) in &cases {
            let direct = entity_cell_for_fact(&state, ft_id, fact);
            let routed = router.route_fact(ft_id, fact);

            assert_eq!(direct.is_some(), routed.is_some(),
                "direct vs router must agree on Some/None for ft = {}", ft_id);
            if let (Some(d), Some(r)) = (direct, routed) {
                assert_eq!(d.cell_name,    r.cell_name,    "cell_name parity, ft = {}", ft_id);
                assert_eq!(d.noun_name,    r.noun_name,    "noun_name parity, ft = {}", ft_id);
                assert_eq!(d.entity_id,    r.entity_id,    "entity_id parity, ft = {}", ft_id);
                assert_eq!(d.field_key,    r.field_key,    "field_key parity, ft = {}", ft_id);
                assert_eq!(d.field_value,  r.field_value,  "field_value parity, ft = {}", ft_id);
            }
        }

        // At least one case must route (Some) and at least one must
        // miss (None) — otherwise we're not exercising both branches.
        let some_count = cases.iter()
            .filter(|(ft, f)| router.route_fact(ft, f).is_some()).count();
        let none_count = cases.iter()
            .filter(|(ft, f)| router.route_fact(ft, f).is_none()).count();
        assert!(some_count >= 1, "test fixture must include at least one routable fact");
        assert!(none_count >= 1, "test fixture must include at least one non-routable fact");
    }

    /// #771: EntityCellRouter::id_field_for must match the per-call
    /// entity_id_field_name across nouns with explicit reference
    /// schemes, default-id fallback, and unknown nouns.
    #[test]
    fn router_id_field_for_matches_per_call_entity_id_field_name() {
        // make_state's entity nouns get referenceScheme="id" by default.
        let state = make_state(
            vec![("Order", "entity"), ("Customer", "entity"), ("Total", "value")],
            vec![
                ("ft1", "Order has Total", vec![("Order", 0), ("Total", 1)]),
            ],
            vec![("UC", vec![("ft1", 0)])],
        );
        let router = EntityCellRouter::new(&state);

        for noun in ["Order", "Customer", "Total", "DoesNotExist"] {
            let direct = entity_id_field_name(&state, noun);
            let routed = router.id_field_for(noun);
            assert_eq!(direct, routed,
                "id_field_for parity must hold for noun = {}", noun);
        }
    }
}

