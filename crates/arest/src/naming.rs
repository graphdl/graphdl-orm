// crates/arest/src/naming.rs
//
// Convention-based naming -- pure functions, no I/O.
// Noun names are the authority (from readings).
// Slugs and table names are deterministic projections.

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// #895 Sweep-1 dispatch-to-data lift: the legacy English pluralization
/// cascade (suffix-table + irregulars) lifts to a typed
/// `PluralizationRuleTable` whose rows live in `readings/core/naming.md`
/// as two parallel enum value types — `Pluralization Pattern` paired
/// with `Pluralization Replacement`. Same shape as
/// `WordComparatorTable` (#783) / `RangeOperatorTable` (#783) /
/// `QuoteEscapeTable` (#844): boot mirrors the historical cascade so
/// behavior round-trips, `from_grammar_state` reads the same data from
/// a parsed reading state, and the domain accessor (`pluralize`)
/// applies the rules first-match-wins.
///
/// Pattern dialect — three forms, dispatched by leading char and
/// trailing `$`:
///   * `^WORD$` matches the entire lowercased word; replacement is
///     returned literally (preserving its case). Used by irregulars
///     (`^child$` → `children`, `^person$` → `people`).
///   * `SUFFIX$` matches the lowercased word's tail; the matched
///     suffix is stripped from the original word and the lowercase
///     replacement is appended (so `Match` + `ch$ → ches` yields
///     `Matches`, preserving the leading capital).
///   * `$` (or empty) matches any word; replacement is appended
///     verbatim. Used as the trailing default → `+s`.
#[derive(Debug, Clone)]
pub struct PluralizationRuleTable {
    /// Pattern + replacement pairs, in cascade order. Order MATTERS —
    /// the original cascade was first-match-wins, so vowel-y patterns
    /// must precede the consonant-y catchall, specific es-suffixes
    /// must precede bare `s$`, and the empty-pattern default must be
    /// last. Mirrors the declaration order in
    /// `readings/core/naming.md` (`Pluralization Pattern` ↔
    /// `Pluralization Replacement` parallel enum values).
    pub rows: Vec<(String, String)>,
}

impl PluralizationRuleTable {
    /// Boot table — must stay in sync with the parallel enum-value
    /// declarations of `Pluralization Pattern` and
    /// `Pluralization Replacement` in `readings/core/naming.md`. The
    /// declaration order mirrors the legacy `pluralize` cascade in
    /// `naming.rs` so first-match-wins behavior round-trips on every
    /// historical input.
    pub fn boot() -> Self {
        PluralizationRuleTable {
            rows: alloc::vec![
                // Irregulars (full-word, case-insensitive). Replacement
                // returned verbatim; case-preservation is the caller's
                // problem (most callers feed canonical-cased nouns).
                ("^child$".to_string(),  "children".to_string()),
                ("^person$".to_string(), "people".to_string()),
                // Vowel-y suffixes — these LOOK consonant-y to a naive
                // `ends_with('y')` test but the preceding vowel makes
                // them regular: `Key` → `Keys`, `Day` → `Days`.
                // Strip the suffix and re-append the lowercase variant
                // ending in `s` (so capitalization on the prefix
                // survives).
                ("ay$".to_string(), "ays".to_string()),
                ("ey$".to_string(), "eys".to_string()),
                ("oy$".to_string(), "oys".to_string()),
                ("uy$".to_string(), "uys".to_string()),
                ("iy$".to_string(), "iys".to_string()),
                // ES-suffix family: ss/sh/ch/x/s → append `es`. Encoded
                // as "strip the suffix, append suffix+es" so the rule
                // engine has uniform shape with the y/z cases.
                ("ss$".to_string(), "sses".to_string()),
                ("sh$".to_string(), "shes".to_string()),
                ("ch$".to_string(), "ches".to_string()),
                ("x$".to_string(),  "xes".to_string()),
                ("s$".to_string(),  "ses".to_string()),
                // Z-suffix: `Quiz` → `Quizzes` (doubled z plus es).
                ("z$".to_string(),  "zzes".to_string()),
                // Consonant-y catchall (vowel-y already handled above).
                ("y$".to_string(),  "ies".to_string()),
                // Default: empty pattern matches any word; append `s`.
                ("$".to_string(),   "s".to_string()),
            ],
        }
    }

    /// Build the table from the runtime parallel enum-value
    /// declarations of `Pluralization Pattern` and
    /// `Pluralization Replacement`. Falls back to `boot()` when either
    /// list is empty or the lengths don't match — a malformed
    /// declaration must not silently truncate the table.
    pub fn from_grammar_state(state: &crate::ast::Object) -> Self {
        let patterns = read_enum_values(state, "Pluralization Pattern");
        let replacements = read_enum_values(state, "Pluralization Replacement");
        if !patterns.is_empty() && patterns.len() == replacements.len() {
            PluralizationRuleTable {
                rows: patterns.into_iter().zip(replacements).collect(),
            }
        } else {
            Self::boot()
        }
    }

    /// Apply the rules first-match-wins and return the pluralized
    /// form. Mirrors the legacy `pluralize` API so callers
    /// (`noun_to_slug`, `noun_to_table`) stay state-free.
    pub fn pluralize(&self, word: &str) -> String {
        let lower = word.to_lowercase();
        for (pattern, replacement) in &self.rows {
            // Full-word irregular: `^WORD$` — case-insensitive equality
            // against the lowercased word. Replacement is returned with
            // the leading character's case lifted from the original
            // input so `Child` → `Children` (not `children`) and the
            // suffix-replacement rules' case-preservation invariant
            // extends uniformly to irregulars.
            if let Some(inner) = pattern.strip_prefix('^').and_then(|p| p.strip_suffix('$')) {
                if lower == inner {
                    let first_uppercase = word.chars().next()
                        .map(|c| c.is_uppercase()).unwrap_or(false);
                    return if first_uppercase {
                        let mut chars = replacement.chars();
                        match chars.next() {
                            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                            None => replacement.clone(),
                        }
                    } else {
                        replacement.clone()
                    };
                }
                continue;
            }
            // Suffix rule: `SUFFIX$` — match the tail of the lowercased
            // word, strip the corresponding bytes from the original
            // (preserving prefix case), append replacement verbatim.
            // Empty pattern (`$` or `""`) matches any word as a
            // zero-length suffix → default rule.
            if let Some(suffix) = pattern.strip_suffix('$') {
                if lower.ends_with(suffix) {
                    let head = &word[..word.len() - suffix.len()];
                    return alloc::format!("{}{}", head, replacement);
                }
                continue;
            }
            // Pattern with neither `^` nor trailing `$` is treated as a
            // bare suffix match (forgiving form for hand-written rule
            // sets). Same strip-and-append semantics.
            if lower.ends_with(pattern.as_str()) {
                let head = &word[..word.len() - pattern.len()];
                return alloc::format!("{}{}", head, replacement);
            }
        }
        // No rule matched — return the input unchanged. The boot table
        // ends in an empty default so this branch is unreachable in
        // practice, but the explicit fallback keeps the function total.
        word.to_string()
    }
}

/// Read the `value0..valueN` columns of an `EnumValues` fact whose
/// `noun` binding equals `type_name`. Returns an empty vector when the
/// cell is missing, the type isn't declared, or the fact carries no
/// value bindings. Mirrors the helper of the same name in
/// `parse_forml2_stage2.rs` — duplicated here so `naming.rs` stays
/// self-contained without exporting a stage-2 internal.
fn read_enum_values(state: &crate::ast::Object, type_name: &str) -> Vec<String> {
    let cell = crate::ast::fetch_or_phi("EnumValues", state);
    let facts = match cell.as_seq() {
        Some(s) => s,
        None => return Vec::new(),
    };
    for f in facts.iter() {
        if crate::ast::binding(f, "noun") != Some(type_name) { continue; }
        return (0..)
            .map_while(|i| {
                let key = alloc::format!("value{i}");
                crate::ast::binding(f, &key).map(String::from)
            })
            .collect();
    }
    Vec::new()
}

/// Simple English pluralization for noun names. Dispatches through the
/// boot `PluralizationRuleTable` so behavior matches the cascade rules
/// declared in `readings/core/naming.md`. Callers wanting a
/// reading-driven rule set construct the table via
/// `PluralizationRuleTable::from_grammar_state(state).pluralize(word)`.
pub fn pluralize(word: &str) -> String {
    PluralizationRuleTable::boot().pluralize(word)
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
/// Looks up the noun's `referenceScheme` directly from the `Noun` cell
/// (set by the parser from `Order(.id) is an entity type.`) and returns
/// the matching field value from `fields`. Defaults the field name to
/// `"id"` when the noun declares no scheme. Returns `None` when the noun
/// is unknown or the chosen field is absent / empty in `fields`.
///
/// Mirrors the read pattern in `rmap::EntityCellRouter::id_field_for` —
/// both treat `referenceScheme` as the single source of truth, no
/// lowercase-field-name heuristics.
pub fn resolve_entity_id(
    state: &crate::ast::Object,
    noun_name: &str,
    fields: &hashbrown::HashMap<String, String>,
) -> Option<String> {
    let noun_cell = crate::ast::fetch_or_phi("Noun", state);
    let noun_def = noun_cell.as_seq()?
        .iter()
        .find(|n| crate::ast::binding(n, "name") == Some(noun_name))?;
    let scheme = crate::ast::binding(noun_def, "referenceScheme").unwrap_or("id");
    fields.get(scheme)
        .filter(|v| !v.is_empty())
        .cloned()
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

    // ─── #895 Sweep-1 dispatch-to-data lift: PluralizationRuleTable ──
    //
    // Same shape as `WordComparatorTable` (#783) / `RangeOperatorTable`
    // (#783) / `QuoteEscapeTable` (#844): the legacy inline cascade in
    // `pluralize` lifts to a typed table whose rows live in
    // `readings/core/naming.md` as two parallel enum value types
    // (`Pluralization Pattern` ↔ `Pluralization Replacement`). Boot
    // mirrors the historical cascade order so the existing
    // `test_pluralize` cases round-trip; `from_grammar_state` reads the
    // same data from a parsed reading state and falls back to boot when
    // the cell is empty.

    /// Build a synthetic state carrying parallel enum-value facts in
    /// the `EnumValues` cell — same layout that the parser produces for
    /// `The possible values of <Type> are 'a', 'b', ...` declarations.
    /// Mirrors `parse_forml2_stage2`'s `synthetic_enum_state` helper.
    fn synthetic_enum_state(enums: &[(&str, &[&str])]) -> crate::ast::Object {
        use crate::ast::{Object, fact_from_pairs};
        use alloc::sync::Arc;
        use hashbrown::HashMap as HashbrownMap;
        let facts: Vec<Object> = enums.iter().map(|(noun, vals)| {
            let mut pairs: Vec<(String, String)> = alloc::vec![
                ("noun".to_string(), (*noun).to_string()),
            ];
            for (i, v) in vals.iter().enumerate() {
                pairs.push((alloc::format!("value{i}"), (*v).to_string()));
            }
            let refs: Vec<(&str, &str)> = pairs.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            fact_from_pairs(&refs)
        }).collect();
        let mut map: HashbrownMap<String, Object> = HashbrownMap::new();
        map.insert("EnumValues".to_string(), Object::Seq(Arc::from(facts)));
        Object::Map(map)
    }

    #[test]
    fn pluralization_rule_table_boot_has_rows_in_declared_cascade_order() {
        let table = PluralizationRuleTable::boot();
        // Boot must mirror the legacy cascade so `pluralize` round-trips
        // the original suffix-table behavior. Vowel-y patterns precede
        // the consonant-y catchall; specific es-suffixes precede the
        // bare `s$` rule; the empty pattern is the trailing default.
        let patterns: Vec<&str> = table.rows.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(patterns, vec![
            "^child$", "^person$",        // irregulars (full-word, lowercase)
            "ay$", "ey$", "oy$", "uy$", "iy$", // vowel-y → +s
            "ss$", "sh$", "ch$", "x$", "s$",   // es-suffixes
            "z$",                              // z → zzes
            "y$",                              // consonant-y catchall → ies
            "$",                               // default → +s
        ], "boot table must mirror the legacy pluralize cascade in order");
    }

    #[test]
    fn pluralization_rule_table_pluralize_round_trips_legacy_cases() {
        let table = PluralizationRuleTable::boot();
        // Every case from `test_pluralize` plus the existing
        // noun_to_slug / noun_to_table fixtures.
        assert_eq!(table.pluralize("Organization"), "Organizations");
        assert_eq!(table.pluralize("Status"), "Statuses");
        assert_eq!(table.pluralize("Entity"), "Entities");
        assert_eq!(table.pluralize("Key"), "Keys");
        assert_eq!(table.pluralize("Quiz"), "Quizzes");
        assert_eq!(table.pluralize("Box"), "Boxes");
        assert_eq!(table.pluralize("Match"), "Matches");
        assert_eq!(table.pluralize("Noun"), "Nouns");
        // Irregulars round-trip through the full-word path.
        assert_eq!(table.pluralize("Child"), "Children");
        assert_eq!(table.pluralize("Person"), "People");
    }

    #[test]
    fn pluralization_rule_table_from_grammar_state_reads_parallel_enums() {
        // Synthetic state with custom rules: `^foo$` → `foos`, `$` → `bar`.
        let state = synthetic_enum_state(&[
            ("Pluralization Pattern", &["^foo$", "$"]),
            ("Pluralization Replacement", &["foos", "bar"]),
        ]);
        let table = PluralizationRuleTable::from_grammar_state(&state);
        assert_eq!(table.rows, vec![
            ("^foo$".to_string(), "foos".to_string()),
            ("$".to_string(),     "bar".to_string()),
        ]);
        // The accessor honors the cell-driven rule set: "foo" matches
        // the irregular and yields the literal replacement; everything
        // else falls through to the default and gets +"bar" appended.
        assert_eq!(table.pluralize("foo"), "foos");
        assert_eq!(table.pluralize("zzz"), "zzzbar");
    }

    #[test]
    fn pluralization_rule_table_falls_back_to_boot_on_empty_state() {
        let state = synthetic_enum_state(&[]);
        let table = PluralizationRuleTable::from_grammar_state(&state);
        assert_eq!(table.rows.len(), PluralizationRuleTable::boot().rows.len(),
            "empty grammar state falls back to the boot table verbatim");
        // Behavior is byte-identical to boot — the cell-driven path
        // must be a strict superset of the hardcoded fallback.
        assert_eq!(table.pluralize("Status"), "Statuses");
        assert_eq!(table.pluralize("Entity"), "Entities");
    }
}
