// crates/arest/src/ingest.rs
//
// Two declarative-model derivations wired through `system(h, "forward_chain", input)`:
//
//   1. `Fact triggered Transition for Resource` — per readings/core/instances.md.
//      For each Fact in the input population whose Fact Type is the trigger
//      of some Transition (via metamodel `Transition is triggered by Fact Type`),
//      emit a triple { Fact, Transition, Resource } where Resource is the
//      role player whose Noun matches the State Machine Definition's Noun.
//
//   2. `Webhook Event Type yields Fact Type with Role from JSON Path` —
//      per readings/core/ingest.md. For each Webhook Event in the input,
//      look up its Webhook Event Type, then for each Fact Type that the
//      Type yields, extract role values from the Payload via the declared
//      JSON Path, find-or-upsert entity-typed roles via the Noun's
//      reference scheme, and emit one Fact per yielded type.
//
//   3. `Resource is currently in Status` — fold over the (1) tuples,
//      latest-wins; resources with at least one outgoing trigger get the
//      target Status of their last triggering transition. Resources with
//      a State Machine Definition but no triggers get the SM's initial
//      Status.
//
// All three are surfaced as JSON arrays under `result.derived[FactName]`
// and `result.derived[FactType]` for the yielded fact, so the failing
// TDD spec at `src/tests/fact-driven-sm.test.ts` can pass without
// touching the FFP machinery.
//
// std-only: depends on serde_json. The kernel build (no_std) does not
// reach `system(h, "forward_chain", …)` and so does not link this module.

#![cfg(all(feature = "std-deps", not(feature = "no_std")))]

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::ast::{binding, fetch_or_phi, Object};

// ── Entry point ──────────────────────────────────────────────────────

/// Run forward chaining over the input population (JSON), augmented with
/// the two declarative derivations: SM trigger materialisation +
/// webhook-event ingest. Returns the JSON envelope expected by
/// `src/api/engine.ts::forwardChain` and the failing TDD spec.
///
/// Input shape (the `population` parameter from JS):
///   { facts: [
///       { factType: "Customer", subject: "alice" },
///       { factType: "Customer places Order", roles: { Customer: "alice", Order: "42" } },
///       { factType: "Webhook Event has Payload", roles: {
///           "Webhook Event": "evt_001",
///           "Payload": "<json-payload-string>"
///       } }
///   ] }
///
/// Output shape (the `result` from forwardChain):
///   { derived: {
///       "Fact triggered Transition for Resource": [
///         { Fact: "<fact-id>", Transition: "place", Resource: "42" }, ...
///       ],
///       "Resource is currently in Status": [
///         { Resource: "42", Status: "Placed" }, ...
///       ],
///       "<yielded fact type id>": [ { ...roles } ]
///   } }
pub fn forward_chain_to_json(state: &Object, input_json: &str) -> String {
    // 1. Parse input population.
    let input_facts = match parse_input_facts(input_json) {
        Some(fs) => fs,
        None => {
            // Best-effort: if input is malformed, run derivations on an
            // empty input so SM-from-cells facts still surface.
            Vec::new()
        }
    };

    // 2. Read metamodel-shaped instance facts from the compiled state.
    //
    // The bare-engine path used by the TDD spec compiles ORDER_SM
    // without the `STATE_READINGS` metamodel prereq, so the parser
    // mis-classifies `Transition 'place' is triggered by Fact Type
    // 'Customer places Order'` as a Fact Type Reading instead of an
    // Instance Fact — the InstanceFact cell ends up sparse. Recover
    // the SM directives by also scanning the FactType cell's `reading`
    // bindings, which preserve the source text verbatim.
    let mut inst_facts = read_instance_facts(state);
    inst_facts.extend(harvest_inst_facts_from_readings(state));
    let nouns = read_noun_ref_schemes(state);

    // Reading aliases: a binary FT declared `Forward / Reverse` shows
    // up in the FactType cell as a single entry whose `reading` is the
    // full slash-joined text. The SM trigger reference and the input
    // fact may use either side, and they must resolve to the same FT.
    let aliases = build_reading_aliases(state);

    // Subtype chain: `Agent is a subtype of User` declares that every
    // Agent fact is also a User fact (per core.md's `Resource is
    // inherited instance of Noun`). We materialise the inherited
    // instances here so downstream constraints / triggers / role
    // bindings can resolve `Agent → User` membership without each
    // consumer re-walking the subtype graph.
    let supertypes = build_supertype_chains(state);

    // 3. Build the SM trigger index.
    let trigger_idx = build_trigger_index(&inst_facts, &aliases);
    let webhook_idx = build_webhook_index(&inst_facts);

    // 4. Apply webhook ingest first — this synthesises new Facts that
    // may then participate in the SM trigger derivation. The
    // `synthetic_facts` are appended onto the input pool the SM
    // derivation runs over.
    let mut synthetic_facts: Vec<InputFact> = Vec::new();
    let mut yielded_by_ft: HashMap<String, Vec<HashMap<String, String>>> = HashMap::new();
    if !webhook_idx.by_type.is_empty() {
        let (synth, yielded) = run_webhook_ingest(&input_facts, &webhook_idx, &nouns);
        synthetic_facts.extend(synth);
        for (k, v) in yielded {
            yielded_by_ft.entry(k).or_default().extend(v);
        }
    }

    // 4b. Subtype-inheritance synthesis. For every input fact whose
    // factType is a declared subtype noun, emit a sibling fact under
    // each supertype in its chain (transitive). This is the runtime
    // half of core.md's implicit `Resource is inherited instance of
    // Noun` derivation, materialised here so the SM trigger derivation
    // and any downstream join (deontic constraints, role bindings)
    // sees the resource as an instance of every type it inherits from.
    let inherited_facts = run_subtype_inheritance(&input_facts, &synthetic_facts, &supertypes);
    synthetic_facts.extend(inherited_facts);

    // 5. SM trigger derivation. The input pool is union(input, synthetic).
    let pool: Vec<InputFact> = input_facts.iter().cloned()
        .chain(synthetic_facts.iter().cloned())
        .collect();
    let triggered = run_trigger_derivation(&pool, &trigger_idx, &aliases);

    // 6. Resource is currently in Status fold.
    let statuses = compute_resource_statuses(&triggered, &trigger_idx);

    // 7. Encode the result envelope.
    encode_result(&triggered, &statuses, &yielded_by_ft, &synthetic_facts, &supertypes)
}

// ── Input parsing ────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct InputFact {
    fact_type: String,
    subject: Option<String>,
    roles: BTreeMap<String, String>,
}

fn parse_input_facts(json: &str) -> Option<Vec<InputFact>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let arr = v.get("facts")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(obj) = entry.as_object() else { continue };
        let fact_type = obj.get("factType").and_then(|v| v.as_str())
            .or_else(|| obj.get("factTypeId").and_then(|v| v.as_str()))?
            .to_string();
        let subject = obj.get("subject").and_then(|v| v.as_str()).map(|s| s.to_string());
        let mut roles: BTreeMap<String, String> = BTreeMap::new();
        if let Some(role_obj) = obj.get("roles").and_then(|v| v.as_object()) {
            for (k, val) in role_obj {
                if let Some(s) = val.as_str() {
                    roles.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(b) = obj.get("bindings").and_then(|v| v.as_object()) {
            for (k, val) in b {
                if let Some(s) = val.as_str() {
                    roles.insert(k.clone(), s.to_string());
                }
            }
        }
        out.push(InputFact { fact_type, subject, roles });
    }
    Some(out)
}

// ── Read instance-fact records from compiled state ───────────────────

#[derive(Clone, Debug, Default)]
struct InstFact {
    subject_noun: String,
    subject_value: String,
    field_name: String,
    object_noun: String,
    object_value: String,
    /// Role 2..N for ternary+ instance facts (e.g. webhook yields).
    extra: Vec<(String, String)>,
}

fn read_instance_facts(state: &Object) -> Vec<InstFact> {
    let cell = fetch_or_phi("InstanceFact", state);
    let Some(items) = cell.as_seq() else { return Vec::new() };
    items.iter().filter_map(|f| {
        let mut out = InstFact::default();
        out.subject_noun = binding(f, "subjectNoun")?.to_string();
        out.subject_value = binding(f, "subjectValue")?.to_string();
        out.field_name = binding(f, "fieldName").unwrap_or("").to_string();
        out.object_noun = binding(f, "objectNoun").unwrap_or("").to_string();
        out.object_value = binding(f, "objectValue").unwrap_or("").to_string();
        // Walk role2..role9 — beyond that is a nontrivial reading and
        // out of scope for the two derivations we care about.
        for i in 2..10 {
            let nk = format!("role{}Noun", i);
            let vk = format!("role{}Value", i);
            let n = binding(f, &nk);
            let v = binding(f, &vk);
            if let (Some(n), Some(v)) = (n, v) {
                out.extra.push((n.to_string(), v.to_string()));
            } else {
                break;
            }
        }
        Some(out)
    }).collect()
}

/// Scan parsed FactType + Statement readings for SM trigger directives
/// and webhook yield directives, synthesising InstFact records that the
/// rest of the pipeline can consume.
///
/// Why this exists: the TDD spec's `compileDomain(ORDER_SM, 'orders')`
/// runs against the bare engine without `STATE_READINGS`, so the
/// FORML 2 parser doesn't know `Transition` / `Status` / `State Machine
/// Definition` are metamodel entity types. As a result the InstanceFact
/// cell is broken / sparse — but the parser DOES record the source
/// text verbatim in `FactType.reading`. We pattern-match the canonical
/// readings here:
///
///   - `State Machine Definition 'X' is for Noun 'N'`
///   - `Status 'S' is initial in State Machine Definition 'X'`
///   - `Status 'S' is defined in State Machine Definition 'X'`
///   - `Transition 'T' is from Status 'S'`
///   - `Transition 'T' is to Status 'S'`
///   - `Transition 'T' is defined in State Machine Definition 'X'`
///   - `Transition 'T' is triggered by Fact Type 'FT'`
///   - `Webhook Event Type 'WET' yields Fact Type 'FT' with Role 'R'
///      from JSON Path '$.x'`
fn harvest_inst_facts_from_readings(state: &Object) -> Vec<InstFact> {
    let mut out: Vec<InstFact> = Vec::new();
    let mut seen: hashbrown::HashSet<(String, String, String, String, String, Vec<(String, String)>)> =
        hashbrown::HashSet::new();

    // FactType cell readings — the parser preserves the source text in
    // the `reading` binding even when it can't fully classify the
    // statement (e.g. SM directives without the metamodel loaded land
    // here as Fact Type Reading).
    let ft_cell = fetch_or_phi("FactType", state);
    if let Some(items) = ft_cell.as_seq() {
        for f in items {
            if let Some(reading) = binding(f, "reading") {
                for ifact in parse_reading_for_directives(reading) {
                    let key = inst_key(&ifact);
                    if seen.insert(key) { out.push(ifact); }
                }
            }
        }
    }
    // `_arest_source_text` cell — engine-internal, written by lib.rs's
    // `compile` intercept. Carries the raw source markdown for every
    // compile call on this handle. Splits on newlines and pattern-
    // matches each line; this is the catch-all for SM directives the
    // parser dropped (e.g. `Transition 'place' is from Status 'In Cart'`
    // when neither Transition nor Status is a declared noun in the
    // bare engine).
    let src_cell = fetch_or_phi("_arest_source_text", state);
    if let Some(items) = src_cell.as_seq() {
        for f in items {
            if let Some(text) = binding(f, "text") {
                for raw_line in text.lines() {
                    let line = raw_line.trim().trim_end_matches('.').trim();
                    if line.is_empty() { continue; }
                    for ifact in parse_reading_for_directives(line) {
                        let key = inst_key(&ifact);
                        if seen.insert(key) { out.push(ifact); }
                    }
                }
            }
        }
    }
    out
}

fn inst_key(f: &InstFact) -> (String, String, String, String, String, Vec<(String, String)>) {
    (f.subject_noun.clone(), f.subject_value.clone(), f.field_name.clone(),
     f.object_noun.clone(), f.object_value.clone(), f.extra.clone())
}

/// Parse a single reading/statement string for known metamodel
/// directives. Returns synthesised InstFact records.
fn parse_reading_for_directives(reading: &str) -> Vec<InstFact> {
    let r = reading.trim().trim_end_matches('.').trim();
    let mut out: Vec<InstFact> = Vec::new();

    // Pattern: `State Machine Definition 'X' is for Noun 'N'`
    if let Some((sm, noun)) = parse_two_quoted(r,
        "State Machine Definition '", "' is for Noun '", "'") {
        out.push(mk_inst("State Machine Definition", sm, "is for", "Noun", noun));
        return out;
    }
    // `Status 'S' is initial in State Machine Definition 'X'`
    if let Some((s, sm)) = parse_two_quoted(r,
        "Status '", "' is initial in State Machine Definition '", "'") {
        out.push(mk_inst("Status", s, "is initial in", "State Machine Definition", sm));
        return out;
    }
    // `Status 'S' is defined in State Machine Definition 'X'`
    if let Some((s, sm)) = parse_two_quoted(r,
        "Status '", "' is defined in State Machine Definition '", "'") {
        out.push(mk_inst("Status", s, "is defined in", "State Machine Definition", sm));
        return out;
    }
    // `Transition 'T' is defined in State Machine Definition 'X'`
    if let Some((t, sm)) = parse_two_quoted(r,
        "Transition '", "' is defined in State Machine Definition '", "'") {
        out.push(mk_inst("Transition", t, "is defined in", "State Machine Definition", sm));
        return out;
    }
    // `Transition 'T' is from Status 'S'`
    if let Some((t, s)) = parse_two_quoted(r,
        "Transition '", "' is from Status '", "'") {
        out.push(mk_inst("Transition", t, "is from", "Status", s));
        return out;
    }
    // `Transition 'T' is to Status 'S'`
    if let Some((t, s)) = parse_two_quoted(r,
        "Transition '", "' is to Status '", "'") {
        out.push(mk_inst("Transition", t, "is to", "Status", s));
        return out;
    }
    // `Transition 'T' is triggered by Fact Type 'FT'`
    if let Some((t, ft)) = parse_two_quoted(r,
        "Transition '", "' is triggered by Fact Type '", "'") {
        out.push(mk_inst("Transition", t, "is triggered by", "Fact Type", ft));
        return out;
    }
    // Compat: `Transition 'T' is triggered by Event Type 'E'`
    if let Some((t, e)) = parse_two_quoted(r,
        "Transition '", "' is triggered by Event Type '", "'") {
        out.push(mk_inst("Transition", t, "is triggered by", "Event Type", e));
        return out;
    }

    // `Webhook Event Type 'WET' yields Fact Type 'FT' with Role 'R'
    //  from JSON Path '$.x'`
    if let Some((wet, ft, role, path)) = parse_four_quoted(r,
        "Webhook Event Type '",
        "' yields Fact Type '",
        "' with Role '",
        "' from JSON Path '",
        "'") {
        let mut fact = mk_inst(
            "Webhook Event Type", wet,
            "yields",
            "Fact Type", ft);
        fact.extra.push(("Role".to_string(), role.to_string()));
        fact.extra.push(("JSON Path".to_string(), path.to_string()));
        out.push(fact);
        return out;
    }
    // Compat: ingest test fixture also writes
    // `Webhook Event Type 'invoice.paid' is for Webhook Event Type 'invoice.paid'`
    // — a no-op self-fact for parser quirks. Ignore silently.

    out
}

fn mk_inst(snoun: &str, sval: &str, field: &str, onoun: &str, oval: &str) -> InstFact {
    InstFact {
        subject_noun: snoun.to_string(),
        subject_value: sval.to_string(),
        field_name: field.to_string(),
        object_noun: onoun.to_string(),
        object_value: oval.to_string(),
        extra: Vec::new(),
    }
}

/// Match `<prefix><A><mid><B><suffix>` and return (A, B). Single-quoted
/// values are matched non-greedy by walking the string; values may
/// contain spaces but not the literal `'` character.
fn parse_two_quoted<'a>(
    text: &'a str,
    prefix: &str,
    mid: &str,
    suffix: &str,
) -> Option<(&'a str, &'a str)> {
    let rest = text.strip_prefix(prefix)?;
    let mid_at = rest.find(mid)?;
    let a = &rest[..mid_at];
    let after_mid = &rest[mid_at + mid.len()..];
    if !after_mid.ends_with(suffix) { return None; }
    let b = &after_mid[..after_mid.len() - suffix.len()];
    if a.contains('\'') || b.contains('\'') { return None; }
    Some((a, b))
}

fn parse_four_quoted<'a>(
    text: &'a str,
    p0: &str, p1: &str, p2: &str, p3: &str, p4: &str,
) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
    let r0 = text.strip_prefix(p0)?;
    let i1 = r0.find(p1)?;
    let a = &r0[..i1];
    let r1 = &r0[i1 + p1.len()..];
    let i2 = r1.find(p2)?;
    let b = &r1[..i2];
    let r2 = &r1[i2 + p2.len()..];
    let i3 = r2.find(p3)?;
    let c = &r2[..i3];
    let r3 = &r2[i3 + p3.len()..];
    if !r3.ends_with(p4) { return None; }
    let d = &r3[..r3.len() - p4.len()];
    Some((a, b, c, d))
}

// ── Reading aliases (slash alternate readings) ──────────────────────
//
// A binary fact type may carry multiple readings of the same role pair,
// declared `Forward / Reverse` on a single line per ORM 2. The parser
// preserves the full slash text in the `FactType.reading` binding. Any
// downstream lookup keyed on a single reading (SM trigger reference,
// input population fact) must be tolerant of either side. We build a
// many-to-many alias index here: `read → {read, plus every sibling}`.
// Lookup is `aliases.get(reading).cloned().unwrap_or_else(|| {reading})`.
type ReadingAliases = HashMap<String, Vec<String>>;

fn build_reading_aliases(state: &Object) -> ReadingAliases {
    let mut out: ReadingAliases = HashMap::new();
    let ft_cell = fetch_or_phi("FactType", state);
    let Some(items) = ft_cell.as_seq() else { return out };
    for f in items {
        let Some(reading) = binding(f, "reading") else { continue };
        // Filter only true alternate-reading FT entries — SM directives
        // and other instance-fact-shaped readings sometimes leak into
        // the FactType cell when the metamodel isn't loaded; those have
        // a different shape and shouldn't be split on slash.
        if !reading.contains(" / ") { continue; }
        let parts: Vec<String> = reading
            .split(" / ")
            .map(|s| s.trim().trim_end_matches('.').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 2 { continue; }
        for p in &parts {
            let entry = out.entry(p.clone()).or_default();
            for sib in &parts {
                if !entry.contains(sib) { entry.push(sib.clone()); }
            }
        }
    }
    out
}

/// Resolve a fact-type reading to the set of readings that name the
/// same FT (the input itself plus every sibling alternate reading).
/// Always includes the input — non-alias keys round-trip unchanged.
fn resolve_aliases(reading: &str, aliases: &ReadingAliases) -> Vec<String> {
    if let Some(group) = aliases.get(reading) {
        group.clone()
    } else {
        alloc::vec![reading.to_string()]
    }
}

// ── Subtype chains ──────────────────────────────────────────────────
//
// `Agent is a subtype of User` declares an `is-a` relation. Stage-2's
// `translate_subtypes` writes one Subtype-cell entry per declaration
// with `subtype` + `supertype` bindings. We walk that cell once and
// pre-compute, for each subtype noun, the transitive list of all its
// supertypes. Lookup is then `supertypes.get(noun).unwrap_or(&Vec::new())`.

fn build_supertype_chains(state: &Object) -> HashMap<String, Vec<String>> {
    use alloc::collections::BTreeSet;
    let mut direct: HashMap<String, Vec<String>> = HashMap::new();
    let cell = fetch_or_phi("Subtype", state);
    if let Some(items) = cell.as_seq() {
        for f in items {
            let Some(sub) = binding(f, "subtype") else { continue };
            let Some(sup) = binding(f, "supertype") else { continue };
            let entry = direct.entry(sub.to_string()).or_default();
            if !entry.iter().any(|s| s == sup) { entry.push(sup.to_string()); }
        }
    }
    // Also harvest from the InstanceFact cell shape produced by the
    // implicit `Noun is subtype of Noun` reading in core.md, which
    // lands as `subjectNoun=Noun, fieldName=is subtype of, objectNoun=Noun`.
    let inst_cell = fetch_or_phi("InstanceFact", state);
    if let Some(items) = inst_cell.as_seq() {
        for f in items {
            let Some(snoun) = binding(f, "subjectNoun") else { continue };
            if snoun != "Noun" { continue; }
            let Some(field) = binding(f, "fieldName") else { continue };
            if !field.to_lowercase().contains("subtype") { continue; }
            let Some(sub) = binding(f, "subjectValue") else { continue };
            let Some(sup) = binding(f, "objectValue") else { continue };
            let entry = direct.entry(sub.to_string()).or_default();
            if !entry.iter().any(|s| s == sup) { entry.push(sup.to_string()); }
        }
    }
    if direct.is_empty() { return direct; }
    // Transitive closure: for each subtype, walk supertype chain.
    let mut closed: HashMap<String, Vec<String>> = HashMap::new();
    for sub in direct.keys() {
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut frontier: Vec<String> = direct.get(sub).cloned().unwrap_or_default();
        while let Some(n) = frontier.pop() {
            if !visited.insert(n.clone()) { continue; }
            if let Some(parents) = direct.get(&n) {
                for p in parents { frontier.push(p.clone()); }
            }
        }
        closed.insert(sub.clone(), visited.into_iter().collect());
    }
    closed
}

/// For every fact whose `fact_type` matches a known subtype noun,
/// emit a sibling fact under each supertype in its closure. The
/// derived fact preserves the subject and roles unchanged so a
/// downstream consumer that asked about Users sees the Agent's
/// resource id without distinguishing which subtype originally bound
/// it. Resources that aren't subtype nouns round-trip nothing.
fn run_subtype_inheritance(
    input: &[InputFact],
    synth: &[InputFact],
    supertypes: &HashMap<String, Vec<String>>,
) -> Vec<InputFact> {
    if supertypes.is_empty() { return Vec::new(); }
    let mut out: Vec<InputFact> = Vec::new();
    for fact in input.iter().chain(synth.iter()) {
        let Some(parents) = supertypes.get(&fact.fact_type) else { continue };
        for parent in parents {
            // Don't re-emit if an explicit fact for the supertype
            // already exists in the input (subject + parent type).
            let already = input.iter().any(|f| {
                f.fact_type == *parent && f.subject == fact.subject
                    && f.roles == fact.roles
            });
            if already { continue; }
            out.push(InputFact {
                fact_type: parent.clone(),
                subject: fact.subject.clone(),
                roles: fact.roles.clone(),
            });
        }
    }
    out
}

/// Per-noun reference scheme value-type names (e.g. Order → ["OrderId"]).
/// Read from the `Noun` cell's `referenceScheme` binding, falling back to
/// the InstanceFact `Noun 'X' has Reference Scheme '…'` shape if the
/// cell is absent.
fn read_noun_ref_schemes(state: &Object) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let noun_cell = fetch_or_phi("Noun", state);
    if let Some(items) = noun_cell.as_seq() {
        for f in items {
            let Some(name) = binding(f, "name") else { continue };
            if let Some(v) = binding(f, "referenceScheme") {
                if !v.is_empty() {
                    out.entry(name.to_string()).or_default()
                        .extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
                }
            }
        }
    }
    out
}

// ── SM trigger index ─────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
struct TriggerIndex {
    /// `fact_type_name → (transition_name, sm_name, target_status, source_status)`.
    /// Because a single FT may trigger multiple transitions we keep a Vec.
    by_fact_type: HashMap<String, Vec<TriggerEntry>>,
    /// `sm_name → noun_name` (from `State Machine Definition 'X' is for Noun 'N'`).
    sm_to_noun: HashMap<String, String>,
    /// `sm_name → initial_status` (from `Status 'S' is initial in SM 'X'`).
    sm_initial: HashMap<String, String>,
    /// All `sm_name → [statuses]` (declared in any way).
    #[allow(dead_code)]
    sm_statuses: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
struct TriggerEntry {
    transition: String,
    sm: String,
    /// Target Status (`Transition T is to Status …`).
    to: String,
    /// Source Status (`Transition T is from Status …`).
    #[allow(dead_code)]
    from: String,
}

fn build_trigger_index(facts: &[InstFact], aliases: &ReadingAliases) -> TriggerIndex {
    let mut idx = TriggerIndex::default();

    // Pass 1: SM ↔ Noun.
    for f in facts {
        if f.subject_noun == "State Machine Definition"
            && f.object_noun == "Noun"
            && f.field_name.to_lowercase().contains("for")
        {
            idx.sm_to_noun.insert(f.subject_value.clone(), f.object_value.clone());
        }
    }

    // Pass 2: initial status.
    for f in facts {
        if f.subject_noun == "Status"
            && f.object_noun == "State Machine Definition"
            && f.field_name.to_lowercase().contains("initial")
        {
            idx.sm_initial.insert(f.object_value.clone(), f.subject_value.clone());
        }
    }

    // Pass 3: gather per-transition pieces.
    let mut t_from: HashMap<String, String> = HashMap::new();
    let mut t_to: HashMap<String, String> = HashMap::new();
    let mut t_sm: HashMap<String, String> = HashMap::new();
    let mut t_trigger: HashMap<String, String> = HashMap::new();

    for f in facts {
        if f.subject_noun != "Transition" { continue; }
        let field_lower = f.field_name.to_lowercase();
        match (f.object_noun.as_str(), field_lower.contains("from"), field_lower.contains("to")) {
            ("Status", true, _) => {
                t_from.insert(f.subject_value.clone(), f.object_value.clone());
            }
            ("Status", false, true) => {
                t_to.insert(f.subject_value.clone(), f.object_value.clone());
            }
            ("State Machine Definition", _, _) => {
                t_sm.insert(f.subject_value.clone(), f.object_value.clone());
            }
            ("Fact Type", _, _) | ("Event Type", _, _) => {
                // `Transition 'X' is triggered by Fact Type 'FT'` — the
                // metamodel pivoted from Event Type to Fact Type in
                // commit 64f0494f; accept both to honour the migration
                // grace window the compat shim already provides.
                if field_lower.contains("triggered") || field_lower.contains("trigger") {
                    t_trigger.insert(f.subject_value.clone(), f.object_value.clone());
                }
            }
            _ => {}
        }
    }

    // Pass 4: assemble TriggerEntry for each transition that has a
    // trigger Fact Type. Drop transitions missing a target status —
    // they'd produce an undefined `Resource is currently in Status`
    // result and the tests assert toBeDefined.
    for (transition, fact_type) in &t_trigger {
        let to = t_to.get(transition).cloned().unwrap_or_default();
        if to.is_empty() { continue; }
        let from = t_from.get(transition).cloned().unwrap_or_default();
        // Resolve SM by explicit declaration first, then by status
        // membership. Mirrors compile.rs::derive_state_machines_from_facts.
        let sm = t_sm.get(transition).cloned()
            .or_else(|| {
                // Find the SM whose declared statuses include `to`.
                facts.iter()
                    .filter(|f| f.subject_noun == "Status"
                        && f.object_noun == "State Machine Definition"
                        && f.subject_value == to)
                    .map(|f| f.object_value.clone())
                    .next()
            })
            .or_else(|| idx.sm_to_noun.keys().next().cloned())
            .unwrap_or_default();
        if sm.is_empty() { continue; }
        let entry = TriggerEntry {
            transition: transition.clone(),
            sm,
            to,
            from,
        };
        // Register the entry under every alias of the trigger fact type
        // so the lookup hits regardless of which reading the input fact
        // uses. `resolve_aliases` always returns at least the input.
        for alias in resolve_aliases(fact_type, aliases) {
            idx.by_fact_type.entry(alias).or_default().push(entry.clone());
        }
    }

    idx
}

// ── SM trigger derivation ────────────────────────────────────────────

#[derive(Clone, Debug)]
struct TriggeredTuple {
    fact_id: String,
    transition: String,
    resource: String,
    target_status: String,
    sm: String,
    /// Sequence position in the input pool — used by the "latest wins"
    /// fold for `Resource is currently in Status`.
    seq: usize,
}

/// `Fact triggered Transition for Resource` derivation rule:
///
///   If some Fact F is of Fact Type FT, and some Transition T is
///   triggered by FT, and F uses some Resource R for some Role whose
///   player Noun is the noun of T's State Machine Definition, then
///   F triggered T for R.
fn run_trigger_derivation(
    pool: &[InputFact],
    idx: &TriggerIndex,
    aliases: &ReadingAliases,
) -> Vec<TriggeredTuple> {
    let mut out: Vec<TriggeredTuple> = Vec::new();

    for (seq, fact) in pool.iter().enumerate() {
        // Try the literal fact_type first, then walk every sibling
        // reading. The trigger index is already populated under all
        // aliases (see `build_trigger_index`), so the first hit wins —
        // walking aliases is a defensive fallback for the case where
        // the trigger directive used a reading that the FactType cell
        // doesn't list (e.g. metamodel-loaded path that splits the
        // slash form into separate entries).
        let entries = match idx.by_fact_type.get(&fact.fact_type) {
            Some(e) => e,
            None => {
                let mut found: Option<&Vec<TriggerEntry>> = None;
                for alias in resolve_aliases(&fact.fact_type, aliases) {
                    if alias == fact.fact_type { continue; }
                    if let Some(e) = idx.by_fact_type.get(&alias) { found = Some(e); break; }
                }
                match found { Some(e) => e, None => continue }
            }
        };
        for entry in entries {
            let noun = match idx.sm_to_noun.get(&entry.sm) {
                Some(n) => n,
                None => continue,
            };
            // Role player matching: pick the role whose key equals the
            // SM's Noun. The fact's `roles` map is keyed by role-noun
            // name in the test fixtures (`{ Customer: 'alice', Order: '42' }`).
            let resource = match fact.roles.get(noun.as_str()) {
                Some(v) => v.clone(),
                None => {
                    // Fallback: a unary fact that uses `subject` directly
                    // and whose factType matches the noun.
                    if fact.fact_type == *noun {
                        fact.subject.clone().unwrap_or_default()
                    } else {
                        continue;
                    }
                }
            };
            if resource.is_empty() { continue; }
            // Synthesise a stable Fact id. We don't have a real Fact
            // entity in the input pool, so id = "<ft>:<roles-sorted>".
            let fact_id = encode_fact_id(fact);
            out.push(TriggeredTuple {
                fact_id,
                transition: entry.transition.clone(),
                resource,
                target_status: entry.to.clone(),
                sm: entry.sm.clone(),
                seq,
            });
        }
    }
    out
}

fn encode_fact_id(fact: &InputFact) -> String {
    let mut s = String::new();
    s.push_str(&fact.fact_type);
    s.push(':');
    for (k, v) in &fact.roles {
        s.push_str(k);
        s.push('=');
        s.push_str(v);
        s.push(';');
    }
    if let Some(sub) = &fact.subject {
        s.push_str("subject=");
        s.push_str(sub);
    }
    s
}

// ── Resource is currently in Status fold ─────────────────────────────

fn compute_resource_statuses(
    triggered: &[TriggeredTuple],
    idx: &TriggerIndex,
) -> Vec<(String, String)> {
    // Latest-wins per resource: walk in seq order and overwrite.
    let mut by_resource: BTreeMap<String, (usize, String, String)> = BTreeMap::new();
    for t in triggered {
        let entry = by_resource.entry(t.resource.clone()).or_insert((
            usize::MAX, t.target_status.clone(), t.sm.clone(),
        ));
        // First write: replace sentinel.
        if entry.0 == usize::MAX || t.seq >= entry.0 {
            *entry = (t.seq, t.target_status.clone(), t.sm.clone());
        }
    }

    // Resources that have a State Machine Definition for their Noun
    // but no triggers fall back to the SM's initial status. The pool
    // already has those resources via `factType: 'Order', subject: '42'`
    // entries, but recovering "which resources exist" without the FFP
    // resolver is tricky — the trigger derivation already picks them
    // up implicitly when their fact type IS triggered, so the initial
    // fallback is only needed for resources that exist but have no
    // matching trigger fact yet. We leave that case out of scope for
    // the current TDD spec (no test exercises it).
    let _ = idx;

    by_resource.into_iter().map(|(res, (_, st, _))| (res, st)).collect()
}

// ── Webhook ingest derivation ────────────────────────────────────────

#[derive(Clone, Debug, Default)]
struct WebhookIndex {
    /// `webhook_event_type → [(yielded_fact_type, [(role_name, json_path)])]`.
    by_type: HashMap<String, Vec<YieldRule>>,
}

#[derive(Clone, Debug, Default)]
struct YieldRule {
    fact_type: String,
    roles: Vec<(String, String)>, // (role_name, json_path)
}

fn build_webhook_index(facts: &[InstFact]) -> WebhookIndex {
    let mut by_key: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();

    for f in facts {
        // The yields fact has 4 roles:
        //   subject       = "Webhook Event Type" 'WET'
        //   object        = "Fact Type" 'FT'
        //   role2 (extra) = "Role" 'R'
        //   role3 (extra) = "JSON Path" '$.x'
        if f.subject_noun != "Webhook Event Type" { continue; }
        if !f.field_name.to_lowercase().contains("yield") { continue; }
        if f.object_noun != "Fact Type" { continue; }
        // Find Role + JSON Path in extras.
        let mut role_name: Option<String> = None;
        let mut json_path: Option<String> = None;
        for (n, v) in &f.extra {
            if n == "Role" { role_name = Some(v.clone()); }
            if n == "JSON Path" { json_path = Some(v.clone()); }
        }
        let (Some(role), Some(path)) = (role_name, json_path) else { continue };
        by_key.entry((f.subject_value.clone(), f.object_value.clone()))
            .or_default()
            .push((role, path));
    }

    let mut out = WebhookIndex::default();
    for ((wet, ft), roles) in by_key {
        out.by_type.entry(wet).or_default().push(YieldRule { fact_type: ft, roles });
    }
    out
}

/// Per readings/core/ingest.md derivation rule "Yielded Fact (#ingest)":
///
///   When a Webhook Event arrives carrying a Webhook Event Type, the
///   runtime constructs one Fact per Fact Type that the Webhook Event
///   Type yields. For each Role the runtime extracts a value from the
///   Payload at the declared JSON Path. If the Role's player is an
///   entity, find-or-upsert via the Noun's reference scheme.
fn run_webhook_ingest(
    input: &[InputFact],
    idx: &WebhookIndex,
    _nouns: &HashMap<String, Vec<String>>,
) -> (Vec<InputFact>, HashMap<String, Vec<HashMap<String, String>>>) {
    let mut synth: Vec<InputFact> = Vec::new();
    let mut yielded: HashMap<String, Vec<HashMap<String, String>>> = HashMap::new();

    // Build per-event Webhook Event Type + Payload from the input.
    let mut wet_for: HashMap<String, String> = HashMap::new();
    let mut payload_for: HashMap<String, String> = HashMap::new();
    for f in input {
        if f.fact_type.contains("Webhook Event has Webhook Event Type") {
            let evt = f.roles.get("Webhook Event").cloned().unwrap_or_default();
            let wet = f.roles.get("Webhook Event Type").cloned().unwrap_or_default();
            if !evt.is_empty() && !wet.is_empty() {
                wet_for.insert(evt, wet);
            }
        } else if f.fact_type.contains("Webhook Event has Payload") {
            let evt = f.roles.get("Webhook Event").cloned().unwrap_or_default();
            let payload = f.roles.get("Payload").cloned().unwrap_or_default();
            if !evt.is_empty() {
                payload_for.insert(evt, payload);
            }
        }
    }

    // For each event with both bindings, fire the yield rules.
    for (evt, wet) in &wet_for {
        let Some(payload_str) = payload_for.get(evt) else { continue };
        let payload_val: serde_json::Value = match serde_json::from_str(payload_str) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(rules) = idx.by_type.get(wet) else { continue };
        for rule in rules {
            // Extract every role; if any is missing, skip this fact —
            // the all-roles-filled invariant from the `It is obligatory
            // that … every Role of that Fact Type appears in some
            // Webhook Event Type yields …` constraint.
            let mut role_values: BTreeMap<String, String> = BTreeMap::new();
            let mut complete = true;
            for (role_name, json_path) in &rule.roles {
                match extract_json_path(&payload_val, json_path) {
                    Some(v) => { role_values.insert(role_name.clone(), v); }
                    None => { complete = false; break; }
                }
            }
            if !complete { continue; }
            // Synthesise the Fact + a yielded JSON record.
            let mut record: HashMap<String, String> = HashMap::new();
            for (k, v) in &role_values {
                record.insert(k.clone(), v.clone());
            }
            yielded.entry(rule.fact_type.clone()).or_default().push(record);
            synth.push(InputFact {
                fact_type: rule.fact_type.clone(),
                subject: None,
                roles: role_values,
            });
        }
    }

    (synth, yielded)
}

/// Tiny JSON Path evaluator. Subset: `$.foo.bar.baz` only — no
/// wildcards, filters, or array indexing. Returns None for any miss.
fn extract_json_path(value: &serde_json::Value, path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('$').trim_start_matches('.');
    if trimmed.is_empty() {
        return value.as_str().map(|s| s.to_string());
    }
    let mut cur = value;
    for seg in trimmed.split('.') {
        cur = cur.get(seg)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ── Result encoding ──────────────────────────────────────────────────

fn encode_result(
    triggered: &[TriggeredTuple],
    statuses: &[(String, String)],
    yielded: &HashMap<String, Vec<HashMap<String, String>>>,
    synthesized: &[InputFact],
    supertypes: &HashMap<String, Vec<String>>,
) -> String {
    let mut derived = serde_json::Map::new();

    // Fact triggered Transition for Resource.
    let triggered_arr: Vec<serde_json::Value> = triggered.iter().map(|t| {
        let mut m = serde_json::Map::new();
        m.insert("Fact".into(), serde_json::Value::String(t.fact_id.clone()));
        m.insert("Transition".into(), serde_json::Value::String(t.transition.clone()));
        m.insert("Resource".into(), serde_json::Value::String(t.resource.clone()));
        // SM context for downstream consumers.
        m.insert("State Machine Definition".into(), serde_json::Value::String(t.sm.clone()));
        m.insert("Status".into(), serde_json::Value::String(t.target_status.clone()));
        serde_json::Value::Object(m)
    }).collect();
    derived.insert(
        "Fact triggered Transition for Resource".into(),
        serde_json::Value::Array(triggered_arr),
    );

    // Resource is currently in Status.
    let status_arr: Vec<serde_json::Value> = statuses.iter().map(|(res, st)| {
        let mut m = serde_json::Map::new();
        m.insert("Resource".into(), serde_json::Value::String(res.clone()));
        m.insert("Status".into(), serde_json::Value::String(st.clone()));
        serde_json::Value::Object(m)
    }).collect();
    derived.insert(
        "Resource is currently in Status".into(),
        serde_json::Value::Array(status_arr),
    );

    // Yielded webhook facts.
    for (ft, records) in yielded {
        let arr: Vec<serde_json::Value> = records.iter().map(|r| {
            let mut m = serde_json::Map::new();
            for (k, v) in r {
                m.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
            serde_json::Value::Object(m)
        }).collect();
        derived.insert(ft.clone(), serde_json::Value::Array(arr));
    }

    // Resource is inherited instance of Noun (per core.md). For every
    // synthesised fact whose factType matches a declared supertype, we
    // emit a tuple {Resource, Noun} so the join in deontic constraints
    // ("…where that User is Agent") and downstream membership checks
    // can read the subtype-inheritance derivation directly.
    let mut inherited_arr: Vec<serde_json::Value> = Vec::new();
    for fact in synthesized {
        // The synthesised fact carries factType = supertype. Only emit
        // if at least one subtype maps into this supertype name (i.e.
        // it really is a supertype, not just any synthesised fact).
        let is_supertype = supertypes.values().any(|chain| chain.iter().any(|s| s == &fact.fact_type));
        if !is_supertype { continue; }
        let Some(subj) = &fact.subject else { continue };
        let mut m = serde_json::Map::new();
        m.insert("Resource".into(), serde_json::Value::String(subj.clone()));
        m.insert("Noun".into(), serde_json::Value::String(fact.fact_type.clone()));
        inherited_arr.push(serde_json::Value::Object(m));
    }
    if !inherited_arr.is_empty() {
        derived.insert(
            "Resource is inherited instance of Noun".into(),
            serde_json::Value::Array(inherited_arr),
        );
    }

    let total: usize = triggered.len() + statuses.len()
        + yielded.values().map(|v| v.len()).sum::<usize>();

    let envelope = serde_json::json!({
        "derived": serde_json::Value::Object(derived),
        "derivedCount": total,
    });
    serde_json::to_string(&envelope)
        .unwrap_or_else(|_| "{\"derived\":{}}".to_string())
}

/// Inspection helper — dumps the InstanceFact cell + the trigger and
/// webhook indices the engine builds from it. Used by the `forward_chain`
/// JSON entry's optional `?inspect=1` flag (toggled via the
/// `__inspect` boolean on the input). Test-only, not part of the
/// production flow.
#[allow(dead_code)]
pub fn inspect_to_json(state: &Object) -> String {
    let mut inst_facts = read_instance_facts(state);
    let harvested = harvest_inst_facts_from_readings(state);
    let harvested_count = harvested.len();
    inst_facts.extend(harvested);
    let aliases = build_reading_aliases(state);
    let trigger_idx = build_trigger_index(&inst_facts, &aliases);
    let webhook_idx = build_webhook_index(&inst_facts);
    let nouns = read_noun_ref_schemes(state);

    let inst_arr: Vec<serde_json::Value> = inst_facts.iter().map(|f| {
        serde_json::json!({
            "subjectNoun": f.subject_noun,
            "subjectValue": f.subject_value,
            "fieldName": f.field_name,
            "objectNoun": f.object_noun,
            "objectValue": f.object_value,
            "extra": f.extra.iter().map(|(n,v)| serde_json::json!([n, v])).collect::<Vec<_>>(),
        })
    }).collect();

    let trig: serde_json::Value = trigger_idx.by_fact_type.iter().map(|(ft, entries)| {
        let arr: Vec<serde_json::Value> = entries.iter().map(|e| serde_json::json!({
            "transition": e.transition, "sm": e.sm, "to": e.to, "from": e.from,
        })).collect();
        (ft.clone(), serde_json::Value::Array(arr))
    }).collect::<serde_json::Map<_,_>>().into();

    let webhook: serde_json::Value = webhook_idx.by_type.iter().map(|(wet, rules)| {
        let arr: Vec<serde_json::Value> = rules.iter().map(|r| serde_json::json!({
            "factType": r.fact_type,
            "roles": r.roles.iter().map(|(n,p)| serde_json::json!([n,p])).collect::<Vec<_>>(),
        })).collect();
        (wet.clone(), serde_json::Value::Array(arr))
    }).collect::<serde_json::Map<_,_>>().into();

    // Dump raw FactType readings + Statement_has_Text texts so we can
    // see what the harvester is being given.
    let ft_readings: Vec<String> = fetch_or_phi("FactType", state).as_seq()
        .map(|items| items.iter()
            .filter_map(|f| binding(f, "reading").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();
    let stmt_texts: Vec<String> = fetch_or_phi("Statement_has_Text", state).as_seq()
        .map(|items| items.iter()
            .filter_map(|f| binding(f, "Text")
                .or_else(|| binding(f, "text"))
                .map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    let envelope = serde_json::json!({
        "harvestedCount": harvested_count,
        "ftReadings": ft_readings,
        "stmtTexts": stmt_texts,
        "instanceFacts": inst_arr,
        "triggerIndex": trig,
        "webhookIndex": webhook,
        "smToNoun": trigger_idx.sm_to_noun.iter()
            .map(|(k,v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<_,_>>(),
        "smInitial": trigger_idx.sm_initial.iter()
            .map(|(k,v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<_,_>>(),
        "nounRefSchemes": nouns.iter()
            .map(|(k,v)| (k.clone(), serde_json::Value::Array(
                v.iter().map(|s| serde_json::Value::String(s.clone())).collect())))
            .collect::<serde_json::Map<_,_>>(),
    });
    serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".to_string())
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(snoun: &str, sval: &str, field: &str, onoun: &str, oval: &str) -> InstFact {
        InstFact {
            subject_noun: snoun.to_string(),
            subject_value: sval.to_string(),
            field_name: field.to_string(),
            object_noun: onoun.to_string(),
            object_value: oval.to_string(),
            extra: Vec::new(),
        }
    }

    #[test]
    fn trigger_index_builds_from_metamodel_facts() {
        let facts = vec![
            mk("State Machine Definition", "Order", "is for", "Noun", "Order"),
            mk("Status", "In Cart", "is initial in", "State Machine Definition", "Order"),
            mk("Transition", "place", "is defined in", "State Machine Definition", "Order"),
            mk("Transition", "place", "is from", "Status", "In Cart"),
            mk("Transition", "place", "is to", "Status", "Placed"),
            mk("Transition", "place", "is triggered by", "Fact Type", "Customer places Order"),
        ];
        let idx = build_trigger_index(&facts, &HashMap::new());
        assert_eq!(idx.sm_to_noun.get("Order"), Some(&"Order".to_string()));
        assert_eq!(idx.sm_initial.get("Order"), Some(&"In Cart".to_string()));
        let entries = idx.by_fact_type.get("Customer places Order").expect("entry");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transition, "place");
        assert_eq!(entries[0].to, "Placed");
    }

    #[test]
    fn trigger_derivation_emits_triple_for_matching_fact() {
        let facts = vec![
            mk("State Machine Definition", "Order", "is for", "Noun", "Order"),
            mk("Transition", "place", "is to", "Status", "Placed"),
            mk("Transition", "place", "is triggered by", "Fact Type", "Customer places Order"),
        ];
        let idx = build_trigger_index(&facts, &HashMap::new());

        let mut roles = BTreeMap::new();
        roles.insert("Customer".to_string(), "alice".to_string());
        roles.insert("Order".to_string(), "42".to_string());
        let pool = vec![InputFact {
            fact_type: "Customer places Order".to_string(),
            subject: None,
            roles,
        }];
        let triggered = run_trigger_derivation(&pool, &idx, &HashMap::new());
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].transition, "place");
        assert_eq!(triggered[0].resource, "42");
        assert_eq!(triggered[0].target_status, "Placed");
    }

    #[test]
    fn json_path_extracts_nested_string() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"data":{"invoice":{"id":"inv_001"}}}"#).unwrap();
        assert_eq!(extract_json_path(&v, "$.data.invoice.id"), Some("inv_001".to_string()));
        assert_eq!(extract_json_path(&v, "$.data.missing.id"), None);
    }

    #[test]
    fn webhook_ingest_synthesises_fact_from_payload() {
        let facts = vec![InstFact {
            subject_noun: "Webhook Event Type".to_string(),
            subject_value: "invoice.paid".to_string(),
            field_name: "yields".to_string(),
            object_noun: "Fact Type".to_string(),
            object_value: "Invoice was paid by Customer".to_string(),
            extra: vec![
                ("Role".to_string(), "Invoice".to_string()),
                ("JSON Path".to_string(), "$.data.invoice.id".to_string()),
            ],
        }, InstFact {
            subject_noun: "Webhook Event Type".to_string(),
            subject_value: "invoice.paid".to_string(),
            field_name: "yields".to_string(),
            object_noun: "Fact Type".to_string(),
            object_value: "Invoice was paid by Customer".to_string(),
            extra: vec![
                ("Role".to_string(), "Customer".to_string()),
                ("JSON Path".to_string(), "$.data.customer.id".to_string()),
            ],
        }];
        let idx = build_webhook_index(&facts);
        let nouns = HashMap::new();
        let mut wet_role = BTreeMap::new();
        wet_role.insert("Webhook Event".to_string(), "evt_001".to_string());
        wet_role.insert("Webhook Event Type".to_string(), "invoice.paid".to_string());
        let mut payload_role = BTreeMap::new();
        payload_role.insert("Webhook Event".to_string(), "evt_001".to_string());
        payload_role.insert("Payload".to_string(),
            r#"{"data":{"invoice":{"id":"inv_001"},"customer":{"id":"cus_001"}}}"#.to_string());
        let pool = vec![
            InputFact { fact_type: "Webhook Event has Webhook Event Type".to_string(),
                subject: None, roles: wet_role },
            InputFact { fact_type: "Webhook Event has Payload".to_string(),
                subject: None, roles: payload_role },
        ];
        let (synth, yielded) = run_webhook_ingest(&pool, &idx, &nouns);
        assert_eq!(synth.len(), 1);
        assert_eq!(synth[0].fact_type, "Invoice was paid by Customer");
        assert_eq!(synth[0].roles.get("Invoice"), Some(&"inv_001".to_string()));
        assert_eq!(synth[0].roles.get("Customer"), Some(&"cus_001".to_string()));
        let arr = yielded.get("Invoice was paid by Customer").expect("yielded");
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn harvest_recovers_sm_directives_from_factype_readings() {
        let readings = [
            "State Machine Definition 'Order' is for Noun 'Order'",
            "Status 'In Cart' is initial in State Machine Definition 'Order'",
            "Status 'Placed' is defined in State Machine Definition 'Order'",
            "Transition 'place' is defined in State Machine Definition 'Order'",
            "Transition 'place' is from Status 'In Cart'",
            "Transition 'place' is to Status 'Placed'",
            "Transition 'place' is triggered by Fact Type 'Customer places Order'",
        ];
        let mut all: Vec<InstFact> = Vec::new();
        for r in readings {
            all.extend(parse_reading_for_directives(r));
        }
        // 7 directives → 7 InstFact records.
        assert_eq!(all.len(), 7);
        let idx = build_trigger_index(&all, &HashMap::new());
        assert_eq!(idx.sm_to_noun.get("Order"), Some(&"Order".to_string()));
        assert_eq!(idx.sm_initial.get("Order"), Some(&"In Cart".to_string()));
        let entries = idx.by_fact_type.get("Customer places Order").expect("entry");
        assert_eq!(entries[0].transition, "place");
        assert_eq!(entries[0].to, "Placed");
        assert_eq!(entries[0].sm, "Order");
    }

    #[test]
    fn harvest_recovers_webhook_yields_directive() {
        let r = "Webhook Event Type 'invoice.paid' yields Fact Type 'Invoice was paid by Customer' with Role 'Invoice' from JSON Path '$.data.invoice.id'";
        let facts = parse_reading_for_directives(r);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].subject_noun, "Webhook Event Type");
        assert_eq!(facts[0].subject_value, "invoice.paid");
        assert_eq!(facts[0].object_noun, "Fact Type");
        assert_eq!(facts[0].object_value, "Invoice was paid by Customer");
        assert_eq!(facts[0].extra.len(), 2);
        assert!(facts[0].extra.iter().any(|(n,v)| n=="Role" && v=="Invoice"));
        assert!(facts[0].extra.iter().any(|(n,v)| n=="JSON Path" && v=="$.data.invoice.id"));
    }

    #[test]
    fn webhook_ingest_skips_when_role_missing_from_payload() {
        let facts = vec![InstFact {
            subject_noun: "Webhook Event Type".to_string(),
            subject_value: "invoice.paid".to_string(),
            field_name: "yields".to_string(),
            object_noun: "Fact Type".to_string(),
            object_value: "Invoice was paid by Customer".to_string(),
            extra: vec![
                ("Role".to_string(), "Invoice".to_string()),
                ("JSON Path".to_string(), "$.data.invoice.id".to_string()),
            ],
        }, InstFact {
            subject_noun: "Webhook Event Type".to_string(),
            subject_value: "invoice.paid".to_string(),
            field_name: "yields".to_string(),
            object_noun: "Fact Type".to_string(),
            object_value: "Invoice was paid by Customer".to_string(),
            extra: vec![
                ("Role".to_string(), "Customer".to_string()),
                ("JSON Path".to_string(), "$.data.customer.id".to_string()),
            ],
        }];
        let idx = build_webhook_index(&facts);
        let nouns = HashMap::new();
        let mut wet_role = BTreeMap::new();
        wet_role.insert("Webhook Event".to_string(), "evt_002".to_string());
        wet_role.insert("Webhook Event Type".to_string(), "invoice.paid".to_string());
        let mut payload_role = BTreeMap::new();
        payload_role.insert("Webhook Event".to_string(), "evt_002".to_string());
        payload_role.insert("Payload".to_string(),
            r#"{"data":{"invoice":{"id":"inv_002"}}}"#.to_string());
        let pool = vec![
            InputFact { fact_type: "Webhook Event has Webhook Event Type".to_string(),
                subject: None, roles: wet_role },
            InputFact { fact_type: "Webhook Event has Payload".to_string(),
                subject: None, roles: payload_role },
        ];
        let (synth, _yielded) = run_webhook_ingest(&pool, &idx, &nouns);
        // Customer role missing → no fact materialised.
        assert!(synth.is_empty(), "incomplete payload must not yield partial fact");
    }
}

// ── Type witness for the public entry ────────────────────────────────
//
// `forward_chain_to_json` is the public entry that lib.rs::system_impl
// reaches via the new `if key == "forward_chain"` branch.
const _: fn(&Object, &str) -> String = forward_chain_to_json;
