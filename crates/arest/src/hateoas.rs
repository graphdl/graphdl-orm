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

/// Handle a HATEOAS write — `POST /arest/entities/{slug}` with a body
/// of `{"id":"...","data":{...}}` — and return the new state plus the
/// JSON envelope the worker's `EntityDB.create` returns
/// (`{id, type, ...data}`). Returns `None` for non-matching paths,
/// non-POST methods, unknown slugs, malformed JSON, or missing `id`.
///
/// Direct-write fallback only — the engine path (validate / derive /
/// apply via `system::apply`) lands once #588 lifts Stage-2 to no_std.
/// Until then this is the only POST path the kernel honours, mirror
/// of the worker's `router.ts::handleEntitiesPost` create-side
/// fallback.
///
/// Caller is responsible for committing the returned state via
/// `system::apply` and emitting the response bytes back over HTTP.
/// `handle_arest_read` afterwards sees the new entity in the same
/// `Noun`'s cell — reads through the same projection path.
pub fn handle_arest_create_for_slug(
    state: &Object,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<(Object, Vec<u8>)> {
    if method != "POST" {
        return None;
    }
    let stripped = path.split('?').next().unwrap_or(path);
    let trimmed = stripped.strip_prefix("/arest/entities/")?.trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains('/') {
        return None;
    }
    // Path-encoded slugs may carry `%20` for spaces (e.g.
    // `Support%20Request` — see `apis/__e2e__/arest.test.ts:240`).
    // We accept either the percent-encoded or already-decoded form.
    let slug_raw = trimmed;
    let slug_decoded = percent_decode(slug_raw);
    // The worker accepts the *noun name* directly in this path
    // (e.g. `/arest/entities/Organization`), not the kebab-pluralised
    // slug. Try noun-name match first; fall back to slug-projection
    // resolution to keep the kernel forgiving.
    let noun = if crate::ast::fetch_or_phi("Noun", state)
        .as_seq()
        .map(|ns| ns.iter().any(|n| crate::ast::binding(n, "name") == Some(&slug_decoded)))
        .unwrap_or(false)
    {
        slug_decoded.clone()
    } else {
        crate::naming::resolve_slug_to_noun(state, slug_raw)
            .or_else(|| crate::naming::resolve_slug_to_noun(state, &slug_decoded))?
    };

    let parsed = crate::json_min::parse(body)?;
    let id = parsed.get("id").and_then(|v| v.as_str())?;
    if id.is_empty() {
        return None;
    }
    let data = parsed.get("data");

    // Build the named-tuple fact: <<id, ID>, <key1, val1>, …>.
    // String values flatten directly; nested objects / arrays / null /
    // booleans are skipped — the engine can't represent them as
    // role values today and the worker's direct-write fallback skips
    // them too. (Floating-point numbers survive as their lexed atom
    // string since `Atom` is opaque to the engine layer.)
    let mut pairs: Vec<Object> = Vec::new();
    pairs.push(Object::seq(alloc::vec![Object::atom("id"), Object::atom(id)]));
    if let Some(JsonValue::Object(fields)) = data {
        for (k, v) in fields {
            if k == "id" {
                continue;
            }
            let val = match v {
                JsonValue::Str(s) => s.as_str(),
                JsonValue::Num(n) => n.as_str(),
                JsonValue::Bool(true) => "T",
                JsonValue::Bool(false) => "F",
                _ => continue,
            };
            pairs.push(Object::seq(alloc::vec![Object::atom(k), Object::atom(val)]));
        }
    }
    let entity = Object::seq(pairs);

    let new_state = crate::ast::cell_push(&noun, entity.clone(), state);
    let response_body = entity_to_json(&noun, &entity).into_bytes();
    Some((new_state, response_body))
}

/// Handle `POST /arest/entity` with a body of
/// `{"noun":"Organization","domain":"organizations","fields":{...}}`.
/// Mirror of the worker's AREST-command create path
/// (`router.ts::handleArestRoute` POST branch). The kernel today
/// can't run the engine path (gated on #588), so this is the
/// direct-write fallback only — same shape `handle_arest_create_for_slug`
/// uses, but with the noun read from the body and a random id
/// (`arest::csprng::random_bytes`) since the request doesn't supply one.
///
/// Returns `None` for non-matching paths, non-POST methods, malformed
/// JSON, missing/unknown noun. Caller commits the new state via
/// `system::apply` and emits the response bytes.
pub fn handle_arest_create(
    state: &Object,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<(Object, Vec<u8>)> {
    if method != "POST" {
        return None;
    }
    let stripped = path.split('?').next().unwrap_or(path);
    if stripped != "/arest/entity" {
        return None;
    }

    let parsed = crate::json_min::parse(body)?;
    let noun_raw = parsed.get("noun").and_then(|v| v.as_str())?;
    if noun_raw.is_empty() {
        return None;
    }
    // Verify the noun is registered. Mirror of the slug resolver's
    // safety net — unknown nouns 404 rather than silently creating
    // a stray cell.
    let noun_registered = crate::ast::fetch_or_phi("Noun", state)
        .as_seq()
        .map(|ns| ns.iter().any(|n| crate::ast::binding(n, "name") == Some(noun_raw)))
        .unwrap_or(false);
    if !noun_registered {
        return None;
    }
    let noun = noun_raw.to_string();

    // Generate an id. The worker uses `crypto.randomUUID()`; the
    // kernel can't always reach a hardware entropy source (QEMU
    // commonly hides RDSEED/RDRAND, and `csprng::random_bytes`
    // panics on reseed failure — #571 tracks the EFI_RNG_PROTOCOL
    // fallback). Until that lands, we generate a stable opaque id
    // from a per-process atomic counter + an FNV-1a hash of the
    // request body. Counter ensures uniqueness across requests in
    // the same boot; FNV-hash makes it opaque so callers can't
    // predict the next id from request shape alone. This is the
    // direct-write fallback's id strategy — the engine path will
    // route through `arest::naming::resolve_entity_id` once #588
    // unblocks the no_std Stage-2 parser.
    let counter = NEXT_ENTITY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let body_hash = fnv1a_64(body);
    let id = alloc::format!("k{:08x}{:016x}", counter, body_hash);

    // Same flatten policy as `handle_arest_create_for_slug`.
    let mut pairs: Vec<Object> = Vec::new();
    pairs.push(Object::seq(alloc::vec![Object::atom("id"), Object::atom(&id)]));
    if let Some(JsonValue::Object(fields)) = parsed.get("fields") {
        for (k, v) in fields {
            if k == "id" {
                continue;
            }
            let val = match v {
                JsonValue::Str(s) => s.as_str(),
                JsonValue::Num(n) => n.as_str(),
                JsonValue::Bool(true) => "T",
                JsonValue::Bool(false) => "F",
                _ => continue,
            };
            pairs.push(Object::seq(alloc::vec![Object::atom(k), Object::atom(val)]));
        }
    }
    let entity = Object::seq(pairs);

    let new_state = crate::ast::cell_push(&noun, entity.clone(), state);
    let response_body = entity_to_json(&noun, &entity).into_bytes();
    Some((new_state, response_body))
}

/// Handle a `POST /arest/entities/{slug}/{id}/transition` — fire a
/// state-machine transition event. Mirror of the worker's
/// `router.ts::POST /api/entities/:noun/:id/transition` (line 617),
/// minus the engine path: the kernel direct-write fallback walks the
/// `State Machine` cell (entity rows with `forResource` +
/// `currentlyInStatus` bindings, mirror of the worker's DurableObject
/// shape) to read current status, walks the `Transition` cell to find
/// a matching `(fromStatus, event)` row, then rewrites the SM row's
/// `currentlyInStatus` and pushes a new `Event` entity.
///
/// Returns `(new_state, response_bytes)` on success — caller commits
/// via `system::apply` and emits the body. The response envelope
/// mirrors the worker's:
///
///   `{"id":"...","noun":"...","previousStatus":"...","status":"...","event":"..."}`
///
/// `None` when:
///   * Method isn't POST or the path doesn't end in `/transition`.
///   * The slug doesn't resolve to a registered noun.
///   * The body isn't valid JSON or `event` is missing/empty.
///   * No State Machine row has `forResource == id`.
///   * No Transition row matches `(currentStatus, event)`.
///
/// The kernel HTTP handler maps `None` to `404`/`400` per the worker's
/// error envelope (`router.ts:646-671`); the engine path lands once
/// #588 lifts Stage-2 to no_std and the kernel can compile readings
/// at runtime.
pub fn handle_arest_transition(
    state: &Object,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<(Object, Vec<u8>)> {
    if method != "POST" {
        return None;
    }
    let stripped = path.split('?').next().unwrap_or(path);
    let inner = stripped.strip_prefix("/arest/entities/")?.trim_end_matches('/');
    let inner = inner.strip_suffix("/transition")?.trim_end_matches('/');
    if inner.is_empty() {
        return None;
    }

    // Split into `{slug}/{id}` — id may itself contain percent-encoded
    // characters (UUIDs don't, but the apis e2e suite encodes them
    // anyway via `encodeURIComponent`). Reject paths with extra
    // segments so e.g. `/arest/entities/X/Y/Z/transition` doesn't
    // silently treat `Y/Z` as the id.
    let mut parts = inner.splitn(2, '/');
    let slug = parts.next()?;
    let id_raw = parts.next()?;
    if slug.is_empty() || id_raw.is_empty() || id_raw.contains('/') {
        return None;
    }
    let id = percent_decode(id_raw);

    // Resolve slug → noun. Same dual-lookup as
    // `handle_arest_create_for_slug` (noun-name match first, then
    // kebab-pluralised slug projection).
    let slug_decoded = percent_decode(slug);
    let noun = if crate::ast::fetch_or_phi("Noun", state)
        .as_seq()
        .map(|ns| ns.iter().any(|n| crate::ast::binding(n, "name") == Some(&slug_decoded)))
        .unwrap_or(false)
    {
        slug_decoded.clone()
    } else {
        crate::naming::resolve_slug_to_noun(state, slug)
            .or_else(|| crate::naming::resolve_slug_to_noun(state, &slug_decoded))?
    };

    // Body shape mirror of router.ts:620 — `{event, domain?}`.
    let parsed = crate::json_min::parse(body)?;
    let event = parsed.get("event").and_then(|v| v.as_str())?;
    if event.is_empty() {
        return None;
    }

    // Walk State Machine cell to find the row whose `forResource`
    // matches `id`. We need both the row (to update) and its index
    // (to swap in the rewritten copy).
    let sm_cell = crate::ast::fetch_or_phi("State Machine", state);
    let sm_seq = sm_cell.as_seq()?;
    let (sm_idx, sm_row) = sm_seq
        .iter()
        .enumerate()
        .find(|(_, sm)| crate::ast::binding(sm, "forResource") == Some(id.as_str()))?;
    let current_status = crate::ast::binding(sm_row, "currentlyInStatus")?.to_string();

    // Walk Transition cell for a row matching `(fromStatus, event)`.
    // The worker's engine path additionally scopes by
    // `forStateMachineDefinition`; the kernel direct-write fallback
    // assumes one SM definition per noun (the apis e2e fixture
    // `Support Request → Categorize` is the only flow exercised
    // today), so the scoping reduces to (status, event).
    let transitions_cell = crate::ast::fetch_or_phi("Transition", state);
    let new_status = transitions_cell
        .as_seq()?
        .iter()
        .find(|t| {
            crate::ast::binding(t, "fromStatus") == Some(current_status.as_str())
                && crate::ast::binding(t, "event") == Some(event)
        })
        .and_then(|t| crate::ast::binding(t, "toStatus"))?
        .to_string();

    // Rebuild State Machine cell with the matched row's
    // `currentlyInStatus` updated. `cell_push` only appends, so we
    // hand-build the new seq and `store` it whole.
    let new_sm_row = update_binding(sm_row, "currentlyInStatus", &new_status);
    let mut new_sm_vec: Vec<Object> = sm_seq.to_vec();
    new_sm_vec[sm_idx] = new_sm_row;
    let new_state = crate::ast::store("State Machine", Object::seq(new_sm_vec), state);

    // Push a new `Event` entity recording the transition. Mirror of
    // the worker's `arestResult.entities` Event persistence
    // (router.ts:680-684). `forResource`/`type` are the cross-cutting
    // role bindings the OpenAPI introspection (#148) surfaces.
    let event_counter = NEXT_ENTITY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let event_id = alloc::format!("evt-{:08x}", event_counter);
    let event_entity = Object::seq(alloc::vec![
        Object::seq(alloc::vec![Object::atom("id"), Object::atom(&event_id)]),
        Object::seq(alloc::vec![Object::atom("forResource"), Object::atom(&id)]),
        Object::seq(alloc::vec![Object::atom("type"), Object::atom(event)]),
    ]);
    let new_state = crate::ast::cell_push("Event", event_entity, &new_state);

    // Build the response envelope — order keys to match the worker's
    // `json({ id, noun, previousStatus, status, event, transitions })`
    // (router.ts:686). `transitions` is omitted today because the
    // direct-write fallback doesn't compute the legal next-step set
    // (the engine path's job once #588 lands).
    let mut out = String::new();
    out.push('{');
    out.push_str("\"id\":");
    out.push_str(&json_string(&id));
    out.push_str(",\"noun\":");
    out.push_str(&json_string(&noun));
    out.push_str(",\"previousStatus\":");
    out.push_str(&json_string(&current_status));
    out.push_str(",\"status\":");
    out.push_str(&json_string(&new_status));
    out.push_str(",\"event\":");
    out.push_str(&json_string(event));
    out.push('}');

    Some((new_state, out.into_bytes()))
}

/// Replace (or append) the value at `key` in a named-tuple `entity`
/// (`Seq([Seq([k,v]), ...])`). Used by the transition handler to
/// rewrite an SM row's `currentlyInStatus` without disturbing other
/// fields. Lives here rather than in `ast` because the named-tuple
/// shape is a hateoas convention, not an engine primitive.
fn update_binding(entity: &Object, key: &str, new_value: &str) -> Object {
    let mut out: Vec<Object> = Vec::new();
    let mut updated = false;
    if let Some(pairs) = entity.as_seq() {
        for pair in pairs {
            if let Some(items) = pair.as_seq() {
                if items.len() == 2 && items[0].as_atom() == Some(key) {
                    out.push(Object::seq(alloc::vec![
                        Object::atom(key),
                        Object::atom(new_value),
                    ]));
                    updated = true;
                    continue;
                }
            }
            out.push(pair.clone());
        }
    }
    if !updated {
        out.push(Object::seq(alloc::vec![
            Object::atom(key),
            Object::atom(new_value),
        ]));
    }
    Object::seq(out)
}

use core::sync::atomic::{AtomicU32, Ordering};

/// Per-process counter for entity-id generation in the direct-write
/// fallback. Reset across kernel boots; ids are unique within a single
/// kernel lifetime, not globally. Once the engine path lands the
/// engine's reference-scheme resolver replaces this.
static NEXT_ENTITY_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 64-bit FNV-1a hash. Good enough to make the id opaque to callers
/// (no preimage from request shape); not a cryptographic hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Decode `%20` and `%XX` percent-escapes in a URL path segment.
/// The kernel's HTTP path arrives raw — we decode it lazily here so
/// `Support%20Request` round-trips to `Support Request` for noun
/// lookup. Hand-rolled instead of pulling `percent-encoding` since
/// the kernel build is no_std.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            match (hi, lo) {
                (Some(hi), Some(lo)) => {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

use crate::json_min::JsonValue;

/// Aggregate parser/registry counts from `state` into the JSON shape
/// the worker's `GET /arest/parse` returns
/// (`src/api/parse.ts::handleParseGet`):
///
/// ```json
/// {
///   "totals": {
///     "domains": N,
///     "nouns": N,
///     "readings": N,
///     "factTypes": N,
///     "constraints": N
///   },
///   "perDomain": { "<slug>": { "nouns": N, "readings": N }, ... }
/// }
/// ```
///
/// Same envelope across both deployment targets so the apis e2e suite
/// (`apis/__e2e__/arest.test.ts`'s `GET /arest/parse returns engine
/// stats`) passes against either.
///
/// On the kernel today `perDomain` is always `{}` because the kernel
/// has no Domain registry yet (#205-style multi-tenant DO sharding
/// is worker-only). `domains` therefore counts the `Domain` cell if
/// it exists, otherwise `0`. The aggregate `nouns`/`readings`/etc.
/// fields walk the live SYSTEM cells directly — same source the
/// engine uses (#150 cell-indexed lookup), so the count is whatever
/// is loaded right now (baked metamodel + any runtime
/// `LoadReading` (#555) results).
pub fn parse_stats(state: &Object) -> Vec<u8> {
    let nouns = cell_count(state, "Noun");
    let readings = cell_count(state, "Reading");
    // Two casing conventions exist in the wild — `FactType` (one
    // word, internal cell name) and `Fact Type` (two words, the
    // canonical noun spelled in readings). Sum both so a state
    // produced by either spelling reports the same count.
    let fact_types = cell_count(state, "FactType") + cell_count(state, "Fact Type");
    let constraints = cell_count(state, "Constraint");
    let domains = cell_count(state, "Domain");

    let mut out = String::new();
    out.push_str("{\"totals\":{");
    out.push_str(&format!("\"domains\":{}", domains));
    out.push_str(&format!(",\"nouns\":{}", nouns));
    out.push_str(&format!(",\"readings\":{}", readings));
    out.push_str(&format!(",\"factTypes\":{}", fact_types));
    out.push_str(&format!(",\"constraints\":{}", constraints));
    out.push_str("},\"perDomain\":{}}");
    out.into_bytes()
}

fn cell_count(state: &Object, name: &str) -> usize {
    crate::ast::fetch_or_phi(name, state)
        .as_seq()
        .map(|s| s.len())
        .unwrap_or(0)
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
    fn parse_stats_empty_state() {
        let body = String::from_utf8(parse_stats(&Object::phi())).unwrap();
        assert!(body.contains("\"domains\":0"), "{body}");
        assert!(body.contains("\"nouns\":0"), "{body}");
        assert!(body.contains("\"readings\":0"), "{body}");
        assert!(body.contains("\"factTypes\":0"), "{body}");
        assert!(body.contains("\"constraints\":0"), "{body}");
        assert!(body.contains("\"perDomain\":{}"), "{body}");
    }

    #[test]
    fn parse_stats_counts_each_cell() {
        let s = Object::phi();
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &s);
        let s = cell_push("Noun", fact(&[("name", "OrgMembership")]), &s);
        let s = cell_push("Reading", fact(&[("text", "alpha")]), &s);
        let s = cell_push("FactType", fact(&[("id", "ft1")]), &s);
        let s = cell_push("Fact Type", fact(&[("id", "ft2")]), &s);
        let s = cell_push("Constraint", fact(&[("id", "c1")]), &s);
        let s = cell_push("Domain", fact(&[("slug", "auto.dev")]), &s);

        let body = String::from_utf8(parse_stats(&s)).unwrap();
        assert!(body.contains("\"nouns\":2"), "{body}");
        assert!(body.contains("\"readings\":1"), "{body}");
        // Sum across both casings — `FactType` (1) + `Fact Type` (1) = 2.
        assert!(body.contains("\"factTypes\":2"), "{body}");
        assert!(body.contains("\"constraints\":1"), "{body}");
        assert!(body.contains("\"domains\":1"), "{body}");
    }

    #[test]
    fn parse_stats_envelope_has_totals_field() {
        // The apis e2e probe (`expect(body.totals).toBeDefined()` +
        // `expect(typeof body.totals.domains).toBe('number')`) is the
        // contract this test pins.
        let body = String::from_utf8(parse_stats(&Object::phi())).unwrap();
        assert!(body.starts_with("{\"totals\":{"), "{body}");
    }

    #[test]
    fn create_for_slug_writes_entity_into_noun_cell() {
        // Hand-stage a Noun cell with Organization registered so the
        // resolver succeeds, then POST a body and verify both the
        // new state contains the entity and the response envelope
        // matches `EntityDB.create`'s `{id, type, ...data}`.
        let s = Object::phi();
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &s);

        let body = br#"{"id":"acme","data":{"name":"Acme Corp","orgSlug":"acme"}}"#;
        let (new_state, resp) = handle_arest_create_for_slug(
            &s, "POST", "/arest/entities/Organization", body,
        ).expect("create must succeed");

        let resp = String::from_utf8(resp).unwrap();
        assert!(resp.contains("\"id\":\"acme\""), "{resp}");
        assert!(resp.contains("\"type\":\"Organization\""), "{resp}");
        assert!(resp.contains("\"name\":\"Acme Corp\""), "{resp}");
        assert!(resp.contains("\"orgSlug\":\"acme\""), "{resp}");

        // Round-trip: the new state should now serve a list with one
        // entity through the read fallback.
        let list = handle_arest_read(&new_state, "GET", "/arest/organizations").expect("list");
        let list = String::from_utf8(list).unwrap();
        assert!(list.contains("\"totalDocs\":1"), "{list}");
        assert!(list.contains("\"id\":\"acme\""), "{list}");
    }

    #[test]
    fn create_for_slug_accepts_percent_encoded_noun() {
        // `/arest/entities/Support%20Request` is the path the apis
        // e2e suite uses (`encodeURIComponent('Support Request')`).
        let s = Object::phi();
        let s = cell_push("Noun", fact(&[("name", "Support Request")]), &s);
        let body = br#"{"id":"sr-1","data":{"title":"alpha"}}"#;
        let (_, resp) = handle_arest_create_for_slug(
            &s, "POST", "/arest/entities/Support%20Request", body,
        ).expect("percent-encoded noun must resolve");
        let resp = String::from_utf8(resp).unwrap();
        assert!(resp.contains("\"type\":\"Support Request\""), "{resp}");
    }

    #[test]
    fn create_for_slug_rejects_non_post() {
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &Object::phi());
        let body = br#"{"id":"acme","data":{}}"#;
        assert!(handle_arest_create_for_slug(&s, "GET", "/arest/entities/Organization", body).is_none());
        assert!(handle_arest_create_for_slug(&s, "DELETE", "/arest/entities/Organization", body).is_none());
    }

    #[test]
    fn create_for_slug_rejects_unknown_noun() {
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &Object::phi());
        let body = br#"{"id":"x","data":{}}"#;
        assert!(handle_arest_create_for_slug(&s, "POST", "/arest/entities/Widget", body).is_none());
    }

    #[test]
    fn create_for_slug_rejects_missing_id() {
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &Object::phi());
        let body = br#"{"data":{"name":"acme"}}"#;
        assert!(handle_arest_create_for_slug(&s, "POST", "/arest/entities/Organization", body).is_none());
    }

    #[test]
    fn create_for_slug_rejects_malformed_json() {
        let s = cell_push("Noun", fact(&[("name", "Organization")]), &Object::phi());
        let body = br#"{not valid json"#;
        assert!(handle_arest_create_for_slug(&s, "POST", "/arest/entities/Organization", body).is_none());
    }

    #[test]
    fn json_string_escapes_specials() {
        assert_eq!(json_string("hello"), "\"hello\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string("a\tb"), "\"a\\tb\"");
    }

    /// Fixture: a state seeded with a Support Request entity, a State
    /// Machine row pointing at it (currentlyInStatus = "Received"),
    /// and a Transition row that turns "Received" + "categorize" into
    /// "Categorized". Mirror of the apis e2e fixture
    /// (`apis/__e2e__/arest.test.ts:286`) the kernel direct-write
    /// fallback is the kernel-side counterpart for.
    fn state_with_sr_state_machine(sr_id: &str) -> Object {
        let s = Object::phi();
        let s = cell_push("Noun", fact(&[("name", "Support Request")]), &s);
        let s = cell_push("Noun", fact(&[("name", "State Machine")]), &s);
        let s = cell_push("Noun", fact(&[("name", "Transition")]), &s);
        let s = cell_push(
            "Support Request",
            fact(&[("id", sr_id), ("title", "alpha")]),
            &s,
        );
        let s = cell_push(
            "State Machine",
            fact(&[
                ("id", "sm-1"),
                ("forResource", sr_id),
                ("currentlyInStatus", "Received"),
            ]),
            &s,
        );
        cell_push(
            "Transition",
            fact(&[
                ("id", "t-1"),
                ("fromStatus", "Received"),
                ("toStatus", "Categorized"),
                ("event", "categorize"),
            ]),
            &s,
        )
    }

    #[test]
    fn transition_fires_categorize_event() {
        let s = state_with_sr_state_machine("sr-1");
        let body = br#"{"event":"categorize","domain":"support"}"#;
        let (new_state, resp) = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/support-requests/sr-1/transition",
            body,
        )
        .expect("transition must succeed");

        let resp = String::from_utf8(resp).unwrap();
        assert!(resp.contains("\"id\":\"sr-1\""), "{resp}");
        assert!(resp.contains("\"previousStatus\":\"Received\""), "{resp}");
        assert!(resp.contains("\"status\":\"Categorized\""), "{resp}");
        assert!(resp.contains("\"event\":\"categorize\""), "{resp}");

        // SM row's currentlyInStatus is now Categorized — and the row
        // is the *same* row (no append-by-mistake).
        let sm = crate::ast::fetch_or_phi("State Machine", &new_state);
        let sm_seq = sm.as_seq().expect("sm cell present");
        assert_eq!(sm_seq.len(), 1, "SM cell must not gain a duplicate row");
        assert_eq!(
            crate::ast::binding(&sm_seq[0], "currentlyInStatus"),
            Some("Categorized")
        );

        // Event row landed on the Event cell.
        let events = crate::ast::fetch_or_phi("Event", &new_state);
        let events_seq = events.as_seq().expect("event cell present");
        assert_eq!(events_seq.len(), 1);
        assert_eq!(crate::ast::binding(&events_seq[0], "type"), Some("categorize"));
        assert_eq!(crate::ast::binding(&events_seq[0], "forResource"), Some("sr-1"));
    }

    #[test]
    fn transition_accepts_kebab_pluralised_slug() {
        // The worker's openapi/UI paths emit kebab-pluralised slugs
        // (`/arest/entities/support-requests/...`). The kernel
        // resolver must accept both forms (matching
        // `handle_arest_create_for_slug`'s dual lookup).
        let s = state_with_sr_state_machine("sr-2");
        let body = br#"{"event":"categorize"}"#;
        let result = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/support-requests/sr-2/transition",
            body,
        );
        assert!(result.is_some(), "kebab-pluralised slug must resolve");
    }

    #[test]
    fn transition_rejects_non_post() {
        let s = state_with_sr_state_machine("sr-1");
        let body = br#"{"event":"categorize"}"#;
        assert!(handle_arest_transition(
            &s, "GET", "/arest/entities/SupportRequest/sr-1/transition", body
        )
        .is_none());
        assert!(handle_arest_transition(
            &s, "DELETE", "/arest/entities/SupportRequest/sr-1/transition", body
        )
        .is_none());
    }

    #[test]
    fn transition_rejects_path_without_transition_suffix() {
        let s = state_with_sr_state_machine("sr-1");
        let body = br#"{"event":"categorize"}"#;
        // Missing /transition — this is the create path, not transition.
        assert!(handle_arest_transition(
            &s, "POST", "/arest/entities/SupportRequest/sr-1", body
        )
        .is_none());
    }

    #[test]
    fn transition_rejects_unknown_event() {
        // SM exists, but no Transition row matches `(Received, escalate)`.
        let s = state_with_sr_state_machine("sr-1");
        let body = br#"{"event":"escalate"}"#;
        let result = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/SupportRequest/sr-1/transition",
            body,
        );
        assert!(result.is_none(), "no transition for escalate from Received");
    }

    #[test]
    fn transition_rejects_missing_state_machine() {
        // Resource exists but no SM row points at it. The handler
        // fails closed (None) so the kernel HTTP layer maps to 400 —
        // mirror of the worker's 'Entity has no state machine' branch
        // (router.ts:646-648).
        let s = Object::phi();
        let s = cell_push("Noun", fact(&[("name", "Support Request")]), &s);
        let s = cell_push("Support Request", fact(&[("id", "sr-orphan")]), &s);
        let body = br#"{"event":"categorize"}"#;
        let result = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/SupportRequest/sr-orphan/transition",
            body,
        );
        assert!(result.is_none(), "no SM → None");
    }

    #[test]
    fn transition_rejects_missing_event_field() {
        let s = state_with_sr_state_machine("sr-1");
        // Body has no `event` field.
        let body = br#"{"domain":"support"}"#;
        let result = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/SupportRequest/sr-1/transition",
            body,
        );
        assert!(result.is_none(), "missing event → None");
    }

    #[test]
    fn transition_rejects_unknown_noun() {
        let s = state_with_sr_state_machine("sr-1");
        let body = br#"{"event":"categorize"}"#;
        let result = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/Widget/sr-1/transition",
            body,
        );
        assert!(result.is_none(), "unknown noun → None");
    }

    #[test]
    fn transition_preserves_unrelated_sm_rows() {
        // Two SMs in the cell, one matching forResource, one not. The
        // transition must update only the matching row and leave the
        // other untouched.
        let s = state_with_sr_state_machine("sr-1");
        let s = cell_push(
            "State Machine",
            fact(&[
                ("id", "sm-other"),
                ("forResource", "sr-other"),
                ("currentlyInStatus", "Received"),
            ]),
            &s,
        );
        let body = br#"{"event":"categorize"}"#;
        let (new_state, _) = handle_arest_transition(
            &s,
            "POST",
            "/arest/entities/support-requests/sr-1/transition",
            body,
        )
        .expect("transition succeeds");
        let sm = crate::ast::fetch_or_phi("State Machine", &new_state);
        let sm_seq = sm.as_seq().expect("sm cell");
        assert_eq!(sm_seq.len(), 2, "both rows preserved");
        // Find each by id and check status — order isn't part of the contract.
        let target_row = sm_seq
            .iter()
            .find(|r| crate::ast::binding(r, "id") == Some("sm-1"))
            .expect("sm-1 still present");
        assert_eq!(
            crate::ast::binding(target_row, "currentlyInStatus"),
            Some("Categorized")
        );
        let other_row = sm_seq
            .iter()
            .find(|r| crate::ast::binding(r, "id") == Some("sm-other"))
            .expect("sm-other still present");
        assert_eq!(
            crate::ast::binding(other_row, "currentlyInStatus"),
            Some("Received"),
            "untouched SM keeps its prior status"
        );
    }

    #[test]
    fn update_binding_replaces_existing_value() {
        let entity = fact(&[("id", "x"), ("currentlyInStatus", "A")]);
        let updated = update_binding(&entity, "currentlyInStatus", "B");
        assert_eq!(crate::ast::binding(&updated, "currentlyInStatus"), Some("B"));
        // Other field preserved.
        assert_eq!(crate::ast::binding(&updated, "id"), Some("x"));
    }

    #[test]
    fn update_binding_appends_when_key_missing() {
        let entity = fact(&[("id", "x")]);
        let updated = update_binding(&entity, "currentlyInStatus", "A");
        assert_eq!(crate::ast::binding(&updated, "currentlyInStatus"), Some("A"));
        assert_eq!(crate::ast::binding(&updated, "id"), Some("x"));
    }
}
