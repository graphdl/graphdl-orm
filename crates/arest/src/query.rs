// crates/arest/src/query.rs
//
// Population query via partial application of graph schemas.
//
// A query is a partially applied graph schema (Backus 1977):
//   Schema     = CONS(Sel(1), ..., Sel(n))       — a Construction
//   Bind roles = eq ∘ [Sel(i), valuē]             — predicate per bound role
//   Filter     = Filter(predicate)                — keep matching facts
//   Extract    = α(Sel(target))                   — map selector over matches
//   Query      = α(Sel(target)) ∘ Filter(pred)   — composed function
//   Execute    = apply(query, population)          — beta reduction
//
// No Func::Native. No manual iteration. Pure AST throughout.

use crate::types::Population;
use crate::ast::{self, Func, Object};
use crate::compile::CompiledSchema;
#[cfg(test)]
use crate::types::FactInstance;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryPredicate {
    pub fact_type_id: String,
    pub target_noun: String,
    pub filter_bindings: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub matches: Vec<String>,  // entity references
    pub count: usize,
}

/// Filter a Population by a predicate.
/// Returns all entity references of target_noun where the filter_bindings match.
pub fn query_population(population: &Population, predicate: &QueryPredicate) -> QueryResult {
    let facts = match population.facts.get(&predicate.fact_type_id) {
        Some(facts) => facts,
        None => return QueryResult { matches: vec![], count: 0 },
    };

    let mut matches = Vec::new();
    for fact in facts {
        let all_filters_match = predicate.filter_bindings.iter().all(|(noun, value)| {
            fact.bindings.iter().any(|(n, v)| n == noun && v == value)
        });
        if all_filters_match {
            if let Some((_, entity_ref)) = fact.bindings.iter().find(|(n, _)| n == &predicate.target_noun) {
                matches.push(entity_ref.clone());
            }
        }
    }

    let count = matches.len();
    QueryResult { matches, count }
}

// ── Partial application as query ────────────────────────────────────
// Query = partial application of a graph schema.
// Given a schema and known bindings, return matching resources from population.

/// Convert a population's facts for a given fact type into an Object sequence
/// suitable for AST operations.
///
/// Each fact becomes a sequence of its bindings (ordered by the schema's role_names).
/// The population becomes a sequence of these fact sequences.
pub fn population_to_object(population: &Population, schema: &CompiledSchema) -> Object {
    let facts = match population.facts.get(&schema.id) {
        Some(f) => f,
        None => return Object::phi(),
    };

    let items: Vec<Object> = facts.iter().map(|fact| {
        let bindings: Vec<Object> = schema.role_names.iter().map(|role_name| {
            fact.bindings.iter()
                .find(|(n, _)| n == role_name)
                .map(|(_, v)| Object::atom(v))
                .unwrap_or(Object::Bottom)
        }).collect();
        Object::seq(bindings)
    }).collect();

    Object::Seq(items)
}

/// Build a predicate Func from filter bindings.
///
/// Single binding:  eq ∘ [Sel(i), valuē]
/// Multiple:        nested Condition — each check gates the next (pure AND).
/// Zero:            constant T (match all)
fn build_predicate(filter_bindings: &[(usize, &str)]) -> Func {
    let checks: Vec<Func> = filter_bindings.iter().map(|(role_idx, value)| {
        Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(*role_idx),
                Func::constant(Object::atom(value)),
            ]),
        )
    }).collect();

    match checks.len() {
        0 => Func::constant(Object::t()),
        1 => checks.into_iter().next().unwrap(),
        _ => {
            // AND via nested Condition: (p₁ → (p₂ → ... → T̄; F̄); F̄)
            // Each check gates the next — all must pass.
            checks.into_iter().rev().fold(
                Func::constant(Object::t()),
                |inner, check| Func::condition(check, inner, Func::constant(Object::f())),
            )
        }
    }
}

/// Build a query Func: α(Sel(target)) ∘ Filter(predicate).
///
/// This is partial application of a graph schema:
///   Schema = CONS(Sel(1), ..., Sel(n))
///   Bind some roles to constants → predicate
///   Filter(predicate) selects matching facts
///   α(Sel(target)) extracts the free role from matches
pub fn build_query(target_role: usize, filter_bindings: &[(usize, &str)]) -> Func {
    let predicate = build_predicate(filter_bindings);
    Func::compose(
        Func::apply_to_all(Func::Selector(target_role)),
        Func::filter(predicate),
    )
}

/// Query a population using partial application of a graph schema.
///
/// Given a compiled schema, a role index to extract (1-indexed), and
/// filter bindings (role_index, value), returns matching values.
///
/// This is: α(Sel(target)) ∘ Filter(predicate) applied to the population.
/// Pure AST — no Native closures, no manual iteration.
pub fn query_with_ast(
    population: &Population,
    schema: &CompiledSchema,
    target_role: usize,
    filter_bindings: &[(usize, &str)],
) -> Vec<String> {
    let defs = std::collections::HashMap::new();
    let pop = population_to_object(population, schema);

    let query = build_query(target_role, filter_bindings);
    let result = ast::apply(&query, &pop, &defs);

    match result.as_seq() {
        Some(items) => items.iter()
            .filter_map(|obj| obj.as_atom().map(|s| s.to_string()))
            .collect(),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_query_filters_by_binding() {
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("SupportRequest".to_string(), "sr-001".to_string()),
                    ("Status".to_string(), "Investigating".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("SupportRequest".to_string(), "sr-002".to_string()),
                    ("Status".to_string(), "Resolved".to_string()),
                ],
            },
        ]);
        let pop = Population { facts };

        let predicate = QueryPredicate {
            fact_type_id: "ft1".to_string(),
            target_noun: "SupportRequest".to_string(),
            filter_bindings: vec![("Status".to_string(), "Investigating".to_string())],
        };

        let result = query_population(&pop, &predicate);
        assert_eq!(result.count, 1);
        assert_eq!(result.matches, vec!["sr-001"]);
    }

    #[test]
    fn test_query_no_matches() {
        let pop = Population { facts: HashMap::new() };
        let predicate = QueryPredicate {
            fact_type_id: "ft1".to_string(),
            target_noun: "X".to_string(),
            filter_bindings: vec![],
        };
        let result = query_population(&pop, &predicate);
        assert_eq!(result.count, 0);
    }

    #[test]
    fn test_query_multiple_matches() {
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("Person".to_string(), "alice".to_string()),
                    ("City".to_string(), "Austin".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("Person".to_string(), "bob".to_string()),
                    ("City".to_string(), "Austin".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("Person".to_string(), "carol".to_string()),
                    ("City".to_string(), "Denver".to_string()),
                ],
            },
        ]);
        let pop = Population { facts };

        let predicate = QueryPredicate {
            fact_type_id: "ft1".to_string(),
            target_noun: "Person".to_string(),
            filter_bindings: vec![("City".to_string(), "Austin".to_string())],
        };

        let result = query_population(&pop, &predicate);
        assert_eq!(result.count, 2);
        assert!(result.matches.contains(&"alice".to_string()));
        assert!(result.matches.contains(&"bob".to_string()));
    }

    #[test]
    fn test_query_with_no_filter_returns_all() {
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("X".to_string(), "a".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("X".to_string(), "b".to_string())],
            },
        ]);
        let pop = Population { facts };

        let predicate = QueryPredicate {
            fact_type_id: "ft1".to_string(),
            target_noun: "X".to_string(),
            filter_bindings: vec![],
        };

        let result = query_population(&pop, &predicate);
        assert_eq!(result.count, 2);
    }

    // ── AST-based query tests (partial application) ─────────────

    fn make_schema(id: &str, role_names: Vec<&str>) -> CompiledSchema {
        let selectors: Vec<Func> = (0..role_names.len())
            .map(|i| Func::Selector(i + 1))
            .collect();
        CompiledSchema {
            id: id.to_string(),
            reading: String::new(),
            construction: Func::Construction(selectors),
            role_names: role_names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn ast_query_filters_by_single_binding() {
        let schema = make_schema("ft1", vec!["User", "Organization"]);
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("User".to_string(), "alice".to_string()),
                    ("Organization".to_string(), "org-1".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("User".to_string(), "bob".to_string()),
                    ("Organization".to_string(), "org-2".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("User".to_string(), "alice".to_string()),
                    ("Organization".to_string(), "org-3".to_string()),
                ],
            },
        ]);
        let pop = Population { facts };

        // Query: find organizations where User = "alice" (role 1 = "alice", extract role 2)
        let results = query_with_ast(&pop, &schema, 2, &[(1, "alice")]);
        assert_eq!(results, vec!["org-1", "org-3"]);
    }

    #[test]
    fn ast_query_filters_by_multiple_bindings() {
        let schema = make_schema("ft1", vec!["User", "Role", "Organization"]);
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("User".to_string(), "alice".to_string()),
                    ("Role".to_string(), "owner".to_string()),
                    ("Organization".to_string(), "org-1".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("User".to_string(), "alice".to_string()),
                    ("Role".to_string(), "member".to_string()),
                    ("Organization".to_string(), "org-2".to_string()),
                ],
            },
        ]);
        let pop = Population { facts };

        // Query: find orgs where User="alice" AND Role="owner"
        let results = query_with_ast(&pop, &schema, 3, &[(1, "alice"), (2, "owner")]);
        assert_eq!(results, vec!["org-1"]);
    }

    #[test]
    fn ast_query_no_matches_returns_empty() {
        let schema = make_schema("ft1", vec!["A", "B"]);
        let pop = Population { facts: HashMap::new() };
        let results = query_with_ast(&pop, &schema, 2, &[(1, "x")]);
        assert!(results.is_empty());
    }

    #[test]
    fn ast_query_no_filter_returns_all() {
        let schema = make_schema("ft1", vec!["X", "Y"]);
        let mut facts = HashMap::new();
        facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("X".to_string(), "a".to_string()), ("Y".to_string(), "1".to_string())],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![("X".to_string(), "b".to_string()), ("Y".to_string(), "2".to_string())],
            },
        ]);
        let pop = Population { facts };

        let results = query_with_ast(&pop, &schema, 1, &[]);
        assert_eq!(results, vec!["a", "b"]);
    }

    // ── build_query produces inspectable AST ────────────────────

    #[test]
    fn build_query_is_pure_ast() {
        // Verify the query function contains no Native nodes
        let query = build_query(2, &[(1, "alice"), (3, "active")]);
        let debug = format!("{:?}", query);
        assert!(!debug.contains("<native>"), "query must be pure AST, got: {}", debug);
        assert!(debug.contains("Filter"), "query must use Filter, got: {}", debug);
    }
}
