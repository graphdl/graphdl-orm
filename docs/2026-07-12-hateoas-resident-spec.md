# HATEOAS in the resident — emit `_links` per Theorem 4 (the arest-show gap)

2026-07-12. Samuel flagged from a live arest-show screenshot: the viewer flat-tabs
every object type and never follows an entity's links — "this isn't following the
HATEOAS navigation map algorithm." Grounded on the whitepaper, the gap is two-level;
this spec fixes the RESIDENT half (the prerequisite). Reach-order-pure: the canon
already defines the algorithm; the resident must *apply* it, exactly as the Python
twin does.

## The whitepaper contract (Theorem 4: HATEOAS as Projection)

- `docs/04-state-machines.md` §HATEOAS: an entity's response carries `_links` — only
  the transitions whose `from` matches current status (plus supertype-inherited); a
  terminal entity has `links(s) = ∅`.
- `docs/07-generators.md` — Theorem 4a (transition links as θ₁ projection) + **4b
  navigation** (one nav URI per binary fact type the noun participates in). `_links` =
  **`links_full(e, n, status(e, P))` = valid transitions ∪ navigation URIs**, and
  "`data` and `_links` are required" in every entity response.
- `docs/09-mcp-verbs.md`: `get` "Returns the entity with HATEOAS links and navigation."
- `docs/01-introduction.md`: Fielding 2000 — navigation over a resource graph.

## The canon already IS the algorithm (nothing to author here)

- `system:links_of`  (arest.canon:6585) = `cat( apply⟨nav_of, N1⟩, apply⟨transitions_of, N2,N3⟩ )`
  — the CAT union `links(e) = nav(e) ∪ transitions(status(e))`. Thm. hateoas.
- `system:nav_of`         (arest.canon:3233) — the 4b navigation half (α(1)∘INSERT(COND eq key)∘…).
- `system:transitions_of` (arest.canon:1190) — the 4a transition half.

The Python host is a pure twin: `engine.py:948 links_of(key_pos, sm, status_pos)` just
`_apply(A("system:links_of"), _S(A(key_pos), sm, A(status_pos)))`. No reimplementation.

## The resident gap (main.rs)

- `get` op (main.rs:12216) resolves the canon via `ev.mu` for `system:entity_view`
  (12239) and `system:ev_cols` (12243), then emits `{app,noun,id,exists,fields,facts}`
  — **no `_links` key**. This violates the "`_links` required" contract.
- `rp_transitions_of` (main.rs:8759) is a native twin of the transitions half; used at
  8968 so `RpSpec.links` = transitions ONLY (not the full `links_of`).
- No `rp_nav_of`: the Theorem-4b navigation half has no native twin and is emitted
  nowhere. `actions` (12327) surfaces transitions as a SEPARATE verb, never unioned
  with nav into a single `_links` on the entity representation.

## The fix (canon-first, minimal, byte-certifiable)

1. In the `get` op, after `entity_view`/`ev_cols`, resolve the links the same way:
       let links = ev.mu(napp(na("system:links_of"),
                               nseq(vec![na(&key_pos), sm_value, na(&status_pos)])));
   and emit them as a `_links` object on the response (relation/event → href/method,
   per the 04-state-machines JSON shape). Operands (`key_pos`, `sm`, `status_pos`) are
   exactly what `create_spec` already computes for the noun (RpMachine.role_pos /
   status_col; the `sm_n` triples built at main.rs:8962) — thread the spec into `get`
   or recompute from the same `smDef`/`subtype` walk `actions` already does (12362+).
2. Canon-first: call `system:links_of` interpreted first (like entity_view). If the mu
   walk is slow at store scale, register a native override — the transitions twin
   (`rp_transitions_of`) already exists; add `rp_nav_of` twinning `system:nav_of` and
   `cat` them, byte-identical to `system:links_of` behind AREST_NO_NATIVE_LINKS.
3. Certify: zero-python differential — native/mu `get` responses byte-identical over
   the tasks corpus (reuse the query-cert harness shape); terminal-status entities must
   emit `_links: {}` (links(s)=∅). Kill switch = the mu oracle.

## Then the viewer (arest-show, engine/csharp/show/Program.cs)

Shell.Build (427) consumes only `cli.Nouns()` and tabs every noun. HATEOAS-correct:
land on a home resource, read its `get` response's `_links`, render those as the
navigation affordances, and drill the resource graph by following links — not a flat
tab per object type. The IHost seam already has `Get`/`Actions`; add the `_links` from
the enriched `get` and navigate by them. Blocked on step 1 landing `_links` first.

The FULLER target is the schema-level **Entity Navigation Graph** — the canon's
`system:nav` family, now documented in `docs/25-navigation-graph.md`. `_links` here is
its per-entity projection (Theorem 4b); the viewer's app-level shell (which noun is a
root, what drills into what, what's a sibling) should be built from `system:nav` /
`nav_kind` (child = drill 1:n, peer = sibling, collection = m:n), which is exactly the
1:n-spanning + m:n-siblings nav map. That family is live in canon but unwired — expose
`system:nav` through the resident alongside `_links`.

## Progress (2026-07-12)

- **Resident `nav` verb STAGED + cargo-check-clean.** `main.rs` store_call gains a
  `"nav"` arm that resolves `system:nav(⟨noun, D⟩)` on the carrier (canon-first, the
  host is the twin — no Rust reimplementation of the child/peer/collection
  classification) and renders `{app, noun, nav:[{kind, fact_type, targets}…]}`.
  Registered in the tools list, the store-only dispatch (`matches!` at ~12655), and
  the store_call match. `cargo check` passes (9.86s, exit 0). Contract is pinned by
  `test_entity_view.py::test_nav_labels_controls_by_the_uniqueness_structure`
  (`system:nav("Task", D)` → the peer/child/collection triples).
- Deliberately native-only: `nav` is NOT in the `AREST_DELEGATE_READS` list — no
  Python nav path, consistent with "ops must not be python-specific". `system:nav`
  reads only the schema cells (`role`/`constraint`/`spans`), not instance rows, so the
  interpreted mu resolution is cheap; a native twin is a later option, not needed now.

## Remaining / sequencing

1. Full `cargo build --release` to link + a resident cert (`nav` over the tasks app
   returns sane child/peer/collection edges) — deferred so a heavy build does not skew
   the still-running native-aggregate gate's timing (arest 11660) and because arest-show
   holds the release exe lock.
2. The `get` `_links` step (system:links_of on the entity) — the per-entity projection.
3. arest-show: navigate by `nav` (drill `child`, siblings for `peer`/`collection`)
   instead of flat `Nouns()` tabs; optionally a diagram generator off the same edges.
