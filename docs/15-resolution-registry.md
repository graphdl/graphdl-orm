# 15 — The Resolution Registry (the operation-override seam)

Chapter 11 (*Runtime Portability*) confines target variation to two trait objects —
`Platform` (resolves the named functions readings delegate to) and `Native`
(persistence and network I/O) — and guarantees that the same `ast::apply` runs
everywhere. That is the DI/IoC seam at the **I/O** level: a target *provides* the
side-effecting functions the model names.

This chapter is its companion at the **operation** level. It answers a different
question: when a canon operation is *correct but slow* — the reference reduction of
`system:entity_view` is minutes at fleet scale, an identity compile is 23M native-twin
primitive calls dominated by interpreter overhead — how does a platform register a
**fast override** for it *without moving the meaning out of the canon*, and how does
the runtime *resolve* the override versus the reference by interface? That is the
"code to an interface, register fast overrides per platform" pattern (iFactr's
cross-platform trick), applied to AREST's certified-twin discipline. The seam exists
today, but as **ad-hoc scattered dispatch**; this chapter specifies it as a single
registry so that new overrides — compile chief among them — are table entries, not
edits to the reducer.

## The pattern: a certified twin *is* an interface implementation

Every operation in AREST has exactly one **meaning**, and it lives in the shared canon
as a `system:*` DEF (the reference implementation — portable, runs wherever the
reducer `mu` runs, in any host language). A platform may *additionally* carry a **fast
override**: a native function that computes the *same answer* by a faster route. The
two are held equal by a **byte-differential parity pin** behind a kill switch — flip
the switch and the override is bypassed, the reference runs, and the two outputs must
be identical byte-for-byte.

Mapped onto the DI/IoC vocabulary the standing direction uses:

| DI/IoC term            | AREST realization                                            |
|------------------------|--------------------------------------------------------------|
| interface              | a canon operation name (`system:entity_view`, the `compile` verb) |
| reference implementation | the canon DEF's body, reduced by `mu` (portable, slow)     |
| registered override    | a per-platform native function keyed by that name (fast)      |
| resolve-by-interface   | the reducer/dispatch looks the name up and picks override-or-reference |
| the DI container       | the **Resolution Registry** (this chapter) — one table, one resolver |

The meaning never leaves the canon. The override is *only* speed, and it is *only*
trusted because the kill switch certifies it equal to the meaning. This is why the
pattern is safe to apply everywhere: an override can never change behavior — at worst
it is a slower or faster route to the same bytes, and the differential proves which.

## What the seam looks like today (two ad-hoc layers)

The pattern is already pervasive — but resolution is hardcoded in two places, keyed by
name but structured as scattered `match` arms rather than a registry.

### Layer 1 — DEF-level, inside the native reducer

`NEval::mu` resolves a leaf name in a fixed order (`engine/rust/src/main.rs:1651`–
`1702`):

```
prim(name, x)                    -- native base primitives AND certified-equal overrides
  ↳ None → process list          -- the app's own compiled-in DEFs (main.rs:1654)
      ↳ miss → NCANON            -- the binary's compiled-in canon DEFs (main.rs:1688)
          ↳ miss → ⊥            -- name is neither prim, process, nor canon
```

The **fast overrides live inside `prim`** (`main.rs:1729`+): alongside the BFP base
primitives (`id`, `tl`, `atom`, `eq`, `cat`, …) sit canon-*named* arms —
`system:ev_cols` (`:1993`), and the documented `system:vb_fetch` / `system:entity_view`
twins (`:1972`–`:1989`) — each annotated *"CERTIFIED-EQUAL OVERRIDE of DEF(…)"* and
each twinned by a parity pin in `tests/derive.rs`. When `prim` returns `None`, the same
`mu` falls through to the **reference** reduction over `NCANON`. So the override/reference
choice is real and correct — but it is expressed as hand-placed arms in a 250-line
`match`, discoverable only by reading it.

The `_h_*` translator cooks (`src/cooks.rs`, dispatched through `native_cook`,
`main.rs:5394`) are the same shape one level up: a native translator fires when a
translator name *carries no canon DEF*, else the canon body reduces.

### Layer 2 — op-level, inside the MCP dispatch

`mcp_call_inner` (`main.rs:12768`) is a second, independent resolver — a `match tool`
that binds each verb to native or Python:

| verb(s)                              | binding                              |
|--------------------------------------|--------------------------------------|
| `get` `actions` `schema` `synthesize` `query` `cells` `derive` `nav` | native `store_call` |
| `apps_compile`                       | native `native_apps_compile`; Python `delegate_verb` as the opt-in oracle (`AREST_PYTHON_COMPILE`) |
| `apply`                              | native `native_apply`, else Python `delegate_verb` |
| `verify` `validate`                  | native when switched on or when no Python CLI resolves; else Python `delegate_read` |
| `retract`                            | **Python `delegate_verb` (whole)**   |
| `sql` `explain`                      | Python `delegate_read`               |

and every override consults the **one kill-switch convention** at its seam:
`AREST_NO_OVERRIDE=<name>[,<name>…]` bypasses the named overrides, `AREST_NO_OVERRIDE=*`
bypasses them all (the pure-reference oracle). The pre-registry per-name switches
(`AREST_NO_THETA_ARMS`, `AREST_QUERY_INTERPRETED`, `AREST_SYNTH_SCOTT`,
`AREST_DELEGATE_READS`, `AREST_PYTHON_COMPILE`) remain honored as aliases that fold into
the same killed-name set (`parse_no_override`). `HOST_OVERRIDES` enumerates every name the
Rust host registers, and a standing test asserts that enumeration is a subset of the canon
catalog.

`compile` is the proof the seam works: its meaning is entirely in the canon (chapter 06),
`op_compile_model` drives the whole pipeline natively (classify → translate → fold → rekey →
derive → status → replay → machine_fold → layout → scheduler → generator → create_handlers →
save), and `native_apps_compile` composes an app's `readings/` atop the base and runs it —
so a Rust-configured environment with no Python runs the user-facing compile, and the Python
delegate survives only as the differential oracle. `retract`, `sql`, and `explain` still
reach Python through hardcoded arms; they are the remaining candidates for the same
treatment.

## The target: one registry, one resolver, one kill switch

The refactor is behavior-identical by construction — it *relocates* the existing arms,
it does not rewrite them.

### 1. The interface catalog (canon side — theory-driven)

The set of override-eligible operations is not host trivia; it is a fact about the
system, so it belongs in the canon as self-described data. Model it the same way
everything else is modeled:

```
Operation is identified by its name.                      -- the interface
Operation is reference-implemented by exactly one DEF.    -- already true: system:X IS X's reference
Operation may be overridden.                              -- a boolean: is a fast twin permitted?
```

Today this catalog is *implicit* — every `system:*` DEF is its own reference impl, and
"is overridable" is encoded only by whether a `prim` arm happens to exist. Making it an
explicit fact type means the catalog is queryable (`query Operation_may_be_overridden`),
the parity pins can be generated from it, and a target can *enumerate* which operations
it is expected to twin. The reference impls stay in the canon; only the *override
bindings* are per-platform (they must be — they are native functions), so they cannot
live in canon data. The canon carries the **catalog**; each host carries a **bag of
overrides keyed to catalog names**; the resolver composes them. That is precisely
"register implementations keyed by the interface, request by interface type."

### 2. The override table (host side — one place)

Replace the scattered arms with a single registration point per platform:

```rust
// one table, the DI container. Every certified-equal override is registered here,
// keyed by its canon interface name. Absence ⇒ the reference reduction runs.
type Override = fn(&N, &Srv) -> Option<N>;     // Some(answer) = handled; None = defer to reference
static OVERRIDES: &[(&str, Override)] = &[
    ("system:ev_cols",      ev_cols_override),
    ("system:entity_view",  entity_view_override),
    ("system:vb_fetch",     vb_fetch_override),
    // … the _h_* cooks, reading_parse, ftid …
    // compile joins here (below) — a new row, not a dispatch edit
];
```

The functions are the ones that exist today, lifted out of the `prim` match verbatim.

### 3. One resolver, consulted by both layers

`mu`'s leaf step and `mcp_call_inner` both call the same resolver instead of open-coding
their choice:

```rust
fn resolve(name: &str, x: &N, srv: &Srv) -> Resolution {
    if !killed(name) {                                  // uniform kill switch (below)
        if let Some(f) = OVERRIDES_MAP.get(name) {
            if let Some(ans) = f(x, srv) { return Resolution::Override(ans); }
        }
    }
    Resolution::Reference                               // fall to process → NCANON → mu
}
```

At the op layer, `apps_compile` / `retract` / `sql` … the "Python delegate" becomes just
one more override function (`delegate_verb`) registered against those names — so the
*mechanism* is identical whether the fast path is native Rust or an out-of-process
oracle. The dispatch `match tool` shrinks to "route to store vs. resolver"; the
per-verb policy moves into the table.

### 4. One differential kill switch

Collapse `AREST_DELEGATE_READS`, `AREST_SYNTH_SCOTT`, `AREST_NO_INCR_AGG` into one
convention:

```
AREST_NO_OVERRIDE=<name>[,<name>…]      -- bypass these overrides; run the reference
AREST_NO_OVERRIDE=*                     -- bypass ALL overrides (the pure-canon oracle)
```

`AREST_NO_OVERRIDE=*` is then the single "run everything through the portable reference
reducer" mode — the differential oracle every parity pin certifies against, and the exact
mode a minimal `no_std` target runs in (no overrides, no Python, just `mu` over the canon).
The existing switches become documented aliases during migration, then retire.

## Compile through the seam — off Python

`compile` resolves in the principled order the registry prescribes:

1. **native override** — `native_apps_compile` composes the app's `readings/` atop the
   base and drives `op_compile_model`, the complete native pipeline (classify → translate
   → fold → rekey → derive → status → replay → machine_fold → layout → scheduler →
   generator → create_handlers → save). This is the default.
2. **canon reference** — `mu` reducing the compile's canon DEFs directly. Slow, but it
   runs **wherever the reducer runs**, with no Python and no native override — the bare
   target's row in chapter 11's map, and what makes compile *portable* rather than
   *Python-specific*.
3. **Python delegate** — `delegate_verb`, the opt-in differential oracle
   (`AREST_PYTHON_COMPILE`), never required and never reached under `AREST_NO_OVERRIDE=*`.

The requirement *"compile and other ops must not be Python-specific — we cannot guarantee
Python in a Rust-configured environment"* is thereby a **structural** property, not a
hope: Python is one implementation *behind* the compile interface, selected last. The
byte-parity differential (`tools/apps_compile_parity.py`) holds the native override equal
to the oracle over real apps through the real flow.

## Migration (behavior-identical, certifiable at each step)

1. **Unify the switches** — done: `AREST_NO_OVERRIDE` with the old vars as aliases, one
   `overrides_killed(name)` consulted at every seam.
2. **Register the catalog in canon** — done: `shared/base/resolution.md` declares
   `Operation is overridable` and enumerates the catalog; the host's `HOST_OVERRIDES`
   is asserted a subset of it by test.
3. **Lift, don't rewrite** — remaining: physically move the `prim` override arm bodies
   and the `match tool` bindings into an `OVERRIDES` fn-pointer table consulted by one
   resolver, so a new override is a table row rather than a dispatch edit. The kill
   switch under `*` must stay byte-identical to the per-alias runs on the fixture and
   corpus — the same discipline as every twin pass.

Each step is gated by the same differential the twins already use, so the registry is
adopted without a single un-certified change. (`compile` is the seam's first op-level
citizen — registered, certified, and default-flipped.)

## Chapter 11's map entry

Through the seam, chapter 11's `compile` cell reads:

| Primitive | Local (CLI)                                             | bare / `no_std` |
|-----------|--------------------------------------------------------|-----------------|
| `compile` | native override → canon reference → Python oracle | canon reference (`AREST_NO_OVERRIDE=*`) |

## Relation to the rest of the docs

- **Chapter 11** is the I/O half of the same idea (`Platform`/`Native` traits); this
  chapter is the operation half (the `OVERRIDES` table). Together they are AREST's whole
  DI/IoC story: *meaning in the canon, side effects and speed injected per platform,
  everything certified equal to the canon reference.*
- **`2026-07-10-rust-native-compile.md`** is the design record behind the compile
  override's native pipeline.
- **`2026-07-12-incremental-aggregate-spec.md`** is another registry citizen: a fast
  override of the fixpoint aggregate pass, gated by (today) `AREST_NO_INCR_AGG` — which
  step 2 folds into `AREST_NO_OVERRIDE`.
- **The `system:nav` family (chapter 25)** is the current cautionary tale for why the
  differential matters: a canon fix that is correct under the reference reducer can still
  read wrong through an override whose twin has drifted — which is precisely what the kill
  switch exists to catch.

## What resolves through the seam today

`apps_compile` is native: the dispatch arm resolves to `native_apps_compile` (no Python)
unless `AREST_PYTHON_COMPILE` selects the Python delegate as the differential oracle. The
oracle run is also how the host-specific sqlite `.db` projection the `sql` verb reads is
regenerated — itself a Python-bound op the registry should absorb. `verify` and `validate`
resolve natively when their switch is set (`AREST_NATIVE_VERIFY` / `AREST_NATIVE_VALIDATE`)
or when no Python CLI is resolvable; both reduce constraint and rule meaning on the N
carrier through the canon. `retract`, `sql`, and `explain` still reach Python through
hardcoded arms — they are the remaining candidates for registration.

The parity harnesses (`tools/apps_compile_parity.py`, `tools/twin_equality.py`) are the
executable certification record for all of the above; run them after any change to a twin
or to the compile path.
