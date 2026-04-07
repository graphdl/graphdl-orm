// crates/arest/src/rmap.rs
//
// RMAP -- Relational Mapping Procedure (Halpin, Ch. 10)
//
// Pure function: Domain -> table definitions.
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

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use crate::types::Domain;

// -- Output types -----------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub col_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
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

fn to_snake(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            let prev = name.chars().nth(i - 1).unwrap_or(' ');
            if prev.is_lowercase() {
                result.push('_');
            }
        }
        if ch == ' ' || ch == '-' {
            result.push('_');
        } else {
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        }
    }
    result
}

fn fk_column_name(noun_name: &str) -> String {
    format!("{}_id", to_snake(noun_name))
}

fn value_column_name(noun_name: &str) -> String {
    to_snake(noun_name)
}

fn column_name_for_target(ir: &Domain, noun_name: &str) -> String {
    match ir.nouns.get(noun_name) {
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

pub fn rmap(ir: &Domain) -> Vec<TableDef> {
    let mut tables: Vec<TableDef> = Vec::new();
    let mut emitted: HashSet<String> = HashSet::new();

    // -- Step 0.1: Binarize exclusive unaries ------------------------
    let mut binarized_ft_ids: HashSet<String> = HashSet::new();
    let mut xo_columns: HashMap<String, Vec<(String, Vec<String>, bool)>> = HashMap::new();

    for constraint in &ir.constraints {
        if constraint.kind != "XO" { continue }
        if constraint.spans.len() < 2 { continue }

        let ft_ids: Vec<&str> = constraint.spans.iter().map(|s| s.fact_type_id.as_str()).collect();
        let unary_fts: Vec<_> = ft_ids.iter()
            .filter_map(|id| ir.fact_types.get(*id))
            .filter(|ft| ft.roles.len() == 1)
            .collect();
        if unary_fts.len() < 2 { continue }

        let entity_name = &unary_fts[0].roles[0].noun_name;
        let mut values = Vec::new();
        for ft in &unary_fts {
            let reading = &ft.reading;
            if let Some(caps) = reading.split(" is ").last() {
                values.push(caps.trim_end_matches('.').to_string());
            } else {
                let words: Vec<&str> = reading.split_whitespace().collect();
                values.push(words.last().unwrap_or(&"").to_string());
            }
        }
        for id in &ft_ids {
            binarized_ft_ids.insert(id.to_string());
        }

        let is_mandatory = unary_fts.iter().any(|ft| {
            let ft_id_str = ft_ids.iter().find(|id| ir.fact_types.get(**id).map(|f| std::ptr::eq(f, *ft)).unwrap_or(false));
            ft_id_str.map_or(false, |fid| {
                ir.constraints.iter().any(|c| c.kind == "MC" && c.spans.iter().any(|s| s.fact_type_id == *fid))
            })
        });

        let col_name = if values.iter().any(|v| v.to_lowercase() == "male" || v.to_lowercase() == "female") {
            "sex".to_string()
        } else {
            "status".to_string()
        };

        xo_columns.entry(entity_name.clone()).or_default().push((col_name, values, !is_mandatory));
    }

    // -- Step 0.3: Subtype absorption --------------------------------
    // Determine which subtypes have their own fact types (partitioned strategy)
    // vs which should be absorbed into the supertype (single-table strategy).
    let mut subtype_to_root: HashMap<String, String> = HashMap::new();
    let mut parent_of: HashMap<String, String> = HashMap::new();
    for (name, st) in &ir.subtypes {
        parent_of.insert(name.clone(), st.clone());
    }
    for name in parent_of.keys() {
        let mut current = name.clone();
        let mut visited = HashSet::new();
        while let Some(parent) = parent_of.get(&current) {
            if visited.contains(&current) { break }
            visited.insert(current.clone());
            current = parent.clone();
        }
        subtype_to_root.insert(name.clone(), current);
    }

    // Detect subtypes that have their own fact types -> partitioned strategy
    let mut partitioned_subtypes: HashSet<String> = HashSet::new();
    for subtype_name in subtype_to_root.keys() {
        let has_own_facts = ir.fact_types.values().any(|ft| {
            ft.roles.iter().any(|r| r.noun_name == *subtype_name)
        });
        if has_own_facts {
            partitioned_subtypes.insert(subtype_name.clone());
        }
    }

    let subtype_names: HashSet<&String> = subtype_to_root.keys().collect();
    let resolve_entity = |name: &str| -> String {
        // Partitioned subtypes map to themselves, not the root
        if partitioned_subtypes.contains(name) {
            return name.to_string();
        }
        subtype_to_root.get(name).cloned().unwrap_or_else(|| name.to_string())
    };

    // -- Index constraints -------------------------------------------
    let mut ucs_by_ft: HashMap<String, Vec<Vec<usize>>> = HashMap::new();
    let mut mc_set: HashSet<String> = HashSet::new();
    let mut vcs_by_ft_role: HashMap<String, Vec<String>> = HashMap::new();

    for c in &ir.constraints {
        match c.kind.as_str() {
            "UC" => {
                for span in &c.spans {
                    ucs_by_ft.entry(span.fact_type_id.clone()).or_default();
                }
                // Group spans by fact type -- each UC may span multiple roles
                let roles: Vec<usize> = c.spans.iter().map(|s| s.role_index).collect();
                if let Some(ft_id) = c.spans.first().map(|s| &s.fact_type_id) {
                    ucs_by_ft.entry(ft_id.clone()).or_default().push(roles);
                }
            }
            "MC" => {
                for span in &c.spans {
                    mc_set.insert(format!("{}:{}", span.fact_type_id, span.role_index));
                }
            }
            "VC" => {
                if let Some(ref entity) = c.entity {
                    for span in &c.spans {
                        if let Some(vals) = ir.enum_values.get(entity) {
                            vcs_by_ft_role.insert(
                                format!("{}:{}", span.fact_type_id, span.role_index),
                                vals.clone(),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // -- Classify fact types -----------------------------------------
    let mut compound_facts: Vec<&str> = Vec::new();
    let mut functional_facts: Vec<&str> = Vec::new();

    for (ft_id, ft) in &ir.fact_types {
        if binarized_ft_ids.contains(ft_id) { continue }
        if ft.roles.len() < 2 { continue }

        let ucs = ucs_by_ft.get(ft_id).cloned().unwrap_or_default();
        let mut is_compound = false;
        let mut is_functional = false;
        for uc in &ucs {
            if uc.len() >= 2 { is_compound = true }
            if uc.len() == 1 { is_functional = true }
        }
        if is_compound { compound_facts.push(ft_id) }
        if is_functional { functional_facts.push(ft_id) }
    }

    // -- Step 3 prep: Detect 1:1 -------------------------------------
    let mut one_to_one_ft_ids: HashSet<String> = HashSet::new();
    for ft_id in &functional_facts {
        let ft = &ir.fact_types[*ft_id];
        if ft.roles.len() != 2 { continue }
        let ucs = ucs_by_ft.get(*ft_id).cloned().unwrap_or_default();
        let single_uc_roles: Vec<usize> = ucs.iter().filter(|uc| uc.len() == 1).map(|uc| uc[0]).collect();
        let r0 = ft.roles[0].role_index;
        let r1 = ft.roles[1].role_index;
        if single_uc_roles.contains(&r0) && single_uc_roles.contains(&r1) {
            one_to_one_ft_ids.insert(ft_id.to_string());
        }
    }

    // -- Step 1: Compound UC -> separate table ------------------------
    let noun_name_set: HashSet<String> = ir.nouns.keys().cloned().collect();

    for ft_id in &compound_facts {
        let ft = &ir.fact_types[*ft_id];
        let ucs = ucs_by_ft.get(*ft_id).unwrap();
        let spanning_uc = ucs.iter().max_by_key(|uc| uc.len()).unwrap();

        let mut columns = Vec::new();
        let mut pk_cols = Vec::new();
        for role in &ft.roles {
            let col_name = column_name_for_target(ir, &role.noun_name);
            let is_entity = ir.nouns.get(&role.noun_name).map_or(false, |n| n.object_type == "entity");
            columns.push(TableColumn {
                name: col_name.clone(),
                col_type: "TEXT".to_string(),
                nullable: false,
                references: if is_entity { Some(to_snake(&role.noun_name)) } else { None },
            });
            if spanning_uc.contains(&role.role_index) {
                pk_cols.push(col_name);
            }
        }

        let table_name = compound_table_name(&ft.reading, &ft.roles, &noun_name_set);
        tables.push(TableDef { name: table_name.clone(), columns, primary_key: pk_cols, checks: None, unique_constraints: None });
        emitted.insert(table_name);
    }

    // -- Step 2: Functional roles -> entity table ---------------------
    let mut entity_columns: HashMap<String, (Vec<TableColumn>, HashSet<String>, Vec<String>)> = HashMap::new();

    for ft_id in &functional_facts {
        if one_to_one_ft_ids.contains(*ft_id) { continue }
        let ft = &ir.fact_types[*ft_id];
        let ucs = ucs_by_ft.get(*ft_id).cloned().unwrap_or_default();

        for uc in &ucs {
            if uc.len() != 1 { continue }
            let source_role_idx = uc[0];
            let source_role = match ft.roles.iter().find(|r| r.role_index == source_role_idx) {
                Some(r) => r,
                None => continue,
            };
            let source_noun = match ir.nouns.get(&source_role.noun_name) {
                Some(n) if n.object_type == "entity" => n,
                _ => continue,
            };

            let entity_key = resolve_entity(&source_role.noun_name);
            let entry = entity_columns.entry(entity_key).or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new()));
            let is_subtype = subtype_names.contains(&source_role.noun_name);

            for role in &ft.roles {
                if role.role_index == source_role_idx { continue }
                let col_name = column_name_for_target(ir, &role.noun_name);
                let is_mandatory = mc_set.contains(&format!("{}:{}", ft_id, source_role_idx));
                let is_entity = ir.nouns.get(&role.noun_name).map_or(false, |n| n.object_type == "entity");

                entry.0.push(TableColumn {
                    name: col_name.clone(),
                    col_type: "TEXT".to_string(),
                    nullable: if is_subtype { true } else { !is_mandatory },
                    references: if is_entity { Some(to_snake(&role.noun_name)) } else { None },
                });

                // VC check
                let vc_key = format!("{}:{}", ft_id, role.role_index);
                if let Some(vals) = vcs_by_ft_role.get(&vc_key) {
                    let quoted = vals.iter().map(|v| format!("'{}'", v)).collect::<Vec<_>>().join(", ");
                    entry.2.push(format!("{} IN ({})", col_name, quoted));
                }
            }
            let _ = source_noun; // used for type check above
        }
    }

    // -- Step 3: 1:1 absorption (with direction bias) ------------------
    // Count fact type participation per noun for "larger table" heuristic
    let mut noun_ft_count: HashMap<&str, usize> = HashMap::new();
    for ft in ir.fact_types.values() {
        for role in &ft.roles {
            *noun_ft_count.entry(&role.noun_name).or_insert(0) += 1;
        }
    }

    for ft_id in &one_to_one_ft_ids {
        let ft = &ir.fact_types[ft_id];
        let role0 = &ft.roles[0];
        let role1 = &ft.roles[1];
        let mc0 = mc_set.contains(&format!("{}:{}", ft_id, role0.role_index));
        let mc1 = mc_set.contains(&format!("{}:{}", ft_id, role1.role_index));

        // Direction bias priority:
        // 1. Mandatory constraint (absorb toward mandatory side)
        // 2. Entity vs value type (absorb toward entity)
        // 3. Larger table (more fact types)
        // 4. Reading direction (first noun is parent)
        let (absorb_into, fk_target, is_mandatory) = if mc0 && !mc1 {
            (resolve_entity(&role0.noun_name), &role1.noun_name, true)
        } else if mc1 && !mc0 {
            (resolve_entity(&role1.noun_name), &role0.noun_name, true)
        } else {
            // No mandatory asymmetry -- apply direction bias
            let is_entity0 = ir.nouns.get(&role0.noun_name).map_or(false, |n| n.object_type == "entity");
            let is_entity1 = ir.nouns.get(&role1.noun_name).map_or(false, |n| n.object_type == "entity");
            let both_mandatory = mc0 && mc1;

            if is_entity0 && !is_entity1 {
                // Absorb toward entity (role0)
                (resolve_entity(&role0.noun_name), &role1.noun_name, both_mandatory)
            } else if is_entity1 && !is_entity0 {
                // Absorb toward entity (role1)
                (resolve_entity(&role1.noun_name), &role0.noun_name, both_mandatory)
            } else {
                // Both entities (or both values) -- use fact type count
                let count0 = noun_ft_count.get(role0.noun_name.as_str()).copied().unwrap_or(0);
                let count1 = noun_ft_count.get(role1.noun_name.as_str()).copied().unwrap_or(0);
                if count0 > count1 {
                    (resolve_entity(&role0.noun_name), &role1.noun_name, both_mandatory)
                } else if count1 > count0 {
                    (resolve_entity(&role1.noun_name), &role0.noun_name, both_mandatory)
                } else {
                    // Equal -- use reading direction (role0 is first in reading)
                    (resolve_entity(&role0.noun_name), &role1.noun_name, both_mandatory)
                }
            }
        };

        let entry = entity_columns.entry(absorb_into).or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new()));
        let is_target_entity = ir.nouns.get(fk_target.as_str()).map_or(false, |n| n.object_type == "entity");
        entry.0.push(TableColumn {
            name: column_name_for_target(ir, fk_target),
            col_type: "TEXT".to_string(),
            nullable: !is_mandatory,
            references: if is_target_entity { Some(to_snake(fk_target)) } else { None },
        });
    }

    // -- Step 0.1 continued: inject XO columns -----------------------
    for (entity_name, xo_cols) in &xo_columns {
        let resolved = resolve_entity(entity_name);
        let entry = entity_columns.entry(resolved).or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new()));
        for (col_name, values, nullable) in xo_cols {
            entry.0.push(TableColumn {
                name: col_name.clone(),
                col_type: "TEXT".to_string(),
                nullable: *nullable,
                references: None,
            });
            let quoted = values.iter().map(|v| format!("'{}'", v)).collect::<Vec<_>>().join(", ");
            entry.2.push(format!("{} IN ({})", col_name, quoted));
        }
    }

    // -- Step 2.5: External UC -> UNIQUE constraints -------------------
    // External UCs span multiple fact types. Collect them per target entity.
    let mut external_ucs: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for c in &ir.constraints {
        if c.kind != "UC" { continue }
        if c.spans.len() < 2 { continue }
        // Check if spans reference different fact types
        let ft_ids: HashSet<&str> = c.spans.iter().map(|s| s.fact_type_id.as_str()).collect();
        if ft_ids.len() < 2 { continue }
        // This is an external UC. Find the target entity: the noun that plays
        // the source role in each spanned fact type (the UC'd role's counterpart).
        // For each span, the UC is on the non-source role; the source role
        // identifies which entity table the column lives in.
        let mut uc_cols: Vec<String> = Vec::new();
        let mut target_entity: Option<String> = None;
        for span in &c.spans {
            let ft = match ir.fact_types.get(&span.fact_type_id) {
                Some(ft) => ft,
                None => continue,
            };
            // The UC'd role is the one at span.role_index -> its column name
            let uc_role = match ft.roles.iter().find(|r| r.role_index == span.role_index) {
                Some(r) => r,
                None => continue,
            };
            let col_name = column_name_for_target(ir, &uc_role.noun_name);
            uc_cols.push(col_name);
            // The source role is the other role in the binary fact type
            for role in &ft.roles {
                if role.role_index != span.role_index {
                    let resolved = resolve_entity(&role.noun_name);
                    target_entity = Some(resolved);
                }
            }
        }
        if let Some(entity) = target_entity {
            if uc_cols.len() >= 2 {
                external_ucs.entry(entity).or_default().push(uc_cols);
            }
        }
    }

    // -- Emit entity tables ------------------------------------------
    for (entity_name, (columns, _, checks)) in &entity_columns {
        // Skip absorbed subtypes (non-partitioned) -- they are in the root table
        if subtype_names.contains(entity_name) && !partitioned_subtypes.contains(entity_name) {
            continue;
        }
        let table_name = to_snake(entity_name);
        let is_partitioned_subtype = partitioned_subtypes.contains(entity_name);

        // Feature #59: Compound reference scheme -> composite PK
        let compound_ref = ir.ref_schemes.get(entity_name)
            .filter(|parts| parts.len() >= 2);

        let (all_cols, pk) = if let Some(ref_parts) = compound_ref {
            // Compound reference scheme: use ref parts as composite PK
            let pk_cols: Vec<String> = ref_parts.iter()
                .map(|part| column_name_for_target(ir, part))
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

        let table = TableDef {
            name: table_name.clone(),
            columns: all_cols,
            primary_key: pk,
            checks: if checks.is_empty() { None } else { Some(checks.clone()) },
            unique_constraints: ext_ucs,
        };
        tables.push(table);
        emitted.insert(table_name);
    }

    // -- Step 4: Independent entity -> single-column table ------------
    let mut referenced: HashSet<String> = HashSet::new();
    for t in &tables {
        for col in &t.columns {
            if let Some(ref r) = col.references {
                referenced.insert(r.clone());
            }
        }
    }
    for ref_table in &referenced {
        if emitted.contains(ref_table) { continue }
        let noun = ir.nouns.iter().find(|(name, def)| to_snake(name) == *ref_table && def.object_type == "entity");
        if noun.is_none() { continue }
        let noun_entry = noun.unwrap();
        // Skip non-partitioned subtypes (they are absorbed into supertype)
        if subtype_names.contains(&noun_entry.0) && !partitioned_subtypes.contains(noun_entry.0) { continue }
        tables.push(TableDef {
            name: ref_table.clone(),
            columns: vec![TableColumn { name: "id".to_string(), col_type: "TEXT".to_string(), nullable: false, references: None }],
            primary_key: vec!["id".to_string()],
            checks: None,
            unique_constraints: None,
        });
        emitted.insert(ref_table.clone());
    }

    tables
}

// -- WASM export -----------------------------------------------------

/// Run RMAP on the currently loaded IR and return table definitions as JSON.
pub fn rmap_from_loaded_ir(ir: &Domain) -> Vec<TableDef> {
    rmap(ir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn make_ir(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
    ) -> Domain {
        let mut ir = Domain {
            domain: "test".to_string(),
            nouns: HashMap::new(),
            fact_types: HashMap::new(),
            constraints: Vec::new(),
            state_machines: HashMap::new(),
            derivation_rules: Vec::new(), general_instance_facts: Vec::new(),
            subtypes: HashMap::new(), enum_values: HashMap::new(),
            ref_schemes: HashMap::new(), objectifications: HashMap::new(),
            named_spans: HashMap::new(), autofill_spans: vec![],
        };
        for (name, obj_type) in nouns {
            ir.nouns.insert(name.to_string(), NounDef {
                object_type: obj_type.to_string(),
                world_assumption: WorldAssumption::default(),
            });
        }
        for (id, reading, roles) in fact_types {
            ir.fact_types.insert(id.to_string(), FactTypeDef {
                schema_id: String::new(),
                reading: reading.to_string(),
                readings: vec![],
                roles: roles.iter().map(|(name, idx)| RoleDef {
                    noun_name: name.to_string(),
                    role_index: *idx,
                }).collect(),
            });
        }
        for (kind, spans) in constraints {
            ir.constraints.push(ConstraintDef {
                id: format!("c_{}", ir.constraints.len()),
                kind: kind.to_string(),
                modality: "Alethic".to_string(),
                text: String::new(),
                spans: spans.iter().map(|(ft_id, role_idx)| SpanDef {
                    fact_type_id: ft_id.to_string(),
                    role_index: *role_idx,
                    subset_autofill: None,
                }).collect(),
                ..Default::default()
            });
        }
        ir
    }

    #[test]
    fn functional_binary_produces_entity_table() {
        // Person has Name (UC on Person role -> Name absorbed into Person table)
        let ir = make_ir(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![("UC", vec![("ft1", 0)])], // UC on Person -> each Person has at most one Name
        );
        let tables = rmap(&ir);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "person");
        assert_eq!(tables[0].columns.len(), 2); // id + name
        assert_eq!(tables[0].columns[1].name, "name");
        assert!(tables[0].columns[1].references.is_none()); // value type, no FK
    }

    #[test]
    fn compound_uc_produces_junction_table() {
        // Person teaches Course (UC spanning both roles -> junction table)
        let ir = make_ir(
            vec![("Person", "entity"), ("Course", "entity")],
            vec![("ft1", "Person teaches Course", vec![("Person", 0), ("Course", 1)])],
            vec![("UC", vec![("ft1", 0), ("ft1", 1)])], // compound UC
        );
        let tables = rmap(&ir);
        assert!(tables.iter().any(|t| t.name == "person_teaches_course"));
        let jt = tables.iter().find(|t| t.name == "person_teaches_course").unwrap();
        assert_eq!(jt.primary_key.len(), 2);
    }

    #[test]
    fn mandatory_constraint_produces_not_null() {
        // Person has Name (UC on Person + MC on Person -> Name is NOT NULL)
        let ir = make_ir(
            vec![("Person", "entity"), ("Name", "value")],
            vec![("ft1", "Person has Name", vec![("Person", 0), ("Name", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("MC", vec![("ft1", 0)]),
            ],
        );
        let tables = rmap(&ir);
        let person = tables.iter().find(|t| t.name == "person").unwrap();
        let name_col = person.columns.iter().find(|c| c.name == "name").unwrap();
        assert!(!name_col.nullable); // MC -> NOT NULL
    }

    #[test]
    fn entity_fk_gets_references() {
        // Order belongs to Customer (UC on Order)
        let ir = make_ir(
            vec![("Order", "entity"), ("Customer", "entity")],
            vec![("ft1", "Order belongs to Customer", vec![("Order", 0), ("Customer", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let tables = rmap(&ir);
        let order = tables.iter().find(|t| t.name == "order").unwrap();
        let cust_col = order.columns.iter().find(|c| c.name == "customer_id").unwrap();
        assert_eq!(cust_col.references.as_deref(), Some("customer"));
    }

    #[test]
    fn independent_entity_gets_id_table() {
        // Customer referenced by Order but has no own fact types with UC
        let ir = make_ir(
            vec![("Order", "entity"), ("Customer", "entity")],
            vec![("ft1", "Order belongs to Customer", vec![("Order", 0), ("Customer", 1)])],
            vec![("UC", vec![("ft1", 0)])],
        );
        let tables = rmap(&ir);
        let customer = tables.iter().find(|t| t.name == "customer").unwrap();
        assert_eq!(customer.columns.len(), 1); // just id
        assert_eq!(customer.primary_key, vec!["id"]);
    }

    // ── Feature #57: External Uniqueness Constraint ─────────────────

    #[test]
    fn external_uc_produces_unique_constraint() {
        // Room is in Building (UC on Room role -> functional)
        // Room has RoomNr (UC on Room role -> functional)
        // External UC spans both fact types on Room roles -> UNIQUE(building_id, room_nr)
        let ir = make_ir(
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
        let tables = rmap(&ir);
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

    // ── Feature #58: Partitioned Subtype Absorption ─────────────────

    fn make_ir_with_subtypes(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
        subtypes: Vec<(&str, &str)>,
    ) -> Domain {
        let mut ir = make_ir(nouns, fact_types, constraints);
        for (child, parent) in subtypes {
            ir.subtypes.insert(child.to_string(), parent.to_string());
        }
        ir
    }

    #[test]
    fn partitioned_subtype_gets_own_table() {
        // Person is the supertype. Employee is a subtype of Person.
        // Person has Name (functional on Person).
        // Employee has Salary (functional on Employee -- subtype-specific).
        // Because Employee has its own fact type, it should get a partitioned table.
        let ir = make_ir_with_subtypes(
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
        let tables = rmap(&ir);
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
        let ir = make_ir_with_subtypes(
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
        let tables = rmap(&ir);
        let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        // VIPCustomer should NOT get its own table (no fact types of its own)
        assert!(!table_names.contains(&"v_i_p_customer") && !table_names.contains(&"vip_customer"),
            "VIPCustomer should not get its own table: {:?}", table_names);
        // Person table should still exist
        assert!(table_names.contains(&"person"));
    }

    // ── Feature #59: Compound Reference Scheme ──────────────────────

    fn make_ir_with_ref_schemes(
        nouns: Vec<(&str, &str)>,
        fact_types: Vec<(&str, &str, Vec<(&str, usize)>)>,
        constraints: Vec<(&str, Vec<(&str, usize)>)>,
        ref_schemes: Vec<(&str, Vec<&str>)>,
    ) -> Domain {
        let mut ir = make_ir(nouns, fact_types, constraints);
        for (noun, parts) in ref_schemes {
            ir.ref_schemes.insert(
                noun.to_string(),
                parts.iter().map(|s| s.to_string()).collect(),
            );
        }
        ir
    }

    #[test]
    fn compound_ref_scheme_produces_composite_pk() {
        // Room is in Building (UC on Room), Room has RoomNr (UC on Room)
        // Compound reference scheme: Room is identified by (Building, RoomNr)
        let ir = make_ir_with_ref_schemes(
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
        let tables = rmap(&ir);
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

    // ── Feature #60: Fact Type Direction Bias ────────────────────────

    #[test]
    fn one_to_one_absorbs_toward_entity_not_value() {
        // Country has CountryCode (1:1, both UC).
        // Should absorb CountryCode into Country (entity over value).
        let ir = make_ir(
            vec![("Country", "entity"), ("CountryCode", "value")],
            vec![("ft1", "Country has CountryCode", vec![("Country", 0), ("CountryCode", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft1", 1)]),
            ],
        );
        let tables = rmap(&ir);
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
        let ir = make_ir(
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
        let tables = rmap(&ir);
        let person = tables.iter().find(|t| t.name == "person").unwrap();
        assert!(person.columns.iter().any(|c| c.name == "ssn_id"),
            "Person should absorb ssn_id, columns: {:?}",
            person.columns.iter().map(|c| &c.name).collect::<Vec<_>>());
    }

    #[test]
    fn one_to_one_absorbs_using_reading_direction() {
        // Husband is married to Wife (1:1, both entities, same number of fact types)
        // Reading direction: Husband is first -> absorb into Husband
        let ir = make_ir(
            vec![("Husband", "entity"), ("Wife", "entity")],
            vec![("ft1", "Husband is married to Wife", vec![("Husband", 0), ("Wife", 1)])],
            vec![
                ("UC", vec![("ft1", 0)]),
                ("UC", vec![("ft1", 1)]),
            ],
        );
        let tables = rmap(&ir);
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
}
