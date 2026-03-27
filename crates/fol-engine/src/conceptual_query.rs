// crates/fol-engine/src/conceptual_query.rs
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
    let mut values = Vec::new();
    let mut stripped = String::new();
    let mut in_quote = false;
    let mut current_val = String::new();

    for ch in query.chars() {
        if ch == '\'' {
            if in_quote {
                values.push(current_val.clone());
                current_val.clear();
            }
            in_quote = !in_quote;
        } else if in_quote {
            current_val.push(ch);
        } else {
            stripped.push(ch);
        }
    }

    // Normalize whitespace
    let stripped = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    (stripped, values)
}

fn find_nouns_in_segment(segment: &str, nouns: &[String]) -> Vec<String> {
    let mut sorted: Vec<&String> = nouns.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut found = Vec::new();
    let mut remaining = segment.to_lowercase();

    for noun in sorted {
        let lower = noun.to_lowercase();
        if remaining.contains(&lower) {
            found.push(noun.clone());
            remaining = remaining.replacen(&lower, "", 1);
        }
    }

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
    let mut text = reading.text.clone();
    let mut sorted = reading.nouns.clone();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));
    for noun in &sorted {
        text = text.replace(noun, "");
    }
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

    let mut noun_sequence = Vec::new();
    for seg_nouns in &segment_nouns {
        for noun in seg_nouns {
            if noun_sequence.is_empty() || noun_sequence.last().map(|n: &String| n.to_lowercase()) != Some(noun.to_lowercase()) {
                noun_sequence.push(noun.clone());
            }
        }
    }

    if noun_sequence.is_empty() {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: None };
    }
    if noun_sequence.len() == 1 {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: Some(noun_sequence[0].clone()) };
    }

    let mut path = Vec::new();
    let root_noun = noun_sequence[0].clone();

    for i in 1..noun_sequence.len() {
        let to = &noun_sequence[i];
        for j in (0..i).rev() {
            let from = &noun_sequence[j];
            if let Some((reading, inverse)) = find_reading(from, to, readings) {
                let predicate = extract_predicate(reading);
                path.push(QueryPathStep {
                    from: from.clone(),
                    predicate,
                    to: to.clone(),
                    inverse,
                });
                break;
            }
        }
    }

    if path.is_empty() {
        return ConceptualQueryResult { path: vec![], filters: vec![], root_noun: None };
    }

    // Map filters to target nouns
    let mut filters = Vec::new();
    let original_segments: Vec<&str> = query.split(" that ").collect();
    let mut filter_idx = 0;
    for seg in &original_segments {
        let count = seg.matches('\'').count() / 2;
        if count > 0 {
            let seg_nouns = find_nouns_in_segment(&seg.replace('\'', ""), nouns);
            let field = seg_nouns.last().cloned().unwrap_or_default();
            for _ in 0..count {
                if filter_idx < filter_values.len() && !field.is_empty() {
                    filters.push(Filter { field: field.clone(), value: filter_values[filter_idx].clone() });
                }
                filter_idx += 1;
            }
        }
    }

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
