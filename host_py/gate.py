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
