# model_d fold → Rust: port spec (#20, the slice after cooks)

op_compile_model's translation layer (cooks.rs, 310404b4) emits per-statement
⟨asserts, objs⟩ at zero divergence. This spec is the FOLD that turns those
fires into the store: today Rust's `model_d` starts empty and stays empty;
after this slice the op returns a compiled D.

Python sources of truth (read them first):
- driver: `compiler.py` `_stmt_translator_impl`'s `g()` (~:2240) — per fire,
  asserts fold THEN objs fold, in emission order.
- `initial_D()` — `compiler.py:71`: ONE cell, `⟨CELL, "FILE", ⟨⟩⟩`.
- `run_append(fact, D, cell)` — `engine.py:224`. The per-assert store append.
- `DefineIn(name, obj)` — `engine.py:71` (canonical `ast:DefineIn`): store the
  definition as an ORDINARY cell `⟨CELL, name, obj⟩` via Store's
  replace-top-or-prepend. Definitions travel with the store (Prop. tenant).

## The fold, exactly

Thread D statement by statement in dispatch order; per fire:

1. **asserts** — for each `(cell, fact)` in emission order:
   `D = run_append(fact, D, cell)`. Semantics (engine.py:224, mirror
   precisely):
   - Find the FIRST cell named `cell` (first-match-wins, like srv.cells).
   - Absent → fresh singleton population `(fact,)`, cell PREPENDED to D.
   - Present → dedup by `_eqobj` (TYPE-STRICT: `1 ≠ "1" ≠ 1.0` — the same
     discipline the cook differential enforced); if already a member, the
     population object is REUSED unchanged; else the fact PREPENDS (new fact
     at the HEAD of the population).
   - Either way the cell RE-TOPS: the (possibly new) cell moves to the FRONT
     of the cell list; the cells before it are rebuilt in order; the tail
     after it is shared. Deeper same-named cells survive underneath (Store
     semantics) — first-match-wins reads see the new top.
   - Non-plain contents (population isn't a plain tuple) → Python defers to
     canonical `run`; in Rust this arm should not occur during compile (every
     compile-time cell is plain) — if hit, error loudly rather than diverge.
2. **objs** — for each `(name, obj)` in emission order:
   `D = DefineIn(name, obj)` = Store(name):⟨obj, D⟩ — same
   replace-top-or-prepend cell move as above, but the cell CONTENTS is the
   definition VALUE (a lambda object), not a population. The cooks already
   deliver `obj` as the reduced canonical value (from_lam-equal proven);
   store it as-is.

The final store's CELL ORDER and each population's ROW ORDER are
deterministic consequences of these moves. Do not sort, do not use a map
without an order list — byte parity of the dump is the acceptance.

## Rust shape

- Keep a `Vec<(Leaf, V)>` in cell order (srv.cells' own shape) + the same
  first-match-wins find. The re-top move is: remove first match (O(idx)),
  push_front the new cell. Python's hot cells sit near the front after the
  first append (the g-loop re-tops them), so idx stays small — same
  complexity argument as engine.py:224's docstring.
- Populations as `V` seqs; PREPEND = new row CONS'd on the front.
- `_eqobj` twin: type-strict leaf equality (int/float/string discriminated),
  structural over tuples. The kernel's existing native eq should already be
  this — verify against `kernel.py _eqobj` before reusing.
- Seed: `initial_D()` = one `("FILE", empty-seq)` cell — but note the op's
  `context_from:"resident"` mode starts from the RESIDENT store instead
  (the identity-app differential already exercised that path read-only).

## Pipeline seat

This fold completes compile_model's Stage-1. After it, the op still lacks
run_rules-to-fixpoint over the folded D (op_run_rules exists — wire it),
then the protocol tail per docs/2026-07-11-machine-fold-port-spec.md
(status_facts → replay → machine_fold → layout/scheduler/generator cells →
create_handlers → save).

## Acceptance

Differential per corpus (same ten the cooks used): Python
`compile_model_selfhost` final D dumped via full from_lam; Rust op's folded
store dumped via write_v — compare BYTE-WISE (cell order, row order, value
types). Then compile-time: core.md end-to-end native vs the 4.4s
translation-only datapoint (the fold adds the appends; expect seconds, not
minutes). The identity app atop the resident base is the must-pass
(context_from + subtype lifts + deontic transform all live there).
