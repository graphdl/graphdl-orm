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

# ---- statement grouping: accumulate lines until one ends with '.' (multi-line aware).
# The corpus writes NORMA storage markers AFTER the period ('Fact Type has Format. **');
# normalize to the marker-before-period form the derivation stripper reads. ----
_TRAIL_MARK = re.compile(r"^(.*\S)\.\s*(\*\*|\+\+|\*|\+)$")


def statements(text):
    out, buf = [], []
    for line in text.splitlines():
        s = line.strip()
        if not s or s == "Fact Types:":
            continue
        mm = _TRAIL_MARK.match(s)
        if mm:
            s = f"{mm.group(1)} {mm.group(2)}."
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
    # the state-machine readings of the whitepaper §1 listing: a machine is a set of facts
    ("sm_def", re.compile(r"^State Machine Definition '(.+)' is for Noun '(.+)'\.$")),
    ("sm_initial", re.compile(r"^Status '(.+)' is initial in State Machine Definition '(.+)'\.$")),
    ("sm_from", re.compile(r"^Transition '(.+)' is from Status '(.+)'\.$")),
    ("sm_to", re.compile(r"^Transition '(.+)' is to Status '(.+)'\.$")),
    ("sm_trigger", re.compile(r"^Transition '(.+)' is triggered by Fact Type '(.+)'\.$")),
    ("value_constraint", re.compile(r"^[Tt]he possible values? of (.+?) (?:are|is) (.+)\.$")),
    ("spanning_uc", re.compile(r"^[Ii]n each population of (.+), each (.+) combination occurs at most once\.$")),
    # Halpin §7.2: frequency generalizes the spanning form from 'once' to bounded counts
    ("frequency", re.compile(r"^[Ii]n each population of (.+), each (.+) combination occurs (at most|at least|exactly) (\d+) times?\.$")),
    # Halpin §7.3 ring constraints, as the corpus grammar's trailing markers on a reading
    ("ring", re.compile(r"^(.+?) is (acyclic|asymmetric|antisymmetric|intransitive|irreflexive|symmetric)\.$")),
    # subtyping (corpus trailing marker 'is a subtype of'; RMAP step 0 absorbs to the top)
    ("subtype_of", re.compile(r"^(.+) is a subtype of (.+)\.$")),
    ("objectification", re.compile(r"^[Tt]his association with (.+) provides the preferred identification scheme for (.+)\.$")),
    ("set_comparison", re.compile(r"^[Ff]or each (.+?), (exactly|at most) one of the following holds: (.+)\.$")),
    # negative forms (constraint verbalization paper): map to the SAME constraint as the positive twin
    ("neg_uniqueness", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) more than one (.+)\.$")),
    ("neg_mandatory", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) no (.+)\.$")),
    ("disjunctive_mandatory", re.compile(r"^[Ff]or each (.+?), (.+ or .+)\.$")),
    ("inverse_uc", re.compile(r"^[Ff]or each (.+?), at most one (.+) (?:that|those) .+\.$")),
    ("subset", re.compile(r"^[Ii]f (.+) then (.+)\.$")),                      # 'if A then B' = subset (modus ponens)
    ("equality", re.compile(r"^(.+) if and only if (.+)\.$")),                # 'A iff B' = equality
    # the book's rule surface (Halpin ch.2 ex.4 D1): numbered variables, ' if ' head-body,
    # ' and ' conjunction; a digit in the head keeps plain readings out of this recognizer
    ("rule_if", re.compile(r"^(\S.*?\d\S*.*?) if (.+)\.$")),
    # a derivation RULE reading (leading * = derived): a linear role path from a root object type
    # (infosci Mapping_ORM_to_Datalog: *Each FastCarDriver is some Person who drives some Car ...)
    ("derivation_rule", re.compile(r"^\*Each (.+?) is some (.+?) who (.+)\.$")),
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
            names.add(_name_refmode(g[0])[0])                 # strip a (.RefMode) parenthetical
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


def _role_path(body):
    """A linear role-path body -> ordered hops [(verb, type|None)]: 'drives some Car that is fast'
    -> [('drives','Car'), ('is fast', None)]. Split on the ' that '/' who ' navigation connectives;
    a hop 'V some T' is a step to object type T via predicate V, else a unary/property hop."""
    hops = []
    for part in re.split(r" that | who ", body):
        m = re.match(r"^(.+?) some (.+)$", part.strip())
        hops.append((m.group(1), m.group(2)) if m else (part.strip(), None))
    return hops


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


_REFMODE = re.compile(r"^(.+?)\(\.(.+)\)$")                   # Name(.RefMode), per the whitepaper


def _name_refmode(text):
    m2 = _REFMODE.match(text.strip())
    return (m2.group(1), m2.group(2)) if m2 else (text.strip(), None)


def _h_entity(g, k, m):
    name, rm = _name_refmode(g[0])
    return [("instanceOf", (name, "ObjectType"))] + ([("refMode", (name, rm))] if rm else []), []

def _h_value(g, k, m):
    name, rm = _name_refmode(g[0])
    return [("instanceOf", (name, "ValueType"))] + ([("refMode", (name, rm))] if rm else []), []

def _h_ref_scheme(g, k, m):
    return [("instanceOf", (g[0], "ObjectType")), ("instanceOf", (g[1], "ValueType")),
            ("refScheme", (g[0], g[1]))], []

def _h_objectification(g, k, m):
    return [("instanceOf", (g[1], "ObjectType")), ("objectification", (g[1], g[0]))], []

def _h_meta(cell):
    return lambda g, k, m: ([(cell, (g[0],))], [])             # data_type / ref_mode metadata

def _h_value_constraint(g, k, m):
    # enforced BOTH as a named object and on the value type's own cell (validate_for kind 'value')
    return [("valueConstraint", (g[0], g[1], m)), ("constraint", (g[0] + "_vc", "value", g[0], m))], \
        [(g[0] + "_vc", _value_constraint(g[1]))]


def _mandatory_parts(ft, subject, m, pos=1):
    """The M-fact + spans + the two attachment objects of one mandatory constraint:
    fact-side (entities read from the subject type's cell) and entity-side (facts from ft)."""
    cid = ft + "_mand"
    return [("constraint", (cid, "mandatory", ft, subject, m)), ("spans", (cid, pos))], \
        [(cid, C.scoped_mandatory_entities(subject)), (cid + "_e", C.scoped_mandatory_facts(ft))]


def _h_uniqueness(g, k, m):
    reading = g[0] + " " + g[2]
    ft, facts = _fact_type(reading, k)                         # mixfix template + roles
    _t, rtypes = _reading(reading, k)
    subject = _subject(g[0], k)[0]
    pos = rtypes.index(subject) + 1 if subject in rtypes else 1   # computed, not assumed
    also, aobjs = _mandatory_parts(ft, subject, m, pos) if g[1] == "exactly one" else ([], [])
    return facts + [("constraint", (ft + "_uc", "uniqueness", ft, m)),
                    ("spans", (ft + "_uc", pos))] + also, \
        [(ft + "_uc", C.uniqueness([pos]))] + aobjs            # the quantified role's position

def _h_mandatory(g, k, m):
    ft, facts = _fact_type(g[0] + " " + g[1], k)
    mfacts, mobjs = _mandatory_parts(ft, _subject(g[0], k)[0], m)
    return facts + mfacts, mobjs

def _h_neg_uniqueness(g, k, m):
    ft, facts = _fact_type(" ".join(g), k)                     # reconstruct the reading; same constraint
    return facts + [("constraint", (ft + "_uc", "uniqueness", ft, m))], [(ft + "_uc", C.uniqueness([1]))]

def _h_neg_mandatory(g, k, m):
    ft, facts = _fact_type(" ".join(g), k)
    mfacts, mobjs = _mandatory_parts(ft, _subject(g[0], k)[0], m)
    return facts + mfacts, mobjs

def _h_spanning(g, k, m):
    ftn = g[0].replace(" ", "_")
    cid = ftn + "_uc"
    return [("constraint", (cid, "spanning_uniqueness", ftn, m)),
            ("spans", (cid, 1)), ("spans", (cid, 2))], [(cid, C.uniqueness([1, 2]))]


def _h_frequency(g, k, m):
    template, rtypes = _reading(g[0], k)                       # resolve the population's reading
    ftn = _ftid_from(template, rtypes)
    names = [s.strip() for s in g[1].split(",")]
    roles = [rtypes.index(nm) + 1 for nm in names if nm in rtypes] or [1]
    n = int(g[3])
    lo, hi = {"at most": (None, n), "at least": (n, None), "exactly": (n, n)}[g[2]]
    cid = ftn + "_freq"
    return [("constraint", (cid, "frequency", ftn, m))] + [("spans", (cid, p)) for p in roles], \
        [(cid, C.frequency(roles, lo, hi))]


_RING_BUILDERS = {"irreflexive": C.ring_irreflexive, "symmetric": C.ring_symmetric,
                  "asymmetric": C.ring_asymmetric, "antisymmetric": C.ring_antisymmetric,
                  "intransitive": C.ring_intransitive, "acyclic": C.ring_acyclic}


def _h_ring(g, k, m):
    ft, facts = _fact_type(g[0], k)
    cid = ft + "_ring_" + g[1]
    return facts + [("constraint", (cid, "ring_" + g[1], ft, m))], [(cid, _RING_BUILDERS[g[1]]())]


def _h_subtype(g, k, m):
    sub, sup = g[0].strip(), g[1].strip()
    cid = _slug(sub) + "_sub_" + _slug(sup)
    return [("instanceOf", (sub, "ObjectType")), ("instanceOf", (sup, "ObjectType")),
            ("subtype", (sub, sup)),
            ("constraint", (cid, "subtype", sub, sup, m))], [(cid, C.scoped_subset(sup))]

_QUANT = re.compile(r"\b(some|that|each|no|an|a) ")


def _clause_ft(text, known):
    """A constraint clause (quantified reading text) → the fact-type id it references:
    strip the quantifier words, then resolve as a fact-type reading. The string boundary
    of set-comparison/subset clause resolution (full RolePath unification is Stage 2)."""
    ft, _facts = _fact_type(_QUANT.sub("", text.strip()).strip(), known)
    return ft


def _h_set_comparison(g, k, m):
    subj, mode, body = g
    clauses = tuple(_clause_ft(c, k) for c in body.split(";") if c.strip())
    kind = {"exactly": "exclusive_or", "at most": "exclusion"}[mode]
    cid = _slug(subj) + {"exactly": "_xor", "at most": "_excl"}[mode]
    scoped = {"exactly": lambda ft: C.scoped_exclusive_or(subj, clauses, ft),
              "at most": lambda ft: C.scoped_exclusion(clauses, ft)}[mode]
    objs = [(cid, {"exactly": C.exclusive_or, "at most": C.exclusion}[mode]())] + \
           [(cid + "@" + ft, scoped(ft)) for ft in clauses]    # one attachment per clause cell
    return [("constraint", (cid, kind, subj, clauses, m))], objs

def _h_disjunctive(g, k, m):
    body = g[-1]
    subj, rest = _subject(body, k) if len(g) == 1 else (_subject(g[0], k)[0], body)
    clauses = tuple(_clause_ft(subj + " " + c, k) for c in rest.split(" or ") if c.strip())
    cid = "ior_" + _slug(subj)[:40]
    objs = [(cid, C.inclusive_or())] + \
           [(cid + "@" + ft, C.scoped_inclusive_or(subj, clauses, ft)) for ft in clauses]
    return [("constraint", (cid, "disjunctive_mandatory", subj, clauses, m))], objs

def _h_subset(g, k, m):
    ante, cons_txt = g
    conseq, _, _where = cons_txt.partition(" where ")         # a 'where' join condition, if present
    ft_a, ft_b = _clause_ft(ante, k), _clause_ft(conseq, k)
    cid = "subset_" + _slug(ante)[:40]
    return [("constraint", (cid, "subset", ft_a, ft_b, m))], \
        [(cid, C.scoped_subset(ft_b))]                         # attached to the antecedent cell

def _h_equality(g, k, m):
    ft_a, ft_b = _clause_ft(g[0], k), _clause_ft(g[1], k)
    cid = "eq_" + _slug(g[0])[:40]
    return [("constraint", (cid, "equality", ft_a, ft_b, m))], \
        [(cid + "_a", C.scoped_equality_side(ft_b)), (cid + "_b", C.scoped_equality_side(ft_a))]

def _h_negation(g, k, m):
    a, pred = _subject(g[0], k)
    return [("negation", (a, pred + " " + g[1]))], []

def _h_possibility(g, k, m):
    return [("possibility", (g[0][:80], m))], []

def _h_inverse_uc(g, k, m):
    a, _r = _subject(g[0], k)
    return [("constraint", (_slug(a) + "_inv_uc", "uniqueness", a, m))], []

_QUOTED = re.compile(r"'([^']*)'")


def _h_fact(g, k, m):
    kind, reading = _strip_derivation(g[0])                    # NORMA */**/+/++ derivation-storage marker
    if "'" in reading:
        # an INSTANCE fact (the corpus's dominant form): quoted ids fill the declared
        # roles; the row lands in the fact type's own cell, the population runtime reads
        ids = tuple(_QUOTED.findall(reading))
        dequoted = re.sub(r"\s+", " ", _QUOTED.sub("", reading)).strip()
        ft, _decl = _fact_type(dequoted, k)
        return [(ft, ids)], []
    ft, facts = _fact_type(reading, k)                         # mixfix template + ordered roles
    deriv = [("derivation", (ft, kind))] if kind else []      # link the fact type to its derivation/storage
    return facts + deriv, []


# ---- the state-machine readings (whitepaper §1): a machine is a SET OF FACTS in M ----
def _h_sm_def(g, k, m):
    return [("smDef", (g[0], g[1]))], []

def _h_sm_initial(g, k, m):
    return [("smStatus", (g[1], g[0], "initial"))], []        # ⟨sm, status, initial⟩

def _h_sm_from(g, k, m):
    return [("smFrom", (g[0], g[1]))], []                     # ⟨transition, from-status⟩

def _h_sm_to(g, k, m):
    return [("smTo", (g[0], g[1]))], []                       # ⟨transition, to-status⟩

def _h_sm_trigger(g, k, m):
    return [("smTrigger", (g[0], _clause_ft(g[1], k)))], []   # ⟨transition, trigger fact type⟩

_VARTOK = re.compile(r"^(\w+)(\d+)$")
_SOME = re.compile(r"\b(some|that) ")                          # existentials only: the article
                                                               # 'a' is predicate text ('is a
                                                               # parent of'), never stripped

def _rule_atom(text, known):
    """A rule clause → (fact type id, ordered variable list): numbered variables generalize
    to their base type for fact-type resolution ('Person1' plays a Person role), and the
    variable sequence follows the reading's role order (the book's D1 convention)."""
    vars_, out = [], []
    for tok in text.split():
        mm = _VARTOK.match(tok)
        if mm and mm.group(1) in known:
            vars_.append(mm.group(1) + mm.group(2))
            out.append(mm.group(1))
        else:
            out.append(tok)
    ft, _decl = _fact_type(_SOME.sub("", " ".join(out)).strip(), known)
    return ft, vars_


def _h_rule_if(g, k, m):
    """The book's rule form: Head if Clause [and Clause…]. The clauses join linearly on
    shared variables; the head projects its variables from the joined tuple; the compiled
    object consumes D (cross-cell) and run_rules derives to the lfp."""
    import zlib
    from . import system as _sys
    head_txt, body = g
    clauses = [c.strip() for c in body.split(" and ")]
    hft, hvars = _rule_atom(head_txt, k)
    atoms = [_rule_atom(c, k) for c in clauses]
    rule_cid = hft + "_rule_" + format(zlib.crc32(body.encode()), "x")
    _hf, decl = _fact_type(re.sub(r"\d+", "", head_txt).strip(), k)
    A_ = decl + [("derivation", (hft, "fully-derived")),
                 ("derivationRule", (hft, atoms[0][0], len(atoms))),
                 ("ruleDerives", (rule_cid, hft))]
    for aft, _av in atoms:
        A_.append(("ruleReads", (rule_cid, aft)))
    # column map: first atom contributes its full var list; each later atom appends the
    # variables after its join column (NatJoin keeps the running tuple and drops S.1)
    cols, obj = {}, None
    ok = True
    for i, (aft, avars) in enumerate(atoms):
        if i == 0:
            for v in avars:
                cols.setdefault(v, len(cols) + 1)
        else:
            # linear chain: the joined variable must be the running tuple's LAST column
            if not avars or cols.get(avars[0]) != len(cols):
                ok = False                                    # unsupported shape: M-facts only
                break
            for v in avars[1:]:
                cols.setdefault(v, len(cols) + 1)
    if ok and all(v in cols for v in hvars):
        obj = _sys.compile_rule([a[0] for a in atoms], [cols[v] for v in hvars])
    return A_, ([(rule_cid, obj)] if obj is not None else [])


def _h_derivation_rule(g, k, m):
    from . import system as _sys
    derived, root, body = g
    hops = _role_path(body)                                    # the role path from the root
    rule_cid = _slug(derived) + "_rule"
    A = [("instanceOf", (derived, "ObjectType")), ("derivation", (_slug(derived), "fully-derived")),
         ("derivationRule", (_slug(derived), root, len(hops))),
         ("ruleDerives", (rule_cid, _slug(derived)))]          # frontier: what the rule feeds
    prev = root
    for verb, target in hops:                                  # frontier: what the rule reads
        reading = f"{prev} {verb} {target}" if target else f"{prev} {verb}"
        A.append(("ruleReads", (rule_cid, _clause_ft(reading, k))))
        prev = target or prev
    # a two-hop linear path (root -V1-> T, T -V2-> ...) is a join on the shared type projecting the
    # root: rule:⟨hop1, hop2⟩ = NatJoin(2) then Project([1]) (infosci ORM->Datalog).
    cons = [(rule_cid, _sys.join_rule2(2, [1]))] if len(hops) == 2 else []
    return A, cons


_PLAN = {
    "entity_type": _h_entity, "value_type": _h_value, "ref_scheme": _h_ref_scheme,
    "objectification": _h_objectification, "data_type": _h_meta("data_type"), "ref_mode": _h_meta("ref_mode"),
    "value_constraint": _h_value_constraint, "uniqueness": _h_uniqueness, "mandatory": _h_mandatory,
    "neg_uniqueness": _h_neg_uniqueness, "neg_mandatory": _h_neg_mandatory, "spanning_uc": _h_spanning,
    "frequency": _h_frequency, "ring": _h_ring, "subtype_of": _h_subtype,
    "set_comparison": _h_set_comparison, "disjunctive_mandatory": _h_disjunctive,
    "subset": _h_subset, "equality": _h_equality, "derivation_rule": _h_derivation_rule,
    "rule_if": _h_rule_if,
    "negation": _h_negation, "possibility": _h_possibility, "inverse_uc": _h_inverse_uc,
    "sm_def": _h_sm_def, "sm_initial": _h_sm_initial, "sm_from": _h_sm_from,
    "sm_to": _h_sm_to, "sm_trigger": _h_sm_trigger,
    "fact_type_reading": _h_fact,
}


def _plan(kind, g, known, modality="alethic"):
    """Dispatch the reading kind to its handler (application by key), never an if/elif chain."""
    return _PLAN.get(kind, lambda g, k, m: ([], []))(g, known, modality)


def compile(stmt, D, known=()):
    from .reduce import apply as _apply
    from .lam import atom as _A
    kind, g, modality = analyze(stmt)
    asserts, cons = _plan(kind, g, known, modality)
    for cell, fact in asserts:
        D = _apply(_A(2), ast.run(to_lam(fact), D, cell_name=cell))
    for name, obj in cons:
        # a compiled definition is stored INTO the schema's own D, not the process seed
        # (Def. AREST / Cor. closure): ingestion mutates only the store being ingested into
        D = _apply(ast.DefineIn(name, obj), D)
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


def _cells(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return list(c[2])
    return []


# How each constraint KIND attaches to a cell's validate: fact (cid, kind, …scope…, modality) +
# the target cell → the (name, local?) attachments. A local attachment consumes the target
# population P; a scoped one consumes ⟨P, D⟩ and fetches sibling cells (audit C3 — every parsed
# family enforces; nothing drops silently).
_ATTACH = {
    "uniqueness":            lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "spanning_uniqueness":   lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "frequency":             lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_irreflexive":      lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_symmetric":        lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_asymmetric":       lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_antisymmetric":    lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_intransitive":     lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "ring_acyclic":          lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "subtype":               lambda f, ft: [(f[0], False)] if f[2] == ft else [],
    "external_uniqueness":   lambda f, ft: [(f[0], False)] if f[2] == ft else [],
    "value":                 lambda f, ft: [(f[0], True)] if f[2] == ft else [],
    "mandatory":             lambda f, ft: ([(f[0], False)] if f[2] == ft else [])
                                         + ([(f[0] + "_e", False)] if f[3] == ft else []),
    "subset":                lambda f, ft: [(f[0], False)] if f[2] == ft else [],
    "equality":              lambda f, ft: ([(f[0] + "_a", False)] if f[2] == ft else [])
                                         + ([(f[0] + "_b", False)] if f[3] == ft else []),
    "exclusion":             lambda f, ft: [(f[0] + "@" + ft, False)] if ft in f[3] else [],
    "exclusive_or":          lambda f, ft: [(f[0] + "@" + ft, False)] if ft in f[3] else [],
    "disjunctive_mandatory": lambda f, ft: [(f[0] + "@" + ft, False)] if ft in f[3] else [],
}


def validate_for(fact_type, D):
    """Build `fact_type`'s validate from M's constraint facts, respecting modality: alethic
    constraints block commit, deontic ones only flag (AREST Def. Violation). Attachment is
    read off M by kind (_ATTACH); the constraint names reflect to their objects via rho within
    the step's D (Cor. closure). Every parsed family enforces — local ones over the target
    population, scoped ones over ⟨P, D⟩."""
    from .lam import atom as _A
    local, scoped = [], []
    for f in _cells(D, "constraint"):
        if len(f) < 3:
            continue
        for name, is_local in _ATTACH.get(f[1], lambda f, ft: [])(f, fact_type):
            (local if is_local else scoped).append((_A(name), f[-1]))
    return system.validate_modal(local, scoped)


def parse(reading):
    kind, g = classify(reading.strip() if reading.strip().endswith(".") else reading.strip() + ".")
    if kind == "UNPARSED":
        raise ValueError(f"reading outside the fragment R: {reading!r}")
    return kind, g


# ---- verbalize / nf (Prop. spec): each kind renders its own canonical sentence, and the
# modal prefix is re-emitted from the parsed modality and sign. Cross-form normalization
# (negative twin -> positive primary) is the kernel quotient ~ and lives in compile, not
# here, so parse(nf(r)) keeps r's kind. ----
_RENDER = {
    "entity_type": lambda g: f"{g[0]} is an entity type",
    "value_type": lambda g: f"{g[0]} is a value type",
    "ref_scheme": lambda g: f"Reference Scheme: {g[0]} has {g[1]}",
    "ref_mode": lambda g: f"Reference Mode: {g[0]}",
    "data_type": lambda g: f"Data Type: {g[0]}",
    "value_constraint": lambda g: f"The possible values of {g[0]} are {g[1]}",
    "spanning_uc": lambda g: f"In each population of {g[0]}, each {g[1]} combination occurs at most once",
    "frequency": lambda g: f"In each population of {g[0]}, each {g[1]} combination occurs {g[2]} {g[3]} times",
    "ring": lambda g: f"{g[0]} is {g[1]}",
    "subtype_of": lambda g: f"{g[0]} is a subtype of {g[1]}",
    "objectification": lambda g: f"This association with {g[0]} provides the preferred identification scheme for {g[1]}",
    "set_comparison": lambda g: f"For each {g[0]}, {g[1]} one of the following holds: {g[2]}",
    "disjunctive_mandatory": lambda g: (f"For each {g[0]}, {g[1]}" if len(g) == 2 else f"Each {g[0]}"),
    "subset": lambda g: f"If {g[0]} then {g[1]}",
    "equality": lambda g: f"{g[0]} if and only if {g[1]}",
    "derivation_rule": lambda g: f"*Each {g[0]} is some {g[1]} who {g[2]}",
    "rule_if": lambda g: f"{g[0]} if {g[1]}",
    "negation": lambda g: f"{g[0]} ~{g[1]}",
    "uniqueness": lambda g: f"Each {g[0]} {g[1]} {g[2]}",
    "mandatory": lambda g: f"Each {g[0]} some {g[1]}",
    "neg_uniqueness": lambda g: ("any {0} more than one {1}".format(*g) if len(g) == 2 else
                                 "For each {0}, it is impossible that that {0} {1} more than one {2}".format(*g)),
    "neg_mandatory": lambda g: ("any {0} no {1}".format(*g) if len(g) == 2 else
                                "For each {0}, it is impossible that that {0} {1} no {2}".format(*g)),
    "inverse_uc": lambda g: f"For each {g[0]}, at most one {g[1]} that applies",
    "fact_type_reading": lambda g: g[0],
    "sm_def": lambda g: f"State Machine Definition '{g[0]}' is for Noun '{g[1]}'",
    "sm_initial": lambda g: f"Status '{g[0]}' is initial in State Machine Definition '{g[1]}'",
    "sm_from": lambda g: f"Transition '{g[0]}' is from Status '{g[1]}'",
    "sm_to": lambda g: f"Transition '{g[0]}' is to Status '{g[1]}'",
    "sm_trigger": lambda g: f"Transition '{g[0]}' is triggered by Fact Type '{g[1]}'",
}

_PREFIX = {("alethic", "positive"): "", ("deontic", "positive"): "It is obligatory that ",
           ("deontic", "negative"): "It is forbidden that ",
           ("alethic", "negative"): "It is impossible that "}


def nf(reading):
    """nf = verbalize ∘ compile ∘ parse (Prop. spec, conformance gate 1): the canonical
    sentence of the reading's construct. Idempotent by construction: the renderer emits a
    sentence its own kind's recognizer accepts with the same groups."""
    stmt = reading.strip()
    stmt = stmt if stmt.endswith(".") else stmt + "."
    mod, sign, _inner = _split_modality(stmt)
    kind, g = classify(stmt)
    if kind == "UNPARSED":
        raise ValueError(f"reading outside the fragment R: {reading!r}")
    if kind == "possibility":
        prefix = "It is permitted that " if mod == "deontic" else "It is possible that "
        return prefix + g[0] + "."
    return _PREFIX[(mod, sign)] + _RENDER[kind](g) + "."
