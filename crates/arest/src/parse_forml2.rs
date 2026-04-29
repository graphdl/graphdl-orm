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

// ── Parse mode stubs ────────────────────────────────────────────────
//
// The legacy cascade consulted per-thread BOOTSTRAP_MODE (allow
// metamodel noun redeclaration during bundled-readings load) and
// STRICT_MODE (reject undeclared partition subtypes instead of
// auto-creating them). Both behaviours are gone: stage12 doesn't
// apply the metamodel-redeclaration guard (that check lives in
// `ast::find_metamodel_shadow`, disabled at the platform_compile
// boundary for unrelated reasons), and stage12's noun detection
// derives declared names directly from text so the loose
// auto-creation path never triggers either.
//
// The public setters are kept as no-ops so `lib.rs`, `main.rs`, and
// the example binaries compile unchanged — their calls were
// scaffolding around the legacy guard, not a behavioural request.

/// No-op after #285: stage12 doesn't apply the legacy metamodel
/// redeclaration guard. Retained for API compatibility with callers
/// that wrap bundled-reading loads in `set_bootstrap_mode(true)` /
/// `set_bootstrap_mode(false)` guards.
pub fn set_bootstrap_mode(_on: bool) {}

/// No-op after #285: the loose/strict distinction disappeared with
/// the legacy cascade. Retained so `main.rs --strict` still
/// type-checks.
#[allow(dead_code)]
pub(crate) fn set_strict_mode(_on: bool) {}












/// True when `clause` starts with `for each <Noun>` and is followed
/// by at least one more declared noun reference (the predicate over
/// the universally-quantified variable). Accepts universal-quantifier
/// antecedents like
///     for each Authority that applies to that Support Response,
///       that Support Response satisfies that Authority
/// so the overall derivation rule is not flagged as unresolved.
fn is_universal_quantifier_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim();
    let Some(after) = trimmed.strip_prefix("for each ") else { return false; };
    // Must mention a declared noun after `for each`.
    noun_names.iter().any(|n| after.starts_with(n.as_str()))
        // ...and at least one more noun reference in the tail.
        && noun_names.iter().any(|n| {
            let needle = format!(" {}", n);
            after.contains(&needle)
        })
}

/// True when `clause` has the shape `<Noun> is extracted from <Noun>`
/// or `<Noun> is derived from <Noun>`. Both operands must be declared.
/// Used for ML-style computed bindings (free-text extraction,
/// classifier outputs) where the underlying extractor is registered
/// at runtime. Classification here suppresses the false-unresolved
/// noise; the actual extraction function lives in DEFS.
fn is_extraction_clause(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    [" is extracted from ", " is derived from "].iter().any(|kw| {
        let Some(idx) = trimmed.find(kw) else { return false; };
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + kw.len()..].trim();
        let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
        is_noun(lhs) && is_noun(rhs)
    })
}

/// Strip existential / anaphoric quantifiers from FT references so
/// `Feature Request concerns some API Product` resolves against the
/// declared `Feature Request concerns API Product`. Only ` some ` and
/// ` that ` (as whole-word tokens) are removed — the surrounding
/// noun / verb text is untouched.
fn strip_existential_quantifiers(clause: &str) -> String {
    clause
        .replace(" some ", " ")
        .replace(" that ", " ")
        .replace("  ", " ")
        .trim()
        .to_string()
}

/// True when `clause` has the shape `<Noun> has <Noun> '<literal>'`
/// with both nouns declared. Accepts state-machine status filters and
/// enum-value filters where the underlying FT isn't always declared
/// textually (e.g. Status is SM-managed).
fn is_noun_has_noun_literal(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim().trim_end_matches('.');
    // Hand-rolled equivalent of `^(.+?) has (.+?) '[^']*'$`.
    // Strip the trailing space-prefixed quoted literal, then split on
    // the first ` has ` to recover (subj, attr).
    let Some((without_literal, _)) = strip_trailing_quoted_literal(trimmed) else {
        return false;
    };
    let Some(idx) = without_literal.find(" has ") else { return false; };
    let subj = without_literal[..idx].trim();
    let attr = without_literal[idx + " has ".len()..].trim();
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
fn expand_that_relatives(
    antecedent: &str,
    noun_names: &[String],
    catalog: &SchemaCatalog,
) -> String {
    let mut current = antecedent.to_string();
    loop {
        let positions: Vec<usize> = current
            .match_indices(" that ")
            .map(|(i, _)| i)
            .collect();
        let expand_at = positions.into_iter().find(|&i| {
            let tail = &current[i + " that ".len()..];
            let tail_trim = tail.trim_start();
            if is_that_anaphora_ref(tail_trim, noun_names) { return false; }
            // Only expand when the head — text up to this ` that ` —
            // resolves to a declared FT. Otherwise leave the clause
            // for downstream classifiers to handle whole.
            let head = &current[..i];
            head_resolves(head, noun_names, catalog)
        });
        let Some(pos) = expand_at else { break; };
        let head = &current[..pos];
        let tail = &current[pos + " that ".len()..];
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
fn is_that_anaphora_ref(tail: &str, noun_names: &[String]) -> bool {
    // Shape 1 + 2: <Noun> or <Noun><digits>
    if noun_names.iter().any(|n| {
        let Some(after) = tail.strip_prefix(n.as_str()) else { return false; };
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
    // right end. Existing code only consumes the captured subject.
    let Some((without_literal, _)) = strip_trailing_quoted_literal(stripped) else {
        return false;
    };
    let raw_subj = without_literal
        .strip_suffix(" is not")
        .or_else(|| without_literal.strip_suffix(" is"));
    let Some(raw_subj) = raw_subj else { return false; };
    let raw_subj = raw_subj.trim();
    let (base, _) = parse_role_token(raw_subj);
    noun_names.iter().any(|n| n == base)
}

/// True when `clause` has the shape `<Noun> is (a|an) <Noun>` with
/// both sides resolving to declared nouns. Treated as a typing
/// predicate rather than a fact-type reference.
fn is_subtype_instance_check(clause: &str, noun_names: &[String]) -> bool {
    let trimmed = clause.trim();
    [" is a ", " is an "].iter().any(|kw| {
        let Some(idx) = trimmed.find(kw) else { return false; };
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + kw.len()..].trim();
        let is_noun = |s: &str| noun_names.iter().any(|n| n == s);
        is_noun(lhs) && is_noun(rhs)
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
    const RANGE_OPS: &[&str] = &[" within ", " before ", " after "];
    RANGE_OPS.iter().any(|op| {
        let Some(idx) = clause.find(op) else { return false; };
        let head = clause[..idx].trim();
        head_resolves(head, noun_names, catalog)
    })
}

/// #277 Category F — bare-value tail comparisons like
/// `HTTP Status of 500 or more`, `HTTP Status of 500 or less`,
/// `HTTP Status of at least 500`, `HTTP Status of at most 500`.
/// The FT reference is the subject noun; the `of <N> <comparator>`
/// tail is an implicit comparator filter on the value side.
fn is_bare_value_comparison(clause: &str, noun_names: &[String]) -> bool {
    const TAILS: &[&str] = &[
        " or more", " or less", " or greater", " or fewer",
    ];
    let trimmed = clause.trim().trim_end_matches('.');
    let ends_with_tail = TAILS.iter().any(|t| trimmed.ends_with(t));
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
    const COMPARATORS: &[&str] = &[
        " exceeds ", " is greater than ", " is less than ",
        " is at least ", " is at most ", " is more than ",
        " equals ", " is equal to ",
    ];
    COMPARATORS.iter().any(|kw| {
        let Some(idx) = clause.find(kw) else { return false; };
        let lhs = clause[..idx].trim();
        let rhs = clause[idx + kw.len()..].trim();
        let side_has_noun = |side: &str| noun_names.iter().any(|n| {
            // Whole-side match or noun as a whole-word substring.
            side == n
                || side.starts_with(&format!("{} ", n))
                || side.ends_with(&format!(" {}", n))
                || side.contains(&format!(" {} ", n))
        });
        side_has_noun(lhs) && side_has_noun(rhs)
    })
}















// =========================================================================
// Main parser -- fold recognizers over lines
// =========================================================================



/// SSRF defense (#25). Reject URLs that point at internal/loopback/link-local
/// networks, file:// schemes, or internal DNS names. Hardcoded patterns only â€”
/// no DNS resolution, no network I/O. Called during platform_compile to
/// validate External System instance facts before they enter state.
pub fn is_forbidden_url(url: &str) -> bool {
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

    // IPv4 checks: parse dotted-quad octets. Non-numeric hosts fall through.
    let octets: Vec<u16> = host_bare.split('.')
        .filter_map(|p| p.parse::<u16>().ok())
        .collect();
    let is_ipv4 = octets.len() == 4 && octets.iter().all(|o| *o <= 255);
    match is_ipv4 {
        true => {
            let (a, b) = (octets[0], octets[1]);
            // 127.*.*.* loopback
            // 10.*.*.* private
            // 169.254.*.* link-local (incl. AWS metadata 169.254.169.254)
            // 192.168.*.* private
            // 172.16-31.*.* private
            let forbidden_v4 = a == 127
                || a == 10
                || (a == 169 && b == 254)
                || (a == 192 && b == 168)
                || (a == 172 && b >= 16 && b <= 31);
            match forbidden_v4 {
                true => return true,
                false => {}
            }
        }
        false => {}
    }

    // IPv6 link-local: fe80::/10 â€” first octet of the address
    // is 0xfe and top two bits of the second are 10 (0x80..0xbf).
    // Covers fe80: through febf:.
    let ipv6_linklocal = host_bare.starts_with("fe8")
        || host_bare.starts_with("fe9")
        || host_bare.starts_with("fea")
        || host_bare.starts_with("feb");
    match ipv6_linklocal {
        true => return true,
        false => {}
    }

    // IPv6 unique-local: fc00::/7 (fc00 through fdff)
    let ipv6_ula = host_bare.starts_with("fc") || host_bare.starts_with("fd");
    // Only treat as ULA if the host looks like an IPv6 address (contains ':').
    match ipv6_ula && host_bare.contains(':') {
        true => return true,
        false => {}
    }

    false
}

/// Scan the InstanceFact cell in parsed state and return the first
/// forbidden URL found, if any. Used by platform_compile to reject
/// External System federation to internal/loopback/link-local hosts.
pub fn find_forbidden_instance_url(state: &crate::ast::Object) -> Option<String> {
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("InstanceFact", state)
        .as_seq()
        .and_then(|facts| {
            facts.iter().find_map(|f| {
                let object_value = binding(f, "objectValue")?;
                is_forbidden_url(object_value).then(|| object_value.to_string())
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
    use crate::ast::{fetch_or_phi, binding};
    fetch_or_phi("Noun", state)
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
    use crate::ast::{fetch_or_phi, binding};
    // Pre-collect Role cell facts so each FactType iteration is O(|R|)
    // rather than re-fetching per FT.
    let role_cell = fetch_or_phi("Role", state);
    let role_facts: Vec<&crate::ast::Object> = role_cell.as_seq()
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    fetch_or_phi("FactType", state)
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
#[cfg(feature = "std-deps")]
pub fn parse_to_state_from(input: &str, d: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context(input, d)
}

/// Alias for `parse_to_state_from` kept for API compatibility. Legacy
/// took only nouns; stage12's context path accepts the full state and
/// extracts what it needs.
#[cfg(feature = "std-deps")]
pub fn parse_to_state_with_nouns(input: &str, existing: &crate::ast::Object) -> Result<crate::ast::Object, String> {
    crate::parse_forml2_stage2::parse_to_state_via_stage12_with_context(input, existing)
}




/// Re-resolve a rules vec given just the typed lookups it needs.
/// No ParseCtx struct required â€” callers pass their HashMaps directly.
pub(crate) fn re_resolve_rules(
    rules: &mut Vec<DerivationRuleDef>,
    nouns: &HashMap<String, NounDef>,
    fact_types: &HashMap<String, FactTypeDef>,
) {
    let mut noun_names: Vec<String> = nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut catalog = SchemaCatalog::new();
    fact_types.iter().for_each(|(ft_id, ft)| {
        let role_nouns: Vec<&str> = ft.roles.iter().map(|r| r.noun_name.as_str()).collect();
        // Verb extraction: text after the first noun up to the second
        // (binary+), or everything after the single noun (unary — #274
        // Category A). Without the unary branch the catalog would
        // register `Customer is in EEA` with an empty verb, which
        // collides with every other unary keyed on [customer].
        let verb = noun_names.iter()
            .find(|n| ft.reading.starts_with(n.as_str()))
            .map(|first| {
                let after = &ft.reading[first.len()..];
                noun_names.iter()
                    .find_map(|second| after.find(second.as_str()).map(|pos| after[..pos].trim()))
                    .unwrap_or_else(|| after.trim())
            })
            .unwrap_or("");
        catalog.register(ft_id, &role_nouns, verb, &ft.reading);
    });

    rules.iter_mut().for_each(|rule| {
        resolve_derivation_rule(rule, nouns, fact_types, &catalog);
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
            || (stripped.contains(" if ") && !stripped.starts_with("If "))
            || stripped.contains(" := ");
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
    let target_resolves = noun_names.iter().any(|n| n == &target)
        || target.split_whitespace().next()
            .map_or(false, |first| noun_names.iter().any(|n| n == first));
    if !target_resolves { return None; }
    Some((role, op, target, where_clause))
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
    // Quick exit â€” no apostrophe means nothing to expand.
    if !text.contains("'s ") {
        return None;
    }

    // Walk the text looking for `<Noun>'s <Noun2>` sequences.
    // We use a simple left-to-right scan: find the first `'s `, identify the
    // noun that ends just before the apostrophe, identify the noun that begins
    // just after the space, then emit the expanded two-clause form.
    let mut result = text.to_string();
    let mut changed = false;

    // Iterate until no more `'s ` tokens remain (handles chained possessives).
    loop {
        let Some(apos_pos) = result.find("'s ") else { break };

        // Find noun1: the longest known noun ending at apos_pos.
        let prefix = &result[..apos_pos];
        let noun1 = noun_names.iter()
            .filter(|n| prefix.ends_with(n.as_str()))
            .max_by_key(|n| n.len())
            .cloned();

        // Find noun2: the longest known noun starting at apos_pos + 3.
        let after = &result[apos_pos + 3..]; // skip `'s `
        let noun2 = noun_names.iter()
            .filter(|n| after.starts_with(n.as_str()))
            .max_by_key(|n| n.len())
            .cloned();

        match (noun1, noun2) {
            (Some(n1), Some(n2)) => {
                // Build the expanded form:
                //   "<prefix-without-n1><n1> has <n2> and that <n2><suffix-without-n2>"
                let n1_start = apos_pos - n1.len();
                let n2_end = apos_pos + 3 + n2.len();
                let before_n1 = &result[..n1_start];
                let after_n2 = &result[n2_end..];
                result = format!(
                    "{}{} has {} and that {}{}",
                    before_n1, n1, n2, n2, after_n2
                );
                changed = true;
            }
            _ => {
                // Unknown noun around the apostrophe â€” leave as-is to avoid
                // corrupting input the parser can't understand.
                break;
            }
        }
    }

    changed.then_some(result)
}

/// Resolve a derivation rule's text into structured fact type references.
///
/// Splits on " if "/" iff " to get consequent and antecedent parts,
/// then matches each part's nouns against ir.fact_types by role noun names.
/// Anaphoric "that X" references are stripped to bare noun name "X".
///
/// Per-antecedent inline numeric comparisons (Halpin FORML Example 5) are
/// extracted via `split_antecedent_comparator` BEFORE fact-type resolution,
/// so `has Population >= 1000000` resolves to the base FT `has Population`
/// with an AntecedentFilter attached restricting that antecedent's population.
/// Temporal predicates are runtime clock checks with no declared FT.
fn is_temporal_predicate(clause: &str) -> bool {
    let l = clause.to_lowercase();
    l.contains("now is ") || l.contains(" in the past") || l.contains(" in the future")
        || l.contains("is current") || l.contains("is expired")
        || l.contains("is fresh") || l.contains("is stale")
}

fn resolve_derivation_rule(
    rule: &mut DerivationRuleDef,
    nouns_map: &HashMap<String, NounDef>,
    fact_types_map: &HashMap<String, FactTypeDef>,
    catalog: &SchemaCatalog,
) {
    // Shim: old code paths referred to `ir.nouns` / `ir.fact_types`.
    // Rebind so the body below compiles unchanged.
    struct IrShim<'a> {
        nouns: &'a HashMap<String, NounDef>,
        fact_types: &'a HashMap<String, FactTypeDef>,
    }
    let ir = IrShim { nouns: nouns_map, fact_types: fact_types_map };
    // Longest-first noun list for Theorem 1 matching
    let mut noun_names: Vec<String> = ir.nouns.keys().cloned().collect();
    noun_names.sort_by(|a, b| b.len().cmp(&a.len()));

    // Pre-process: expand possessive syntax (`X's Y`) into explicit join form
    // (`X has Y and that Y`) so the anaphora detector below can classify the
    // rule as a Join derivation.  Only the antecedent portion is rewritten;
    // the consequent is left unchanged.
    if rule.text.contains("'s ") {
        // Split off everything up to and including the iff/if/`:=` keyword,
        // expand only the antecedent portion, then reassemble.
        let sep_offset = rule.text.find(" := ")
            .map(|i| (i, i + 4))
            .or_else(|| rule.text.find(" iff ").map(|i| (i, i + 5)))
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

    // Split on " := ", " iff ", or " if " to get (consequent, antecedent_text)
    let (consequent_text, antecedent_raw) = rule.text
        .find(" := ")
        .map(|i| (&rule.text[..i], &rule.text[i + 4..]))
        .or_else(|| rule.text.find(" iff ")
            .map(|i| (&rule.text[..i], &rule.text[i + 5..])))
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
    let antecedent_parts: Vec<&str> = split_top_level_and(antecedent_text)
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
    let resolve_fact_type = |fragment: &str| -> Option<String> {
        let cleaned = strip_anaphora(fragment);
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
        catalog.resolve(&role_refs, verb_opt)
            .or_else(|| catalog.resolve(&role_refs, None))
    };

    // Detect "that X" anaphoric references -- nouns preceded by "that " in
    // antecedent parts become join keys.
    let join_keys: Vec<String> = antecedent_parts.iter()
        .flat_map(|part| {
            noun_names.iter().filter_map(|noun| {
                let pattern = format!("that {}", noun);
                part.contains(&pattern).then(|| noun.clone())
            }).collect::<Vec<_>>()
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
    // Hand-rolled equivalent of regex ` '([^']*)'\s*$`: capture the
    // single-quoted literal at end of string, after a leading space.
    let consequent_trailing_literal =
        strip_trailing_quoted_literal(consequent_text).map(|(_, lit)| lit);
    let resolved_consequent = resolve_fact_type(consequent_text).unwrap_or_default();
    rule.consequent_cell = crate::types::ConsequentCellSource::Literal(resolved_consequent);
    if let Some(lit) = consequent_trailing_literal {
        if !rule.consequent_cell.is_empty_literal() {
            let role = ir.fact_types.get(rule.consequent_cell.literal_id())
                .and_then(|ft| ft.roles.last())
                .map(|r| r.noun_name.clone())
                .unwrap_or_default();
            if !role.is_empty() {
                rule.consequent_role_literals.push(
                    crate::types::ConsequentRoleLiteral { role, value: lit });
            }
        }
    }

    // Resolve antecedents, carrying inline-comparator filters AND
    // arithmetic-definitional clauses alongside. A definitional clause
    // like `Volume is Size * Size * Size` does not resolve to a fact
    // type â€” it populates consequent_computed_bindings instead. Filter
    // clauses like `has Population >= 1000000` resolve to the base FT
    // with an AntecedentFilter pinned to that antecedent's position.
    let mut resolved_ids: Vec<String> = Vec::new();
    let mut filters: Vec<crate::types::AntecedentFilter> = Vec::new();
    let mut role_literals: Vec<crate::types::AntecedentRoleLiteral> = Vec::new();
    let mut computed: Vec<crate::types::ConsequentComputedBinding> = Vec::new();
    let mut aggregates: Vec<crate::types::ConsequentAggregate> = Vec::new();
    for part in antecedent_parts.iter() {
        // Aggregate clauses (Halpin `<role> is the <op> of <target> where â€¦`).
        // They resolve the where-clause to a source FT and record the
        // group-key role â€” the non-target role on that FT. Match ahead of
        // the generic definitional path so `â€¦ is the count of â€¦` isn't
        // mistaken for arithmetic.
        if let Some((role, op, target, where_clause)) =
            try_parse_aggregate_clause(part, &noun_names)
        {
            // Resolve where-clause to an FT id via the catalog.
            let (stripped, _) = split_antecedent_comparator(&where_clause);
            if let Some(ft_id) = resolve_fact_type(&stripped) {
                // Group-key role = any role on source FT other than target.
                let group_key_role = ir.fact_types.get(&ft_id)
                    .and_then(|ft| ft.roles.iter().find(|r| r.noun_name != target))
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                aggregates.push(crate::types::ConsequentAggregate {
                    role,
                    op,
                    target_role: target,
                    source_fact_type_id: ft_id,
                    group_key_role,
                });
            }
            continue;
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

        // (1) Comparator-stripped FT lookup (direct + hyphen fallback + negation fallback)
        let (stripped, comparator) = split_antecedent_comparator(part);
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
        let ft_resolved = resolve_fact_type(&stripped)
            .or_else(|| (dehyphenated != stripped).then(|| resolve_fact_type(&dehyphenated)).flatten())
            .or_else(|| (destripped_literal != stripped)
                .then(|| resolve_fact_type(&destripped_literal)).flatten())
            .or_else(|| {
                let pos = strip_anaphora(part)
                    .replace(" is not ", " is ")
                    .replace(" has no ", " has ")
                    .replace(" does not ", " ");
                let pos = pos.trim_start_matches("no ").trim_start_matches("not ");
                // Strip " where ..." suffix â€” negated clauses with
                // where-filters ("no X is defined in Y where Z")
                // need the base FT without the filter tail.
                let pos = pos.split(" where ").next().unwrap_or(pos);
                resolve_fact_type(pos)
            });

        if let Some(ft_id) = ft_resolved {
            if let Some((op, value)) = comparator.clone() {
                let role = ir.fact_types.get(&ft_id)
                    .and_then(|ft| ft.roles.last())
                    .map(|r| r.noun_name.clone())
                    .unwrap_or_default();
                filters.push(crate::types::AntecedentFilter {
                    antecedent_index: resolved_ids.len(),
                    role, op, value,
                });
            }
            if let Some(lit) = trailing_literal.clone() {
                let role = ir.fact_types.get(&ft_id)
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
            resolved_ids.push(ft_id);
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
        if part.trim().starts_with("that ") && noun_names.iter()
            .any(|n| part.to_lowercase().contains(&n.to_lowercase()))
        { continue; }
        if part.contains(" is that ") || part.contains(" is some ") { continue; }

        // (6) Temporal predicates â€” genuinely new, no existing fn.
        if is_temporal_predicate(part) { continue; }

        // (7) Subtype instance check: `X is a Y` / `X is an Y` where
        //     both X and Y are declared nouns. Subtype membership is
        //     inherent to the schema (Noun-is-subtype-of-Noun facts),
        //     not a separate FT. Recognised so readings like
        //       TCPA Violation is for Robocall ... if Robocall is
        //         an Autodialed Call and ...
        //     don't spuriously flag the subtype check as unresolved.
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

        // (10) Universal quantifier: `for each <Noun> <predicate>`.
        //      Recognised when the clause starts with `for each` and
        //      contains a declared noun. The compiled form is a
        //      population-level restriction; classification here just
        //      suppresses the noise so legitimate universals don't
        //      flag as unresolved.
        if is_universal_quantifier_clause(part, &noun_names) { continue; }

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

        // Nothing classified this clause.
        rule.unresolved_clauses.push(part.to_string());
    }
    rule.antecedent_sources = resolved_ids.into_iter()
        .map(crate::types::AntecedentSource::FactType).collect();
    rule.antecedent_filters = filters;
    rule.antecedent_role_literals = role_literals;
    rule.consequent_computed_bindings = computed;
    rule.consequent_aggregates = aggregates;

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
                    if ft_id.is_empty() { None } else { ir.fact_types.get(ft_id) }
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
        rule.consequent_bindings = ir.fact_types.get(rule.consequent_cell.literal_id())
            .map(|ft| ft.roles.iter().map(|r| r.noun_name.clone()).collect())
            .unwrap_or_default();
    });

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





















/// #283 — Every fact-type id in the `FactType` cell. Replaces
/// `ir.fact_types.keys()`. Test-only helper.
#[cfg(test)]
fn fact_type_ids(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<String> {
    cells.get("FactType")
        .map(|facts| facts.iter()
            .filter_map(|f| crate::ast::binding(f, "id").map(String::from))
            .collect())
        .unwrap_or_default()
}


/// #283 — Reading text for a fact-type via the `FactType` cell.
/// Test-only helper.
#[cfg(test)]
fn fact_type_reading(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
    id: &str,
) -> Option<String> {
    cells.get("FactType")?
        .iter()
        .find(|f| crate::ast::binding(f, "id") == Some(id))
        .and_then(|f| crate::ast::binding(f, "reading").map(String::from))
}





/// #283 — Rebuild `Vec<DerivationRuleDef>` from the `DerivationRule` cell.
/// The `json` binding on each fact is the lossless encoding.
/// Test-only helper (legacy JSON-blob path).
#[cfg(all(test, feature = "std-deps"))]
fn derivation_rules_from_cells(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<DerivationRuleDef> {
    let Some(facts) = cells.get("DerivationRule") else { return Vec::new() };
    facts.iter().filter_map(|f| {
        let json = crate::ast::binding(f, "json")?;
        serde_json::from_str::<DerivationRuleDef>(json).ok()
    }).collect()
}

/// #283 — Rebuild `Vec<ConstraintDef>` from the `Constraint` cell.
/// The cell is lossless — `constraint_to_fact` embeds the full JSON
/// encoding of each ConstraintDef under the `json` binding.
/// Test-only helper.
#[cfg(all(test, feature = "std-deps"))]
fn constraints_from_cells(
    cells: &HashMap<String, Vec<crate::ast::Object>>,
) -> Vec<ConstraintDef> {
    let Some(facts) = cells.get("Constraint") else { return Vec::new() };
    facts.iter().filter_map(|f| {
        let json = crate::ast::binding(f, "json")?;
        serde_json::from_str::<ConstraintDef>(json).ok()
    }).collect()
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
        let mut key: Vec<String> = role_nouns.iter().map(|n| {
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
}


/// Parse a role token into (base_noun_name, full_token_with_subscript).
/// "Person1" -> ("Person", "Person1"). "User" -> ("User", "User").
fn parse_role_token(token: &str) -> (&str, &str) {
    let boundary = token
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    (&token[..boundary], token)
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
            while let Some(found) = text_lower[pos..].find(name_lower.as_str()) {
                let start = pos + found;
                let mut end = start + name_lower.len();
                let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
                // Extend end past any trailing ASCII digit subscript.
                while end < text.len() && text.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                // After the (possibly-extended) end, the next byte must
                // not be alphanumeric â€” otherwise the match was part of
                // a longer identifier.
                let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();
                let no_overlap = !used.iter().any(|&(s, e)| start < e && end > s);

                if before_ok && after_ok && no_overlap {
                    // Capture the subscripted token (e.g. "Person3") so
                    // callers distinguish the ring positions. The base
                    // name is recovered via parse_role_token at the
                    // resolve site.
                    let captured = &text[start..end];
                    matches.push((start, end, captured.to_string()));
                    used.push((start, end));
                }
                pos = start + 1;
                if pos >= text.len() { break; }
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

/// Hand-rolled equivalent of trailing-comparator regex
///   `\s*(>=|<=|!=|<>|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$`
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
        // 7. Operator alternation, longest first (so `>=` beats `>`).
        const OPS: &[&str] = &[">=", "<=", "!=", "<>", ">", "<", "="];
        let Some(op) = OPS.iter().find(|op| {
            op.len() <= op_end && &s[op_end - op.len()..op_end] == **op
        }) else {
            continue;
        };
        let op_start = op_end - op.len();
        // 8. `stripped` mirrors `text[..whole.start()].trim_end()`.
        //    `whole.start()` is the start of the leading `\s*` run
        //    before the operator; trim_end on the slice up to the
        //    operator gives the same result.
        let stripped = text[..op_start].trim_end().to_string();
        return Some((stripped, op, value));
    }
    None
}

// =========================================================================
// Instance fact parsing (state machines)
// =========================================================================

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
}








