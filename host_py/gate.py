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


def sweep(D):
    """V over the settled store: [{fact_type, kinds, offenders, alethic}] for
    every non-empty violation set; [] is a clean bill (G4). The constraint
    kinds ride from the store's constraint A-rows (cid, kind, …scope…,
    modality) for the report's legibility."""
    from . import system, forml, defs

    partition = system.rmap_partition(D)
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
    for f in system._pop_rows(D, "factType"):
        if not f:
            continue
        ft = f[0]
        val = forml.validate_for(ft, D, partition)
        pop = tuple(tuple(r) for r in system._pop_rows(D, ft))
        pair = L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL)))
        with defs.step(D):
            _p, v, flag = from_lam(_ap(val, pair))
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
