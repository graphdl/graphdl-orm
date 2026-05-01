//! Stage-1 tokenizer: text → `Statement` cells.
//!
//! #285 two-stage bootstrap. Stage-1 is a minimal Rust parser that
//! tokenizes each FORML 2 statement into the `Statement` / `Role
//! Reference` cell shape that `readings/forml2-grammar.md` (#284)
//! expects. Stage-2 applies the grammar's derivation rules to populate
//! the downstream metamodel cells.
//!
//! Stage-1 does NO classification — it only extracts:
//!
//! - `Statement has Text` — the canonical trimmed, period-stripped
//!   statement text.
//! - `Statement has Head Noun` — the first declared noun mentioned in
//!   the text (longest-first match per Theorem 1).
//! - `Statement has Verb` — the text between the head noun and the
//!   second role's noun (or the tail for unaries).
//! - `Statement has Trailing Marker` — the suffix phrase starting
//!   with a known marker keyword (`is an entity type`, `is abstract`,
//!   `is acyclic`, etc.) when present.
//! - `Statement has Derivation Marker` — ORM 2 `*` / `**` / `+`.
//! - `Statement has Quantifier` — leading quantifier (`each`, `at most
//!   one`, ...) when present.
//! - `Statement has Role Reference` — one `Role Reference` per matched
//!   noun, each with its `Role Position`, `Head Noun`, and optional
//!   `Literal Value`.

extern crate alloc;
use alloc::{string::{String, ToString}, vec::Vec, format, borrow::Cow};
use hashbrown::HashMap;
use crate::ast::{Object, fact_from_pairs};

type Cells = HashMap<String, Vec<Object>>;

/// Tokenize a single FORML 2 statement into the Stage-1 cell shape.
///
/// - `statement_id`: unique id for this Statement (caller-assigned).
/// - `text`: the raw statement text (trailing period allowed; this
///   function trims it).
/// - `nouns`: declared noun names. Matching uses longest-first.
///
/// Returns a `Cells` map the caller can merge into a larger state.
pub fn tokenize_statement(statement_id: &str, text: &str, nouns: &[String]) -> Cells {
    let mut sorted_nouns: Vec<&str> = nouns.iter().map(|s| s.as_str()).collect();
    sorted_nouns.sort_by(|a, b| b.len().cmp(&a.len()));
    tokenize_statement_presorted(statement_id, text, &sorted_nouns)
}

/// Pre-built first-byte index over a length-desc noun list. Stage-2
/// builds one of these per parse and reuses it across all ~500 calls
/// to `tokenize_statement_with_buckets`, avoiding the per-call bucket
/// rebuild.
pub struct NounBuckets<'a> {
    by_first_byte: [Option<Vec<&'a str>>; 128],
}

impl<'a> NounBuckets<'a> {
    /// `sorted_nouns` must be length-desc. Bucketing by first byte
    /// preserves that order inside each bucket, which keeps
    /// longest-first match semantics (`Support Request` before
    /// `Request`).
    pub fn from_sorted(sorted_nouns: &[&'a str]) -> Self {
        let mut by_first_byte: [Option<Vec<&'a str>>; 128] = [const { None }; 128];
        for n in sorted_nouns {
            if let Some(&b0) = n.as_bytes().first() {
                if b0 < 128 {
                    by_first_byte[b0 as usize].get_or_insert_with(Vec::new).push(*n);
                }
            }
        }
        NounBuckets { by_first_byte }
    }

    fn bucket(&self, b0: u8) -> Option<&[&'a str]> {
        if b0 >= 128 { return None; }
        self.by_first_byte[b0 as usize].as_deref()
    }
}

/// Hot-path entry point: caller has already sorted the noun slice by
/// length descending. Builds a `NounBuckets` index and delegates; for
/// bulk tokenization prefer `tokenize_statement_with_buckets` so the
/// index is shared across calls.
pub fn tokenize_statement_presorted(statement_id: &str, text: &str, sorted_nouns: &[&str]) -> Cells {
    let buckets = NounBuckets::from_sorted(sorted_nouns);
    tokenize_statement_with_buckets(statement_id, text, &buckets)
}

/// Fastest entry point for bulk tokenization: caller has pre-built
/// the `NounBuckets` index. Stage-2 uses this for the per-line loop.
pub fn tokenize_statement_with_buckets(statement_id: &str, text: &str, buckets: &NounBuckets<'_>) -> Cells {
    let mut cells: Cells = HashMap::new();
    // Canonical text: drop the trailing period AND the leading ORM 2
    // derivation marker (`* ` / `** ` / `+ `). Legacy's `try_derivation`
    // strips the prefix via strip_prefix; downstream DerivationRule.id
    // (FNV of text) needs the same normalization so both pipelines
    // produce the same hash.
    let trimmed = text.trim().trim_end_matches('.').trim();
    let no_prefix = trimmed
        .strip_prefix("** ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .unwrap_or(trimmed);

    // 1. Strip trailing ORM 2 derivation marker. Apply to the
    //    canonical text too so downstream consumers (Text cell,
    //    FactType id construction) don't carry the ` *` suffix.
    let (body, derivation_marker) = strip_derivation_marker(no_prefix);
    let canonical: &str = body.trim_end_matches('.').trim();
    let body: &str = canonical;

    // 1b. Strip ORM 2 reference-scheme parens: `Function(.id) is ...`
    //     becomes `Function is ...`. Legacy does this in
    //     parse_entity_decl; Stage-1 needs the same so the trailing
    //     marker (`is an entity type`) is recognizable as a prefix of
    //     the post-noun tail.
    let body_stripped: Cow<'_, str> = strip_ref_scheme_parens(body);
    let body: &str = body_stripped.as_ref();

    // 2. Strip leading deontic operator (`It is obligatory/forbidden/permitted that`).
    let (body, deontic_operator) = strip_leading_deontic_operator(body);

    // 3. Strip leading quantifier.
    let (body, quantifier) = strip_leading_quantifier(body);

    // 3. Longest-first noun matching over the body. Caller supplies
    //    a pre-built `NounBuckets` so we skip per-call partitioning.
    let role_refs = match_role_references(body, buckets);

    // 4. Head Noun + Verb derivation from role refs.
    let head_noun = role_refs.first().map(|r| r.noun.clone());
    let mut verb = extract_verb(body, &role_refs);

    // 5. Trailing Marker.
    let mut trailing_marker = extract_trailing_marker(body, &role_refs);
    // Synthetic ring marker: `No <Noun> <verb> itself` is ORM 2's
    // irreflexive-constraint shorthand. Legacy's `try_ring` matches
    // it directly; Stage-2 relies on the `is irreflexive` trailing
    // marker to route through the Ring Constraint recognizer.
    // Check `canonical` (not `body`) because the leading quantifier
    // stripper already consumed the `No `.
    if canonical.starts_with("No ") && canonical.ends_with(" itself") {
        trailing_marker = Some("is irreflexive".to_string());
    }

    // 6. Enum Values leading-phrase override.
    //    `The possible values of <Noun> are '<v1>', '<v2>', ...`
    //    does not fit the "verb between roles" shape — the signalling
    //    phrase sits BEFORE the noun. Detect and override Verb so the
    //    grammar's Enum Values Declaration recognizer (keyed on Verb
    //    = 'the possible values of') fires. Captured values land as
    //    `Statement has Enum Value '<v>'` repeated tokens.
    let enum_values = extract_enum_values(body);
    if enum_values.is_some() {
        verb = Some("the possible values of".to_string());
    }

    // --- Statement cell ---
    push(&mut cells, "Statement", fact_from_pairs(&[
        ("id", statement_id), ("name", statement_id),
    ]));
    push(&mut cells, "Statement_has_Text", fact_from_pairs(&[
        ("Statement", statement_id), ("Text", canonical),
    ]));
    if let Some(hn) = head_noun.as_ref() {
        push(&mut cells, "Statement_has_Head_Noun", fact_from_pairs(&[
            ("Statement", statement_id), ("Head_Noun", hn),
        ]));
    }
    if let Some(v) = verb.as_ref() {
        if !v.is_empty() {
            push(&mut cells, "Statement_has_Verb", fact_from_pairs(&[
                ("Statement", statement_id), ("Verb", v),
            ]));
        }
    }
    if let Some(tm) = trailing_marker.as_ref() {
        push(&mut cells, "Statement_has_Trailing_Marker", fact_from_pairs(&[
            ("Statement", statement_id), ("Trailing_Marker", tm),
        ]));
    }
    if let Some(q) = quantifier.as_ref() {
        push(&mut cells, "Statement_has_Quantifier", fact_from_pairs(&[
            ("Statement", statement_id), ("Quantifier", q),
        ]));
    }
    // Inner quantifiers. Constraint statements put the quantifier
    // that determines their kind INSIDE the sentence, not at the
    // start: `Each Order was placed by exactly one Customer.` (UC on
    // 'exactly one'); `Each Order has at most 5 and at least 2 Line
    // Items.` (FC on 'at most' AND 'at least'). Emit one fact per
    // distinct inner quantifier so the grammar's recognizer rules
    // can fire. Longest-first: when 'at most one' is present, drop
    // the bare 'at most' so FC doesn't mis-fire on a UC sentence.
    for q in inner_quantifiers(body) {
        push(&mut cells, "Statement_has_Quantifier", fact_from_pairs(&[
            ("Statement", statement_id), ("Quantifier", q),
        ]));
    }
    if let Some(dm) = derivation_marker.as_ref() {
        push(&mut cells, "Statement_has_Derivation_Marker", fact_from_pairs(&[
            ("Statement", statement_id), ("Derivation_Marker", dm),
        ]));
    }
    if let Some(op) = deontic_operator.as_ref() {
        push(&mut cells, "Statement_has_Deontic_Operator", fact_from_pairs(&[
            ("Statement", statement_id), ("Deontic_Operator", op),
        ]));
    }
    // Existential marker: any role has a literal value. Lets the grammar
    // classify Instance Facts without walking the role list.
    if role_refs.iter().any(|r| r.literal.is_some()) {
        push(&mut cells, "Statement_has_Literal_Role", fact_from_pairs(&[
            ("Statement", statement_id), ("Literal_Role", "true"),
        ]));
    }
    // Keyword markers: scan the body for derivation / conditional
    // keywords so the grammar's `Statement has Keyword 'iff'` recognizers
    // can fire without requiring the keyword to land in the Verb slot
    // (which only captures text between roles 0 and 1).
    //
    // Exclude sentences that carry one of the multi-clause constraint
    // keywords below — `if and only if` would otherwise also emit the
    // shorter `if` keyword and spuriously classify the statement as a
    // Derivation Rule.
    let is_constraint_body = CONSTRAINT_KEYWORDS.iter().any(|k| body.contains(*k));
    if !is_constraint_body {
        // Pre-materialized `" <kw> "` needles — parallel to KEYWORDS.
        // Avoids allocating a fresh String per (line × keyword) pair.
        const KEYWORD_NEEDLES: &[&str] = &[" iff ", " if ", " when "];
        for (kw_idx, kw) in KEYWORDS.iter().enumerate() {
            if body.contains(KEYWORD_NEEDLES[kw_idx]) {
                push(&mut cells, "Statement_has_Keyword", fact_from_pairs(&[
                    ("Statement", statement_id), ("Keyword", kw),
                ]));
            }
        }
        // `If ... then ...` sentences starting with capital `If` are
        // conditional derivation rules (ORM 2). Legacy's `try_subset`
        // fires BEFORE `try_derivation` in Pass 2b priority — but
        // only when the antecedent contains 2+ DISTINCT declared
        // noun types; otherwise `try_subset` returns None and
        // `try_derivation` emits the rule. Stage-1 can't do the
        // noun-count check here (it doesn't have the full noun
        // catalog), so emit Keyword 'if' UNCONDITIONALLY for every
        // `If ... then ...` shape. Stage-2's translators
        // (`translate_set_constraints`, `translate_derivation_rules`)
        // then arbitrate using the declared noun list in the
        // classified state — matching legacy's pass-order precedence.
        if body.starts_with("If ") && body.contains(" then ") {
            push(&mut cells, "Statement_has_Keyword", fact_from_pairs(&[
                ("Statement", statement_id), ("Keyword", "if"),
            ]));
        }
        // `<consequent> := <antecedent>.` is legacy's explicit derivation
        // rule separator (parse_forml2.rs treats " := ", " iff ", " if "
        // as interchangeable derivation markers). The grammar's
        // Derivation Rule recognizers key off Keyword 'iff' / 'if' /
        // 'when', so emit Keyword 'iff' here to route `:=` rules into
        // the same classification path.
        if body.contains(" := ") {
            push(&mut cells, "Statement_has_Keyword", fact_from_pairs(&[
                ("Statement", statement_id), ("Keyword", "iff"),
            ]));
        }
    }
    // Multi-clause constraint keyword markers. Each is a phrase that
    // signals a specific constraint kind:
    //   'if and only if'                         → Equality Constraint
    //   'at most one of the following holds'     → Exclusion Constraint
    //   'exactly one of the following holds'     → Exclusive-Or Constraint
    //   'at least one of the following holds'    → Or Constraint
    for kw in CONSTRAINT_KEYWORDS {
        if body.contains(*kw) {
            push(&mut cells, "Statement_has_Constraint_Keyword", fact_from_pairs(&[
                ("Statement", statement_id), ("Constraint_Keyword", kw),
            ]));
        }
    }
    // Subset Constraint (ORM 2): conditional frame with existential
    // `some` in the antecedent and anaphoric `that` in the consequent.
    //   `If some <X> <verb> some <Y> then that <X> ...`
    // We key the grammar's recognizer on a synthetic `if some then
    // that` token so the rule stays a simple literal-keyword match.
    if body.starts_with("If some ") && body.contains(" then that ") {
        push(&mut cells, "Statement_has_Constraint_Keyword", fact_from_pairs(&[
            ("Statement", statement_id),
            ("Constraint_Keyword", "if some then that"),
        ]));
    }
    // Enum values list — one token fact per value.
    if let Some(values) = enum_values.as_ref() {
        for v in values {
            push(&mut cells, "Statement_has_Enum_Value", fact_from_pairs(&[
                ("Statement", statement_id), ("Enum_Value", v.as_str()),
            ]));
        }
    }
    // --- Role References ---
    for (i, rr) in role_refs.iter().enumerate() {
        let role_id = format!("{statement_id}:role:{i}");
        push(&mut cells, "Role_Reference", fact_from_pairs(&[
            ("id", role_id.as_str()), ("name", role_id.as_str()),
        ]));
        push(&mut cells, "Statement_has_Role_Reference", fact_from_pairs(&[
            ("Statement", statement_id), ("Role_Reference", role_id.as_str()),
        ]));
        push(&mut cells, "Role_Reference_has_Head_Noun", fact_from_pairs(&[
            ("Role_Reference", role_id.as_str()), ("Head_Noun", rr.noun.as_str()),
        ]));
        push(&mut cells, "Role_Reference_has_Role_Position", fact_from_pairs(&[
            ("Role_Reference", role_id.as_str()), ("Role_Position", &i.to_string()),
        ]));
        if let Some(lit) = rr.literal.as_ref() {
            push(&mut cells, "Role_Reference_has_Literal_Value", fact_from_pairs(&[
                ("Role_Reference", role_id.as_str()), ("Literal_Value", lit.as_str()),
            ]));
        }
    }

    cells
}

fn push(cells: &mut Cells, name: &str, fact: Object) {
    cells.entry(name.to_string()).or_default().push(fact);
}

/// Detect whether an unquoted noun name would collide with a grammar
/// reserved keyword (Theorem 1's no-substring hypothesis). Returns
/// `Some(keyword)` naming the offending term when a collision exists.
///
/// Matching is case-insensitive and word-bounded: the keyword must
/// appear as a whole word in the noun name (surrounded by whitespace
/// or the name's start/end). `Each Way Bet` collides on `each`;
/// `At Most One Hop` collides on `at most one`; `Teacher` does NOT
/// collide on `each` because `each` is not a whole word inside it.
///
/// The keyword set is the single-word quantifiers / modalities /
/// connectives plus the multi-clause `CONSTRAINT_KEYWORDS`. Multi-
/// word phrases are tested first so `at most one` beats `at most`
/// when both would apply.
///
/// Documented rule at `docs/02-writing-readings.md` §"Noun names and
/// reserved words". Escape: quote the noun name
/// (`Noun 'Each Way Bet' is an entity type.`).
///
/// no_std-clean (#653): pure `alloc::format!` + `str::contains`, no
/// serde/regex/std reach. Originally cfg-gated to `std-deps` out of
/// caution; gate dropped so stage2 can call it under no_std as well.
pub fn reserved_keyword_in(name: &str) -> Option<&'static str> {
    // Longest first so `at most one` beats `at most` on `At Most One
    // Hop`, etc.
    const RESERVED: &[&str] = &[
        "at most one of the following holds",
        "at least one of the following holds",
        "exactly one of the following holds",
        "if some then that",
        "if and only if",
        "at most one",
        "at least one",
        "exactly one",
        "at most",
        "at least",
        "each", "no", "some", "exactly",
        "iff", "if", "then", "when",
        "obligatory", "forbidden", "permitted",
        "possible", "impossible",
    ];
    let padded: alloc::string::String =
        alloc::format!(" {} ", name.to_lowercase());
    RESERVED.iter()
        .find(|kw| {
            let needle = alloc::format!(" {} ", kw);
            padded.contains(needle.as_str())
        })
        .copied()
}

fn strip_derivation_marker(text: &str) -> (&str, Option<String>) {
    if let Some(before) = text.strip_suffix(" **") {
        return (before, Some("derived-and-stored".to_string()));
    }
    if let Some(before) = text.strip_suffix(" *") {
        return (before, Some("fully-derived".to_string()));
    }
    if let Some(before) = text.strip_suffix(" +") {
        return (before, Some("semi-derived".to_string()));
    }
    (text, None)
}

const KEYWORDS: &[&str] = &["iff", "if", "when"];

/// Multi-clause constraint keywords (Halpin FORML / ORM 2). Each
/// phrase uniquely signals one constraint kind per the grammar's
/// recognizer rules.
const CONSTRAINT_KEYWORDS: &[&str] = &[
    "if and only if",
    "at most one of the following holds",
    "exactly one of the following holds",
    "at least one of the following holds",
];

const QUANTIFIERS: &[&str] = &[
    "at most one ",
    "at least one ",
    "exactly one ",
    "at most ",
    "at least ",
    "Each ",
    "each ",
    "Some ",
    "some ",
    "No ",
    "no ",
];

fn strip_leading_quantifier(text: &str) -> (&str, Option<String>) {
    for q in QUANTIFIERS {
        if let Some(rest) = text.strip_prefix(q) {
            return (rest, Some(q.trim().to_lowercase()));
        }
    }
    (text, None)
}

const DEONTIC_PREFIXES: &[(&str, &str)] = &[
    ("It is obligatory that ", "obligatory"),
    ("It is forbidden that ",  "forbidden"),
    ("It is permitted that ",  "permitted"),
    ("it is obligatory that ", "obligatory"),
    ("it is forbidden that ",  "forbidden"),
    ("it is permitted that ",  "permitted"),
];

fn strip_leading_deontic_operator(text: &str) -> (&str, Option<String>) {
    for (prefix, op) in DEONTIC_PREFIXES {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (rest, Some(op.to_string()));
        }
    }
    (text, None)
}

/// Strip ORM 2 reference-scheme parens from a statement body.
///
/// A reference scheme follows an entity-type name to declare how
/// instances are referenced: `Customer(.Name)`, `Order(.Number,
/// .Year)`, `Bound(.id)`. The legacy parser handles this in
/// `parse_entity_decl`; Stage-1 does the equivalent in text so the
/// trailing marker extractor doesn't see the parens between the
/// head noun and `is an entity type`.
///
/// The pattern we strip: `(.<chars-excluding-close-paren>)`. Only
/// parens that open with `(.` are reference schemes — ordinary
/// parenthetical prose (rare in FORML 2) is left alone.
fn strip_ref_scheme_parens(body: &str) -> Cow<'_, str> {
    // Fast path: no reference-scheme parens. Almost every non-entity-
    // declaration statement hits this branch, so skipping the String
    // allocation here shaves real time off bulk tokenization.
    if !body.contains("(.") {
        return Cow::Borrowed(body);
    }
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(pos) = rest.find("(.") {
        out.push_str(&rest[..pos]);
        match rest[pos..].find(')') {
            Some(close_rel) => {
                rest = &rest[pos + close_rel + 1..];
            }
            None => {
                out.push_str(&rest[pos..]);
                return Cow::Owned(out);
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[derive(Debug, Clone)]
struct RoleRef {
    noun: String,
    literal: Option<String>,
    /// Byte offset where the noun match starts in the body.
    start: usize,
    /// Byte offset where the noun match ends (exclusive). Excludes any
    /// following `'literal'` — that's tracked by `span_end`.
    end: usize,
    /// Effective end including any `' literal '` that followed. Used
    /// by the verb extractor so `Customer 'alice' places Order` yields
    /// verb = "places", not "'alice' places".
    span_end: usize,
}

fn match_role_references(text: &str, buckets: &NounBuckets<'_>) -> Vec<RoleRef> {
    let mut refs: Vec<RoleRef> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Non-ASCII multibyte chars can leave `i` mid-codepoint; the
        // `&text[i..]` slice requires a char boundary. Noun names are
        // ASCII per convention (#189), so it's safe to skip interior
        // bytes until the next boundary.
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        // Require word boundary: previous char is non-alphanumeric (or start).
        let at_boundary = i == 0 || {
            let prev = bytes[i - 1];
            !prev.is_ascii_alphanumeric() && prev != b'_'
        };
        if !at_boundary {
            i += 1;
            continue;
        }
        let bucket = match buckets.bucket(bytes[i]) {
            Some(b) => b,
            None => { i += 1; continue; }
        };
        let rest = &text[i..];
        // Longest-first match within the first-byte bucket.
        if let Some(noun) = bucket.iter().find(|n| {
            rest.starts_with(**n) && {
                let end = i + n.len();
                end == bytes.len() || {
                    let next = bytes[end];
                    !next.is_ascii_alphanumeric() && next != b'_'
                }
            }
        }) {
            let start = i;
            let end = i + noun.len();
            // Skip if this match is entirely inside an earlier match.
            let nested = refs.iter().any(|r| start >= r.start && end <= r.end);
            if !nested {
                let (literal, span_end) = extract_following_literal_span(text, end);
                refs.push(RoleRef {
                    noun: (*noun).to_string(),
                    literal,
                    start,
                    end,
                    span_end,
                });
                i = span_end;
                continue;
            }
        }
        i += 1;
    }
    refs
}

/// Extract a single-quoted literal immediately after a noun match.
/// Halpin's `<Noun> '<value>'` instance-fact / ref-scheme-literal form.
/// Returns `(literal, span_end)` where `span_end` is the byte offset
/// immediately after the closing `'` (or `from` if no literal).
fn extract_following_literal_span(text: &str, from: usize) -> (Option<String>, usize) {
    let rest = &text[from..];
    let after_ws = rest.trim_start();
    let ws_len = rest.len() - after_ws.len();
    if !after_ws.starts_with('\'') {
        return (None, from);
    }
    let body = &after_ws[1..];
    match body.find('\'') {
        Some(end) => {
            let literal = body[..end].to_string();
            // from + ws_len + 1 (opening quote) + end + 1 (closing quote)
            let span_end = from + ws_len + 1 + end + 1;
            (Some(literal), span_end)
        }
        None => (None, from),
    }
}

fn extract_verb(text: &str, refs: &[RoleRef]) -> Option<String> {
    match refs.len() {
        0 => None,
        1 => {
            let tail = &text[refs[0].span_end..];
            Some(tail.trim().to_string())
        }
        _ => {
            let between = &text[refs[0].span_end..refs[1].start];
            Some(between.trim().to_string())
        }
    }
}

/// Trailing markers are suffixes that signal statement kind. The list
/// comes from the Rust classifier's fixed set. Stage-2 derivation
/// rules match against these marker atoms, not against raw prose.
const TRAILING_MARKERS: &[&str] = &[
    "is an entity type",
    "is a value type",
    "is abstract",
    "is acyclic",
    "is asymmetric",
    "is antisymmetric",
    "is intransitive",
    "is irreflexive",
    "is reflexive",
    "is symmetric",
    "is transitive",
    "are mutually exclusive",
    "is partitioned into",
    "is a subtype of",
];

/// Scan the body for inner quantifier phrases and return those
/// present, in longest-first order. Longest-match semantics prevent
/// UC-on-'at most one' sentences from also firing FC (which requires
/// both 'at most' and 'at least' without the 'one' suffix).
fn inner_quantifiers(body: &str) -> Vec<&'static str> {
    const LONG: &[&str] = &["at most one", "at least one", "exactly one"];
    // Parallel arrays — SHORT[i]'s "one" extension is SHORT_ONE[i].
    // Pre-materialized so the filter below doesn't `format!` per call.
    const SHORT: &[&str] = &["at most", "at least"];
    const SHORT_ONE: &[&str] = &["at most one", "at least one"];
    // Multi-clause superset suppression: when the body uses
    // `<quant> of the following holds` (a Constraint Keyword that the
    // grammar rule classifies as XC/XO/OR), the bare LONG quantifier
    // hit is a false positive — the same words are part of the
    // multi-clause keyword, not an inner UC signal. Without this
    // suppression both classifiers fire and the cardinality
    // translator (UC family) wins over the set translator (XO/XC/OR
    // family), so `For each Task, exactly one of the following holds:
    // …` lands as kind=UC instead of kind=XO.
    let long_hits: Vec<&'static str> = LONG.iter()
        .filter(|q| body.contains(**q))
        .filter(|q| {
            // Suppress when the keyword superset is present.
            let superset = alloc::format!("{} of the following holds", q);
            !body.contains(&superset)
        })
        .copied()
        .collect();
    // A bare 'at most' / 'at least' is emitted only when its
    // "<phrase> one" extension isn't already present in the body —
    // that way `at most 5 and at least 2` yields both shorts but
    // `at most one Customer` yields only the long form.
    let short_hits: Vec<&'static str> = SHORT.iter()
        .enumerate()
        .filter(|(i, q)| body.contains(**q) && !body.contains(SHORT_ONE[*i]))
        .map(|(_, q)| *q)
        .collect();
    // ORM 2 plural `some` is the MC signal — `Each X has some Y` is
    // the "at least one" shorthand. Emit as its own quantifier atom
    // so the grammar's MC rule matches on either spelling. Require a
    // space on each side to avoid matching inside words like `someone`
    // or at the start when `some` is the leading quantifier (stripped
    // upstream by `strip_leading_quantifier`).
    let some_hit: Option<&'static str> = body.contains(" some ").then_some("some");
    long_hits.into_iter().chain(short_hits).chain(some_hit).collect()
}

/// Parse the enum-values leading phrase.
///
/// `The possible values of <Noun> are '<v1>', '<v2>', ...`
/// returns `Some(["v1", "v2", ...])`. `None` if the text does not
/// open with the phrase or if no `are` clause follows. Recognised
/// only when the sentence opens with the literal prefix so ordinary
/// fact type readings can't be mistaken for enum declarations.
fn extract_enum_values(body: &str) -> Option<Vec<String>> {
    const PREFIX: &str = "The possible values of ";
    let rest = body.strip_prefix(PREFIX)?;
    let (_noun_part, tail) = rest.split_once(" are ")?;
    let values: Vec<String> = tail.split(',')
        .filter_map(|chunk| chunk.trim()
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .map(String::from))
        .collect();
    (!values.is_empty()).then_some(values)
}

fn extract_trailing_marker(text: &str, refs: &[RoleRef]) -> Option<String> {
    // Look only past the last noun match (including any trailing
    // literal). That keeps "is a value type" from matching inside
    // "X is a value type" where X is a noun.
    let start = refs.last().map(|r| r.span_end).unwrap_or(0);
    let tail = text[start..].trim();
    TRAILING_MARKERS.iter()
        .find(|m| tail == **m || tail.starts_with(*m))
        .map(|m| (*m).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::binding;

    fn nouns(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn stmt_binding(cells: &Cells, cell: &str, field: &str) -> Option<String> {
        cells.get(cell)?.first().and_then(|f| binding(f, field).map(String::from))
    }

    #[test]
    fn entity_type_declaration_extracts_head_noun_and_trailing_marker() {
        let c = tokenize_statement("s1", "Customer is an entity type.", &nouns(&["Customer"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Customer"));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is an entity type"));
        assert_eq!(stmt_binding(&c, "Statement_has_Text", "Text").as_deref(),
                   Some("Customer is an entity type"));
    }

    #[test]
    fn value_type_declaration_extracts_trailing_marker() {
        let c = tokenize_statement("s1", "Priority is a value type.", &nouns(&["Priority"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is a value type"));
    }

    #[test]
    fn abstract_declaration_extracts_trailing_marker() {
        let c = tokenize_statement("s1", "Request is abstract.", &nouns(&["Request"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is abstract"));
    }

    #[test]
    fn binary_fact_type_extracts_head_verb_and_two_role_refs() {
        let c = tokenize_statement("s1", "Customer places Order.",
                                   &nouns(&["Customer", "Order"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Customer"));
        assert_eq!(stmt_binding(&c, "Statement_has_Verb", "Verb").as_deref(),
                   Some("places"));
        assert_eq!(c.get("Role_Reference").map(|v| v.len()), Some(2));
    }

    // ─── #309 reserved-keyword rejection ──────────────────────────────

    #[test]
    fn reserved_keyword_in_catches_each_way_bet() {
        assert_eq!(super::reserved_keyword_in("Each Way Bet"), Some("each"));
    }

    #[test]
    fn reserved_keyword_in_catches_no_show_fee() {
        assert_eq!(super::reserved_keyword_in("No Show Fee"), Some("no"));
    }

    #[test]
    fn reserved_keyword_in_catches_at_most_one_hop_longest_first() {
        // `at most one` wins over `at most` per longest-first ordering.
        assert_eq!(super::reserved_keyword_in("At Most One Hop"),
                   Some("at most one"));
    }

    #[test]
    fn reserved_keyword_in_catches_multi_clause_phrase() {
        assert_eq!(super::reserved_keyword_in("If And Only If Then Noun"),
                   Some("if and only if"));
    }

    #[test]
    fn reserved_keyword_in_accepts_normal_names() {
        assert_eq!(super::reserved_keyword_in("Order"), None);
        assert_eq!(super::reserved_keyword_in("Customer"), None);
        assert_eq!(super::reserved_keyword_in("Line Item"), None);
        assert_eq!(super::reserved_keyword_in("Fact Type"), None);
    }

    #[test]
    fn reserved_keyword_in_does_not_match_substring_inside_word() {
        // Word-bounded: `each` must be a whole word, not a letter
        // sequence inside one.
        assert_eq!(super::reserved_keyword_in("Teacher"), None);
        assert_eq!(super::reserved_keyword_in("Beach"), None);
        assert_eq!(super::reserved_keyword_in("Reacher"), None);
    }

    #[test]
    fn reserved_keyword_in_is_case_insensitive() {
        assert_eq!(super::reserved_keyword_in("EACH way bet"), Some("each"));
        assert_eq!(super::reserved_keyword_in("no Show Fee"), Some("no"));
    }

    #[test]
    fn entity_type_with_ref_scheme_parens_extracts_trailing_marker() {
        // `Function(.id) is an entity type.` — the `(.id)` suffix is an
        // ORM 2 reference scheme; legacy parser strips it via
        // parse_entity_decl. Stage-1 must do the same so the trailing
        // marker `is an entity type` survives.
        let c = tokenize_statement("s1",
            "Function(.id) is an entity type.",
            &nouns(&["Function"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Function"));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is an entity type"));
    }

    #[test]
    fn tokenize_survives_utf8_multibyte_text() {
        // Regression: `§` is 2 bytes; the byte-index loop used to slice
        // `&text[i..]` at a non-char-boundary. #294 diagnostic caught it.
        let c = tokenize_statement("s1",
            "Paper §4 Table 1 correspondence: see Function.",
            &nouns(&["Function"]));
        assert!(c.contains_key("Statement"));
    }

    #[test]
    fn instance_fact_captures_literal_on_role_reference() {
        let c = tokenize_statement("s1", "Customer 'alice' places Order 'o-7'.",
                                   &nouns(&["Customer", "Order"]));
        let rr_lit = c.get("Role_Reference_has_Literal_Value")
            .expect("literals cell must exist");
        assert_eq!(rr_lit.len(), 2);
        let literals: Vec<_> = rr_lit.iter()
            .filter_map(|f| binding(f, "Literal_Value"))
            .collect();
        assert!(literals.contains(&"alice"), "got literals: {:?}", literals);
        assert!(literals.contains(&"o-7"), "got literals: {:?}", literals);
    }

    #[test]
    fn derivation_marker_fully_derived_star() {
        let c = tokenize_statement("s1", "Customer has Full Name *.",
                                   &nouns(&["Customer", "Full Name"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("fully-derived"));
    }

    #[test]
    fn derivation_marker_derived_and_stored_double_star() {
        let c = tokenize_statement("s1", "Order has Total **.",
                                   &nouns(&["Order", "Total"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("derived-and-stored"));
    }

    #[test]
    fn derivation_marker_semi_derived_plus() {
        let c = tokenize_statement("s1", "Person is Grandparent +.",
                                   &nouns(&["Person", "Grandparent"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Derivation_Marker", "Derivation_Marker").as_deref(),
                   Some("semi-derived"));
    }

    #[test]
    fn quantifier_each_stripped_and_captured() {
        let c = tokenize_statement("s1", "Each Customer places at most one Order.",
                                   &nouns(&["Customer", "Order"]));
        // Outer quantifier is `each`; the inner `at most one` precedes Order
        // and is currently captured by the extract_verb step between refs.
        // First-pass Stage-1 keeps the outer quantifier only.
        assert_eq!(stmt_binding(&c, "Statement_has_Quantifier", "Quantifier").as_deref(),
                   Some("each"));
    }

    #[test]
    fn ring_acyclic_marker() {
        let c = tokenize_statement("s1", "Category has parent Category is acyclic.",
                                   &nouns(&["Category"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Trailing_Marker", "Trailing_Marker").as_deref(),
                   Some("is acyclic"));
    }

    #[test]
    fn longest_first_noun_match_wins() {
        let c = tokenize_statement("s1", "Support Request has Priority.",
                                   &nouns(&["Request", "Support Request", "Priority"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Support Request"));
    }

    #[test]
    fn subtype_declaration_verb_captured() {
        let c = tokenize_statement("s1", "Support Request is a subtype of Request.",
                                   &nouns(&["Support Request", "Request"]));
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Support Request"));
        assert_eq!(stmt_binding(&c, "Statement_has_Verb", "Verb").as_deref(),
                   Some("is a subtype of"));
    }

    #[test]
    fn equality_constraint_emits_if_and_only_if_keyword() {
        // EQ shape: `<clause A> if and only if <clause B>`.
        // Stage-1 captures the keyword phrase as its own token so
        // the grammar's Equality Constraint recognizer can fire
        // without colliding with the Derivation Rule's ` if `
        // match.
        let c = tokenize_statement(
            "s1",
            "Each Employee is paid if and only if Employee has Salary.",
            &nouns(&["Employee", "Salary"]),
        );
        let ks: Vec<String> = c.get("Statement_has_Constraint_Keyword")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Constraint_Keyword").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(ks.iter().any(|k| k == "if and only if"),
            "expected 'if and only if' constraint-keyword token; got {:?}", ks);
    }

    #[test]
    fn exclusion_constraint_emits_at_most_one_of_the_following_holds() {
        let c = tokenize_statement(
            "s1",
            "For each Account at most one of the following holds: Account is open; Account is closed.",
            &nouns(&["Account"]),
        );
        let ks: Vec<String> = c.get("Statement_has_Constraint_Keyword")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Constraint_Keyword").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(ks.iter().any(|k| k == "at most one of the following holds"),
            "expected 'at most one of the following holds'; got {:?}", ks);
    }

    #[test]
    fn exclusive_or_constraint_emits_exactly_one_keyword() {
        let c = tokenize_statement(
            "s1",
            "For each Order exactly one of the following holds: Order is draft; Order is placed.",
            &nouns(&["Order"]),
        );
        let ks: Vec<String> = c.get("Statement_has_Constraint_Keyword")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Constraint_Keyword").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(ks.iter().any(|k| k == "exactly one of the following holds"),
            "expected 'exactly one of the following holds'; got {:?}", ks);
    }

    #[test]
    fn subset_constraint_emits_if_some_then_that_keyword() {
        // Subset Constraint shape (ORM 2): `If some <X> <verb> some
        // <Y> then that <X> ...` — existential `some` in the
        // antecedent + anaphoric `that` in the consequent.
        let c = tokenize_statement(
            "s1",
            "If some User owns some Organization then that User has some Email.",
            &nouns(&["User", "Organization", "Email"]),
        );
        let ks: Vec<String> = c.get("Statement_has_Constraint_Keyword")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Constraint_Keyword").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(ks.iter().any(|k| k == "if some then that"),
            "expected 'if some then that' constraint-keyword; got {:?}", ks);
    }

    #[test]
    fn or_constraint_emits_at_least_one_keyword() {
        let c = tokenize_statement(
            "s1",
            "For each User at least one of the following holds: User has Email; User has Phone.",
            &nouns(&["User", "Email", "Phone"]),
        );
        let ks: Vec<String> = c.get("Statement_has_Constraint_Keyword")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Constraint_Keyword").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(ks.iter().any(|k| k == "at least one of the following holds"),
            "expected 'at least one of the following holds'; got {:?}", ks);
    }

    #[test]
    fn uniqueness_constraint_emits_exactly_one_quantifier() {
        // `Each Order was placed by exactly one Customer.` is a UC:
        // the grammar rule fires on Statement has Quantifier 'exactly
        // one'. The leading 'each' is still emitted, and the inner
        // 'exactly one' lands as its own Quantifier fact so both
        // grammar-rule variants can fire.
        let c = tokenize_statement(
            "s1",
            "Each Order was placed by exactly one Customer.",
            &nouns(&["Order", "Customer"]),
        );
        let qs: Vec<String> = c.get("Statement_has_Quantifier")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Quantifier").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(qs.iter().any(|q| q == "exactly one"),
            "expected 'exactly one' quantifier token; got {:?}", qs);
    }

    #[test]
    fn mandatory_role_constraint_emits_at_least_one_quantifier() {
        let c = tokenize_statement(
            "s1",
            "Each Customer has at least one Email.",
            &nouns(&["Customer", "Email"]),
        );
        let qs: Vec<String> = c.get("Statement_has_Quantifier")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Quantifier").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(qs.iter().any(|q| q == "at least one"),
            "expected 'at least one' quantifier token; got {:?}", qs);
    }

    #[test]
    fn frequency_constraint_emits_at_most_and_at_least_quantifiers() {
        // FC rule requires BOTH 'at most' and 'at least' (without
        // the "one" suffix). Stage-1 must emit both tokens so the
        // grammar's `and`-conjunction rule fires.
        let c = tokenize_statement(
            "s1",
            "Each Order has at most 5 and at least 2 Line Items.",
            &nouns(&["Order", "Line Item"]),
        );
        let qs: Vec<String> = c.get("Statement_has_Quantifier")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Quantifier").map(String::from))
                .collect())
            .unwrap_or_default();
        assert!(qs.iter().any(|q| q == "at most"),
            "expected 'at most' quantifier token; got {:?}", qs);
        assert!(qs.iter().any(|q| q == "at least"),
            "expected 'at least' quantifier token; got {:?}", qs);
    }

    #[test]
    fn enum_values_declaration_sets_verb_and_emits_values() {
        // Stage-1 recognises `The possible values of <Noun> are
        // '<v1>', '<v2>', ...` so the grammar's Enum Values Declaration
        // rule (keyed on Verb = 'the possible values of') can fire.
        // Each quoted literal lands as a separate Statement has Enum
        // Value token so the translator can read them as a list.
        let c = tokenize_statement(
            "s1",
            "The possible values of Priority are 'low', 'medium', 'high'.",
            &nouns(&["Priority"]),
        );
        assert_eq!(stmt_binding(&c, "Statement_has_Head_Noun", "Head_Noun").as_deref(),
                   Some("Priority"));
        assert_eq!(stmt_binding(&c, "Statement_has_Verb", "Verb").as_deref(),
                   Some("the possible values of"));
        let vals: Vec<String> = c.get("Statement_has_Enum_Value")
            .map(|facts| facts.iter()
                .filter_map(|f| binding(f, "Enum_Value").map(String::from))
                .collect())
            .unwrap_or_default();
        assert_eq!(vals, vec!["low", "medium", "high"]);
    }
}
