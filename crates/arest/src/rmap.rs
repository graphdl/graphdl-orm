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

use serde::{Serialize, Deserialize};
use hashbrown::{HashMap, HashSet};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// -- Output types -----------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub col_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<String>>,
    /// Additional UNIQUE constraints (each inner Vec is a set of column names)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_constraints: Option<Vec<Vec<String>>>,
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
    Object::Map(map)
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
    let rows = crate::ast::fetch_or_phi("RMAPTable", cells);
    rows.as_seq()?.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(snake.as_str()))
        .and_then(|f| crate::ast::binding(f, "name").map(String::from))
}

/// Return every column of a table in declaration order (sorted by the
/// `position` field). Returns an empty vec if the table is unknown.
pub(crate) fn columns_for_table(cells: &crate::ast::Object, table_name: &str) -> Vec<ColumnView> {
    let rows = crate::ast::fetch_or_phi("RMAPColumn", cells);
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

/// Return the primary-key columns of a table in order. Empty when the
/// table has no `RMAPTable` row or the `primaryKey` binding is empty.
pub fn primary_key_of_table(cells: &crate::ast::Object, table_name: &str) -> Vec<String> {
    let rows = crate::ast::fetch_or_phi("RMAPTable", cells);
    let Some(seq) = rows.as_seq() else { return Vec::new(); };
    seq.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(table_name))
        .and_then(|f| crate::ast::binding(f, "primaryKey"))
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(|p| p.to_string()).collect())
        .unwrap_or_default()
}

/// Return the extra UNIQUE constraints declared on a table — each inner
/// `Vec<String>` is a set of column names. Empty when none are declared.
pub(crate) fn unique_constraints_of_table(
    cells: &crate::ast::Object,
    table_name: &str,
) -> Vec<Vec<String>> {
    let rows = crate::ast::fetch_or_phi("RMAPTable", cells);
    let Some(seq) = rows.as_seq() else { return Vec::new(); };
    seq.iter()
        .find(|f| crate::ast::binding(f, "name") == Some(table_name))
        .and_then(|f| crate::ast::binding(f, "uniqueConstraints"))
        .filter(|s| !s.is_empty())
        .map(|s| s.split(';')
            .map(|grp| grp.split(',').map(|p| p.to_string()).collect())
            .collect())
        .unwrap_or_default()
}

/// #214: RMAP as a Func tree entry point.
///
/// Returns a `Func::Native` leaf that, when applied via `ast::apply`
/// against the state Object, produces an `Object::atom` containing
/// the JSON-serialized `Vec<TableDef>`. This is the ρ-dispatchable
/// form of RMAP — any caller that operates on Func trees (including
/// future lowered compile pipelines or the MCP dispatch layer) can
/// treat RMAP as a first-class ρ-application instead of reaching in
/// to the Rust procedure directly.
///
/// The leaf still wraps Halpin's procedural Ch. 10 algorithm as its
/// body. A deeper FFP rewrite would decompose the six RMAP passes
/// (binarize → absorb → classify-UC → one-to-one-absorb → compound-
/// ref-scheme → constraint-map) into a `FoldL` over per-pass Funcs,
/// each reading / augmenting an intermediate state Object. That
/// decomposition is tracked as a follow-up; keeping the body intact
/// here preserves every current behaviour.
pub fn rmap_func() -> crate::ast::Func {
    use alloc::sync::Arc;
    crate::ast::Func::Native(Arc::new(|state: &crate::ast::Object| {
        let tables = rmap(state);
        let json = serde_json::to_string(&tables).unwrap_or_else(|_| "[]".to_string());
        crate::ast::Object::atom(&json)
    }))
}

/// Decode the output of `apply(rmap_func(), state, state)` back into
/// `Vec<TableDef>`. The Func emits a JSON atom; this helper is the
/// inverse of that encoding.
pub fn decode_rmap_result(obj: &crate::ast::Object) -> Vec<TableDef> {
    obj.as_atom()
        .and_then(|s| serde_json::from_str::<Vec<TableDef>>(s).ok())
        .unwrap_or_default()
}

pub fn rmap(state: &crate::ast::Object) -> Vec<TableDef> {
    use crate::ast::{fetch_or_phi, binding};
    use crate::types::*;

    // Build typed lookups from cells â€” same data state_to_domain
    // produced, without the Domain struct.
    let noun_cell = fetch_or_phi("Noun", state);
    let mut nouns: HashMap<String, NounDef> = HashMap::new();
    let mut subtypes: HashMap<String, String> = HashMap::new();
    let mut ref_schemes: HashMap<String, Vec<String>> = HashMap::new();
    let mut enum_values: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(ns) = noun_cell.as_seq() {
        for f in ns.iter() {
            let name = binding(f, "name").unwrap_or("").to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
            if let Some(st) = binding(f, "superType") { subtypes.insert(name.clone(), st.to_string()); }
            if let Some(v) = binding(f, "referenceScheme") { ref_schemes.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
            if let Some(v) = binding(f, "enumValues") { enum_values.insert(name.clone(), v.split(',').map(|s| s.to_string()).collect()); }
        }
    }
    let role_cell = fetch_or_phi("Role", state);
    let fact_types: HashMap<String, FactTypeDef> = fetch_or_phi("FactType", state).as_seq()
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
    let constraints: Vec<ConstraintDef> = fetch_or_phi("Constraint", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = (0..4).filter_map(|i| {
                let ft_id = get(&format!("span{}_factTypeId", i))?;
                let ri = get(&format!("span{}_roleIndex", i))?;
                Some(SpanDef { fact_type_id: ft_id, role_index: ri.parse().unwrap_or(0), subset_autofill: None })
            }).collect();
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: None, clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
            }
        }).collect())
        .unwrap_or_default();

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
            match c.kind.as_str() {
                "UC" => {
                    c.spans.iter().for_each(|span| { ucs.entry(span.fact_type_id.clone()).or_default(); });
                    let roles: Vec<usize> = c.spans.iter().map(|s| s.role_index).collect();
                    c.spans.first()
                        .map(|s| &s.fact_type_id)
                        .into_iter()
                        .for_each(|ft_id| { ucs.entry(ft_id.clone()).or_default().push(roles.clone()); });
                }
                "MC" => {
                    mc.extend(c.spans.iter().map(|s| format!("{}:{}", s.fact_type_id, s.role_index)));
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
            (ft_id.as_str(), ucs.iter().any(|uc| uc.len() >= 2), ucs.iter().any(|uc| uc.len() == 1))
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
        let ucs = ucs_by_ft.get(*ft_id).unwrap();
        let spanning_uc = ucs.iter().max_by_key(|uc| uc.len()).unwrap();

        let columns: Vec<TableColumn> = ft.roles.iter().map(|role| {
            let col_name = column_name_for_target(&nouns, &role.noun_name);
            let is_entity = nouns.get(&role.noun_name).map_or(false, |n| n.object_type == "entity");
            TableColumn {
                name: col_name,
                col_type: "TEXT".to_string(),
                nullable: false,
                references: if is_entity { Some(to_snake(&role.noun_name)) } else { None },
            }
        }).collect();
        let pk_cols: Vec<String> = ft.roles.iter()
            .filter(|role| spanning_uc.contains(&role.role_index))
            .map(|role| column_name_for_target(&nouns, &role.noun_name))
            .collect();

        let table_name = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
        TableDef { name: table_name, columns, primary_key: pk_cols, checks: None, unique_constraints: None }
    }).collect();
    emitted.extend(compound_tables.iter().map(|t| t.name.clone()));
    tables.extend(compound_tables);

    // -- Step 2/3: Functional, 1:1 absorption, XO injection ----------
    //
    // Three pure data streams of (entity_key, column, Option<check>),
    // reduced into entity_columns via foldl (Backus insert combining form).
    // No external state mutation â€” each stream is computed from inputs only.

    let noun_ft_count: HashMap<&str, usize> = fact_types.values()
        .flat_map(|ft| ft.roles.iter().map(|r| r.noun_name.as_str()))
        .fold(HashMap::new(), |mut acc, name| { *acc.entry(name).or_insert(0) += 1; acc });

    let functional_additions: Vec<(String, TableColumn, Option<String>)> = functional_facts.iter()
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
                    };
                    let vc_key = format!("{}:{}", ft_id, role.role_index);
                    let check = vcs_by_ft_role.get(&vc_key).map(|vals| {
                        let quoted = vals.iter().map(|v| format!("'{}'", v)).collect::<Vec<_>>().join(", ");
                        format!("{} IN ({})", col_name, quoted)
                    });
                    (entity_key.clone(), column, check)
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // 1:1 absorption: direction bias via pure if-expression chain.
    // (Control flow has no side effects â€” returns a tuple, inputs â†’ output.)
    let one_to_one_additions: Vec<(String, TableColumn, Option<String>)> = one_to_one_ft_ids.iter().map(|ft_id| {
        let ft = &fact_types[ft_id];
        let role0 = &ft.roles[0];
        let role1 = &ft.roles[1];
        let mc0 = mc_set.contains(&format!("{}:{}", ft_id, role0.role_index));
        let mc1 = mc_set.contains(&format!("{}:{}", ft_id, role1.role_index));

        let (absorb_into, fk_target, is_mandatory) = if mc0 && !mc1 {
            (resolve_entity(&role0.noun_name), &role1.noun_name, true)
        } else if mc1 && !mc0 {
            (resolve_entity(&role1.noun_name), &role0.noun_name, true)
        } else {
            let is_entity0 = nouns.get(&role0.noun_name).map_or(false, |n| n.object_type == "entity");
            let is_entity1 = nouns.get(&role1.noun_name).map_or(false, |n| n.object_type == "entity");
            let both_mandatory = mc0 && mc1;
            if is_entity0 && !is_entity1 {
                (resolve_entity(&role0.noun_name), &role1.noun_name, both_mandatory)
            } else if is_entity1 && !is_entity0 {
                (resolve_entity(&role1.noun_name), &role0.noun_name, both_mandatory)
            } else {
                let count0 = noun_ft_count.get(role0.noun_name.as_str()).copied().unwrap_or(0);
                let count1 = noun_ft_count.get(role1.noun_name.as_str()).copied().unwrap_or(0);
                if count1 > count0 {
                    (resolve_entity(&role1.noun_name), &role0.noun_name, both_mandatory)
                } else {
                    // count0 >= count1 -- default to role0 (reading direction)
                    (resolve_entity(&role0.noun_name), &role1.noun_name, both_mandatory)
                }
            }
        };
        let is_target_entity = nouns.get(fk_target.as_str()).map_or(false, |n| n.object_type == "entity");
        let column = TableColumn {
            name: column_name_for_target(&nouns, fk_target),
            col_type: "TEXT".to_string(),
            nullable: !is_mandatory,
            references: if is_target_entity { Some(to_snake(fk_target)) } else { None },
        };
        (absorb_into, column, None)
    }).collect();

    let xo_additions: Vec<(String, TableColumn, Option<String>)> = xo_columns.iter()
        .flat_map(|(entity_name, xo_cols)| {
            let resolved = resolve_entity(entity_name);
            xo_cols.iter().map(move |(col_name, values, nullable)| {
                let column = TableColumn {
                    name: col_name.clone(),
                    col_type: "TEXT".to_string(),
                    nullable: *nullable,
                    references: None,
                };
                let quoted = values.iter().map(|v| format!("'{}'", v)).collect::<Vec<_>>().join(", ");
                let check = format!("{} IN ({})", col_name, quoted);
                (resolved.clone(), column, Some(check))
            }).collect::<Vec<_>>()
        })
        .collect();

    // Foldl all additions into entity_columns.
    // fold's accumulator mutation IS the insert combining form (Backus, FP Â§11).
    let entity_columns: HashMap<String, (Vec<TableColumn>, HashSet<String>, Vec<String>)> =
        functional_additions.into_iter()
            .chain(one_to_one_additions.into_iter())
            .chain(xo_additions.into_iter())
            .fold(HashMap::new(), |mut map, (key, col, check)| {
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
                };
                let mut all = vec![id_col];
                all.extend(columns.iter().cloned());
                (all, vec!["id".to_string()])
            };

            // Feature #57: Attach external UC as UNIQUE constraints
            let ext_ucs = external_ucs.get(entity_name).cloned();

            TableDef {
                name: table_name,
                columns: all_cols,
                primary_key: pk,
                checks: if checks.is_empty() { None } else { Some(checks.clone()) },
                unique_constraints: ext_ucs,
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
                columns: vec![TableColumn { name: "id".to_string(), col_type: "TEXT".to_string(), nullable: false, references: None }],
                primary_key: vec!["id".to_string()], checks: None, unique_constraints: None,
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
    use crate::ast::{fetch_or_phi, binding};
    use crate::types::*;
    let mut nouns: HashMap<String, NounDef> = HashMap::new();
    let mut subtypes: HashMap<String, String> = HashMap::new();
    if let Some(ns) = fetch_or_phi("Noun", state).as_seq() {
        for f in ns.iter() {
            let name = binding(f, "name").unwrap_or("").to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            nouns.insert(name.clone(), NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() });
            if let Some(st) = binding(f, "superType") { subtypes.insert(name.clone(), st.to_string()); }
        }
    }
    let role_cell = fetch_or_phi("Role", state);
    let fact_types: HashMap<String, FactTypeDef> = fetch_or_phi("FactType", state).as_seq()
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
    let constraints: Vec<ConstraintDef> = fetch_or_phi("Constraint", state).as_seq()
        .map(|facts| facts.iter().map(|f| {
            let get = |key: &str| binding(f, key).map(|s| s.to_string());
            let spans = (0..4).filter_map(|i| {
                let ft_id = get(&format!("span{}_factTypeId", i))?;
                let ri = get(&format!("span{}_roleIndex", i))?;
                Some(SpanDef { fact_type_id: ft_id, role_index: ri.parse().unwrap_or(0), subset_autofill: None })
            }).collect();
            ConstraintDef {
                id: get("id").unwrap_or_default(), kind: get("kind").unwrap_or_default(),
                modality: get("modality").unwrap_or_default(), deontic_operator: get("deonticOperator"),
                text: get("text").unwrap_or_default(), spans,
                set_comparison_argument_length: None, clauses: None, entity: get("entity"),
                min_occurrence: None, max_occurrence: None,
            }
        }).collect())
        .unwrap_or_default();
    let mut map = HashMap::new();
    let noun_name_set: HashSet<String> = nouns.keys().cloned().collect();

    // Index UCs by fact type (same as RMAP step classification)
    let ucs_by_ft: HashMap<String, Vec<Vec<usize>>> = constraints.iter()
        .filter(|c| c.kind == "UC")
        .fold(HashMap::new(), |mut acc, c| {
            let roles: Vec<usize> = c.spans.iter().map(|s| s.role_index).collect();
            c.spans.first().into_iter().for_each(|s| {
                acc.entry(s.fact_type_id.clone()).or_default().push(roles.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, Object, fact_from_pairs};
    use crate::types::*;

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
        Object::Map(cells.into_iter().map(|(k, v)| (k, Object::Seq(v.into()))).collect())
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
        if let Object::Map(ref mut m) = state {
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
        if let Object::Map(ref mut m) = state {
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
}
