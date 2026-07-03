"""The Backus-level optimizer, first increment: object-to-object rewrites from the
algebra of programs (Backus 12.2; the catalog and its oracle doctrine live in
docs/2026-07-03-backus-optimizer-catalog.md). HOST TOOLING by design: rewrites
never fork the source — a rewritten object is a TWIN of a canonical one, held to
observational equality, the same contract as the FAST overrides.

v1 carries only the unconditionally ⊥-safe laws, the ones whose propositions hold
on every object with no definedness qualification:

* composition associativity (the I-family's spine): ⟨COMP, ⟨COMP, a, b⟩, c⟩ and
  ⟨COMP, a, ⟨COMP, b, c⟩⟩ flatten to ⟨COMP, a, b, c⟩ — Backus writes compositions
  right-associated with parentheses omitted for exactly this reason.
* III.2 (identity): id elements of a composition drop; a unary composition is its
  element; an empty one is id.
* II.3.1 (redundant test): p → (p → f; g); h ≡ p → f; h.

The QUALIFIED laws (I.5 projection elimination and friends) wait for the operand
oracle, since their propositions hold only where the discarded branches are
defined. rewrite() is pure over from_lam trees; twin() rewrites a compiled object
and asserts observational equality on caller-supplied operands before answering,
which is the oracle the catalog demands."""
from .lam import to_lam, from_lam


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
