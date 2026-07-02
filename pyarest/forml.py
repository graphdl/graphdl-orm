"""compile ∘ parse (D3, Cor. closure): FORML 2 readings — NORMA's verbalization output —
parsed to M-facts and asserted by `create` with the addressed entity being M itself. No
compiler subsystem: compiling a schema is ordinary commands over M's cells (Cor. closure).
`parse` is the string boundary (spec D5).

Grammar based on real NORMA verbalization (VerbalizationCoreSnippets.xml + the constraint
verbalization paper, Halpin & Curland): multi-word names, the Fact Types:/Reference Scheme:/
Data Type: blocks, the quantifiers, the MODAL operators (it is necessary/possible/obligatory/
permitted/forbidden/impossible that), and the multi-line constructs. Modality is first-class:
alethic constraints block commit, deontic ones only flag (AREST Def. Violation / eq. create) —
so each constraint is tagged {alethic|deontic}. Value constraints cover enumerations and
open/closed ranges. Parsing is two-pass over a document; compile_model folds it into M.
"""
import re
from .lam import to_lam, from_lam
from . import ast, system
from . import constraints as C

# ---- statement grouping: accumulate lines until one ends with '.' (multi-line aware) ----
def statements(text):
    out, buf = [], []
    for line in text.splitlines():
        s = line.strip()
        if not s or s == "Fact Types:":
            continue
        buf.append(s)
        if s.endswith("."):
            out.append(" ".join(buf)); buf = []
    if buf:
        out.append(" ".join(buf))
    return out


# ---- modality: strip a leading modal operator, yielding (modality, sign, inner) ----
# alethic = necessity (blocks commit); deontic = obligation (flags only). possibility = the
# ABSENCE of a constraint (informational), not something to enforce (the paper's dual form).
_MODAL = [
    ("It is obligatory that ", "deontic", "positive"),
    ("It is forbidden that ", "deontic", "negative"),
    ("It is permitted that ", "deontic", "possibility"),
    ("It is necessary that ", "alethic", "positive"),
    ("It is impossible that ", "alethic", "negative"),
    ("It is possible that ", "alethic", "possibility"),
]


def _split_modality(stmt):
    for op, mod, sign in _MODAL:
        if stmt.startswith(op):
            return mod, sign, stmt[len(op):].strip()
    return "alethic", "positive", stmt


# ---- classification of the (modality-stripped) inner statement ----
_CLASSIFY = [
    ("entity_type", re.compile(r"^(.+) is an entity type\.$")),
    ("value_type", re.compile(r"^(.+) is a value type\.$")),
    ("ref_scheme", re.compile(r"^Reference Scheme: (.+) has (.+)\.$")),
    ("ref_mode", re.compile(r"^Reference Mode: (.+)\.$")),
    ("data_type", re.compile(r"^Data Type: (.+)\.$")),
    ("value_constraint", re.compile(r"^[Tt]he possible values? of (.+?) (?:are|is) (.+)\.$")),
    ("spanning_uc", re.compile(r"^[Ii]n each population of (.+), each (.+) combination occurs at most once\.$")),
    ("objectification", re.compile(r"^[Tt]his association with (.+) provides the preferred identification scheme for (.+)\.$")),
    ("set_comparison", re.compile(r"^[Ff]or each (.+?), (exactly|at most) one of the following holds: (.+)\.$")),
    ("derivation", re.compile(r"^[Ii]f (.+) then (.+)\.$")),
    ("inverse_uc", re.compile(r"^[Ff]or each (.+?), at most one (.+) (?:that|those) .+\.$")),
    ("uniqueness", re.compile(r"^[Ee]ach (.+?) (at most one|exactly one) (.+)\.$")),
    ("mandatory", re.compile(r"^[Ee]ach (.+?) some (.+)\.$")),
    ("negation", re.compile(r"^(.+) ~(.+)\.$")),
    ("fact_type_reading", re.compile(r"^(.+)\.$")),
]


def analyze(stmt):
    """stmt → (kind, groups, modality). A possibility/permitted statement is the absence of a
    constraint (informational). Otherwise the inner is classified and tagged with its modality."""
    mod, sign, inner = _split_modality(stmt)
    if sign == "possibility":
        return "possibility", (inner.rstrip("."),), mod
    for kind, pat in _CLASSIFY:
        m = pat.match(inner)
        if m:
            return kind, m.groups(), mod
    return "UNPARSED", (inner,), mod


def classify(stmt):
    kind, groups, _mod = analyze(stmt)
    return kind, groups


# ---- two-pass name resolution: split a reading against the known type names ----
def _known(stmts):
    names = set()
    for s in stmts:
        k, g = classify(s)
        if k in ("entity_type", "value_type"):
            names.add(g[0])
        elif k == "ref_scheme":
            names.add(g[0]); names.add(g[1])
        elif k == "objectification":
            names.add(g[1])
    return sorted(names, key=len, reverse=True)


def _subject(text, known):
    for k in known:
        if text == k or text.startswith(k + " "):
            return k, text[len(k):].strip()
    first = text.split(" ", 1)
    return first[0], (first[1] if len(first) > 1 else "")


def _object(text, known):
    for k in known:
        if text == k:
            return k, ""
        if text.endswith(" " + k):
            return k, text[:-(len(k) + 1)].strip()
    last = text.rsplit(" ", 1)
    return last[-1], (" ".join(last[:-1]) if len(last) > 1 else "")


def _ftid(a, pred, b):
    return (a + " " + pred + " " + b).replace(" ", "_")


def _num(s):
    s = s.strip()
    for cast in (int, float):
        try:
            return cast(s)
        except ValueError:
            pass
    return s


def _value_constraint(spec):
    """A NORMA value spec → a value constraint object over role 1: enumeration or open/closed
    range. Ranges: [lo..hi], 'at least L to at most H', '… to/and below H', 'above L …', and the
    one-sided forms."""
    spec = spec.strip()
    m = re.match(r"^\[(.+?)\.\.(.+?)\]$", spec)                          # [lo..hi]
    if m:
        return C.value_range(1, _num(m.group(1)), _num(m.group(2)))
    for pat, kw in [(r"^at least (.+?) to at most (.+)$", (False, False)),
                    (r"^at least (.+?) (?:to|and) below (.+)$", (False, True)),
                    (r"^above (.+?) to at most (.+)$", (True, False)),
                    (r"^above (.+?) (?:to|and) below (.+)$", (True, True))]:
        m = re.match(pat, spec)
        if m:
            return C.value_range(1, _num(m.group(1)), _num(m.group(2)), lo_open=kw[0], hi_open=kw[1])
    for pat, kw in [(r"^at least (.+)$", ("lo", False)), (r"^above (.+)$", ("lo", True)),
                    (r"^at most (.+)$", ("hi", False)), (r"^below (.+)$", ("hi", True))]:
        m = re.match(pat, spec)
        if m:
            v = _num(m.group(1))
            return (C.value_range(1, lo=v, lo_open=kw[1]) if kw[0] == "lo"
                    else C.value_range(1, hi=v, hi_open=kw[1]))
    vals = tuple(_num(v) for v in re.split(r",| and ", spec) if v.strip())
    return C.value_enumeration(1, vals)


# ---- planning: (kind, groups, modality) + known → (assertions, constraints) ----
def _plan(kind, g, known, modality="alethic"):
    A, cons = [], []
    if kind == "entity_type":
        A = [("instanceOf", (g[0], "ObjectType"))]
    elif kind == "value_type":
        A = [("instanceOf", (g[0], "ValueType"))]
    elif kind == "ref_scheme":
        A = [("instanceOf", (g[0], "ObjectType")), ("instanceOf", (g[1], "ValueType")),
             ("refScheme", (g[0], g[1]))]
    elif kind == "objectification":
        A = [("instanceOf", (g[1], "ObjectType")), ("objectification", (g[1], g[0]))]
    elif kind in ("data_type", "ref_mode"):
        A = [(kind, (g[0],))]
    elif kind == "value_constraint":
        name, spec = g
        A = [("valueConstraint", (name, spec, modality))]
        cons = [(name + "_vc", _value_constraint(spec))]
    elif kind in ("uniqueness", "mandatory"):
        subj, rest = (g[0], g[2]) if kind == "uniqueness" else (g[0], g[1])
        a, pred = _subject(subj, known)
        b, _ad = _object(rest, known)
        ft = _ftid(a, pred, b)
        A = [("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
        if kind == "uniqueness":
            A.append(("constraint", (ft + "_uc", "uniqueness", ft, modality)))
            cons.append((ft + "_uc", C.uniqueness([1])))
            if g[1] == "exactly one":
                A.append(("constraint", (ft + "_mand", "mandatory", ft, modality)))
        else:
            A.append(("constraint", (ft + "_mand", "mandatory", ft, modality)))
    elif kind == "spanning_uc":
        ftname = g[0].replace(" ", "_")
        A = [("constraint", (ftname + "_uc", "spanning_uniqueness", ftname, modality))]
        cons.append((ftname + "_uc", C.uniqueness([1, 2])))
    elif kind == "set_comparison":
        subj, mode, body = g
        clauses = tuple(c.strip() for c in body.split(";") if c.strip())
        cid = subj.replace(" ", "_") + ("_xor" if mode == "exactly" else "_excl")
        A = [("constraint", (cid, "exclusive_or" if mode == "exactly" else "exclusion", subj, len(clauses), modality))]
    elif kind == "derivation":
        A = [("derivation", (g[0][:60], g[1][:60]))]
    elif kind == "negation":
        a, pred = _subject(g[0], known)
        A = [("negation", (a, pred + " " + g[1]))]
    elif kind == "possibility":
        A = [("possibility", (g[0][:80], modality))]
    elif kind == "inverse_uc":
        a, _ = _subject(g[0], known)
        A = [("constraint", (a.replace(" ", "_") + "_inv_uc", "uniqueness", a, modality))]
    elif kind == "fact_type_reading":
        a, rest = _subject(g[0], known)
        if rest:
            b, _ad = _object(rest, known)
            pred = rest[:len(rest) - len(b)].strip() if rest.endswith(b) else rest
            ft = _ftid(a, pred, b)
            A = [("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
        else:
            A = [("factType", (g[0].replace(" ", "_"), g[0]))]
    return A, cons


def compile(stmt, D, known=()):
    from .defs import define
    from .reduce import apply as _apply
    from .lam import atom as _A
    kind, g, modality = analyze(stmt)
    asserts, cons = _plan(kind, g, known, modality)
    for cell, fact in asserts:
        D = _apply(_A(2), ast.run(to_lam(fact), D, cell_name=cell))
    for name, obj in cons:
        define(name, obj)
    return D, kind


def compile_model(text, D=None):
    """Fold `compile` over a whole NORMA verbalization into M (two-pass). Returns (D, report)."""
    from . import meta
    from collections import Counter
    if D is None:
        D = meta.initial_D()
    stmts = statements(text)
    known = _known(stmts)
    report, unparsed = Counter(), []
    for s in stmts:
        D, kind = compile(s, D, known)
        report[kind] += 1
        if kind == "UNPARSED":
            unparsed.append(s)
    return D, {"total": len(stmts), "kinds": dict(report), "unparsed": unparsed}


# constraint kinds compiled to a single-population violation object (enforceable on a fact type's
# own population); mandatory/subset/set-comparison need other inputs and are excluded here.
_ENFORCEABLE = {"uniqueness", "spanning_uniqueness", "ring_irreflexive", "ring_symmetric"}


def _cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return list(c[2])
    return []


def validate_for(fact_type, D):
    """Build `fact_type`'s validate from M's constraint facts, respecting modality: alethic
    constraints block commit, deontic ones only flag (AREST Def. Violation). The guard is read
    off M — the constraint names reflect to their objects via rho (Cor. closure), no host table."""
    from .lam import atom as _A
    pairs = [(_A(f[0]), f[-1]) for f in _cells(D, "constraint")
             if len(f) >= 4 and f[2] == fact_type and f[1] in _ENFORCEABLE]
    return system.validate_modal(pairs)


def parse(reading):
    kind, g = classify(reading.strip() if reading.strip().endswith(".") else reading.strip() + ".")
    if kind == "UNPARSED":
        raise ValueError(f"reading outside the fragment R: {reading!r}")
    return kind, g


def nf(reading):
    kind, g = parse(reading)
    if kind == "entity_type":
        return f"{g[0]} is an entity type"
    if kind == "value_type":
        return f"{g[0]} is a value type"
    raise ValueError(f"no normal form for {reading!r}")
