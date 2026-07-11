# Native pipeline tail: rules wiring, report contract, base seed (#20)

The slices between the landed fold (719bb5cb) and the flip, in dependency
order. Each is mechanical against the cited Python source; the judgment
calls are made here.

## 1. rekey + rules wiring (op_compile_model's missing tail)

Python: `compile_model` (compiler.py:2478) = `compile_model_selfhost` (the
landed fold's boundary) + `system.rekey_transitions(D2)` + the rep dict.
Then the PIPELINE (protocol.py:1760+) runs `system.run_rules(D)` on the
result (post-model fixpoint), and onward per the phase order.

- Port `rekey_transitions` (engine.py — machine-scope transition identity;
  read its body, it is a bounded cell rewrite) OR evaluate it if it is
  canon-backed — check for a `system:` def first, same as the fold's
  discovery pattern.
- Wire the EXISTING native op_run_rules fixpoint over the folded store as
  the post-model derive. Identity-skip: if the fold produced no cells
  beyond the seed, skip (the probe-app case).
- Differential boundary for this slice: Python `compile_model` (with
  rekey) + `run_rules` vs native, same write_v byte compare, same ten
  corpora + one app with machines (identity has one — its rekey actually
  fires).

## 2. The report contract (rep JSON parity)

Python's rep (compiler.py:2490): `{"total": len(statements(text)),
"kinds": {}, "unparsed": rep["unclassified"], "prose": rep["prose"],
"rule_diagnostics": [tuple rows of the ruleDiag pop]}`. The pipeline
(protocol.py) then adds `"projected"` (only when the storage driver has
sql — op_sql_project already answers this natively at byte parity) and
`"app"`.

- `total` counts statements(text) — the SAME segmentation the dispatch
  walks; the native op already computes it (the trace's total).
- `kinds` is the EMPTY dict in the selfhost path (seed leftover) — emit
  `{}` verbatim, do not innovate.
- `rule_diagnostics` = the ruleDiag population AFTER rules, as rows.
- Acceptance: json-equal (key order canonicalized) against Python's rep
  for the ten corpora + identity; `projected` compares via the existing
  sql_project parity.

## 3. model-D seed (the native base thaw)

Python: every app compile seeds from `_base_D()` (protocol.py:1730) =
the base readings dir compiled once through `persist.ingest_frozen`
(protocol.py:185) — a content-keyed sqlite snapshot cache; the key hashes
the readings text AND the engine fingerprint, so invalidation is by
construction.

The Rust twin does NOT read Python's cache format. It mirrors the
CONSTRUCTION:
- Compile the base readings corpus (shared/base dir — the registry's
  `base_dir`) through the native pipeline (slices 1+2 above) ONCE.
- Persist the result as `base.store.json` (the store's write_v/JSON dump,
  the same shape the serve loop's `{"d":...}` preamble accepts) beside the
  grammar sidecar, keyed the same way: content hash of the base readings +
  the binary's build fingerprint in a sibling `.key` file (or embedded
  field). Mismatch → recompile and rewrite (tmp-then-rename).
- App compiles seed their fold from the thawed base store exactly as
  `context_from:"resident"` already works — the identity corpus proved
  that path at byte parity.
- ACCEPTANCE: the natively-seeded base store byte-equals Python's
  `_base_D()` dump (write the Python dump via a fold_pydump variant that
  loads `_base_D` instead of compiling a corpus). Then one app corpus
  compiled atop each host's own seed must byte-equal END-TO-END.
- Regen rule (the sidecar lesson, 310404b4): base.store.json regenerates
  whenever base readings or the engine change — the key makes staleness
  impossible to miss, unlike the grammar sidecar incident.

## Sequencing note

machine_fold and the rest of the protocol tail (status_facts → replay →
machine_fold → layout/scheduler/generator → create_handlers → save) ride
docs/2026-07-11-machine-fold-port-spec.md and slot AFTER slice 1 (they
need the post-model fixpoint). The flip needs all of: slices 1–3, the
machine-fold tail, then the fleet gate (full suite + the apps corpus
compiled on both hosts, differential).
