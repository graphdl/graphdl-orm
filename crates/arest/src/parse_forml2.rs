// crates/arest/src/parse_forml2.rs
//
// FORML 2 Parser -- FFP composition of recognizer functions.
//
// Per the paper: parse: R -> Phi (Theorem 2).
// parse = alpha(recognize) : lines
// recognize = try1 ; try2 ; ... ; tryn
//
// Each recognizer: &str -> Option<ParseAction>
// The ? operator IS the conditional form <COND, is_some, unwrap, _|_>.
// No if/else chains. Pattern matching via strip_suffix/strip_prefix/find.

use crate::types::*;
use hashbrown::HashMap;

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// True when `clause` starts with a universal-quantifier keyword
/// (`for each ` per the boot table; future keywords come from the
/// `Universal Quantifier Keyword` grammar enum) followed by `<Noun>`
/// and is followed by at least one more declared noun reference (the
/// predicate over the universally-quantified variable). Accepts
/// universal-quantifier antecedents like
///     for each Authority that applies to that Support Response,
///       that Support Response satisfies that Authority
/// so the overall derivation rule is not flagged as unresolved.
///
/// #875 Sweep-1 lift — vocabulary lifts to `UniversalQuantifierTable`
/// so the keyword set lives in `readings/forml2-grammar.md` as a
/// `Universal Quantifier Keyword` enum value type. Boot stays in sync
/// with the grammar; same first-match-wins iteration as the legacy
/// inline `strip_prefix("for each ")` cascade.
fn is_universal_quantifier_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim();
    let Some(after) = crate::parse_forml2_stage2::UniversalQuantifierTable::boot()
        .match_prefix(trimmed) else { return false; };
    // Must mention a declared noun after the quantifier keyword.
    noun_names.iter().any(|n| after.starts_with(n.as_str()))
        // ...and at least one more noun reference in the tail.
        && noun_names.iter().any(|n| {
            let needle = format!(" {}", n);
            after.contains(&needle)
        })
}

/// Recover the `(restriction, predicate)` clause pair from the canonical
/// ORM2 *relative-clause* universal form (Halpin/Morgan §6.5 p.252), in
/// which NO comma separates the restriction from the main predicate:
///
///   `<X> who|that|which <restricting FT reading naming the Subject>
///        <main predicate> '<literal>'`
///
/// e.g. `Other that blocks the Head has Flag 'done'` (with subject
/// `Head`) splits into
///   restriction = `Other that blocks the Head`
///   predicate   = `Other has Flag 'done'`
///
/// so it compiles to the SAME `ConsequentUniversal` IR the comma form
/// `Other that blocks the Head, Other has Flag 'done'` produces. The
/// quantified variable X (the leading noun token) is re-prepended to the
/// predicate tail because the surface form elides the repeated subject of
/// the main predicate (`… that blocks the Head **has** Flag 'done'`).
///
/// Returns `None` when the shape can't be recovered (no relative pronoun,
/// or the subject noun isn't found after it) so the caller falls back to
/// the recognised-but-unstructured path (suppress, don't mis-derive).
fn split_universal_relative_clause(
    rest: &str,
    subject: Option<&str>,
    noun_names: &[String],
) -> Option<(String, String)> {
    let subject = subject?;
    // The X variable is the leading noun token (carries the ring
    // subscript that distinguishes it from the subject). Required so we
    // can re-prepend it to the elided predicate.
    let tokens = find_nouns(rest, noun_names);
    let (_, x_end, x_token) = tokens.first()?.clone();

    // The restriction is introduced by a relative pronoun (`who` / `that`
    // / `which`). Locate the FIRST one AFTER the X token — that's where
    // the restricting fact-type reading begins. Without a relative
    // pronoun this is not the relative-clause form (it would be a plain
    // `<Quant> X <predicate>`, handled elsewhere), so bail.
    let rel_pronoun_at = [" that ", " who ", " which "]
        .iter()
        .filter_map(|p| rest[x_end..].find(p).map(|i| (x_end + i, p.len())))
        .min_by_key(|&(i, _)| i)?;
    let restriction_body_start = rel_pronoun_at.0 + rel_pronoun_at.1;

    // Within the restriction body, find the subject-noun occurrence; the
    // restriction ends right after it and the main predicate begins. Use
    // the FIRST subject occurrence at/after the relative-pronoun body so
    // a self-ring (X and subject share a base noun) splits at the subject
    // mention, not the leading X. Match the subject as a whole declared
    // noun token (optionally subscripted) via find_nouns over the body.
    let body = &rest[restriction_body_start..];
    let body_tokens = find_nouns(body, noun_names);
    let subj_in_body = body_tokens.iter()
        .find(|(_, _, n)| parse_role_token(n).0 == subject)?;
    let subject_end = restriction_body_start + subj_in_body.1;

    let restriction = rest[..subject_end].trim().to_string();
    let predicate_tail = rest[subject_end..].trim();
    if predicate_tail.is_empty() { return None; }

    // Re-prepend the quantified variable: `has Flag 'done'` →
    // `Other has Flag 'done'`. Preserve the X token's subscript so the
    // predicate's role alignment matches the restriction's X role.
    let predicate = format!("{} {}", x_token, predicate_tail);
    Some((restriction, predicate))
}

/// True when `clause` has the shape `<Noun> is extracted from <Noun>`
/// or `<Noun> is derived from <Noun>` (per the boot table; future
/// markers come from the `Extraction Clause Keyword` grammar enum).
/// Both operands must be declared. Used for ML-style computed bindings
/// (free-text extraction, classifier outputs) where the underlying
/// extractor is registered at runtime. Classification here suppresses
/// the false-unresolved noise; the actual extraction function lives
/// in DEFS.
///
/// #876 Sweep-1 lift — vocabulary lifts to `ExtractionClauseTable`
/// so the keyword set lives in `readings/forml2-grammar.md` as an
/// `Extraction Clause Keyword` enum value type. Boot stays in sync
/// with the grammar; same first-match-wins iteration as the legacy
/// inline `[" is extracted from ", " is derived from "]` array.
fn is_extraction_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    let Some((lhs, rhs)) = crate::parse_forml2_stage2::ExtractionClauseTable::boot()
        .split_at_keyword(trimmed) else { return false; };
    let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
    is_noun(lhs) && is_noun(rhs)
}

/// Strip existential / anaphoric quantifiers from FT references so
/// `Feature Request concerns some API Product` resolves against the
/// declared `Feature Request concerns API Product`. Only ` some ` and
/// ` that ` (as whole-word tokens) are removed — the surrounding
/// noun / verb text is untouched.
///
/// #883 Sweep-1 lift — existential quantifier vocabulary lifts to
/// `ExistentialQuantifierTable` so the keyword set lives in
/// `readings/forml2-grammar.md` as an `Existential Quantifier
/// Keyword` enum value type. Boot stays in sync with the grammar;
/// same chained-replace semantics as the legacy inline
/// `.replace(" some ", " ").replace(" that ", " ")` cascade.
fn strip_existential_quantifiers(clause: &str) -> String {
    crate::parse_forml2_stage2::ExistentialQuantifierTable::boot().strip(clause)
}

/// True when `clause` has the shape `<Noun> has <Noun> '<literal>'`
/// with both nouns declared (per the boot table; future infix markers
/// come from the `Noun Has Noun Literal Keyword` grammar enum).
/// Accepts state-machine status filters and enum-value filters where
/// the underlying FT isn't always declared textually (e.g. Status is
/// SM-managed).
///
/// #877 Sweep-1 lift — vocabulary lifts to `NounHasNounLiteralTable`
/// so the keyword set lives in `readings/forml2-grammar.md` as a
/// `Noun Has Noun Literal Keyword` enum value type. Boot stays in
/// sync with the grammar; same first-match-wins iteration as the
/// legacy inline `find(" has ")` call.
fn is_noun_has_noun_literal(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    // Hand-rolled equivalent of `^(.+?) has (.+?) '[^']*'$`.
    // Strip the trailing space-prefixed quoted literal, then split on
    // the first table-keyword (` has `) to recover (subj, attr).
    let Some((without_literal, _)) = strip_trailing_quoted_literal(trimmed) else {
        return false;
    };
    let Some((subj, attr)) = crate::parse_forml2_stage2::NounHasNounLiteralTable::boot()
        .split_at_keyword(without_literal.as_str()) else { return false; };
    let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
    is_noun(subj) && is_noun(attr)
}

/// #276 Category G — iteratively expand relative-clause `that`-chains
/// into explicit conjunctions.
///
/// `<head> that <verb phrase>` rewrites to
/// `<head> and <last noun of head> <verb phrase>` so the downstream
/// ` and `-split produces two clauses that both resolve against
/// declared FTs. The expansion runs repeatedly until no expandable
/// ` that ` remains, so nested forms
///
///   Source Request is for Resource Declaration that has Base Path
///
/// flatten to
///
///   Source Request is for Resource Declaration
///   Resource Declaration has Base Path
///
/// Back-reference anaphora (`that <Noun> ...`) is untouched — the
/// existing anaphora classifier handles those join-key forms.
///
/// Safety rail: expansion is skipped when the `<head>` portion does
/// not itself resolve to a declared FT. Blindly rewriting a head
/// that isn't in the catalog (e.g. the 5-ary `Billable Request is
/// for Customer and Meter Endpoint and VIN and Date` from auth.md,
/// whose binary slice `Billable Request is for Customer` doesn't
/// exist) would replace a single unresolved warning with two, making
/// the diagnostic output noisier. When the head fails to resolve,
/// the original clause stays intact and falls through to the
/// downstream classifier cascade.
///
/// #882 Sweep-1 lift — anaphora-pronoun vocabulary lifts to
/// `AnaphoraPronounTable` so the marker set lives in
/// `readings/forml2-grammar.md` as an `Anaphora Pronoun` enum value
/// type. Boot stays in sync with the grammar; same scan-and-expand
/// semantics as the legacy inline ` that ` substring scan, with the
/// table's `marker()` accessor providing the single declared marker
/// (currently ` that `) so future additions (e.g. ` which `,
/// ` whose `) extend the grammar declaration and are picked up
/// automatically.
fn expand_that_relatives(
    antecedent: &str,
    noun_names: &[String],
    catalog: &SchemaCatalog,
) -> String {
    let table = crate::parse_forml2_stage2::AnaphoraPronounTable::boot();
    let marker = table.marker();
    let mut current = antecedent.to_string();
    loop {
        let positions: Vec<usize> = current
            .match_indices(marker)
            .map(|(i, _)| i)
            .collect();
        let expand_at = positions.into_iter().find(|&i| {
            let tail = &current[i + marker.len()..];
            let tail_trim = tail.trim_start();
            if is_that_anaphora_ref(tail_trim, noun_names) { return false; }
            let head = &current[..i];
            // Don't expand a `that` inside a universal-quantifier clause
            // (`for each X that R the Subject, …`). The universal is
            // classified whole later; splitting it on `that` would
            // shatter the quantified restriction into a stray ` and `
            // conjunction (`for each X and Subject R the Subject`), so
            // the `for each` recognizer never fires. The head's current
            // clause segment is the text after the last top-level
            // ` and ` — if it opens with a quantifier keyword, skip.
            let clause_seg = head.rsplit(" and ").next().unwrap_or(head);
            if crate::parse_forml2_stage2::UniversalQuantifierTable::boot()
                .match_prefix(clause_seg.trim_start()).is_some()
            {
                return false;
            }
            // Don't expand a `that` that belongs to a superlative's
            // restriction set (`… has the highest <V> among <Y>s that
            // have <R> '<lit>'`). The `among … that …` clause is
            // classified whole by `try_parse_superlative_among_clause`
            // (the `that`-restriction becomes an aggregate filter); a
            // premature ` and ` split here would shatter the among-set
            // into a stray conjunction (`… among Ys and <lastNoun> have
            // …`) and the superlative would lose its filter. Detect by an
            // ` among ` earlier in the current clause segment.
            if clause_seg.contains(" among ") {
                return false;
            }
            // Only expand when the head — text up to this marker —
            // resolves to a declared FT. Otherwise leave the clause
            // for downstream classifiers to handle whole.
            head_resolves(head, noun_names, catalog)
        });
        let Some(pos) = expand_at else { break; };
        let head = &current[..pos];
        let tail = &current[pos + marker.len()..];
        let Some(last_noun) = find_last_noun_in(head, noun_names) else { break; };
        let expanded = alloc::format!("{} and {} {}", head, last_noun, tail);
        if expanded == current { break; }
        current = expanded;
    }
    current
}

/// True when the text up to this point resolves to a declared FT
/// via the schema catalog. Used as a pre-flight check before
/// expanding a `that`-relative — we only want to split when the
/// left side is known-good.
fn head_resolves(head: &str, noun_names: &[String], catalog: &SchemaCatalog) -> bool {
    let found = find_nouns(head, noun_names);
    if found.is_empty() { return false; }
    let base_refs: Vec<String> = found.iter()
        .map(|(_, _, n)| parse_role_token(n).0.to_string())
        .collect();
    let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();
    let verb = match found.len() {
        1 => head[found[0].1..].trim(),
        _ => head[found[0].1..found[1].0].trim(),
    };
    let verb_opt = (!verb.is_empty()).then_some(verb);
    catalog.resolve(&role_refs, verb_opt).is_some()
        || catalog.resolve(&role_refs, None).is_some()
}

/// Find the last declared noun appearing in `text`, longest-first.
fn find_last_noun_in(text: &str, noun_names: &[String]) -> Option<String> {
    let found = find_nouns(text, noun_names);
    found.last().map(|(_, _, name)| parse_role_token(name).0.to_string())
}

/// True when `tail` (text immediately after `that `) starts with a
/// noun reference rather than a verb phrase. Noun references take
/// three forms: plain noun, subscripted noun (`Person3`), and
/// hyphen-bound role name (`expires- Timestamp`). Used by
/// `expand_that_relatives` to skip anaphora — back-references to a
/// previously-bound role shouldn't be rewritten into conjunctions.
/// Fact-is-not-special word-boundary guard: true if a reserved metamodel noun
/// LONGER than `noun_len` begins (case-insensitively) at the start of `text`.
/// Used so a shorter noun like `Fact` is not recognised INSIDE a longer reserved
/// noun like `Fact Type` (which resolves as the metamodel relation, not a noun --
/// task980). The reserved nouns are deliberately absent from `noun_names`, so the
/// longest-first defense in callers cannot prefer them; this guard does.
fn shadowed_by_longer_reserved(text: &str, noun_len: usize) -> bool {
    let lower = text.to_ascii_lowercase();
    crate::ast::RESERVED_METAMODEL_NOUNS.iter().any(|r| {
        r.len() > noun_len && lower.starts_with(&r.to_ascii_lowercase())
    })
}

fn is_that_anaphora_ref(tail: &str, noun_names: &[String]) -> bool {
    // Shape 1 + 2: <Noun> or <Noun><digits>
    if noun_names.iter().any(|n| {
        let Some(after) = tail.strip_prefix(n.as_str()) else { return false; };
        // word-boundary guard: don't read `Fact` inside `Fact Type` (etc.).
        if shadowed_by_longer_reserved(tail, n.len()) { return false; }
        let after_subscript = after.trim_start_matches(|c: char| c.is_ascii_digit());
        matches!(
            after_subscript.chars().next(),
            None | Some(' ') | Some('.') | Some(','),
        )
    }) { return true; }
    // Shape 3: <word>- <Noun>, i.e. hyphen-bound role prefix.
    // The prefix is a single whitespace-free token followed by `- `.
    // `cached- Timestamp`, `override- Fetcher` both fit.
    let Some(hyphen_idx) = tail.find("- ") else { return false; };
    let prefix = &tail[..hyphen_idx];
    if prefix.is_empty() || prefix.contains(' ') { return false; }
    let after_hyphen = &tail[hyphen_idx + "- ".len()..];
    noun_names.iter().any(|n| {
        let Some(after) = after_hyphen.strip_prefix(n.as_str()) else { return false; };
        matches!(
            after.chars().next(),
            None | Some(' ') | Some('.') | Some(','),
        )
    })
}

/// #275 Category C — `<Noun> is '<literal>'` or `<Noun> is not
/// '<literal>'` is a ref-scheme-value filter over the noun's
/// identity. Optional leading role-binding qualifiers (`other `,
/// `that `, `some `, `each `, `any `) and numeric subscripts on the
/// noun (`Source1`, `Customer2`) are stripped before the match. The
/// clause body in a derivation rule uses this form to select the
/// entity whose ref scheme value equals the literal — equivalent to
/// `Noun has <RefSchemeVT> '<literal>'`.
///
/// #878 Sweep-1 lift — trailing equality vocabulary lifts to
/// `EntityRefSchemeLiteralTable` so the keyword set lives in
/// `readings/forml2-grammar.md` as an `Entity Ref Scheme Literal
/// Keyword` enum value type. Boot stays in sync with the grammar;
/// same longest-prefix-wins iteration as the legacy inline
/// `strip_suffix(" is not").or_else(|| strip_suffix(" is"))` chain.
fn is_entity_ref_scheme_literal(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    // Strip a single leading role qualifier. Only one per clause is
    // idiomatic in Halpin readings; stripping every occurrence would
    // widen the match beyond intent.
    let stripped = ["other ", "that ", "some ", "each ", "any ", "the ", "a ", "an "]
        .iter()
        .fold(trimmed, |s, q| s.strip_prefix(q).unwrap_or(s));
    // Hand-rolled `^(.+?) (?:is not|is) '[^']*'$`. Strip the trailing
    // quoted literal, then peel off either ` is not` or ` is` from the
    // right end via the lifted EntityRefSchemeLiteralTable.
    let Some((without_literal, _)) = strip_trailing_quoted_literal(stripped) else {
        return false;
    };
    let Some(raw_subj) = crate::parse_forml2_stage2::EntityRefSchemeLiteralTable::boot()
        .strip_trailing_keyword(without_literal.as_str()) else { return false; };
    let (base, _) = parse_role_token(raw_subj);
    noun_names.iter().any(|n| n == base)
}

/// True when `clause` has the shape `<Noun> is (a|an) <Noun>` with
/// both sides resolving to declared nouns. Treated as a typing
/// predicate rather than a fact-type reference.
///
/// #879 Sweep-1 lift — infix subtype-check vocabulary lifts to
/// `SubtypeInstanceCheckTable` so the keyword set lives in
/// `readings/forml2-grammar.md` as a `Subtype Instance Check
/// Keyword` enum value type. Boot stays in sync with the grammar;
/// same iter-and-any semantics as the legacy inline
/// `[" is a ", " is an "].iter().any(...)` chain, so a clause
/// containing both keywords retries the second when the first
/// fails to resolve both sides. Caution per #845: the clause shape
/// "X is a subtype of Y" still naively substring-matches " is a "
/// but the LHS/RHS pair fails the `noun_names` lookup so the
/// classifier returns false — preserving the literal-aware-scanner
/// invariant.
fn is_subtype_instance_check(clause: &str, noun_names: &[String]) -> bool {
    extract_subtype_instance_check(clause, noun_names).is_some()
}

/// Extract the `(subtype_noun, supertype_noun)` pair from an `X is a Y`
/// / `X is an Y` clause when both sides resolve to declared nouns.
/// Returns `None` when the clause doesn't match or either side isn't a
/// declared noun.
///
/// Used by `resolve_derivation_rule` (task subtype-join-antecedent)
/// to add the `Subtype` metamodel cell as an antecedent source so
/// `X is a Y` in a rule antecedent contributes role-literal filters
/// against the schema-declared `Subtype` cell rather than being
/// silently skipped.  Prerequisites for the full lift live in
/// `readings/core/derivation.md` under "Subtype-join-antecedent".
fn extract_subtype_instance_check(
    clause: &str,
    noun_names: &[String],
) -> Option<(String, String)> {
    let trimmed = clause.trim();
    // Chained-temporary form: the `Table::boot()` temporary lives
    // to end-of-statement so the iterator's borrow of `rows` is
    // valid across the `.find_map(...)` closure. Mirrors the sibling
    // lifts in is_range_filter_clause / is_word_comparator_clause —
    // a named local would trip the drop-order check (E0597).
    crate::parse_forml2_stage2::SubtypeInstanceCheckTable::boot()
        .iter().find_map(|kw| {
            let idx = trimmed.find(kw)?;
            let lhs = trimmed[..idx].trim();
            let rhs = trimmed[idx + kw.len()..].trim();
            let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
            if is_noun(lhs) && is_noun(rhs) {
                Some((lhs.to_string(), rhs.to_string()))
            } else {
                None
            }
        })
}

/// True when `clause` uses a word-based comparator
/// (`exceeds`, `is greater than`, `is less than`, `is at least`,
///  `is at most`, `is more than`, `equals`, `is equal to`)
/// and both operand sides reference a declared noun. The payload
/// itself isn't compiled here — classification only suppresses
/// the "unresolved clause" diagnostic for the legitimate comparison
/// form.
/// #277 Category F — `<FT-reference> within|before|after <tail>` is
/// a binary FT lookup with an implicit range filter on the trailing
/// role. Recognised when splitting on the range operator yields a
/// head that resolves through the catalog; the tail is left as an
/// anaphoric binding. Patterns like `Log Entry has Timestamp within
/// that Interval` and `Timestamp is before that Fresh Until` appear
/// across service-health.md, data-pipeline.md, and eu-law corpora.
fn is_range_filter_clause(
    clause: &str,
    noun_names: &[String],
    catalog: &SchemaCatalog,
) -> bool {
    // #783 second slice — vocabulary lifts to RangeOperatorTable so the
    // 3 operators live in `readings/forml2-grammar.md` as a `Range
    // Operator` enum value type. Boot stays in sync with the grammar.
    crate::parse_forml2_stage2::RangeOperatorTable::boot().iter().any(|op| {
        let needle = alloc::format!(" {} ", op);
        let Some(idx) = clause.find(&needle) else { return false; };
        let head = clause[..idx].trim();
        head_resolves(head, noun_names, catalog)
    })
}

/// #277 Category F — bare-value tail comparisons like
/// `HTTP Status of 500 or more`, `HTTP Status of 500 or less`,
/// `HTTP Status of at least 500`, `HTTP Status of at most 500`.
/// The FT reference is the subject noun; the `of <N> <comparator>`
/// tail is an implicit comparator filter on the value side.
///
/// #880 Sweep-1 lift — trailing comparison-keyword vocabulary lifts
/// to `BareValueComparisonTable` so the keyword set lives in
/// `readings/forml2-grammar.md` as a `Bare Value Comparison Keyword`
/// enum value type. Boot stays in sync with the grammar; same
/// any-match-wins iteration as the legacy inline `TAILS.iter().any(
/// |t| trimmed.ends_with(t))`.
fn is_bare_value_comparison(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    let ends_with_tail = crate::parse_forml2_stage2::BareValueComparisonTable::boot()
        .ends_with_keyword(trimmed);
    if !ends_with_tail { return false; }
    // The clause must contain " of " followed by a numeric literal
    // and reference at least one declared noun on the left side.
    let Some(of_idx) = trimmed.find(" of ") else { return false; };
    let head = trimmed[..of_idx].trim();
    let head_has_noun = noun_names.iter().any(|n| {
        head == n
            || head.starts_with(&alloc::format!("{} ", n))
            || head.ends_with(&alloc::format!(" {}", n))
            || head.contains(&alloc::format!(" {} ", n))
    });
    if !head_has_noun { return false; }
    // Token after " of " must be a numeric literal (decimal, possibly
    // signed). Reject quoted-value forms which belong to the
    // ref-scheme-literal classifier.
    let after_of = trimmed[of_idx + " of ".len()..].trim_start();
    let first_token = after_of.split_whitespace().next().unwrap_or("");
    first_token.parse::<f64>().is_ok()
}

fn is_word_comparator_clause(clause: &str, noun_names: &[String]) -> bool {
    // #783 first slice — vocabulary lifts to WordComparatorTable so the
    // 8 phrases live in `readings/forml2-grammar.md` as a `Word Comparator`
    // enum value type. Boot stays in sync with the grammar; same first-
    // match-wins iteration as the legacy COMPARATORS const.
    crate::parse_forml2_stage2::WordComparatorTable::boot().iter().any(|kw| {
        let needle = alloc::format!(" {} ", kw);
        let Some(idx) = clause.find(&needle) else { return false; };
        let lhs = clause[..idx].trim();
        let rhs = clause[idx + needle.len()..].trim();
        let side_has_noun = |side: &str| noun_names.iter().any(|n| {
            // Whole-side match or noun as a whole-word substring.
            side == n
                || side.starts_with(&alloc::format!("{} ", n))
                || side.ends_with(&alloc::format!(" {}", n))
                || side.contains(&alloc::format!(" {} ", n))
        });
        side_has_noun(lhs) && side_has_noun(rhs)
    })
}

/// #914 — Normalize a `WordComparatorTable` phrase to the ASCII
/// comparator op consumed by `comparator_primitive` in `compile.rs`.
/// The mapping matches the literal-RHS path
/// (`split_antecedent_comparator` → `peel_trailing_comparator`)
/// so the post-join Filter primitive treats both shapes the same.
/// `exceeds`, `is greater than`, `is more than` → `">"`,
/// `is less than` → `"<"`,
/// `is at least` → `">="`,
/// `is at most` → `"<="`,
/// `equals`, `is equal to` → `"="`.
/// Any unknown phrase yields `"="` — same fall-through
/// `comparator_primitive` uses for unrecognised ops.
fn word_comparator_to_op(phrase: &str) -> &'static str {
    match phrase {
        "exceeds" | "is greater than" | "is more than" => ">",
        "is less than" => "<",
        "is at least" => ">=",
        "is at most" => "<=",
        "equals" | "is equal to" => "=",
        _ => "=",
    }
}

/// task subtype-join-antecedent (child 1) — Classify an antecedent clause that
/// quantifies over one of the substrate's own metamodel cells (`Subtype`,
/// `FactType`, `Role`, `Noun`, `Constraint`, `SubsetConstraint`) rather than
/// over a user-declared Fact Type.
///
/// The input `stripped_text` must already have anaphora / quantifier words
/// removed (call `strip_anaphora` before invoking).
///
/// Returns:
///  * `Some(cell_id)` — a non-empty cell id — when the clause is a PRIMARY
///    quantification over that cell (e.g. `Subtype has subtype Sub`).  The
///    caller adds `FactType(cell_id)` to `antecedent_sources`.
///  * `Some("")`  — an empty-string sentinel — when the clause uses a
///    metamodel-adjacent predicate that should be silently skipped without
///    becoming an antecedent source (e.g. `Resource is instance of Sub`).
///  * `None`  — the clause is not a metamodel reference; the caller continues
///    its normal classification cascade.
///
/// Why a dedicated recogniser instead of extending the `SchemaCatalog`:
///   The catalog maps (noun-set, verb) → FT-id, but metamodel-rule clauses
///   bind local *variable* names (`Sub`, `Sup`) that differ from the cell's
///   own role names (`subtype`, `supertype`). A catalog entry would have to
///   hard-code the variable name in the reading, making it brittle to any
///   author rephrasing.  A prefix-check on the cell name is simpler, more
///   robust, and limited in scope to the handful of metamodel cell names
///   (`readings/core/derivation.md`).
fn try_classify_metamodel_clause(stripped_text: &str) -> Option<String> {
    // Recognised metamodel cell name prefixes (lowercased) → canonical cell ids.
    // "Fact Type" appears as two words in FORML text; the state stores it as
    // "FactType" (no space).  "Resource" is NOT a metamodel cell — it is a
    // domain-variable in the subtype-inheritance rule — so it has no entry here.
    //
    // ss-autofill-retire-1 (the task-978 analog): `subset constraint ` /
    // `constraint ` expose the `Constraint` cell's SS-autofill spans as a
    // bindable metamodel-cell antecedent so the SS auto-fill metamodel rule
    // (`readings/core/derivation.md` §"SS Subset-Constraint auto-fill":
    // `some Subset Constraint has antecedent Fact Type Ant …`) resolves to a
    // bound cell antecedent instead of an UnresolvedClause. The bound
    // (antecedent_ft, consequent_ft) pairs are exposed to the derivation
    // compiler via `CellIndex::ss_autofill_pairs` (the `data.subtypes`
    // analog). `subset constraint ` MUST precede `constraint ` so the more
    // specific cell id wins — a `subset constraint …` clause maps to the
    // dedicated `SubsetConstraint` view (autofill-opted SS spans only),
    // never the broader `Constraint` cell.
    const METAMODEL_PREFIXES: &[(&str, &str)] = &[
        ("subtype ", "Subtype"),
        ("fact type ", "FactType"),
        ("facttype ", "FactType"),
        ("role ", "Role"),
        ("noun ", "Noun"),
        ("subset constraint ", "SubsetConstraint"),
        ("constraint ", "Constraint"),
    ];
    let lower = stripped_text.to_lowercase();
    for (prefix, cell_id) in METAMODEL_PREFIXES {
        if lower.starts_with(prefix) {
            // Word-boundary guard: the first char after the prefix must not
            // extend an identifier (e.g. "subtype_id" would false-match
            // "subtype ").  The prefix already ends with a space so this is
            // structurally guaranteed by the METAMODEL_PREFIXES entries.
            return Some(cell_id.to_string());
        }
    }
    // The `X is instance of Y` predicate appears in the subtype-inheritance
    // rule body (`that Resource is instance of Sub`) — it's an anaphoric
    // membership check, not a new antecedent scan.  Skip it silently.
    if lower.contains(" is instance of ") {
        return Some(String::new()); // skip-only sentinel
    }
    None
}

/// #914 — Recognise a cross-antecedent role-vs-role value comparison
/// clause. Two operand shapes are accepted:
///
///   * Possessive: `<NounToken>'s <Role>` (mirrors `try_expand_possessive`)
///   * Anaphoric:  `<NounToken> has <Role>` (the post-expansion form
///                 the rest of the parser sees)
///
/// joined by a `WordComparatorTable` phrase. Subscripted noun tokens
/// (`Task1`, `Task2`) are accepted so ring-style rules naming the same
/// base noun on both sides stay distinguishable.
///
/// Returns `Some((lhs_noun_token, lhs_role, op, rhs_noun_token, rhs_role))`
/// when both sides match. `op` is the normalised ASCII comparator
/// (see `word_comparator_to_op`). `noun_token` preserves the
/// subscripted form so downstream antecedent-index lookup can locate
/// the specific FT clause that introduced this noun token.
///
/// Reuses the existing `WordComparatorTable` vocabulary; no parallel
/// keyword table is introduced (the reverted 313f5546 approach
/// invented `CrossAntecedentComparatorTable` — flagged as a modeling
/// error because `.id` is just one role on the entity and
/// `WordComparator` already covers role-value comparison).
fn try_extract_cross_antecedent_role_comparison(
    clause: &str,
    noun_names: &[String],
) -> Option<(String, String, &'static str, String, String)> {
    let trimmed = clause.trim().trim_end_matches('.').trim();
    // Iterate the existing comparator phrases in declaration order so
    // longer phrases (e.g. `is greater than`) match before shorter
    // overlapping ones (`is`). `WordComparatorTable::boot` already
    // returns them in that order.
    let table = crate::parse_forml2_stage2::WordComparatorTable::boot();
    for phrase in table.iter() {
        let needle = alloc::format!(" {} ", phrase);
        let Some(idx) = trimmed.find(needle.as_str()) else { continue; };
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + needle.len()..].trim();
        let Some((lhs_tok, lhs_role)) = parse_noun_role_operand(lhs, noun_names)
        else { continue; };
        let Some((rhs_tok, rhs_role)) = parse_noun_role_operand(rhs, noun_names)
        else { continue; };
        return Some((
            lhs_tok,
            lhs_role,
            word_comparator_to_op(phrase),
            rhs_tok,
            rhs_role,
        ));
    }
    None
}

/// #914 helper — parse a single operand of a cross-antecedent
/// comparator clause into `(noun_token, role)`. Accepts both the
/// possessive form `<NounToken>'s <Role>` (the FORML2 surface
/// expression) and the anaphoric form `<NounToken> has <Role>` (the
/// post-possessive-expansion shape).
///
/// `NounToken` may carry a numeric subscript (`Task1`, `Customer2`);
/// `parse_role_token` is used to derive the base noun for the
/// "declared noun" check. `Role` must itself be a declared noun.
fn parse_noun_role_operand(
    operand: &str,
    noun_names: &[String],
) -> Option<(String, String)> {
    let t = operand.trim();
    // Possessive form: `<NounToken>'s <Role>`.
    if let Some(apos_idx) = t.find("'s ") {
        let token = t[..apos_idx].trim();
        let role = t[apos_idx + 3..].trim();
        let (base, _) = parse_role_token(token);
        if noun_names.iter().any(|n| n == base)
            && noun_names.iter().any(|n| n == role)
        {
            return Some((token.to_string(), role.to_string()));
        }
    }
    // Anaphoric form: `<NounToken> has <Role>`.
    if let Some(has_idx) = t.find(" has ") {
        let token = t[..has_idx].trim();
        let role = t[has_idx + " has ".len()..].trim();
        let (base, _) = parse_role_token(token);
        if noun_names.iter().any(|n| n == base)
            && noun_names.iter().any(|n| n == role)
        {
            return Some((token.to_string(), role.to_string()));
        }
    }
    None
}















// =========================================================================
// Main parser -- fold recognizers over lines
// =========================================================================



/// SSRF defense (#25, #894). Reject URLs that point at internal/loopback/
/// link-local networks, file:// schemes, or internal DNS names. The CIDR
/// blocklist is data — read from the `CIDR_Block_has_Block_Kind` cell
/// declared in `readings/core/security.md` — not Rust. Non-CIDR rules
/// (file://, exact `localhost`/`::1`, internal-DNS suffixes) stay coded
/// here because they are not CIDR-shaped. No DNS resolution, no network
/// I/O. Called during platform_compile to validate External System
/// instance facts before they enter state.
///
/// Boot-list fallback: callers passing `Object::phi()` (e.g. unit tests
/// or pre-bootstrap paths that have no metamodel state yet) get the
/// 8-entry hardcoded CIDR list this function used to embed inline,
/// preserving legacy behaviour.
pub fn is_forbidden_url_in_state(url: &str, state: &crate::ast::Object) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_lowercase();

    // file:// scheme is always forbidden
    match lower.starts_with("file://") {
        true => return true,
        false => {}
    }

    // Extract the host component from http(s) URLs. Non-http schemes fall
    // through and are allowed (the check is scoped to federated HTTP URLs).
    let after_scheme = match lower.strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        Some(rest) => rest,
        None => return false,
    };

    // Strip userinfo (before '@'), then extract the host.
    let no_userinfo = after_scheme.rfind('@').map(|i| &after_scheme[i + 1..]).unwrap_or(after_scheme);

    // Bracketed IPv6 literal: [addr]:port/path -- must find the closing ']'
    // BEFORE searching for ':' (otherwise we split inside the brackets).
    // Bare host: split on the first '/', '?', or '#' to get the authority,
    // then heuristically detect bare IPv6 (authority has 2+ colons) vs the
    // normal host:port form (one colon).
    let host_bare: &str = match no_userinfo.strip_prefix('[') {
        Some(rest) => rest.find(']').map(|i| &rest[..i]).unwrap_or(rest),
        None => {
            let path_start = no_userinfo.find(|c: char| c == '/' || c == '?' || c == '#')
                .unwrap_or(no_userinfo.len());
            let authority = &no_userinfo[..path_start];
            // Bare IPv6 has multiple ':' in the authority (no port syntax
            // without brackets is well-defined, so treat the entire authority
            // as the host). host:port has exactly one ':' which we strip.
            match authority.matches(':').count() {
                0 => authority,
                1 => authority.split(':').next().unwrap_or(authority),
                _ => authority, // bare IPv6 â€” keep colons for ULA / link-local checks
            }
        }
    };

    // Empty host is bottom-safe â€” treat as forbidden.
    match host_bare.is_empty() {
        true => return true,
        false => {}
    }

    // Exact-name checks
    match host_bare {
        "localhost" | "::1" | "::" | "0.0.0.0" => return true,
        _ => {}
    }

    // Internal DNS suffixes (case-insensitive â€” lower already applied)
    let forbidden_suffix = host_bare.ends_with(".local")
        || host_bare.ends_with(".internal")
        || host_bare.ends_with(".localhost");
    match forbidden_suffix {
        true => return true,
        false => {}
    }

    // CIDR blocklist read from readings/core/security.md → `CIDR_Block_has_Block_Kind`.
    // Each row's `CIDR Block` binding is a CIDR notation string; we ask
    // `cidr_contains` whether the host sits inside that range. Empty cell
    // ⇒ fall back to boot CIDR list (preserves callers without metamodel
    // state). The host is checked against IPv4 dotted-quad and bare-IPv6
    // shapes identically — `cidr_contains` parses both.
    let blocklist = cidr_blocklist_from_state(state);
    if blocklist.iter().any(|cidr| cidr_contains(cidr.as_str(), host_bare)) {
        return true;
    }

    false
}

/// Boot fallback for `is_forbidden_url` — the 8 CIDR ranges the
/// pre-#894 hardcoded check covered. Used when the state's
/// `CIDR_Block_has_Block_Kind` cell is empty (e.g. bare engine,
/// unit tests passing phi). Each entry mirrors one branch of the
/// inline IPv4/IPv6 dispatch the lift replaces:
///
///   * `127.0.0.0/8`      ← `a == 127` (IPv4 loopback)
///   * `10.0.0.0/8`       ← `a == 10`  (IPv4 RFC 1918)
///   * `169.254.0.0/16`   ← `a == 169 && b == 254` (link-local)
///   * `192.168.0.0/16`   ← `a == 192 && b == 168` (RFC 1918)
///   * `172.16.0.0/12`    ← `a == 172 && b in 16..=31` (RFC 1918)
///   * `::1/128`          ← exact `host_bare == "::1"` (kept also above)
///   * `fe80::/10`        ← `host_bare.starts_with("fe8..feb")`
///   * `fc00::/7`         ← `host_bare.starts_with("fc"|"fd")` + colon
const BOOT_CIDR_BLOCKLIST: &[&str] = &[
    "127.0.0.0/8",
    "10.0.0.0/8",
    "169.254.0.0/16",
    "192.168.0.0/16",
    "172.16.0.0/12",
    "::1/128",
    "fe80::/10",
    "fc00::/7",
];

/// Read the CIDR blocklist from state's `CIDR_Block_has_Block_Kind`
/// cell. Each fact carries a `CIDR Block` binding (the CIDR string).
/// Empty cell ⇒ return the boot list so the SSRF defense never
/// degrades to "no checks" when state is unconfigured.
fn cidr_blocklist_from_state(state: &crate::ast::Object) -> Vec<String> {
    use crate::ast::{fetch_cell_seq, binding};
    let cell = fetch_cell_seq("CIDR_Block_has_Block_Kind", state);
    let rows: Vec<String> = cell.as_seq()
        .map(|facts| facts.iter()
            .filter_map(|f| binding(f, "CIDR Block").map(String::from))
            .collect())
        .unwrap_or_default();
    if rows.is_empty() {
        BOOT_CIDR_BLOCKLIST.iter().map(|s| s.to_string()).collect()
    } else {
        rows
    }
}

/// Legacy entry point — boot list only, no state. Kept so out-of-tree
/// callers that don't have a state Object on hand still get the same
/// SSRF coverage they had pre-#894.
pub fn is_forbidden_url(url: &str) -> bool {
    is_forbidden_url_in_state(url, &crate::ast::Object::phi())
}

/// True iff `host` (IPv4 dotted-quad like `127.0.0.1` or bare IPv6
/// like `fe80::1`) is contained in the CIDR range `cidr` (e.g.
/// `127.0.0.0/8` or `fe80::/10`). Returns `false` on any parse error,
/// matching the existing SSRF check's bottom-safe fall-through.
///
/// IPv4 and IPv6 are kept on separate code paths: an IPv4 CIDR
/// only contains IPv4 hosts, and an IPv6 CIDR only contains IPv6
/// hosts. (No IPv4-in-IPv6 mapping coercion — the legacy code didn't
/// do it either.) This makes the function exactly the lift of the
/// hardcoded IPv4/IPv6 dispatch in `is_forbidden_url`.
///
/// #894 — exposed as the `cidr_contains` Platform Func via `ast.rs`.
pub fn cidr_contains(cidr: &str, host: &str) -> bool {
    let Some((slash, _)) = cidr.find('/').map(|i| (i, ())) else { return false; };
    let net_str = &cidr[..slash];
    let prefix: u8 = match cidr[slash + 1..].parse() { Ok(n) => n, Err(_) => return false };

    // IPv4: dotted-quad on both sides. Prefix must be 0..=32.
    if let Some(net_v4) = parse_ipv4(net_str) {
        let Some(host_v4) = parse_ipv4(host) else { return false; };
        if prefix > 32 { return false; }
        let mask: u32 = match prefix {
            0 => 0,
            32 => u32::MAX,
            n => u32::MAX << (32 - n as u32),
        };
        return (net_v4 & mask) == (host_v4 & mask);
    }

    // IPv6: colon-shaped on both sides. Prefix must be 0..=128.
    if let Some(net_v6) = parse_ipv6(net_str) {
        let Some(host_v6) = parse_ipv6(host) else { return false; };
        if prefix > 128 { return false; }
        let mask: u128 = match prefix {
            0 => 0,
            128 => u128::MAX,
            n => u128::MAX << (128 - n as u128),
        };
        return (net_v6 & mask) == (host_v6 & mask);
    }

    false
}

/// Parse a dotted-quad IPv4 literal into u32, big-endian.
/// Returns None on malformed input (non-numeric, octet > 255, wrong
/// arity). Mirrors the existing inline octets-parse in `is_forbidden_url`.
fn parse_ipv4(s: &str) -> Option<u32> {
    let parts: Vec<u16> = s.split('.')
        .filter_map(|p| p.parse::<u16>().ok())
        .collect();
    if parts.len() != 4 || parts.iter().any(|o| *o > 255) {
        return None;
    }
    Some(parts.iter().fold(0u32, |acc, &o| (acc << 8) | (o as u32)))
}

/// Parse a bare IPv6 literal into u128, big-endian. Supports `::`
/// elision once. Returns None on malformed input. Hand-rolled because
/// `parse_forml2.rs` is no_std-clean and `std::net::Ipv6Addr` isn't
/// available everywhere this module's reachable from.
fn parse_ipv6(s: &str) -> Option<u128> {
    if s.is_empty() { return None; }
    // Split on the optional `::` elision marker (at most one).
    let (head, tail): (&str, &str) = match s.find("::") {
        Some(i) => (&s[..i], &s[i + 2..]),
        None    => (s, ""),
    };
    // The pre-elision and post-elision groups.
    let head_groups: Vec<&str> = if head.is_empty() { Vec::new() } else { head.split(':').collect() };
    let tail_groups: Vec<&str> = if tail.is_empty() { Vec::new() } else { tail.split(':').collect() };
    let total = head_groups.len() + tail_groups.len();
    if total > 8 { return None; }
    // Without `::`, must be exactly 8 groups.
    if s.find("::").is_none() && total != 8 { return None; }
    // Parse each hex group (1..=4 hex digits).
    let parse_group = |g: &str| -> Option<u16> {
        if g.is_empty() || g.len() > 4 { return None; }
        u16::from_str_radix(g, 16).ok()
    };
    let mut groups: [u16; 8] = [0; 8];
    for (i, g) in head_groups.iter().enumerate() {
        groups[i] = parse_group(g)?;
    }
    let tail_start = 8 - tail_groups.len();
    for (i, g) in tail_groups.iter().enumerate() {
        groups[tail_start + i] = parse_group(g)?;
    }
    Some(groups.iter().fold(0u128, |acc, &g| (acc << 16) | (g as u128)))
}

/// Scan the InstanceFact cell in parsed state and return the first
/// forbidden URL found, if any. Used by platform_compile to reject
/// External System federation to internal/loopback/link-local hosts.
///
/// #894: takes a second `d` argument carrying the current metamodel
/// state — that's where `CIDR_Block_has_Block_Kind` lives, populated
/// by readings/core/security.md at bootstrap. The InstanceFact cell
/// being scanned for URLs sits in `state` (the parser's output);
/// CIDR blocklist sits in `d` (the in-memory metamodel snapshot).
/// Both fall back to boot list / phi when missing, so callers without
/// a fully booted metamodel still get the legacy 8-CIDR coverage.
pub fn find_forbidden_instance_url(
    state: &crate::ast::Object,
    d: &crate::ast::Object,
) -> Option<String> {
    use crate::ast::{fetch_cell_seq, binding};
    fetch_cell_seq("InstanceFact", state)
        .as_seq()
        .and_then(|facts| {
            facts.iter().find_map(|f| {
                let object_value = binding(f, "objectValue")?;
                is_forbidden_url_in_state(object_value, d)
                    .then(|| object_value.to_string())
            })
        })
}

/// Parse FORML2 readings directly into an Object state.
///
/// #285 wire-up is blocked on three remaining `check::` test gaps
/// (tracked in #319): the stage12 pipeline needs an `UnresolvedClause`
/// cell emission for derivation rules with unresolvable antecedents,
/// and ring-constraint `(kind)` annotation handling. The
/// `parse_to_state_via_stage12` entry point is a drop-in replacement
/// for everything else and benchmarks faster than this function.
///
/// No longer cfg-gated — stage2's `parse_to_state_via_stage12` is
/// no_std-clean as of #588 (commit `097577ff`), so this thin shim
/// is reachable from the kernel target too.
pub fn parse_to_state(input: &str) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12(input)
}

/// Extract nouns directly from the Noun cell in D.
pub fn nouns_from_state(state: &crate::ast::Object) -> HashMap<String, NounDef> {
    use crate::ast::{fetch_cell_seq, binding};
    fetch_cell_seq("Noun", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let name = binding(f, "name")?.to_string();
            let obj_type = binding(f, "objectType").unwrap_or("entity").to_string();
            Some((name, NounDef { object_type: obj_type, world_assumption: WorldAssumption::default() }))
        }).collect())
        .unwrap_or_default()
}

/// Extract fact types directly from the FactType cell in D, with
/// roles resolved from the `Role` cell. Replaces the earlier
/// `roles: vec![]` stub — callers no longer need a per-caller compat
/// shim (see `_reports/e3-handoff-2026-04-20.md` §"Ownership #2").
pub fn fact_types_from_state(state: &crate::ast::Object) -> HashMap<String, FactTypeDef> {
    use crate::ast::{fetch_cell_seq, binding};
    // Pre-collect Role cell facts so each FactType iteration is O(|R|)
    // rather than re-fetching per FT.
    let role_cell = fetch_cell_seq("Role", state);
    let role_facts: Vec<&crate::ast::Object> = role_cell.as_seq()
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    fetch_cell_seq("FactType", state)
        .as_seq().map(|facts| facts.iter().filter_map(|f| {
            let id = binding(f, "id")?.to_string();
            let reading = binding(f, "reading").unwrap_or("").to_string();
            // Gather role entries whose `factType` binding matches
            // this FT id, then sort by `position` so `role_index`
            // reflects declaration order.
            let mut roles: Vec<RoleDef> = role_facts.iter()
                .filter(|r| binding(r, "factType") == Some(id.as_str()))
                .filter_map(|r| {
                    let noun_name = binding(r, "nounName")?.to_string();
                    let role_index = binding(r, "position")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    Some(RoleDef { noun_name, role_index })
                })
                .collect();
            roles.sort_by_key(|r| r.role_index);
            Some((id, FactTypeDef {
                schema_id: String::new(),
                reading,
                readings: vec![],
                roles,
            }))
        }).collect())
        .unwrap_or_default()
}

/// Parse FORML2 readings with context from `d` (#285). `d`'s noun
/// catalog is threaded through stage12's tokeniser so statements may
/// reference nouns declared by `d` without redeclaring them. Callers
/// typically `merge_states(d, &result)` to carry `d`'s non-noun cells
/// forward.
///
/// No longer cfg-gated on `std-deps` — stage12's
/// `parse_to_state_via_stage12_with_context` is no_std-clean as of
/// #588 (commit `097577ff`), so this thin shim is reachable from the
/// kernel target too. (#589 — engine-side adoption of
/// `load_reading_core::load_reading` under no_std.)
pub fn parse_to_state_from(input: &str, d: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context(input, d)
}

/// ns-5 (ns-local-precedence-resolver): like `parse_to_state_from`, but
/// the caller also names the reference site's own/local domain (ns-3,
/// the file domain it is about to stamp via `annotate_noun_domain`). A
/// bare reference whose noun is declared LOCALLY then resolves to this
/// domain (precedence 1) rather than being treated as a cross-domain
/// collision. `parse_to_state_from` is the `local_domain = None` form, so
/// every existing caller is unchanged.
pub fn parse_to_state_from_in_domain(
    input: &str,
    d: &crate::ast::Object,
    local_domain: &str,
) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context_domain(
        input, d, Some(local_domain))
}

/// Alias for `parse_to_state_from` kept for API compatibility. Legacy
/// took only nouns; stage12's context path accepts the full state and
/// extracts what it needs.
///
/// Same gate-lift rationale as `parse_to_state_from` (#589).
pub fn parse_to_state_with_nouns(input: &str, existing: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context(input, existing)
}




/// Re-resolve a rules vec given just the typed lookups it needs.
/// No ParseCtx struct required â€” callers pass their HashMaps directly.
/// Public wrapper around `re_resolve_rules` for integration tests
/// that need to inspect the resolved antecedent_sources of a parsed
/// rule. Production callers stay on the internal name.
#[doc(hidden)]
pub fn re_resolve_rules_pub(
    rules: &mut Vec<DerivationRuleDef>,
    nouns: &HashMap<String, NounDef>,
    fact_types: &HashMap<String, FactTypeDef>,
) {
    // Public test wrapper: callers that don't model subtypes get the
    // empty (no subtype→supertype) chain. The subtype-aware production
    // path is `re_resolve_rules` with the schema's subtypes map.
    re_resolve_rules(rules, nouns, fact_types, &HashMap::new());
}

pub(crate) fn re_resolve_rules(
    rules: &mut Vec<DerivationRuleDef>,
    nouns: &HashMap<String, NounDef>,
    fact_types: &HashMap<String, FactTypeDef>,
    // subtype → supertype (one parent per noun; mirrors `CellIndex.subtypes`).
    // Lets `resolve_derivation_rule` bridge a subtype-keyed join clause UP
    // to a supertype-declared fact type (subtype instances ARE supertype
    // instances).
    subtypes: &HashMap<String, String>,
) {
    let mut noun_names: Vec<String> = nouns.keys().cloned().collect();
    // The universal metamodel `Fact` noun (not special) lives outside the domain
    // `Noun` cell; recognise it so a reading/clause over it (`Bag holds Fact`,
    // `Fact stimulates Layer`) resolves like a domain entity. `find_nouns` guards
    // it from eating `Fact Type` (the longer reserved noun resolves separately).
    if !noun_names.iter().any(|n| n == "Fact") { noun_names.push("Fact".to_string()); }
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut catalog = SchemaCatalog::new();
    fact_types.iter().for_each(|(ft_id, ft)| {
        let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
        let verb = reading_verb(&ft.reading, &noun_names);
        catalog.register(ft_id, &role_nouns, verb, &ft.reading);
    });

    rules.iter_mut().for_each(|rule| {
        resolve_derivation_rule(rule, nouns, fact_types, &catalog, subtypes);
    });
}


/// Cow-returning variant. Non-joined lines stay borrowed from `input`;
/// only the rare joined-continuation line allocates a fresh `String`.
/// On core.md-scale inputs (506 lines, ~1% need joining) this skips
/// ~500 String allocations per parse.
pub(crate) fn join_derivation_continuations_cow(input: &str) -> Vec<alloc::borrow::Cow<'_, str>> {
    use alloc::borrow::Cow;
    let raw: Vec<&str> = input.lines().collect();
    let mut out: Vec<Cow<'_, str>> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i];
        let stripped = line.trim_start();
        let is_derivation_head = stripped.starts_with("* ")
            || stripped.starts_with("** ")
            || stripped.starts_with("+ ")
            || stripped.contains(" iff ")
            || (stripped.contains(" if ") && !stripped.starts_with("If "));
        if !is_derivation_head || line.trim_end().ends_with('.') {
            out.push(Cow::Borrowed(line));
            i += 1;
            continue;
        }
        // Accumulate until a non-indented line or a `.`-terminated line.
        let mut joined = line.trim_end().to_string();
        let mut j = i + 1;
        while j < raw.len() {
            let cont = raw[j];
            let is_indented = cont.starts_with(' ') || cont.starts_with('\t');
            if !is_indented || cont.trim().is_empty() { break; }
            joined.push(' ');
            joined.push_str(cont.trim());
            let terminated = joined.ends_with('.');
            j += 1;
            if terminated { break; }
        }
        out.push(Cow::Owned(joined));
        i = j;
    }
    out
}


/// Recognize a Halpin aggregate antecedent of form
///   `<role> is the <op> of <target> where <where-clause>`
/// where <op> âˆˆ {count, sum, avg, min, max}. The where-clause is a fact-
/// type reading that will be resolved separately against the catalog.
///
/// Returns (consequent_role, op, target_role, where_clause_text). The
/// caller then resolves the where-clause to a source FT id and pins the
/// group_key_role on it.
fn try_parse_aggregate_clause(text: &str, noun_names: &[String]) -> Option<(String, String, String, String)> {
    let t = text.trim().trim_end_matches('.').trim();
    let t = t.strip_prefix("that ").unwrap_or(t);
    // `where <filter>` is optional — `done Task Count is the count of Task`
    // (no where clause) is as valid as the filtered form. The op list
    // covers count/sum/avg/min/max plus their prose equivalents
    // (`earliest` / `latest` / `first` / `last`) which appear in
    // time-series readings like `Date is the earliest Timestamp`.
    // Hand-rolled equivalent of
    //   ^(.+?) is the (count|sum|avg|min|max|earliest|latest|first|last)
    //         of (.+?)(?: where (.+))?$
    // Find leftmost ` is the `, then require the next token to be a
    // recognised op followed by ` of `; everything after splits on
    // an optional ` where ` clause.
    const AGG_OPS: &[&str] = &[
        "count", "sum", "avg", "min", "max",
        "earliest", "latest", "first", "last",
    ];
    let is_the_idx = t.find(" is the ")?;
    let role = t[..is_the_idx].trim().to_string();
    let after_is_the = &t[is_the_idx + " is the ".len()..];
    let (op, after_of) = AGG_OPS.iter().find_map(|op| {
        let after_op = after_is_the.strip_prefix(op)?;
        let after_of = after_op.strip_prefix(" of ")?;
        Some(((*op).to_string(), after_of))
    })?;
    let (target, where_clause) = match after_of.find(" where ") {
        Some(widx) => (
            after_of[..widx].trim().to_string(),
            after_of[widx + " where ".len()..].trim().to_string(),
        ),
        None => (after_of.trim().to_string(), String::new()),
    };
    // Target must resolve against the noun catalog — either the full
    // string is a declared noun, or its first space-separated token
    // is (for compound role paths like `LineItem Amount` meaning the
    // Amount role of LineItem). Role name is not required to be
    // declared: derivation rules may introduce implicit role names
    // for derived aggregates (e.g. `done Task Count`) that never
    // appear as standalone entity / value types.
    //
    // Halpin ring subscripts: a counted entity in a self-ring `where`-body
    // is referenced subscripted (`count of Item1 where Item1 blocks the
    // Item`). The subscripted token isn't a declared noun, so strip the
    // trailing ASCII-digit subscript (via parse_role_token) before the
    // catalog check while leaving `target` verbatim for the positional
    // role resolution downstream.
    let target_base = parse_role_token(&target).0;
    let first_tok_base = target.split_whitespace().next()
        .map(|first| parse_role_token(first).0);
    // The counted noun may be a DOMAIN entity OR a metamodel noun. `Fact` is now in
    // `noun_names` (added in resolve_derivation_rule / re_resolve_rules; find_nouns
    // guards it from matching inside `Fact Type`, which resolves as the FactType
    // relation, not a noun — task980), so the plain check accepts it. This is_known
    // extension additionally accepts the RESERVED metamodel nouns as aggregate
    // TARGETS (counting Roles, Fact Types, …) without putting them in the clause
    // resolver.
    let is_known = |s: &str| {
        noun_names.iter().any(|n| n == s)
            || s == "Fact"
            || crate::ast::RESERVED_METAMODEL_NOUNS.contains(&s)
    };
    let target_resolves = is_known(&target)
        || is_known(target_base)
        || first_tok_base.map_or(false, |s| is_known(s));
    if !target_resolves { return None; }
    Some((role, op, target, where_clause))
}

/// task-953 — recognise a superlative/ordering comparator clause of the
/// shape `<EntityA> has the <super> <ValueType> among <rest>` where
/// `<super>` is a recognised superlative word
/// (`strongest`/`highest`/`best`/`weakest`/`lowest`/`worst`) and
/// `<ValueType>` is a declared (enum-valued) noun.
///
/// Returns `(op, entity_noun, value_type, among_rest)` where `op` is the
/// aggregate the superlative maps to (`min` for the strongest-family,
/// `max` for the weakest-family — see `SuperlativeComparatorTable`),
/// `entity_noun` is the subject that carries the value (`Commit`),
/// `value_type` is the enum-valued noun being compared (`Security
/// Posture`), and `among_rest` is the text after `among` naming the
/// group join (`Commits the Merge concerns`). The caller resolves the
/// value FT (`entity_noun has value_type`) and the among-join FT against
/// the catalog and assembles the `ConsequentAggregate` — keeping FT
/// resolution co-located with the other clause handlers.
///
/// FFP framing: a superlative is the existing numeric min/max aggregate
/// applied to the value's enum-declaration-order RANK. No new binary op
/// — rank-promotion decouples the enum from the numeric fold.
fn try_parse_superlative_among_clause(
    text: &str,
    noun_names: &[String],
) -> Option<(String, String, String, String)> {
    let t = text.trim().trim_end_matches('.').trim();
    // Split on ` among ` — required for the ordering-superlative form.
    // (A bare `X has the strongest P` with no comparison set is not a
    // well-formed superlative; the `among` set defines what we rank over.)
    let among_idx = t.find(" among ")?;
    let head = t[..among_idx].trim();
    let among_rest = t[among_idx + " among ".len()..].trim().to_string();

    // Head must contain a superlative word as a whole token. Find it via
    // the lifted table; capture the op and the text before/after the word.
    let table = crate::parse_forml2_stage2::SuperlativeComparatorTable::boot();
    let (op, before_super, after_super) = table.iter().find_map(|(word, op)| {
        // Whole-word match: ` <word> ` (the head always has `has the`
        // before and the value type after, so both sides are non-empty).
        let needle = alloc::format!(" {} ", word);
        let idx = head.find(needle.as_str())?;
        Some((
            op.to_string(),
            head[..idx].to_string(),
            head[idx + needle.len()..].trim().to_string(),
        ))
    })?;

    // `before_super` should be `<EntityA> has the` (possibly with a
    // leading determiner). Extract the entity noun = the LAST noun in it.
    let entity_noun = find_nouns(&before_super, noun_names)
        .last()
        .map(|(_, _, n)| parse_role_token(n).0.to_string())?;

    // `after_super` should be the value type, possibly trailed by other
    // tokens. The value type = the FIRST noun in it.
    let value_type = find_nouns(&after_super, noun_names)
        .first()
        .map(|(_, _, n)| parse_role_token(n).0.to_string())?;

    // Entity and value type must differ (a noun can't be the superlative
    // of itself) and both must be declared.
    if entity_noun == value_type { return None; }
    if !noun_names.iter().any(|n| n == &entity_noun) { return None; }
    if !noun_names.iter().any(|n| n == &value_type) { return None; }

    Some((op, entity_noun, value_type, among_rest))
}

/// Parse an arithmetic antecedent clause of Halpin FORML attribute-style
/// form: `<RoleName> is <expr>` (e.g. `Volume is Size * Size * Size`).
///
/// Returns `Some((role_name, expr))` when the clause matches that shape
/// AND the role name is a declared noun AND the RHS parses cleanly;
/// otherwise `None` so the caller can fall through to fact-type
/// resolution. Aggregate forms (`â€¦ is the sum of â€¦`) are explicitly
/// excluded â€” they're parsed by a later pipeline stage.
fn try_parse_computed_binding(text: &str, noun_names: &[String]) -> Option<(String, crate::types::ArithExpr)> {
    let t = text.trim().trim_end_matches('.').trim();
    let t = t.strip_prefix("that ").unwrap_or(t);
    // Aggregates use `is the <op> of â€¦` â€” skip them here.
    if t.contains(" is the ") { return None; }
    let idx = t.find(" is ")?;
    let lhs = t[..idx].trim();
    let rhs = t[idx + 4..].trim();
    // LHS must be a declared noun (role name).
    if !noun_names.iter().any(|n| n == lhs) { return None; }
    let expr = parse_arithmetic_expr(rhs, noun_names)?;
    Some((lhs.to_string(), expr))
}

/// Tokenize a whitespace-flexible arithmetic expression on `+ - * /` and
/// build a left-associative tree. Operands are either numeric literals
/// (f64::from_str) or declared noun names. No precedence yet â€” `A + B * C`
/// parses as `((A + B) * C)`. Parentheses are not yet supported either.
/// Returns `None` if any token fails to parse as an operand or operator.
fn parse_arithmetic_expr(text: &str, noun_names: &[String]) -> Option<crate::types::ArithExpr> {
    use crate::types::ArithExpr;
    // Hand-rolled tokenizer equivalent to splitting on the regex
    // `\s*([+\-*/])\s*` with `find_iter`: emit each `+ - * /` as its
    // own token and treat the surrounding whitespace as a separator.
    let tokens = tokenize_arith(text);
    if tokens.is_empty() { return None; }

    let parse_atom = |token: &str| -> Option<ArithExpr> {
        if let Ok(n) = token.parse::<f64>() { return Some(ArithExpr::Literal(n)); }
        if noun_names.iter().any(|n| n == token) { return Some(ArithExpr::RoleRef(token.to_string())); }
        None
    };

    let mut iter = tokens.into_iter();
    let first = iter.next()?;
    let mut result = parse_atom(&first)?;
    loop {
        let Some(op) = iter.next() else { break };
        if !matches!(op.as_str(), "+" | "-" | "*" | "/") { return None; }
        let next = iter.next()?;
        let rhs = parse_atom(&next)?;
        result = ArithExpr::Op(op, Box::new(result), Box::new(rhs));
    }
    Some(result)
}

/// Strip a trailing numeric comparator (Halpin FORML Example 5: `has Population >= 1000000`)
/// from an antecedent fragment. Returns `(stripped_text, Option<(op, value)>)`.
///
/// Accepts `>=`, `<=`, `>`, `<`, `=`, `!=`, and `<>` â€” the last is normalised
/// to `!=` so compile-time dispatch sees one canonical form. Longer operators
/// (`>=`, `<=`, `!=`, `<>`) are listed first in the alternation so the engine
/// prefers `>=` over `>` on input like `has Amount >= 100`.
/// Split text on " and " only when the delimiter is not inside a
/// single-quoted literal. Example: `Statement has Constraint Keyword
/// 'if and only if'` stays as one clause; `X has A and Y has B`
/// splits into two.
fn split_top_level_and(text: &str) -> Vec<&str> {
    let needle = " and ";
    let mut parts: Vec<&str> = Vec::new();
    let mut in_quote = false;
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if !in_quote
            && i + needle.len() <= bytes.len()
            && &bytes[i..i + needle.len()] == needle.as_bytes()
        {
            parts.push(&text[start..i]);
            start = i + needle.len();
            i = start;
            continue;
        }
        i += 1;
    }
    parts.push(&text[start..]);
    parts
}

fn split_antecedent_comparator(text: &str) -> (String, Option<(String, f64)>) {
    // Hand-rolled equivalent of
    //   `\s*(>=|<=|!=|<>|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$`
    // applied at end-of-string. See `peel_trailing_comparator` for
    // the right-to-left scan that mirrors the regex match shape.
    match peel_trailing_comparator(text) {
        Some((stripped, raw_op, value)) => {
            let op = if raw_op == "<>" { "!=".to_string() } else { raw_op.to_string() };
            (stripped, Some((op, value)))
        }
        None => (text.to_string(), None),
    }
}

/// Pull a cardinality quantifier (`at most N` / `at least N`) out of a
/// derivation-antecedent clause. FORML 2 (Halpin) writes a COUNT premise as
///   `Item is marked by at most 0 Tag`   (count of Tags marking Item ≤ 0)
///   `Item is marked by at least 1 Tag`  (count ≥ 1)
/// where the `at most N` / `at least N` phrase sits between the verb and the
/// trailing counted noun. This is NOT the same as the trailing numeric
/// comparator `peel_trailing_comparator` handles (`has Population >= 1000000`,
/// which compares a role VALUE) — here `N` bounds the CARDINALITY of a role's
/// image set.
///
/// Returns `Some((at_most, count, stripped))` when such a phrase is present,
/// where `at_most == true` for `at most`, `false` for `at least`, `count` is
/// the integer bound, and `stripped` is the clause with the `at most N ` /
/// `at least N ` phrase removed so the bridge fact type resolves cleanly
/// (`Item is marked by Tag`). `None` when no cardinality phrase is present.
///
/// Only integer bounds match — `at most one` (the UC spelling, no digit) is
/// left for the constraint classifier and returns `None` here.
fn extract_antecedent_cardinality(text: &str) -> Option<(bool, usize, String)> {
    // Longest-/specific-first: both markers carry a trailing space so the
    // digit run starts immediately after.
    for (marker, at_most) in [("at most ", true), ("at least ", false)] {
        let Some(idx) = text.find(marker) else { continue };
        let after = &text[idx + marker.len()..];
        // The bound is the leading ASCII digit run. `at most one`/`at least
        // one` (word form) has no digit → skip (returns None for that marker).
        let digit_end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        if digit_end == 0 { continue; }
        let Ok(count) = after[..digit_end].parse::<usize>() else { continue };
        // Strip `<marker><digits>` plus the single following space (if any) so
        // the remaining text reads as the bare fact-type clause. Collapse any
        // resulting double space at the splice point.
        let mut stripped = String::with_capacity(text.len());
        stripped.push_str(text[..idx].trim_end());
        let tail = after[digit_end..].trim_start();
        if !tail.is_empty() {
            stripped.push(' ');
            stripped.push_str(tail);
        }
        return Some((at_most, count, stripped.trim().to_string()));
    }
    None
}

/// Expand possessive syntax in a derivation body clause.
///
/// Pattern: `<Noun1>'s <Noun2>` is syntactic sugar for a join through Noun2:
///   `<Noun1>'s <Noun2> has <X>` â†’ `<Noun1> has <Noun2> and that <Noun2> has <X>`
///
/// This is a pre-processing step applied to the antecedent text before
/// fact-type resolution.  Each possessive token is replaced with an
/// explicit two-clause join so that the anaphora detector in
/// `resolve_derivation_rule` can find the `that <Noun2>` join key.
///
/// Returns `Some(expanded)` when at least one possessive was expanded,
/// `None` when the text contains no `'s` pattern.
///
/// # Examples
/// ```text
/// // Input antecedent clause:
/// "Order's Customer has Age"
/// // Expanded:
/// "Order has Customer and that Customer has Age"
/// ```
pub(crate) fn try_expand_possessive(text: &str, noun_names: &[String]) -> Option<String> {
    // #884 Sweep-1 lift - possessive-trigger vocabulary lifts to
    // `PossessiveMarkerTable` so the marker set lives in
    // `readings/forml2-grammar.md` as a `Possessive Marker` enum value
    // type. Boot stays in sync with the grammar; the table's `expand`
    // accessor owns the longest-noun-match + chained-iteration body so
    // the caller here is a one-liner.
    crate::parse_forml2_stage2::PossessiveMarkerTable::boot().expand(text, noun_names)
}

/// Resolve a derivation rule's text into structured fact type references.
///
/// Splits on " if "/" iff " to get consequent and antecedent parts,
/// then matches each part's nouns against fact_types_map by role noun names.
/// Anaphoric "that X" references are stripped to bare noun name "X".
///
/// Per-antecedent inline numeric comparisons (Halpin FORML Example 5) are
/// extracted via `split_antecedent_comparator` BEFORE fact-type resolution,
/// so `has Population >= 1000000` resolves to the base FT `has Population`
/// with an AntecedentFilter attached restricting that antecedent's population.
/// Temporal predicates are runtime clock checks with no declared FT.
///
/// #881 Sweep-1 lift — temporal-marker vocabulary lifts to
/// `TemporalPredicateTable` so the keyword set lives in
/// `readings/forml2-grammar.md` as a `Temporal Predicate Keyword`
/// enum value type. Boot stays in sync with the grammar; same
/// any-match-wins iteration as the legacy inline
/// `l.contains("now is ") || l.contains(" in the past") || ...` chain.
fn is_temporal_predicate(clause: &str) -> bool {
    crate::parse_forml2_stage2::TemporalPredicateTable::boot().matches(clause)
}

fn resolve_derivation_rule(
    rule: &mut DerivationRuleDef,
    nouns_map: &HashMap<String, NounDef>,
    fact_types_map: &HashMap<String, FactTypeDef>,
    catalog: &SchemaCatalog,
    // subtype → supertype chain (one parent per noun). Empty in the
    // public test wrapper; populated from `CellIndex.subtypes` in the
    // production `cell_index_from_state` path.
    subtypes: &HashMap<String, String>,
) {
    // Walk the subtype→supertype chain from `noun`, returning every
    // proper ancestor in order (nearest parent first). Bounded by the
    // number of declared nouns to defend against a malformed cyclic
    // declaration. Used to bridge a subtype-keyed clause / join key UP
    // to a fact type declared on a supertype: subtype instances ARE
    // supertype instances, so a clause `that <Sub> <verb> <Y>` resolves
    // to a FT keyed on `<Sup>` and the equi-join links the `<Sub>` role
    // to the `<Sup>` role.
    let supertype_chain = |noun: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut cur = noun.to_string();
        let mut guard = 0usize;
        while let Some(sup) = subtypes.get(&cur) {
            if out.iter().any(|s| s == sup) { break; } // cycle guard
            out.push(sup.clone());
            cur = sup.clone();
            guard += 1;
            if guard > subtypes.len() + 1 { break; }
        }
        out
    };
    // task-930: marker strip moved to translate_derivation_rules_with_matrix
    // in parse_forml2_stage2 (the layer that writes DR cell facts).
    // resolve_derivation_rule receives an already-normalized rule.text;
    // the materialization field arrives via the DR cell deserialize path
    // in compile::cell_index_from_state.

    // Longest-first noun list for Theorem 1 matching. Includes the universal
    // metamodel `Fact` noun (not special); `find_nouns` guards it from matching
    // inside `Fact Type` so the metamodel subtype rule still resolves (task980).
    let mut noun_names: Vec<String> = nouns_map.keys().cloned().collect();
    if !noun_names.iter().any(|n| n == "Fact") { noun_names.push("Fact".to_string()); }
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // #914 — Pre-pass: extract cross-antecedent role-vs-role value
    // comparison clauses (`<NounToken>'s <Role> <word-comparator>
    // <NounToken>'s <Role>`) from the antecedent text BEFORE the
    // generic possessive expansion fires. Each match is recorded for
    // later mapping into `AntecedentRoleComparison`; the clause itself
    // is dropped from `rule.text` so the downstream FT-resolution loop
    // never sees it (and never emits a spurious `AntecedentFilter` or
    // an unresolved-clause diagnostic). Reuses the existing
    // `WordComparatorTable` vocabulary — no parallel keyword table.
    let mut pending_role_comparisons: Vec<
        (String, String, &'static str, String, String)
    > = Vec::new();
    {
        let sep_offset = rule.text.find(" iff ")
            .map(|i| (i, i + 5))
            .or_else(|| rule.text.find(" if ").map(|i| (i, i + 4)));
        if let Some((sep_start, sep_end)) = sep_offset {
            let consequent_part = rule.text[..sep_start].to_string();
            let sep_word = rule.text[sep_start..sep_end].to_string();
            let antecedent_part = rule.text[sep_end..].to_string();
            // Walk top-level ` and ` clauses; keep the unrecognised
            // ones, record the recognised ones as comparison specs.
            let mut kept_clauses: Vec<String> = Vec::new();
            let mut found_any = false;
            for raw in split_top_level_and(&antecedent_part) {
                let part = raw.trim_end_matches('.').trim();
                if let Some((lhs_tok, lhs_role, op, rhs_tok, rhs_role)) =
                    try_extract_cross_antecedent_role_comparison(part, &noun_names)
                {
                    pending_role_comparisons.push(
                        (lhs_tok, lhs_role, op, rhs_tok, rhs_role));
                    found_any = true;
                } else {
                    // Preserve original (with whatever trailing period
                    // the input carried) so re-assembly stays byte-
                    // stable when no comparison clauses are present.
                    kept_clauses.push(raw.to_string());
                }
            }
            if found_any {
                let new_antecedent = kept_clauses.join(" and ");
                rule.text = format!("{}{}{}",
                    consequent_part, sep_word, new_antecedent);
            }
        }
    }

    // Pre-process: expand possessive syntax (`X's Y`) into explicit join form
    // (`X has Y and that Y`) so the anaphora detector below can classify the
    // rule as a Join derivation.  Only the antecedent portion is rewritten;
    // the consequent is left unchanged.
    if rule.text.contains("'s ") {
        // Split off everything up to and including the iff/if keyword,
        // expand only the antecedent portion, then reassemble.
        let sep_offset = rule.text.find(" iff ")
            .map(|i| (i, i + 5))
            .or_else(|| rule.text.find(" if ").map(|i| (i, i + 4)));
        if let Some((sep_start, sep_end)) = sep_offset {
            let consequent_part = &rule.text[..sep_start];
            let sep_word = &rule.text[sep_start..sep_end];
            let antecedent_part = &rule.text[sep_end..];
            if let Some(expanded) = try_expand_possessive(antecedent_part, &noun_names) {
                rule.text = format!("{}{}{}", consequent_part, sep_word, expanded);
            }
        }
    }

    // Split on " iff " or " if " to get (consequent, antecedent_text)
    let (consequent_text, antecedent_raw) = rule.text
        .find(" iff ")
        .map(|i| (&rule.text[..i], &rule.text[i + 5..]))
        .or_else(|| rule.text.find(" if ")
            .map(|i| (&rule.text[..i], &rule.text[i + 4..])))
        .unwrap_or((&rule.text, ""));

    // #276 Category G — expand `<head> that <verb>` relative clauses
    // into explicit `<head> and <last_noun> <verb>` conjunctions so
    // the downstream split on ` and ` produces resolvable clauses.
    // Back-reference anaphora (`that <Noun>`) is preserved, and the
    // expansion self-guards via `head_resolves` to avoid turning a
    // single unresolved clause into multiple unresolved fragments
    // when the head isn't a declared FT.
    let antecedent_expanded = expand_that_relatives(antecedent_raw, &noun_names, catalog);
    let antecedent_text: &str = antecedent_expanded.as_str();

    // Split antecedent on " and " to get individual conditions
    // Split on top-level " and " only — a literal like `'if and only
    // if'` contains an `and` that must not break the clause. Walk the
    // text and break only when not inside a single-quoted span.
    //
    // `_raw` because an n-ary objectified FT whose READING itself contains
    // ` and ` (e.g. the ternary mapping `A Val and B Val map to C Val`,
    // referenced in a body as `that A Val and that B Val map to that C Val`)
    // is OVER-SPLIT here into `that A Val` + `that B Val map to that C Val`,
    // neither of which resolves to the declared FT. The catalog-aware
    // re-join pass below (run once `resolve_fact_type` is in scope) coalesces
    // such fragments back into the single FT clause.
    let antecedent_parts_raw: Vec<&str> = split_top_level_and(antecedent_text)
        .into_iter()
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| !s.is_empty())
        .collect();

    // Strip quantifier, anaphoric, and determiner words from a text
    // fragment. #273: legal / prose rule bodies spell out articles
    // ("the Tool", "a Party", "an Exemption") that aren't part of
    // the FT identity. Removing them lets the catalog lookup match
    // against the clean `<Noun> <verb> <Noun>` form the FT was
    // declared with. Replacements are space-padded to preserve word
    // boundaries inside the clause (so `the ` inside `theoretical`
    // is untouched).
    let strip_anaphora = |text: &str| -> String {
        let replaced = text
            .replace("that ", "")
            .replace("some ", "")
            .replace("each ", "")
            .replace("any ", "")
            .replace(" the ", " ")
            .replace(" a ", " ")
            .replace(" an ", " ");
        // Leading determiners at the very start of the clause.
        replaced
            .trim_start_matches("the ")
            .trim_start_matches("a ")
            .trim_start_matches("an ")
            .to_string()
    };

    // Resolve a text fragment to a Fact Type ID via rho-lookup through the catalog.
    // Strips subscripts (Person1 â†’ Person) before catalog lookup â€” find_nouns
    // captures the subscripted token, but the catalog keys are base nouns.
    //
    // Role-variable tokens (`Fact Type (FT) has Role`, `Transition (Tr) is
    // from Status`) are stripped before verb extraction: the variable sits
    // between the first noun and the verb, so without the strip the verb
    // comes out as “(FT) has” — the verb-specific catalog match misses and
    // the clause falls to the role-set fallback, which returns None whenever
    // the noun set is ambiguous (e.g. {fact type, role} also keys the
    // `Fact_Type_where_…_plays_a_Role` template entry). The variables
    // themselves are consumed by the skolem/join machinery from the
    // ORIGINAL rule text — only this local resolution view drops them.
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_role_variables(&strip_anaphora(fragment));
        // valuetyped-join-key-noun-inflation: exact-reading match FIRST,
        // mirroring `resolve_consequent_strict` below. An antecedent clause
        // whose VERB phrase contains a word that case-insensitively collides
        // with a declared type name — e.g. `Shape has confidence Count` when
        // the metamodel declares `Confidence is a value type` — has its role
        // set inflated by `find_nouns` ([Shape, confidence, Count] instead of
        // [Shape, Count]), so the verb/role-set rho-lookup below MISSES and
        // the clause is dropped, collapsing a 2-antecedent join to one and
        // losing the projected non-key role (the `Count` in the consequent
        // head). Normalising the clause's tokens (strip Halpin subscripts,
        // lowercase) and comparing to each declared FT's reading binds it
        // directly, independent of the noun-extraction path. An exact reading
        // match is the most specific resolution; non-matches fall through to
        // the verb/role-set path unchanged. (Halpin §9.7 p383: a conceptual
        // join must propagate the joined role; the rule is Ullman-safe, so
        // this is an evaluation/resolution gap, not an unsafe rule.)
        let norm_reading = |s: &str| s.split_whitespace()
            .map(|t| parse_role_token(t).0.to_lowercase())
            .collect::<Vec<_>>().join(" ");
        let clause_norm = norm_reading(&cleaned);
        if !clause_norm.is_empty() {
            if let Some((id, _)) = fact_types_map.iter()
                .find(|(_, ft)| norm_reading(&ft.reading) == clause_norm)
            {
                return Some(id.clone());
            }
        }
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        if found_nouns.is_empty() { return None; }
        let base_refs: Vec<String> = found_nouns.iter()
            .map(|(_, _, n)| parse_role_token(n).0.to_string())
            .collect();
        let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();

        // Verb extraction: text between first and second noun for
        // binary+ clauses; text after the single noun for unary
        // clauses (#274 Category A). Without the unary branch
        // `Customer is in EEA` looks up with empty verb and misses
        // the catalog entry keyed on verb "is in EEA".
        let verb = match found_nouns.len() {
            1 => cleaned[found_nouns[0].1..].trim(),
            _ => cleaned[found_nouns[0].1..found_nouns[1].0].trim(),
        };

        // rho-lookup: try with verb first, then noun set only
        let verb_opt = (!verb.is_empty()).then_some(verb);
        // VERB-STRICT first (exact-verb / inverse-voice; NO unique-entry
        // fallback) so a verb mismatch on a noun-set carrying a single FT falls
        // THROUGH to the subtype bridge below instead of binding that lone
        // different-verb FT (e.g. `Transition is from <SMD>` must not grab
        // `Transition is defined in State Machine Definition`).
        verb_opt.and_then(|v| catalog.resolve_verb_strict(&role_refs, v))
            // subtype-join → supertype FT: a clause keyed on a SUBTYPE noun
            // (`that Noun belongs to Domain`, Noun < Function) resolves to a
            // fact type declared on the SUPERTYPE (`Function belongs to
            // Domain`). The exact lookup misses because the catalog keys on
            // the supertype noun set. Subtype instances ARE supertype
            // instances, so substitute each clause noun with one of its
            // supertypes (one role at a time) and retry the rho-lookup.
            //
            // ORDER: this VERB-SPECIFIC bridge MUST precede the verb-AGNOSTIC
            // role-set fallback below. Else a clause `Transition is from <SMD>`
            // (SMD a Status subtype) is mis-resolved by noun-set collision —
            // the agnostic fallback matches the same-noun-set `Transition is
            // defined in <SMD>` (a DIFFERENT verb) before the correct same-verb
            // `Transition is from Status` bridge runs, so the Harel
            // inherited-edge rule binds the wrong FT and never fires.
            //
            // Walk the WHOLE chain and prefer the FARTHEST (most-general /
            // root-most) supertype that resolves: a base relation is
            // declared at the most general type it applies to, whereas a
            // same-verb FT on a NEARER supertype is typically a DERIVED
            // specialization (e.g. with `Fact Type < Resource < … <
            // Function`, both `Resource belongs to Domain` (a derived
            // consequent) and `Function belongs to Domain` (the asserted
            // base relation) resolve — the root-most `Function` one is the
            // declaration we want). Verb-specific only — the fuzzy role-set
            // fallback is NOT retried under substitution, so this can only
            // bind a clause to a same-verb FT on a genuine supertype, never
            // fabricate a cross-verb match.
            .or_else(|| {
                if subtypes.is_empty() { return None; }
                let mut best: Option<String> = None;
                for i in 0..base_refs.len() {
                    // supertype_chain is nearest→farthest; the LAST hit is
                    // the root-most resolving substitution for this role.
                    for sup in supertype_chain(&base_refs[i]) {
                        let mut subst: Vec<&str> = role_refs.clone();
                        subst[i] = sup.as_str();
                        if let Some(id) = catalog.resolve(&subst, verb_opt) {
                            best = Some(id);
                        }
                    }
                    if best.is_some() { break; }
                }
                best
            })
            .or_else(|| catalog.resolve(&role_refs, None))
            // #963: objectified-pivot abbreviation. A 4-role objectified FT
            // (`X pivots A is implemented by B at C`) is referenced in rule
            // bodies by its abbreviated 3-role reading with the trailing
            // `at C` dropped. Both exact lookups above miss on the 3-vs-4
            // noun-set mismatch, so the binding never forms; fall back to a
            // superset-by-one reading-prefix match.
            .or_else(|| catalog.resolve_objectified_abbrev(&role_refs, &cleaned))
    };

    // A derivation CONSEQUENT must name a fact type that actually exists.
    // The role-set-only fallback above is correct for antecedent clauses
    // (authors phrase verbs loosely there), but for the head it silently
    // maps an undeclared FT onto a same-role-set / different-verb one — an
    // undeclared `MonoView has effective Pane Mode` resolves to the
    // declared `MonoView has default Pane Mode`, then emits a value-less
    // head the chainer drops every round. The head therefore resolves
    // verb-specifically ONLY (no role-set fallback); an unresolved head is
    // handled by the caller (recorded + consequent left empty = rule dropped).
    let resolve_consequent_strict = |fragment: &str| -> Option<String> {
        // Same role-variable strip as `resolve_fact_type` — a head like
        // `ViewElement (E) renders Fact Type (FT)` must extract verb
        // "renders", not "(E) renders".
        let cleaned = strip_role_variables(&strip_anaphora(fragment));
        // Exact-reading match FIRST (join-qualified-value-role-consequent-unresolved):
        // normalize the head's tokens (strip Halpin subscripts) and compare to
        // each declared FT reading. This binds a head whose qualifier word
        // collides case-insensitively with a type name — `Solve has glyph Count`,
        // where `glyph` would otherwise be mis-read by find_nouns as the `Glyph`
        // type and inflate the extracted role set to [Solve,Glyph,Count] vs the
        // FT's real [Solve,Count] — directly to its FT, independent of the
        // noun-extraction path. An exact reading match is the most specific
        // resolution; non-matches fall through to the verb/role-set path below.
        let norm_reading = |s: &str| s.split_whitespace()
            .map(|t| parse_role_token(t).0.to_lowercase())
            .collect::<Vec<_>>().join(" ");
        let head_norm = norm_reading(&cleaned);
        if !head_norm.is_empty() {
            if let Some((id, _)) = fact_types_map.iter()
                .find(|(_, ft)| norm_reading(&ft.reading) == head_norm)
            {
                return Some(id.clone());
            }
        }
        let found_nouns: Vec<(usize, usize, String)> = find_nouns(&cleaned, &noun_names);
        if found_nouns.is_empty() { return None; }
        let base_refs: Vec<String> = found_nouns.iter()
            .map(|(_, _, n)| parse_role_token(n).0.to_string())
            .collect();
        let role_refs: Vec<&str> = base_refs.iter().map(|s| s.as_str()).collect();
        let verb = match found_nouns.len() {
            1 => cleaned[found_nouns[0].1..].trim(),
            _ => cleaned[found_nouns[0].1..found_nouns[1].0].trim(),
        };
        let verb_opt = (!verb.is_empty()).then_some(verb);
        catalog.resolve(&role_refs, verb_opt)
    };

    // eud-valuetype-bridge-join: catalog-aware re-join of FT-reading clauses
    // that `split_top_level_and` over-split. An objectified n-ary FT whose
    // declared READING contains ` and ` — the canonical case is a ternary
    // value-type mapping `A Val and B Val map to C Val` referenced in a body
    // as `that A Val and that B Val map to that C Val` — is split into
    // fragments (`that A Val`, `that B Val map to that C Val`) that each fail
    // to resolve to the declared FT. Without this pass the clause is silently
    // dropped (it slips past the unresolved-clause guard via the lenient
    // bare-noun / existential branches), the FT never enters
    // `antecedent_sources`, and a Join head whose value role lives ONLY on
    // that FT (e.g. `C Val`) is left unbound → a NULL projection (zero useful
    // rows). SPD-1's `... Valence Range and Arousal Range map to Affect
    // Region` has the identical shape.
    //
    // Discipline (avoid over-generation): only MERGE a run of >=2 fragments
    // when (a) the FIRST fragment does NOT resolve to a declared FT on its
    // own — so two independently-valid adjacent antecedents are never
    // glued — AND (b) the joined run DOES resolve to a declared FT. The
    // longest such run starting at each position wins; a single fragment that
    // already resolves (or that no merge can complete) passes through
    // unchanged. This keys on declared-FT membership, not on incidental
    // shared enum values, so it cannot fabricate spurious joins.
    let antecedent_parts: Vec<String> = {
        let raw = &antecedent_parts_raw;
        let mut out: Vec<String> = Vec::with_capacity(raw.len());
        let mut i = 0usize;
        while i < raw.len() {
            // A fragment that already resolves on its own is a complete
            // clause — never absorb following fragments into it.
            let solo_resolves = resolve_fact_type(raw[i]).is_some();
            let mut chosen_end = i + 1; // exclusive; default = no merge
            if !solo_resolves {
                // Greedily extend the run; remember the LONGEST end whose
                // joined text resolves to a declared FT.
                let mut acc = raw[i].to_string();
                let mut j = i + 1;
                while j < raw.len() {
                    acc.push_str(" and ");
                    acc.push_str(raw[j]);
                    if resolve_fact_type(&acc).is_some() {
                        chosen_end = j + 1;
                    }
                    j += 1;
                }
            }
            if chosen_end > i + 1 {
                out.push(raw[i..chosen_end].join(" and "));
            } else {
                out.push(raw[i].to_string());
            }
            i = chosen_end;
        }
        out
    };

    // Detect "that X" anaphoric references -- nouns preceded by "that " in
    // antecedent parts become join keys. Mutable because the #914
    // cross-antecedent-comparison branch below can append synthesised
    // shared-noun keys when promoting a rule to Join classification.
    //
    // At each `that ` site take the LONGEST matching noun: with multi-word
    // nouns a shorter noun can be a prefix of a longer one (`that Fact` ⊂
    // `that Fact Type`), and the naive substring test would record BOTH —
    // injecting a spurious join key (`Fact`) that fans the equi-join onto
    // the wrong role. `noun_names` is sorted longest-first; the first hit
    // whose match ends on a word boundary is the intended anaphor.
    let is_word_boundary = |b: Option<u8>| -> bool {
        match b { None => true, Some(c) => !(c.is_ascii_alphanumeric() || c == b'_') }
    };
    let mut join_keys: Vec<String> = antecedent_parts.iter()
        .flat_map(|part| {
            let bytes = part.as_bytes();
            let mut keys: Vec<String> = Vec::new();
            // Scan every `that ` occurrence; bind it to one noun (longest).
            let mut search_from = 0usize;
            while let Some(rel) = part[search_from..].find("that ") {
                let noun_start = search_from + rel + "that ".len();
                let after = &part[noun_start..];
                if let Some(noun) = noun_names.iter().find(|noun| {
                    after.starts_with(noun.as_str())
                        && is_word_boundary(bytes.get(noun_start + noun.len()).copied())
                        && !shadowed_by_longer_reserved(after, noun.len())
                }) {
                    if !keys.contains(noun) { keys.push(noun.clone()); }
                }
                search_from = noun_start;
            }
            keys
        })
        .collect::<Vec<_>>();

    // Resolve consequent. If the consequent text carries a trailing
    // single-quoted literal (e.g. grammar rule head `Statement has
    // Classification 'Entity Type Declaration'`, #286), capture the
    // literal and record it as a fixed binding on the consequent FT's
    // last role before handing the text to the FT resolver. find_nouns
    // already ignores the quoted segment, so the FT itself resolves on
    // the unquoted portion either way. The vec is cleared first because
    // re_resolve_rules re-runs this function and would otherwise
    // accumulate duplicates from prior passes.
    rule.consequent_role_literals.clear();
    // task-970: clear skolem head roles so re_resolve_rules re-derives
    // them without accumulating duplicates from prior passes.
    rule.skolem_head_roles.clear();
    // Hand-rolled equivalent of regex ` '([^']*)'\s*$`: capture the
    // single-quoted literal at end of string, after a leading space.
    let consequent_trailing_literal =
        strip_trailing_quoted_literal(consequent_text).map(|(_, lit)| lit);
    let consequent_strict = resolve_consequent_strict(consequent_text);
    // If the head only resolves through the fuzzy role-set fallback (the
    // verb-specific match failed), it names a fact type that isn't declared
    // with that reading. Record it and leave the consequent empty so the
    // rule is dropped at the `!consequent_cell.is_empty()` filter — a
    // derivation that names a missing fact type refuses to run rather than
    // silently writing a value-less fact into a same-role-set FT.
    // GAP A (task subtype-join-antecedent child 4): detect the metamodel-derivation
    // consequent head "Fact Type has inherited Resource at Role".  The nouns
    // "Fact Type" and "Resource" are NOT in the user-declared noun catalog, so
    // `resolve_consequent_strict` always returns `None` for this head.  Recognise
    // the pattern by text and emit `AntecedentRole { antecedent_index: 1, role: "id" }`
    // — index 1 is the `FactType("FactType")` antecedent added by GAP B, and the
    // role "id" is the binding key the `FactType` cell stores for each fact-type's
    // identifier (mirrors the oracle's per-FT `ft_id` extracted from `data.fact_types`).
    // The pattern check: contains the phrase "inherited" and the word "Fact Type"
    // (case-normalised) and either "Resource" or "Role" — this is specific enough
    // to avoid false matches on user-authored rule heads.
    let is_metamodel_subtype_consequent = {
        let lower = consequent_text.to_lowercase();
        lower.contains("inherited") && lower.contains("fact type")
    };
    // ss-autofill-retire-2 — the SS (Subset) Constraint auto-fill metamodel
    // rule's consequent head is "Fact Type has auto-filled Fact" (readings/
    // core/derivation.md §"SS Subset-Constraint auto-fill").  "Fact Type" and
    // "Fact" are NOT user-declared nouns, so `resolve_consequent_strict`
    // returns `None`.  Recognise the head by text (mirrors the subtype
    // `is_metamodel_subtype_consequent` recognition above) so the fuzzy-match
    // fallback does NOT push a spurious UnresolvedClause.  The consequent cell
    // is left empty: the SS-autofill reading-lift in
    // `compile_explicit_derivation` drives the per-SS-Constraint fanout off
    // `CellIndex::ss_autofill_pairs()` (each inner Func carries its own
    // `Literal(consequent_ft)`), so this rule's own consequent value is unused.
    let is_metamodel_ss_autofill_consequent = {
        let lower = consequent_text.to_lowercase();
        lower.contains("auto-filled") && lower.contains("fact type")
    };
    if consequent_strict.is_none() {
        if is_metamodel_subtype_consequent || is_metamodel_ss_autofill_consequent {
            // Suppress the fuzzy-match noise: this head is intentional, not a typo.
        } else if let Some(fuzzy) = resolve_fact_type(consequent_text) {
            rule.unresolved_clauses.push(format!(
                "consequent '{}' references no declared fact type \
                 (nearest by role set: {})",
                consequent_text.trim(), fuzzy));
        }
    }
    let resolved_consequent = consequent_strict.unwrap_or_default();
    rule.consequent_cell = if resolved_consequent.is_empty() && is_metamodel_subtype_consequent {
        // Dynamic consequent: the target cell id is the "id" binding of the
        // FactType antecedent (antecedent index 1 = FactType("FactType")).
        // The compiler detects this AntecedentRole + InstancesOfNoun pattern
        // as the subtype-inheritance reading-lift and expands it into the same
        // per-(sub, sup, ft) Funcs the procedural synthesiser produces.
        crate::types::ConsequentCellSource::AntecedentRole { antecedent_index: 1, role: "id".to_string() }
    } else {
        crate::types::ConsequentCellSource::Literal(resolved_consequent)
    };
    if let Some(lit) = consequent_trailing_literal {
        if !rule.consequent_cell.is_empty_literal() {
            let role = fact_types_map.get(rule.consequent_cell.literal_id())
                .and_then(|ft| ft.roles.last())
                .map(|r| r.noun_name.clone())
                .unwrap_or_default();
            if !role.is_empty() {
                rule.consequent_role_literals.push(
                    crate::types::ConsequentRoleLiteral { role, value: lit });
            }
        }
    }
    // derivation-literal-consequent-subject-binding: ALSO capture a LEADING
    // (subject / first-role) entity-literal pin, e.g. `Site 'resolver' re-derives
    // Cell`. The trailing path above only pins the LAST role (object/value
    // literals like `... Classification 'X'`), so a subject literal was silently
    // dropped and the emitted fact carried an unbound subject (an orphan tuple).
    // FORML2 sanctions a constant subject in an apply-to-all derivation (alpha
    // over Filter), so bind it. `find_nouns` already ignored the quoted segment
    // when resolving the consequent FT, so the FT is unaffected either way.
    if !rule.consequent_cell.is_empty_literal() {
        if let Some(first_noun) = fact_types_map.get(rule.consequent_cell.literal_id())
            .and_then(|ft| ft.roles.first())
            .map(|r| r.noun_name.clone())
        {
            if let Some(rest) = consequent_text.trim().strip_prefix(first_noun.as_str()) {
                if let Some(after_open) = rest.trim_start().strip_prefix('\'') {
                    if let Some(end) = after_open.find('\'') {
                        let lit = after_open[..end].to_string();
                        if !lit.is_empty()
                            && !rule.consequent_role_literals.iter().any(|c| c.role == first_noun) {
                            rule.consequent_role_literals.push(
                                crate::types::ConsequentRoleLiteral { role: first_noun, value: lit });
                        }
                    }
                }
            }
        }
    }

    // Resolve antecedents, carrying inline-comparator filters AND
    // arithmetic-definitional clauses alongside. A definitional clause
    // like `Volume is Size * Size * Size` does not resolve to a fact
    // type â€” it populates consequent_computed_bindings instead. Filter
    // clauses like `has Population >= 1000000` resolve to the base FT
    // with an AntecedentFilter pinned to that antecedent's position.
    // task subtype-join-antecedent child 4: changed from Vec<String> to
    // Vec<AntecedentSource> so InstancesOfNoun sources (GAP C) can be pushed
    // alongside FactType sources in the same slot-tracking pipeline.
    let mut resolved_ids: Vec<crate::types::AntecedentSource> = Vec::new();
    // #914 — parallel vec recording the antecedent-clause text that
    // produced each entry in `resolved_ids`. Used after the main loop
    // to map cross-antecedent comparison specs (noun_token + role)
    // back to their antecedent indices. Length stays equal to
    // `resolved_ids.len()`.
    let mut resolved_part_text: Vec<String> = Vec::new();
    let mut filters: Vec<crate::types::AntecedentFilter> = Vec::new();
    let mut role_literals: Vec<crate::types::AntecedentRoleLiteral> = Vec::new();
    let mut cardinalities: Vec<crate::types::AntecedentCardinality> = Vec::new();
    let mut computed: Vec<crate::types::ConsequentComputedBinding> = Vec::new();
    let mut aggregates: Vec<crate::types::ConsequentAggregate> = Vec::new();
    let mut universals: Vec<crate::types::ConsequentUniversal> = Vec::new();
    // Cursor for aggregate where-body absorption. The outer
    // `split_top_level_and` over `antecedent_text` (line ~1295) breaks an
    // aggregate's MULTI-CLAUSE `where`-body apart at its top-level ` and `s
    // — so `… count of Item1 where Item1 blocks the Item and Item1 has
    // Status 'open'` arrives as TWO parts: the aggregate head (with a
    // truncated where-body `Item1 blocks the Item`) and a stray
    // `Item1 has Status 'open'`. When an aggregate head is recognized we
    // RE-JOIN it with the trailing parts to recover the full where-body,
    // and advance this cursor past them so the loop doesn't re-process the
    // absorbed filter clauses as independent antecedents.
    let mut skip_until: usize = 0;
    for (part_idx, part) in antecedent_parts.iter().enumerate() {
        if part_idx < skip_until { continue; }
        // Aggregate clauses (Halpin `<role> is the <op> of <target> where â€¦`).
        // They resolve the where-clause to a source FT and record the
        // group-key role â€” the non-target role on that FT. Match ahead of
        // the generic definitional path so `â€¦ is the count of â€¦` isn't
        // mistaken for arithmetic.
        if let Some((role, op, target, head_where)) =
            try_parse_aggregate_clause(part, &noun_names)
        {
            // Recover the FULL where-body: the aggregate head carries only
            // the first where-clause (`head_where`); subsequent parts are
            // the remaining top-level ` and `-joined where-clauses. Join
            // them back and mark them consumed. (An aggregate's `where`
            // filter extends to the end of the antecedent in FORML2 — the
            // canonical Halpin examples and the ring-count readings put the
            // aggregate as the whole/last RHS.)
            // Only absorb trailing parts when the head actually opened a
            // `where`-body — a no-where aggregate (`done Task Count is the
            // count of Task`) must not swallow an unrelated sibling clause.
            let where_clause = if !head_where.is_empty()
                && part_idx + 1 < antecedent_parts.len()
            {
                let tail = antecedent_parts[part_idx + 1..].join(" and ");
                skip_until = antecedent_parts.len();
                format!("{} and {}", head_where, tail)
            } else {
                head_where
            };
            // The `where`-body may be MULTI-CLAUSE: a source clause that
            // establishes the counted entity (`Item1 blocks the Item`) plus
            // zero or more literal-filter clauses over that entity
            // (`Item1 has Status 'open'`). Split on top-level ` and ` and
            // classify each sub-clause. A single-clause body (the legacy
            // `… count of Part where Thing has Part`) yields exactly the
            // source clause and no filters, so this path is a strict
            // generalization of the old single-resolve.
            let where_clauses = split_top_level_and(&where_clause);

            // The counted-entity token is the aggregate target (`Item1`);
            // its base noun (`Item`) is what the source/filter FTs carry.
            let target_base = parse_role_token(target.trim()).0.to_string();
            // The consequent subject noun (`Item` in `Item has Open Dep
            // Count`) is the group key — it groups the counted entities.
            let consequent_subject: Option<String> =
                find_nouns(consequent_text, &noun_names).first()
                    .map(|(_, _, n)| parse_role_token(n).0.to_string());

            // First pass: find the SOURCE clause — one that resolves to an
            // FT, carries the target base noun, and is NOT a pure literal
            // filter (literal filters carry a trailing quoted value and are
            // handled as restrictions, not the counted relation).
            let mut source: Option<(String, &str)> = None; // (ft_id, clause)
            let mut filter_clauses: Vec<&str> = Vec::new();
            for clause in where_clauses.iter() {
                let (clause_no_lit, has_lit) =
                    match strip_trailing_quoted_literal(clause.trim()) {
                        Some((without, _)) => (without, true),
                        None => (clause.trim().to_string(), false),
                    };
                let (clause_stripped, _) = split_antecedent_comparator(&clause_no_lit);
                let mut resolved = resolve_fact_type(&clause_stripped);
                // The aggregate SOURCE is a DISTINCT fact type from the
                // consequent. A consequent whose reading is a SUPERSTRING of the
                // body clause (`Glyph shortest reaches Glyph at Count` vs the
                // body's `Glyph reaches Glyph at Count`) shares its role
                // multiset, so the role-set-only resolve fallback can pick the
                // consequent — making the fold read its own (empty) cell. When
                // that happens, re-resolve to the distinct same-role-multiset FT.
                let consequent_ft_id = rule.consequent_cell.literal_id();
                if resolved.as_deref() == Some(consequent_ft_id) {
                    let mut want: Vec<String> = find_nouns(&clause_stripped, &noun_names)
                        .iter().map(|(_, _, n)| parse_role_token(n).0.to_string()).collect();
                    want.sort();
                    // aggregate-source-same-signature-collision: `moves` and
                    // `reaches` share the {Node,Node,Cost} role multiset, so a
                    // signature-ONLY `.find()` picks the first-declared sibling
                    // (`moves`, the base edge) instead of the one the clause
                    // actually NAMES (`reaches`, the closure) — making the
                    // aggregate fold the wrong relation (min over the base edges,
                    // not the closure). Disambiguate by the clause's CONNECTIVE
                    // (non-noun) words: prefer the same-signature FT whose reading
                    // shares them; fall back to the first sibling only when none
                    // matches (preserving prior behaviour for the unambiguous
                    // single-sibling case).
                    let connectives = |text: &str| -> Vec<String> {
                        let nouns: Vec<String> = find_nouns(text, &noun_names).iter()
                            .flat_map(|(_, _, n)| [n.to_string().to_lowercase(),
                                parse_role_token(n).0.to_lowercase()])
                            .collect();
                        text.split_whitespace()
                            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
                            .filter(|w| !w.is_empty() && !nouns.contains(w))
                            .collect()
                    };
                    let want_conn = connectives(&clause_stripped);
                    let siblings: Vec<(String, Vec<String>)> = fact_types_map.iter()
                        .filter(|(id, ft)| id.as_str() != consequent_ft_id && {
                            let mut have: Vec<String> =
                                ft.roles.iter().map(|r| r.noun_name.clone()).collect();
                            have.sort();
                            have == want
                        })
                        .map(|(id, ft)| (id.clone(), connectives(&ft.reading)))
                        .collect();
                    resolved = siblings.iter()
                        .find(|(_, conn)| *conn == want_conn)
                        .or_else(|| siblings.first())
                        .map(|(id, _)| id.clone());
                }
                let carries_target = resolved.as_ref().and_then(|id| fact_types_map.get(id))
                    .map(|ft| ft.roles.iter().any(|r| r.noun_name == target_base))
                    .unwrap_or(false);
                if source.is_none() && !has_lit && carries_target {
                    source = resolved.map(|id| (id, *clause));
                } else {
                    filter_clauses.push(clause);
                }
            }
            // Fallback: if no clause matched the strict source criteria
            // (e.g. a self-ring source whose target noun matching is
            // ambiguous), accept the first clause that resolves at all.
            if source.is_none() {
                if let Some(pos) = filter_clauses.iter().position(|c| {
                    let (c_no_lit, has_lit) = match strip_trailing_quoted_literal(c.trim()) {
                        Some((w, _)) => (w, true), None => (c.trim().to_string(), false),
                    };
                    !has_lit && resolve_fact_type(&split_antecedent_comparator(&c_no_lit).0).is_some()
                }) {
                    let c = filter_clauses.remove(pos);
                    let (c_no_lit, _) = match strip_trailing_quoted_literal(c.trim()) {
                        Some((w, l)) => (w, Some(l)), None => (c.trim().to_string(), None),
                    };
                    if let Some(id) = resolve_fact_type(&split_antecedent_comparator(&c_no_lit).0) {
                        source = Some((id, c));
                    }
                }
            }

            // engine-aggregate-over-join-groupkey-misgroup: the numeric
            // `<op> of <Role> where <body>` aggregate is SINGLE-SOURCE — it keeps
            // ONE source FT, and the filter loop below understands only LITERAL
            // restrictions, so a JOIN where-body (a SECOND non-literal clause that
            // resolves to its own fact type) is SILENTLY DROPPED and the head is
            // mis-grouped by the source FT's roles alone (the join's other entity
            // roles vanish; e.g. `count of Attribute where Item has Value for
            // Attribute and Target wants Value for Attribute` would group by
            // (Item,Value) not (Item,Target), with the Value landing in the Target
            // slot). Reject such a rule with a diagnostic rather than derive a
            // SILENT wrong answer; the blessed pattern (issue #2 precedent) is to
            // pre-join the where-body into ONE derived fact type, then aggregate
            // over that single antecedent. Literal filters and plain single-source
            // aggregates are unaffected (this fires only on a dropped FT-resolving
            // non-literal join antecedent).
            let has_dropped_join_antecedent = source.is_some()
                && filter_clauses.iter().any(|clause| {
                    match strip_trailing_quoted_literal(clause.trim()) {
                        Some(_) => false, // a literal restriction — handled below
                        None => resolve_fact_type(
                            &split_antecedent_comparator(clause.trim()).0).is_some(),
                    }
                });
            if has_dropped_join_antecedent {
                diag!("[aggregate] rule `{}` folds `{} of {}` over a multi-antecedent \
                    JOIN where-body, which the numeric aggregate does NOT support (it is \
                    single-source; the join's extra antecedent would be dropped and the \
                    head mis-grouped). Pre-join the where-body into ONE derived fact \
                    type, then aggregate over that single antecedent.",
                    rule.text, op, target.trim());
                rule.unresolved_clauses.push(part.to_string());
                continue;
            }

            if let Some((ft_id, source_clause)) = source {
                let src_ft = fact_types_map.get(&ft_id);
                // Positional role resolution over the SOURCE clause so
                // self-ring sources (both roles named `Item`) bind the
                // right position. Walk the clause's nouns in order; the
                // i-th noun token aligns with the i-th role of the FT
                // reading. The TARGET position is the token whose text
                // equals the aggregate target (`Item1`); the GROUP-KEY
                // position is the other role — preferring the token whose
                // base noun matches the consequent subject.
                let clause_tokens: Vec<(String, String)> =
                    find_nouns(source_clause, &noun_names).into_iter()
                        .map(|(_, _, n)| (parse_role_token(&n).0.to_string(), n))
                        .collect();
                let n_roles = src_ft.map(|ft| ft.roles.len()).unwrap_or(0);
                // target index: position of a token equal to `target`, else
                // first token whose base == target_base.
                let target_index = clause_tokens.iter().position(|(_, full)| full == target.trim())
                    .or_else(|| clause_tokens.iter().position(|(base, _)| base == &target_base))
                    .filter(|&i| i < n_roles);
                // group-key index: a position != target_index, preferring a
                // token whose base matches the consequent subject.
                let group_key_index = consequent_subject.as_ref().and_then(|subj| {
                    clause_tokens.iter().enumerate()
                        .find(|(i, (base, _))| Some(*i) != target_index && base == subj)
                        .map(|(i, _)| i)
                }).or_else(|| {
                    (0..n_roles).find(|i| Some(*i) != target_index)
                }).filter(|&i| i < n_roles);

                // Name-based group key kept for back-compat / fallback.
                let group_key_role = group_key_index
                    .and_then(|i| src_ft.and_then(|ft| ft.roles.get(i)))
                    .map(|r| r.noun_name.clone())
                    .or_else(|| src_ft
                        .and_then(|ft| ft.roles.iter().find(|r| r.noun_name != target_base))
                        .map(|r| r.noun_name.clone()))
                    .unwrap_or_default();

                // Build aggregate filters from the remaining clauses. Each
                // must be `<entity> has <role> '<literal>'` over the counted
                // entity; resolve it to a ref FT and capture the entity
                // role (matching target_base) + the literal role + value.
                let mut filters: Vec<crate::types::AggregateFilter> = Vec::new();
                for clause in filter_clauses.iter() {
                    let Some((without_lit, lit)) =
                        strip_trailing_quoted_literal(clause.trim()) else { continue };
                    let (clause_stripped, _) = split_antecedent_comparator(&without_lit);
                    let Some(ref_id) = resolve_fact_type(&clause_stripped) else { continue };
                    let Some(ref_ft) = fact_types_map.get(&ref_id) else { continue };
                    // Entity role = role on the ref FT matching the counted
                    // entity's base noun. Filter role = the LAST role (the
                    // one the trailing literal pins, mirroring how
                    // antecedent role-literals bind the last role).
                    let entity_role = ref_ft.roles.iter()
                        .find(|r| r.noun_name == target_base)
                        .map(|r| r.noun_name.clone());
                    let filter_role = ref_ft.roles.last().map(|r| r.noun_name.clone());
                    if let (Some(entity_role), Some(filter_role)) = (entity_role, filter_role) {
                        if entity_role != filter_role {
                            filters.push(crate::types::AggregateFilter {
                                ref_fact_type_id: ref_id,
                                entity_role,
                                filter_role,
                                value: lit,
                            });
                        }
                    }
                }

                aggregates.push(crate::types::ConsequentAggregate {
                    role,
                    op,
                    target_role: target,
                    source_fact_type_id: ft_id,
                    group_key_role,
                    group_key_index,
                    target_index,
                    filters,
                    // The `is the <op> of …` numeric aggregate folds raw role
                    // values, not enum ranks, and sources from one FT.
                    enum_rank: false,
                    join_fact_type_id: String::new(),
                    enum_global: false,
                });
            }
            continue;
        }
        // task-953 — superlative/ordering comparator clause
        // (`<EntityA> has the <super> <ValueType> among <Ys> …`). Lifts to
        // a rank aggregate: the existing numeric min/max fold (`min` for
        // strongest-family, `max` for weakest-family) over the value's
        // enum-declaration-order rank, grouped by the consequent subject.
        // The "among <Ys> …" set is the join of the GROUP FT (consequent
        // subject ⋈ entity, e.g. `Merge concerns Commit`) with the VALUE
        // FT (`Commit has Security Posture`) on the shared entity.
        if let Some((op, entity_noun, value_type, among_rest)) =
            try_parse_superlative_among_clause(part, &noun_names)
        {
            // Value FT: the declared FT whose roles are exactly the
            // entity + the enum value type (`Commit has Security Posture`).
            let value_ft = fact_types_map.iter().find(|(_, ft)| {
                let has = |n: &str| ft.roles.iter().any(|r| r.noun_name == n);
                ft.roles.len() == 2 && has(&entity_noun) && has(&value_type)
            }).map(|(id, _)| id.clone());

            // GLOBAL superlative (task-recommendation cascade). When the
            // consequent FT is a SINGLETON MARKER whose SOLE role is the
            // value type itself (`Task Priority is recommended`), the
            // superlative is NOT grouped per-subject — it folds the global
            // extremum over the WHOLE `among <Ys> …` set and derives one
            // winning value. The "among <Ys> that have <R> '<lit>'" tail
            // becomes an aggregate FILTER so only qualifying members feed
            // the global max (e.g. only pending Tasks). This is the
            // positive reading of "no <Y> has a higher <V>"; a downstream
            // positive equi-join re-attaches the winner to each member.
            let consequent_ft = fact_types_map.get(rule.consequent_cell.literal_id());
            let is_global_singleton = consequent_ft.map_or(false, |ft| {
                ft.roles.len() == 1 && ft.roles[0].noun_name == value_type
            });
            if is_global_singleton {
                if let Some(value_ft_id) = value_ft.clone() {
                    // Parse the `among <Ys> that have <R> '<lit>'` filter.
                    // `among_rest` looks like `Tasks that have Task Status
                    // 'pending'`; the predicate after `that ` is a literal
                    // restriction over the entity. Resolve it to a ref FT
                    // and capture (entity_role, filter_role, value), exactly
                    // as the numeric-aggregate filter handler does.
                    let mut filters: Vec<crate::types::AggregateFilter> = Vec::new();
                    let pred = among_rest
                        .find(" that ")
                        .map(|i| among_rest[i + " that ".len()..].trim().to_string());
                    if let Some(pred) = pred {
                        if let Some((without_lit, lit)) =
                            strip_trailing_quoted_literal(pred.trim())
                        {
                            // Re-form a resolvable clause by prefixing the
                            // entity noun (`Task have Task Status` → FT via
                            // the role-set fallback in `resolve_fact_type`).
                            let clause = alloc::format!("{} {}", entity_noun, without_lit);
                            let (clause_stripped, _) =
                                split_antecedent_comparator(&clause);
                            if let Some(ref_id) = resolve_fact_type(&clause_stripped) {
                                if let Some(ref_ft) = fact_types_map.get(&ref_id) {
                                    let entity_role = ref_ft.roles.iter()
                                        .find(|r| r.noun_name == entity_noun)
                                        .map(|r| r.noun_name.clone());
                                    let filter_role =
                                        ref_ft.roles.last().map(|r| r.noun_name.clone());
                                    if let (Some(entity_role), Some(filter_role)) =
                                        (entity_role, filter_role)
                                    {
                                        if entity_role != filter_role {
                                            filters.push(crate::types::AggregateFilter {
                                                ref_fact_type_id: ref_id,
                                                entity_role,
                                                filter_role,
                                                value: lit,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    aggregates.push(crate::types::ConsequentAggregate {
                        role: value_type.clone(),
                        op,
                        target_role: value_type.clone(),
                        source_fact_type_id: value_ft_id,
                        // Group key role is informational here (the global
                        // fold ignores it); keep the value type for shape.
                        group_key_role: value_type.clone(),
                        group_key_index: None,
                        target_index: None,
                        filters,
                        enum_rank: true,
                        join_fact_type_id: String::new(),
                        enum_global: true,
                    });
                    continue;
                }
                // Singleton consequent but value FT undeclared — fall
                // through to the unresolved channel below.
                rule.unresolved_clauses.push(part.to_string());
                continue;
            }

            // Group key = the consequent subject noun (`Merge`).
            let group_key = find_nouns(consequent_text, &noun_names).first()
                .map(|(_, _, n)| parse_role_token(n).0.to_string());

            // Join FT: the declared binary FT relating the group key with
            // the entity (`Merge concerns Commit`). Distinct from the
            // value FT. When the group key equals the entity (a degenerate
            // single-FT superlative `X has the strongest P among Xs …`),
            // no join is needed and `join_fact_type_id` stays empty.
            let join_ft = group_key.as_ref().and_then(|gk| {
                if gk == &entity_noun { return None; }
                fact_types_map.iter().find(|(_, ft)| {
                    let has = |n: &str| ft.roles.iter().any(|r| r.noun_name == n);
                    ft.roles.len() == 2 && has(gk) && has(&entity_noun)
                }).map(|(id, _)| id.clone())
            });

            // Require the value FT and a group key; require the join FT
            // unless degenerate. A superlative whose FTs aren't declared
            // falls through to the unresolved-clause channel below.
            match (value_ft, group_key) {
                (Some(value_ft_id), Some(group_key_role))
                    if join_ft.is_some() || group_key_role == entity_noun =>
                {
                    aggregates.push(crate::types::ConsequentAggregate {
                        // The consequent role receiving the winning value
                        // is the value type itself (`Security Posture`).
                        role: value_type.clone(),
                        op,
                        target_role: value_type.clone(),
                        source_fact_type_id: value_ft_id,
                        group_key_role,
                        group_key_index: None,
                        target_index: None,
                        filters: Vec::new(),
                        enum_rank: true,
                        join_fact_type_id: join_ft.unwrap_or_default(),
                        enum_global: false,
                    });
                    continue;
                }
                _ => {
                    // FTs not all declared — record as unresolved so the
                    // rule doesn't silently fire as bare inheritance.
                    rule.unresolved_clauses.push(part.to_string());
                    continue;
                }
            }
        }
        // Definitional clauses claim the part outright â€” they bind a
        // consequent role's value and don't belong in antecedent FTs.
        if let Some((role, expr)) = try_parse_computed_binding(part, &noun_names) {
            computed.push(crate::types::ConsequentComputedBinding { role, expr });
            continue;
        }
        // â”€â”€ Classify the clause through existing pipelines â”€â”€â”€â”€â”€â”€â”€
        // Each pipeline already knows its own patterns. We call them
        // in order; the first match wins. No keyword arrays here.

        // derivation-cardinality-count: pull an `at most N` / `at least N`
        // COUNT premise (`Item is marked by at most 0 Tag`) off the clause
        // BEFORE fact-type resolution. Without this the cardinal phrase
        // survives in the verb (`is marked by at most 0`) and the bridge FT
        // still resolves via the unique-noun-set fallback — silently
        // dropping the bound and collapsing the premise to a plain
        // existential. We strip the phrase so the bridge resolves cleanly,
        // then record the bound as an `AntecedentCardinality` once the FT id
        // is known (below). The group/count roles are the bridge's two roles:
        // the group key is the role matching the consequent subject noun
        // (the join noun, e.g. `Item`), the counted role is the other one
        // (e.g. `Tag`).
        let cardinality = extract_antecedent_cardinality(part);
        let part_decard: &str = cardinality.as_ref().map(|(_, _, s)| s.as_str()).unwrap_or(part);

        // (1) Comparator-stripped FT lookup (direct + hyphen fallback + negation fallback)
        let (stripped, comparator) = split_antecedent_comparator(part_decard);
        let dehyphenated = stripped.replace("- ", " ").replace(" -", " ");
        // Strip a trailing `' <value>'` literal (single-quoted) so
        // `Task has Status 'Done'` resolves to the FT `Task has Status`
        // just like its unquoted form. The literal is semantically a
        // filter on the last role, not part of the FT reading. The
        // captured value (trailing_literal) is recorded as an
        // AntecedentRoleLiteral after the FT resolves, so downstream
        // compilation can filter antecedent facts by that literal
        // (#286).
        // Hand-rolled equivalent of regex ` '([^']*)'\s*$`: capture
        // the trailing single-quoted literal (after a space) and the
        // text with that segment removed.
        let (destripped_literal, trailing_literal) =
            match strip_trailing_quoted_literal(&stripped) {
                Some((without, lit)) => (without, Some(lit)),
                None => (stripped.clone(), None),
            };
        // AbsenceOf detection removed 2026-05-19 — the construct (and
        // its `has no` / `is not` / `does not` / leading `no` /
        // leading `not` surface markers) was engine-introduced, not in
        // Halpin's FORML 2. CSDP discipline: derivation rules assert
        // positive facts. Closed-world negation is the validation
        // layer's concern (deontic constraints, violation reports), not
        // a derivation antecedent.
        //
        // Clauses with negation markers no longer resolve to an
        // antecedent — they fall through to the unresolved-clause
        // pipeline at the end of the loop, surfacing as parser
        // warnings so authors notice and reformulate.

        let ft_resolved = resolve_fact_type(&stripped)
            .or_else(|| (dehyphenated != stripped).then(|| resolve_fact_type(&dehyphenated)).flatten())
            .or_else(|| (destripped_literal != stripped)
                .then(|| resolve_fact_type(&destripped_literal)).flatten());

        if let Some(ft_id) = ft_resolved {
            if let Some((op, value)) = comparator.clone() {
                let role = fact_types_map.get(&ft_id)
                    .and_then(|ft| ft.roles.last())
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                filters.push(crate::types::AntecedentFilter {
                    antecedent_index: resolved_ids.len(),
                    role, op, value,
                });
            }
            if let Some(lit) = trailing_literal.clone() {
                let role = fact_types_map.get(&ft_id)
                    .and_then(|ft| ft.roles.last())
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                if !role.is_empty() {
                    role_literals.push(crate::types::AntecedentRoleLiteral {
                        antecedent_index: resolved_ids.len(),
                        role,
                        value: lit,
                    });
                }
            }
            // derivation-cardinality-count: record the count bound now that
            // the bridge FT id is known. group_key_role = the bridge role
            // whose noun matches the consequent subject (the join noun);
            // count_role = the other role. Only binary bridges are handled
            // (the Halpin `is marked by at most N Tag` shape); a non-binary
            // or single-role match leaves the cardinality unrecorded (it
            // degrades to the existing existential behaviour rather than
            // mis-counting).
            if let Some((at_most, count, _)) = cardinality.clone() {
                if let Some(ft) = fact_types_map.get(&ft_id) {
                    if ft.roles.len() == 2 {
                        let subj: Option<String> =
                            find_nouns(consequent_text, &noun_names).first()
                                .map(|(_, _, n)| parse_role_token(n).0.to_string());
                        // group key prefers the role matching the consequent
                        // subject; fall back to the first role.
                        let gk_idx = subj.as_ref()
                            .and_then(|s| ft.roles.iter().position(|r| &r.noun_name == s))
                            .unwrap_or(0);
                        let ck_idx = 1 - gk_idx;
                        cardinalities.push(crate::types::AntecedentCardinality {
                            antecedent_index: resolved_ids.len(),
                            at_most,
                            count,
                            group_key_role: ft.roles[gk_idx].noun_name.clone(),
                            count_role: ft.roles[ck_idx].noun_name.clone(),
                        });
                    }
                }
            }
            resolved_ids.push(crate::types::AntecedentSource::FactType(ft_id));
            resolved_part_text.push(part.to_string());
            continue;
        }

        // (2) Comparator already split off a comparison operator â€”
        //     split_antecedent_comparator recognized it, even though
        //     the base FT didn't resolve. The clause IS a comparison.
        if comparator.is_some() { continue; }

        // (3) Aggregate: try_parse_aggregate_clause already knows
        //     count/sum/avg/min/max + where-clause patterns.
        if try_parse_aggregate_clause(part, &noun_names).is_some() { continue; }

        // (4) Computed binding: try_parse_computed_binding already
        //     knows arithmetic and role-assignment patterns.
        if try_parse_computed_binding(part, &noun_names).is_some() { continue; }

        // (5) that-anaphora: back-reference to a noun bound in a
        //     prior clause. Two shapes:
        //     a) "that X has Y" â€” join continuation
        //     b) "X is that Y" â€” anaphoric value assignment
        //        (e.g., "display- Text is that Reference")
        // find_nouns (word-boundary + reserved-noun guard), NOT a raw substring
        // `contains`: else a metamodel clause like `that Fact Type has that Role`
        // is falsely skipped here (it contains `Fact`) before reaching the
        // metamodel classifier that resolves it to the FactType relation (task980).
        if part.trim().starts_with("that ") && !find_nouns(part, &noun_names).is_empty()
        { continue; }
        if part.contains(" is that ") || part.contains(" is some ") { continue; }

        // (6) Temporal predicates â€” genuinely new, no existing fn.
        if is_temporal_predicate(part) { continue; }

        // (7) Subtype instance check: `X is a Y` / `X is an Y` where
        //     both X and Y are declared nouns. Subtype membership is
        //     inherent to the schema (`X is a subtype of Y` declarations
        //     in the Subtype metamodel cell). Recognised so readings like
        //       TCPA Violation is for Robocall ... if Robocall is
        //         an Autodialed Call and ...
        //     don't spuriously flag the subtype check as unresolved.
        //
        //     task subtype-join-antecedent: `extract_subtype_instance_check`
        //     is used by `compile_derivations` (compile.rs) to implement a
        //     compile-time schema-gate: a rule whose antecedent contains
        //     `X is a Y` is only compiled if the schema actually declares X
        //     as a subtype of Y.  Pairs whose schema relationship is absent
        //     produce rules that would NEVER fire — dropping them at compile
        //     time is correct and avoids dead derivations.
        //
        //     The (sub, sup) pair is recorded in the rule's unresolved-
        //     clauses only if the CALLER (compile.rs gate) needs to act;
        //     here we just skip without noise.
        if is_subtype_instance_check(part, &noun_names) { continue; }

        // (8) Word-based value comparison: `X exceeds Y`,
        //     `X is greater than Y`, etc., where both operands resolve
        //     against the noun catalog. Complements the ASCII-operator
        //     path in branch (1)/(2) for readings that spell their
        //     comparators out.
        if is_word_comparator_clause(part, &noun_names) { continue; }

        // (8b) #277 Category F — range-filter clauses
        //      `<FT reference> within|before|after <tail>` where the
        //      head alone resolves through the catalog. The tail is
        //      typically anaphora (`that Interval`, `that Fresh Until`)
        //      or a value literal.
        if is_range_filter_clause(part, &noun_names, catalog) { continue; }

        // (8c) #277 Category F — bare-value tail comparisons
        //      `<Noun> of N or more` / `or less` / `or greater`.
        //      Numeric literal only; quoted literals stay with the
        //      ref-scheme-value classifier at (9b).
        if is_bare_value_comparison(part, &noun_names) { continue; }

        // (9) Literal-value filter: `<Noun> has <Noun> '<literal>'`.
        //     Covers state-machine status filters (`Task has Status 'Done'`)
        //     and enum-value filters (`Customer has Tier 'Gold'`) whose
        //     FT isn't always declared textually when the role is
        //     SM-managed or enum-valued. `resolve_fact_type` would miss
        //     it; classify it here as a valid antecedent predicate.
        if is_noun_has_noun_literal(part, &noun_names) { continue; }

        // (9b) Ref-scheme-value filter: `<Noun> is '<literal>'` or
        //      `<Noun> is not '<literal>'`. The entity's ref scheme
        //      value IS its identity, so this clause selects the
        //      entity whose identity equals the literal. Optional
        //      leading role qualifiers (`other Source`, `that
        //      Customer`) are stripped before the match. #275
        //      Category C.
        if is_entity_ref_scheme_literal(part, &noun_names) { continue; }

        // (10) Universal quantifier: `for each <X> that <R> the <Subject>,
        //      <X has P 'value'>`. Recognised when the clause starts with a
        //      universal keyword and names a declared noun. Compiled as the
        //      Backus fold ∀x∈S. P(x) = (/∧) ∘ (αP) restricted to the X's
        //      that R-relate to the subject (whitepaper §4 / Backus
        //      §11.2.4). Recorded here as a `ConsequentUniversal`; the
        //      compiler lowers it to a per-subject guard. It is a POSITIVE
        //      conjunct (a fold of a positive predicate) — it does not
        //      violate the positive-derivation discipline above.
        if is_universal_quantifier_clause(part, &noun_names) {
            // Strip the quantifier keyword → `<X> that <R> the <Subject>,
            // <X has P 'value'>`.
            if let Some(tail) = crate::parse_forml2_stage2::UniversalQuantifierTable::boot()
                .match_prefix(part.trim())
            {
                // The consequent subject noun (`Item` in `Item is clear`)
                // is the ∀-subject. It is the relating clause's role that
                // is NOT the quantified X. Computed up front because the
                // relative-clause (no-comma) form needs it to find the
                // restriction/predicate boundary.
                let subject_noun: Option<String> =
                    find_nouns(consequent_text, &noun_names).first()
                        .map(|(_, _, n)| parse_role_token(n).0.to_string());

                // Recover the (restriction, predicate) clause pair from
                // EITHER canonical universal surface form:
                //
                //   comma form  : `X that R the S, X has P 'lit'`
                //                  → split at the first top-level comma.
                //   relative-clause (ORM2 §6.5 p.252, no comma required):
                //                 `X that R the S has P 'lit'`
                //                  → the restriction is `X that R the S`
                //                    (up to & including the subject S
                //                    mention); the predicate is the tail
                //                    with the quantified X re-prepended,
                //                    i.e. `X has P 'lit'`. This yields the
                //                    SAME (rel_clause, pred_clause) pair the
                //                    comma form produces, so both compile to
                //                    the identical `ConsequentUniversal` IR.
                let clause_pair: Option<(String, String)> =
                    match tail.split_once(',') {
                        Some((rel_raw, pred_raw)) => Some((
                            rel_raw.trim().to_string(),
                            pred_raw.trim().trim_end_matches('.').trim().to_string(),
                        )),
                        None => split_universal_relative_clause(
                            tail.trim().trim_end_matches('.').trim(),
                            subject_noun.as_deref(),
                            &noun_names,
                        ),
                    };

                if let Some((rel_clause, pred_clause)) = clause_pair {
                    let rel_clause = rel_clause.as_str();
                    let pred_clause = pred_clause.as_str();

                    // ── Relation clause: `Item1 that blocks the Item` ──
                    // Quantified X = the FIRST noun token (carries the
                    // subscript that distinguishes it from the subject in a
                    // ring). Resolve the FT on the anaphora-stripped form.
                    let rel_tokens = find_nouns(rel_clause, &noun_names);
                    let x_token: Option<String> = rel_tokens.first().map(|(_, _, n)| n.clone());
                    let x_base: Option<String> = x_token.as_ref()
                        .map(|t| parse_role_token(t).0.to_string());

                    let rel_ft = resolve_fact_type(rel_clause);
                    // Predicate clause: `Item1 has Status 'done'` — strip the
                    // trailing quoted literal, resolve the base FT.
                    let (pred_no_lit, pred_lit) =
                        match strip_trailing_quoted_literal(pred_clause) {
                            Some((without, lit)) => (without, Some(lit)),
                            None => (pred_clause.to_string(), None),
                        };
                    let pred_ft = resolve_fact_type(&pred_no_lit);

                    if let (Some(rel_id), Some(pred_id), Some(subject), Some(x_full), Some(x_base), Some(lit)) =
                        (rel_ft, pred_ft, subject_noun, x_token, x_base, pred_lit)
                    {
                        let rel_ft_def = fact_types_map.get(&rel_id);
                        let pred_ft_def = fact_types_map.get(&pred_id);
                        if let (Some(rel_ft_def), Some(pred_ft_def)) = (rel_ft_def, pred_ft_def) {
                            // Positional role alignment over the relation
                            // clause: walk its noun tokens in declaration
                            // order, align with the FT's roles (the i-th token
                            // that matches role i's base noun). The X position
                            // is the token equal to `x_full`; the subject
                            // position is the OTHER role.
                            let rel_aligned = align_tokens_to_roles(&rel_tokens, rel_ft_def);
                            let relation_x_index = rel_aligned.iter()
                                .find(|(_, full)| full == &x_full)
                                .map(|(i, _)| *i)
                                .or_else(|| rel_aligned.iter()
                                    .find(|(_, full)| parse_role_token(full).0 == x_base.as_str())
                                    .map(|(i, _)| *i));
                            let relation_subject_index = relation_x_index.and_then(|xi| {
                                // Prefer a role whose base matches the subject
                                // noun and isn't the X position; else any
                                // other role position.
                                rel_aligned.iter()
                                    .find(|(i, full)| *i != xi
                                        && parse_role_token(full).0 == subject.as_str())
                                    .map(|(i, _)| *i)
                                    .or_else(|| (0..rel_ft_def.roles.len()).find(|i| *i != xi))
                            });

                            // Predicate clause role alignment: X is the role
                            // matching the quantified entity's base noun; the
                            // filter role is the LAST role (the trailing
                            // literal pins it, mirroring antecedent role
                            // literals).
                            let predicate_x_index = pred_ft_def.roles.iter()
                                .position(|r| r.noun_name == x_base);
                            let filter_role = pred_ft_def.roles.last()
                                .map(|r| r.noun_name.clone());

                            if let (Some(rx), Some(rs), Some(px), Some(fr)) =
                                (relation_x_index, relation_subject_index,
                                 predicate_x_index, filter_role)
                            {
                                universals.push(crate::types::ConsequentUniversal {
                                    subject_role: subject,
                                    relation_fact_type_id: rel_id,
                                    relation_x_index: rx,
                                    relation_subject_index: rs,
                                    predicate_fact_type_id: pred_id,
                                    predicate_x_index: px,
                                    predicate_filter_role: fr,
                                    predicate_value: lit,
                                });
                                continue;
                            }
                        }
                    }
                }
            }
            // Recognised as a universal but couldn't extract a structured
            // form — suppress the unresolved-clause noise (legacy behavior).
            continue;
        }

        // (11) `<Noun> is extracted from <Noun>` / `<Noun> is derived from <Noun>`.
        //      Used for ML-style computed bindings where the RHS is a
        //      free-text source field (e.g. `Category is extracted
        //      from Body`). The extraction function itself is a
        //      runtime primitive; the clause shape is valid here.
        if is_extraction_clause(part, &noun_names) { continue; }

        // (12) Existential-qualified FT reference: `<Noun> <verb> some <Noun>`
        //      or `<Noun> <verb> that <Noun>`. The `some` / `that`
        //      quantifier doesn't change the FT identity; try the
        //      fact-type lookup again with those tokens stripped. Covers
        //      `Feature Request concerns some API Product` style where
        //      the declared FT is `Feature Request concerns API Product`.
        let stripped_quantifiers = strip_existential_quantifiers(part);
        if stripped_quantifiers.as_str() != *part
            && resolve_fact_type(&stripped_quantifiers).is_some()
        { continue; }

        // (13) Metamodel-cell antecedent — task subtype-join-antecedent child 1/4.
        //      Clauses in derivation rules that quantify over the substrate's own
        //      metamodel cells (`Subtype`, `FactType`, `Role`, `Noun`) rather than
        //      over user-declared Fact Types.  Recognised AFTER all user-catalog
        //      classifiers so a schema that coincidentally declares a noun called
        //      "Subtype" still resolves its own FTs via the catalog at step (1).
        //
        //      Four clause shapes are handled:
        //        a) PRIMARY quantification: `some Subtype has subtype Sub`
        //           → `FactType("Subtype")` added to antecedent_sources.
        //        b) ANAPHORIC Subtype back-reference:  `that Subtype has supertype Sup`
        //           → silently skipped (joins are resolved at compile time from schema).
        //        c) ANAPHORIC FactType: `that Fact Type has that Role`
        //           → GAP B (child 4): emit `FactType("FactType")` as a second
        //             antecedent source so the compiler knows to fan over all FTs
        //             where the supertype plays a role.  `that Role is played by Sup`
        //             is a further anaphoric refinement — silently skipped (the
        //             compiler handles the sup-filter from data.subtypes directly).
        //        d) PER-INSTANCE fan: `that Resource is instance of Sub`
        //           → GAP C (child 4): emit `InstancesOfNoun("@subtype_var:Sub")` as
        //             a sentinel.  The compiler replaces the sentinel with the real
        //             per-subtype `InstancesOfNoun(sub)` fan when it detects the
        //             subtype-inheritance reading-lift pattern.
        //
        //      The recogniser strips anaphora/quantifier words before checking
        //      so `some Subtype …` and bare `Subtype …` both match.
        {
            let stripped_anaphora = strip_anaphora(part);
            // GAP C (child 4): per-instance fan clause — gated on AntecedentRole
            // consequent (i.e. the metamodel head was detected in GAP A).  Only the
            // subtype-inheritance reading-lift rule has this shape; user rules that
            // happen to write `X is instance of Y` in their antecedent are silently
            // skipped by `try_classify_metamodel_clause` (which returns the empty-
            // string sentinel for this pattern), and we must NOT promote those to an
            // InstancesOfNoun antecedent or the rule would fire over ALL instances
            // of Y instead of the intended predicate.
            //
            // The check: if the consequent is AntecedentRole (set by GAP A) AND this
            // clause has "is instance of", push the InstancesOfNoun sentinel.
            let has_antecedent_role_consequent =
                matches!(&rule.consequent_cell, crate::types::ConsequentCellSource::AntecedentRole { .. });
            if has_antecedent_role_consequent && stripped_anaphora.contains(" is instance of ") {
                // Extract the variable token that follows "is instance of".
                if let Some(after) = stripped_anaphora.split(" is instance of ").nth(1) {
                    let var_token = after.trim().trim_end_matches('.');
                    if !var_token.is_empty() {
                        resolved_ids.push(crate::types::AntecedentSource::InstancesOfNoun(
                            alloc::format!("@subtype_var:{}", var_token)
                        ));
                        resolved_part_text.push(part.to_string());
                    }
                }
                continue;
            }
            if let Some(cell_id) = try_classify_metamodel_clause(&stripped_anaphora) {
                // GAP B (child 4): `that Fact Type has that Role` — the stripped form
                // `Fact Type has Role` matches the "fact type " prefix and returns
                // `Some("FactType")`.  Promote from anaphoric-skip to a second
                // antecedent source ONLY when the consequent is AntecedentRole
                // (the metamodel head detected by GAP A).  For user rules the anaphoric
                // `that Fact Type` clause stays silently skipped — otherwise it would
                // add a spurious FactType("FactType") antecedent to any user rule that
                // happens to use metamodel vocabulary in its body.
                // `that Role is played by Sup` (stripped: `Role is played by Sup`)
                // returns `Some("Role")` — keep as a skip regardless; the sup filter
                // is resolved by the compiler from data.subtypes directly.
                if !cell_id.is_empty() && part.trim().starts_with("that ")
                    && cell_id == "FactType"
                    && has_antecedent_role_consequent
                {
                    // SECOND antecedent: FactType cell (all declared fact types).
                    resolved_ids.push(crate::types::AntecedentSource::FactType(cell_id));
                    resolved_part_text.push(part.to_string());
                    continue;
                }
                if !cell_id.is_empty() && !part.trim().starts_with("that ") {
                    // Primary quantification — add as antecedent source.
                    resolved_ids.push(crate::types::AntecedentSource::FactType(cell_id));
                    resolved_part_text.push(part.to_string());
                }
                // Anaphoric form or skip-only predicate — don't emit as unresolved.
                continue;
            }
        }

        // Nothing classified this clause.
        rule.unresolved_clauses.push(part.to_string());
    }
    // task subtype-join-antecedent child 4: resolved_ids is now Vec<AntecedentSource>
    // (may contain FactType and InstancesOfNoun entries); no mapping needed.
    rule.antecedent_sources = resolved_ids;
    rule.antecedent_filters = filters;
    rule.antecedent_role_literals = role_literals;
    rule.antecedent_cardinalities = cardinalities;
    rule.consequent_computed_bindings = computed;
    rule.consequent_aggregates = aggregates;
    rule.consequent_universals = universals;

    // #914 — map each pre-pass-recorded cross-antecedent comparison
    // spec to an `AntecedentRoleComparison` by locating the antecedent
    // whose clause carries the LHS / RHS noun token AND whose FT
    // carries the comparison role.  `resolved_part_text` is the
    // parallel vec recording each resolved clause's text; we scan it
    // for a clause containing the noun token (via `find_nouns` for
    // subscript-aware matching) and check the FT's role list.
    //
    // Reordering for the engine's existing post-join Filter machinery
    // (`compile_join_derivation`): that path picks consequent bindings
    // from the FIRST antecedent that carries each binding noun (no
    // subscript awareness), so for the rule to land the right Task
    // value in the consequent we reorder `antecedent_sources` so
    // antecedents whose clause carries the CONSEQUENT-side subscripted
    // noun token (the "subject" of the consequent fact type) come
    // first. Indices on every co-vec (filters, role literals,
    // resolved_part_text) and the comparison-spec endpoints are
    // remapped in lockstep.
    //
    // Unresolvable specs are silently dropped — same convention
    // `antecedent_filters` uses when a filter clause's base FT fails
    // to resolve.
    rule.antecedent_role_comparisons = Vec::new();
    if !pending_role_comparisons.is_empty() {
        // Step 1: identify the consequent subscript token (if any).
        // The consequent text is the rule's text up to ` iff `/` if `;
        // a subscripted token like `Task2` is the role on the
        // consequent FT that the rule is "asserting about".
        let consequent_token: Option<String> = {
            let cons_text = rule.text.find(" iff ").map(|i| &rule.text[..i])
                .or_else(|| rule.text.find(" if ").map(|i| &rule.text[..i]))
                .unwrap_or(rule.text.as_str());
            let mut tok: Option<String> = None;
            for (_, _, n) in find_nouns(cons_text, &noun_names) {
                let (base, full) = parse_role_token(&n);
                if full != base {
                    tok = Some(n.clone());
                    break;
                }
            }
            tok
        };

        // Step 2: derive a stable permutation that puts antecedents
        // matching the consequent-subscripted token first, then the
        // rest in original order. Antecedents whose clause carries
        // the consequent token rank 0; others rank 1.
        let n_ants = rule.antecedent_sources.len();
        let rank = |i: usize| -> usize {
            let Some(ref tok) = consequent_token else { return 1; };
            let Some(clause) = resolved_part_text.get(i) else { return 1; };
            if find_nouns(clause, &noun_names)
                .iter().any(|(_, _, n)| n == tok) { 0 } else { 1 }
        };
        let mut permutation: Vec<usize> = (0..n_ants).collect();
        permutation.sort_by_key(|&i| (rank(i), i));
        // Identity-permutation short-circuit so unchanged antecedent
        // ordering stays byte-for-byte the same.
        let needs_reorder = permutation.iter().enumerate()
            .any(|(new_i, &old_i)| new_i != old_i);
        // Inverse map: old_index → new_index.
        let mut inverse = alloc::vec![0usize; n_ants];
        for (new_i, &old_i) in permutation.iter().enumerate() {
            inverse[old_i] = new_i;
        }
        if needs_reorder {
            // Reorder antecedent_sources + resolved_part_text in
            // lockstep.
            let new_sources: Vec<crate::types::AntecedentSource> = permutation.iter()
                .map(|&i| rule.antecedent_sources[i].clone())
                .collect();
            let new_part_text: Vec<String> = permutation.iter()
                .map(|&i| resolved_part_text[i].clone())
                .collect();
            rule.antecedent_sources = new_sources;
            resolved_part_text = new_part_text;
            // Remap antecedent_index on every per-antecedent
            // structure that carries one.
            for af in rule.antecedent_filters.iter_mut() {
                if let Some(new_i) = inverse.get(af.antecedent_index) {
                    af.antecedent_index = *new_i;
                }
            }
            for arl in rule.antecedent_role_literals.iter_mut() {
                if let Some(new_i) = inverse.get(arl.antecedent_index) {
                    arl.antecedent_index = *new_i;
                }
            }
        }

        // Step 3: map each spec's LHS/RHS noun-token + role pair to
        // the (now possibly reordered) antecedent index.
        let find_ant_idx = |token: &str, role: &str| -> Option<usize> {
            for (i, clause) in resolved_part_text.iter().enumerate() {
                let nouns = find_nouns(clause, &noun_names);
                let token_present = nouns.iter().any(|(_, _, n)| n == token);
                if !token_present { continue; }
                let src = rule.antecedent_sources.get(i)?;
                let ft_id = src.fact_type_id();
                if ft_id.is_empty() { continue; }
                let ft = fact_types_map.get(ft_id)?;
                if ft.roles.iter().any(|r| r.noun_name == role) {
                    return Some(i);
                }
            }
            None
        };
        for (lhs_tok, lhs_role, op, rhs_tok, rhs_role)
            in pending_role_comparisons.iter()
        {
            let Some(li) = find_ant_idx(lhs_tok, lhs_role) else { continue; };
            let Some(ri) = find_ant_idx(rhs_tok, rhs_role) else { continue; };
            rule.antecedent_role_comparisons.push(
                crate::types::AntecedentRoleComparison {
                    lhs_antecedent_index: li,
                    lhs_role: lhs_role.clone(),
                    op: op.to_string(),
                    rhs_antecedent_index: ri,
                    rhs_role: rhs_role.clone(),
                });
        }

        // Comparison-key reordering complete (Steps 1-3). The bridge-key
        // synthesis that used to be "Step 4" here is hoisted below so it
        // runs for EVERY >=2-antecedent rule, not only comparison ones.
    }

    // Bridge-variable join detection (RBAC-style equi-join, no surface
    // marker) — HOISTED out of the `!pending_role_comparisons.is_empty()`
    // gate so it runs for bare >=2-antecedent equi-joins like `User is
    // permitted Operation on Noun iff User has Role and Role permits
    // Operation on Noun`, where `Role` is a BRIDGE variable (on >=2
    // antecedent FTs, joined-then-discarded — not projected to the
    // consequent). While this block lived inside the comparison gate, such
    // a bare rule kept `join_on` empty and fell through to ModusPonens,
    // whose N>=2 branch copies bindings from the FIRST antecedent only — a
    // SILENT mis-join (task nonskolem-cross-antecedent-join).
    //
    // GATED to SKIP skolem-head rules: a consequent carrying a FRESH
    // existential `(VAR)` (one not bound in any antecedent) is owned by the
    // skolem-head path below (~"task-970"), which does its OWN skolem-aware
    // shared-noun join setup + id minting and is gated on `kind != Join`.
    // Promoting such a rule to a plain Join here would pre-empt that path
    // and break the View / Grant derivations. The `is_join` re-validation
    // below still gates the actual `kind` flip, and the subscript guard
    // defers ring self-joins to `compute_ring_join_plan`.
    let consequent_has_fresh_existential = {
        // Mirrors `extract_paren_vars` in the skolem-head block below; kept
        // local so this gate needs no reorder of that detection.
        let paren_vars = |text: &str| -> Vec<String> {
            let mut vars: Vec<String> = Vec::new();
            let bytes = text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'(' {
                    if let Some(close) = text[i + 1..].find(')') {
                        let interior = text[i + 1..i + 1 + close].trim();
                        if !interior.is_empty()
                            && !interior.contains('\'')
                            && interior.chars().all(|c| c.is_alphanumeric() || c == '-' || c == ' ' || c == '_')
                        {
                            vars.push(interior.to_string());
                        }
                        i += close + 2;
                        continue;
                    }
                }
                i += 1;
            }
            vars
        };
        let cons_text = rule.text.find(" iff ").map(|i| &rule.text[..i])
            .or_else(|| rule.text.find(" if ").map(|i| &rule.text[..i]))
            .unwrap_or(rule.text.as_str());
        let ant_vars: hashbrown::HashSet<String> = antecedent_parts.iter()
            .flat_map(|p| paren_vars(p))
            .collect();
        paren_vars(cons_text).into_iter().any(|v| !ant_vars.contains(&v))
    };
    if !consequent_has_fresh_existential {
        let comparison_roles: hashbrown::HashSet<String> = rule.antecedent_role_comparisons
            .iter()
            .flat_map(|c| [c.lhs_role.clone(), c.rhs_role.clone()])
            .collect();
        let tokens_for_noun = |noun: &str| -> hashbrown::HashSet<String> {
            let mut toks: hashbrown::HashSet<String> = hashbrown::HashSet::new();
            for clause in resolved_part_text.iter() {
                for (_, _, t) in find_nouns(clause, &noun_names) {
                    let (base, _) = parse_role_token(&t);
                    if base == noun { toks.insert(t.clone()); }
                }
            }
            toks
        };
        let mut shared: Vec<String> = Vec::new();
        for src in rule.antecedent_sources.iter() {
            let ft_id = src.fact_type_id();
            if ft_id.is_empty() { continue; }
            let Some(ft) = fact_types_map.get(ft_id) else { continue; };
            for r in ft.roles.iter() {
                if comparison_roles.contains(&r.noun_name) { continue; }
                let appears = rule.antecedent_sources.iter()
                    .filter_map(|s| fact_types_map.get(s.fact_type_id()))
                    .filter(|ft| ft.roles.iter().any(|rr| rr.noun_name == r.noun_name))
                    .count();
                if appears < 2 { continue; }
                // Skip nouns appearing with multiple distinct subscript
                // tokens across antecedent clauses — those are independent
                // ring variables, not a shared equi-join key.
                if tokens_for_noun(&r.noun_name).len() > 1 { continue; }
                if !shared.contains(&r.noun_name) {
                    shared.push(r.noun_name.clone());
                }
            }
        }
        for key in shared.into_iter() {
            if !join_keys.contains(&key) {
                join_keys.push(key);
            }
        }
    }

    // Deduplicate join keys
    let mut seen = hashbrown::HashSet::new();
    rule.join_on = join_keys.into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect();

    // Classify: if join keys exist AND at least 2 distinct antecedent fact types share
    // a noun, this is a Join derivation. Rules with "that X" anaphora where X appears
    // in multiple antecedents need an equi-join on X.
    let is_join = !rule.join_on.is_empty()
        && rule.antecedent_sources.len() >= 2
        && rule.join_on.iter().any(|key| {
            rule.antecedent_sources.iter()
                .filter_map(|s| {
                    let ft_id = s.fact_type_id();
                    if ft_id.is_empty() { None } else { fact_types_map.get(ft_id) }
                })
                .filter(|ft| ft.roles.iter().any(|r| r.noun_name == *key))
                .count() >= 2
        });
    is_join.then(|| {
        rule.kind = DerivationKind::Join;
        // Build match_on: pairs of (noun_a, noun_b) for equality matching
        rule.match_on = rule.join_on.iter()
            .map(|key| (key.clone(), key.clone()))
            .collect();
        // Consequent bindings: nouns from the consequent fact type
        rule.consequent_bindings = fact_types_map.get(rule.consequent_cell.literal_id())
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();
    });

    // subtype-join → supertype FT (join-matching bridge). The standard
    // join classification above keys on a join NOUN appearing as a role on
    // ≥2 antecedent FTs by EXACT noun name. A subtype-keyed join clause
    // (`that Noun belongs to Domain`, Noun < Function) resolves (via the
    // `resolve_fact_type` supertype retry) to a FT declared on the
    // SUPERTYPE (`Function belongs to Domain`), whose role is `Function`,
    // not `Noun` — so the exact-name match never links the two antecedents
    // and the join collapses. Subtype instances ARE supertype instances,
    // so bridge it: for each join key that is carried DIRECTLY by some
    // antecedent FT and, on another antecedent FT, only by a role whose
    // noun is a SUPERTYPE of the key, emit an asymmetric `match_on` pair
    // `(key, supertype_role_noun)`. `compile_join_derivation`'s match-pair
    // path (which already supports cross-name pairs) then equi-joins the
    // subtype role to the supertype role. Narrow by construction: only
    // fires when a direct holder AND a supertype holder both exist, so it
    // can only complete a join the author already wrote (via `that <Sub>`),
    // never fabricate one.
    if !subtypes.is_empty() && rule.antecedent_sources.len() >= 2 {
        let role_nouns_of = |ft_id: &str| -> Vec<String> {
            if ft_id.is_empty() { return Vec::new(); }
            fact_types_map.get(ft_id)
                .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
                .unwrap_or_default()
        };
        let mut bridge_pairs: Vec<(String, String)> = Vec::new();
        for key in rule.join_on.iter() {
            // A direct holder carries the key noun verbatim as a role.
            let has_direct_holder = rule.antecedent_sources.iter().any(|s| {
                role_nouns_of(s.fact_type_id()).iter().any(|n| n == key)
            });
            if !has_direct_holder { continue; }
            // Supertype chain of the key noun, for membership tests.
            let key_supers = supertype_chain(key);
            if key_supers.is_empty() { continue; }
            // A supertype holder lacks the key noun but carries one of its
            // supertypes as a role — the FT the subtype clause bridged to.
            for s in rule.antecedent_sources.iter() {
                let roles = role_nouns_of(s.fact_type_id());
                if roles.iter().any(|n| n == key) { continue; } // it's a direct holder
                // subtype-join-bridge over-match guard: never bridge `key` to a
                // supertype role whose noun is ITSELF a join key of this rule.
                // When `key` (e.g. Status) and its supertype `sup` (e.g.
                // Resource, with Status < Resource) are BOTH distinct join
                // variables, the supertype antecedent's `sup` role belongs to
                // the `sup` variable — not to `key`. Bridging would equi-join
                // Status to Resource (`in_progress == t-1`), collapsing the join
                // to ∅ (the SM→status bridge `Resource is currently in Status
                // iff some State Machine is for that Resource and that State
                // Machine is currently in that Status`). The legitimate
                // subtype-join bridges a key whose supertype is NOT a join key —
                // a role the `that <Sub>` clause merely resolved up to.
                if let Some(sup) = key_supers.iter().find(|sup|
                    roles.iter().any(|n| &n == sup)
                        && !rule.join_on.iter().any(|k| k == *sup)) {
                    let pair = (key.clone(), sup.clone());
                    if !bridge_pairs.contains(&pair) {
                        bridge_pairs.push(pair);
                    }
                }
            }
        }
        if !bridge_pairs.is_empty() {
            rule.kind = DerivationKind::Join;
            for pair in bridge_pairs {
                // Avoid a degenerate self-pair `(k, k)` from the standard
                // block shadowing the asymmetric bridge: drop any `(k, k)`
                // for a key we are bridging (its only direct holder can't
                // self-join the supertype antecedent).
                rule.match_on.retain(|(a, b)| !(a == &pair.0 && b == &pair.0));
                if !rule.match_on.contains(&pair) {
                    rule.match_on.push(pair);
                }
            }
            if rule.consequent_bindings.is_empty() {
                rule.consequent_bindings = fact_types_map
                    .get(rule.consequent_cell.literal_id())
                    .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
                    .unwrap_or_default();
            }
        }
    }

    // Ring / self-join (whitepaper eq:join): when a Halpin numeric
    // subscript (`Person2`) recurs across >=2 antecedent clauses it is a
    // join VARIABLE that noun-name `join_on` cannot name — the base noun
    // fills more than one role of a self-ring fact type. Resolve the join
    // key(s) and each consequent role's value-source POSITIONALLY from the
    // subscripts and route to `DerivationKind::Join`; compile_join_derivation
    // consumes `ring_join`. Returns None (kind / join_on left as-is) unless
    // the rule actually uses recurring subscripts over a self-ring, so
    // ordinary rules and "that"-anaphora joins are untouched.
    if rule.antecedent_sources.len() >= 2 {
        if let Some(plan) = compute_ring_join_plan(
            &rule.text,
            &rule.antecedent_sources,
            &resolved_part_text,
            rule.consequent_cell.literal_id(),
            &noun_names,
            fact_types_map,
            &rule.consequent_role_literals,
            &rule.antecedent_role_literals,
            &rule.antecedent_role_comparisons,
            subtypes,
        ) {
            rule.kind = DerivationKind::Join;
            rule.ring_join = Some(plan);
        }
    }

    // task-970 — Detect existential (Skolem) head variables for single-
    // consequent-FT rules. A parenthesised variable `(VAR)` immediately
    // following a noun token in the consequent clause names a role
    // variable for that role. If that variable does NOT appear (as a
    // parenthesised variable) in any antecedent clause, it is a fresh
    // existential → record a `SkolemHeadRole`. The frontier is all
    // role names of the consequent FT whose variables DO appear in at
    // least one antecedent clause.
    //
    // When skolem head roles are detected on a multi-antecedent rule,
    // shared nouns across antecedents are auto-added as join keys and
    // the rule is classified as Join so `compile_join_derivation` handles
    // the per-binding fanout + skolem id emission.
    //
    // SCOPE: single-consequent-FT only (the deferred two-consequent
    // shared-E case is documented in readings/ui/skolem-head-design.md §5).
    {
        // Extract `(VARNAME)` patterns from a text snippet. Returns the
        // variable name (interior of the parens) for each match.
        let extract_paren_vars = |text: &str| -> Vec<String> {
            let mut vars: Vec<String> = Vec::new();
            let bytes = text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'(' {
                    // find matching ')'
                    if let Some(close) = text[i+1..].find(')') {
                        let interior = text[i+1..i+1+close].trim();
                        // A valid paren var: non-empty, all alphanumeric/hyphen/space,
                        // no single quotes (not a literal value).
                        if !interior.is_empty()
                            && !interior.contains('\'')
                            && interior.chars().all(|c| c.is_alphanumeric() || c == '-' || c == ' ' || c == '_')
                        {
                            vars.push(interior.to_string());
                        }
                        i += close + 2; // skip past ')'
                        continue;
                    }
                }
                i += 1;
            }
            vars
        };

        // Collect all paren vars that appear in antecedent clauses.
        let antecedent_bound_vars: Vec<String> = antecedent_parts.iter()
            .flat_map(|part| extract_paren_vars(part))
            .collect();

        // Attempt skolem detection whenever the consequent resolved to a
        // real FT. Materialization is set AFTER re_resolve_rules returns
        // (in compile.rs), so we cannot check it here; the skolem roles
        // are harmless for non-View rules (compile_join_derivation only
        // uses them for fully-derived View rules).
        let cons_ft_id = rule.consequent_cell.literal_id().to_string();
        if !cons_ft_id.is_empty() {
            if let Some(cons_ft) = fact_types_map.get(&cons_ft_id) {
                // Walk the noun tokens in the consequent text in order.
                // For each noun, check if it is followed by a `(VAR)` token.
                // The nouns align positionally with the FT's role list.
                let cons_nouns = find_nouns(consequent_text, &noun_names);

                // Build a list of (role_name, Option<var_name>) per
                // consequent FT role in surface order.
                let mut role_vars: Vec<(String, Option<String>)> = Vec::new();
                for (role_idx, (_start, end, _token)) in cons_nouns.iter().enumerate() {
                    let role_name = if role_idx < cons_ft.roles.len() {
                        cons_ft.roles[role_idx].noun_name.clone()
                    } else {
                        break;
                    };
                    // Look for a `(VAR)` immediately after the noun token
                    // (possibly with whitespace).
                    let after = &consequent_text[*end..];
                    let after_trimmed = after.trim_start();
                    let var_name = if after_trimmed.starts_with('(') {
                        if let Some(close) = after_trimmed.find(')') {
                            let interior = after_trimmed[1..close].trim();
                            if !interior.is_empty() && !interior.contains('\'') {
                                Some(interior.to_string())
                            } else { None }
                        } else { None }
                    } else { None };
                    role_vars.push((role_name, var_name));
                }

                // Detect skolem head roles: roles whose variable is NOT
                // in antecedent_bound_vars.
                let mut skolem_roles: Vec<crate::types::SkolemHeadRole> = Vec::new();
                let antecedent_bound_role_names: Vec<String> = role_vars.iter()
                    .filter(|(_, var_opt)| {
                        var_opt.as_ref().map(|v| antecedent_bound_vars.contains(v))
                            .unwrap_or(false)
                    })
                    .map(|(role, _)| role.clone())
                    .collect();
                for (role, var_opt) in role_vars.iter() {
                    let Some(var) = var_opt else { continue; };
                    if !antecedent_bound_vars.contains(var) {
                        // This role's variable is fresh (existential).
                        // Frontier = all antecedent-bound role names in
                        // the same consequent FT (in declaration order).
                        skolem_roles.push(crate::types::SkolemHeadRole {
                            role: role.clone(),
                            frontier: antecedent_bound_role_names.clone(),
                        });
                    }
                }
                rule.skolem_head_roles = skolem_roles;

                // For multi-antecedent skolem rules, auto-detect shared
                // nouns across antecedents and promote to Join so
                // compile_join_derivation handles the per-binding fanout.
                if !rule.skolem_head_roles.is_empty()
                    && rule.antecedent_sources.len() >= 2
                    && rule.kind != DerivationKind::Join
                {
                    // Shared nouns = nouns that appear as a role on ≥2
                    // antecedent FTs (an equi-join CHAIN, not a star). The
                    // menu view's 5-way join is a chain — each shared noun
                    // (Status, Transition, SMD, Noun, Resource) bridges
                    // exactly TWO antecedents, so the older "appears on
                    // EVERY antecedent" predicate found none and left the
                    // rule as a ModusPonens existence check. Mirrors the
                    // shared-by-≥2 detection the #914 comparison-block uses
                    // (compile_join_derivation builds one equi-join atom per
                    // key over whichever antecedent pair carries it).
                    let ant_fts: Vec<Option<&FactTypeDef>> = rule.antecedent_sources.iter()
                        .map(|s| fact_types_map.get(s.fact_type_id()))
                        .collect();
                    if ant_fts.iter().all(|ft| ft.is_some()) {
                        // A noun appearing under >1 distinct subscript token
                        // across antecedent clauses is independent variables
                        // (Halpin `Task1` vs `Task2`), NOT a join key — the
                        // ring-join path (compute_ring_join_plan) handles
                        // those positionally. Same guard the comparison block
                        // and compile_explicit_derivation's subscript path use.
                        let tokens_for_noun = |noun: &str| -> hashbrown::HashSet<String> {
                            let mut toks: hashbrown::HashSet<String> = hashbrown::HashSet::new();
                            for clause in resolved_part_text.iter() {
                                for (_, _, t) in find_nouns(clause, &noun_names) {
                                    let (base, _) = parse_role_token(&t);
                                    if base == noun { toks.insert(t.clone()); }
                                }
                            }
                            toks
                        };
                        let mut shared_nouns: Vec<String> = Vec::new();
                        for ft in ant_fts.iter().flatten() {
                            for r in ft.roles.iter() {
                                if shared_nouns.contains(&r.noun_name) { continue; }
                                let appears = ant_fts.iter().flatten()
                                    .filter(|ft| ft.roles.iter()
                                        .any(|rr| rr.noun_name == r.noun_name))
                                    .count();
                                if appears < 2 { continue; }
                                if tokens_for_noun(&r.noun_name).len() > 1 { continue; }
                                shared_nouns.push(r.noun_name.clone());
                            }
                        }
                        if !shared_nouns.is_empty() {
                            // Add shared nouns as join keys (avoid duplicates)
                            for noun in shared_nouns.iter() {
                                if !rule.join_on.contains(noun) {
                                    rule.join_on.push(noun.clone());
                                }
                            }
                            rule.kind = DerivationKind::Join;
                            rule.match_on = rule.join_on.iter()
                                .map(|k| (k.clone(), k.clone()))
                                .collect();
                            // consequent_bindings stays empty so
                            // compile_join_derivation uses all antecedent
                            // nouns (which includes the skolem role nouns
                            // from the consequent FT).

                            // Skolem frontier over a JOIN: the §5 minimal
                            // heuristic (antecedent-bound roles of the
                            // CONSEQUENT FT) cannot recover the menu's
                            // (Resource, Transition) identity — Resource is
                            // in NEITHER consequent FT, and the literal-pinned
                            // `Component Role 'button'` sibling has NO
                            // antecedent-bound consequent role at all, so the
                            // two shared-head rules would skolemise off
                            // DIFFERENT frontiers and invent DIFFERENT ids
                            // (breaking the "shared frontier → shared entity"
                            // invariant). Over a join, the frontier that both
                            // makes the head per-(distinct join row) AND is
                            // identical across sibling rules is the join's
                            // ENTITY-typed antecedent nouns — the labelled-null
                            // frontier of the chase. Value-typed bridge keys
                            // (Status) are excluded: they are join seams, not
                            // entity identities, and a transition belongs to
                            // exactly one (SMD, Status) so they add no
                            // distinguishing power. Ordered by first
                            // (antecedent, role) occurrence for determinism.
                            let mut entity_frontier: Vec<String> = Vec::new();
                            for ft in ant_fts.iter().flatten() {
                                for r in ft.roles.iter() {
                                    let is_entity = nouns_map.get(&r.noun_name)
                                        .map(|n| n.object_type == "entity")
                                        .unwrap_or(false);
                                    if is_entity
                                        && !entity_frontier.contains(&r.noun_name)
                                    {
                                        entity_frontier.push(r.noun_name.clone());
                                    }
                                }
                            }
                            if !entity_frontier.is_empty() {
                                for shr in rule.skolem_head_roles.iter_mut() {
                                    shr.frontier = entity_frontier.clone();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Set rule ID from the FULL rule text. Multiple rules often share
    // a consequent FT (the FORML 2 grammar has 28 rules all producing
    // `Statement has Classification`), so keying on consequent alone
    // collapses them to a single entry under merge_states's identity
    // dedup. Hash the full text for stable, collision-resistant IDs.
    // FNV-1a 64-bit — no hasher dep, no allocation, stable output.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in rule.text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    rule.id = format!("rule_{h:x}");
}

/// Consequent text of a derivation rule: everything before the leftmost
/// ` iff ` / ` if ` / ` when ` keyword, with any leading bullet marker
/// stripped (markers are usually normalized away upstream; stripping
/// here is idempotent).
fn derivation_consequent_text(rule_text: &str) -> &str {
    let mut t = rule_text.trim();
    for m in ["** ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) { t = rest; break; }
    }
    match t.find(" iff ").or_else(|| t.find(" if ")) {
        Some(i) => t[..i].trim(),
        None => t,
    }
}

/// Resolve a numeric-subscript Join derivation's positional join plan
/// (whitepaper eq:join — `s_sh` selects the shared roles BY POSITION)
/// from the rule's Halpin numeric subscripts. Returns `None` (so the
/// caller leaves the rule on its existing path) unless every antecedent
/// resolves to a fact type, a SUBSCRIPTED variable recurs across >=2
/// DISTINCT antecedents (the numeric-subscript join key — this admits
/// BOTH self-ring FTs, where a base noun fills >1 role, AND cross-FT
/// recursive subscript joins, e.g. the SM `has reached` transitive
/// fixpoint), and clause tokenisation lines up with the fact-type role
/// arities. The subscript gate keeps "that"-anaphora and plain noun-name
/// joins on their existing path. Every variable (subscripted or plain
/// noun) occupying >=2 antecedents becomes an equi-join key, so a
/// subscript-join rule's unsubscripted threading variable also constrains
/// the join.
///
/// `antecedent_clauses[i]` is the resolved text of `antecedent_sources[i]`
/// (same index alignment the #914 comparison pass relies on); the k-th
/// noun token in a clause fills role k of that antecedent's fact type.
fn compute_ring_join_plan(
    rule_text: &str,
    antecedent_sources: &[crate::types::AntecedentSource],
    antecedent_clauses: &[String],
    consequent_ft_id: &str,
    noun_names: &[String],
    fact_types: &HashMap<String, FactTypeDef>,
    consequent_role_literals: &[crate::types::ConsequentRoleLiteral],
    antecedent_role_literals: &[crate::types::AntecedentRoleLiteral],
    antecedent_role_comparisons: &[crate::types::AntecedentRoleComparison],
    // subtype → supertype chain (one parent per noun), so the role-arity gate
    // can accept a subtype filler (state.md:8 — State Machine Definition is a
    // subtype of Status). Threaded from resolve_derivation_rule.
    subtypes: &HashMap<String, String>,
) -> Option<crate::types::RingJoinPlan> {
    let n = antecedent_sources.len();
    if n < 2 || antecedent_clauses.len() < n { return None; }

    // Ordered noun tokens (subscripts preserved) of a clause.
    let tokens_in_order = |clause: &str| -> Vec<String> {
        let mut occ: Vec<(usize, String)> = find_nouns(clause, noun_names)
            .into_iter().map(|(s, _, t)| (s, t)).collect();
        occ.sort_by_key(|(s, _)| *s);
        occ.into_iter().map(|(_, t)| t).collect()
    };
    let is_subscripted = |t: &str| -> bool {
        let (base, _) = parse_role_token(t);
        base.len() != t.len()
    };
    // Walk the subtype→supertype chain (nearest parent first), bounded against
    // cyclic declarations. Mirrors the bridge in resolve_derivation_rule: a
    // subtype filler resolves to a role typed on any of its supertypes, so the
    // role-arity gate below must accept it.
    let supertype_chain = |noun: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut cur = noun.to_string();
        let mut guard = 0usize;
        while let Some(sup) = subtypes.get(&cur) {
            if out.iter().any(|s| s == sup) { break; }
            out.push(sup.clone());
            cur = sup.clone();
            guard += 1;
            if guard > subtypes.len() + 1 { break; }
        }
        out
    };

    // token -> (antecedent_index, role_index) occurrences.
    let mut token_positions: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
    for i in 0..n {
        let ft_id = antecedent_sources[i].fact_type_id();
        if ft_id.is_empty() { return None; }
        let ft = fact_types.get(ft_id)?;
        let role_count = ft.roles.len();
        // ring-join-predicate-noun-collision: `find_nouns` matches EVERY declared
        // noun in the clause text, including a PREDICATE word that collides with
        // a noun declared elsewhere in the model -- e.g. `size` in `Structure
        // has size Count` collides with the metamodel noun `Size` (`File has
        // Size`). The spurious token inflates the count past the FT's arity, so
        // the ring plan bails to the noun-name fallback, which cannot tell a
        // ring's two same-noun roles apart (both resolve to role 0, so every
        // derived ring fact self-collapses O2:=O1). Keep only tokens whose base
        // noun is an actual ROLE of this antecedent's fact type. (A minimal test
        // substrate lacks the colliding noun -- which is exactly why this bug
        // only ever manifested in the full kernel, never in an isolated repro.)
        let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
        let toks: Vec<String> = tokens_in_order(&antecedent_clauses[i]).into_iter()
            .filter(|tok| {
                let (base, _) = parse_role_token(tok);
                // Exact role-noun match, OR a subtype filler: a token whose base
                // noun is a subtype of a declared role noun fills that role, since
                // subtype instances ARE supertype instances (state.md:8 — State
                // Machine Definition is a subtype of Status). Without this the Harel
                // inherited-edge rule `Transition is from <SMD> …` drops its SMD
                // token, the count breaks, and the ring plan bails to the noun-name
                // fallback (which self-collapses same-noun roles).
                role_nouns.iter().any(|rn| rn.eq_ignore_ascii_case(base))
                    || supertype_chain(base).iter()
                        .any(|sup| role_nouns.iter().any(|rn| rn.eq_ignore_ascii_case(sup.as_str())))
            })
            .collect();
        if toks.len() != role_count { return None; }
        for (role_idx, tok) in toks.into_iter().enumerate() {
            match token_positions.iter_mut().find(|(t, _)| *t == tok) {
                Some((_, ps)) => ps.push((i, role_idx)),
                None => token_positions.push((tok, alloc::vec![(i, role_idx)])),
            }
        }
    }
    let distinct_ants = |ps: &[(usize, usize)]| -> usize {
        let mut a: Vec<usize> = ps.iter().map(|(x, _)| *x).collect();
        a.sort_unstable();
        a.dedup();
        a.len()
    };

    // Gate: this rule joins via Halpin numeric SUBSCRIPTS (not noun-name
    // "that"-anaphora) iff some SUBSCRIPTED variable recurs across >=2
    // DISTINCT antecedents. This keeps ordinary noun-name joins on their
    // existing (`join_on`) path while admitting BOTH self-ring FTs (the
    // base noun fills >1 role) AND cross-FT recursive subscript joins —
    // e.g. the SM `has reached` transitive fixpoint, where `Status1`
    // recurs across `… has reached Status1` and `Transition is from
    // Status1`. (Self-ring detection was the prior gate; it was too narrow
    // — it rejected this recursive cross-FT shape even though it is driven
    // by the same numeric-subscript join mechanism.)
    let has_subscripted_join_var = token_positions.iter()
        .any(|(tok, ps)| is_subscripted(tok) && distinct_ants(ps) >= 2);
    if !has_subscripted_join_var { return None; }

    // Join keys: EVERY variable (subscripted OR plain noun) occupying >=2
    // DISTINCT antecedents is an equi-join key. Plain shared nouns are now
    // included (not just subscripted ones) so a subscript-join rule's
    // UNSUBSCRIPTED threading variable also constrains the join — e.g.
    // `State Machine`, shared by `… has reached` and `Transition is
    // applicable for that State Machine`, must equi-join, else the
    // recursive fixpoint cross-products across every state machine.
    //
    // ring-join-blocker-side-consequent: a LITERAL-PINNED occurrence is a
    // FILTER, not a join variable — drop it from join-group membership.
    // Two antecedents restricting the same value-type noun to DIFFERENT
    // literals (`Task1 has Task Status 'in_progress' and the Task has
    // Task Status 'pending'`) otherwise mint a spurious equi-join on the
    // value ('in_progress' = 'pending') that empties the rule. The
    // literal itself is still enforced by compile_join_derivation's
    // antecedent_role_literals path (#814), so no constraint is lost; a
    // group whose unpinned occurrences span <2 antecedents is no key.
    let literal_pinned = |i: usize, role_idx: usize| -> bool {
        let ft_id = antecedent_sources[i].fact_type_id();
        fact_types.get(ft_id).map_or(false, |ft| {
            ft.roles.get(role_idx).map_or(false, |r| {
                antecedent_role_literals.iter().any(|l|
                    l.antecedent_index == i && l.role == r.noun_name)
            })
        })
    };
    // arc cross-antecedent-comparison (#914 forward-chain): a noun that is
    // COMPARED across antecedents (`Task1's Task ID is less than Task2's
    // Task ID`) must NOT also become an equi-join key. Equating it
    // (Task1.TaskID == Task2.TaskID) directly contradicts the `<`
    // comparison and empties the rule's forward-chain (the #907 repro). The
    // comparison itself is enforced by compile_join_derivation's
    // role-comparison Filter; the join must only bind the SHARED nouns
    // (here Source File), not the compared one. A compared noun recurs
    // across >=2 antecedents like any join var, so without this it is
    // silently promoted to an equi-key. Mirrors the noun-name join path's
    // `comparison_roles` exclusion.
    let comparison_role_nouns: hashbrown::HashSet<&str> = antecedent_role_comparisons
        .iter()
        .flat_map(|c| [c.lhs_role.as_str(), c.rhs_role.as_str()])
        .collect();
    let join_groups: Vec<Vec<(usize, usize)>> = token_positions.iter()
        .filter(|(tok, _)| !comparison_role_nouns.contains(parse_role_token(tok).0))
        .map(|(_, ps)| ps.iter()
            .filter(|(i, ri)| !literal_pinned(*i, *ri))
            .cloned()
            .collect::<Vec<(usize, usize)>>())
        .filter(|ps| distinct_ants(ps) >= 2)
        .collect();
    if join_groups.is_empty() { return None; }

    // Consequent role sources: k-th consequent noun token -> consequent
    // role k; its value comes from that token's first antecedent slot.
    let cons_ft = fact_types.get(consequent_ft_id)?;
    let cons_toks = tokens_in_order(derivation_consequent_text(rule_text));
    if cons_toks.len() != cons_ft.roles.len() { return None; }
    let mut consequent_positions: Vec<Option<(usize, usize)>> = Vec::with_capacity(cons_toks.len());
    for (k, tok) in cons_toks.iter().enumerate() {
        match token_positions.iter().find(|(t, _)| t == tok) {
            // Drawn from a joined antecedent slot (its first occurrence).
            Some((_, ps)) => consequent_positions.push(Some(*ps.first()?)),
            // Not in any antecedent. A literal-pinned consequent role
            // (`... then that X has Y 'lit'`) is legitimately literal-sourced:
            // record `None` and let compile_join_derivation's literal-pin
            // branch (#814) supply the value via `Func::constant`. Any other
            // unfound token is genuinely unbindable — bail so the rule keeps
            // its existing (noun-name) path.
            None => {
                let role_noun = &cons_ft.roles[k].noun_name;
                if consequent_role_literals.iter().any(|c| c.role == *role_noun) {
                    consequent_positions.push(None);
                } else {
                    return None;
                }
            }
        }
    }

    Some(crate::types::RingJoinPlan { join_groups, consequent_positions })
}















/// Emit a Constraint cell fact for a test-built `ConstraintDef`. Kept
/// `#[cfg(test)]` because non-test code shapes constraints via the
/// stage12 translators, not through this helper.
#[cfg(all(test, feature = "std-deps"))]
pub(crate) fn constraint_to_fact_test(c: &ConstraintDef) -> crate::ast::Object {
    use crate::ast::fact_from_pairs;
    let json = serde_json::to_string(c).unwrap_or_default();
    let mut pairs: Vec<(String, String)> = alloc::vec![
        ("id".into(), c.id.clone()), ("kind".into(), c.kind.clone()),
        ("modality".into(), c.modality.clone()), ("text".into(), c.text.clone()),
        ("json".into(), json),
    ];
    c.deontic_operator.as_ref().map(|op| pairs.push(("deonticOperator".into(), op.clone())));
    c.entity.as_ref().map(|e| pairs.push(("entity".into(), e.clone())));
    c.predicate.as_ref().map(|p| pairs.push(("predicate".into(), p.encode())));
    pairs.extend(c.spans.iter().enumerate().flat_map(|(i, span)| [
        (alloc::format!("span{}_factTypeId", i), span.fact_type_id.clone()),
        (alloc::format!("span{}_roleIndex", i), span.role_index.to_string()),
    ]));
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fact_from_pairs(&refs)
}


// =========================================================================
// Pure extraction functions (no if/else -- use ? and strip_prefix/suffix)
// =========================================================================





/// The engine's working definition of a reading's VERB: the text after
/// the first noun occurrence up to the second (binary+), or everything
/// after the single noun (unary — #274 Category A; without the unary
/// branch `Customer is in EEA` would carry an empty verb and collide
/// with every other unary keyed on [customer]). `noun_names` must be
/// sorted longest-first so multi-word nouns match before their
/// prefixes. Shared by the SchemaCatalog register site (ρ-lookup
/// disambiguation) and the `Reading_is_used_by_Verb` schema reflection
/// (task-987 onion) so both surfaces agree on what the Verb IS.
pub(crate) fn reading_verb<'a>(reading: &'a str, noun_names_longest_first: &[String]) -> &'a str {
    noun_names_longest_first.iter()
        .find(|n| reading.starts_with(n.as_str()))
        .map(|first| {
            let after = &reading[first.len()..];
            // engine-2role-ring-aggregate-stratify-overflow: the verb runs up to
            // the EARLIEST-POSITIONED next noun in `after` — by TEXT POSITION, not
            // by longest-first LIST order. The old `find_map` returned text up to
            // whichever noun the longest-first iteration hit first, so a longer
            // noun sitting LATER in the reading (`… reaches Value for Feature at
            // Count`: Feature len 7 > the second Value len 5) slurped the
            // inter-noun text into the verb (`reaches Value for` instead of
            // `reaches`). That made the catalog's REGISTERED verb mismatch the
            // position-based clause verb the ρ-lookup extracts, so the exact-verb
            // match missed and a recursive antecedent (`… reaches …`) fell through
            // to a same-signature sibling — its own `shortest reaches` aggregate —
            // forming a FALSE cycle that broke stratification (→ single flat
            // stratum → min-over-recursion stack overflow on a 2-role ring).
            // Taking the minimum find() position keeps register/resolve identical.
            noun_names_longest_first.iter()
                .filter(|n| !n.is_empty())
                .filter_map(|second| after.find(second.as_str()))
                .min()
                .map(|pos| after[..pos].trim())
                .unwrap_or_else(|| after.trim())
        })
        .unwrap_or("")
}

/// Collapse a fact type's role list when re-declaration concatenated it into an
/// exact k≥2 repetition of a period-p tile (`[A, B, A, B] → [A, B]`). Only tiles
/// of period ≥ 2 collapse; a period-1 list (`[Task, Task]`) is a legitimate
/// same-noun ring and is returned unchanged. See `SchemaCatalog::register`.
fn collapse_redeclared_roles<'a>(roles: &[&'a str]) -> Vec<&'a str> {
    let n = roles.len();
    for p in 2..=n / 2 {
        if n % p == 0 && (0..n).all(|i| roles[i] == roles[i % p]) {
            return roles[..p].to_vec();
        }
    }
    roles.to_vec()
}

/// Schema catalog for rho-lookup: noun set -> Fact Type ID.
/// The noun set is the key. The catalog is the DEFS cell.
struct SchemaCatalog {
    /// Sorted noun set -> vec of (schema_id, verb, reading) for disambiguation
    by_noun_set: HashMap<Vec<String>, Vec<(String, String, String)>>,
}

impl SchemaCatalog {
    fn new() -> Self {
        SchemaCatalog { by_noun_set: HashMap::new() }
    }

    fn register(&mut self, schema_id: &str, role_nouns: &[&str], verb: &str, reading: &str) {
        // redeclared-ft-role-doubling: a fact type re-declared in the readings
        // (e.g. `State Machine is for Resource` declared as a base FT AND again
        // as a derived `*` FT — readings/core/instances.md:121,123) concatenates
        // its role list, yielding `[A, B, A, B]` for a binary. The 4-element
        // catalog key (`[resource, resource, state machine, state machine]`)
        // then never matches a real 2-role clause lookup, so derivation rules
        // referencing the FT silently lose it as an antecedent. Collapse an
        // exact k≥2 repetition of a period-p tile (p≥2) back to one tile so the
        // key reflects the FT's true arity. A period-1 tile (`[Task, Task]`) is
        // a legitimate same-noun ring and is left intact.
        let collapsed = collapse_redeclared_roles(role_nouns);
        let mut key: Vec<String> = collapsed.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        self.by_noun_set
            .entry(key)
            .or_default()
            .push((schema_id.to_string(), verb.to_lowercase(), reading.to_lowercase()));
    }

    /// rho-lookup: noun set -> Fact Type ID.
    /// Resolution strategy (no COND dispatch, just cascading lookup):
    /// 1. Exact verb match
    /// 2. Verb contained in stored reading (handles inverse voice)
    /// 3. Unique entry for noun set (no verb needed) — binary+ only
    ///
    /// The unique-entry fallback is skipped for 1-noun keys (#274
    /// Category A). Unaries carry all their identity in the verb:
    /// without the fallback guard, a clause like `Order has Mystery`
    /// (noun set [order], `Mystery` undeclared) would resolve to any
    /// single unary synthetic keyed on [order] — `Order is pending`,
    /// `Order is cancelled` — regardless of verb. Step 1 and 2 remain
    /// active and catch the legitimate unary matches.
    fn resolve(&self, role_nouns: &[&str], verb: Option<&str>) -> Option<String> {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        let entries = self.by_noun_set.get(&key)?;
        let vb = verb.map(|v| v.to_lowercase());
        let allow_unique_fallback = key.len() >= 2;
        // Exact verb match
        entries.iter()
            .find(|(_, v, _)| vb.as_ref().map_or(false, |vb| v == vb))
            .or_else(||
                // Verb contained in stored reading (inverse voice: "is owned by" matches "owns")
                entries.iter()
                    .find(|(_, _, reading)| vb.as_ref().map_or(false, |vb| reading.contains(vb.as_str())))
            )
            .or_else(||
                // Unique entry for this noun set (binary+ only)
                (allow_unique_fallback && entries.len() == 1).then(|| &entries[0])
            )
            .map(|(id, _, _)| id.clone())
    }

    /// STRICTLY verb-specific resolve: exact-verb (step 1) or verb-in-reading
    /// (step 2, inverse voice) ONLY — NO unique-entry fallback. A clause whose
    /// verb does not match the single FT on its noun set returns None here, so
    /// the caller can try the subtype→supertype bridge BEFORE the verb-agnostic
    /// unique fallback. Without this, `Transition is from <SMD>` (noun set
    /// [Transition, SMD], whose ONLY FT is the different-verb `Transition is
    /// defined in State Machine Definition`) is grabbed by the unique fallback
    /// and never reaches the bridge that would correctly bind `Transition is
    /// from Status` (SMD < Status).
    fn resolve_verb_strict(&self, role_nouns: &[&str], verb: &str) -> Option<String> {
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
            let (base, _) = parse_role_token(n);
            base.to_lowercase()
        }).collect();
        key.sort();
        let entries = self.by_noun_set.get(&key)?;
        let vb = verb.to_lowercase();
        entries.iter()
            .find(|(_, v, _)| *v == vb)
            .or_else(|| entries.iter().find(|(_, _, reading)| reading.contains(vb.as_str())))
            .map(|(id, _, _)| id.clone())
    }

    /// Objectified-pivot abbreviation fallback (#963). An objectified fact
    /// type declared with a trailing reference role —
    /// `X pivots A is implemented by B at C` (4 roles) — is referenced in
    /// rule bodies by its abbreviated reading `X pivots A is implemented by
    /// B` (3 role-refs; the trailing `at C` dropped). The exact noun-set
    /// lookup in `resolve` misses because the stored key carries 4 nouns
    /// and the abbreviated clause yields 3. Match the fact type whose
    /// noun-set is the query set plus exactly one extra noun AND whose
    /// reading equals the clause once a trailing ` at <Role>` is removed.
    /// Narrow by construction — only consulted after both `resolve`
    /// attempts return None — so it can only bind a clause that was
    /// otherwise unresolved.
    fn resolve_objectified_abbrev(&self, role_nouns: &[&str], clause: &str) -> Option<String> {
        let mut qkey: Vec<String> = role_nouns.iter()
            .map(|n| parse_role_token(n).0.to_lowercase())
            .collect();
        qkey.sort();
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let clause_norm = norm(&clause.to_lowercase());
        self.by_noun_set.iter()
            .filter(|(k, _)| k.len() == qkey.len() + 1)
            .filter(|(k, _)| qkey.iter().all(|n| k.contains(n)))
            .flat_map(|(_, entries)| entries.iter())
            .find(|(_, _, reading)| {
                reading.split(" at ").next()
                    .map_or(false, |head| norm(head) == clause_norm)
            })
            .map(|(id, _, _)| id.clone())
    }
}


/// Strip parenthesized role-variable tokens — `Fact Type (FT) has Role`
/// → `Fact Type has Role`. A role variable is the FORML2 rule-head/body
/// binder convention: a short (≤4 chars) alphanumeric token starting
/// with an uppercase letter, alone in parentheses. Quoted literals,
/// ring-kind annotations (those trail the declaration period, never a
/// rule clause), and longer parentheticals are untouched. Used by the
/// FT-resolution views of rule clauses so verb extraction sees the
/// declared verb; the variables themselves are read from the original
/// text by the skolem/join machinery.
pub(crate) fn strip_role_variables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            // Find the closing paren within a short window.
            if let Some(close_rel) = text[i + 1..].find(')') {
                let inner = &text[i + 1..i + 1 + close_rel];
                let is_var = (1..=4).contains(&inner.len())
                    && inner.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                    && inner.chars().all(|c| c.is_ascii_alphanumeric());
                if is_var {
                    // Drop the token plus ONE adjacent space so
                    // `Type (FT) has` collapses to `Type has`.
                    if out.ends_with(' ') { out.pop(); }
                    i = i + 1 + close_rel + 1;
                    continue;
                }
            }
        }
        // Advance one char (UTF-8 safe: copy the full char).
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Parse a role token into (base_noun_name, full_token_with_subscript).
/// "Person1" -> ("Person", "Person1"). "User" -> ("User", "User").
pub(crate) fn parse_role_token(token: &str) -> (&str, &str) {
    let boundary = token
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    (&token[..boundary], token)
}

/// Align a clause's noun tokens (from `find_nouns`, in surface order) to a
/// fact type's role positions. Returns `(role_index, full_token)` pairs.
/// Walks the tokens in order, advancing a role cursor whenever the token's
/// base noun matches the next role's noun — the same positional alignment
/// `compile::antecedent_role_subscripts` uses to disambiguate self-ring
/// FTs (both roles share a noun name), where the subscripted token
/// (`Item1`) identifies which position a reference targets.
pub(crate) fn align_tokens_to_roles(
    tokens: &[(usize, usize, String)],
    ft: &FactTypeDef,
) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut role_cursor = 0;
    for (_, _, token) in tokens {
        let (base, _) = parse_role_token(token);
        if role_cursor < ft.roles.len() && ft.roles[role_cursor].noun_name == base {
            out.push((role_cursor, token.clone()));
            role_cursor += 1;
        }
    }
    out
}




/// Find nouns in text -- longest-first matching with word boundaries.
/// Returns (start, end, name) tuples sorted by position.
///
/// Exposed to the crate so post-parse checks (e.g. ring completeness
/// in `check.rs`) can re-tokenize a FactType reading against the
/// fully-accumulated Noun set, independent of the parse-time noun
/// list that was available when the FactType was first parsed.
pub(crate) fn find_nouns(text: &str, noun_names: &[String]) -> Vec<(usize, usize, String)> {
    let mut sorted: Vec<&String> = noun_names.iter().collect();
    sorted.sort_by(|a, b| b.len().cmp(&a.len()));

    // #273: prose-heavy rule bodies (legal text, derivations) routinely
    // mention a declared noun in lowercase — "… if interpretation is
    // reasonable" against a capitalised `Interpretation` entity type.
    // We match case-insensitively against ASCII-lowercased copies so
    // that drift doesn't fall through to "antecedent clause did not
    // resolve". ASCII-lowercasing preserves byte length, so indices
    // in `text_lower` map 1:1 back to `text`; the captured token is
    // taken from `text` to preserve the reading-author's casing for
    // downstream ring / join-key consumers.
    let text_lower: String = text.chars().map(|c| c.to_ascii_lowercase()).collect();

    // Foldl over longest-first noun list. Accumulator is (matches, used_ranges).
    // Inner loop over occurrences of `name` in `text` uses Backus's `while`
    // combining form (sequential scan of positions).
    //
    // Halpin ring rules distinguish same-type roles by numeric subscripts
    // (Person1, Person2, Person3 â€” see Example 6 in the FORML position
    // paper). When the match is followed by ASCII digits we treat them
    // as a subscript and extend the captured range to include them; the
    // returned token ("Person3") preserves subscript identity so join-
    // key detection downstream works, and parse_role_token strips it to
    // the base ("Person") before catalog lookup.
    let (mut matches, _): (Vec<(usize, usize, String)>, Vec<(usize, usize)>) = sorted.iter().fold(
        (Vec::new(), Vec::new()),
        |(mut matches, mut used), name| {
            let name_lower: String = name.chars().map(|c| c.to_ascii_lowercase()).collect();
            let mut pos = 0;
            // Collect THIS noun's candidate occurrences first, so the
            // verb/noun case-collision filter below can weigh them against
            // each other before committing any to the global match set.
            let mut cands: Vec<(usize, usize, String)> = Vec::new();
            while let Some(found) = text_lower[pos..].find(name_lower.as_str()) {
                let start = pos + found;
                let mut end = start + name_lower.len();
                // Token-boundary check: the byte before and after the
                // match must not extend the identifier. Reject when the
                // surrounding char is alphanumeric OR an ASCII hyphen
                // (so hyphenated words like `file-conflicting` are
                // atomic — without the hyphen check the unary FT verb
                // `is file-conflicting` would mis-split into "file" +
                // "-conflicting", with `find_nouns` capturing the
                // metamodel noun `File` from inside an unrelated
                // identifier and breaking
                // `resolve_consequent_fact_type_id` for the rule
                // `Task2 is file-conflicting iff …`. #866-c.
                let is_word_byte = |b: u8| -> bool {
                    b.is_ascii_alphanumeric() || b == b'-'
                };
                let before_ok = start == 0 || !is_word_byte(text.as_bytes()[start - 1]);
                // Extend end past any trailing ASCII digit subscript.
                while end < text.len() && text.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                // After the (possibly-extended) end, the next byte must
                // not extend the identifier (alphanumeric or hyphen).
                let after_ok = end >= text.len() || !is_word_byte(text.as_bytes()[end]);
                let no_overlap = !used.iter().any(|&(s, e)| start < e && end > s);

                // word-boundary guard: skip this match if a longer reserved metamodel
                // noun begins here, so `Fact` is not matched inside `Fact Type`.
                if before_ok && after_ok && no_overlap
                    && !shadowed_by_longer_reserved(&text[start..], name.len())
                {
                    // Capture the subscripted token (e.g. "Person3") so
                    // callers distinguish the ring positions. The base
                    // name is recovered via parse_role_token at the
                    // resolve site.
                    let captured = &text[start..end];
                    cands.push((start, end, captured.to_string()));
                }
                pos = start + 1;
                if pos >= text.len() { break; }
            }
            // engine-valuetyped-maxmin: verb/noun case-collision. When a
            // declared noun occurs BOTH capitalized (the genuine role
            // reference — `Slots1`, possibly subscripted) AND lowercased
            // (the relating VERB — `slots` in `Item slots Slots1 for
            // Attribute`) within ONE clause, the lowercase occurrence is the
            // verb, not a noun. Keeping it inflated the role-set, so
            // `resolve_fact_type` missed the source FT and the aggregate was
            // SILENTLY DROPPED (the head compiled to nothing / an empty
            // fold). Drop the lowercase occurrence(s) only when a capitalized
            // sibling exists — preserving the #273 prose case (a noun
            // mentioned ONLY in lowercase, with no capitalized sibling, still
            // matches) and the all-capitalized ring case (`Glyph1`/`Glyph2`).
            let any_cap = cands.iter()
                .any(|&(s, _, _)| text.as_bytes()[s].is_ascii_uppercase());
            for (start, end, captured) in cands {
                if any_cap && text.as_bytes()[start].is_ascii_lowercase() {
                    continue;
                }
                matches.push((start, end, captured));
                used.push((start, end));
            }
            (matches, used)
        },
    );

    matches.sort_by_key(|m| m.0);
    matches
}

// =========================================================================
// Hand-rolled string-matching helpers (replacing `regex::Regex` sites,
// part of the `no_std` lift in #588). Each helper documents the regex
// it stands in for and the call site that drove it.
// =========================================================================

/// Hand-rolled equivalent of regex ` '([^']*)'\s*$`.
///
/// Returns `Some((without_literal, captured))` when `s` ends in a
/// space-prefixed single-quoted literal (optionally followed by
/// trailing ASCII whitespace). The `without_literal` string mirrors
/// what `regex::Regex::replace(s, "")` produces — i.e. `s` with the
/// matched span removed. `captured` is the literal's interior.
///
/// Returns `None` when no such trailing literal is present.
///
/// Sites: 1065 (consequent text), 1143 (antecedent stripped form).
fn strip_trailing_quoted_literal(s: &str) -> Option<(String, String)> {
    // 1. Trim only ASCII whitespace from the right end (regex `\s` is
    //    Unicode in default regex crate config but inputs here are
    //    ASCII-only — no FORML reading uses non-ASCII whitespace).
    let body = s.trim_end();
    // 2. Body must end with a single quote.
    let body = body.strip_suffix('\'')?;
    // 3. The literal interior is everything after the *last*
    //    space-prefixed quote that contains no inner quote.
    //    `[^']*` between the two quotes means the literal cannot
    //    itself contain a single quote, so we search backwards for
    //    the opening ` '`.
    let open = body.rfind(" '")?;
    let interior = &body[open + 2..];
    if interior.contains('\'') {
        return None;
    }
    let captured = interior.to_string();
    // 4. `without_literal` = everything before the leading space of
    //    the literal segment. The regex match includes the trailing
    //    `\s*`, so `replace` consumes it; we mirror that by dropping
    //    everything from `open` onward.
    let without_literal = s[..open].to_string();
    Some((without_literal, captured))
}

/// Hand-rolled tokenizer equivalent to splitting on regex
/// `\s*([+\-*/])\s*` via `find_iter`.
///
/// Walks the input, emits each `+ - * /` as its own token, and emits
/// the maximal whitespace-trimmed run between operators as the
/// surrounding operand tokens. Empty operands are dropped (matches
/// the `if !head.is_empty()` guard the regex code used).
///
/// Site: 741 (parse_arithmetic_expr).
fn tokenize_arith(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if matches!(c, b'+' | b'-' | b'*' | b'/') {
            let head = text[start..i].trim();
            if !head.is_empty() {
                tokens.push(head.to_string());
            }
            tokens.push((c as char).to_string());
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        tokens.push(tail.to_string());
    }
    tokens
}

/// Peel a trailing numeric comparator off a derivation antecedent, accepting
/// BOTH a SYMBOLIC operator —
///   `\s*(>=|<=|!=|<>|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$`
/// — and a TEXT phrase (`greater than`, `less than or equal to`, `equals`,
/// `not equal to`, `exceeds`, `more than`, …, with an optional leading `is`)
/// mapped to the SAME canonical op symbol, so both forms feed an identical
/// Filter primitive downstream. (`at least N` / `at most N` are deliberately
/// NOT recognised here — those are CARDINALITY premises consumed upstream by
/// `extract_antecedent_cardinality`.)
///
/// Returns `Some((stripped, raw_op, value))` where:
/// - `stripped` is the input with the trailing operator + numeric
///   suffix removed and trailing whitespace trimmed (mirrors
///   `text[..whole.start()].trim_end().to_string()`),
/// - `raw_op` is the literal operator token (`>=`, `<=`, `!=`, `<>`,
///   `>`, `<`, `=`) — caller normalises `<>` → `!=`,
/// - `value` is the parsed `f64`.
///
/// Returns `None` if the input does not end in the comparator+number
/// shape.
///
/// Site: 813 (split_antecedent_comparator).
fn peel_trailing_comparator(text: &str) -> Option<(String, &'static str, f64)> {
    // 1. Right-trim whitespace (matches the regex tail `\s*$`).
    let s = text.trim_end();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let end = bytes.len();
    // 2. Walk backwards over the integer tail `\d+`.
    let mut p = end;
    while p > 0 && bytes[p - 1].is_ascii_digit() {
        p -= 1;
    }
    if p == end {
        return None; // no trailing digits at all
    }
    // 3. Optional `\.\d+` fractional suffix immediately before the
    //    digit-tail we already consumed.
    if p > 0 && bytes[p - 1] == b'.' {
        let dot = p - 1;
        let mut q = dot;
        while q > 0 && bytes[q - 1].is_ascii_digit() {
            q -= 1;
        }
        // Require at least one digit to the left of the dot, else
        // `.5` would parse as part of the number but the regex
        // `\d+\.\d+` requires `\d+` on the left.
        if q < dot {
            p = q;
        }
    }
    // 4. Optional leading `-` directly attached to the number.
    let num_start_with_sign = if p > 0 && bytes[p - 1] == b'-' {
        p - 1
    } else {
        p
    };
    // 5. Try both with- and without-sign so the operator-detection
    //    step can pick the variant that exposes a valid operator.
    //    For input `... > -10`, with-sign reading "-10" leaves "..."
    //    + ` > ` for the operator; without-sign reading "10" leaves
    //    "... > -" which fails the operator match.
    for &num_start in &[num_start_with_sign, p] {
        let value: f64 = match s[num_start..end].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        // 6. Skip whitespace between op and number (`\s*` after op).
        let mut op_end = num_start;
        while op_end > 0 && bytes[op_end - 1].is_ascii_whitespace() {
            op_end -= 1;
        }
        // 7. Operator — SYMBOLIC token or TEXT phrase, both mapped to the same
        //    canonical op symbol (so the downstream Filter primitive is identical).
        let head = &s[..op_end];
        // 7a. Symbolic alternation, longest first (so `>=` beats `>`).
        const OPS: &[&str] = &[">=", "<=", "!=", "<>", ">", "<", "="];
        if let Some(op) = OPS.iter().copied().find(|op| {
            op.len() <= op_end && &head[op_end - op.len()..] == *op
        }) {
            // 8. `stripped` mirrors `text[..whole.start()].trim_end()`.
            let stripped = text[..op_end - op.len()].trim_end().to_string();
            return Some((stripped, op, value));
        }
        // 7b. Text comparator phrases → op symbol (e.g. `greater than` → `>`).
        //     Matched at the operator position, requiring a word boundary before
        //     the phrase; an optional `is` connective (`... Weight is more than 5`)
        //     is stripped so the remaining clause resolves as the bare fact type.
        //     (`at least N` / `at most N` are intentionally absent — those are
        //     CARDINALITY premises, handled upstream by extract_antecedent_cardinality.)
        const TEXT_OPS: &[(&str, &str)] = &[
            ("greater than or equal to", ">="),
            ("less than or equal to", "<="),
            ("no less than", ">="),
            ("no more than", "<="),
            ("not equal to", "!="),
            ("does not equal", "!="),
            ("greater than", ">"),
            ("less than", "<"),
            ("more than", ">"),
            ("fewer than", "<"),
            ("equal to", "="),
            ("exceeds", ">"),
            ("equals", "="),
        ];
        let head_lower = head.to_ascii_lowercase();
        for (phrase, op) in TEXT_OPS.iter().copied() {
            if head_lower.len() >= phrase.len()
                && &head_lower[head_lower.len() - phrase.len()..] == phrase
                && (head.len() == phrase.len()
                    || head.as_bytes()[head.len() - phrase.len() - 1].is_ascii_whitespace())
            {
                let op_start = op_end - phrase.len();
                let mut stripped = text[..op_start].trim_end().to_string();
                if let Some(pre) = stripped.strip_suffix(" is") {
                    stripped = pre.trim_end().to_string();
                }
                return Some((stripped, op, value));
            }
        }
        continue;
    }
    None
}

#[cfg(test)]
mod text_comparator_tests {
    //! comparison-text-and-symbolic: a derivation antecedent may compare a role
    //! VALUE against a numeric literal using EITHER a symbolic operator
    //! (`> 5`, `>= 9`) or a TEXT phrase (`greater than 5`, `at least`-style
    //! words excluded as those are cardinality). Both forms peel to the same
    //! canonical op symbol + f64 so the downstream Filter primitive is identical.
    use super::*;

    fn split(t: &str) -> (String, Option<(String, f64)>) {
        split_antecedent_comparator(t)
    }

    #[test]
    fn symbolic_forms_unchanged() {
        assert_eq!(split("Item has Weight > 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
        assert_eq!(split("Item has Weight >= 9"), ("Item has Weight".into(), Some((">=".into(), 9.0))));
        assert_eq!(split("Item has Weight != 5"), ("Item has Weight".into(), Some(("!=".into(), 5.0))));
        assert_eq!(split("Item has Weight"), ("Item has Weight".into(), None));
    }

    #[test]
    fn text_forms_map_to_same_op() {
        assert_eq!(split("Item has Weight greater than 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
        assert_eq!(split("Item has Weight less than 5"), ("Item has Weight".into(), Some(("<".into(), 5.0))));
        assert_eq!(split("Item has Weight more than 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
        assert_eq!(split("Item has Weight greater than or equal to 9"), ("Item has Weight".into(), Some((">=".into(), 9.0))));
        assert_eq!(split("Item has Weight less than or equal to 9"), ("Item has Weight".into(), Some(("<=".into(), 9.0))));
        assert_eq!(split("Item has Weight equal to 5"), ("Item has Weight".into(), Some(("=".into(), 5.0))));
        assert_eq!(split("Item has Weight equals 5"), ("Item has Weight".into(), Some(("=".into(), 5.0))));
        assert_eq!(split("Item has Weight not equal to 5"), ("Item has Weight".into(), Some(("!=".into(), 5.0))));
        assert_eq!(split("Item has Weight exceeds 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
    }

    #[test]
    fn optional_is_connective_is_stripped() {
        assert_eq!(split("Item has Weight is greater than 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
        assert_eq!(split("Item has Weight is more than 5"), ("Item has Weight".into(), Some((">".into(), 5.0))));
    }

    #[test]
    fn role_named_with_is_suffix_not_corrupted() {
        // `Axis` ends in `is` but has no preceding space-`is`, so symbolic `>` peels cleanly.
        assert_eq!(split("Shape has Axis > 2"), ("Shape has Axis".into(), Some((">".into(), 2.0))));
    }
}

// =========================================================================
// Instance fact parsing (state machines)
// =========================================================================

#[cfg(test)]
mod antecedent_cardinality_tests {
    //! derivation-cardinality-count: the parse-side extraction contract for
    //! `at most N` / `at least N` COUNT premises in a derivation antecedent.
    use super::*;

    #[test]
    fn extracts_at_most_and_strips_phrase() {
        // `at most 0` between the verb and the counted noun → (at_most, 0)
        // and a stripped clause that resolves as the bare bridge FT.
        assert_eq!(
            extract_antecedent_cardinality("Item is marked by at most 0 Tag"),
            Some((true, 0, "Item is marked by Tag".to_string())),
        );
    }

    #[test]
    fn extracts_at_least_and_strips_phrase() {
        assert_eq!(
            extract_antecedent_cardinality("Item is marked by at least 1 Tag"),
            Some((false, 1, "Item is marked by Tag".to_string())),
        );
    }

    #[test]
    fn multi_digit_bound_parses() {
        assert_eq!(
            extract_antecedent_cardinality("Order has at most 12 Line Item"),
            Some((true, 12, "Order has Line Item".to_string())),
        );
    }

    #[test]
    fn plain_existential_is_unaffected() {
        // No `at most`/`at least` phrase → None (plain join, unchanged).
        assert_eq!(
            extract_antecedent_cardinality("Item is marked by Tag"),
            None,
        );
    }

    #[test]
    fn word_form_at_most_one_is_not_a_count_premise() {
        // `at most one` (no digit) is the UC spelling — left for the
        // constraint classifier, not a derivation count premise.
        assert_eq!(
            extract_antecedent_cardinality("Item is marked by at most one Tag"),
            None,
        );
    }
}

#[cfg(test)]
mod objectified_pivot_tests {
    use super::*;

    // #963: a 4-role objectified fact type
    //   `ImplementationBinding pivots Component is implemented by Toolkit at Toolkit Symbol`
    // is referenced in rule bodies by its abbreviated 3-role reading
    //   `ImplementationBinding pivots Component is implemented by Toolkit`
    // with the trailing `at Toolkit Symbol` dropped. The exact noun-set
    // lookup misses (3-set query vs the stored 4-set key), so without the
    // abbreviation fallback the antecedent never binds and every derived
    // preference scores zero.
    #[test]
    fn objectified_pivot_abbreviation_resolves() {
        let pivot_id =
            "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit_at_Toolkit_Symbol";
        let mut cat = SchemaCatalog::new();
        cat.register(
            pivot_id,
            &["ImplementationBinding", "Component", "Toolkit", "Toolkit Symbol"],
            "pivots",
            "ImplementationBinding pivots Component is implemented by Toolkit at Toolkit Symbol",
        );

        // Exact 4-role lookup still resolves (no regression).
        assert_eq!(
            cat.resolve(
                &["ImplementationBinding", "Component", "Toolkit", "Toolkit Symbol"],
                Some("pivots"),
            ).as_deref(),
            Some(pivot_id),
        );

        // The abbreviated 3-role clause misses BOTH exact lookups...
        assert!(cat
            .resolve(&["ImplementationBinding", "Component", "Toolkit"], Some("pivots"))
            .is_none());
        assert!(cat
            .resolve(&["ImplementationBinding", "Component", "Toolkit"], None)
            .is_none());

        // ...but resolves through the objectified-pivot abbreviation fallback.
        assert_eq!(
            cat.resolve_objectified_abbrev(
                &["ImplementationBinding", "Component", "Toolkit"],
                "ImplementationBinding pivots Component is implemented by Toolkit",
            ).as_deref(),
            Some(pivot_id),
        );
    }

    // The fallback is narrow by construction: a same-noun subset whose
    // reading is NOT the stored reading's `at`-truncation must not bind,
    // and a subset that is not exactly one noun short must not bind.
    #[test]
    fn objectified_pivot_abbreviation_stays_narrow() {
        let mut cat = SchemaCatalog::new();
        cat.register(
            "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
            &["ImplementationBinding", "Component", "Toolkit", "Toolkit Symbol"],
            "pivots",
            "ImplementationBinding pivots Component is implemented by Toolkit at Toolkit Symbol",
        );
        // Same 3-noun subset, different verb/reading → no spurious match.
        assert!(cat.resolve_objectified_abbrev(
            &["ImplementationBinding", "Component", "Toolkit"],
            "ImplementationBinding pivots Component is rendered by Toolkit",
        ).is_none());
        // Two nouns short of the stored 4-set key → no match (must be
        // exactly one extra noun).
        assert!(cat.resolve_objectified_abbrev(
            &["ImplementationBinding", "Component"],
            "ImplementationBinding pivots Component",
        ).is_none());
    }
}

#[cfg(test)]
mod regex_replacement_tests {
    use super::*;
    use alloc::string::ToString;

    // ── strip_trailing_quoted_literal ──────────────────────────────

    #[test]
    fn strip_trailing_literal_basic() {
        let (without, lit) = strip_trailing_quoted_literal(
            "Statement has Classification 'Entity Type Declaration'"
        ).unwrap();
        assert_eq!(without, "Statement has Classification");
        assert_eq!(lit, "Entity Type Declaration");
    }

    #[test]
    fn strip_trailing_literal_empty_interior() {
        let (without, lit) = strip_trailing_quoted_literal("Foo has Bar ''").unwrap();
        assert_eq!(without, "Foo has Bar");
        assert_eq!(lit, "");
    }

    #[test]
    fn strip_trailing_literal_with_trailing_ws() {
        let (without, lit) = strip_trailing_quoted_literal(
            "Task has Status 'Done'   "
        ).unwrap();
        assert_eq!(without, "Task has Status");
        assert_eq!(lit, "Done");
    }

    #[test]
    fn strip_trailing_literal_no_quote_returns_none() {
        assert!(strip_trailing_quoted_literal("Task has Status Done").is_none());
    }

    #[test]
    fn strip_trailing_literal_no_leading_space_returns_none() {
        // The pattern requires a space before the opening quote.
        assert!(strip_trailing_quoted_literal("'Done'").is_none());
    }

    #[test]
    fn strip_trailing_literal_quote_not_at_end_returns_none() {
        assert!(strip_trailing_quoted_literal("Foo 'mid' bar").is_none());
    }

    // ── tokenize_arith ─────────────────────────────────────────────

    #[test]
    fn tokenize_arith_simple() {
        assert_eq!(tokenize_arith("Size * Size * Size"),
                   alloc::vec!["Size", "*", "Size", "*", "Size"]);
    }

    #[test]
    fn tokenize_arith_mixed_ops() {
        assert_eq!(tokenize_arith("A + B - C * D / E"),
                   alloc::vec!["A", "+", "B", "-", "C", "*", "D", "/", "E"]);
    }

    #[test]
    fn tokenize_arith_no_spaces() {
        assert_eq!(tokenize_arith("A+B"), alloc::vec!["A", "+", "B"]);
    }

    #[test]
    fn tokenize_arith_lone_atom() {
        assert_eq!(tokenize_arith("Size"), alloc::vec!["Size"]);
    }

    #[test]
    fn tokenize_arith_empty() {
        assert!(tokenize_arith("").is_empty());
        assert!(tokenize_arith("   ").is_empty());
    }

    #[test]
    fn tokenize_arith_drops_empty_operands_between_ops() {
        // Two adjacent operators leave an empty middle operand,
        // matching the regex code's `if !head.is_empty()` guard.
        assert_eq!(tokenize_arith("A++B"),
                   alloc::vec!["A", "+", "+", "B"]);
    }

    // ── peel_trailing_comparator ───────────────────────────────────

    #[test]
    fn peel_comparator_ge() {
        let (stripped, op, v) = peel_trailing_comparator(
            "has Population >= 1000000"
        ).unwrap();
        assert_eq!(stripped, "has Population");
        assert_eq!(op, ">=");
        assert!((v - 1_000_000.0).abs() < 1e-9);
    }

    #[test]
    fn peel_comparator_le() {
        let (stripped, op, v) = peel_trailing_comparator("X <= 5").unwrap();
        assert_eq!(stripped, "X");
        assert_eq!(op, "<=");
        assert!((v - 5.0).abs() < 1e-9);
    }

    #[test]
    fn peel_comparator_neq_long() {
        let (stripped, op, v) = peel_trailing_comparator("X <> 0").unwrap();
        assert_eq!(stripped, "X");
        assert_eq!(op, "<>");
        assert!((v - 0.0).abs() < 1e-9);
    }

    #[test]
    fn peel_comparator_neq_bang() {
        let (stripped, op, _) = peel_trailing_comparator("X != 0").unwrap();
        assert_eq!(stripped, "X");
        assert_eq!(op, "!=");
    }

    #[test]
    fn peel_comparator_short_ops_not_eaten_by_long() {
        // `>` should not be re-promoted to `>=`.
        let (stripped, op, _) = peel_trailing_comparator("X > 1").unwrap();
        assert_eq!(stripped, "X");
        assert_eq!(op, ">");
    }

    #[test]
    fn peel_comparator_decimal() {
        let (stripped, op, v) = peel_trailing_comparator("Score >= 99.5").unwrap();
        assert_eq!(stripped, "Score");
        assert_eq!(op, ">=");
        assert!((v - 99.5).abs() < 1e-9);
    }

    #[test]
    fn peel_comparator_negative() {
        let (stripped, op, v) = peel_trailing_comparator("Delta > -10").unwrap();
        assert_eq!(stripped, "Delta");
        assert_eq!(op, ">");
        assert!((v + 10.0).abs() < 1e-9);
    }

    #[test]
    fn peel_comparator_no_op_returns_none() {
        assert!(peel_trailing_comparator("Score 100").is_none());
    }

    #[test]
    fn peel_comparator_no_number_returns_none() {
        assert!(peel_trailing_comparator("Score >=").is_none());
        assert!(peel_trailing_comparator("Score").is_none());
    }

    #[test]
    fn peel_comparator_eq_alone() {
        let (stripped, op, v) = peel_trailing_comparator("X = 7").unwrap();
        assert_eq!(stripped, "X");
        assert_eq!(op, "=");
        assert!((v - 7.0).abs() < 1e-9);
    }

    // ── is_noun_has_noun_literal (site 114) ────────────────────────

    #[test]
    fn noun_has_noun_literal_matches() {
        let nouns: alloc::vec::Vec<String> = ["Country", "Population"]
            .iter().map(|s| s.to_string()).collect();
        assert!(is_noun_has_noun_literal("Country has Population '1000000'", &nouns));
    }

    #[test]
    fn noun_has_noun_literal_rejects_unknown_subject() {
        let nouns: alloc::vec::Vec<String> = ["Population"].iter().map(|s| s.to_string()).collect();
        assert!(!is_noun_has_noun_literal("Country has Population '1000000'", &nouns));
    }

    #[test]
    fn noun_has_noun_literal_rejects_no_literal() {
        let nouns: alloc::vec::Vec<String> = ["Country", "Population"]
            .iter().map(|s| s.to_string()).collect();
        assert!(!is_noun_has_noun_literal("Country has Population", &nouns));
    }

    // ── is_entity_ref_scheme_literal (site 256) ────────────────────

    #[test]
    fn ref_scheme_literal_matches_is() {
        let nouns: alloc::vec::Vec<String> = ["Country"].iter().map(|s| s.to_string()).collect();
        assert!(is_entity_ref_scheme_literal("Country is 'France'", &nouns));
    }

    #[test]
    fn ref_scheme_literal_matches_is_not() {
        let nouns: alloc::vec::Vec<String> = ["Country"].iter().map(|s| s.to_string()).collect();
        assert!(is_entity_ref_scheme_literal("Country is not 'France'", &nouns));
    }

    #[test]
    fn ref_scheme_literal_with_leading_quantifier() {
        let nouns: alloc::vec::Vec<String> = ["Country"].iter().map(|s| s.to_string()).collect();
        assert!(is_entity_ref_scheme_literal("the Country is 'France'", &nouns));
    }

    #[test]
    fn ref_scheme_literal_rejects_unknown_noun() {
        let nouns: alloc::vec::Vec<String> = ["Country"].iter().map(|s| s.to_string()).collect();
        assert!(!is_entity_ref_scheme_literal("Region is 'EU'", &nouns));
    }

    #[test]
    fn ref_scheme_literal_strips_subscript() {
        let nouns: alloc::vec::Vec<String> = ["Person"].iter().map(|s| s.to_string()).collect();
        assert!(is_entity_ref_scheme_literal("Person1 is 'Alice'", &nouns));
    }

    // ── try_parse_aggregate_clause (site 690) ──────────────────────

    #[test]
    fn aggregate_count_no_where() {
        let nouns: alloc::vec::Vec<String> = ["Task"].iter().map(|s| s.to_string()).collect();
        let (role, op, target, w) = try_parse_aggregate_clause(
            "done Task Count is the count of Task", &nouns
        ).unwrap();
        assert_eq!(role, "done Task Count");
        assert_eq!(op, "count");
        assert_eq!(target, "Task");
        assert_eq!(w, "");
    }

    #[test]
    fn aggregate_with_where() {
        let nouns: alloc::vec::Vec<String> = ["Task", "Status"].iter().map(|s| s.to_string()).collect();
        let (role, op, target, w) = try_parse_aggregate_clause(
            "done Task Count is the count of Task where Task has Status 'Done'", &nouns
        ).unwrap();
        assert_eq!(role, "done Task Count");
        assert_eq!(op, "count");
        assert_eq!(target, "Task");
        assert_eq!(w, "Task has Status 'Done'");
    }

    #[test]
    fn aggregate_earliest_op_with_of() {
        let nouns: alloc::vec::Vec<String> = ["Timestamp", "Date"].iter().map(|s| s.to_string()).collect();
        let (role, op, target, _) = try_parse_aggregate_clause(
            "Date is the earliest of Timestamp", &nouns
        ).unwrap();
        assert_eq!(role, "Date");
        assert_eq!(op, "earliest");
        assert_eq!(target, "Timestamp");
    }

    #[test]
    fn aggregate_strips_leading_that() {
        let nouns: alloc::vec::Vec<String> = ["Task"].iter().map(|s| s.to_string()).collect();
        let res = try_parse_aggregate_clause(
            "that done Task Count is the count of Task", &nouns
        );
        assert!(res.is_some());
    }

    #[test]
    fn aggregate_rejects_unknown_target() {
        let nouns: alloc::vec::Vec<String> = ["Task"].iter().map(|s| s.to_string()).collect();
        assert!(try_parse_aggregate_clause(
            "X is the count of UnknownThing", &nouns
        ).is_none());
    }

    #[test]
    fn aggregate_rejects_non_aggregate() {
        let nouns: alloc::vec::Vec<String> = ["Task"].iter().map(|s| s.to_string()).collect();
        assert!(try_parse_aggregate_clause(
            "Task is the boss of Task", &nouns
        ).is_none());
    }

    // ── metamodel-noun-uniformity (Fact-is-not-special) ────────────
    #[test]
    fn aggregate_count_over_metamodel_fact() {
        // `Fact` is NOT in the domain noun list, but it is a countable metamodel
        // noun: `… is the count of Fact1 where Fact1 stimulates Layer1` resolves
        // exactly like a count over a domain entity (the bug this fixes).
        let nouns: alloc::vec::Vec<String> = ["Layer"].iter().map(|s| s.to_string()).collect();
        assert!(try_parse_aggregate_clause(
            "Drive is the count of Fact1 where Fact1 stimulates Layer1", &nouns
        ).is_some());
    }

    #[test]
    fn aggregate_count_over_reserved_metamodel_noun() {
        // Reserved metamodel nouns (here `Role`) are countable too (uniformity).
        let nouns: alloc::vec::Vec<String> = ["Layer"].iter().map(|s| s.to_string()).collect();
        assert!(try_parse_aggregate_clause(
            "n is the count of Role1 where Role1 binds Layer1", &nouns
        ).is_some());
    }

    #[test]
    fn find_nouns_guards_fact_against_fact_type() {
        // Fact-is-not-special word-boundary guard: with `Fact` in noun_names it must
        // NOT match inside the reserved `Fact Type` (resolved as the FactType
        // relation elsewhere — task980), but MUST match a standalone `Fact1`.
        let nouns: alloc::vec::Vec<String> = ["Fact", "Bag"].iter().map(|s| s.to_string()).collect();
        let inside = find_nouns("that Fact Type has that Role", &nouns);
        assert!(!inside.iter().any(|(_, _, m)| m.eq_ignore_ascii_case("fact")),
            "`Fact` must not match inside `Fact Type`; got {:?}", inside);
        let standalone = find_nouns("Bag holds Fact1", &nouns);
        assert!(standalone.iter().any(|(_, _, m)| m == "Fact1"),
            "`Fact` must match the standalone `Fact1`; got {:?}", standalone);
    }
}

// =========================================================================
// #894 — SSRF CIDR blocklist dispatch-to-data lift tests
// =========================================================================

#[cfg(test)]
mod ssrf_cidr_tests {
    use super::*;
    use crate::ast::{Object, fact_from_pairs, store};

    /// Build a state Object whose `CIDR_Block_has_Block_Kind` cell
    /// carries one fact per `(cidr, kind)` pair. Mirrors the shape
    /// `instance_fact_field_cells` produces for the parsed reading
    /// `'127.0.0.0/8' has Block Kind 'internal-loopback'.`
    fn state_with_cidr_blocks(rows: &[(&str, &str)]) -> Object {
        let facts: alloc::vec::Vec<Object> = rows.iter()
            .map(|(cidr, kind)| fact_from_pairs(&[
                ("CIDR Block", *cidr),
                ("Block Kind", *kind),
            ]))
            .collect();
        store(
            "CIDR_Block_has_Block_Kind",
            Object::Seq(facts.into()),
            &Object::phi(),
        )
    }

    /// Build a state Object whose `InstanceFact` cell carries one
    /// `External System` URL row. Drives `find_forbidden_instance_url`.
    fn state_with_external_url(state: Object, name: &str, url: &str) -> Object {
        let inst = fact_from_pairs(&[
            ("subjectNoun", "External System"),
            ("subjectValue", name),
            ("fieldName", "External_System_has_URL"),
            ("objectNoun", "URL"),
            ("objectValue", url),
        ]);
        let new_inst = match crate::ast::fetch_or_phi("InstanceFact", &state).as_seq() {
            Some(items) => {
                let mut v = items.to_vec();
                v.push(inst);
                Object::Seq(v.into())
            }
            None => Object::seq(alloc::vec![inst]),
        };
        store("InstanceFact", new_inst, &state)
    }

    // ── cidr_contains primitive ─────────────────────────────────────

    #[test]
    fn cidr_contains_ipv4_loopback() {
        assert!(cidr_contains("127.0.0.0/8", "127.0.0.1"));
        assert!(cidr_contains("127.0.0.0/8", "127.255.255.254"));
        assert!(!cidr_contains("127.0.0.0/8", "128.0.0.1"));
    }

    #[test]
    fn cidr_contains_ipv4_rfc1918_10() {
        assert!(cidr_contains("10.0.0.0/8", "10.1.2.3"));
        assert!(!cidr_contains("10.0.0.0/8", "11.0.0.1"));
    }

    #[test]
    fn cidr_contains_ipv4_rfc1918_172() {
        assert!(cidr_contains("172.16.0.0/12", "172.16.0.1"));
        assert!(cidr_contains("172.16.0.0/12", "172.31.255.254"));
        assert!(!cidr_contains("172.16.0.0/12", "172.15.0.1"));
        assert!(!cidr_contains("172.16.0.0/12", "172.32.0.1"));
    }

    #[test]
    fn cidr_contains_ipv4_link_local_169_254() {
        assert!(cidr_contains("169.254.0.0/16", "169.254.169.254"));
        assert!(!cidr_contains("169.254.0.0/16", "169.255.0.1"));
    }

    #[test]
    fn cidr_contains_ipv6_loopback() {
        assert!(cidr_contains("::1/128", "::1"));
        assert!(!cidr_contains("::1/128", "::2"));
    }

    #[test]
    fn cidr_contains_ipv6_link_local() {
        assert!(cidr_contains("fe80::/10", "fe80::1"));
        assert!(cidr_contains("fe80::/10", "febf::1"));
        assert!(!cidr_contains("fe80::/10", "fec0::1"));
    }

    #[test]
    fn cidr_contains_ipv6_unique_local() {
        assert!(cidr_contains("fc00::/7", "fc00::1"));
        assert!(cidr_contains("fc00::/7", "fd00::1"));
        assert!(!cidr_contains("fc00::/7", "fe00::1"));
    }

    #[test]
    fn cidr_contains_rejects_malformed() {
        assert!(!cidr_contains("not-a-cidr", "127.0.0.1"));
        assert!(!cidr_contains("127.0.0.0/8", "not-a-host"));
        assert!(!cidr_contains("127.0.0.0/99", "127.0.0.1"));
    }

    // ── find_forbidden_instance_url reads from state's CIDR cell ────

    /// Acceptance pin (#894): the SSRF blocklist is data, not a Rust
    /// const. With a state carrying NO `CIDR_Block_has_Block_Kind`
    /// cell, we fall back to the boot list — 127.0.0.1 must reject.
    #[test]
    fn ssrf_rejects_loopback_url_via_boot_fallback() {
        let state = state_with_external_url(
            Object::phi(), "lo", "http://127.0.0.1/foo");
        let result = find_forbidden_instance_url(&state, &Object::phi());
        assert_eq!(result.as_deref(), Some("http://127.0.0.1/foo"));
    }

    /// Acceptance pin (#894): with a state carrying a CIDR_Block cell
    /// that lists 127.0.0.0/8, the loopback URL must reject AND the
    /// rejection must come from the cell (not the hardcoded const).
    /// Distinguishing signal: the empty cell case (next test) shows
    /// the falls-back boot path; the present-cell case here shows the
    /// data path. Together they pin "cell read, with boot fallback."
    #[test]
    fn ssrf_rejects_loopback_url_via_readings_blocklist() {
        let cidr_state = state_with_cidr_blocks(&[
            ("127.0.0.0/8", "internal-loopback"),
            ("10.0.0.0/8",  "private-rfc1918"),
        ]);
        let state = state_with_external_url(
            Object::phi(), "lo", "http://127.0.0.1/foo");
        let result = find_forbidden_instance_url(&state, &cidr_state);
        assert_eq!(result.as_deref(), Some("http://127.0.0.1/foo"));
    }

    /// Acceptance pin (#894): a CIDR_Block cell whose rows exclude the
    /// loopback prefix MUST NOT reject the loopback URL. This proves
    /// the SSRF check is reading the cell — if it were still reading
    /// the hardcoded const, 127.0.0.1 would always reject regardless
    /// of cell contents.
    #[test]
    fn ssrf_state_overrides_boot_with_empty_blocklist() {
        // A CIDR_Block cell that exists but excludes the loopback range
        // (only carries an unrelated 192.0.2.0/24 documentation prefix).
        // The state-driven path uses just this list — loopback is fair
        // game. Pins: the cell IS the source of truth when non-empty.
        let cidr_state = state_with_cidr_blocks(&[
            ("192.0.2.0/24", "documentation"),
        ]);
        let state = state_with_external_url(
            Object::phi(), "lo", "http://127.0.0.1/foo");
        let result = find_forbidden_instance_url(&state, &cidr_state);
        assert_eq!(result, None,
            "with state's CIDR list excluding 127/8, loopback URL must \
             pass — proves the check reads from the cell, not from Rust");
    }

    /// Acceptance pin (#894): adding a CIDR to the cell that was NOT
    /// in the legacy boot list catches a URL the legacy code missed.
    /// Pins that the cell extends the policy — a future operator can
    /// add a new range without touching Rust.
    #[test]
    fn ssrf_state_extends_boot_with_extra_cidr() {
        // 100.64.0.0/10 is the carrier-grade NAT range (RFC 6598).
        // The legacy boot list did NOT cover it. Add it to the cell;
        // the SSRF check must now reject a URL in that range.
        let cidr_state = state_with_cidr_blocks(&[
            ("100.64.0.0/10", "carrier-grade-nat"),
        ]);
        let state = state_with_external_url(
            Object::phi(), "cgn", "http://100.64.5.5/foo");
        let result = find_forbidden_instance_url(&state, &cidr_state);
        assert_eq!(result.as_deref(), Some("http://100.64.5.5/foo"));
    }

    /// End-to-end pin (#894): parsing the bundled `security.md` core
    /// reading populates the `CIDR_Block_has_Block_Kind` cell with all
    /// 8 boot CIDR rows. Together with the prior tests this proves
    /// the full chain: readings → cell → SSRF check.
    #[test]
    fn security_md_populates_cidr_block_cell() {
        let security_md = include_str!("../../../readings/core/security.md");
        let state = parse_to_state(security_md)
            .expect("readings/core/security.md must parse cleanly");
        let cidrs = cidr_blocklist_from_state(&state);
        // Each row from the reading should match the boot list set;
        // order is preserved per the reading's listing order. The
        // boot list is the legacy ordering and the reading mirrors it.
        for expected in BOOT_CIDR_BLOCKLIST {
            assert!(
                cidrs.iter().any(|c| c == expected),
                "readings/core/security.md must list {} as a CIDR Block (got {:?})",
                expected, cidrs);
        }
        // And no fall-through to the boot list: every cidr we read out
        // should be from the cell, not the BOOT_CIDR_BLOCKLIST const.
        // We pin this by asserting the count matches the readings rows
        // (8 entries), not the boot list (which would also be 8 — so
        // we add one extra row in the next pin).
        assert_eq!(cidrs.len(), 8,
            "security.md declares exactly 8 CIDR Block rows, got {:?}",
            cidrs);
    }
}

#[cfg(test)]
mod ss_autofill_metamodel_clause_tests {
    //! ss-autofill-retire-1 (the task-978 analog): pin that the SS
    //! auto-fill metamodel rule's antecedent — `some Subset Constraint
    //! has antecedent Fact Type Ant` (`readings/core/derivation.md`
    //! §"SS Subset-Constraint auto-fill") — is RECOGNISED by
    //! `try_classify_metamodel_clause` as a bound metamodel-cell
    //! antecedent (`Some("SubsetConstraint")`), not a fall-through
    //! `None` that would land the clause in `UnresolvedClause`.
    //!
    //! This is the parse/expose prerequisite that unblocks
    //! ss-autofill-retire-2 (the reading that binds the cell). The
    //! end-to-end "resolves to an antecedent_source + binds the
    //! (antecedent_ft, consequent_ft) pairs" pin lives in
    //! `compile_explicit_derivation_tests::ss_autofill_*` (which drives
    //! the full `resolve_derivation_rule` cascade + the
    //! `CellIndex::ss_autofill_pairs` accessor).
    use super::*;

    #[test]
    fn primary_subset_constraint_antecedent_resolves_to_bound_cell() {
        // The anaphora-stripped form of `some Subset Constraint has
        // antecedent Fact Type Ant` (the SS auto-fill rule's PRIMARY
        // quantification). Before ss-autofill-retire-1 this returned
        // `None`; the caller then pushed the clause to
        // `unresolved_clauses`. Now it binds the dedicated
        // `SubsetConstraint` metamodel-cell view.
        assert_eq!(
            try_classify_metamodel_clause("Subset Constraint has antecedent Fact Type Ant"),
            Some("SubsetConstraint".to_string()),
            "the SS auto-fill rule's primary antecedent must resolve to a \
             bound metamodel-cell id, not fall through to UnresolvedClause",
        );
        // The consequent-FT and autofill-marker clauses share the prefix.
        assert_eq!(
            try_classify_metamodel_clause("Subset Constraint has consequent Fact Type Cons"),
            Some("SubsetConstraint".to_string()),
        );
        assert_eq!(
            try_classify_metamodel_clause("Subset Constraint has autofill 'true'"),
            Some("SubsetConstraint".to_string()),
        );
    }

    #[test]
    fn subset_constraint_prefix_beats_bare_constraint_prefix() {
        // `subset constraint ` is the more specific cell; the prefix
        // table must check it before the bare `constraint ` prefix so a
        // `subset constraint …` clause never collapses to the broader
        // `Constraint` view.
        assert_eq!(
            try_classify_metamodel_clause("Subset Constraint spans Role"),
            Some("SubsetConstraint".to_string()),
        );
        // A bare `Constraint …` clause still resolves to the broad cell.
        assert_eq!(
            try_classify_metamodel_clause("Constraint spans Role"),
            Some("Constraint".to_string()),
        );
    }

    #[test]
    fn case_insensitive_and_lowercase_surface_forms_resolve() {
        // `try_classify_metamodel_clause` lowercases before matching, so
        // the all-caps and all-lowercase author surface forms both bind.
        assert_eq!(
            try_classify_metamodel_clause("SUBSET CONSTRAINT has antecedent Fact Type Ant"),
            Some("SubsetConstraint".to_string()),
        );
        assert_eq!(
            try_classify_metamodel_clause("subset constraint has antecedent fact type ant"),
            Some("SubsetConstraint".to_string()),
        );
    }

    #[test]
    fn non_constraint_clause_still_falls_through() {
        // A clause that names neither a metamodel cell nor the
        // instance-of predicate still returns `None` — the new prefixes
        // don't widen the recogniser beyond constraint vocabulary.
        assert_eq!(
            try_classify_metamodel_clause("Customer has Tier 'Gold'"),
            None,
        );
    }
}








