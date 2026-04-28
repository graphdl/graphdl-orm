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
    let (markdown, domain, nouns_json) = match parse_input_envelope(input) {
        Ok(x) => x,
        Err(e) => return error_array(&e),
    };

    let parsed = if with_nouns {
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
}
