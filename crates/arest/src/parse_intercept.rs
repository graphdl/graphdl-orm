// crates/arest/src/parse_intercept.rs
//
// Stateless `parse` / `parse_with_nouns` dispatch for the JS worker
// (`src/api/engine.ts::parseReadings` & `parseReadingsWithNouns`).
// Both call `system(0, key, json)` — handle 0 is a sentinel for "no
// tenant needed, just run the parser and hand me entities". Without
// this intercept `system_impl` falls into `tenant_lock(0) → None →
// ⊥`, which surfaces as "Unexpected token '⊥', "⊥" is not valid
// JSON" on every seed file.
//
// The intercept wraps `parse_to_state` / `parse_to_state_with_nouns`
// and serialises the resulting Object into the entity-array shape the
// worker's `parse.ts` expects:
//
//   [{ id, type, domain, data }, ...]
//
// where `type` is the human-readable cell name ("Fact Type" not
// "FactType"), `id` is `${domain}:${type}:${name|index}`, and `data`
// is the fact's bindings flattened into a `{key: value}` object.
// `materializeBatch` on the worker side walks this list and writes
// one EntityDB row per element.
//
// Only emitted under `feature = "std-deps"` (and not `feature =
// "no_std"`) — the parser itself requires std, so the kernel build
// can skip the whole module.

#![cfg(all(feature = "std-deps", not(feature = "no_std")))]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use crate::ast::{Object, fetch_or_phi, binding};

/// Cell-name → worker-facing entity type. Order matters only for
/// determinism: the worker doesn't care about element order, but
/// stable output makes diffs over the seed log readable.
const CELLS: &[(&str, &str)] = &[
    ("Noun",                    "Noun"),
    ("FactType",                "Fact Type"),
    ("Role",                    "Role"),
    ("Reading",                 "Reading"),
    ("Constraint",              "Constraint"),
    ("DerivationRule",          "Derivation Rule"),
    ("StateMachineDefinition",  "State Machine Definition"),
    ("Status",                  "Status"),
    ("Transition",              "Transition"),
    ("InstanceFact",            "Instance Fact"),
    ("ExternalSystem",          "External System"),
    ("CompiledSchema",          "CompiledSchema"),
];

/// Public entry. Returns the entity-array JSON or a single-element
/// error array (`[{"_error": "..."}]`) so the worker's
/// `JSON.parse(...)` always succeeds — the worker can then surface
/// the message in `errors[]` without a thrown SyntaxError.
///
/// `with_nouns = true` reads the `nouns` field from the input JSON
/// and threads it through the parser as context (cross-domain noun
/// resolution: tier-N readings can reference nouns declared in
/// tiers 1..N-1).
pub fn parse_dispatch(input: &str, with_nouns: bool) -> String {
    // Wrap the whole parse in `catch_unwind` so a panic anywhere in
    // stage1/stage2 (or the entity walker) surfaces as a structured
    // error instead of trapping the whole worker as `RuntimeError:
    // unreachable`. wasm-pack's bundler target uses `panic = "abort"`
    // by default, but `panic::catch_unwind` still works on wasm32 in
    // recent stable: it uses the unwind ABI when the crate's panic
    // strategy is unwind, and a no-op-then-abort otherwise. Either
    // way the *first* panic before catch is intercepted by the
    // `console_error_panic_hook`'s custom handler, which writes the
    // panic message to console.error before unwinding/trapping. We
    // ALSO grab the message via `panic::set_hook` here so the worker
    // sees it in the parse response without needing `wrangler tail`.
    let panic_msg = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let panic_msg_for_hook = std::sync::Arc::clone(&panic_msg);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut buf = panic_msg_for_hook.lock().unwrap();
        *buf = format!("{info}");
    }));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_dispatch_inner(input, with_nouns)
    }));

    std::panic::set_hook(prev_hook);

    match result {
        Ok(json) => json,
        Err(_) => {
            let msg = panic_msg.lock().unwrap().clone();
            error_array(if msg.is_empty() { "parser panicked (no message captured)" } else { &msg })
        }
    }
}

fn parse_dispatch_inner(input: &str, with_nouns: bool) -> String {
    let (markdown, domain, _nouns_json) = match parse_input_envelope(input) {
        Ok(x) => x,
        Err(e) => return error_array(&e),
    };
    let _ = with_nouns;

    // wasm32 path: skip parse_to_state and use the regex fallback.
    // The full Stage-2 parser hits an LLVM-emitted `unreachable`
    // somewhere in `cached_grammar_state` (the bootstrap that
    // compiles `forml2-grammar.md` into derivation Funcs and runs
    // them to fixpoint) — even on empty input, before any user
    // markdown is touched. wasm32-unknown-unknown is panic="abort"
    // (forced by rustc — no unwind ABI on this target), so the
    // trap propagates as `RuntimeError: unreachable` with no
    // useful diagnostic. Tracked under the #588 lift.
    //
    // The fallback recognises the two readings the seed pipeline
    // actually needs to materialise for cross-domain noun
    // resolution to work: noun declarations (`X is an entity type.`
    // / `X is a value type.`) and FORML 2 statements (any non-blank,
    // non-comment line terminated by `.`). It loses fact-type and
    // constraint extraction — those need real Stage-2 — but lets
    // the seed populate the registry's Noun + Reading cells, which
    // is what unblocks tier-N parses on the next pass.
    //
    // Native targets keep the full parser (the `cfg` flips the
    // implementation, not the public API).
    #[cfg(target_arch = "wasm32")]
    {
        return regex_fallback_parse(&markdown, &domain);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let parsed = if with_nouns {
            let nouns_json = match parse_input_envelope(input) {
                Ok((_, _, n)) => n,
                Err(_) => "{}".to_string(),
            };
            let context = synthetic_state_from_nouns(&nouns_json);
            crate::parse_forml2::parse_to_state_with_nouns(&markdown, &context)
        } else {
            crate::parse_forml2::parse_to_state(&markdown)
        };

        let state = match parsed {
            Ok(s) => s,
            Err(e) => return error_array(&e),
        };

        serialize_entities(&state, &domain)
    }
}

/// Regex-light noun + reading extractor — the wasm32 fallback path
/// when the full Stage-2 parser can't run. Pure string scanning, no
/// derivation engine, no grammar bootstrap. Returns the same
/// `[{id, type, domain, data}, ...]` shape `serialize_entities`
/// would produce, so the worker's `materializeBatch` doesn't care
/// which path produced it.
///
/// Patterns recognised:
///   * `Foo is an entity type.`       → Noun(name=Foo, objectType=entity)
///   * `Foo is a value type.`         → Noun(name=Foo, objectType=value)
///   * `Foo is an abstract entity.`   → Noun(name=Foo, objectType=abstract)
///   * Any non-blank non-`#` line ending in `.` → Reading(text=line)
///
/// The noun-name extraction is intentionally permissive: anything
/// before "is an entity type" / "is a value type" (trimmed, with
/// markdown markers stripped) becomes the name. Quoted forms
/// (`Foo 'Each Way Bet' is an entity type.`) are NOT handled — the
/// fallback misses those, which is acceptable since the bundled
/// readings that need them are baked into the metamodel and never
/// hit this path.
fn regex_fallback_parse(markdown: &str, domain: &str) -> String {
    let mut out = String::from("[");
    let mut first = true;
    let mut reading_index = 0usize;

    for raw in markdown.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        // FORML 2 statements always end with `.` (optionally
        // followed by an ORM 2 derivation marker `*` / `**` / `+`).
        // Skip prose lines that don't terminate this way — same
        // filter Stage-2 applies.
        let body = line.trim_end_matches(|c: char| matches!(c, '*' | '+' | ' '));
        if !body.ends_with('.') { continue; }
        let stmt = body.trim_end_matches('.').trim();
        if stmt.is_empty() { continue; }

        // Noun declaration shape — be lenient about determiner
        // ("an" / "a") and exact suffix.
        let noun_decl = noun_from_decl(stmt);
        if let Some((name, object_type)) = noun_decl {
            if !first { out.push(','); }
            first = false;
            push_entity(&mut out, domain, "Noun", &name, &[
                ("name", &name),
                ("objectType", object_type),
            ]);
            // Continue — also emit the line as a Reading so
            // downstream "what readings exist?" queries see it.
        }

        // Reading: every statement-terminator line.
        if !first { out.push(','); }
        first = false;
        let reading_id = format!("r{reading_index}");
        reading_index += 1;
        push_entity(&mut out, domain, "Reading", &reading_id, &[
            ("text", line),
        ]);
    }

    out.push(']');
    out
}

/// Recognise noun-declaration shape. Returns `(name, objectType)`
/// where objectType is one of "entity" / "value" / "abstract".
fn noun_from_decl(stmt: &str) -> Option<(String, &'static str)> {
    // Lowercase suffix scan for tolerance to leading capitalisation
    // and incidental whitespace.
    let lower = stmt.to_lowercase();
    let (object_type, suffix) = if lower.ends_with(" is an entity type") {
        ("entity", " is an entity type")
    } else if lower.ends_with(" is a entity type") {
        ("entity", " is a entity type")  // tolerant typo
    } else if lower.ends_with(" is a value type") {
        ("value", " is a value type")
    } else if lower.ends_with(" is an value type") {
        ("value", " is an value type")  // tolerant typo
    } else if lower.ends_with(" is an abstract entity type") {
        ("abstract", " is an abstract entity type")
    } else if lower.ends_with(" is abstract") {
        ("abstract", " is abstract")
    } else {
        return None;
    };

    // Cut the suffix off the original (case-preserving) string.
    let cut = stmt.len() - suffix.len();
    let name = stmt[..cut].trim().to_string();
    if name.is_empty() { return None; }
    Some((name, object_type))
}

/// Emit one `{id, type, domain, data}` entity as JSON. Inline so
/// the fallback doesn't need to build an intermediate Object tree.
fn push_entity(out: &mut String, domain: &str, type_name: &str, key: &str, fields: &[(&str, &str)]) {
    out.push('{');
    out.push_str("\"id\":");
    push_json_string(out, &format!("{domain}:{type_name}:{key}"));
    out.push_str(",\"type\":");
    push_json_string(out, type_name);
    out.push_str(",\"domain\":");
    push_json_string(out, domain);
    out.push_str(",\"data\":{");
    let mut first = true;
    let mut saw_domain = false;
    for (k, v) in fields {
        if !first { out.push(','); }
        first = false;
        push_json_string(out, k);
        out.push(':');
        push_json_string(out, v);
        if *k == "domain" { saw_domain = true; }
    }
    if !saw_domain {
        if !first { out.push(','); }
        out.push_str("\"domain\":");
        push_json_string(out, domain);
    }
    out.push_str("}}");
}

/// Pull `markdown`, `domain`, and (optionally) raw `nouns` JSON out
/// of the worker-supplied envelope. Tolerates either field being
/// absent — the parser handles empty markdown / domain gracefully and
/// the noun-context path treats missing `nouns` as an empty catalog.
fn parse_input_envelope(input: &str) -> Result<(String, String, String), String> {
    let v: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("invalid envelope JSON: {e}"))?;
    let markdown = v.get("markdown")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let domain = v.get("domain")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let nouns_json = v.get("nouns")
        .map(|x| x.to_string())
        .unwrap_or_else(|| "{}".to_string());
    Ok((markdown, domain, nouns_json))
}

/// Build a minimal `Object` whose `Noun` cell mirrors the worker-
/// supplied catalog. Stage-2's tokeniser only needs the noun *names*
/// (and their declared object-type) to resolve cross-domain
/// references; everything else (fact types, constraints) is
/// re-derived from the markdown being parsed.
fn synthetic_state_from_nouns(nouns_json: &str) -> Object {
    let v: serde_json::Value = match serde_json::from_str(nouns_json) {
        Ok(v) => v,
        Err(_) => return Object::phi(),
    };
    let map = match v.as_object() {
        Some(m) => m,
        None => return Object::phi(),
    };

    let mut noun_facts: Vec<Object> = Vec::with_capacity(map.len());
    for (name, def) in map {
        let object_type = def.as_object()
            .and_then(|o| o.get("objectType"))
            .and_then(|x| x.as_str())
            .unwrap_or("entity");
        noun_facts.push(crate::ast::fact_from_pairs(&[
            ("name", name.as_str()),
            ("objectType", object_type),
        ]));
    }

    // `Object::Map` wraps `hashbrown::HashMap`, not std's — the
    // crate switches to hashbrown unconditionally so the kernel
    // build can avoid linking std. Using std's HashMap here compiles
    // but fails type-equality at the constructor site.
    let mut cells: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
    cells.insert("Noun".to_string(), Object::Seq(noun_facts.into()));
    Object::Map(cells)
}

/// Walk every cell named in `CELLS` and emit one entity per fact.
/// Hand-built JSON serialisation: serde_json::to_string would need
/// us to either build a serde model first (extra clones) or write a
/// custom Serializer (overkill for a flat shape). Direct String
/// formatting is the smallest-blast-radius option.
fn serialize_entities(state: &Object, domain: &str) -> String {
    let mut out = String::from("[");
    let mut first = true;

    for (cell_name, type_name) in CELLS {
        let cell = fetch_or_phi(cell_name, state);
        let facts = match cell.as_seq() {
            Some(f) => f,
            None => continue,
        };

        for (idx, fact) in facts.iter().enumerate() {
            if !first { out.push(','); }
            first = false;

            let id = compute_entity_id(domain, type_name, fact, idx);
            let data_json = bindings_to_json(fact, domain);

            out.push('{');
            out.push_str("\"id\":");
            push_json_string(&mut out, &id);
            out.push_str(",\"type\":");
            push_json_string(&mut out, type_name);
            out.push_str(",\"domain\":");
            push_json_string(&mut out, domain);
            out.push_str(",\"data\":");
            out.push_str(&data_json);
            out.push('}');
        }
    }

    out.push(']');
    out
}

/// Stable, collision-resistant id per entity. Prefer a binding-driven
/// id (`name`, then `id`) so the worker's `materializeBatch` can
/// upsert the same EntityDB row across re-parses; fall back to a
/// type-scoped index when no obvious key exists (keeps the entity
/// distinct from siblings even if all bindings happen to match).
fn compute_entity_id(domain: &str, type_name: &str, fact: &Object, idx: usize) -> String {
    let key = binding(fact, "name")
        .or_else(|| binding(fact, "id"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{idx}"));
    format!("{domain}:{type_name}:{key}")
}

/// Flatten a fact's named-tuple bindings into `{key: value, ...}`
/// JSON. Always includes `domain` so worker-side filters that key on
/// `data.domain` (`indexNoun(name, slug)`) work without a per-cell
/// special case here.
fn bindings_to_json(fact: &Object, domain: &str) -> String {
    let mut out = String::from("{");
    let mut first = true;
    let mut saw_domain = false;

    if let Some(pairs) = fact.as_seq() {
        for pair in pairs {
            let items = match pair.as_seq() {
                Some(i) if i.len() == 2 => i,
                _ => continue,
            };
            let k = match items[0].as_atom() { Some(k) => k, None => continue };
            let v = match items[1].as_atom() { Some(v) => v, None => continue };

            if !first { out.push(','); }
            first = false;

            push_json_string(&mut out, k);
            out.push(':');
            push_json_string(&mut out, v);

            if k == "domain" { saw_domain = true; }
        }
    }

    if !saw_domain {
        if !first { out.push(','); }
        out.push_str("\"domain\":");
        push_json_string(&mut out, domain);
    }

    out.push('}');
    out
}

/// JSON string escape — covers the subset RFC 8259 requires for
/// strings produced by the parser (no control chars below  ,
/// quotes, backslashes). Inline rather than serde to avoid the extra
/// allocation per binding.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Single-element error array. Always valid JSON so the worker's
/// `JSON.parse(...)` succeeds and the message can flow into
/// `errors[]` with the file's slug attached.
fn error_array(msg: &str) -> String {
    let mut out = String::from("[{\"_error\":");
    push_json_string(&mut out, msg);
    out.push_str("}]");
    out
}

// ── Tests ─────────────────────────────────────────────────────────
//
// Run with: cargo test -p arest --features std-deps parse_intercept
//
// These cover the contract `src/api/parse.ts` depends on:
//
//   1. `system(0, "parse", ...)` returns a JSON array (not "⊥") for
//      well-formed input — the seed pipeline's first `JSON.parse(...)`
//      must not throw.
//   2. Each entity has the four-field shape {id, type, domain, data}
//      that `materializeBatch` writes to EntityDB.
//   3. Nouns flow through with `data.name` set, since the worker
//      indexes them via `registry.indexNoun(noun.data.name, slug)`.
//   4. `parse_with_nouns` honours the supplied catalog so tier-N
//      readings resolve nouns declared by tiers 1..N-1.
//   5. Malformed envelopes return a single-element error array (not
//      "⊥") so the seed surfaces the failure with the file's slug
//      instead of crashing on JSON.parse.

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> serde_json::Value {
        let raw = parse_dispatch(input, /* with_nouns */ false);
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse_dispatch did not return JSON: {e}\n  raw: {raw}"))
    }

    fn parse_with_nouns(input: &str) -> serde_json::Value {
        let raw = parse_dispatch(input, /* with_nouns */ true);
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse_dispatch did not return JSON: {e}\n  raw: {raw}"))
    }

    #[test]
    fn parse_returns_array_not_bottom_for_simple_noun_declaration() {
        let input = r#"{"markdown":"Customer is an entity type.","domain":"x"}"#;
        let v = parse(input);
        assert!(v.is_array(), "expected JSON array, got: {v}");
    }

    #[test]
    fn parse_emits_noun_with_name_binding() {
        let input = r#"{"markdown":"Customer is an entity type.\nOrder is an entity type.","domain":"shop"}"#;
        let v = parse(input);
        let arr = v.as_array().expect("array");
        let noun_names: Vec<&str> = arr.iter()
            .filter(|e| e["type"] == "Noun")
            .filter_map(|e| e["data"]["name"].as_str())
            .collect();
        assert!(noun_names.contains(&"Customer"),
            "expected Customer in nouns; got nouns: {noun_names:?}\n  full: {v}");
        assert!(noun_names.contains(&"Order"),
            "expected Order in nouns; got nouns: {noun_names:?}");
    }

    #[test]
    fn every_entity_has_four_canonical_fields() {
        let input = r#"{"markdown":"Customer is an entity type.","domain":"shop"}"#;
        let v = parse(input);
        for e in v.as_array().expect("array") {
            assert!(e["id"].is_string(),     "missing id: {e}");
            assert!(e["type"].is_string(),   "missing type: {e}");
            assert!(e["domain"].is_string(), "missing domain: {e}");
            assert!(e["data"].is_object(),   "missing data: {e}");
            assert_eq!(e["domain"], "shop",  "domain not propagated: {e}");
        }
    }

    #[test]
    fn entity_ids_include_domain_and_type_prefix() {
        let input = r#"{"markdown":"Customer is an entity type.","domain":"shop"}"#;
        let v = parse(input);
        let arr = v.as_array().expect("array");
        let noun = arr.iter()
            .find(|e| e["type"] == "Noun" && e["data"]["name"] == "Customer")
            .expect("Customer noun");
        // The worker upserts via this id; collisions across types
        // would silently overwrite siblings, so the type prefix is
        // load-bearing.
        assert_eq!(noun["id"], "shop:Noun:Customer");
    }

    #[test]
    fn parse_with_nouns_carries_supplied_catalog_through_resolution() {
        // Tier-N reading references a noun declared in tier-(N-1). Without
        // the noun catalog, stage-2 stage-1's tokeniser cannot classify
        // "Order" as a known noun and the FT containing it never appears.
        let input = r#"{
            "markdown":"Customer places Order.",
            "domain":"orders",
            "nouns":{"Customer":{"objectType":"entity"},"Order":{"objectType":"entity"}}
        }"#;
        let v = parse_with_nouns(input);
        let arr = v.as_array().expect("array");
        let has_ft = arr.iter().any(|e| e["type"] == "Fact Type");
        assert!(has_ft,
            "expected at least one Fact Type from cross-noun reading; got: {v}");
    }

    #[test]
    fn malformed_envelope_returns_error_array_not_bottom() {
        // Worker's `JSON.parse` must not throw on the engine's
        // response no matter what we hand it; a single-element error
        // array is the contract.
        let raw = parse_dispatch("not json at all", /* with_nouns */ false);
        let v: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("expected valid JSON even for bad input; got {e}\n  raw: {raw}"));
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert!(arr[0]["_error"].is_string(), "expected _error field; got: {v}");
    }

    #[test]
    fn empty_markdown_returns_empty_array_not_error() {
        // The seed pipeline batch-loads files; an empty file should
        // come back as a clean zero-entity result, not as an error
        // that pollutes the per-domain `errors[]` count.
        let input = r#"{"markdown":"","domain":"empty"}"#;
        let v = parse(input);
        let arr = v.as_array().expect("array");
        assert!(arr.is_empty() || arr.iter().all(|e| e["_error"].is_null()),
            "empty markdown should not error; got: {v}");
    }

    // ── Regex fallback (wasm32 path) tests ──────────────────────
    //
    // The fallback runs only on `target_arch = "wasm32"` in
    // production, but the test suite runs on the host. Call it
    // directly to verify the regex shape — the seed pipeline's
    // tier-N invocations depend on at least Nouns and Readings
    // flowing forward, and these tests guard that contract.

    fn fallback(markdown: &str, domain: &str) -> serde_json::Value {
        let raw = regex_fallback_parse(markdown, domain);
        serde_json::from_str(&raw).expect("fallback emits valid JSON")
    }

    #[test]
    fn fallback_extracts_entity_noun_from_canonical_declaration() {
        let v = fallback("Customer is an entity type.", "shop");
        let arr = v.as_array().expect("array");
        let nouns: Vec<&str> = arr.iter()
            .filter(|e| e["type"] == "Noun")
            .filter_map(|e| e["data"]["name"].as_str())
            .collect();
        assert_eq!(nouns, vec!["Customer"]);
        // Must also tag the line as a Reading so per-domain
        // reading counts on the worker stay nonzero.
        let readings: Vec<&str> = arr.iter()
            .filter(|e| e["type"] == "Reading")
            .filter_map(|e| e["data"]["text"].as_str())
            .collect();
        assert_eq!(readings.len(), 1);
        assert!(readings[0].contains("Customer is an entity type"));
    }

    #[test]
    fn fallback_distinguishes_value_from_entity_object_type() {
        let md = "Color is a value type.\nCustomer is an entity type.";
        let v = fallback(md, "shop");
        let by_name: std::collections::HashMap<&str, &str> = v.as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "Noun")
            .filter_map(|e| Some((e["data"]["name"].as_str()?, e["data"]["objectType"].as_str()?)))
            .collect();
        assert_eq!(by_name.get("Color"), Some(&"value"));
        assert_eq!(by_name.get("Customer"), Some(&"entity"));
    }

    #[test]
    fn fallback_skips_blank_and_comment_and_prose_lines() {
        // Markdown headings (`#`), blanks, and prose lines without
        // a `.` terminator must NOT be picked up as Readings — the
        // worker uses Reading count as a quality metric and we
        // don't want incidental section text inflating it.
        let md = "# Heading\n\nSome prose with no terminator\nCustomer is an entity type.";
        let v = fallback(md, "shop");
        let readings: Vec<&str> = v.as_array().unwrap().iter()
            .filter(|e| e["type"] == "Reading")
            .filter_map(|e| e["data"]["text"].as_str())
            .collect();
        assert_eq!(readings.len(), 1, "only the terminated line should be a Reading; got: {readings:?}");
    }

    #[test]
    fn fallback_id_format_matches_full_parser_convention() {
        // `materializeBatch` upserts on entity id; if the fallback
        // emits `id` that doesn't match the full parser's format,
        // tier-1 entities written by one path collide with tier-N
        // writes by the other. Both paths use
        // `${domain}:${Type}:${name|sequential}`.
        let v = fallback("Customer is an entity type.", "shop");
        let arr = v.as_array().expect("array");
        let noun = arr.iter().find(|e| e["type"] == "Noun").expect("noun");
        assert_eq!(noun["id"], "shop:Noun:Customer");
    }

    #[test]
    fn fallback_handles_terminator_with_derivation_marker() {
        // FORML 2 derivation rules end `. *` / `. **` / `. +`. The
        // fallback strips trailing markers so the line is still
        // recognised as a statement.
        let v = fallback("Customer has Email. *", "shop");
        let readings: Vec<&str> = v.as_array().unwrap().iter()
            .filter(|e| e["type"] == "Reading")
            .filter_map(|e| e["data"]["text"].as_str())
            .collect();
        assert_eq!(readings.len(), 1, "marker-terminated line should still be a Reading; got: {readings:?}");
    }

    #[test]
    fn fallback_through_parse_dispatch_returns_nouns_on_host_too() {
        // On wasm32 production builds `parse_dispatch_inner` calls
        // `regex_fallback_parse`. On the host (where this test
        // runs) it calls the full parser. This test exercises the
        // host path end-to-end as a smoke that the public entry
        // point still works after the fallback was wired in.
        let input = r#"{"markdown":"Customer is an entity type.","domain":"x"}"#;
        let v: serde_json::Value = serde_json::from_str(&parse_dispatch(input, false))
            .expect("valid JSON");
        let nouns: Vec<&str> = v.as_array().unwrap().iter()
            .filter(|e| e["type"] == "Noun")
            .filter_map(|e| e["data"]["name"].as_str())
            .collect();
        assert!(nouns.contains(&"Customer"), "expected Customer noun; got: {v}");
    }
}
