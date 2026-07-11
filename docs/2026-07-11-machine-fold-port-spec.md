# machine_fold → Rust: port spec (#20 critical path)

The compile pipeline's event fold (protocol.py:1788). Python source of truth:
`engine/python/engine.py:2780` (`machine_fold`), docstring included — read it
first; this spec adds the Rust mapping and the traps, it does not replace the
source.

## The headline: the semantics are ALREADY CANON — evaluate, don't port

Every judgment-bearing piece of the fold is a shared canon def the Rust host
can evaluate today (the evaluator + canon DEFS are resident: main.rs:7394
already does `ev.mu(napp(na("system:sm_join"), ...))` for view_menu):

| Python host binding | Canon def | Python site | What the Rust side does |
|---|---|---|---|
| `sm_triples(D)` | `system:sm_join` | engine.py:1536 | EXISTS — reuse the 7388–7394 pattern verbatim (pops smFrom/smTrigger/smTo → nseq → mu) |
| `rmap_partition(D)` | `system:partition` | engine.py:1658 | `ev.mu(napp(na("system:partition"), D))` → pairs ⟨table, ft⟩ → invert to `HashMap<ft, table>`. MEMOIZE per compile (Python: 28 calls/compile, ~20% of wall time — one `let part = ...` up front, thread it) |
| `table_columns(part, table)` | `system:table_columns` | engine.py:2621 | apply to ⟨table, partition-pairs⟩; returns the absorbed fts in column order |
| `_governed_player(D, ft)` | `system:governed_player` | engine.py:3024 | apply to ⟨ft, D⟩; `()` ⇒ None |
| `ft_view` absorbed read | `system:ftpop_absorbed` (via `ftpop_expr`) | engine.py:2671 | apply; own-table fts short-circuit to the pop; unary fts reshape `(k,"T")` → `(k,)` host-side |

The host-native remainder is mechanical:

1. **`_rowsort`** (engine.py:1101) — deterministic mixed-type row order.
   Key per element: `(type-name, str(value))`. Python type names are the
   contract: `"float" < "int" < "str"` LEXICALLY, so ints sort before
   strings and floats before both. Rust: emit the same key strings
   (`f64`→"float", `i64`→"int", `Leaf::S`→"str") — do NOT sort by Rust
   discriminant order. Ties broken by the string form of the value.
2. **`bulk_absorbed_install`** (engine.py:2725, ~55 lines) — pure cell-list
   surgery: land each ⟨key, value⟩ on the entity's 3NF row (`table:key`
   cell; fresh rows hole-padded with "#", width = 1 + len(cols)), key joins
   the table index cell, ft view cache unions. `replace_keys=true` (the
   fold's mode) PRUNES the installed keys' stale view rows before the
   union — one status per entity. Unary ft ⇒ column value "T".
   The serve loop's cell-mutation machinery (main.rs:8661+, the coherence
   block) shows the resident-index update pattern to follow.
3. **The fold driver itself** (engine.py:2780–2892, ~110 lines) — the greedy
   walk. Direct transcription; the traps are below.

## Driver semantics — the parts a transcription gets wrong

Read the Python loop side-by-side; these are the load-bearing subtleties:

- **Event collection** (engine.py:2807–2818): for each trigger ft, the
  governed player's role POSITION comes from the `role` pop (⟨id, ft, pos,
  player⟩, pos 1-based); an event row's entity key is `row[pos-1]`, skipped
  if `""` or `"φ"`. Events key by `(noun, entity)`, value = list of fts —
  DUPLICATES MATTER (two rows of the same trigger ft = two events).
- **Current status** (2819–2827): read through `ft_view` with the partition —
  the status ft is usually RMAP-absorbed by fold time (status_facts ran
  before, protocol.py:1768).
- **The walk** (2829–2845): per (noun, entity) in SORTED order; events
  sorted; start = current status, else the machine's initial
  (`Status_is_initial_in_State_Machine_Definition` ⟨status, smd⟩ — note
  r[1]→r[0] inversion). Fire the FIRST fireable event (triples match
  `from == cur && trigger == ev`), REMOVE it (`evs.pop(i)`), restart the
  scan; stop when a full scan fires nothing. Unfireable events stay as
  rows — the write path's no-op semantics.
- **Write-iff-ran** (2846–2850): write when `fired_any && cur !=
  current.get(...)` — BUT a round-trip back to a status equal to the
  RECORDED current does not write, while an entity with NO recorded status
  that walks back to the initial DOES write (current.get is None ≠ initial).
  Transcribe the condition exactly; don't "simplify".
- **SM init** (2851–2874): every governed entity with no status row
  materializes the machine's initial. Entity source = the noun's own pop
  UNIONED with role-1 keys of every ft the noun heads (`role` rows with
  pos==1, player==noun, ft != status_ft). Keys sorted BY str (mixed types).
  Skip `""`/`"φ"`/already-have/already-written. The `written` set dedups
  against the walk's writes ⟨sft, entity⟩.
- **The commit** (2875–2892): group by status ft. Absorbed ⇒
  `bulk_absorbed_install(..., replace_keys=true)`. Own-table (partition maps
  sft to itself) ⇒ union-OVERWRITE the pop directly: keep rows whose key ∉
  written keys, union the new rows, `_rowsort`, store the cell.
- **Identity fast path**: no machines (`sm_triples` empty) ⇒ return D
  unchanged, AND the pipeline skips the post-fold run_rules when nothing
  changed (protocol.py:1790 `if D2 is not D`). Rust: return a `changed:
  bool` and gate the post-fold derive on it.

## Pipeline seat (protocol.py:1760–1801 is the order contract)

compile_model → run_rules → **status_facts** → replay → run_rules →
**machine_fold** → (run_rules iff changed) → layout_cells → scheduler_cells
→ generator_cells → create_handlers → save.

- The fold runs BEFORE layout_cells: `rmapColumns` does NOT exist yet.
  Derive the partition via `system:partition` (above) — do not read the cell.
- `status_facts` (the phase before replay) must already be in the Rust
  pipeline for the fold to see absorbed status columns; if it isn't ported
  yet, port it first (engine.py — `def status_facts`) or the fold writes
  into the noun_status wart.
- `layout_cells` (engine.py:1693) is ~15 lines once `system:partition` +
  `system:table_columns` evaluate: rows ⟨table, 2+j, ft⟩ for absorbed fts,
  replace the `rmapColumns` cell wholesale.

## Acceptance

Differential vs Python, per stage: same store in, compare (a) the full cell
list after the fold (byte order matters — `_rowsort` and sorted() walks make
it deterministic), (b) the changed flag. Corpus: the tasks app (the
sm-migration class that motivated the fold — 75 tasks, readings-carried
events), plus one fresh-compile app with machines but no events (pure
SM-init path), plus a no-machine app (identity path). The tasks board's
final statuses are the known-good: 2026-07-08's wedge (all init) is the
regression this fold exists to prevent.
