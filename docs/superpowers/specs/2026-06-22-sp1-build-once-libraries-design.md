# SP1: Build-once Libraries — Warm-Base Compilation

- **Date:** 2026-06-22
- **Status:** Design approved (brainstorming); pending spec review → implementation plan
- **Scope:** SP1 of the "isolated by default, sharable when needed" library architecture (SP2 cross-db, SP3 release tree-shake, SP4 shared write-libraries follow).

## 1. Problem

Every app compile re-derives the metamodel's self-derived cells from scratch. Root-caused 2026-06-22 (`Chain Cost Driver 'supertype-union-reconstitution'` in the claude ledger): the wall is the `Function` supertype-union reconstitution (`reconstitute_absorbed_ft` + `resolve_view`) — `Function` is the root supertype (Noun/Verb/Fact Type/Resource < Function), so its membership is the union of every subtype, reconstituted ~24ms **per read**; `AREST_VIEW_TRACE` counted ~595 `Function` + 595 `Function_belongs_to_Domain` re-derives in a 25s window (57% of the storm), and a **trivial 1-entity app fails to converge in 90s**. Two blind fixes walled (faithful-negation; per-pass `resolve_view` memo — the latter can't help because the redundancy is cross-stratum and `Function` is un-cacheable as the chain derives subtypes).

The architectural fix: don't re-derive the base/libraries per app. Build them once.

## 2. Goal & non-goals

**Goal.** Pay each library's derivation (the LFP of its rules) **once per (readings-content, binary)**, not per app compile. App compiles become O(app delta). Target: support.auto.dev and arc-agi-3 compile in **seconds** (the performance bar: beat a human on time as well as score).

**Non-goals (this SP).**
- Cross-db `ATTACH`/reference (SP2 — "sharable when needed").
- Release tree-shake persistence (SP3); shared write-libraries (SP4).
- **Any change to FP/lambda semantics.** A library db is pre-computed `Object` cells + `Func` defs — the same artifacts the engine already produces and persists. The whitepaper's "everything is a lambda" model and WASM/native portability are unchanged. SP1 caches the LFP and loads it; it is explicitly **not** a rewrite.

## 3. Architecture

**A library = a readings dir compiled on its dependency libraries.** The metamodel is the root library; `kernel`, `spd-1`, `sherlock`, `arc-meta`, etc. are libraries built on the metamodel + their declared deps. A **blank library db** = the library's schema cells **+ the fully-derived LFP of the library's own rules over its own population**, with **no** downstream/app population.

**Build (produce the blank db) — automatic, content-keyed.** Extend the existing metamodel parse-cache pattern (`loadcache.rs`, the `~/.temp/arest-*` sidecar):
- Key = FNV(library readings content + dependency library keys + binary self-hash).
- Miss → compile the library *on its already-built deps* (recursively; metamodel is the base case with no deps) to its LFP → store the blank db.
- Hit → reuse. "Built once" per (content, binary); no manual command.

**App compile — warm load + delta-derive.**
1. Resolve the app's referenced libraries (its dependency dirs) → ensure each is built (recursive, topological) → **load each blank library db's cells (schema + derived) as the prior state** (the warm base). The `Function` reconstitution and all base derivations arrive *already materialized*.
2. Parse the app's own readings → the **delta** (new nouns/FTs/rules/population).
3. Run the **existing seeded-delta semi-naive chain** (`forward_chain_defs_state_seeded_with_delta`) over the delta. The `#836` derived-cell drop is **scoped to app-derived cells**; library cells are reused, not re-dropped/re-derived.
4. Persist base + app into the app's own db (**isolated copy** — the "copy the blank pre-built db in" model).

## 4. Data flow

```
BUILD (once per content+binary, per library, deps-first):
  readings ──parse(cached)──▶ state ──compile──▶ defs ──forward-chain to LFP──▶ blank lib db  (cached)
                                   ▲
                          prior = union of dep blank dbs (warm)

APP COMPILE (every time, O(delta)):
  prebuilt lib dbs ──load──▶ warm base ─┐
  app readings ────parse───▶ delta  ────┴─▶ seeded-delta chain (app cells only) ──▶ persist app db
```

## 5. Correctness & edge handling

- **Common case** — app adds *new* nouns/FTs/rules: no library cell's inputs change → every library cell reused as-is; the seeded-delta chain fires only app rules.
- **Edge case** — app *extends* a library cell (adds facts/rules feeding a library derivation, e.g. a new Transition affecting a library SM's negation-derived `terminal`/`rooted`): detect app facts/rules whose target FT is a **library** FT → add just those specific library cells to the delta drop+rederive (scoped, never all).
- **Delta-soundness.** The seeded-delta view-swap is sound for monotone-positive deltas (AREST.tex semi-naive). Negation / non-monotone over a library cell is the carve-out → covered by the scoped re-derive of affected cells above.
- **Equivalence gate (the safety net).** *app-on-prebuilt-library* MUST be byte-identical to *app-on-full-recompile* (identity-aware cell+def comparison). Any divergence is a soundness bug and fails the gate — this is how we guarantee SP1 cannot silently corrupt an app.

## 6. Testing

- **Equivalence:** extend the existing cold-vs-warm gate (`6249/838` cold==warm) to compile a representative app **warm** (on prebuilt libraries) vs **cold** (full recompile) and assert identical final state.
- **Timing:** the Widget base compile drops from >90s-non-converging to a few seconds; support.auto.dev and arc-agi-3 compile in seconds.
- **Regression:** the full lib gate (`cargo tall`) stays green; build incrementally and run the gate per step (the user cannot absorb a rewrite/regression).

## 7. Touchpoints (code)

- `cli/entry.rs` — multi-dir compile path (~2909-3762); metamodel parse-cache (~3050-3063); the `#836` drop (~3523); seeded-delta chain call site (~2049).
- `loadcache.rs` — extend to library-build caching (the blank-db artifact, content+binary+deps keyed).
- `evaluate.rs` — reuse `forward_chain_defs_state_seeded_with_delta`; scope the `#836` drop to app cells.
- New: a **library-resolution** step mapping an app's dependency dirs → library build keys → prebuilt blank dbs (build-on-miss, recursive/topological).

## 8. Risks & mitigations

| Risk | Mitigation |
|---|---|
| R1 delta-soundness for app-extends-library (negation) | scoped affected-cell re-derive (§5) + the cold==warm gate (the hard guarantee) |
| R2 library dep graph / build order | topological build (deps first); content-key includes dep keys so a dep change invalidates dependents |
| R3 seeded-delta chain "didn't help Widget earlier" | that path was the full drop-all pipeline; SP1 *routes* the app compile through warm-load + seeded-delta with the base pre-built (not dropped) — verified by timing |
| R4 "no rewrite" constraint | SP1 reuses parse-cache + seeded-delta chain + the existing cells/defs store; zero FP/WASM semantic change |

## 9. Success criteria

- support.auto.dev and arc-agi-3 compile in **seconds** (warm).
- cold==warm equivalence holds (correctness, gated).
- `cargo tall` green.
- Each library's derivation is paid once per (content, binary) and reused across app compiles.
