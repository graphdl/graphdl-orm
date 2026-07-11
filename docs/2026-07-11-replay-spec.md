# replay_entries → Rust: port spec (#20, the last pipeline slice)

Python source of truth: protocol.py:251 (replay_entries) — read it first;
the pipeline seat is protocol.py:1773-1778 (after status_facts, before
machine_fold — the fold consumes replay's installed event facts).

## The four entry arms

1. **Plain entries** (the bulk): buffer rows by fact type; FLUSH via
   - absorbed → bulk_absorbed_install (the mf slice's
     mf_bulk_absorbed_install, replace_keys=false — VERIFY the landed fn
     exposes the non-replace mode; python's default unions), keyed off ONE
     partition computed once per replay (part_box — mirror the memo),
   - own-table → Store(ft) of the rowsorted union.
2. **retract**: flush first, then Store(ft) of the pop minus the exact row
   (type-strict equality).
3. **migrate**: flush first, then one bulk install (absorbed) or one
   rowsorted-union Store (own-table) of entry["facts"].
4. **TRIGGER entries** (fact types in smTrigger): flush first, then fire
   the GATED CREATE — python: create_spec (schema recipe, memoized per ft
   in spec_box) + _create_from_spec (ast.run = build_system §14.3.1 with
   machine/mealy/links/validate/resolve objects, ALL canonical builders).
   THE RUST PATH ALREADY EXISTS: apply_core (main.rs:~8956's native_apply
   calls it) commits live writes through the same create semantics — the
   replay arm should call the SAME create internals apply_core uses (factor
   the shared piece if needed; do NOT write a second create). Log order
   invariant: an entity's initial status lands before its events, which is
   why triggers flush the buffer first.

## Entry source (the sink interface)

The file sink reads <app>/<app>.events.jsonl (EventSink.read; protocol.py
~:381). Rust already appends via append_event — mirror its read: one JSON
object per line, entries in file order. The sink is an interface; the
replay function takes the ENTRY LIST, not a path.

## Wiring

op_compile_model's tail grows the replay phase between status_facts and
machine_fold, gated on the op receiving the app's entries (a new op field
"replay_entries": [...] or a path the host resolves — prefer entries-in-
request to keep the op pure; the MCP apps_compile wrapper reads the sink
and passes them). After replay: run_rules (post-replay, protocol.py:1777)
— note python runs this UNBOUNDED; mirror exactly (the frontier lesson:
do not innovate boundaries).

## Acceptance

The tasks app WITH its event log (the 59 log entries the mf slice
excluded): python boundary = compile → rules → status_facts → replay →
rules → machine_fold → (rules iff changed) → layout_cells; rust the same
via the extended op. Byte-equal store + json-equal rep. The live board's
1,085 statuses are the known-good end state (the mf report's remaining
delta). Plus a retract/migrate synthetic corpus (make one: a small app +
a hand-written events.jsonl exercising all four arms — bank it in tmp as
the standing replay fixture).

## Watermark

persist._with_watermark (protocol.py:364) stamps len(entries) as the
eventWatermark cell (filter old cell, append new — the layout_cells
order discipline). Same slice, trivial.
