"""Conceptual schema optimization (Halpin book 12.5), decisions as derived facts.

Halpin's procedure transforms a conceptual schema so the standard Rmap yields a more
efficient implementation, and he names four judgment factors: target system, query
pattern, update pattern, clarity. Here each is data, in the pattern this system
already uses for encryption (seal derives modes from data types and constraints) and
table shapes (RMAP derives them from uniqueness constraints):

  * TRIGGERS are constraint patterns M already holds. Step 4's trigger is an
    exclusive family of unaries on one noun; the table-width-guideline/PSG1 trigger
    is a small enumerated role inside an n-ary fact type.
  * THRESHOLDS are declared facts (optThreshold rows), defaulting to Halpin's own
    "reasonable number (e.g., 5)" for enumeration width.
  * The QUERY PATTERN is the measured read log (system.read_pop), host-side like the
    event log; "focused" is a count.
  * CLARITY is moot: the authored M is never rewritten. Halpin sanctions applying
    the transforms "automatically as an invisible, preprocessing stage to Rmap", and
    that is where the apply side will live, behind a population round-trip oracle.

plan(D) is PURE analysis: suggestions ranked focused-first, each citing the M facts
that fired it (the grounds), so a firing is explainable and a disagreement is settled
by changing a declaration or reading the log. The formal lineage: equivalence and the
transformation theorems are Halpin (1989b) and Halpin & Proper (1995); the
objective-over-equivalent-schemas formulation is van Bommel & van der Weide (1992),
both cited from the book's own chapter notes."""
import re

from . import system
from .lam import to_lam, from_lam

_READ_LOG = {}


def read_pop(D, ft, partition=None):
    """THE public population read (a ρ-application over the cell or the absorbed
    view). Logged HERE, host-side, per fact type: like arrival order, the read log is
    the log's and no fact of the domain (Prop. onestep's distinction), which is also
    why it lives in this host-tooling module and not in the canonical system module —
    the optimizer's 'focused queries' are measured counts, not guesses."""
    _READ_LOG[ft] = _READ_LOG.get(ft, 0) + 1
    if partition is not None and partition.get(ft, ft) != ft:
        return system.ft_view(D, ft, partition)
    return {tuple(r) if isinstance(r, tuple) else (r,)
            for r in system._pop_rows(D, ft)}


def read_counts():
    """The measured query pattern: fact type → public reads this process."""
    return dict(_READ_LOG)


def reset_read_log():
    _READ_LOG.clear()


_QUOTED = re.compile(r"'([^']*)'")

_DEFAULTS = {"enum_width": 5}                                 # Halpin's "e.g., 5"


def _thresholds(D):
    t = dict(_DEFAULTS)
    for r in system._pop_rows(D, "optThreshold"):
        if len(r) >= 2:
            try:
                t[r[0]] = int(r[1])
            except (TypeError, ValueError):
                pass
    return t


def _enum_width(spec):
    """The width of an ENUMERATED value constraint, or None when the constraint is a
    range or bound ('at most 5', '1..9') — only a closed enumeration sanctions
    absorption (PSG1 needs the values b1..bn)."""
    if not isinstance(spec, str) or " at " in f" {spec} " or ".." in spec:
        return None
    quoted = _QUOTED.findall(spec)
    if quoted:
        return len(quoted)
    parts = [p.strip() for p in spec.split(",") if p.strip()]
    return len(parts) if parts and all(" " not in p for p in parts) else None


def plan(D, reads=None):
    """Advisory suggestions over M, ranked focused-first. Pure: nothing is rewritten,
    nothing asserted; each suggestion cites its grounds."""
    reads = reads or {}
    roles = [r for r in system._pop_rows(D, "role") if len(r) >= 4]
    arity = {}
    player = {}
    for (_rid, ft, pos, typ) in roles:
        arity[ft] = max(arity.get(ft, 0), pos)
        player[(ft, pos)] = typ
    enums = {}
    for r in system._pop_rows(D, "valueConstraint"):
        if len(r) >= 2:
            w = _enum_width(r[1])
            if w is not None:
                enums[r[0]] = w
    th = _thresholds(D)
    out = []

    # Step 4 (book 12.5): an exclusive family of unaries on one noun generalizes to a
    # single functional binary over the enumerated family; the exclusion becomes the
    # key uniqueness. Trigger: an exclusion/exclusive_or constraint whose clauses are
    # ALL unary fact types sharing their role player.
    for f in system._pop_rows(D, "constraint"):
        if len(f) >= 4 and f[1] in ("exclusion", "exclusive_or") \
                and isinstance(f[3], tuple) and len(f[3]) >= 2:
            fts = tuple(f[3])
            if all(arity.get(ft) == 1 for ft in fts):
                nouns = {player.get((ft, 1)) for ft in fts}
                if len(nouns) == 1:
                    out.append({
                        "kind": "generalize_exclusive_unaries",
                        "noun": next(iter(nouns)),
                        "fact_types": fts,
                        "reads": max(reads.get(ft, 0) for ft in fts),
                        "grounds": {"constraint": f[0], "family": len(fts)},
                    })

    # PSG1 absorption under the table width guideline (book 12.5 steps 2.1/3): a
    # small, closed enumeration playing a role in an n-ary sanctions specializing the
    # predicate by absorbing it. Width bounded by the DECLARED threshold; stability
    # (no enumeration changes in the M log's window) joins the grounds when the
    # M-history wiring lands.
    for ft, a in sorted(arity.items()):
        if a < 3:
            continue
        for pos in range(1, a + 1):
            v = player.get((ft, pos))
            w = enums.get(v)
            if w is not None and w <= th["enum_width"]:
                out.append({
                    "kind": "absorb_enumerated_role",
                    "fact_type": ft,
                    "role": pos,
                    "value_type": v,
                    "reads": reads.get(ft, 0),
                    "grounds": {"valueConstraint": v, "width": w,
                                "threshold": th["enum_width"]},
                })

    return sorted(out, key=lambda s: -s["reads"])


# =====================================================================
# The Backus-level optimizer (merged from rewrite.py, 2026-07-04, the
# fewer-files push: one module owns optimization at both levels, the
# schema level above per Halpin 12.5 and the object level below per
# Backus 12.2). HOST TOOLING by design: a rewritten object is a TWIN of
# a canonical one, held to observational equality, the same contract as
# the FAST overrides; v1 carries only the unconditionally bottom-safe
# laws (composition associativity, identity elimination, CONST
# absorption). The catalog and oracle doctrine live in
# docs/2026-07-03-backus-optimizer-catalog.md.
# =====================================================================


def _is(t, head):
    return isinstance(t, tuple) and len(t) > 0 and t[0] == head


def _flatten_comp(elems):
    out = []
    for e in elems:
        if _is(e, "COMP"):
            out.extend(_flatten_comp(list(e[1:])))
        else:
            out.append(e)
    return out


def rewrite(tree):
    """One bottom-up pass of the ⊥-safe laws over a from_lam object tree."""
    if not isinstance(tree, tuple) or not tree:
        return tree
    t = tuple(rewrite(x) for x in tree)
    if _is(t, "COMP"):
        elems = [e for e in _flatten_comp(list(t[1:])) if e != "id"]   # assoc + III.2
        if not elems:
            return "id"
        if len(elems) == 1:
            return elems[0]
        return ("COMP",) + tuple(elems)
    if _is(t, "COND") and len(t) == 4:
        p, then, els = t[1], t[2], t[3]
        if _is(then, "COND") and len(then) == 4 and then[1] == p:      # II.3.1
            return ("COND", p, then[2], els)
    return t


def twin(obj, operands, step_D=None):
    """Rewrite a compiled object and hold it to observational equality on the given
    operands (the catalog's oracle). Answers the rewritten object as a lambda value,
    or the original when the rewrite changed nothing. Raises on divergence, because
    a twin that diverges is a bug, not a fallback."""
    from . import defs
    from .reduce import apply
    import pyarest.lam as L

    tree = from_lam(obj)
    better = rewrite(tree)
    if better == tree:
        return obj
    cand = to_lam(better)
    D = step_D if step_D is not None else L.SEQ(L.NIL)
    for x in operands:
        with defs.step(D):
            got = from_lam(apply(cand, x))
            want = from_lam(apply(obj, x))
        if got != want:
            raise AssertionError(
                f"twin diverged on {from_lam(x)!r}: {got!r} != {want!r}")
    return cand
