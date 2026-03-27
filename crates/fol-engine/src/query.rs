// crates/fol-engine/src/query.rs
//
// Population query via the FP AST.
//
// A query is a partially applied graph schema:
//   BinaryToUnary(Eq, known_resource) checks if a role matches.
//   ApplyToAll maps the check over a population.
//   Insert(/or) aggregates to "exists".
//   Compose chains these into a derivation.
//
// The old QueryPredicate struct is kept for backward compatibility.
// New code should use query_with_ast which operates on Func nodes.

use crate::types::Population;
use crate::ast::{self, Func, Object};
use crate::compile::CompiledSchema;
#[cfg(test)]
use crate::types::FactInstance;
use serde::{Serialize, Deserialize};
use std::sync::Arc;

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

// ── AST-based query ──────────────────────────────────────────────────
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

/// Query a population using AST partial application.
///
/// Given a compiled schema, a role index to extract (1-indexed), and
/// filter bindings (role_index, value), returns matching values.
///
/// This is: α(target_selector) ∘ filter(bindings) applied to the population.
/// Filter uses composition + condition on each binding.
pub fn query_with_ast(
    population: &Population,
    schema: &CompiledSchema,
    target_role: usize,
    filter_bindings: &[(usize, &str)],
) -> Vec<String> {
    let defs = std::collections::HashMap::new();
    let pop = population_to_object(population, schema);

    if matches!(pop, Object::Seq(ref items) if items.is_empty()) {
        return vec![];
    }

    // Build a predicate that checks all filter bindings:
    // For each (role_idx, value): eq ∘ [Selector(role_idx), valuē]
    // Combined with AND: all must be T
    let mut check_fns: Vec<Func> = filter_bindings.iter().map(|(role_idx, value)| {
        Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(*role_idx),
                Func::constant(Object::atom(value)),
            ]),
        )
    }).collect();

    // Build combined predicate: and all checks
    // For a single check, just use it directly.
    // For multiple, chain with native AND.
    let predicate = if check_fns.is_empty() {
        Func::constant(Object::t()) // No filters = always match
    } else if check_fns.len() == 1 {
        check_fns.remove(0)
    } else {
        // Native AND combinator
        let and_fn: ast::Fn1 = Arc::new(|x: &Object| {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let a = items[0].as_atom().unwrap_or("F");
                    let b = items[1].as_atom().unwrap_or("F");
                    if a == "T" && b == "T" { Object::t() } else { Object::f() }
                }
                _ => Object::Bottom,
            }
        });

        // /and ∘ [check₁, check₂, ..., checkₙ]
        let all_checks = Func::construction(check_fns);
        Func::compose(
            Func::insert(Func::Native(and_fn)),
            all_checks,
        )
    };

    // Apply predicate to each fact, collect target role from matching facts
    let mut results = Vec::new();
    if let Some(items) = pop.as_seq() {
        for item in items {
            let check = ast::apply(&predicate, item, &defs);
            if check == Object::t() {
                let target = ast::apply(&Func::Selector(target_role), item, &defs);
                if let Some(val) = target.as_atom() {
                    results.push(val.to_string());
                }
            }
        }
    }

    results
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

    // ── AST-based query tests ────────────────────────────────────

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
}
