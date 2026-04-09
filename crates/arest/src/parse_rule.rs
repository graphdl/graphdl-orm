// crates/arest/src/parse_rule.rs
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

    let lower = text.to_lowercase();
    let mut found: Vec<NounMatch> = sorted.iter().filter_map(|noun| {
        let start = lower.find(&noun.to_lowercase())?;
        let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
        let end = start + noun.len();
        let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();
        (before_ok && after_ok).then(|| NounMatch { name: noun.to_string(), start, end })
    }).collect();
    found.sort_by_key(|m| m.start);
    found
}

fn split_conjuncts(rhs: &str, nouns: &[String]) -> Vec<String> {
    // Mask noun names containing "and" to avoid splitting on them
    let masked = rhs.to_string();
    let mut sorted: Vec<&String> = nouns.iter().filter(|n| n.contains(" and ") || n.contains(" And ")).collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    let (masked, replacements) = sorted.iter()
        .filter(|noun| rhs.contains(noun.as_str()))
        .fold((masked, Vec::new()), |(m, mut r), noun| {
            let placeholder = noun.replace(' ', "\0");
            let new_m = m.replace(noun.as_str(), &placeholder);
            r.push(((*noun).clone(), placeholder));
            (new_m, r)
        });

    let parts: Vec<String> = masked.split(" and ").map(|s|
        replacements.iter().fold(s.trim().to_string(), |result, (original, placeholder)|
            result.replace(placeholder, original))
    ).collect();

    parts
}

fn extract_comparison(text: &str) -> (String, Option<Comparison>) {
    let re = regex::Regex::new(r"\s*(>=|<=|!=|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$").unwrap();
    re.captures(text)
        .map(|caps| {
            let m = caps.get(0).unwrap();
            let cleaned = text[..m.start()].trim().to_string();
            let op = caps[1].to_string();
            let value: f64 = caps[2].parse().unwrap_or(0.0);
            (cleaned, Some(Comparison { op, value }))
        })
        .unwrap_or_else(|| (text.to_string(), None))
}

fn extract_literal(text: &str) -> (String, Option<String>) {
    text.rfind('\'')
        .and_then(|idx| text[..idx].rfind('\'').map(|start| {
            let value = text[start + 1..idx].to_string();
            let cleaned = text[..start].trim().to_string();
            (cleaned, Some(value))
        }))
        .unwrap_or_else(|| (text.to_string(), None))
}

fn parse_triple(text: &str, nouns: &[String]) -> RuleTriple {
    let cleaned = text.strip_prefix("that ").unwrap_or(text);

    let (cleaned, literal_value) = extract_literal(cleaned);
    let (cleaned, comparison) = extract_comparison(&cleaned);

    let found = find_nouns(&cleaned, nouns);

    match found.len() {
        0 | 1 => RuleTriple {
            subject: found.first().map(|m| m.name.clone()).unwrap_or_default(),
            predicate: cleaned,
            object: String::new(),
            qualifier: None,
            comparison,
            literal_value,
        },
        _ => {
            let subject = found[0].name.clone();
            let object_match = if found.len() >= 3 { &found[1] } else { found.last().unwrap() };
            let predicate = cleaned[found[0].end..object_match.start].trim().to_string();
            let qualifier = (found.len() >= 3).then(|| {
                let qual = found.last().unwrap();
                let qual_pred = cleaned[object_match.end..qual.start].trim().to_string();
                Qualifier { predicate: qual_pred, object: qual.name.clone() }
            });
            RuleTriple {
                subject,
                predicate,
                object: object_match.name.clone(),
                qualifier,
                comparison,
                literal_value,
            }
        }
    }
}

pub fn parse_rule(text: &str, nouns: &[String]) -> Result<DerivationRule, String> {
    let cleaned = text.trim_end_matches('.');

    let split_idx = cleaned.find(":=")
        .ok_or_else(|| format!("Derivation rule must contain ':=': {}", text))?;

    let lhs = cleaned[..split_idx].trim();
    let rhs = cleaned[split_idx + 2..].trim();

    let consequent = parse_triple(lhs, nouns);

    // Identity: "the same" — pure expression form
    let identity_rule = rhs.contains("the same").then(|| {
        let antecedent = parse_triple(rhs, nouns);
        DerivationRule {
            text: text.to_string(), consequent: consequent.clone(), antecedents: vec![antecedent], kind: "identity".to_string(), aggregate: None,
        }
    });

    // Aggregate: "count of X where ..." — pure expression form
    let agg_re = regex::Regex::new(r"(?i)^(count|sum|avg|min|max)\s+of\s+(.+?)\s+where\s+(.+)$").unwrap();
    let aggregate_rule = || agg_re.captures(rhs).map(|caps| {
        let func = caps[1].to_lowercase();
        let agg_noun_text = caps[2].trim();
        let where_clause = caps[3].trim();

        let agg_nouns = find_nouns(agg_noun_text, nouns);
        let agg_noun = agg_nouns.first().map(|m| m.name.clone()).unwrap_or_else(|| agg_noun_text.to_string());

        let conjuncts = split_conjuncts(where_clause, nouns);
        let antecedents: Vec<RuleTriple> = conjuncts.iter().map(|c| parse_triple(c, nouns)).collect();

        DerivationRule {
            text: text.to_string(), consequent: consequent.clone(), antecedents, kind: "aggregate".to_string(),
            aggregate: Some(Aggregate { func, noun: agg_noun }),
        }
    });

    // Default: split on "and"
    let default_rule = || {
        let conjuncts = split_conjuncts(rhs, nouns);
        let antecedents: Vec<RuleTriple> = conjuncts.iter().map(|c| parse_triple(c, nouns)).collect();
        let has_comparison = antecedents.iter().any(|a| a.comparison.is_some());
        let kind = if has_comparison { "comparison" } else { "join" };
        DerivationRule {
            text: text.to_string(), consequent: consequent.clone(), antecedents, kind: kind.to_string(), aggregate: None,
        }
    };

    Ok(identity_rule.or_else(aggregate_rule).unwrap_or_else(default_rule))
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
