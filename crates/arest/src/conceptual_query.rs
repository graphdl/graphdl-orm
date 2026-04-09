// crates/arest/src/conceptual_query.rs
//
// Conceptual query resolution — natural language query → path of reading hops.
//
// "Support Requests that have Priority 'High'" resolves to:
//   path: [{ from: "SupportRequest", predicate: "has", to: "Priority", inverse: false }]
//   filters: [{ field: "Priority", value: "High" }]
//
// Algorithm:
// 1. Extract quoted literals as filters
// 2. Split on "that" to get segments
// 3. Find nouns in each segment (longest-first matching)
// 4. For each pair, find a reading connecting them
// 5. Build path + associate filters with target nouns

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPathStep {
    pub from: String,
    pub predicate: String,
    pub to: String,
    pub inverse: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptualQueryResult {
    pub path: Vec<QueryPathStep>,
    pub filters: Vec<Filter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_noun: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Filter {
    pub field: String,
    pub value: String,
}

pub struct Reading {
    pub text: String,
    pub nouns: Vec<String>,
}

fn extract_filters(query: &str) -> (String, Vec<String>) {
    // foldl over chars with (values, stripped, (in_quote, current_val)) accumulator.
    // Backus insert combining form — no external mutation.
    let (values, stripped, _) = query.chars().fold(
        (Vec::<String>::new(), String::new(), (false, String::new())),
        |(mut values, mut stripped, (in_quote, mut current_val)), ch| {
            if ch == '\'' {
                if in_quote {
                    values.push(current_val.clone());
                    current_val.clear();
                }
                (values, stripped, (!in_quote, current_val))
            } else if in_quote {
                current_val.push(ch);
                (values, stripped, (in_quote, current_val))
            } else {
                stripped.push(ch);
                (values, stripped, (in_quote, current_val))
            }
        },
    );

    // Normalize whitespace
    let stripped = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    (stripped, values)
}

fn find_nouns_in_segment(segment: &str, nouns: &[String]) -> Vec<String> {
    let mut sorted: Vec<&String> = nouns.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let (found, _) = sorted.iter().fold(
        (Vec::new(), segment.to_lowercase()),
        |(mut found, remaining), noun| {
            let lower = noun.to_lowercase();
            if remaining.contains(&lower) {
                found.push((*noun).clone());
                let remaining = remaining.replacen(&lower, "", 1);
                (found, remaining)
            } else {
                (found, remaining)
            }
        },
    );

    found
}

fn find_reading<'a>(from: &str, to: &str, readings: &'a [Reading]) -> Option<(&'a Reading, bool)> {
    let from_lower = from.to_lowercase();
    let to_lower = to.to_lowercase();

    // Forward: from is first noun
    let forward = readings.iter().find(|r| {
        r.nouns.len() >= 2
            && r.nouns[0].to_lowercase() == from_lower
            && r.nouns.iter().any(|n| n.to_lowercase() == to_lower)
    });
    if let Some(r) = forward { return Some((r, false)) }

    // Inverse: from appears but is not first
    let inverse = readings.iter().find(|r| {
        r.nouns.len() >= 2
            && r.nouns.iter().any(|n| n.to_lowercase() == from_lower)
            && r.nouns.iter().any(|n| n.to_lowercase() == to_lower)
            && r.nouns[0].to_lowercase() != from_lower
    });
    if let Some(r) = inverse { return Some((r, true)) }

    None
}

fn extract_predicate(reading: &Reading) -> String {
    let mut sorted = reading.nouns.clone();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));
    let text = sorted.iter().fold(reading.text.clone(), |acc, noun| acc.replace(noun, ""));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn resolve_conceptual_query(
    query: &str,
    nouns: &[String],
    readings: &[Reading],
) -> ConceptualQueryResult {
    let (stripped, filter_values) = extract_filters(query);

    let segments: Vec<&str> = stripped.split(" that ").collect();
    let segment_nouns: Vec<Vec<String>> = segments.iter()
        .map(|seg| find_nouns_in_segment(seg, nouns))
        .collect();

    let noun_sequence: Vec<String> = segment_nouns
        .iter()
        .flat_map(|seg_nouns| seg_nouns.iter().cloned())
        .fold(Vec::new(), |mut acc, noun| {
            if acc.last().map(|n: &String| n.to_lowercase()) != Some(noun.to_lowercase()) {
                acc.push(noun);
            }
            acc
        });

    if noun_sequence.is_empty() {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: None };
    }
    if noun_sequence.len() == 1 {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: Some(noun_sequence[0].clone()) };
    }

    let root_noun = noun_sequence[0].clone();

    let path: Vec<QueryPathStep> = (1..noun_sequence.len())
        .filter_map(|i| {
            let to = &noun_sequence[i];
            (0..i).rev().find_map(|j| {
                let from = &noun_sequence[j];
                find_reading(from, to, readings).map(|(reading, inverse)| QueryPathStep {
                    from: from.clone(),
                    predicate: extract_predicate(reading),
                    to: to.clone(),
                    inverse,
                })
            })
        })
        .collect();

    if path.is_empty() {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: None };
    }

    // Map filters to target nouns
    let filters: Vec<Filter> = query
        .split(" that ")
        .scan(0usize, |filter_idx, seg| {
            let count = seg.matches('\'').count() / 2;
            if count > 0 {
                let seg_nouns = find_nouns_in_segment(&seg.replace('\'', ""), nouns);
                let field = seg_nouns.last().cloned().unwrap_or_default();
                let seg_filters: Vec<Filter> = (0..count)
                    .filter_map(|_| {
                        let idx = *filter_idx;
                        *filter_idx += 1;
                        if idx < filter_values.len() && !field.is_empty() {
                            Some(Filter { field: field.clone(), value: filter_values[idx].clone() })
                        } else {
                            None
                        }
                    })
                    .collect();
                Some(seg_filters)
            } else {
                Some(vec![])
            }
        })
        .flatten()
        .collect();

    ConceptualQueryResult { path, filters, root_noun: Some(root_noun) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_binary_query() {
        let nouns = vec!["Person".to_string(), "Name".to_string()];
        let readings = vec![Reading { text: "Person has Name".to_string(), nouns: vec!["Person".to_string(), "Name".to_string()] }];
        let result = resolve_conceptual_query("Person that has Name", &nouns, &readings);
        assert_eq!(result.path.len(), 1);
        assert_eq!(result.path[0].from, "Person");
        assert_eq!(result.path[0].to, "Name");
        assert!(!result.path[0].inverse);
    }

    #[test]
    fn query_with_filter() {
        let nouns = vec!["SupportRequest".to_string(), "Priority".to_string()];
        let readings = vec![Reading { text: "SupportRequest has Priority".to_string(), nouns: vec!["SupportRequest".to_string(), "Priority".to_string()] }];
        let result = resolve_conceptual_query("SupportRequest that has Priority 'High'", &nouns, &readings);
        assert_eq!(result.path.len(), 1);
        assert_eq!(result.filters.len(), 1);
        assert_eq!(result.filters[0].field, "Priority");
        assert_eq!(result.filters[0].value, "High");
    }

    #[test]
    fn single_noun_returns_root() {
        let nouns = vec!["Person".to_string()];
        let result = resolve_conceptual_query("Person", &nouns, &[]);
        assert_eq!(result.path.len(), 0);
        assert_eq!(result.root_noun.as_deref(), Some("Person"));
    }

    #[test]
    fn inverse_reading() {
        let nouns = vec!["Department".to_string(), "Academic".to_string()];
        let readings = vec![Reading { text: "Academic works for Department".to_string(), nouns: vec!["Academic".to_string(), "Department".to_string()] }];
        let result = resolve_conceptual_query("Department that Academic works for", &nouns, &readings);
        assert_eq!(result.path.len(), 1);
        assert!(result.path[0].inverse);
    }
}
