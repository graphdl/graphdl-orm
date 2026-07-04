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

NO NON-CANONICAL FORML: the grammar accepts NORMA's canonical verbalizations (and the
whitepaper/corpus surfaces for constructs NORMA lacks) — never engine-invented dialects.
A literal bound on a value role is a VALUE CONSTRAINT ('The possible values of Rating
are at most 5.'), not a bespoke trailing form.
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
    out, buf, in_comment = [], [], False
    for line in text.splitlines():
        s = line.strip()
        # markdown structure is not sentence content: comment blocks vanish, and a
        # heading BREAKS the accumulation (it never continues a sentence)
        if in_comment:
            if "-->" in s:
                in_comment = False
            continue
        if s.startswith("<!--"):
            in_comment = "-->" not in s
            continue
        if s.startswith("#"):
            buf = []
            continue
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
    # the process completion of the §1 shape: guards and Mealy/Moore output functions,
    # all M-facts; a guard is a (possibly derived) fact type, hence positive, so the
    # groundedness condition on state transitions holds by construction
    ("sm_guard", re.compile(r"^Transition '(.+)' is guarded by Fact Type '(.+)'\.$")),
    ("sm_emit", re.compile(r"^Transition '(.+)' emits '(.+)'\.$")),
    ("sm_moore", re.compile(r"^Status '(.+)' emits '(.+)'\.$")),
    ("value_constraint", re.compile(r"^[Tt]he possible values? of (.+?) (?:are|is) (.+)\.$")),
    ("spanning_uc", re.compile(r"^[Ii]n each population of (.+), each (.+) combination occurs at most once\.$")),
    # the corpus's roles-first spelling of the same constraint (base state.md,
    # bill-negotiation, support.auto.dev)
    ("spanning_uc2", re.compile(r"^[Ee]ach (.+?) combination occurs at most once "
                                r"in the population of (.+)\.$")),
    # the corpus's for-each mandatory: 'For each Reading, some Role is used in
    # that Reading.' — declares the fact type through the anaphoric scan and
    # mandates the for-each subject at its role position
    ("for_each_mandatory", re.compile(r"^For each (.+?), some (.+)\.$")),
    # Halpin §7.2: frequency generalizes the spanning form from 'once' to bounded counts
    ("frequency", re.compile(r"^[Ii]n each population of (.+), each (.+) combination occurs (at most|at least|exactly) (\d+) times?\.$")),
    # Halpin §7.3 ring constraints, as the corpus grammar's trailing markers on a reading
    ("ring", re.compile(r"^(.+?) is (acyclic|asymmetric|antisymmetric|intransitive|irreflexive|symmetric)\.$")),
    # subtyping (corpus trailing marker 'is a subtype of'; RMAP step 0 absorbs to the top)
    ("subtype_of", re.compile(r"^(.+) is a subtype of (.+)\.$")),
    # the corpus's brace subtype family (9 occurrences): each link plus pairwise exclusion
    ("brace_subtypes", re.compile(r"^\{(.+)\} are (mutually exclusive )?subtypes of (.+)\.$")),
    ("objectification", re.compile(r"^[Tt]his association with (.+) provides the preferred identification scheme for (.+)\.$")),
    ("set_comparison", re.compile(r"^[Ff]or each (.+?), (exactly|at most) one of the following holds: (.+)\.$")),
    # negative forms (constraint verbalization paper): map to the SAME constraint as the positive twin
    ("neg_uniqueness", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) more than one (.+)\.$")),
    ("neg_mandatory", re.compile(r"^[Ff]or each (.+?), it is impossible that that .+? (.+) no (.+)\.$")),
    ("disjunctive_mandatory", re.compile(r"^[Ff]or each (.+?), (.+ or .+)\.$")),
    ("inverse_uc", re.compile(r"^[Ff]or each (.+?), (at most one|exactly one) (.+) (?:that|those) .+\.$")),
    ("subset", re.compile(r"^[Ii]f (.+) then (.+)\.$")),                      # 'if A then B' = subset (modus ponens)
    # grammar-as-readings recognizers (forml2-grammar.md: 'the parser is this file'):
    # a quoted-head iff rule classifies Statements from their field facts
    ("class_rule", re.compile(r"^(\S[^']*?) has (\S[^']*?) '(.+?)' iff (.+)\.$")),
    ("equality", re.compile(r"^(.+) if and only if (.+)\.$")),                # 'A iff B' = equality
    # the book's rule surface (Halpin ch.2 ex.4 D1): numbered variables, ' if ' head-body,
    # ' and ' conjunction; a digit in the head keeps plain readings out of this
    # recognizer. The corpus's biconditional spelling 'iff' (the closed-world reading
    # of n rules per head, per the ORM-to-datalog mapping) is a synonym here, and its
    # 'where'-scoped bodies fold into the same conjunction in the handler.
    # quote-aware: the keyword and the digit must sit OUTSIDE literals (an instance
    # fact whose quoted value cites ' iff ' or a digit is not a rule — the old
    # engine's literal-aware keyword scan)
    ("rule_if", re.compile(r"^(?![*+])((?:[^']|'[^']*')*?\d\S*(?:[^']|'[^']*')*?) iff? (.+)\.$")),
    # the live corpus's unnumbered anaphoric spelling, canonical per the old grammar's
    # own classifier ('Statement has Classification Derivation Rule iff Statement has
    # Keyword iff' — arest readings/forml2-grammar.md): variables are type-name
    # occurrences, that/some qualifiers bind anaphorically, and an optional leading
    # NORMA derivation-storage marker (* ** + ++) names the storage kind
    ("rule_iff", re.compile(r"^(?:([*+]{1,2}) )?((?:[^']|'[^']*')*?) iff (.+)\.$")),
    # a derivation RULE reading (leading * = derived): a linear role path from a root object type
    # (infosci Mapping_ORM_to_Datalog: *Each FastCarDriver is some Person who drives some Car ...)
    ("derivation_rule", re.compile(r"^\*Each (.+?) is some (.+?) who (.+)\.$")),
    ("neg_uniqueness", re.compile(r"^any (.+?) more than one (.+)\.$")),      # neg of 'each A .. at most one B'
    ("neg_mandatory", re.compile(r"^any (.+?) no (.+)\.$")),                   # neg of 'each A .. some B'
    ("disjunctive_mandatory", re.compile(r"^[Ee]ach (.+ or .+)\.$")),         # inclusive-or / disjunctive mandatory
    ("uniqueness", re.compile(r"^[Ee]ach (.+?) (at most one|exactly one) (.+)\.$")),
    ("mandatory", re.compile(r"^[Ee]ach (.+?) some (.+)\.$")),
    # finality depth: where optimistic acceptance hardens deontic→alethic (writer model)
    ("finality", re.compile(r"^(\S+) becomes final at depth (\d+)\.$")),
    # NORMA's unary negation pattern: the reading creates the PAIRED negation fact type
    ("neg_pair", re.compile(r"^(\S+) (does not|is not) (\S.*)\.$")),
    ("negation", re.compile(r"^(.+) ~(.+)\.$")),
    ("fact_type_reading", re.compile(r"^(.+)\.$")),
]


def analyze(stmt):
    """stmt → (kind, groups, modality). A possibility/permitted statement is the absence of a
    constraint (informational). Otherwise the inner is classified and tagged with its modality.
    A TRAILING parenthetical is an annotation, not sentence content: the old corpus writes
    'Verb is performed during Transition (Mealy semantics).' and its cell is
    Verb_is_performed_during_Transition — the aside strips before classification."""
    stmt = re.sub(r"\s*\([^()]*\)\.$", ".", stmt)
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


_QUOTED_SPAN = re.compile(r"'[^']*'")


def _prose_suspect(text, known):
    """A readings PARAGRAPH pretending to be a reading. The tell is STRUCTURAL: a
    comma or parenthesis outside quoted spans — no legitimate fact-type reading
    carries either (the base's 916 statements included), while prose runs on
    commas and asides. A merely-unknown Title-case word is NOT the tell: the old
    corpus declares role nouns implicitly in readings ('Noun has Object Type.'
    with Object Type declared nowhere is the base's own style, and its cells ride
    every live db), so the old #789 word-level test applies to rule clauses and
    instance facts, not to plain readings."""
    bare = _QUOTED_SPAN.sub(" ", text)
    # the colon tell is SENTENCE punctuation (': ' with a following space); a
    # colon inside a token is a CURIE (schema:Product — the federation lineage)
    return ("," in bare) or ("(" in bare) or (")" in bare) or (": " in bare)


class _Known(set):
    """The known TYPE NAMES, carrying the prepass context rules need: the subtype
    closure (noun → its ancestors), the declared fact-type slugs (rule heads
    included, for antecedent resolution), and the PLAIN reading declarations
    (rule heads excluded — a rule against a plainly-declared fact type must not
    re-mark its storage kind; the reading's own trailing marker owns that)."""
    def __new__(cls, names, subs=None, fts=None, plain=None):
        self = super().__new__(cls, names)
        self.subs = subs or {}
        self.fts = fts or set()
        self.plain = plain or set()
        return self

    def __init__(self, names, subs=None, fts=None, plain=None):
        super().__init__(names)


def _context_of(D):
    """The known context READ OFF a compiled store — declared type names, subtype
    edges, fact-type slugs — so a model can compile ATOP a preloaded base (the old
    engine folds CORE_READINGS ahead of every app; this is the same seam with the
    base thawed from frozen ingestion instead of recompiled)."""
    names = {r[0] for r in system._pop_rows(D, "instanceOf")
             if len(r) >= 2 and r[1] in ("ObjectType", "ValueType")}
    fts = {f[0] for f in system._pop_rows(D, "factType") if f}
    edges = [(r[0], r[1]) for r in system._pop_rows(D, "subtype") if len(r) >= 2]
    return names, edges, fts


def _prepass_context(stmts, names, extra_edges=(), extra_fts=()):
    """Collect subtype edges (closed transitively), declared fact-type slugs, and
    the PLAIN reading declarations (fts minus rule heads)."""
    edges = list(extra_edges)
    fts = set(extra_fts)
    plain = set(extra_fts)
    for s in stmts:
        kind, g = classify(s)
        if kind == "subtype_of":
            edges.append((g[0].strip(), g[1].strip()))
        elif kind == "brace_subtypes":
            for sub in g[0].split(","):
                edges.append((sub.strip(), g[2].strip()))
        elif kind == "fact_type_reading" and "'" not in g[0]:
            if _prose_suspect(g[0], names):
                continue                                       # a paragraph, not a reading
            ft, _ = _fact_type(_strip_derivation(g[0])[1], names)
            fts.add(ft)
            plain.add(ft)
        elif kind in ("rule_if", "rule_iff"):
            # a rule HEAD is a declaration (NORMA's starred reading): later rules'
            # antecedents resolve against it exactly like an explicit reading
            head = g[0] if kind == "rule_if" else g[1]
            ft, _ = _fact_type(re.sub(r"\d+", "", head).strip(), names)
            fts.add(ft)
        elif kind == "uniqueness":
            ft, _ = _fact_type(g[0] + " " + g[2], names)
            fts.add(ft)
            plain.add(ft)
        elif kind == "mandatory":
            ft, _ = _fact_type(g[0] + " " + g[1], names)
            fts.add(ft)
            plain.add(ft)
    parents = {}
    for (a, b) in edges:
        parents.setdefault(a, set()).add(b)
    closure = {}
    for start in parents:
        seen, todo = set(), [start]
        while todo:
            cur = todo.pop()
            for p in parents.get(cur, ()):
                if p not in seen:
                    seen.add(p)
                    todo.append(p)
        closure[start] = seen
    return closure, fts, plain


# ---- two-pass name resolution: split a reading against the known type names ----
# sentence vocabulary that never OPENS a type name: the grammar's Prose Stopword
# enum plus the connective/negation sentence-leaders of the live corpus
_IMPLICIT_STOP = {"If", "When", "Then", "That", "This", "An", "A", "The",
                  "Each", "Some", "No", "Every", "Not", "It", "There", "Once",
                  "For", "In", "Of", "To", "On", "At", "By", "With", "And",
                  "Or", "Only"}


def _implicit_nouns(stmts):
    """The old corpus's implicit role nouns: a maximal run of Title-case tokens
    inside a non-prose statement IS a noun, declared by occurrence (the old
    engine's Role Reference extraction — its dbs bind Event Type, Fact Type and
    Noun this way with no explicit declaration anywhere). Quoted spans are data;
    prose and list forms (comma or parenthesis outside quotes) are never mined;
    numeric subscripts strip per token."""
    names = set()
    for s in stmts:
        s = re.sub(r"\s*\([^()]*\)\.$", ".", s)               # trailing annotation
        bare = _QUOTED_SPAN.sub(" ", s)
        if ("," in bare) or ("(" in bare) or (")" in bare):
            continue
        run = []
        for tok in bare.split():
            base = tok.strip(".;:").rstrip("0123456789")
            if base and base[0].isupper() and base not in _IMPLICIT_STOP:
                run.append(base)
                continue
            if len(run) >= 1:
                names.add(" ".join(run))
            run = []
        if run:
            names.add(" ".join(run))
    return names


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
    names |= _implicit_nouns(stmts)
    return sorted(names, key=len, reverse=True)


def _subject(text, known):
    """The leading object type of a reading + the remainder (a find over known types — the string
    boundary): used by negation/inverse-uc where only the subject is needed. LONGEST
    name first: 'State Machine Definition has …' must never truncate its subject to a
    declared prefix type ('State Machine') — set order made it nondeterministic."""
    for k in sorted(known, key=lambda s: -len(s)):
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


def _h_spanning_corpus(g, k, m):
    """'Each A, B combination occurs at most once in the population of <reading>.'
    — the roles-first spelling; the reading declares implicitly, old-corpus style."""
    names = [s.strip() for s in g[0].split(",")]
    ftn, decl = _fact_type(g[1], k)
    _t, rtypes = _reading(g[1], k)
    roles, used = [], {}
    for nm in names:
        occ = [i for i, t in enumerate(rtypes) if t == nm]
        if occ:
            roles.append(occ[min(used.get(nm, 0), len(occ) - 1)] + 1)
            used[nm] = used.get(nm, 0) + 1
    roles = roles or [1, 2]
    cid = ftn + "_uc"
    return (decl + [("constraint", (cid, "spanning_uniqueness", ftn, m))]
            + [("spans", (cid, p)) for p in roles]), [(cid, C.uniqueness(roles))]


def _dequalify(text, known):
    """The clause with its anaphoric qualifiers dropped — the declared reading
    behind 'Role is used in that Reading' (the same scan _rule_atom runs)."""
    kset = sorted(known, key=lambda x: -len(x.split()))
    toks, out, i = text.split(), [], 0
    while i < len(toks):
        if toks[i] in _QUALIFIERS and _type_span(toks, i + 1, kset):
            i += 1
            continue
        out.append(toks[i])
        i += 1
    return " ".join(out)


def _h_for_each_mandatory(g, k, m):
    """'For each S, some <clause over S>.' — the clause declares the fact type
    (implicitly, old-corpus style) and S's role in it is mandatory."""
    subject, clause = g[0].strip(), _dequalify(g[1], k)
    ft, decl = _fact_type(clause, k)
    _t, rtypes = _reading(clause, k)
    pos = (rtypes.index(subject) + 1) if subject in rtypes else 1
    mfacts, mobjs = _mandatory_parts(ft, subject, m, pos)
    return decl + mfacts, mobjs


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
    """A subtype declaration MEANS upward inclusion — subtype instances ARE supertype
    instances — so it installs the derivation rule super(x) ← sub(x) through the
    ordinary rule machinery (semi-naive variants included; chains compose round by
    round). The subset constraint remains the check; the rule is the meaning."""
    from . import system as _sys
    sub, sup = g[0].strip(), g[1].strip()
    cid = _slug(sub) + "_sub_" + _slug(sup)
    rid = _slug(sub) + "_isa_" + _slug(sup)
    facts = [("instanceOf", (sub, "ObjectType")), ("instanceOf", (sup, "ObjectType")),
             ("subtype", (sub, sup)),
             ("constraint", (cid, "subtype", sub, sup, m)),
             ("ruleDerives", (rid, sup)), ("ruleReads", (rid, sub)),
             ("ruleAtom", (rid, 1, sub)), ("ruleCopies", (rid, sub, sup))]
    objs = [(cid, C.scoped_subset(sup)),
            (rid, _sys.compile_rule([sub], [1], [1])),
            (f"{rid}~d1", _sys.compile_rule_delta([sub], [1], 0, [1]))]
    return facts, objs


def _h_brace_subtypes(g, k, m):
    """The corpus's brace family: '{A, B} are mutually exclusive subtypes of X.' is each
    subtype link (through _h_subtype, so RMAP step 0 and the governedBy closure see them
    like any other) plus, when marked, the pairwise exclusion between the subtype
    populations."""
    subs = tuple(s.strip() for s in g[0].split(","))
    A_, objs = [], []
    for s in subs:
        a, o = _h_subtype((s, g[2]), k, m)
        A_ += a
        objs += o
    if g[1]:
        cid = "sxc_" + _slug("_".join(subs))[:40]
        A_.append(("constraint", (cid, "exclusion", subs[0], subs, m)))
        # a mutually exclusive family is Halpin's PARTITION mapping: the subtypes keep
        # their own RMAP tables (the layout splits; the SEMANTIC subtyping — inclusion
        # rules, clause lift — is unchanged)
        A_ += [("subtypePartition", (s, g[2].strip())) for s in subs]
        objs += [(cid, C.exclusion())] + \
                [(cid + "@" + s, C.scoped_exclusion(subs, s)) for s in subs]
    return A_, objs

_QUANT = re.compile(r"\b(some|that|each|no|an|a) ")
_QUANT_MIN = re.compile(r"\b(some|that|each|no) ")


def _clause_ft(text, known):
    """A constraint clause (quantified reading text) → the fact-type id it references.
    Resolution prefers a DECLARED fact type under the MINIMAL quantifier strip
    (some/that/each/no — an article is predicate text, the rule path's lesson: 'is a
    manager' declares Employee_is_a_manager, and stripping the article resolved the
    clause to a cell that does not exist, a silently unenforced constraint). The full
    strip stays as the fallback, itself preferring a declared hit, so article-free
    models keep their ids. The string boundary of set-comparison/subset clause
    resolution (full RolePath unification is Stage 2)."""
    t = re.sub(r"\s+", " ", text.strip())
    fts = getattr(known, "fts", None) or ()
    ft_min, _ = _fact_type(_QUANT_MIN.sub("", t).strip(), known)
    if ft_min in fts:
        return ft_min
    ft_full, _facts = _fact_type(_QUANT.sub("", t).strip(), known)
    return ft_full


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


def _conj(rest):
    """'does not smoke' pairs with 'smokes': naive third-person conjugation of the first
    word (the fragment's boundary; NORMA conjugates properly)."""
    head, _, tail = rest.partition(" ")
    head = head + ("es" if head.endswith(("s", "x", "z", "ch", "sh")) else "s")
    return head + ((" " + tail) if tail else "")


_CLAUSE_RE = re.compile(r"^(\S.*?) has (\S.*?)(?: '(.+?)')?$")


def _h_class_rule(g, k, m):
    """The grammar-as-readings recognizer form (forml2-grammar.md): 'Statement has
    Classification C iff Statement has Field ⟨lit⟩ [and …]' compiles into an ordinary
    rule deriving ⟨sid, C⟩ from the field cells — the parser IS the file, run by
    run_rules. Each literal a body clause tests is recorded as a classLit fact; that
    population is Stage-1's ENTIRE tokenizer vocabulary."""
    import zlib
    from . import system as _sys
    subjh, fieldh, headlit, body = g
    head_ft = _slug(f"{subjh} has {fieldh}")
    clauses = []
    # split on ' and ' only OUTSIDE quotes ('if and only if' is one literal)
    for c in re.split(r" and (?=(?:[^']*'[^']*')*[^']*$)", body):
        mm = _CLAUSE_RE.match(c.strip())
        if not mm:
            return [], []
        s2, f2, lit = mm.groups()
        clauses.append((_slug(f"{s2} has {f2}"), lit))
    rid = head_ft + "_cls_" + format(zlib.crc32((headlit + "|" + body).encode()), "x")
    A_ = [("ruleDerives", (rid, head_ft))]
    for (ftb, lit) in clauses:
        A_.append(("ruleReads", (rid, ftb)))
        if lit is not None:
            A_.append(("classLit", (ftb, lit)))
    return A_, [(rid, _sys.class_rule(clauses, headlit))]


def stage1_vocabulary(D):
    """Stage-1's token vocabulary, read off the ingested grammar: exactly the literals
    the recognizer rules test (classLit). The tokenizer knows nothing else."""
    from . import system as _sys
    return {(r[0], r[1]) for r in _sys._pop_rows(D, "classLit") if len(r) >= 2}


def tokenize_statement(D, stmt, nouns=(), sid="s1"):
    """Stage-1, the bootstrap kernel: extract field FACTS from one statement. The
    vocabulary is stage1_vocabulary (from D, never hardcoded); a Trailing Marker must
    trail; Role References are known-noun occurrences; a quoted token is a Literal
    Role. Returns [(field_ft, (sid, value)), …]."""
    text = stmt.strip().rstrip(".")
    out = []
    for (ftb, lit) in sorted(stage1_vocabulary(D), key=lambda p: -len(p[1])):
        # case-insensitive: Stage-1 recognises phrases at any position, 'The possible
        # values of …' included (the file's own Verb-override comment)
        if not re.search(r"(?<![A-Za-z])" + re.escape(lit) + r"(?![A-Za-z])", text, re.IGNORECASE):
            continue
        if ftb == "Statement_has_Trailing_Marker" and not text.lower().endswith(lit.lower()):
            continue
        out.append((ftb, (sid, lit)))
    for n in nouns:
        if re.search(r"(?<![A-Za-z])" + re.escape(n) + r"(?![A-Za-z])", text):
            out.append(("Statement_has_Role_Reference", (sid, n)))
    quoted = _QUOTED.findall(text)
    if quoted:
        out.append(("Statement_has_Literal_Role", (sid, quoted[0])))
    return out


def classify_via_M(D, stmt, nouns=(), sid="s1"):
    """Stage-2 through the substrate: assert the statement's field facts into D, run
    the recognizer RULES (run_rules — the parser is the file, the classifier is the
    rule runner), and read the Statement's classifications back."""
    from .reduce import apply as _apply
    from .lam import to_lam
    from . import system as _sys
    changed = set()
    for (ftb, row) in tokenize_statement(D, stmt, nouns, sid):
        D = _apply(_A2(), ast.run(to_lam(row), D, cell_name=ftb))
        changed.add(ftb)
    if not changed:
        return set()
    D = _sys.run_rules(D, changed=changed)
    return {r[1] for r in _sys._pop_rows(D, "Statement_has_Classification")
            if len(r) >= 2 and r[0] == sid}


def _A2():
    from .lam import atom as _A
    return _A(2)


def _h_neg_pair(g, k, m):
    """NORMA's unary negation pattern (UnaryValuePattern.Negation, FactType.cs): 'X is
    not R.' / 'X does not R.' creates the PAIRED positive-shaped negation fact type,
    linked by negOf, with the pair exclusion auto-asserted (nothing is both). Negative
    information is stored as ordinary monotone facts, so the substrate stays CALM; the
    closed world is the ordinary disjunctive-mandatory over the pair, and defaults are
    read-time (docs/2026-07-02-negation-model.md)."""
    subj, mode, rest = g
    if subj not in k:
        return _h_fact((f"{subj} {mode} {rest}",), k, m)      # unknown subject: plain reading
    pos_read = f"{subj} is {rest}" if mode == "is not" else f"{subj} {_conj(rest)}"
    pos, decl_p = _fact_type(pos_read, k)
    neg, decl_n = _fact_type(f"{subj} {mode} {rest}", k)
    cid = "negx_" + neg[:40]
    pair = (pos, neg)
    A_ = decl_p + decl_n + [("negOf", (neg, pos)),
                            ("constraint", (cid, "exclusion", neg, pair, "alethic"))]
    objs = [(cid, C.exclusion())] + \
           [(cid + "@" + ft, C.scoped_exclusion(pair, ft)) for ft in pair]
    return A_, objs

def _h_possibility(g, k, m):
    return [("possibility", (g[0][:80], m))], []

def _h_inverse_uc(g, k, m):
    """The inverse-role UC anchors to the FACT TYPE at the subject's computed position
    (a real role-2 uniqueness, so doubly-functional 1:1 fact types are detectable);
    'exactly one' adds the mandatory at the same position, Halpin's fewer-nulls signal."""
    a, _r = _subject(g[0], k)
    reading = f"{g[2]} {g[0]}"
    ft, facts = _fact_type(reading, k)
    _t, rtypes = _reading(reading, k)
    pos = rtypes.index(a) + 1 if a in rtypes else 2
    cid = _slug(a) + "_inv_uc"
    also, aobjs = _mandatory_parts(ft, a, m, pos) if g[1] == "exactly one" else ([], [])
    return facts + [("constraint", (cid, "uniqueness", ft, m)), ("spans", (cid, pos))] + also, aobjs

_QUOTED = re.compile(r"'([^']*)'")


def _h_fact(g, k, m):
    kind, reading = _strip_derivation(g[0])                    # NORMA */**/+/++ derivation-storage marker
    if "'" in reading:
        # an INSTANCE fact (the corpus's dominant form): quoted ids fill the declared
        # roles; the row lands in the fact type's own cell, the population runtime reads
        ids = tuple(_QUOTED.findall(reading))
        dequoted = re.sub(r"\s+", " ", _QUOTED.sub("", reading)).strip()
        ft, _decl = _fact_type(dequoted, k)
        # the subtype lift, as in _rule_atom: an instance fact authored via a subtype
        # resolves UP to the supertype-declared fact type when its own is undeclared
        # (subtype instances ARE supertype instances; the fact lives once)
        if isinstance(k, _Known) and k.fts and ft not in k.fts:
            _t, rtypes = _reading(dequoted, k)
            for anc in sorted(k.subs.get(rtypes[0], ()) if rtypes else ()):
                lifted, _ = _fact_type(dequoted.replace(rtypes[0], anc, 1), k)
                if lifted in k.fts:
                    return [(lifted, ids)], []
        return [(ft, ids)], []
    ft, facts = _fact_type(reading, k)                         # mixfix template + ordered roles
    deriv = [("derivation", (ft, kind))] if kind else []      # link the fact type to its derivation/storage
    return facts + deriv, []


# ---- the state-machine readings (whitepaper §1): a machine is a SET OF FACTS in M ----
# the machine definition IS a set of facts (whitepaper §1; the old cells carry
# Transition_is_from_Status et al. populated from these very statements), so each
# DSL statement asserts BOTH the machinery fact and the ordinary instance fact —
# rules like the base's rooted-status derivation read the plain cells
def _h_sm_def(g, k, m):
    return [("smDef", (g[0], g[1])),
            ("State_Machine_Definition_is_for_Noun", (g[0], g[1]))], []

def _h_sm_initial(g, k, m):
    return [("smStatus", (g[1], g[0], "initial")),            # ⟨sm, status, initial⟩
            ("Status_is_initial_in_State_Machine_Definition", (g[0], g[1]))], []

def _h_sm_from(g, k, m):
    return [("smFrom", (g[0], g[1])),                         # ⟨transition, from-status⟩
            ("Transition_is_from_Status", (g[0], g[1]))], []

def _h_sm_to(g, k, m):
    return [("smTo", (g[0], g[1])),                           # ⟨transition, to-status⟩
            ("Transition_is_to_Status", (g[0], g[1]))], []

def _h_sm_trigger(g, k, m):
    return [("smTrigger", (g[0], _clause_ft(g[1], k)))], []   # ⟨transition, trigger fact type⟩

def _h_sm_guard(g, k, m):
    return [("smGuard", (g[0], _clause_ft(g[1], k)))], []     # ⟨transition, guard fact type⟩

def _h_sm_emit(g, k, m):
    return [("smEmit", (g[0], g[1]))], []                     # ⟨transition, Mealy def name⟩

def _h_sm_moore(g, k, m):
    return [("smMoore", (g[0], g[1]))], []                    # ⟨status, Moore def name⟩

# the anaphoric qualifiers, the old engine's strip_role_qualifiers set. Stripping is a
# FALLBACK, tried only when the verbatim reading resolves to no declared fact type —
# 'a' is often predicate text ('Person is a Parent' keeps its article), while
# 'that Resource' in the corpus's anaphoric rules normalizes to the bare reading.
_QUALIFIERS = {"that", "some", "the", "other", "a", "an"}


def _type_span(toks, i, kset):
    """The longest known type reading left-to-right from toks[i], its LAST word
    optionally carrying a numeric subscript (Halpin's Task1 / State Machine2).
    → (base type, subscript, token span) or None."""
    for k in kset:
        kw = k.split()
        last = i + len(kw) - 1
        if last < len(toks) and toks[i:last] == kw[:-1]:
            mm = re.fullmatch(re.escape(kw[-1]) + r"(\d*)", toks[last])
            if mm:
                return k, mm.group(1), len(kw)
    return None


def _quoted_at(toks, i):
    """The quoted literal starting at toks[i] → (text without quotes, next index)."""
    buf = []
    for j in range(i, len(toks)):
        buf.append(toks[j])
        if toks[j].endswith("'") and (j > i or len(toks[j]) > 1):
            return " ".join(buf)[1:-1], j + 1
    return " ".join(buf).strip("'"), len(toks)


def _rule_atom(text, known):
    """A rule clause → (fact type id, ordered variables, literal restrictions).
    Variables are type-name occurrences — the corpus's unnumbered anaphoric
    spelling and the book's numbered D1 convention are one mechanism, a numeric
    subscript distinguishing same-type twins. A quoted literal directly after a
    role mention restricts that role. Fact-type resolution tries the verbatim
    reading first, then the qualifier-stripped one (the old engine's chain)."""
    kset = sorted(known, key=lambda k: -len(k.split()))
    toks, vars_, lits = text.split(), [], []
    verbatim, stripped = [], []
    i = 0
    while i < len(toks):
        tok = toks[i]
        if tok in _QUALIFIERS and _type_span(toks, i + 1, kset):
            verbatim.append(tok)                              # kept as reading text
            i += 1
            continue
        span = _type_span(toks, i, kset)
        if span:
            base, sub, ln = span
            vars_.append(base + sub)
            verbatim.append(base)
            stripped.append(base)
            i += ln
            if i < len(toks) and toks[i].startswith("'"):
                lit, i = _quoted_at(toks, i)
                lits.append((len(vars_) - 1, lit))
            continue
        verbatim.append(tok)
        stripped.append(tok)
        i += 1
    fts = known.fts if isinstance(known, _Known) else ()
    ft, _decl = _fact_type(" ".join(verbatim), known)
    if fts and ft not in fts:
        alt, _ = _fact_type(" ".join(stripped), known)
        if alt in fts:
            ft = alt
    # the subtype lift: a clause keyed on a subtype resolves UP to the supertype's
    # declared fact type when its own is undeclared (subtype instances ARE supertype
    # instances; the fact lives once, in the supertype-keyed cell)
    if vars_ and fts and ft not in fts:
        base = re.sub(r"\d+$", "", vars_[0])
        reading = " ".join(stripped)
        for anc in sorted(known.subs.get(base, ())):
            lifted, _ = _fact_type(reading.replace(base, anc, 1), known)
            if lifted in fts:
                return lifted, vars_, lits
    return ft, vars_, lits


def _coercion(clause, known):
    """The corpus's re-keying idiom: a bare 'A is B' over two known types with NO
    declared fact type is an identity binding between the two variables (subtype
    coercion: one instance plays both). A declared 'A is B' reading stays an
    ordinary atom — declaration wins, as in the old engine's reading resolution."""
    toks = clause.split()
    kset = sorted(known, key=lambda k: -len(k.split()))
    sa = _type_span(toks, 0, kset)
    if not sa or sa[2] >= len(toks) or toks[sa[2]] != "is":
        return None
    sb = _type_span(toks, sa[2] + 1, kset)
    if not sb or sa[2] + 1 + sb[2] != len(toks):
        return None
    if isinstance(known, _Known) and known.fts:
        ft, _ = _fact_type(f"{sa[0]} is {sb[0]}", known)
        if ft in known.fts:
            return None
    return sa[0] + sa[1], sb[0] + sb[1]


# the output and source are VARIABLES by the rule convention: numbered
# (Count1 of Count2) or the corpus's unnumbered type-name spelling (Arity of
# Role — the base's own Fact_Type_has_Arity rule)
_AGG_CLAUSE = re.compile(r"^(.+?) is the (min|max|count|sum) of (.+)$")
_CMP_CLAUSE = re.compile(
    r"^(\S*\d\S*) (exceeds|is greater than|is less than|is at least|is at most|equals) (\S+)$")
_CMP_OPS = {"exceeds": "gt", "is greater than": "gt", "is less than": "lt",
            "is at least": "ge", "is at most": "le", "equals": "eq"}


def _h_rule_if(g, k, m, kind="fully-derived"):
    """The book's rule form: Head if Clause [and Clause…]. Fact-type clauses join
    linearly on shared variables; COMPARATOR clauses (the corpus's word comparators, a
    bound variable against a literal or another bound variable) do not join — they
    RESTRICT the running tuple as filters; COERCION clauses ('Task is Resource', two
    known types, no declared fact type) alias their variables to one column; the head
    projects its variables; the compiled object consumes D (cross-cell) and run_rules
    derives to the lfp."""
    import zlib
    from . import system as _sys
    head_txt, body = g[0], g[1]
    # ' and ' splits at TOP level; a fragment's ' where '-chain then scopes to
    # the fragment's own quantifier: inside a 'no'-group it stays the negated
    # existential's conjunction (it must never escape as a top-level clause),
    # after an aggregate it hoists to top-level conjunction (the corpus's
    # bag-scoping spelling, the behavior existing models compiled against)
    clauses, neg_groups = [], []
    for frag in (c.strip() for c in body.split(" and ")):
        if frag.startswith("no "):
            neg_groups.append([p.strip() for p in frag[3:].split(" where ")])
        elif " where " in frag:
            clauses.extend(p.strip() for p in frag.split(" where "))
        else:
            clauses.append(frag)
    hft, hvars, _hlits = _rule_atom(head_txt, k)
    rule_cid = hft + "_rule_" + format(zlib.crc32(body.encode()), "x")
    _hf, decl = _fact_type(re.sub(r"\d+", "", head_txt).strip(), k)
    # the rule's leading marker marks the RULE; the fact type's storage kind
    # belongs to its READING declaration (trailing marker there, or none). Only
    # a head the rule itself declares defaults to the rule's kind — the old
    # base's SM current-status is plainly declared with imperative writers
    # beside its seed rule, and must not become fully-derived here.
    head_is_new = not (isinstance(k, _Known) and hft in k.plain)
    A_ = decl + ([("derivation", (hft, kind))] if head_is_new else []) \
        + [("ruleDerives", (rule_cid, hft))]
    # one pass, clauses in order: joins extend the column map, comparators filter
    # it. The AGGREGATE clause is extracted first and processed LAST: the corpus
    # places it at the head of the body with its bag scoped by the where-clauses
    # after it, so its source binds only once the joins have run.
    cols, atoms, filters, joins = {}, [], [], []
    ok, diag, agg = True, None, None
    agg_clause = next((c for c in clauses if _AGG_CLAUSE.match(c)), None)
    if agg_clause is not None:
        clauses = [c for c in clauses if c != agg_clause]
    for c in clauses:
        mm = _CMP_CLAUSE.match(c)
        if mm and mm.group(1) in cols:
            subj, opw, objtxt = mm.groups()
            if objtxt in cols:
                filters.append(_sys.cmp_filter(_CMP_OPS[opw], cols[subj],
                                               col2=cols[objtxt]))
            else:
                lit = _num(objtxt)
                if isinstance(lit, str):
                    ok = False
                    diag = (f"comparator operand {objtxt!r} is neither a bound "
                            f"variable nor a literal")
                    break
                filters.append(_sys.cmp_filter(_CMP_OPS[opw], cols[subj], lit=lit))
            continue
        coer = _coercion(c, k)
        if coer is not None:
            a, b = coer
            if a in cols and b in cols:
                filters.append(_sys.cmp_filter("eq", cols[a], col2=cols[b]))
            elif a in cols:
                cols[b] = cols[a]                          # alias: one instance, two names
            elif b in cols:
                cols[a] = cols[b]
            else:
                ok = False
                diag = f"coercion clause {c!r} has no bound side"
                break
            continue
        aft, avars, alits = _rule_atom(c, k)
        A_.append(("ruleReads", (rule_cid, aft)))
        if not atoms:
            for v in avars:
                cols.setdefault(v, len(cols) + 1)
        elif avars and cols.get(avars[0]) == len(cols) and len(set(avars)) == len(avars):
            # the linear chain the fragment always compiled: NatJoin on the running
            # tuple's last column — existing models keep bit-identical plans
            joins.append(None)
            for v in avars[1:]:
                cols.setdefault(v, len(cols) + 1)
        else:
            # the general conjunctive shape (Codd's join is not restricted to the
            # last column): join on EVERY bound variable at its position, keep each
            # fresh one ONCE at its first occurrence (a repeat's equality is the
            # fragment boundary, as on the linear path); no bound variable at all is
            # the degenerate cross product
            pairs = tuple((cols[v], i + 1) for i, v in enumerate(avars) if v in cols)
            fresh, seen = [], set()
            for i, v in enumerate(avars):
                if v not in cols and v not in seen:
                    fresh.append(i + 1)
                    seen.add(v)
            joins.append((pairs, tuple(fresh)))
            for v in avars:
                cols.setdefault(v, len(cols) + 1)
        for (vi, lit) in alits:                            # 'Task Status ⟨lit⟩': the role's column
            filters.append(_sys.cmp_filter("eq", cols[avars[vi]], lit=_num(lit)))
        atoms.append((aft, avars))
    # negation groups compile AFTER the positive body binds its columns: the
    # group is its own little conjunctive body (fresh namespace — the 'no X'
    # subject SHADOWS any outer X; other group variables shared-if-bound), and
    # the anti-join keys on the shared columns
    negs = []
    if ok and neg_groups and atoms:
        for parts in neg_groups:
            gatoms, gcols, gfilters, gjoins, subject = [], {}, [], [], None
            for ci, c in enumerate(parts):
                aft, avars, alits = _rule_atom(c, k)
                A_.append(("ruleReads", (rule_cid, aft)))
                if ci == 0:
                    subject = avars[0] if avars else None
                if not gatoms:
                    for v in avars:
                        gcols.setdefault(v, len(gcols) + 1)
                else:
                    pairs = tuple((gcols[v], i + 1)
                                  for i, v in enumerate(avars) if v in gcols)
                    fresh, seen = [], set()
                    for i, v in enumerate(avars):
                        if v not in gcols and v not in seen:
                            fresh.append(i + 1)
                            seen.add(v)
                    gjoins.append((pairs, tuple(fresh)))
                    for v in avars:
                        gcols.setdefault(v, len(gcols) + 1)
                for (vi, lit) in alits:
                    gfilters.append(_sys.cmp_filter("eq", gcols[avars[vi]],
                                                    lit=_num(lit)))
                gatoms.append((aft, avars))
            shared = [v for v in gcols if v in cols and v != subject]
            if not shared:
                ok = False
                diag = "negation group shares no bound variable with the body"
                break
            gwidths = [max(len(av), 1) for (_aft, av) in gatoms]
            negs.append(([a[0] for a in gatoms],
                         [gcols[v] for v in shared], gwidths, gfilters,
                         gjoins, [cols[v] for v in shared]))
    if ok and agg_clause is not None:
        out_v, op, over_v = _AGG_CLAUSE.match(agg_clause).groups()
        if neg_groups:
            ok = False
            diag = "an aggregate with a negation group is not supported"
        elif over_v in cols and out_v not in cols:
            agg = (op, cols[over_v], out_v)
        else:
            ok = False
            diag = (f"aggregate clause needs a bound source and an unbound "
                    f"output ({agg_clause!r})")
    obj = None
    widths = [max(len(av), 1) for (_aft, av) in atoms]
    if ok and atoms and agg is not None:
        op, over_col, out_v = agg
        # a NUMBERED output variable sits in hvars and is excluded from the group;
        # the corpus's UNNUMBERED spelling names the head's aggregated role (last in
        # the head reading), so every numbered head variable is a group key
        rest = [v for v in hvars if v != out_v]
        if all(v in cols for v in rest):
            A_.append(("derivationRule", (hft, atoms[0][0], len(atoms))))
            A_.append(("ruleAgg", (rule_cid,)))
            obj = _sys.compile_agg_rule([a[0] for a in atoms],
                                        [cols[v] for v in rest], over_col, op,
                                        widths, filters, joins)
            # stratified above the closure, full recompute: no ~d variants
            return A_, [(rule_cid, obj)]
        diag = f"aggregate head variables unbound or output {out_v!r} not in head"
    elif ok and atoms and all(v in cols for i, v in enumerate(hvars)
                              if i not in {vi for vi, _l in _hlits}):
        A_.append(("derivationRule", (hft, atoms[0][0], len(atoms))))
        # a head literal fixes its role to a constant: rho applies the spec entry
        # ⟨CONST, lit⟩ as the constant function, so the projection stays one form
        litmap = {vi: lit for vi, lit in _hlits}
        proj = [("CONST", _num(litmap[i])) if i in litmap else cols[v]
                for i, v in enumerate(hvars)]
        if negs:
            # stratified above the closure, full recompute — like aggregates
            A_.append(("ruleNeg", (rule_cid,)))
            obj = _sys.compile_rule_neg([a[0] for a in atoms], proj, len(cols),
                                        widths, filters, joins, negs)
            return A_, [(rule_cid, obj)]
        if len(atoms) == 1 and not filters and proj == list(range(1, widths[0] + 1)):
            # a COPY rule (one positive atom, no filters, identity head): it proves
            # atom ⊆ head at every fixed point, so a matching subset/subtype check
            # is statically discharged (validate_for reads this fact)
            A_.append(("ruleCopies", (rule_cid, atoms[0][0], hft)))
        obj = _sys.compile_rule([a[0] for a in atoms], proj, widths, filters,
                                joins)
    elif ok:
        fixed = {hvars[vi] for vi, _l in _hlits if vi < len(hvars)}
        unbound = sorted(set(hvars) - set(cols) - fixed) if atoms else []
        diag = (f"head variable(s) {unbound} unbound in the body" if unbound
                else "no fact-type clause in the body")
    if obj is None:
        # the rule stays M-facts only, but it SAYS WHY (the diagnostics class)
        if diag:
            A_.append(("ruleDiag", (rule_cid, diag)))
        return A_, []
    # semi-naive: the atom list as M-facts, and one ~d delta variant per atom position
    out = [(rule_cid, obj)]
    for i, (aft, _av) in enumerate(atoms):
        A_.append(("ruleAtom", (rule_cid, i + 1, aft)))
        out.append((f"{rule_cid}~d{i + 1}",
                    _sys.compile_rule_delta([a[0] for a in atoms], proj,
                                            i, widths, filters, joins)))
    return A_, out


# NORMA's derivation-storage markers in LEADING position (the corpus's spelling;
# _DERIVATION handles the same marks trailing a name)
_MARKER_KIND = {"*": "fully-derived", "**": "derived-and-stored",
                "+": "semi-derived", "++": "partially-derived-and-stored"}


def _h_rule_iff(g, k, m):
    """The unnumbered anaphoric rule: strip the storage marker, then the one rule
    handler — numbered and unnumbered spellings are the same mechanism."""
    marker, head, body = g
    return _h_rule_if((head, body), k, m,
                      kind=_MARKER_KIND.get(marker or "*", "fully-derived"))


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
    "spanning_uc2": _h_spanning_corpus, "for_each_mandatory": _h_for_each_mandatory,
    "frequency": _h_frequency, "ring": _h_ring, "subtype_of": _h_subtype,
    "brace_subtypes": _h_brace_subtypes,
    "set_comparison": _h_set_comparison, "disjunctive_mandatory": _h_disjunctive,
    "subset": _h_subset, "equality": _h_equality, "derivation_rule": _h_derivation_rule,
    "rule_if": _h_rule_if,
    "rule_iff": _h_rule_iff,
    "negation": _h_negation, "neg_pair": _h_neg_pair, "class_rule": _h_class_rule,
    "finality": lambda g, k, m: ([("finality", (g[0], int(g[1])))], []),
    "possibility": _h_possibility, "inverse_uc": _h_inverse_uc,
    "sm_def": _h_sm_def, "sm_initial": _h_sm_initial, "sm_from": _h_sm_from,
    "sm_to": _h_sm_to, "sm_trigger": _h_sm_trigger,
    "sm_guard": _h_sm_guard, "sm_emit": _h_sm_emit, "sm_moore": _h_sm_moore,
    "fact_type_reading": _h_fact,
}


def _plan(kind, g, known, modality="alethic"):
    """Dispatch the reading kind to its handler (application by key), never an if/elif chain."""
    return _PLAN.get(kind, lambda g, k, m: ([], []))(g, known, modality)


def compile(stmt, D, known=()):
    from .reduce import apply as _apply
    from .lam import atom as _A
    kind, g, modality = analyze(stmt)
    if kind == "fact_type_reading" and _prose_suspect(g[0], known):
        # a readings PARAGRAPH, not a reading: report it, never declare it (the
        # old engine's check warns the author; silence was the data loss)
        kind, g = "UNPARSED", (stmt,)
    elif kind == "rule_iff" and _prose_suspect(g[1], known):
        # prose containing ' iff ' claims the rule recognizer, but a real rule
        # HEAD is a reading — commas, colons or parentheses there mean paragraph
        kind, g = "UNPARSED", (stmt,)
    asserts, cons = _plan(kind, g, known, modality)
    for cell, fact in asserts:
        D = _apply(_A(2), ast.run(to_lam(fact), D, cell_name=cell))
    for name, obj in cons:
        # a compiled definition is stored INTO the schema's own D, not the process seed
        # (Def. AREST / Cor. closure): ingestion mutates only the store being ingested into
        D = _apply(ast.DefineIn(name, obj), D)
    return D, kind


# ---- self-host gate two: classification by the ingested RULES, dispatch by the
# ingested Classification-has-Translator table; Stage-1 (the regex productions) only
# extracts fields. Generic classifications yield to specific ones, mirroring the
# grammar file's own arbitration-rule values. ----
_GENERIC_CLASSIFICATIONS = {"Fact Type Reading", "Instance Fact"}

_PRODUCTION_CACHE = {}


def _productions():
    """kind → its Stage-1 patterns (the bootstrap kernel's field extractors)."""
    if not _PRODUCTION_CACHE:
        for kind, pat in _CLASSIFY:
            _PRODUCTION_CACHE.setdefault(kind, []).append(pat)
    return _PRODUCTION_CACHE


def _stmt_translator_impl(kinds):
    """A statement translator as a REGISTERED definition (self-host gate three):
    ⟨stmt, modality, ctx, D⟩ ↦ D′. The small components decode; D threads through as
    lambda untouched. Inside, the Stage-1 productions extract fields and _plan
    asserts — the translator's own production list is its private binding, not an
    engine dispatch table."""
    def impl(mu):
        def g(operand):
            from .reduce import apply as _apply
            from .lam import atom as _A, from_lam as _fl
            stmt = _fl(_apply(_A(1), operand))
            mod = _fl(_apply(_A(2), operand)) or None
            names, subs, fts = _fl(_apply(_A(3), operand))
            D = _apply(_A(4), operand)
            known = _Known(names, {s: tuple(a) for (s, a) in subs}, set(fts))
            for kind in kinds:
                mm = next((p.match(stmt) for p in _productions().get(kind, ())
                           if p.match(stmt)), None)
                if mm is None:
                    continue
                asserts, objs = _plan(kind, mm.groups(), known, mod)
                for cell, fact in asserts:
                    D = _apply(_A(2), ast.run(to_lam(fact), D, cell_name=cell))
                for name, obj in objs:
                    D = _apply(ast.DefineIn(name, obj), D)
                break
            return D
        return g
    return impl


def register_translators():
    """Register the statement translators into DEFS under the names the grammar's
    Classification-has-Translator readings dispatch to (the same boundary as the
    federation connectors: DEFS is the DI container, swapping is re-registering).
    Idempotent; call again to restore the real bindings after a test swapped one."""
    from .defs import register
    for name, kinds in (
        ("translate_nouns", ["entity_type", "value_type", "subtype_of",
                             "brace_subtypes"]),
        ("translate_subtypes", ["subtype_of", "brace_subtypes"]),
        ("translate_enum_values", ["value_constraint"]),
        ("translate_data_types", ["data_type"]),
        ("translate_instance_facts", ["fact_type_reading"]),
        ("translate_fact_types", ["fact_type_reading"]),
        ("translate_derivation_mode_facts", ["fact_type_reading"]),
        ("translate_derivation_rules", ["rule_if", "rule_iff", "derivation_rule", "class_rule"]),
        ("translate_cardinality_constraints", ["uniqueness", "inverse_uc",
                                               "spanning_uc", "spanning_uc2",
                                               "frequency",
                                               "neg_uniqueness", "mandatory",
                                               "for_each_mandatory",
                                               "neg_mandatory"]),
        ("translate_ring_constraints", ["ring"]),
        ("translate_set_constraints", ["set_comparison", "subset", "equality",
                                       "disjunctive_mandatory"]),
        ("translate_value_constraints", ["value_constraint"]),
        ("translate_state_machines", ["sm_def", "sm_initial", "sm_from", "sm_to",
                                      "sm_trigger", "sm_guard", "sm_emit",
                                      "sm_moore"]),
        ("translate_finality", ["finality"]),
        ("translate_negation", ["neg_pair", "negation"]),
    ):
        register(name, _stmt_translator_impl(kinds))

_GRAMMAR_CACHE = {}


def grammar_D():
    """The ingested grammar (shared/forml2-grammar.md — 'the parser is this
    file'), cached per process and THAWED from the local persistence model across
    processes (persist.ingest_frozen: the compiled D freezes to a content-keyed
    snapshot; the first process on a machine pays the ingest, later ones thaw in
    milliseconds — definitions are data, so the snapshot carries the rules)."""
    if "D" not in _GRAMMAR_CACHE:
        from . import persist, paths
        p = paths.shared("forml2-grammar.md")
        _GRAMMAR_CACHE["D"] = persist.ingest_frozen(open(p, encoding="utf-8").read())
    return _GRAMMAR_CACHE["D"]


def compile_model_selfhost(text, D=None):
    """Gate two of the self-host: per statement, tokenize (Stage-1, the bootstrap
    kernel) → classify via the RULES (run_rules over the ingested grammar) → dispatch
    via the ingested Classification-has-Translator table → translate (Stage-1 field
    extraction feeding the handler). Statements the rules do not classify are reported
    unclassified — the rules, not the regex order, are the classifier. Asserts are
    idempotent, so co-firing translators are harmless by construction."""
    from . import meta, system as _sys, defs as _dm
    from .reduce import apply as _apply
    from .lam import atom as _A
    import pyarest.lam as _L
    gD = grammar_D()
    dispatch = {}
    for r in _sys._pop_rows(gD, "Classification_has_Translator"):
        if len(r) >= 2:
            dispatch.setdefault(r[0], []).append(r[1])
    stmts = statements(text)
    names = _known(stmts)
    subs, fts, plain = _prepass_context(stmts, names)
    known = _Known(names, subs, fts, plain)
    ctx = to_lam((tuple(sorted(names)),
                  tuple(sorted((s, tuple(sorted(a))) for s, a in subs.items())),
                  tuple(sorted(fts))))
    if D is None:
        D = meta.initial_D()
    unclassified = []
    for stmt in stmts:
        mod, sign, inner = _split_modality(stmt)
        if sign == "possibility":
            continue
        cls = classify_via_M(gD, inner, nouns=known)
        specific = cls - _GENERIC_CLASSIFICATIONS
        cls = specific or cls
        translators = []
        for c in sorted(cls):
            for t in dispatch.get(c, []):
                if t not in translators:
                    translators.append(t)
        if not translators:
            unclassified.append(stmt)
            continue
        for t in translators:
            if _dm.latest.get(t, ("",))[0] != "registered":
                continue                                       # a name M declares, this
            operand = _L.SEQ(                                  # host lacks: skipped, the
                _L.CONS(_A(inner))(                            # boundary's graceful absence
                    _L.CONS(_A(mod or ""))(
                        _L.CONS(ctx)(_L.CONS(D)(_L.NIL)))))
            with _dm.step(D):
                D = _apply(_A(t), operand)                     # rho: dispatch through DEFS
    return D, {"unclassified": unclassified}


def compile_model(text, D=None, context_from=None):
    """Fold `compile` over a whole NORMA verbalization into M (two-pass). Returns
    (D, report). With `context_from`, the known context seeds from that store —
    compile the app's statements ATOP a preloaded base whose types, subtypes and
    fact types resolve exactly like in-text declarations."""
    from . import meta
    from collections import Counter
    if D is None:
        D = meta.initial_D()
    b_names, b_edges, b_fts = ((set(), (), ()) if context_from is None
                               else _context_of(context_from))
    stmts = statements(text)
    names = set(_known(stmts)) | b_names
    subs, fts, plain = _prepass_context(stmts, names, b_edges, b_fts)
    known = _Known(names, subs, fts, plain)
    report, unparsed = Counter(), []
    for s in stmts:
        D, kind = compile(s, D, known)
        report[kind] += 1
        if kind == "UNPARSED":
            unparsed.append(s)
    diags = [tuple(r) for r in system._pop_rows(D, "ruleDiag")]
    return D, {"total": len(stmts), "kinds": dict(report), "unparsed": unparsed,
               "rule_diagnostics": diags}


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


def validate_for(fact_type, D, partition=None):
    """Build `fact_type`'s validate from M's constraint facts, respecting modality: alethic
    constraints block commit, deontic ones only flag (AREST Def. Violation). Attachment is
    read off M by kind (_ATTACH); the constraint names reflect to their objects via rho within
    the step's D (Cor. closure). Every parsed family enforces — local ones over the target
    population, scoped ones over ⟨P, D⟩. With a `partition`, a scoped constraint whose read
    fact type is ABSORBED is rebuilt over the VIEW (ftpop_expr: index + dynamic fetch), the
    seam the RMAP plan recorded — like the spans-driven families, M is load-bearing and the
    object is constructed at validate time."""
    from .lam import atom as _A

    def _absorbed(ft):
        return partition is not None and isinstance(ft, str) and partition.get(ft, ft) != ft

    def _vp(ft):
        return system.ftpop_expr(ft, partition) if _absorbed(ft) else ft

    def _rebuilt(f, name):
        kind = f[1]
        if kind in ("subtype", "subset") and name == f[0] and _absorbed(f[3]):
            return C.scoped_subset(_vp(f[3]))
        if kind == "equality":
            if name == f[0] + "_a" and _absorbed(f[3]):
                return C.scoped_equality_side(_vp(f[3]))
            if name == f[0] + "_b" and _absorbed(f[2]):
                return C.scoped_equality_side(_vp(f[2]))
        if kind == "mandatory" and name == f[0] + "_e" and _absorbed(f[2]):
            return C.scoped_mandatory_facts(_vp(f[2]))
        if kind in ("exclusion", "exclusive_or", "disjunctive_mandatory") and "@" in name:
            clauses = tuple(f[3])
            if any(_absorbed(c) for c in clauses):
                pops = {c: system.ftpop_expr(c, partition) for c in clauses if _absorbed(c)}
                target = name.split("@", 1)[1]
                if kind == "exclusion":
                    return C.scoped_exclusion(clauses, target, pops)
                if kind == "exclusive_or":
                    return C.scoped_exclusive_or(f[2], clauses, target, pops)
                return C.scoped_inclusive_or(f[2], clauses, target, pops)
        return None

    spans = {}
    for r in _cells(D, "spans"):
        if len(r) == 2:
            spans.setdefault(r[0], []).append(r[1])
    copies = {tuple(r[1:3]) for r in _cells(D, "ruleCopies") if len(r) >= 3}
    local, scoped = [], []
    for f in _cells(D, "constraint"):
        if len(f) < 3:
            continue
        if f[1] in ("subtype", "subset") and len(f) >= 4 and (f[2], f[3]) in copies:
            # a copy rule antecedent->consequent proves the inclusion at every fixed
            # point of F_S (Def. derive), and Def. create validates the candidate
            # POST-state, whose derived population contains the copy: discharged
            continue
        for name, is_local in _ATTACH.get(f[1], lambda f, ft: [])(f, fact_type):
            # spec §4.3: the constraint FACT selects the family expression and binds the
            # role sequence — for the spans-driven families the object is CONSTRUCTED
            # from M's spans facts at validate time, so M is load-bearing, not decorative
            if is_local and f[1] in ("uniqueness", "spanning_uniqueness") and name in spans:
                local.append((C.uniqueness(sorted(spans[name])), f[-1]))
                continue
            fresh = None if is_local else _rebuilt(f, name)
            if fresh is not None:
                scoped.append((fresh, f[-1]))
            else:
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
    "spanning_uc2": lambda g: (f"Each {g[0]} combination occurs at most once "
                               f"in the population of {g[1]}"),
    "for_each_mandatory": lambda g: f"For each {g[0]}, some {g[1]}",
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
    "rule_iff": lambda g: f"{g[1]} iff {g[2]}",
    "negation": lambda g: f"{g[0]} ~{g[1]}",
    "neg_pair": lambda g: f"{g[0]} {g[1]} {g[2]}",
    "finality": lambda g: f"{g[0]} becomes final at depth {g[1]}",
    "brace_subtypes": lambda g: "{%s} are %ssubtypes of %s" % (g[0], g[1] or "", g[2]),
    "class_rule": lambda g: f"{g[0]} has {g[1]} '{g[2]}' iff {g[3]}",
    "uniqueness": lambda g: f"Each {g[0]} {g[1]} {g[2]}",
    "mandatory": lambda g: f"Each {g[0]} some {g[1]}",
    "neg_uniqueness": lambda g: ("any {0} more than one {1}".format(*g) if len(g) == 2 else
                                 "For each {0}, it is impossible that that {0} {1} more than one {2}".format(*g)),
    "neg_mandatory": lambda g: ("any {0} no {1}".format(*g) if len(g) == 2 else
                                "For each {0}, it is impossible that that {0} {1} no {2}".format(*g)),
    "inverse_uc": lambda g: f"For each {g[0]}, {g[1]} {g[2]} that applies",
    "fact_type_reading": lambda g: g[0],
    "sm_def": lambda g: f"State Machine Definition '{g[0]}' is for Noun '{g[1]}'",
    "sm_initial": lambda g: f"Status '{g[0]}' is initial in State Machine Definition '{g[1]}'",
    "sm_from": lambda g: f"Transition '{g[0]}' is from Status '{g[1]}'",
    "sm_to": lambda g: f"Transition '{g[0]}' is to Status '{g[1]}'",
    "sm_trigger": lambda g: f"Transition '{g[0]}' is triggered by Fact Type '{g[1]}'",
    "sm_guard": lambda g: f"Transition '{g[0]}' is guarded by Fact Type '{g[1]}'",
    "sm_emit": lambda g: f"Transition '{g[0]}' emits '{g[1]}'",
    "sm_moore": lambda g: f"Status '{g[0]}' emits '{g[1]}'",
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


# the statement translators register at import, like the federation bindings: the
# names the grammar dispatches to resolve through DEFS from the first statement on
register_translators()
