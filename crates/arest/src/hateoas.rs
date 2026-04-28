// crates/arest/src/hateoas.rs
//
// Engine-less HATEOAS read fallback (#609 / arest-router parity).
//
// Mirror of the worker's `handleArestReadFallback`
// (`src/api/arest-router.ts`): given a state and an HTTP request,
// resolve `/arest/{slug}` → noun → entity-cell walk → JSON. No engine
// path required — pure cell-graph reads.
//
// Lives in the engine crate so the same body works under both the
// kernel (`no_std`, `--features no_std,core`) and any future std host
// caller. The kernel wraps this with smoltcp + `system::with_state`;
// the worker keeps its TS handler since it needs DurableObject I/O.

#[allow(unused_imports)]
use alloc::{format, string::{String, ToString}, vec::Vec};

use crate::ast::{binding, fetch_or_phi, Object};
use crate::naming::resolve_slug_to_noun;

/// Handle a HATEOAS read against `state`. Returns `Some(json_bytes)`
/// when the path matches `GET /arest/{slug}` or
/// `GET /arest/{slug}/{id}` and the slug resolves to a registered
/// noun; `None` otherwise (caller should 404 / fall through).
///
/// Single-entity shape (matches `EntityDB.get()` envelope):
///   `{"id":"...","type":"Noun","field":"value",...}`
///
/// Collection shape (matches the worker fallback's flatten):
///   `{"type":"Noun","docs":[{...},...],"totalDocs":N}`
///
/// The kernel state stores entities directly in their noun's cell
/// after the SYSTEM mutator (#451) — same ρ-application path the
/// engine uses; no separate EntityDB. So a list endpoint is just
/// `fetch_or_phi(noun, state)`, an entity is the seq element with
/// matching `id` binding.
pub fn handle_arest_read(state: &Object, method: &str, path: &str) -> Option<Vec<u8>> {
    if method != "GET" {
        return None;
    }
    let stripped = path.split('?').next().unwrap_or(path);
    let trimmed = stripped.strip_prefix("/arest/")?.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.splitn(2, '/');
    let slug = parts.next()?;
    let id = parts.next();
    if slug.is_empty() {
        return None;
    }

    let noun = resolve_slug_to_noun(state, slug)?;
    let cell = fetch_or_phi(&noun, state);

    match id {
        Some(id) if !id.is_empty() => {
            let entity = cell.as_seq()?.iter().find(|e| binding(e, "id") == Some(id))?;
            Some(entity_to_json(&noun, entity).into_bytes())
        }
        _ => Some(collection_to_json(&noun, &cell).into_bytes()),
    }
}

/// Serialise a single entity fact (a named-tuple of `<key, value>`
/// pairs) to JSON, flattening the bindings onto the top level and
/// guaranteeing `id` and `type` come first.
fn entity_to_json(noun: &str, entity: &Object) -> String {
    let mut out = String::new();
    out.push('{');

    // Lead with id + type so consumers can switch on them without
    // scanning the body. Mirror of the worker's
    // `{ id, type, ...data }` flatten.
    let id = binding(entity, "id");
    out.push_str("\"id\":");
    out.push_str(&json_string(id.unwrap_or("")));
    out.push_str(",\"type\":");
    out.push_str(&json_string(noun));

    if let Some(pairs) = entity.as_seq() {
        for pair in pairs {
            let items = match pair.as_seq() {
                Some(p) if p.len() == 2 => p,
                _ => continue,
            };
            let (k, v) = match (items[0].as_atom(), items[1].as_atom()) {
                (Some(k), Some(v)) => (k, v),
                _ => continue,
            };
            // id was already emitted; type is an out-of-band label.
            if k == "id" || k == "type" {
                continue;
            }
            out.push(',');
            out.push_str(&json_string(k));
            out.push(':');
            out.push_str(&json_string(v));
        }
    }
    out.push('}');
    out
}

/// Serialise a collection cell as `{type, docs, totalDocs}`. An empty
/// or missing cell still returns a well-formed envelope with
/// `totalDocs: 0` — matches the worker fallback.
fn collection_to_json(noun: &str, cell: &Object) -> String {
    let docs: Vec<String> = cell
        .as_seq()
        .map(|seq| seq.iter().map(|e| entity_to_json(noun, e)).collect())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str("{\"type\":");
    out.push_str(&json_string(noun));
    out.push_str(",\"docs\":[");
    for (i, doc) in docs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(doc);
    }
    out.push_str("],\"totalDocs\":");
    out.push_str(&format!("{}", docs.len()));
    out.push('}');
    out
}

/// Quote and escape a string per RFC 8259 §7. Hand-rolled because the
/// kernel target is `no_std` and `serde_json` isn't reachable there;
/// the surface area we need (entity field values + noun names) is
/// small enough that an inlined encoder is simpler than wiring an
/// optional serde_json dep through the kernel build.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell_push, Object};

    fn fact(pairs: &[(&str, &str)]) -> Object {
        Object::seq(
            pairs
                .iter()
                .map(|(k, v)| Object::seq(alloc::vec![Object::atom(k), Object::atom(v)]))
                .collect(),
        )
    }

    fn state_with_org(id: &str, name: &str) -> Object {
        let noun_decl = fact(&[("name", "Organization")]);
        let org = fact(&[("id", id), ("name", name)]);
        let s = cell_push("Noun", noun_decl, &Object::phi());
        cell_push("Organization", org, &s)
    }

    #[test]
    fn rejects_non_get() {
        let s = state_with_org("acme", "Acme");
        assert!(handle_arest_read(&s, "POST", "/arest/organizations").is_none());
        assert!(handle_arest_read(&s, "DELETE", "/arest/organizations/acme").is_none());
    }

    #[test]
    fn rejects_non_arest_paths() {
        let s = state_with_org("acme", "Acme");
        assert!(handle_arest_read(&s, "GET", "/api/welcome").is_none());
        assert!(handle_arest_read(&s, "GET", "/").is_none());
        assert!(handle_arest_read(&s, "GET", "/arest").is_none());
        assert!(handle_arest_read(&s, "GET", "/arest/").is_none());
    }

    #[test]
    fn unknown_slug_returns_none() {
        let s = state_with_org("acme", "Acme");
        assert!(handle_arest_read(&s, "GET", "/arest/widgets").is_none());
    }

    #[test]
    fn collection_emits_docs_plus_totalDocs() {
        let s = state_with_org("acme", "Acme Corp");
        let body = handle_arest_read(&s, "GET", "/arest/organizations").expect("matched");
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("\"type\":\"Organization\""), "{body}");
        assert!(body.contains("\"totalDocs\":1"), "{body}");
        assert!(body.contains("\"id\":\"acme\""), "{body}");
        assert!(body.contains("\"name\":\"Acme Corp\""), "{body}");
    }

    #[test]
    fn empty_cell_returns_zero_totalDocs() {
        // Noun is registered, but no entities exist — fallback should
        // still emit a valid envelope with empty docs.
        let noun_decl = fact(&[("name", "Organization")]);
        let s = cell_push("Noun", noun_decl, &Object::phi());
        let body = handle_arest_read(&s, "GET", "/arest/organizations").expect("matched");
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("\"docs\":[]"), "{body}");
        assert!(body.contains("\"totalDocs\":0"), "{body}");
    }

    #[test]
    fn single_entity_flattens_bindings() {
        let s = state_with_org("acme", "Acme Corp");
        let body = handle_arest_read(&s, "GET", "/arest/organizations/acme").expect("matched");
        let body = String::from_utf8(body).unwrap();
        assert!(body.starts_with("{\"id\":\"acme\",\"type\":\"Organization\""), "{body}");
        assert!(body.contains("\"name\":\"Acme Corp\""), "{body}");
    }

    #[test]
    fn missing_entity_id_returns_none() {
        let s = state_with_org("acme", "Acme");
        assert!(handle_arest_read(&s, "GET", "/arest/organizations/missing").is_none());
    }

    #[test]
    fn query_string_is_stripped() {
        let s = state_with_org("acme", "Acme");
        let body = handle_arest_read(&s, "GET", "/arest/organizations?limit=10").expect("matched");
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("\"type\":\"Organization\""), "{body}");
    }

    #[test]
    fn json_string_escapes_specials() {
        assert_eq!(json_string("hello"), "\"hello\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string("a\tb"), "\"a\\tb\"");
    }
}
