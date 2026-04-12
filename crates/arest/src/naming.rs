// crates/arest/src/naming.rs
//
// Convention-based naming -- pure functions, no I/O.
// Noun names are the authority (from readings).
// Slugs and table names are deterministic projections.

/// Simple English pluralization for noun names.
pub fn pluralize(word: &str) -> String {
    let lower = word.to_lowercase();
    let es_suffix = lower.ends_with("ss") || lower.ends_with("sh") || lower.ends_with("ch") || lower.ends_with('x') || lower.ends_with('s');
    let z_suffix = lower.ends_with('z');
    let y_consonant = lower.ends_with('y')
        && !lower.ends_with("ay") && !lower.ends_with("ey")
        && !lower.ends_with("oy") && !lower.ends_with("uy")
        && !lower.ends_with("iy");
    match (es_suffix, z_suffix, y_consonant) {
        (true, _, _) => format!("{}es", word),             // Status/Box/Match/Bush -> ...es
        (_, true, _) => format!("{}zes", word),            // Quiz -> Quizzes
        (_, _, true) => format!("{}ies", &word[..word.len() - 1]), // Entity -> Entities
        _ => format!("{}s", word),
    }
}

/// Noun name -> REST collection slug (kebab-case, pluralized).
/// "Organization" -> "organizations"
/// "OrgMembership" -> "org-memberships"
/// "Fact Type" -> "fact-types"
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

/// Noun name -> SQL table name (snake_case, pluralized).
/// "Organization" -> "organizations"
/// "OrgMembership" -> "org_memberships"
/// "Fact Type" -> "fact_types"
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
        // Fold chars into (finished_words, current_word). Each char either
        // starts a new word (uppercase after non-empty current) or extends
        // the current word -- a pure Backus cond inside the fold.
        let (words, last) = name.chars().fold(
            (Vec::<String>::new(), String::new()),
            |(ws, cur), ch| {
                let boundary = ch.is_uppercase() && !cur.is_empty();
                let (ws, cur) = if boundary {
                    let ws = ws.into_iter().chain(std::iter::once(cur)).collect();
                    (ws, String::new())
                } else {
                    (ws, cur)
                };
                let cur = cur + &ch.to_string();
                (ws, cur)
            },
        );
        // Append trailing word as a pure cond: empty -> words, non-empty -> words + last.
        last.is_empty()
            .then(|| words.clone())
            .unwrap_or_else(|| words.into_iter().chain(std::iter::once(last)).collect())
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
    // Reference scheme is not stored in the IR nouns directly -- it's in the
    // fact types where this noun plays a role with a value-type noun.
    // For now, check if any fact type has this noun as subject with a value-type object.
    // The first value-type role's field value becomes the ID.
    //
    // A more precise approach: parse the (.RefScheme) from the noun declaration.
    // But the IR doesn't carry refScheme yet. When it does, use it directly.

    // Heuristic: find a field that looks like a reference scheme
    // Common patterns: "slug", "email", "code", "name" for value-type refs
    fields.iter()
        .filter(|(field, value)| !value.is_empty() && {
            let lower = field.to_lowercase();
            lower == "slug" || lower.ends_with("slug") || lower == "email" || lower == "code" || lower == "id"
        })
        .map(|(_, value)| value.clone())
        .next()
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
        assert_eq!(noun_to_slug("Fact Type"), "fact-types");
        assert_eq!(noun_to_slug("State Machine Definition"), "state-machine-definitions");
        assert_eq!(noun_to_slug("Status"), "statuses");
    }

    #[test]
    fn test_noun_to_table() {
        assert_eq!(noun_to_table("Organization"), "organizations");
        assert_eq!(noun_to_table("OrgMembership"), "org_memberships");
        assert_eq!(noun_to_table("Fact Type"), "fact_types");
        assert_eq!(noun_to_table("SupportRequest"), "support_requests");
        assert_eq!(noun_to_table("Status"), "statuses");
    }
}
