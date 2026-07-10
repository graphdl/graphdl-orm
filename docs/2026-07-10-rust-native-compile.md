# Rust-native compile: the residual host layer is a ~120-line driver, not a parser (#20/#18)

Design/scoping record, 2026-07-10, code-grounded. The production #20 win is COMPLETING
the Rust-native compile — the Rust engine shells the whole `compile` to Python today
(`engine/rust/src/main.rs:5948`, `"compile" => delegate_call("compile", ...)`). This is
the "can't guarantee Python in a Rust-configured environment" gap. This doc scopes that
build precisely and shows it is bounded.

## The reframe

The Rust-native compile has been treated as a huge parser+compiler rewrite. It is not.
The Rust engine ALREADY HOLDS every primitive the compile bottoms out on. The residual
gap is a thin host DRIVER plus the downstream derive phases — and most of the semantic
work is canon the native reducer already runs.

## The Rust already has every primitive (evidence)

- reducer: `reduce_over` (main.rs:2821), `mu` (1321), the native `NEval` carrier (fast path)
- `lex` twin: `register("lex")` (878), `fn lex_rows` (951) — the string→tokens boundary,
  proven equal to the canon by the 2026-07-07 ruling. This is the one irreducible host
  primitive the canon parse calls down to, and it is DONE natively.
- store: `store_into` (3004); resident mirror `srv.d/cells/nd/ncells/nprocess`
- derive: `op_run_rules` (3133) — semi-naive, already takes `changed` (the frontier)
- the canon PARSE surface, reducible by the native reducer because it bottoms out on `lex`:
  `system:reading_parse` (shared/system.canon:4656), `system:ftid` (4675),
  `system:clause_ft` (5093). Their Python twins (`_reading` compiler.py:593, `_ftid_from`
  621, `_clause_ft` 939) are the certified-equal host overrides, equality enforced over
  the whole base corpus by `test_reading_canon`.

So: parsing a reading is `reduce(system:reading_parse)` over `lex` — the Rust can already
do this. No parser to port.

## The gap #1: the driver (`compile_model_selfhost`, compiler.py:2049, ~120 lines)

The g-loop is a THIN host driver over canon, not a compiler. Its anatomy:

HOST control-flow (the new Rust to write, ~120 lines, `op_run_rules`-shaped):
- `statements(text)` — split into statements (small string op)
- `_split_modality(stmt)` (compiler.py:234) — alethic/deontic + sign + inner (regex)
- the Prose / `_SM_SUSPECT` guards (regex heuristics that keep prose from minting fts)
- build the dispatch table from `Classification_has_Translator` rows (grammar data)
- construct the translator operand `⟨inner, mfield, ctx, D⟩` and the per-statement loop

CANON / native (already in the Rust):
- `classify_all_via_M(gD, ...)` — classification is `run_rules` over the ingested grammar;
  the rules classify, not regex order. The Rust has `op_run_rules`.
- each translator `_apply(_A(t), operand)` — dispatch through DEFS (rho); `t` is a canon
  DEF name reduced by the reducer. The Rust has the reducer.
- `reading_parse`/`ftid`/`clause_ft` inside the translators — reducible (see above).

So `op_compile_model` ≈ `op_run_rules` in shape: read `text` from `j`, build the grammar
dispatch, per statement classify natively → dispatch the translator DEF natively → store,
and return the seed's report contract `{total, kinds, unparsed, unclassified, prose}`.

## The gap #2: the downstream derive phases

After `compile_model` the pipeline runs (from `Registry.compile`, protocol.py:~1713):
run_rules (post-model) → status_facts → create_handlers → sql-project → machine_fold →
replay → save. `run_rules` and `save` (store_into) are already native. The rest is being
mapped phase-by-phase for canon-reducible vs needs-native-Rust:

Phase map (Registry.compile order; verdicts from the reader pass):

| phase (fn) | verdict | native-Rust need |
|---|---|---|
| read+base (protocol.py:1729, ingest_frozen) | HOST small | file IO + frozen-snapshot thaw |
| compile_model g-loop (compiler.py:2049) | MIXED host-dominated | the driver + the translators (see below) |
| run_rules post-model (engine.py:1200) | **CANON — DONE natively** | is `op_run_rules` (3133) |
| status_facts (engine.py:3499) | MIXED | thin glue; re-runs the g-loop — rides the g-loop |
| replay (protocol.py:251) | HOST | buffered installs/retract/migrate over the store |
| machine_fold (engine.py:2763) | HOST | deterministic transition walk (control flow) |
| layout_cells (engine.py:1691) | MIXED | `system:partition` canon + host row-assembly |
| scheduler_cells (engine.py:1795) | MIXED | `system:pass_order`/`pass_bound` canon + host |
| create_handlers (engine.py:3150) | MIXED | handler bodies are canon (`build_system`) + host loop |
| generator_cells (engine.py:1823) | HOST low-value | `dsl:` summaries — defer |
| save+sidecar (protocol.py:432,1812) | HOST small | serialization; sidecar format already read by resident |
| sql-project (protocol.py:832) | HOST | RMAP + CREATE/INSERT — sqlite backend only |

## CORRECTION to the reframe (the reader found this; do not re-optimism it)

The translator BODIES are NOT canon yet. The 16 `translate_*` closures resolve
`_apply(_A(t), operand)` to a **host Python closure** `_stmt_translator_impl`
(compiler.py:1935), running regex `_productions()` + the `_plan`/`_h_*` planners
(compiler.py:1810-2025). So today `reduce_over(srv, atom(t), ...)` would NOT run them —
the Rust would have to re-implement 16 regex-heavy translators. THIS is why #18 is the
LINCHPIN of #20, not a parallel nicety: only after the `_h_*` bodies are canon does the
Rust dispatch them by reduction and the driver become pure glue.

What IS already canon-reducible in the g-loop: CLASSIFICATION — `classify_all_via_M`
asserts stage-1 fields (`stage1_fields`, native twin at main.rs:817) then derives
`Statement_has_Classification` via `run_rules` (native). And `reading_parse`/`ftid`/
`clause_ft` (system.canon 4656/4675/5093). So classify is free; the translator bodies and
the driver control-flow are the host surface.

## Canonization frontier (the #18 work-list, pinned)

Python has 39 `_h_*` translators (compiler.py:739-1768). Canon so far (~10):
h_objectification (5506), h_sm_def/initial/from/to/emit/moore (5511-5521), h_ref_scheme
(5525), h_meta_data_type/ref_mode (5529-5531). Remaining frontier includes the constraint
family (uniqueness, mandatory, frequency, ring, subset/equality, set_comparison), the
entity/value pair (need `_name_refmode`), the fact/subtype handlers, and the two residual
SM ones **`_h_sm_trigger`/`_h_sm_guard`** (compiler.py:1377-1381) — identical to the
canon `h_sm_from` shape except field 2 is wrapped in `system:clause_ft` (already canon) with
the `known` context threaded in. Each remaining one carries a real dependency (a parse
helper, a `C.*` constraint builder, or context threading) — careful canon authoring, fleet-
certified, not mechanical copies.

## The two real #20 levers (measured)

Cross-referencing the phase map with the 56s null-app breakdown
(`docs/2026-07-10-incremental-compile.md`): the g-loop is only 3.7s of 56s. The dominant
cost is the derive phases re-deriving frozen-invariant base state (create_handlers 13.6s,
run_rules 12.6s, sql-project 11.9s). So:

1. **Incremental freeze** (biggest perf win, ~40s->seconds): freeze the base's DERIVED
   artifacts, pay only the app delta. Host-independent compile-STRATEGY change (not a Python
   micro-opt — "all platforms must be performant"). Designed in the incremental-compile doc.
2. **Native compile** (deployability — no Python in a Rust env): gated on #18 canonizing the
   translators (this doc). Removes the process hop; only ~3.7s of raw perf but it is the
   "compile without Python" requirement.

Both are needed to hit "compile in seconds once schema is in memory": the freeze removes the
40s, canon+native remove the process hop and keep each reduction cheap.

## Integration + the twin strategy (already proven for `lex`)

Add `fn op_compile_model(j, srv) -> Result<String,String>` following `op_run_rules`, and
change the `"compile"` arm (main.rs:5948) to call it as the native PRIMARY, keeping the
Python `delegate_call` as the certified-equal FALLBACK / differential oracle during
migration. This is the exact twin pattern the engine already uses: canon defines the
meaning, the host carries a fast twin, the 4-host differential fleet enforces byte-equality
as the acceptance gate. No second registry, no semantic fork — a performance twin.

## Why this is the #20 production win

- Eliminates the Python delegation — a Rust-configured environment compiles with no Python
  (the directive: "compile should not be Python-specific").
- Compile becomes native reduction over the resident canon + the frozen base, so it lands
  the "compile + validate in seconds once schema is in memory" target natively.
- Pairs with the two host-independent strategy wins already recorded: the incremental
  freeze (`docs/2026-07-10-incremental-compile.md`, pay only the app delta) and the canon
  algorithmic fixes (`ast:Pop` O(n^2)->O(position), 4bcbdbcf). Canon makes each reduction
  cheap; incrementality makes the reductions few; native removes the process hop.

## Convergence with #18

#18 (canonize the `_h_*` translators) and #20 (Rust-native compile) are ONE effort: every
translator that canonizes is one the Rust reducer runs for free, shrinking `op_compile_model`
toward pure glue. The driver is the residual host layer that survives canonization (text
splitting, modality, the classify/dispatch loop) — bounded and stable.

## Rust-side driver internals (idioms confirmed against main.rs)

- apply a translator DEF `t` to the operand: `reduce_over(srv, atom(Leaf::S(t)), operand,
  fuel)` (2821) returns the new `D` — the direct analog of Python `D = _apply(_A(t),
  operand)`. Correctness path first; the `NEval` native carrier (`native_verbalize`, 2833:
  build `NEval{cells,process,defs_n,fuel}` and `ev.mu(napp(napp(A(name),arg),..))`) is the
  ~40x fast path for the hot dispatch loop once it is proven equal.
- CLASSIFY has no native function yet (grep: only a `classify_heads` comment at 3516). It is
  assembled in `op_compile_model` from primitives that all exist: lex each statement → store
  its fields → run the grammar's classification rules through the `op_run_rules` machinery
  (BATCH — one derive for every statement's fields, the stratum-4 discipline, not one lfp per
  statement) → read each statement's classification set back from the derived cell. This is
  the crux of the driver's complexity; it is wiring of `lex`/`store_into`/`op_run_rules`, not
  a missing capability.
- the rest is small host string-work: `statements(text)` (split), `_split_modality`
  (alethic/deontic + sign + inner), the Prose/`_SM_SUSPECT` guards, and the report assembly
  to the seed contract.

## Build order

1. `op_compile_model` driver — port `compile_model_selfhost` (unblocks native compile_model),
   test byte-equal against the Python `compile` via the differential fleet.
2. The downstream phases the map flags HOST-CODE, reusing `op_run_rules`/`store_into` for the
   canon-reducible ones.
3. Flip the `"compile"` arm to native-primary once the fleet is green; retain Python as the
   oracle behind a flag.
