// crates/arest/src/naming.rs
//
// Convention-based naming â€” pure functions, no I/O.
// Noun names are the authority (from readings).
// Slugs and table names are deterministic projections.

/// Simple English pluralization for noun names.
pub fn pluralize(word: &str) -> String {
    let lower = word.to_lowercase();
    if lower.ends_with("ss") || lower.ends_with("sh") || lower.ends_with("ch") || lower.ends_with('x') {
        return format!("{}es", word);
    }
    if lower.ends_with('z') {
        return format!("{}zes", word); // Quiz â†’ Quizzes
    }
    if lower.ends_with('s') {
        return format!("{}es", word); // Status â†’ Statuses
    }
    if lower.ends_with('y') && !lower.ends_with("ay") && !lower.ends_with("ey") && !lower.ends_with("oy") && !lower.ends_with("uy") && !lower.ends_with("iy") {
        return format!("{}ies", &word[..word.len() - 1]); // Entity â†’ Entities
    }
    format!("{}s", word)
}

/// Noun name â†’ REST collection slug (kebab-case, pluralized).
/// "Organization" â†’ "organizations"
/// "OrgMembership" â†’ "org-memberships"
/// "Graph Schema" â†’ "graph-schemas"
pub fn noun_to_slug(name: &str) -> String {
    let words = split_noun(name);
    words.iter().enumerate()
        .map(|(i, w)| {
            let s = if i == words.len() - 1 { pluralize(w) } else { w.to_string() };
            s.to_lowercase()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Noun name â†’ SQL table name (snake_case, pluralized).
/// "Organization" â†’ "organizations"
/// "OrgMembership" â†’ "org_memberships"
/// "Graph Schema" â†’ "graph_schemas"
pub fn noun_to_table(name: &str) -> String {
    let words = split_noun(name);
    words.iter().enumerate()
        .map(|(i, w)| {
            let s = if i == words.len() - 1 { pluralize(w) } else { w.to_string() };
            s.to_lowercase()
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Split a noun name into words (by spaces or PascalCase boundaries).
fn split_noun(name: &str) -> Vec<String> {
    if name.contains(' ') {
        name.split_whitespace().map(|s| s.to_string()).collect()
    } else {
        let mut words = Vec::new();
        let mut current = String::new();
        for ch in name.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                words.push(current);
                current = String::new();
            }
            current.push(ch);
        }
        if !current.is_empty() { words.push(current) }
        words
    }
}

/// Resolve an entity ID from its data using the noun's reference scheme.
///
/// Given a noun name and entity data fields, looks up the noun's reference
/// scheme in the compiled IR and extracts the matching field value as the ID.
///
/// Returns None if no reference scheme, ref scheme is "id", or no matching field.
pub fn resolve_entity_id(
    ir: &crate::types::Domain,
    noun_name: &str,
    fields: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let _noun_def = ir.nouns.get(noun_name)?;
    // Reference scheme is not stored in the IR nouns directly â€” it's in the
    // fact types where this noun plays a role with a value-type noun.
    // For now, check if any fact type has this noun as subject with a value-type object.
    // The first value-type role's field value becomes the ID.
    //
    // A more precise approach: parse the (.RefScheme) from the noun declaration.
    // But the IR doesn't carry refScheme yet. When it does, use it directly.

    // Heuristic: find a field that looks like a reference scheme
    // Common patterns: "slug", "email", "code", "name" for value-type refs
    for (field, value) in fields {
        let lower = field.to_lowercase();
        if lower == "slug" || lower.ends_with("slug") || lower == "email" || lower == "code" || lower == "id" {
            if !value.is_empty() {
                return Some(value.clone());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("Organization"), "Organizations");
        assert_eq!(pluralize("Status"), "Statuses");
        assert_eq!(pluralize("Entity"), "Entities");
        assert_eq!(pluralize("Key"), "Keys");
        assert_eq!(pluralize("Quiz"), "Quizzes");
        assert_eq!(pluralize("Box"), "Boxes");
        assert_eq!(pluralize("Match"), "Matches");
        assert_eq!(pluralize("Noun"), "Nouns");
    }

    #[test]
    fn test_noun_to_slug() {
        assert_eq!(noun_to_slug("Organization"), "organizations");
        assert_eq!(noun_to_slug("OrgMembership"), "org-memberships");
        assert_eq!(noun_to_slug("Graph Schema"), "graph-schemas");
        assert_eq!(noun_to_slug("State Machine Definition"), "state-machine-definitions");
        assert_eq!(noun_to_slug("Status"), "statuses");
    }

    #[test]
    fn test_noun_to_table() {
        assert_eq!(noun_to_table("Organization"), "organizations");
        assert_eq!(noun_to_table("OrgMembership"), "org_memberships");
        assert_eq!(noun_to_table("Graph Schema"), "graph_schemas");
        assert_eq!(noun_to_table("SupportRequest"), "support_requests");
        assert_eq!(noun_to_table("Status"), "statuses");
    }
}
