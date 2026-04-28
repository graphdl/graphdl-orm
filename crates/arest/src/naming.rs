// crates/arest/src/naming.rs
//
// Convention-based naming -- pure functions, no I/O.
// Noun names are the authority (from readings).
// Slugs and table names are deterministic projections.

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

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
                    let ws = ws.into_iter().chain(core::iter::once(cur)).collect();
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
            .unwrap_or_else(|| words.into_iter().chain(core::iter::once(last)).collect())
    }
}

/// Resolve an entity ID from its data using the noun's reference scheme.
///
/// Given a noun name and entity data fields, looks up the noun's reference
/// scheme in the compiled state and extracts the matching field value as the ID.
///
/// Returns None if no reference scheme, ref scheme is "id", or no matching field.
pub fn resolve_entity_id(
    state: &crate::ast::Object,
    noun_name: &str,
    fields: &hashbrown::HashMap<String, String>,
) -> Option<String> {
    // Guard: noun must exist in the Noun cell.
    let noun_cell = crate::ast::fetch_or_phi("Noun", state);
    let _noun_def = noun_cell.as_seq()?.iter()
        .find(|n| crate::ast::binding(n, "name") == Some(noun_name))?;
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

/// Resolve a REST collection slug to its Noun name by walking the
/// `Noun` cell in `state`. Mirror of the worker's
/// `resolveSlugToNoun(registry, slug)` (`src/collections.ts`), with
/// the same convention: every registered Noun's name is fed through
/// `noun_to_slug` and matched byte-for-byte against `slug`.
///
/// Returns `None` if no Noun produces the given slug — callers should
/// 404 the request rather than fall back, mirroring the worker
/// behaviour where an unknown collection is a hard miss.
///
/// Used by:
///   * arest-kernel's HATEOAS read fallback (#609 / #610) to map
///     `/arest/organizations` → `Organization` without a hand-written
///     slug→noun table.
///   * The arest worker's read path indirectly (the worker has its
///     own DurableObject-backed registry, but the convention is
///     identical so behaviour stays bit-for-bit equivalent across
///     deployment targets — same e2e suite passes).
pub fn resolve_slug_to_noun(state: &crate::ast::Object, slug: &str) -> Option<String> {
    crate::ast::fetch_or_phi("Noun", state)
        .as_seq()?
        .iter()
        .filter_map(|n| crate::ast::binding(n, "name"))
        .find(|name| noun_to_slug(name) == slug)
        .map(|name| name.to_string())
}

/// Atom IDs — entity reference values, enum members, slugs — must be
/// ASCII-only and case-insensitive-equivalent under ASCII fold. Free-form
/// text fields (Description, Violation message bodies, Reading text) keep
/// full Unicode since their identity is byte-exact, not case-folded.
///
/// Why the constraint:
///   - `Func::Lower` (#162) case-folds ASCII only. Adding Unicode case
///     mapping costs an i18n table in every WASM module we emit.
///   - FPGA fact-ingress ports (#168) allocate fixed-width name wires;
///     length-bounded ASCII fits a 32-byte port, full Unicode doesn't.
///   - SQL collation + OpenAPI path-parameter matching both rely on
///     byte-level equality or ASCII fold, so non-ASCII IDs round-trip
///     through the stack inconsistently.
///
/// Returns true if every byte of `s` is in the printable ASCII range
/// 0x20..=0x7E and the string is non-empty. Rejects control characters,
/// NUL bytes, and any multi-byte UTF-8 sequence.
pub fn atom_id_is_valid(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| (0x20..=0x7E).contains(&b))
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

    #[test]
    fn resolve_slug_round_trips_through_noun_to_slug() {
        // Build a state with three nouns; resolve_slug_to_noun should
        // match each one through the shared `noun_to_slug` projection.
        use crate::ast::{cell_push, Object};
        let nouns = ["Organization", "OrgMembership", "State Machine Definition"];
        let state = nouns.iter().fold(Object::phi(), |acc, name| {
            let fact = Object::seq(alloc::vec![
                Object::seq(alloc::vec![Object::atom("name"), Object::atom(name)]),
            ]);
            cell_push("Noun", fact, &acc)
        });

        assert_eq!(resolve_slug_to_noun(&state, "organizations"), Some("Organization".to_string()));
        assert_eq!(resolve_slug_to_noun(&state, "org-memberships"), Some("OrgMembership".to_string()));
        assert_eq!(
            resolve_slug_to_noun(&state, "state-machine-definitions"),
            Some("State Machine Definition".to_string()),
        );
    }

    #[test]
    fn resolve_slug_returns_none_for_unknown() {
        use crate::ast::{cell_push, Object};
        let fact = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("name"), Object::atom("Organization")]),
        ]);
        let state = cell_push("Noun", fact, &Object::phi());

        assert_eq!(resolve_slug_to_noun(&state, "support-requests"), None);
        assert_eq!(resolve_slug_to_noun(&state, ""), None);
    }

    #[test]
    fn resolve_slug_returns_none_when_noun_cell_empty() {
        // A bare state with no Noun cell at all: nothing to match.
        assert_eq!(resolve_slug_to_noun(&crate::ast::Object::phi(), "organizations"), None);
    }

    #[test]
    fn atom_id_accepts_printable_ascii() {
        // Canonical atom IDs across AREST.
        assert!(atom_id_is_valid("acme"));
        assert!(atom_id_is_valid("ord-1"));
        assert!(atom_id_is_valid("Order 42"));
        assert!(atom_id_is_valid("user@example.com"));
        assert!(atom_id_is_valid("Fact_Type_has_Role"));
        assert!(atom_id_is_valid("Widget Id")); // space is printable ASCII
        assert!(atom_id_is_valid("!#$%&()*+,./"));
    }

    #[test]
    fn atom_id_rejects_empty() {
        assert!(!atom_id_is_valid(""));
    }

    #[test]
    fn atom_id_rejects_control_characters() {
        assert!(!atom_id_is_valid("line1\nline2"));
        assert!(!atom_id_is_valid("tab\there"));
        assert!(!atom_id_is_valid("null\0byte"));
        assert!(!atom_id_is_valid("bell\x07"));
    }

    #[test]
    fn atom_id_rejects_non_ascii_bytes() {
        // Multi-byte UTF-8: each sequence has at least one byte >= 0x80.
        assert!(!atom_id_is_valid("café"));
        assert!(!atom_id_is_valid("naïve"));
        assert!(!atom_id_is_valid("Москва"));
        assert!(!atom_id_is_valid("東京"));
        assert!(!atom_id_is_valid("emoji\u{1F600}here"));
    }

    #[test]
    fn atom_id_rejects_del_and_boundary_bytes() {
        // 0x7F is DEL — printable-ASCII range excludes it.
        assert!(!atom_id_is_valid("\x7F"));
        // 0x1F is Unit Separator — below the printable range.
        assert!(!atom_id_is_valid("\x1F"));
        assert!(!atom_id_is_valid("ok\x1Funit_sep"));
    }
}
