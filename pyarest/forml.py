"""compile ∘ parse (D3, Cor. closure): FORML 2 readings — NORMA's verbalization output —
parsed to M-facts and asserted by `create` with the addressed entity being M itself. No
compiler subsystem: compiling a schema is ordinary commands over M's cells (Cor. closure).
`parse` is the string boundary (spec D5).

The grammar is based on real NORMA verbalization (VerbalizationCoreSnippets.xml + observed
output): multi-word object names, the `Fact Types:` / `Reference Scheme:` / `Data Type:`
blocks, the quantifiers, the modal "It is possible that" possibility twin, and the multi-line
constructs (`… of the following holds:` exclusion/exclusive-or, `In each population of …
combination occurs at most once` spanning UC, `This association with … provides the preferred
identification scheme for …` objectification, `If … then … where …` derivations).

Parsing is two-pass over a whole verbalization: pass 1 collects declared type names, pass 2
splits readings against them. compile_model folds it all into M and returns a coverage report.
"""
import re
from .lam import to_lam, from_lam
from . import ast, system
from . import constraints as C

_OUT = (" who ",)                                            # currently only in derivations (handled)

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


# ---- classification: (kind, raw) for every statement (total on the observed grammar) ----
_CLASSIFY = [
    ("entity_type", re.compile(r"^(.+) is an entity type\.$")),
    ("value_type", re.compile(r"^(.+) is a value type\.$")),
    ("ref_scheme", re.compile(r"^Reference Scheme: (.+) has (.+)\.$")),
    ("ref_mode", re.compile(r"^Reference Mode: (.+)\.$")),
    ("data_type", re.compile(r"^Data Type: (.+)\.$")),
    ("possibility", re.compile(r"^It is possible that (.+)\.$")),
    ("spanning_uc", re.compile(r"^In each population of (.+), each (.+) combination occurs at most once\.$")),
    ("objectification", re.compile(r"^This association with (.+) provides the preferred identification scheme for (.+)\.$")),
    ("set_comparison", re.compile(r"^For each (.+?), (exactly|at most) one of the following holds: (.+)\.$")),
    ("derivation", re.compile(r"^If (.+) then (.+)\.$")),
    ("inverse_uc", re.compile(r"^For each (.+?), at most one (.+) (?:that|those) .+\.$")),
    ("uniqueness", re.compile(r"^Each (.+?) (at most one|exactly one) (.+)\.$")),
    ("mandatory", re.compile(r"^Each (.+?) some (.+)\.$")),
    ("negation", re.compile(r"^(.+) ~(.+)\.$")),
    ("fact_type_reading", re.compile(r"^(.+)\.$")),
]


def classify(stmt):
    for kind, pat in _CLASSIFY:
        m = pat.match(stmt)
        if m:
            return kind, m.groups()
    return "UNPARSED", (stmt,)


# ---- two-pass name resolution: split a reading against the known type names ----
def _known(stmts):
    """pass 1: the declared object/value/objectified type names (longest first for greedy match)."""
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
    """longest known type that is a prefix of `text`; returns (typeName, rest)."""
    for k in known:
        if text == k or text.startswith(k + " "):
            return k, text[len(k):].strip()
    first = text.split(" ", 1)                               # fallback: the first word
    return first[0], (first[1] if len(first) > 1 else "")


def _object(text, known):
    """longest known type that is a suffix of `text` (an adname may precede); (typeName, adname)."""
    for k in known:
        if text == k:
            return k, ""
        if text.endswith(" " + k):
            return k, text[:-(len(k) + 1)].strip()
    last = text.rsplit(" ", 1)                               # fallback: the last word
    return last[-1], (" ".join(last[:-1]) if len(last) > 1 else "")


def _ftid(a, pred, b):
    return (a + " " + pred + " " + b).replace(" ", "_")


# ---- planning: (kind, groups) + known → (assertions, constraints) ----
def _plan(kind, g, known):
    A = []                                                   # (cell, fact tuple)
    cons = []                                                # (name, constraint object)
    if kind == "entity_type":
        A = [("instanceOf", (g[0], "ObjectType"))]
    elif kind == "value_type":
        A = [("instanceOf", (g[0], "ValueType"))]
    elif kind == "ref_scheme":
        A = [("instanceOf", (g[0], "ObjectType")), ("instanceOf", (g[1], "ValueType")),
             ("refScheme", (g[0], g[1]))]
    elif kind == "objectification":
        roles, obj = g
        A = [("instanceOf", (obj, "ObjectType")), ("objectification", (obj, roles))]
    elif kind in ("data_type", "ref_mode"):
        A = [(kind, (g[0],))]                                # metadata, recorded
    elif kind in ("uniqueness", "mandatory"):
        subj, rest = (g[0], g[2]) if kind == "uniqueness" else (g[0], g[1])
        a, pred = _subject(subj, known)
        b, _ad = _object(rest, known)
        ft = _ftid(a, pred, b)
        A = [("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
        if kind == "uniqueness":
            A.append(("constraint", (ft + "_uc", "uniqueness", ft)))
            cons.append((ft + "_uc", C.uniqueness([1])))
            if g[1] == "exactly one":
                A.append(("constraint", (ft + "_mand", "mandatory", ft)))
        else:
            A.append(("constraint", (ft + "_mand", "mandatory", ft)))
    elif kind == "spanning_uc":
        ftname = g[0].replace(" ", "_")                      # the association's fact type
        A = [("constraint", (ftname + "_uc", "spanning_uniqueness", ftname, g[1]))]
        cons.append((ftname + "_uc", C.uniqueness([1, 2])))  # the role combination is unique
    elif kind == "set_comparison":
        subj, mode, body = g
        clauses = tuple(c.strip() for c in body.split(";") if c.strip())
        cid = subj.replace(" ", "_") + ("_xor" if mode == "exactly" else "_excl")
        A = [("constraint", (cid, "exclusive_or" if mode == "exactly" else "exclusion", subj, len(clauses)))]
    elif kind == "derivation":
        ante, cons_txt = g
        A = [("derivation", (ante[:60], cons_txt[:60]))]
    elif kind == "negation":
        subj, rest = g
        a, pred = _subject(subj, known)
        A = [("negation", (a, pred + " " + rest))]
    elif kind == "possibility":
        A = [("possibility", (g[0][:80],))]
    elif kind == "inverse_uc":
        a, _ = _subject(g[0], known)
        A = [("constraint", (a.replace(" ", "_") + "_inv_uc", "uniqueness", a))]
    elif kind == "fact_type_reading":
        a, rest = _subject(g[0], known)
        if rest:
            b, _ad = _object(rest, known)
            pred = rest[:len(rest) - len(b)].strip() if rest.endswith(b) else rest
            ft = _ftid(a, pred, b)
            A = [("factType", (ft, pred)), ("role", (ft + ".1", ft, 1, a)), ("role", (ft + ".2", ft, 2, b))]
        else:
            A = [("factType", (g[0].replace(" ", "_"), g[0]))]   # unary
    return A, cons


# ---- compile: assert one statement's facts into M and reflect its constraint objects ----
def compile(stmt, D, known=()):
    from .defs import define
    from .reduce import apply as _apply
    from .lam import atom as _A
    kind, g = classify(stmt)
    asserts, cons = _plan(kind, g, known)
    for cell, fact in asserts:
        D = _apply(_A(2), ast.run(to_lam(fact), D, cell_name=cell))
    for name, obj in cons:
        define(name, obj)
    return D, kind


def compile_model(text, D=None):
    """Fold `compile` over a whole NORMA verbalization into M (two-pass). Returns (D, report),
    the report giving per-kind counts and any UNPARSED statements — honest coverage, not a claim."""
    from . import meta
    from collections import Counter
    if D is None:
        D = meta.initial_D()
    stmts = statements(text)
    known = _known(stmts)
    report = Counter()
    unparsed = []
    for s in stmts:
        D, kind = compile(s, D, known)
        report[kind] += 1
        if kind == "UNPARSED":
            unparsed.append(s)
    return D, {"total": len(stmts), "kinds": dict(report), "unparsed": unparsed}


def parse(reading):
    """A single reading → (kind, groups). Raises on UNPARSED (outside the recognized grammar)."""
    kind, g = classify(reading.strip() if reading.strip().endswith(".") else reading.strip() + ".")
    if kind == "UNPARSED":
        raise ValueError(f"reading outside the fragment R: {reading!r}")
    return kind, g


def nf(reading):
    """Normal form on the declaration slice: nf = verbalize ∘ parse, idempotent (Prop. Spec)."""
    kind, g = parse(reading)
    if kind == "entity_type":
        return f"{g[0]} is an entity type"
    if kind == "value_type":
        return f"{g[0]} is a value type"
    raise ValueError(f"no normal form for {reading!r}")
