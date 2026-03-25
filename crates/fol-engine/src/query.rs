// crates/fol-engine/src/query.rs
//
// Population query: filter a Population by a predicate and return matching entity references.
// Used by collection predicate evaluation (e.g., "all SupportRequests where Status = Investigating").

use crate::types::Population;
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
}
