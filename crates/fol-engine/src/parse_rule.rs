// crates/fol-engine/src/parse_rule.rs
//
// Derivation Rule Parser — parses ORM derivation rules of the form:
//   Consequent := Antecedent1 and Antecedent2 and ...
//
// Produces structured triples from free-text rules using known noun names.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<Qualifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison: Option<Comparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal_value: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Qualifier {
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub op: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivationRule {
    pub text: String,
    pub consequent: RuleTriple,
    pub antecedents: Vec<RuleTriple>,
    pub kind: String, // "join", "comparison", "aggregate", "identity"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<Aggregate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aggregate {
    #[serde(rename = "fn")]
    pub func: String,
    pub noun: String,
}

struct NounMatch {
    name: String,
    start: usize,
    end: usize,
}

fn find_nouns(text: &str, nouns: &[String]) -> Vec<NounMatch> {
    let mut sorted: Vec<&String> = nouns.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut found = Vec::new();
    let lower = text.to_lowercase();

    for noun in sorted {
        let noun_lower = noun.to_lowercase();
        if let Some(start) = lower.find(&noun_lower) {
            // Check word boundaries
            let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
            let end = start + noun.len();
            let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();
            if before_ok && after_ok {
                found.push(NounMatch { name: noun.clone(), start, end });
            }
        }
    }

    found.sort_by_key(|m| m.start);
    found
}

fn split_conjuncts(rhs: &str, nouns: &[String]) -> Vec<String> {
    // Mask noun names containing "and" to avoid splitting on them
    let mut masked = rhs.to_string();
    let mut sorted: Vec<&String> = nouns.iter().filter(|n| n.contains(" and ") || n.contains(" And ")).collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut replacements = Vec::new();
    for noun in sorted {
        let placeholder = noun.replace(' ', "\0");
        if masked.contains(noun.as_str()) {
            masked = masked.replace(noun.as_str(), &placeholder);
            replacements.push((noun.clone(), placeholder));
        }
    }

    let parts: Vec<String> = masked.split(" and ").map(|s| {
        let mut result = s.trim().to_string();
        for (original, placeholder) in &replacements {
            result = result.replace(placeholder, original);
        }
        result
    }).collect();

    parts
}

fn extract_comparison(text: &str) -> (String, Option<Comparison>) {
    let re = regex::Regex::new(r"\s*(>=|<=|!=|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$").unwrap();
    if let Some(caps) = re.captures(text) {
        let m = caps.get(0).unwrap();
        let cleaned = text[..m.start()].trim().to_string();
        let op = caps[1].to_string();
        let value: f64 = caps[2].parse().unwrap_or(0.0);
        return (cleaned, Some(Comparison { op, value }));
    }
    (text.to_string(), None)
}

fn extract_literal(text: &str) -> (String, Option<String>) {
    if let Some(idx) = text.rfind('\'') {
        if let Some(start) = text[..idx].rfind('\'') {
            let value = text[start + 1..idx].to_string();
            let cleaned = text[..start].trim().to_string();
            return (cleaned, Some(value));
        }
    }
    (text.to_string(), None)
}

fn parse_triple(text: &str, nouns: &[String]) -> RuleTriple {
    let cleaned = text.strip_prefix("that ").unwrap_or(text);

    let (cleaned, literal_value) = extract_literal(cleaned);
    let (cleaned, comparison) = extract_comparison(&cleaned);

    let found = find_nouns(&cleaned, nouns);

    if found.len() < 2 {
        return RuleTriple {
            subject: found.first().map(|m| m.name.clone()).unwrap_or_default(),
            predicate: cleaned,
            object: String::new(),
            qualifier: None,
            comparison,
            literal_value,
        };
    }

    let subject = found[0].name.clone();
    let object_match = if found.len() >= 3 { &found[1] } else { found.last().unwrap() };
    let predicate = cleaned[found[0].end..object_match.start].trim().to_string();

    let mut triple = RuleTriple {
        subject,
        predicate,
        object: object_match.name.clone(),
        qualifier: None,
        comparison,
        literal_value,
    };

    if found.len() >= 3 {
        let qual = found.last().unwrap();
        let qual_pred = cleaned[object_match.end..qual.start].trim().to_string();
        triple.qualifier = Some(Qualifier { predicate: qual_pred, object: qual.name.clone() });
    }

    triple
}

pub fn parse_rule(text: &str, nouns: &[String]) -> Result<DerivationRule, String> {
    let cleaned = text.trim_end_matches('.');

    let split_idx = match cleaned.find(":=") {
        Some(idx) => idx,
        None => return Err(format!("Derivation rule must contain ':=': {}", text)),
    };

    let lhs = cleaned[..split_idx].trim();
    let rhs = cleaned[split_idx + 2..].trim();

    let consequent = parse_triple(lhs, nouns);

    // Identity: "the same"
    if rhs.contains("the same") {
        let antecedent = parse_triple(rhs, nouns);
        return Ok(DerivationRule {
            text: text.to_string(), consequent, antecedents: vec![antecedent], kind: "identity".to_string(), aggregate: None,
        });
    }

    // Aggregate: "count of X where ..."
    let agg_re = regex::Regex::new(r"(?i)^(count|sum|avg|min|max)\s+of\s+(.+?)\s+where\s+(.+)$").unwrap();
    if let Some(caps) = agg_re.captures(rhs) {
        let func = caps[1].to_lowercase();
        let agg_noun_text = caps[2].trim();
        let where_clause = caps[3].trim();

        let agg_nouns = find_nouns(agg_noun_text, nouns);
        let agg_noun = agg_nouns.first().map(|m| m.name.clone()).unwrap_or_else(|| agg_noun_text.to_string());

        let conjuncts = split_conjuncts(where_clause, nouns);
        let antecedents: Vec<RuleTriple> = conjuncts.iter().map(|c| parse_triple(c, nouns)).collect();

        return Ok(DerivationRule {
            text: text.to_string(), consequent, antecedents, kind: "aggregate".to_string(),
            aggregate: Some(Aggregate { func, noun: agg_noun }),
        });
    }

    // Default: split on "and"
    let conjuncts = split_conjuncts(rhs, nouns);
    let antecedents: Vec<RuleTriple> = conjuncts.iter().map(|c| parse_triple(c, nouns)).collect();
    let has_comparison = antecedents.iter().any(|a| a.comparison.is_some());
    let kind = if has_comparison { "comparison" } else { "join" };

    Ok(DerivationRule {
        text: text.to_string(), consequent, antecedents, kind: kind.to_string(), aggregate: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_join_rule() {
        let nouns = vec!["Constraint".to_string(), "Modality Type".to_string()];
        let rule = parse_rule("Constraint is semantic := Constraint has modality of Modality Type 'Deontic'.", &nouns).unwrap();
        assert_eq!(rule.kind, "join");
        assert_eq!(rule.consequent.subject, "Constraint");
        assert_eq!(rule.antecedents[0].literal_value.as_deref(), Some("Deontic"));
    }

    #[test]
    fn comparison_rule() {
        let nouns = vec!["Layer State".to_string(), "Arousal".to_string()];
        let rule = parse_rule("Layer State is alarmed := Layer State has Arousal > 0.8.", &nouns).unwrap();
        assert_eq!(rule.kind, "comparison");
        assert!(rule.antecedents[0].comparison.is_some());
        assert_eq!(rule.antecedents[0].comparison.as_ref().unwrap().op, ">");
    }

    #[test]
    fn missing_delimiter_errors() {
        let result = parse_rule("not a rule", &[]);
        assert!(result.is_err());
    }
}
