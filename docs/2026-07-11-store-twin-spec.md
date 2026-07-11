# The store twin: a fast IStore inside the fixpoint (#35, second half)

Samuel's framing (2026-07-11): "the store itself is the next interface to
twin." The canon fixes the MEANING of cell interaction; the host registers a
fast implementation behind the same observable contract, certified by the
same byte differential as every other twin. The reduction path stays the
oracle.

## Why (the three taxes, measured)

1. Representation: four live views (d Scott SEQ, cells Vec, nd, ncells);
   one store_into = two O(cells) spine walks + a v_to_n of the contents;
   every fixpoint call pays full-store conversions at the boundary.
2. Joins by term reduction: distl materializes n×m pairs, α maps, folds
   filter — a nested-loop join built from allocation. (The 23.1M-reduction
   profile: ~95% native prims — fast steps, quadratic shape.)
3. Set semantics by linear scan: eqobj dedup per append, full-pop decode
   per read, sort per write.

The fixpoint multiplies all three by rounds × rules. tasks: 6–9 min.
The old engine's cells were plain maps with IndexBy hash-joins — its
6-second compiles are this design.

## The observable contract (what the canon fixes — DO NOT reinterpret)

- FIRST-MATCH-WINS reads by cell name over a cell LIST (deeper same-named
  cells survive underneath).
- Store (ast:Store, §13.3.4): pop the topmost same-named cell, PREPEND the
  new cell at the front. This is the fixpoint head-write primitive.
- setcell (reconcile/bulk-install): REPLACE IN PLACE when present, APPEND
  AT THE END when absent. Never moves a cell.
- Set semantics on populations: type-strict equality (1 ≠ "1" ≠ 1.0);
  run_append dedup keeps the existing row; new rows PREPEND.
- Row order: exactly what the mirrored primitives produce (sort_rows'
  "float"<"int"<"str" type-name-then-str key where sorts occur).
- The DUMP (write_v of the final D) is the certification surface: the twin
  must reproduce the exact final cell order and row order.

## The twin — ONE store, no redundant views (Samuel, 2026-07-11)

The whitepaper has ONE D. The four live views (d, cells, nd, ncells) are a
port accident — serializations kept in sync as if they were stores. 3NF's
own point: store once, without redundancy. So:

```rust
struct FastStore {
    // name-key -> stack of populations (top = index 0); rows as Vec<V>
    map: HashMap<String, Vec<Pop>>,
    // the cell ORDER as the dump will emit it: Vec<(name Leaf, stack depth id)>
    order: Vec<(Leaf, u32)>,
    // per (cell, column) lazy hash index for joins; invalidated on write
    idx: HashMap<(String, usize), HashMap<String, Vec<usize>>>,
}
```

- THE STORE IS THE ONLY COPY. The Scott d materializes ONLY at the dump
  boundary; there is no maintained native mirror. NEval grows a
  CELL-PROVIDER seam (an IStore trait: fetch_pop(name) -> rows): when a
  FetchPop-shaped term reduces, the evaluator asks the store — no
  ncells/nd copies, no per-write mirror maintenance. The canon-evaluation
  sites (system:partition, ftpop_absorbed, sm_join …) read through the
  same seam.
- Store: O(1) amortized — remove name's top from order (positions via a
  name→order-slot map or tombstones + periodic compaction), push front.
- setcell: O(1) replace, or push back.
- reads: O(1) to the top pop; pop rows stay decoded (Vec<V>) — no repeated
  from_lam.
- join index: built on first use per (cell, column) per round; a write to
  the cell drops its indexes. Key = the same type-strict key_of the
  delta variants use.

## Where it lands

op_run_rules' round loop (the hot path): store_into/pop_rows/eval_rules
route through FastStore for the duration of the fixpoint; the op's entry
and exit convert once. op_compile_model's fold could adopt the same twin
later (fold_fire's run_append/DefineIn map 1:1) — second step, after the
fixpoint twin certifies.

## Lambdas are data: compile rule bodies to plans (Church–Turing license)

Within the computable layer a lambda term IS a data structure. A rule
body's COMP/ALPHA/distl pipeline is a JOIN PLAN encoded as a term — so
compile it ONCE (per op entry, or materialized at compile time next to
passHeads, schedule-as-data) into a plan struct, and EXECUTE THE PLAN over
FastStore's indexes: hash-join per atom pair (probe the smaller side),
project, filter — materializing only surviving rows. The ruleAtom facts
already carry the extracted join positions (the ~d delta variants read
them today). The term stays as the SPEC and the differential oracle; the
plan is the implementation; extensional equality is the certification.
Shapes the planner does not recognize (aggregates, negation groups,
comparators) keep the reducer path — twin what is hot, keep the canon for
the rest, exactly the DEFS override discipline.

## Entity-scoped deltas (the whitepaper's own grain)

The whitepaper's cell design already shards by entity (noun:id row cells,
the routed write). Use the same grain for derivation invalidation: the
delta sets should name ROWS (dirty entities), not just cells — a
frontier-bounded round then joins only the changed keys' rows against the
indexes instead of re-scanning whole populations. machine_fold's per-entity
walk and the routed create already work at this grain; the fixpoint should
too.

## Certification

1. Micro: property tests of Store/setcell/append order semantics against
   the existing primitives over random cell lists (same final dump).
2. The standing 11-corpus byte differential (mf boundary) — tasks included.
3. Perf gate: tasks-app fixpoint wall time before/after; target the old
   engine's seconds-class.

## Sequencing

After the flip-critical slices (replay, seed, tail ops) — this is a perf
lever, not a correctness gap; the frontier fix (landed 2026-07-11) already
removes the post-fold fixpoint's bulk. But if tasks-scale compiles gate
Samuel's daily loop before the flip, promote it.
