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
    # negative forms (constraint verbalization paper): map to the SAME constraint as the positive twin
    ("neg_uniqueness", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) more than one (.+)\.$")),
    ("neg_mandatory", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) no (.+)\.$")),
    ("disjunctive_mandatory", re.compile(r"^[Ff]or each (.+?), (.+ or .+)\.$")),
    ("inverse_uc", re.compile(r"^[Ff]or each (.+?), at most one (.+) (?:that|those) .+\.$")),
    ("subset", re.compile(r"^[Ii]f (.+) then (.+)\.$")),                      # 'if A then B' = subset (modus ponens)
    ("equality", re.compile(r"^(.+) if and only if (.+)\.$")),                # 'A iff B' = equality
    ("neg_uniqueness", re.compile(r"^any (.+?) more than one (.+)\.$")),      # neg of 'each A .. at most one B'
    ("neg_mandatory", re.compile(r"^any (.+?) no (.+)\.$")),                   # neg of 'each A .. some B'
    ("disjunctive_mandatory", re.compile(r"^[Ee]ach (.+ or .+)\.$")),         # inclusive-or / disjunctive mandatory
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
    """The leading object type of a reading + the remainder (a find over known types — the string
    boundary): used by negation/inverse-uc where only the subject is needed."""
    for k in known:
        if text == k or text.startswith(k + " "):
            return k, text[len(k):].strip()
    first = text.split(" ", 1)
    return first[0], (first[1] if len(first) > 1 else "")


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


def _reading(text, known):
    """A fact-type reading → (template, roles): a mixfix predicate template with {i} placeholders
    plus the ordered role object types (the paper's field-replacement model). Scans left to right,
    replacing each known type (longest, word-bounded) with a placeholder; front text, inter-object
    text, and trailing text remain in the template, so unary, binary and n-ary readings, front
    text ('the birth of {0} occurred in {1}'), and hyphen binding ('adj-Type') all parse."""
    kset = sorted(known, key=lambda k: -len(k.split()))
    toks, roles, out, i = text.split(), [], [], 0
    while i < len(toks):
        tok = toks[i]
        if "-" in tok and not tok.endswith("-"):             # forward hyphen binding: adj-Type -> role Type
            _pre, _, post = tok.partition("-")
            if post in known:
                roles.append(post); out.append("{%d}" % (len(roles) - 1)); i += 1; continue
        matched = next((k for k in kset if toks[i:i + len(k.split())] == k.split()), None)
        if matched:
            roles.append(matched); out.append("{%d}" % (len(roles) - 1)); i += len(matched.split())
        else:
            out.append(tok); i += 1
    return " ".join(out), roles


def _ftid_from(template, roles):
    """A stable fact-type id: the template with its role types substituted back in, slugified."""
    s = template
    for i, r in enumerate(roles):
        s = s.replace("{%d}" % i, r)
    return re.sub(r"[^0-9A-Za-z]+", "_", s).strip("_")


def _role_facts(ft, roles):
    return [("role", (ft + "." + str(i + 1), ft, i + 1, r)) for i, r in enumerate(roles)]


def _fact_type(reading, known):
    """A reading → (ftid, assertions) declaring the fact type (template) and its roles in M."""
    template, roles = _reading(reading, known)
    ft = _ftid_from(template, roles)
    return ft, [("factType", (ft, template))] + _role_facts(ft, roles)


# NORMA derivation-storage markers (ORMCore.dsl / ORMDiagram.resx: '{0} *' etc.), trailing a fact
# type / object type name. They link the fact type to its derivation and storage methods:
#   *  Derived                     — population from derive (lfp F_S) on demand; nothing stored
#   ** DerivedAndStored            — derive materializes into the cell (kept in sync)
#   +  PartiallyDerived            — asserted facts augmented by derive on demand (semiderived)
#   ++ PartiallyDerivedAndStored   — asserted + derived, materialized
_DERIVATION = [(" **", "derived-and-stored"), (" ++", "partially-derived-and-stored"),
               (" *", "fully-derived"), (" +", "semi-derived")]


def _strip_derivation(text):
    """(derivation-storage kind, name-without-marker) — None if the name carries no marker."""
    for mark, kind in _DERIVATION:
        if text.endswith(mark):
            return kind, text[:-len(mark)].strip()
    return None, text


# NORMA value specs → a value constraint object over role 1. A pattern table (regex is the string
# boundary); the first match's builder wins, else an enumeration. No if/elif dispatch.
_VALUE_SPECS = [
    (re.compile(r"^\[(.+?)\.\.(.+?)\]$"), lambda gp: C.value_range(1, _num(gp[0]), _num(gp[1]))),
    (re.compile(r"^at least (.+?) to at most (.+)$"), lambda gp: C.value_range(1, _num(gp[0]), _num(gp[1]))),
    (re.compile(r"^at least (.+?) (?:to|and) below (.+)$"), lambda gp: C.value_range(1, _num(gp[0]), _num(gp[1]), hi_open=True)),
    (re.compile(r"^above (.+?) to at most (.+)$"), lambda gp: C.value_range(1, _num(gp[0]), _num(gp[1]), lo_open=True)),
    (re.compile(r"^above (.+?) (?:to|and) below (.+)$"), lambda gp: C.value_range(1, _num(gp[0]), _num(gp[1]), lo_open=True, hi_open=True)),
    (re.compile(r"^at least (.+)$"), lambda gp: C.value_range(1, lo=_num(gp[0]))),
    (re.compile(r"^above (.+)$"), lambda gp: C.value_range(1, lo=_num(gp[0]), lo_open=True)),
    (re.compile(r"^at most (.+)$"), lambda gp: C.value_range(1, hi=_num(gp[0]))),
    (re.compile(r"^below (.+)$"), lambda gp: C.value_range(1, hi=_num(gp[0]), hi_open=True)),
]


def _value_constraint(spec):
    spec = spec.strip()
    hit = next(((pat.match(spec), build) for pat, build in _VALUE_SPECS if pat.match(spec)), None)
    return hit[1](hit[0].groups()) if hit else \
        C.value_enumeration(1, tuple(_num(v) for v in re.split(r",| and ", spec) if v.strip()))


# ---- planning: (kind, groups, modality) + known → (assertions, constraints) ----
# Each reading kind is planned by its own handler (g, known, modality) -> (assertions, constraints).
# Dispatch is by key into this table (application/reflection), never an if/elif chain.
_slug = lambda s: re.sub(r"[^0-9A-Za-z]+", "_", s).strip("_")


def _h_entity(g, k, m):
    return [("instanceOf", (g[0], "ObjectType"))], []

def _h_value(g, k, m):
    return [("instanceOf", (g[0], "ValueType"))], []

def _h_ref_scheme(g, k, m):
    return [("instanceOf", (g[0], "ObjectType")), ("instanceOf", (g[1], "ValueType")),
            ("refScheme", (g[0], g[1]))], []

def _h_objectification(g, k, m):
    return [("instanceOf", (g[1], "ObjectType")), ("objectification", (g[1], g[0]))], []

def _h_meta(cell):
    return lambda g, k, m: ([(cell, (g[0],))], [])             # data_type / ref_mode metadata

def _h_value_constraint(g, k, m):
    return [("valueConstraint", (g[0], g[1], m))], [(g[0] + "_vc", _value_constraint(g[1]))]

def _h_uniqueness(g, k, m):
    ft, facts = _fact_type(g[0] + " " + g[2], k)               # mixfix template + roles
    also = {"exactly one": [("constraint", (ft + "_mand", "mandatory", ft, m))]}.get(g[1], [])
    return facts + [("constraint", (ft + "_uc", "uniqueness", ft, m))] + also, \
        [(ft + "_uc", C.uniqueness([1]))]                      # the 'Each A' role is unique

def _h_mandatory(g, k, m):
    ft, facts = _fact_type(g[0] + " " + g[1], k)
    return facts + [("constraint", (ft + "_mand", "mandatory", ft, m))], []

def _h_neg_uniqueness(g, k, m):
    ft, facts = _fact_type(" ".join(g), k)                     # reconstruct the reading; same constraint
    return facts + [("constraint", (ft + "_uc", "uniqueness", ft, m))], [(ft + "_uc", C.uniqueness([1]))]

def _h_neg_mandatory(g, k, m):
    ft, facts = _fact_type(" ".join(g), k)
    return facts + [("constraint", (ft + "_mand", "mandatory", ft, m))], []

def _h_spanning(g, k, m):
    ftn = g[0].replace(" ", "_")
    return [("constraint", (ftn + "_uc", "spanning_uniqueness", ftn, m))], [(ftn + "_uc", C.uniqueness([1, 2]))]

def _h_set_comparison(g, k, m):
    subj, mode, body = g
    n = len([c for c in body.split(";") if c.strip()])
    kind = {"exactly": "exclusive_or", "at most": "exclusion"}[mode]
    cid = _slug(subj) + {"exactly": "_xor", "at most": "_excl"}[mode]
    obj = {"exactly": C.exclusive_or, "at most": C.exclusion}[mode]()
    return [("constraint", (cid, kind, subj, n, m))], [(cid, obj)]

def _h_disjunctive(g, k, m):
    body = g[-1]
    n = len([d for d in body.split(" or ") if d.strip()])
    cid = "ior_" + _slug(g[0] if len(g) > 1 else body)[:40]
    return [("constraint", (cid, "disjunctive_mandatory", n, m))], [(cid, C.inclusive_or())]

def _h_subset(g, k, m):
    ante, cons_txt = g
    conseq, _, _where = cons_txt.partition(" where ")         # a 'where' join condition, if present
    cid = "subset_" + _slug(ante)[:40]
    return [("constraint", (cid, "subset", ante[:60], conseq[:60], m))], [(cid, C.subset())]

def _h_equality(g, k, m):
    cid = "eq_" + _slug(g[0])[:40]
    return [("constraint", (cid, "equality", g[0][:60], g[1][:60], m))], [(cid, C.equality())]

def _h_negation(g, k, m):
    a, pred = _subject(g[0], k)
    return [("negation", (a, pred + " " + g[1]))], []

def _h_possibility(g, k, m):
    return [("possibility", (g[0][:80], m))], []

def _h_inverse_uc(g, k, m):
    a, _r = _subject(g[0], k)
    return [("constraint", (_slug(a) + "_inv_uc", "uniqueness", a, m))], []

def _h_fact(g, k, m):
    kind, reading = _strip_derivation(g[0])                    # NORMA */**/+/++ derivation-storage marker
    ft, facts = _fact_type(reading, k)                         # mixfix template + ordered roles
    deriv = [("derivation", (ft, kind))] if kind else []      # link the fact type to its derivation/storage
    return facts + deriv, []


_PLAN = {
    "entity_type": _h_entity, "value_type": _h_value, "ref_scheme": _h_ref_scheme,
    "objectification": _h_objectification, "data_type": _h_meta("data_type"), "ref_mode": _h_meta("ref_mode"),
    "value_constraint": _h_value_constraint, "uniqueness": _h_uniqueness, "mandatory": _h_mandatory,
    "neg_uniqueness": _h_neg_uniqueness, "neg_mandatory": _h_neg_mandatory, "spanning_uc": _h_spanning,
    "set_comparison": _h_set_comparison, "disjunctive_mandatory": _h_disjunctive,
    "subset": _h_subset, "equality": _h_equality,
    "negation": _h_negation, "possibility": _h_possibility, "inverse_uc": _h_inverse_uc,
    "fact_type_reading": _h_fact,
}


def _plan(kind, g, known, modality="alethic"):
    """Dispatch the reading kind to its handler (application by key), never an if/elif chain."""
    return _PLAN.get(kind, lambda g, k, m: ([], []))(g, known, modality)


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
