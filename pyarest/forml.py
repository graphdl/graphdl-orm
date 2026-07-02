"""compile ∘ parse (D3, Cor. closure): FORML 2 readings — NORMA's verbalization output —
parsed to M-facts and asserted by `create` with the addressed entity being M itself. There
is no compiler subsystem: compiling a schema is ordinary commands over M's cells, validated
by M's own constraints (Cor. closure). `parse` is the string boundary (spec D5).

The grammar is based on NORMA's verbalization snippets (VerbalizationCoreSnippets.xml): the
quantifiers (each / some / at most one / exactly one / no / that), the modal "it is possible
that", and the family keywords (is identified by; the possible value(s) of … is/are; if … then;
… if and only if; no … may cycle back to itself; is an instance of / is a kind of; is by
definition). A reading maps to (a) facts asserted into M's cells and (b) compiled constraint
objects reflected under a name so (rho c):P = V_c. `compile_model` folds a whole document in.
"""
import re
from .lam import to_lam, from_lam
from . import ast, system
from . import constraints as C

_OUT_OF_R = (" who ", " that has ", " which ")               # pronoun-correlated clauses (Def. Fragment)
_QUANT = r"at most one|exactly one|at least one|some|one|no"

# Each rule: (compiled pattern, kind). The handler for `kind` in `_assert` turns the captured
# groups into M-cell assertions and/or named constraint objects. Order matters (first match).
_RULES = [
    (re.compile(r"^(\w+) is an entity type$"), "entity_type"),
    (re.compile(r"^(\w+) is a value type$"), "value_type"),
    (re.compile(r"^(\w+) is identified by (\w+)$"), "ref_scheme"),
    (re.compile(r"^[Ee]ach (\w+) is an instance of (\w+)$"), "subtype"),
    (re.compile(r"^[Ee]ach (\w+) is a kind of (\w+)$"), "subtype"),
    (re.compile(r"^[Ee]ach (\w+) is by definition (\w+)$"), "subtype"),
    (re.compile(r"^[Tt]he possible values of (\w+) are (.+)$"), "value_constraint"),
    (re.compile(r"^[Tt]he possible value of (\w+) is (.+)$"), "value_constraint"),
    (re.compile(r"^[Nn]o (\w+) (.+?) itself$"), "ring_irreflexive"),
    (re.compile(r"^[Ee]ach (\w+) (.+?) (" + _QUANT + r") (\w+)$"), "binary"),
    (re.compile(r"^[Ii]t is possible that (?:some )?(\w+) (.+?)(?: some| more than one)? (\w+)$"), "possibility"),
]


def _ft(a, pred, b):
    return a + "_" + "_".join(pred.split()) + "_" + b            # a stable fact-type id


def parse(reading):
    """A FORML reading → (kind, groups). Total on the covered grammar; raises on a reading
    outside the fragment R (pronoun-correlated / unrecognized)."""
    text = reading.strip().rstrip(".").strip()
    if any(p in f" {text} " for p in _OUT_OF_R):
        raise ValueError(f"reading outside the fragment R (pronoun-correlated): {reading!r}")
    for pat, kind in _RULES:
        m = pat.match(text)
        if m:
            return (kind, m.groups())
    raise ValueError(f"reading outside the fragment R: {reading!r}")


def _plan(kind, g):
    """(kind, groups) → (assertions, constraints): the M-cell facts to assert and the named
    constraint objects to compile. Object types mentioned are declared idempotently."""
    asserts, cons = [], []
    if kind == "entity_type":
        asserts = [("instanceOf", (g[0], "ObjectType"))]
    elif kind == "value_type":
        asserts = [("instanceOf", (g[0], "ValueType"))]
    elif kind == "ref_scheme":
        ot, vt = g
        asserts = [("instanceOf", (ot, "ObjectType")), ("instanceOf", (vt, "ValueType")),
                   ("refScheme", (ot, vt))]
    elif kind == "subtype":
        sub, sup = g
        asserts = [("instanceOf", (sub, "ObjectType")), ("instanceOf", (sup, "ObjectType")),
                   ("subtype", (sub, sup))]
    elif kind == "value_constraint":
        role, vals = g[0], tuple(v.strip() for v in re.split(r",| and ", g[1]) if v.strip())
        # record the allowed values in M; enforcement compiles once the role's fact type is linked
        asserts = [("valueConstraint", (role, vals))]
    elif kind == "ring_irreflexive":
        a, pred = g[0], g[1]
        ft = _ft(a, pred, a)
        asserts = [("instanceOf", (a, "ObjectType")), ("factType", (ft, pred)),
                   ("constraint", (ft + "_irreflexive", "ring_irreflexive", ft))]
        cons = [(ft + "_irreflexive", C.ring_irreflexive())]
    elif kind == "binary":
        a, pred, quant, b = g
        ft = _ft(a, pred, b)
        asserts = [("instanceOf", (a, "ObjectType")), ("instanceOf", (b, "ObjectType")),
                   ("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
        if quant in ("at most one", "exactly one", "one"):
            asserts.append(("constraint", (ft + "_uc", "uniqueness", ft)))
            cons.append((ft + "_uc", C.uniqueness([1])))
        if quant in ("exactly one", "at least one", "some"):
            asserts.append(("constraint", (ft + "_mand", "mandatory", ft)))
            cons.append((ft + "_mand", C.mandatory()))
    elif kind == "possibility":
        a, pred, b = g
        ft = _ft(a, pred, b)
        asserts = [("instanceOf", (a, "ObjectType")), ("instanceOf", (b, "ObjectType")),
                   ("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
    return asserts, [(n, o) for n, o in cons if o is not None]


def compile(reading, D, constraints=()):
    """compile = create over M: assert one reading's facts into M's cells (validated by M's own
    `constraints`), and reflect its constraint objects under their names. Returns the new D."""
    from .defs import define
    from .reduce import apply as _apply
    from .lam import atom as _A
    asserts, cons = _plan(*parse(reading))
    validate = system.validate_of(list(constraints)) if constraints else None
    for cell, fact in asserts:
        D = _apply(_A(2), ast.run(to_lam(fact), D, validate_obj=validate, cell_name=cell))  # thread D'
    for name, obj in cons:
        define(name, obj)                                       # rho(name) = the violation object
    return D


def compile_model(text, D=None):
    """Fold `compile` over a whole NORMA verbalization (readings separated by '.'/newlines) into
    M — the drop-in path: a modelled schema becomes a live population of M (D3 / Cor. closure)."""
    from . import meta
    if D is None:
        D = meta.initial_D()
    for sentence in re.split(r"(?<=[.])\s+|\n+", text):
        s = sentence.strip()
        if s:
            D = compile(s, D)
    return D


def verbalize(cell_name, fact):
    """The inverse of parse for the declaration slice — an M-fact → its FORML 2 reading."""
    f = fact if isinstance(fact, tuple) else from_lam(fact)
    if cell_name == "instanceOf":
        name, kind = f
        return f"{name} is an entity type" if kind == "ObjectType" else f"{name} is a value type"
    raise ValueError(f"cannot verbalize {cell_name} fact {f!r}")


def nf(reading):
    """Normal form nf = verbalize ∘ parse on the declaration slice — idempotent (Prop. Spec)."""
    kind, g = parse(reading)
    if kind == "entity_type":
        return f"{g[0]} is an entity type"
    if kind == "value_type":
        return f"{g[0]} is a value type"
    raise ValueError(f"no normal form for {reading!r}")
