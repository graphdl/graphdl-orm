"""gate — THE ONE GATE (SPEC 2.1–2.5, Def 5/6, Thm 1).

Day-3 build order: first the ABSOLUTE validate sweep (this file), then the
step/commit envelope on top once G4 holds — the sweep judges the whole result
population with no baseline subtraction, which is only usable when the base
satisfies its own schema (SPEC 10.2/G4; the quarry's baseline-delta hack
existed exactly because its base was dirty).

The sweep is transcribed from the quarry Registry.validate (protocol.py,
canon-first 0fa14b7c): walk the DECLARED fact types, apply each one's
validate_for — the same object a create runs — to ⟨P_ft, D⟩ inside D's own
step, and union the non-empty violation sets (Def 6: V_c = (ρ c):P″, alethic
refuses, deontic warns). Registered judge entries (the LLM legs) re-enter
with their Day-6 salvage, not here.
"""

from .lam import to_lam, from_lam
from .reduce import apply as _ap
from . import lam as L


def settle(D):
    """derive_S = lfp(F_S, ·) (Def 5, Lem 1): the store with every derivation
    at its least fixed point. The sweep judges P″, never the raw compile —
    the first G4 run proved it: a parsed mirror rule that never fired left
    the mandatory violated on the un-derived store."""
    from . import system
    return system.run_rules(D)


def sweep(D):
    """V over the settled store: [{fact_type, kinds, offenders, alethic}] for
    every non-empty violation set; [] is a clean bill (G4). The constraint
    kinds ride from the store's constraint A-rows (cid, kind, …scope…,
    modality) for the report's legibility."""
    from . import system, forml, defs
    from .lam import from_lam as _fl

    # THE STORE DECLARES ITS OWN POSTURE (layout_cells: "the partition is
    # knowledge about the store, so it rides in the store as a cell"). A raw
    # compile_model store has no rmapColumns and reads as ALL-OWN-TABLE;
    # validating it through the schema-derived partition builds view members
    # over rows that were never laid — reads past short rows, ⊥ (the G4
    # drill's ('view','Function',47) finding, 2026-07-14). Laid stores get
    # the partition; raw stores validate own-table.
    laid = any(isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"
               and c[1] == "rmapColumns" for c in _fl(D))
    partition = system.rmap_partition(D) if laid else None
    kinds = {}
    for c in system._pop_rows(D, "constraint"):
        if len(c) < 3:
            continue
        scope = c[2:-1] if c[-1] in ("alethic", "deontic") else c[2:]
        for part in scope:
            for t in (part if isinstance(part, tuple) else (part,)):
                if isinstance(t, str):
                    kinds.setdefault(t, set()).add(c[1])
    out = []
    # the M bridge (compiler.M_MAP): five metamodel fact types' populations
    # ride the machinery cells ('factType', 'role', …) until grammar-as-readings
    # unifies cell naming; the checkers' implied members already read the
    # machinery side, so the population fetch must too — one-sided reads were
    # 4 of the 6 alethic families in the 2026-07-14 census.
    from .compiler import M_MAP

    for f in system._pop_rows(D, "factType"):
        if not f:
            continue
        ft = f[0]
        val = forml.validate_for(ft, D, partition)
        pop = tuple(tuple(r) for r in system._pop_rows(D, ft))
        if not pop and ft in M_MAP:
            # the M-named population when it exists (the reflection pass
            # materializes primaries); the machinery cell as the fallback
            pop = tuple(tuple(r) for r in system._pop_rows(D, M_MAP[ft]))
        pair = L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))
        try:
            with defs.step(D):
                ans = from_lam(_ap(val, pair))
            _p, v, flag = ans
        except Exception as e:
            # FAIL CLOSED (Def 5): a validate object that cannot answer the
            # ⟨P, V, flag⟩ triple is a defect, and an unjudgeable population
            # refuses like an alethic violation — never a silent pass.
            out.append({"fact_type": ft, "kinds": sorted(kinds.get(ft, ())),
                        "offenders": [["<validate answered no triple>",
                                       repr(e)[:120]]],
                        "alethic": True})
            continue
        if v:
            out.append({"fact_type": ft,
                        "kinds": sorted(kinds.get(ft, ())),
                        "offenders": [list(x) if isinstance(x, tuple) else [x]
                                      for x in v],
                        "alethic": flag == "T"})
    return out


def alethic(violations):
    """The refusing subset (Def 6): the sweep's entries whose constraint is
    alethic — nonempty means the store does not satisfy its schema."""
    return [v for v in violations if v.get("alethic")]


def compile_serving(text, context_from=None):
    """Compile-for-serving (SPEC 11.1; the posture rule, 2026-07-14): a store
    that will take WRITES is laid — status fact types materialized, the RMAP
    layout as data (rmapColumns rides in the store), derivations settled.
    compile-for-reading stays the raw own-table compile_model."""
    from . import forml, system
    D, rep = forml.compile_model(text, context_from=context_from)
    D = system.status_facts(D)
    D = system.layout_cells(D)
    return settle(D), rep


def _scoped_fts(D, touched):
    """The §6.1 judgment scope: 'the recalculation a write forces is bounded
    to the entity's cell and the role-player cells its constraints can reach,
    a scope the schema fixes at compile time.' From the touched fact types,
    take every constraint whose scope intersects them, then every fact type
    those constraints reach — the exclusion family judges BOTH sides of a
    write. The FULL sweep stays the compile/audit gate (G4)."""
    from . import system
    touched = set(touched)
    judged = set(touched)
    for c in system._pop_rows(D, "constraint"):
        if len(c) < 3:
            continue
        scope = c[2:-1] if c[-1] in ("alethic", "deontic") else c[2:]
        names = set()
        for part in scope:
            for t in (part if isinstance(part, tuple) else (part,)):
                if isinstance(t, str):
                    names.add(t)
        if names & touched:
            judged |= names
    return judged


def sweep_scoped(D, touched):
    """The write gate's judgment (2.2 through the §6.1 bound): the absolute
    sweep restricted to the fact types the step's cells reach. Same checkers,
    same fail-closed posture, smaller universe."""
    from . import system, forml, defs
    from .compiler import M_MAP

    judged = _scoped_fts(D, touched)
    declared = {f[0] for f in system._pop_rows(D, "factType") if f}
    partition = None
    laid = any(isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"
               and c[1] == "rmapColumns" for c in _fl_all(D))
    if laid:
        partition = system.rmap_partition(D)
    out = []
    for ft in sorted(judged & declared):
        val = forml.validate_for(ft, D, partition)
        pop = tuple(tuple(r) for r in system._pop_rows(D, ft))
        if not pop and ft in M_MAP:
            pop = tuple(tuple(r) for r in system._pop_rows(D, M_MAP[ft]))
        pair = L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))
        try:
            with defs.step(D):
                ans = from_lam(_ap(val, pair))
            _p, v, flag = ans
        except Exception as e:
            out.append({"fact_type": ft, "kinds": [],
                        "offenders": [["<validate answered no triple>",
                                       repr(e)[:120]]],
                        "alethic": True})
            continue
        if v:
            out.append({"fact_type": ft, "kinds": [],
                        "offenders": [list(x) if isinstance(x, tuple) else [x]
                                      for x in v],
                        "alethic": flag == "T"})
    return out


def _fl_all(D):
    from .lam import from_lam as _fl
    return _fl(D)


def _retract_row(D, ft, fact):
    """2.5 (Decision D1): retraction removes one ASSERTED row from the fact
    type's log cell — the log is the population the checkers read ("create
    AND retract keep it current", the quarry's query discipline). A fully
    derived (*) fact type refuses: its rows are consequences. The derived
    heads clear and the fixed point recomputes from the shrunk base (correct
    first; DRed incrementality later under 7.3). A laid store's absorbed
    COLUMN refreshes at the next compile — the quarry's supersession
    discipline: on commit the retraction is a log entry and the store
    rebuilds through compile, so materialized views recompute from truth."""
    from . import system, ast
    from .reduce import apply as _apply
    from .lam import atom as _A
    modes = {r[0]: r[1] for r in system._pop_rows(D, "derivation") if len(r) >= 2}
    if modes.get(ft) in ("*", "fully", "derived"):
        return None, f"{ft} is fully derived: a consequence, not retractable (2.5)"
    rows = [tuple(r) for r in system._pop_rows(D, ft)]
    if tuple(fact) not in rows:
        return None, f"no such asserted fact in {ft}"
    kept = tuple(r for r in rows if r != tuple(fact))
    cells = []
    for c in _fl_all(D):
        if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL" and c[1] == ft:
            cells.append(("CELL", ft, kept))
        else:
            cells.append(c)
    D2 = to_lam(tuple(cells))
    # clear derived heads so the lfp recomputes from the asserted base
    heads = {r[1] for r in system._pop_rows(D2, "ruleDerives") if len(r) >= 2}
    if heads:
        cells = []
        for c in _fl_all(D2):
            if (isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"
                    and c[1] in heads):
                cells.append(("CELL", c[1], ()))
            else:
                cells.append(c)
        D2 = to_lam(tuple(cells))
    return D2, None


class Journal:
    """§12: the durable event log. One line per COMMITTED step —
    {"tx": n, "ops": [[op, ft, [fact…]], …]} in arrival order (transaction
    time, 4.2); a refused step appends nothing (12.1). Derived facts never
    journal — they are consequences (2.5)."""

    def __init__(self, path):
        import json, os
        self.path = path
        self.lines = []
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                self.lines = [json.loads(ln) for ln in f if ln.strip()]

    @property
    def tx(self):
        return len(self.lines)

    def append(self, ops):
        import json
        entry = {"tx": self.tx, "ops": ops}
        self.lines.append(entry)
        with open(self.path, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False,
                               separators=(",", ":")) + "\n")


def replay(D, journal):
    """12.2: fold the journal through the SAME gate. Each entry committed
    against its predecessor state and the gate is deterministic, so replay
    reproduces D exactly. An entry that fails replay signals corruption:
    HALT with the report — never silently diverge, never silently resurrect
    (the quarry's events journal resurrected a forbidden fact into every
    rebuilt store, canon-first 181bc775)."""
    for entry in (journal.lines if isinstance(journal, Journal) else journal):
        ops = [(op, ft, tuple(fact)) for op, ft, fact in entry["ops"]]
        res = step(D, ops)
        if not res["committed"]:
            raise RuntimeError(
                f"journal replay halted at tx {entry.get('tx')}: a committed "
                f"step no longer commits — corruption or drift. "
                f"violations: {res['violations']!r}")
        D = res["D"]
    return D


def step(D, ops, journal=None):
    """One AST transition over a batch input (SPEC 2.1–2.5, Thm 1, §12).

    ops: [("create"|"retract", fact_type, fact-tuple), …] — applied together,
    judged together, committed or refused ATOMICALLY (2.4: the input names
    finitely many assertions and retractions; every valid-to-valid move is
    one step, so no sequencing wedge exists). resolve rides engine.create
    (routing + the machine step, Prop 2) and _retract_row (2.5); derive
    settles to the least fixed point (Def 5); the judgment is the one gate
    over the §6.1 scope (sweep_scoped); an alethic violation answers the
    ORIGINAL D (Def 5: 'otherwise leaving D unchanged'). A committed step's
    ops append to the journal; a refused step appends nothing (12.1)."""
    from . import system

    D2 = D
    touched = set()
    for op, ft, fact in ops:
        touched.add(ft)
        if op == "create":
            res = system.create(D2, ft, to_lam(tuple(fact)))
            o = from_lam(_ap(L.atom(1), res))
            if o == "ERROR":
                return {"committed": False, "D": D,
                        "violations": [{"fact_type": ft, "kinds": ["create"],
                                        "offenders": [list(fact)],
                                        "alethic": True}]}
            D2 = _ap(L.atom(2), res)
        elif op == "retract":
            D2, err = _retract_row(D2, ft, fact)
            if err:
                return {"committed": False, "D": D,
                        "violations": [{"fact_type": ft, "kinds": ["retract"],
                                        "offenders": [[err]],
                                        "alethic": True}]}
        else:
            raise ValueError(f"unknown op {op!r}")
    D2 = settle(D2)
    V = sweep_scoped(D2, touched)
    bad = alethic(V)
    if bad:
        return {"committed": False, "violations": V, "D": D}
    if journal is not None:
        journal.append([list(o) if isinstance(o, tuple) else o for o in
                        [[op, ft, list(fact)] for op, ft, fact in ops]])
    return {"committed": True, "violations": V, "D": D2}
