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
//   0.3. Subtype absorption (absorb into root supertype)
//   1.   Compound UC -> separate table (M:N, ternary+)
//   2.   Functional roles -> grouped into entity table
//   3.   1:1 absorption (absorb toward mandatory side)
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
    let subtype_names: HashSet<&String> = subtype_to_root.keys().collect();
    let resolve_entity = |name: &str| -> String {
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
        tables.push(TableDef { name: table_name.clone(), columns, primary_key: pk_cols, checks: None });
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

    // -- Step 3: 1:1 absorption --------------------------------------
    for ft_id in &one_to_one_ft_ids {
        let ft = &ir.fact_types[ft_id];
        let role0 = &ft.roles[0];
        let role1 = &ft.roles[1];
        let mc0 = mc_set.contains(&format!("{}:{}", ft_id, role0.role_index));
        let mc1 = mc_set.contains(&format!("{}:{}", ft_id, role1.role_index));

        let (absorb_into, fk_target, is_mandatory) = if mc0 && !mc1 {
            (resolve_entity(&role0.noun_name), &role1.noun_name, true)
        } else if mc1 && !mc0 {
            (resolve_entity(&role1.noun_name), &role0.noun_name, true)
        } else {
            (resolve_entity(&role0.noun_name), &role1.noun_name, mc0)
        };

        let entry = entity_columns.entry(absorb_into).or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new()));
        entry.0.push(TableColumn {
            name: fk_column_name(fk_target),
            col_type: "TEXT".to_string(),
            nullable: !is_mandatory,
            references: Some(to_snake(fk_target)),
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

    // -- Emit entity tables ------------------------------------------
    for (entity_name, (columns, _, checks)) in &entity_columns {
        if subtype_names.contains(entity_name) { continue }
        let table_name = to_snake(entity_name);
        let id_col = TableColumn { name: "id".to_string(), col_type: "TEXT".to_string(), nullable: false, references: None };
        let mut all_cols = vec![id_col];
        all_cols.extend(columns.iter().cloned());
        let table = TableDef {
            name: table_name.clone(),
            columns: all_cols,
            primary_key: vec!["id".to_string()],
            checks: if checks.is_empty() { None } else { Some(checks.clone()) },
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
        if subtype_names.contains(&noun.unwrap().0) { continue }
        tables.push(TableDef {
            name: ref_table.clone(),
            columns: vec![TableColumn { name: "id".to_string(), col_type: "TEXT".to_string(), nullable: false, references: None }],
            primary_key: vec!["id".to_string()],
            checks: None,
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
}
