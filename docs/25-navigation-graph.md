# 25 — The Entity Navigation Graph

Theorem 4 (chapter 04, *HATEOAS as Projection*) and its navigation half, Theorem
4b (chapter 07), give the **local** view: for one entity `e`, the links it carries
— its valid transitions plus one navigation URI per binary fact type it
participates in. That is a neighbour-star: from `e`, here are the adjacent
resources.

A user interface, and an ORM diagram, need the **global** structure the star is a
slice of: which noun is a root, what drills into what, what sits beside what. That
structure is the **Entity Navigation Graph**, and AREST derives it — like
everything else — from the model itself, with no hand-authored menu and no
separate cardinality annotation. It lives in the shared canon as the
`system:nav_*` family (`engine/shared/arest.canon:4255`–`4396`); this chapter is
its specification.

The graph is *one* structure with two renderings: unfolded in time it is the
**navigation map** (how you drill an app); embedded in space it is the **ORM
diagram layout**. The same edge classification drives both.

## Cardinality is read off uniqueness — it is not stored

AREST never stores a "1:n" annotation. Following ORM (Halpin & Morgan,
*Information Modeling and Relational Databases*, 2nd ed., §4 on uniqueness), a
fact type's cardinality **is** its uniqueness-constraint pattern:

- a uniqueness constraint spanning a **proper subset** of the roles makes the fact
  type **functional** on that subset (the subset determines the rest) —
  many-to-one, a **1:n** relationship;
- a uniqueness constraint spanning **all** the roles makes it **one-to-one** (1:1);
- **no** uniqueness constraint makes it **many-to-many** (m:n).

So the classifier only has to look at the fact type's uniqueness constraints and
its arity. The canon does exactly that:

- `system:ev_roles` (`arest.canon:4057`) — the fact type's roles (from the
  `role` population), hence its **arity** via `system:nav_arity` = `length ∘
  ev_roles` (`:4292`).
- `system:nav_ucs` (`:4255`) — the constraints of `kind = "uniqueness"` whose
  target is this fact type: its uniqueness constraints.
- `system:nav_spans` (`:4277`) — the number of roles a given uniqueness constraint
  spans (`length` of its `spans` row).
- `system:nav_spanning` (`:4295`) — `not ∘ null` over the uniqueness constraints
  filtered to those whose span **equals the arity**: **true iff some UC spans every
  role**, i.e. the fact type is 1:1.

## The edge classification — `nav_kind`

`system:nav_kind` (`arest.canon:4310`) reduces the above to one of three tags:

```
nav_kind(ft) =  collection   if  nav_ucs(ft) is empty        -- m:n
                peer         if  nav_spanning(ft)             -- 1:1
                child        otherwise                        -- 1:n (functional)
```

- **`child`** — a functional (1:n) fact type. This is a *drill* edge: one side
  determines the other, so from a row on the determining side you descend into the
  dependent rows. Chaining child edges — 1:n into that n's own 1:n, and so on — is
  the spanning descent the navigation map is built from.
- **`peer`** — a 1:1 fact type. The two nouns are co-equal; the edge is a
  *sibling* placed at the same level, not a descent.
- **`collection`** — an m:n fact type. A junction; rendered as a related
  collection beside the node, never a descent (descending an m:n would not
  terminate).

The determinant/dependent *direction* of a `child` edge is refined by which role
carries the subset UC; `nav_kind` returns the class, and the projection
(`nav_of`, below) resolves the direction for a concrete entity.

## The edge and the graph — `nav_control` and `nav`

- `system:nav_players` (`:4324`) — the noun playing each role of the fact type.
- `system:nav_remaining` (`:4332`) — from the current noun, the *other* players:
  the edge's targets (correctly handling ring fact types, where a noun plays more
  than one role, by dropping only one occurrence of self).
- `system:nav_control` (`:4355`) — the **edge record**:
  `⟨ nav_kind(ft), from-noun, nav_remaining(ft, from-noun) ⟩` — a classified,
  directed edge.
- `system:nav_playedfts` (`:4364`) — the fact types the noun plays a role in
  (its `role` rows), and `system:ev_ownfts` (`:4794`) its owned/objectified ones.
- `system:nav` (`:4380`) — the top level: `ALPHA(nav_control)` over the noun's
  played and owned fact types. **`nav(n)` is the set of classified edges out of
  noun `n`.**

The Entity Navigation Graph of an app is `G = ⋃_n nav(n)` — every noun's edge set,
each edge tagged `child` / `peer` / `collection`.

## Rendering 1 — the navigation map

Take the sub-graph of **`child`** edges. It is (barring modelled cycles) a DAG on
the functional relationships. A navigation map is a walk of it:

1. **Roots** are nouns reachable as an entry (top-level collections — those with an
   incoming `collection`/`peer` affordance but not themselves a functional
   dependent), or an explicitly chosen home noun.
2. From a node, **descend** each `child` edge — 1:n into the dependent noun, then
   that noun's own `child` edges, recursively: the "1:n → that n's 1:n → …" spanning
   descent.
3. At each node, its **`peer`** and **`collection`** edges are **siblings** —
   lateral affordances rendered beside the drill path, not descended.
4. A node whose only out-edges are `peer`/`collection` is a **leaf** of the drill.

This is the whole navigation-map generation algorithm: no per-screen menu is
maintained; the legal next moves at any node are recomputed from `nav(n)` and split
by `nav_kind` into *descend* (child) versus *beside* (peer/collection).

## Rendering 2 — the ORM diagram layout

The same graph, embedded in the plane, is the ORM diagram's layout engine:

- **`child`** edges are **rank edges** — a layered (Sugiyama-style) assignment on
  the functional DAG. The determinant is one rank up, the dependent one rank down;
  chained child edges deepen the levels.
- **`peer`** edges join nouns at the **same rank**, placed adjacent.
- **`collection`** edges are drawn as **junction siblings** between their two
  participants, off the main descent.

The drill order of the navigation map and the top-to-bottom rank order of the
diagram are the *same* ordering of the *same* graph — which is why one algorithm
served both the nav-map generator and the diagram layout engine.

## Relation to Theorem 4 — the graph is what the links project from

`system:nav` is the **schema-level** graph. `system:nav_of` (`arest.canon:3233`),
which Theorem 4b uses, is its **instance-level projection**: for a concrete entity
`e` of noun `n`, each out-edge `⟨kind, target⟩` of `nav(n)` becomes a concrete
link in `e`'s representation —

- `child` → the dependent collection filtered by `e`'s key,
- `peer` → the 1:1 counterpart entity,
- `collection` → the related m:n set,

which is exactly `links_full(e, n, status) = nav(e) ∪ transitions(status(e))` of
chapter 07. Theorem 4's per-entity links are the shadow this schema graph casts on
a single entity. The graph is primary; the HATEOAS links are its projection.

## Worked example (the classification test)

`engine/tests/test_entity_view.py::test_nav_labels_controls_by_the_uniqueness_structure`
pins the whole classification on one fixture. A `Task` noun plays three fact types:

| fact type            | uniqueness constraint          | `nav_kind` |
| -------------------- | ------------------------------ | ---------- |
| `Task_blocks_Task`   | spans **both** roles (the pair is the key) | **peer** (1:1) |
| `Task_owns_Widget`   | spans **only** Task's role     | **child** (1:n — Widgets are children of the key) |
| `Task_notes`         | **none**                       | **collection** (m:n) |

and asserts `system:nav("Task", D)` reduces to exactly

```
(("peer",       "Task_blocks_Task", ("Task",)),
 ("child",      "Task_owns_Widget", ("Widget",)),
 ("collection", "Task_notes",       ("Note",)))
```

with `system:nav("Zebra", D) = ()` for an unplayed noun, and
`system:links("Task", e, D) = ⟨nav(e), transitions(status(e))⟩` closing the union of
Theorem 4. The classification above is therefore executable and verified, not a
description of intent.

## Real-store behavior (2026-07-12): RMAP absorption hides the 1:n edges

The classification above is correct on an **un-absorbed** schema — the fixture leaves
`rmapColumns` empty, so every fact type stays own-table and `nav_kind` sees its
uniqueness constraint. On a **real compiled store** it degenerates, and running the
wired resident `nav` verb over the `tasks` app makes this concrete:

- `nav("Task")` returns seven edges, **all `collection`** (`Task_blocks_Task`,
  `Task_touches_Source_File`, the derived `Task_carries_*_Task_Priority`, …).
- `nav("Park Reason")`, `nav("Status")`, `nav("Tick Action")` return **empty**.
- `child` and `peer` **never fire**.

**Root cause (isolated on the real D, 2026-07-12): `system:ev_ownfts` IS the excluder.**
`system:nav` draws its candidate pool from `system:ev_ownfts` (`arest.canon:4794`) =
`factType` **minus** `rmapColumns`. On the real `tasks` store 23 Task fact types are
absorbed into `rmapColumns` (columns 2–24), including `Task_is_parked_for_Park_Reason`
and `Task_is_currently_in_Status`, so the pool loses them **before** classification. A
stage-by-stage trace through the real reducer settled it:

| stage | on real `Task` | verdict |
| --- | --- | --- |
| `nav_playedfts("Task")` | includes both absorbed fts (32 fts) | played-set correct |
| `ev_ownfts("Task",…)` | **excludes** both (absorbed stripped) | **the drop is here** |
| `Filter(member …)` | would keep them | not the excluder |
| `nav_kind("Task_is_parked_…")` | `"child"` | would classify right if it arrived |

The absorbed functional fts are exactly the `child`/`peer` edges, so they never fire.
(An earlier note here claimed the excluder was "downstream" and that an `ev_allfts`
substitution was a real-store no-op — both were **measurement errors**: the Python
oracle double-wrapped the store (`load_sqlite` already returns a lam D), and the Rust
resident reading was a host-twin divergence, not the canon's behavior. Corrected.)

**The fix (applied, `arest.canon:4389`):** swap `system:nav`'s pool from `ev_ownfts` to
the full `factType` fetch. Absorption is a *storage* decision (own table vs. column); it
must not change *navigation* — an absorbed functional ft is still a reference edge, the
strongest nav link. `ev_ownfts` itself is unchanged (its other caller `system:ev_facts`
correctly needs own-table-only). **Proven in the real reducer:** nav("Task") goes
7 → 30 edges, `{collection: 22, child: 8}`, purely additive (all 7 baseline preserved),
surfacing `('child','Task_is_parked_for_Park_Reason',('Park Reason',))`,
`('child','Task_has_Owner',('Owner',))`, etc.; the fixture regression still passes.
(Side note: it also surfaces 15 absorbed **unary** booleans as target-less
`('collection', ft, ())` edges — consistent with how own-table unaries already appear;
add an arity-≥-2 filter if unary flags are unwanted as nav links. Design refinement, not
a bug.)

## Status (2026-07-13)

- **Sidecar-staleness fix DONE + certed end-to-end (2026-07-13).** The durable fix is
  implemented at every write/read site and proven on the real store. *Python writer*
  (`Registry._sidecar`, protocol.py:1901): the frozen process keeps a `"compiled"` def only
  when its name has no `:` — dropping the 325 namespaced engine-canon defs, keeping the 8
  bare app defs. *Rust ingest + grammar-scratch + native-compile writer* (main.rs:3043,
  6544, 10188): the same `':'`-filter, so existing stale sidecars load engine canon from
  live `NCANON`, and `op_compile_model` can never re-freeze it. **Certs:** a scratch compile
  writes a sidecar of exactly 8 bare defs (0 engine-canon); 30 Python tests across
  sidecar/ingest/defs/differential/CLI/app-compile stay green; a 123-app + grammar + base
  corpus scan confirms the filter drops nothing app-own; and the **release resident, over
  the un-recompiled stale `tasks` sidecar, now returns nav = 30 {collection 22, child 8}
  (was 7 all-collection)** with `query`/`schema` intact. So every canon fix now reaches
  already-compiled apps live, and the native compile inherits clean sidecars by
  construction. This resolves the nav divergence end-to-end.
- **Canon fix applied + Python-verified** — `system:nav` now navigates absorbed 1:n
  relations; real-store `tasks` gives 30 edges / 8 `child` through the reference reducer,
  fixture unchanged.
- **Resident `nav` verb wired** — `arest.exe` resolves `system:nav` canon-first.
- **RESOLVED — it was NOT a native-mu twin bug; it is stale-sidecar shadowing (diagnosed
  2026-07-12).** After the fix + clean rebuild the resident still returned 7 `collection`,
  and an earlier note here (and the ledger) wrongly called this an `NCANON`/native-`mu`
  twin bug. Instrumented tracing on the real store disproved that: the native carrier is
  faithful (native `ast:FetchPop` and the fresh-`NCANON` reduction both give the correct
  30). The resident returned 7 because it resolved `system:nav` from the **app's persisted
  process** — a *frozen copy of the entire engine canon* that every sidecar carries — and
  that copy predates the `4389` fix. `NEval::mu` resolves a name `process`-before-`NCANON`
  (main.rs:1654 then 1688), so the stale `system:nav` in the sidecar shadowed the corrected
  `NCANON` and the fix at 1688 was never reached. The Scott `make_mu` resolves in the same
  order, so this is *not* a twin divergence — both mus agree; both read the stale copy.
  **Root of the freeze:** `Registry._sidecar` (protocol.py:1901-1903) freezes every
  `"compiled"` def (which is all of `theta:`/`ast:`/`system:`/`constraints:`,
  kernel.py:229) into each app's process — defeating `NCANON`, whose whole purpose
  (main.rs:296-303) is to supply engine canon to stores that *don't* carry it. So **any**
  canon fix is silently shadowed for every already-compiled app, and this touches the
  native compile/derive path (`eval_full`/`eval_delta` over `srv.nprocess`) too, not just
  nav. This is a concrete instance of the chapter-15 thesis: an app must carry only its
  *own* defs and defer engine-canon names to the shared live reference; freezing a copy of
  the reference is the anti-pattern.
- **The fix (systemic).** *Immediate, zero-code:* recompile the app (`apps_compile tasks`)
  — the sidecar's frozen `system:nav` refreshes to the corrected def and the resident
  returns 30. *Durable (fixes existing stale sidecars without recompile):* at the
  sidecar-process ingest (main.rs:3043-3056, which feeds **both** the Scott `PROCESS` and
  native `srv.nprocess`, keeping the twins aligned) **skip names in an engine-canon
  namespace** so they always resolve from live `CANON`/`NCANON`; apply the same filter at
  `load_grammar_scratch` (main.rs:6544) and in the writers (`Registry._sidecar`
  protocol.py:1901-1903, Rust `sidecar_payload` main.rs:~11552) so sidecars stop freezing
  engine canon at all. Do **not** reorder only the native mu to NCANON-before-process — the
  Scott mu has the same order, so a native-only reorder would create a *new* native-vs-Scott
  divergence; the ingest/write filter feeds both mus the same list and preserves equivalence.
  - **The exact discriminator (verified 2026-07-12, correcting the diagnosis list).** Engine
    canon uses **five** namespaces — `system:` (221 defs), `constraints:` (43), `ast:` (37),
    `theta:` (22), and **`monad:` (2)**; the original recommendation named only the first
    four, so `monad:` would stay frozen. App-own defs are **bare-named** (no colon): a real
    `claude.store.json` process is exactly 325 namespaced engine defs + 8 bare app defs
    (`resolve, derive, validate, emit, create, run, rmap, csdp`). So the safe, complete
    filter is *"keep a `"compiled"` def iff its name has no `:`"* — drop every namespaced
    (engine) def, keep the bare app defs. Equivalence to certify is *filtered-sidecar +
    `NCANON` == a freshly-recompiled sidecar* (not == the stale one — the whole point is that
    changed engine defs now resolve to their corrected form).
- **Then** — `get` `_links` (per-entity projection); `arest-show` navigating by `nav`.

**Reusable real-D oracle recipe** (no double-wrap): `D = load_sqlite(".../tasks.db")`
(already a lam D — do NOT `to_lam` it); `from_lam(R(A("system:nav"), _S(A("Task"), D)))`.
Trace any sub-step by applying `system:nav_playedfts` / `system:ev_ownfts` / `nav_kind`
to the same `D`. Scratch scripts in the job tmp: `navreal3.py`, and the agent's
`navtrace.py` / `navtest_fix.py`.
