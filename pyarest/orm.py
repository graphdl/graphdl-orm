"""ORM constraint primitives as FFP objects over the FILE population (paper
Def. Schema; Halpin ORM2). Each constraint c is a ρ-application: V_c = (ρc):X
is the finite set of bindings that offend c (Def. Violation), and validate
rejects an alethic commit iff some V_c ≠ φ. Built on Codd's θ₁ (theta.py) — no
new primitives.

  set difference   X ∖ Y = ⟨x∈X : x∉Y⟩ = α(1) ∘ Filter(∉) ∘ distr
  uniqueness       Unique(L):X       the tuples of X sharing their L-key
  mandatory        Mandatory(r):⟨O,X⟩ the objects of O not playing role r
"""
from .objects import Atom, Seq, PHI
from .theta import Filter, NatJoin, Project

_S = lambda *xs: Seq(xs)
_COMP = Atom("COMP")
_CONS = Atom("CONS")
_CONST = Atom("CONST")
_ALPHA = Atom("ALPHA")
_ID = Atom("id")
_1 = Atom("1")
_2 = Atom("2")
_EQ = Atom("eq")
_NULL = Atom("null")
_NOT = Atom("not")
_LEN = Atom("length")
_DISTL = Atom("distl")
_DISTR = Atom("distr")

_sel = lambda i: Atom(str(i))
_comp = lambda *fs: _S(_COMP, *fs)
_cons = lambda *fs: _S(_CONS, *fs)

# x ∉ Y  as a predicate on ⟨x, Y⟩ :  null ∘ Filter(eq) ∘ distl
_notmember = _comp(_NULL, Filter(_EQ), _DISTL)

# set difference  X ∖ Y = ⟨x∈X : x∉Y⟩  =  α(1) ∘ Filter(∉) ∘ distr
setminus = _comp(_S(_ALPHA, _1), Filter(_notmember), _DISTR)


def Mandatory(r):
    """Mandatory constraint on role r: V = ObjectPop ∖ ⟨r-th value of each fact⟩.
        Mandatory(r):⟨O, X⟩ = setminus:⟨O, α(r):X⟩
    """
    return _comp(setminus, _cons(_1, _comp(_S(_ALPHA, _sel(r)), _2)))


def Unique(L):
    """Uniqueness constraint on roles L: V = the tuples of X whose L-key recurs.
    A tuple offends iff #{t' : keyL(t')=keyL(t)} ≠ 1 (it matches only itself when
    unique).  Unique(L):X = α(1) ∘ Filter(dup) ∘ distr ∘ [id, id]
    """
    keyL = _cons(*tuple(_sel(i) for i in L))                        # the L-key: [sel_L]
    same = _comp(_EQ, _cons(_comp(keyL, _1), _comp(keyL, _2)))      # keyL(t) = keyL(t')
    matches = _comp(Filter(same), _DISTL)                          # t' ∈ X with same key
    dup = _comp(_NOT, _EQ, _cons(_comp(_LEN, matches), _S(_CONST, Atom(1))))  # count ≠ 1
    return _comp(_S(_ALPHA, _1), Filter(dup), _DISTR, _cons(_ID, _ID))


# ============================ ORM schema structure ============================
# Fact types have roles; a role is played by an object type; both are first-class
# (a ring fact type has two roles with one player).
_VT = Atom("ValueType")
_ET = Atom("EntityType")
_ROLE = Atom("Role")
_FT = Atom("FactType")


# ===================== data types (NORMA PortableDataType) =====================
# The intrinsic data types NORMA seeds into every model (AddIntrinsicDataTypes-
# FixupListener); one DataType-derived class per value. Each is classified by its
# group, its range support (how open range bounds behave), whether it carries a
# length/precision facet and a scale facet, and whether its values auto-generate.
#   range support  "continuous"    real / lexical / temporal — open bounds exact
#                  "discontinuous" integral — an open bound snaps to the nearest
#                                  value (DataType.AdjustDiscontinuous*Bound)
#                  "none"          unorderable (uuid, raw, logical) — no ranges
#   autogen        "required" (auto-generating identifier — counter, UUID, timestamp,
#                  or surrogate, per the whitepaper's Def. Schema), "available" (opt-in:
#                  the signed integers), "" (never)
# Verified against DataTypesGenerator.cs (RangeSupport / AutoGeneratable / Scale).
_CONTINUOUS, _DISCRETE, _NORANGE = "continuous", "discontinuous", "none"
_DATA_TYPES = {                       # group      range         length scale autogen
    "Unspecified":            ("none",     _CONTINUOUS, False, False, ""),
    "Fixed Length Text":      ("text",     _CONTINUOUS, True,  False, ""),
    "Variable Length Text":   ("text",     _CONTINUOUS, True,  False, ""),
    "Large Length Text":      ("text",     _CONTINUOUS, False, False, ""),
    "Signed Integer":         ("numeric",  _DISCRETE,   False, False, "available"),
    "Signed Small Integer":   ("numeric",  _DISCRETE,   False, False, "available"),
    "Signed Large Integer":   ("numeric",  _DISCRETE,   False, False, "available"),
    "Unsigned Integer":       ("numeric",  _DISCRETE,   False, False, ""),
    "Unsigned Tiny Integer":  ("numeric",  _DISCRETE,   False, False, ""),
    "Unsigned Small Integer": ("numeric",  _DISCRETE,   False, False, ""),
    "Unsigned Large Integer": ("numeric",  _DISCRETE,   False, False, ""),
    "Auto Counter":           ("numeric",  _DISCRETE,   False, False, "required"),
    "Floating Point":         ("numeric",  _CONTINUOUS, True,  False, ""),
    "Single Precision Floating Point": ("numeric", _CONTINUOUS, False, False, ""),
    "Double Precision Floating Point": ("numeric", _CONTINUOUS, False, False, ""),
    "Decimal":                ("numeric",  _CONTINUOUS, True,  True,  ""),
    "Money":                  ("numeric",  _CONTINUOUS, True,  True,  ""),
    "UUID":                   ("numeric",  _NORANGE,    False, False, "required"),
    "Fixed Length Raw Data":  ("rawdata",  _NORANGE,    True,  False, ""),
    "Variable Length Raw Data": ("rawdata", _NORANGE,   True,  False, ""),
    "Large Length Raw Data":  ("rawdata",  _NORANGE,    False, False, ""),
    "Picture Raw Data":       ("rawdata",  _NORANGE,    False, False, ""),
    "OLE Object Raw Data":    ("rawdata",  _NORANGE,    False, False, ""),
    "Auto Timestamp":         ("temporal", _CONTINUOUS, False, False, "required"),
    "Time":                   ("temporal", _CONTINUOUS, False, False, ""),
    "Date":                   ("temporal", _CONTINUOUS, False, False, ""),
    "Date and Time":          ("temporal", _CONTINUOUS, False, False, ""),
    "True or False":          ("logical",  _NORANGE,    False, False, ""),
    "Yes or No":              ("logical",  _NORANGE,    False, False, ""),
    "Row Id":                 ("other",    _NORANGE,    False, False, "required"),   # surrogate
    "Object Id":              ("other",    _NORANGE,    False, False, "required"),   # surrogate
}
_dt_facts = lambda n: _DATA_TYPES.get(n, ("none", _CONTINUOUS, False, False, ""))


def data_type(name, length=0, scale=0):
    """An intrinsic data type carrying its per-use facets: DataTypeLength (max
    length / precision) and DataTypeScale (digits right of the decimal point).
    These facets live on the value type (ObjectType, CustomStorage) and flow
    straight through to a column under RMAP."""
    return _S(Atom("DataType"), Atom(name), Atom(length), Atom(scale))

dt_name = lambda dt: dt.xs[1].v
dt_length = lambda dt: dt.xs[2].v
dt_scale = lambda dt: dt.xs[3].v
dt_range_support = lambda dt: _dt_facts(dt_name(dt))[1]     # continuous / discontinuous / none
dt_autogen = lambda dt: _dt_facts(dt_name(dt))[4]           # required / available / ""


def value_type(name, data="Variable Length Text", length=0, scale=0, auto_generated=None):
    """A value type — an object type that HAS a data type. IsValueType ≡ DataType≠∅
    (ObjectType.cs): value-ness is *having* a data type, not a separate kind. Carries
    the length/scale facets and whether its values are auto-generated (defaulting
    from the data type: an Auto Counter / Auto Timestamp is 'required' → auto)."""
    dt = data if isinstance(data, Seq) else data_type(data, length, scale)
    auto = (dt_autogen(dt) == "required") if auto_generated is None else auto_generated
    return _S(_VT, Atom(name), dt, Atom("auto") if auto else Atom("supplied"))

vt_name = lambda vt: vt.xs[1].v
vt_data_type = lambda vt: vt.xs[2]                          # the ⟨DataType, name, len, scale⟩
vt_auto = lambda vt: vt.xs[3].v == "auto"


def role(fact_type_name, position, player):
    """A Role — a first-class object (NORMA Role): played by an object type
    (ObjectTypePlaysRole) at a position in a fact type. Constraints reference
    these role objects, not indices."""
    return _S(_ROLE, Atom(fact_type_name), Atom(position), Atom(player))


def fact_type(name, players, reading):
    """A fact type; its roles are created here, each played by an object type.
    `players` is the list of player names in role order."""
    roles = _S(*tuple(role(name, i, p) for i, p in enumerate(players)))
    return _S(_FT, Atom(name), roles, Atom(reading))


ft_roles = lambda ft: ft.xs[2].xs                 # the fact type's role objects
role_fact_type = lambda r: r.xs[1].v
role_position = lambda r: r.xs[2].v
role_player = lambda r: r.xs[3].v


def entity_type(name, preferred_identifier, independent=False):
    """An entity type — an object type with no data type (IsValueType is false),
    identified by its preferred identifier: a uniqueness constraint (internal or
    external) or a reference scheme (EntityTypeHasPreferredIdentifier). `independent`
    (NORMA IsIndependent) marks a type whose instances may exist while playing no
    roles — under RMAP it maps to its own table."""
    return _S(_ET, Atom(name), preferred_identifier,
              Atom("independent") if independent else Atom("dependent"))

et_name = lambda et: et.xs[1].v
et_preferred_identifier = lambda et: et.xs[2]
et_independent = lambda et: et.xs[3].v == "independent"


# --- a constraint reduces to its own violation set: (ρc):P = V_c (Def. Violation) ---
# c = ⟨head, kind, roleSeqs, modality, *extra⟩ with head = ⟨COMP, vc, arrange⟩. By
# metacomposition (ρc):P = (ρ head):⟨c,P⟩ = vc:(arrange:⟨c,P⟩): `arrange` pulls the θ₁
# core's inputs — a fact type's relation, an object-type population, a pair of
# relations — out of the tagged population P (the 2nd of ⟨c,P⟩); `vc` is the core.
_TL = Atom("tl")
_P = _2                                                              # ⟨c, P⟩ → P


def _relation(ft_name):
    """Fact type `ft_name`'s untagged relation from the tagged population (a set of
    ⟨factType, v₁..vₙ⟩ facts): α(tl) ∘ Filter(eq∘[1, ft_namē])."""
    tag_is = _comp(_EQ, _cons(_1, _S(_CONST, Atom(ft_name))))
    return _comp(_S(_ALPHA, _TL), Filter(tag_is))

_rel = lambda ft: _comp(_relation(ft), _P)                          # R_ft out of ⟨c, P⟩
_ft_of = lambda role_seq: role_fact_type(role_seq[0])               # the (single) fact type


def _constraint(kind, role_seqs, modality, vc, arrange, *extra):
    """A constraint FFP object with head = vc ∘ arrange, so that (ρc):P = V_c."""
    return _S(_S(_COMP, vc, arrange), Atom(kind),
              _S(*(_S(*rs) for rs in role_seqs)), Atom(modality), *extra)

c_kind = lambda c: c.xs[1].v
c_roles = lambda c: c.xs[2]                                         # ⟨⟨role,…⟩, …⟩
c_modality = lambda c: c.xs[3].v
c_evaluator = lambda c: c.xs[0].xs[1]                              # the θ₁ core vc
c_extra = lambda c: c.xs[4:]


# ---- uniqueness over a ConstraintRoleSequence of Role objects (NORMA) ----
def _external_uniqueness(role_seq):
    """External uniqueness across two binary identifying fact types ⟨entity, value⟩:
    join on the shared entity (role 1) and require the value tuple unique — the
    joined ⟨entity, v₁, v₂⟩ with a non-unique ⟨v₁, v₂⟩ offend.  = Unique[2,3] ∘ ⋈₁."""
    return _comp(Unique([2, 3]), NatJoin(1))


def uniqueness(role_seq, modality="alethic"):
    """Uniqueness over a role sequence: internal (roles share one fact type) or
    external (spanning two identifying fact types, joined on the shared entity)."""
    fts = list(dict.fromkeys(role_fact_type(r) for r in role_seq))
    internal = len(fts) == 1
    if internal:
        vc, arrange = Unique([role_position(r) + 1 for r in role_seq]), _rel(fts[0])
    else:
        vc, arrange = _external_uniqueness(role_seq), _S(_CONS, _rel(fts[0]), _rel(fts[1]))
    return _constraint("UniquenessConstraint", [role_seq], modality, vc, arrange,
                       Atom("internal") if internal else Atom("external"))


def preferred_identifier(uniqueness_constraint):
    """An entity type's identity IS a uniqueness constraint (internal or external)."""
    return uniqueness_constraint


# ======================= more constraint families over θ₁ =====================
_CAT = Atom("cat")
_swap = _S(_CONS, _2, _1)

# x ∈ Y  as a predicate on ⟨x, Y⟩ :  not ∘ null ∘ Filter(eq) ∘ distl
_member = _comp(_NOT, _NULL, Filter(_EQ), _DISTL)
# intersection  A ∩ B = ⟨x∈A : x∈B⟩  =  α(1) ∘ Filter(∈) ∘ distr
intersection = _comp(_S(_ALPHA, _1), Filter(_member), _DISTR)

# set-comparison constraints on ⟨A, B⟩ (each a relation; e.g. projected role columns)
Subset = setminus                                                    # V = A ∖ B      (⊆ iff φ)
Exclusion = intersection                                             # V = A ∩ B      (disjoint iff φ)
Equality = _comp(_CAT, _S(_CONS, setminus, _comp(setminus, _swap)))  # V = (A∖B)∪(B∖A)  (= iff φ)


# =============================== reference scheme =============================
def reference_scheme(mode, data_type, generated=False):
    """How an entity type is identified: a NORMA reference mode (General, Popular,
    UnitBased) over a data type, auto-generated or supplied (paper Def. Schema)."""
    kind = Atom("auto") if generated else Atom("supplied")
    return _S(Atom("RefScheme"), Atom(mode), Atom(data_type), kind)


# NORMA intrinsic reference modes: name → (implied PortableDataType, kind). A
# reference mode carries the data type the value type it creates will get (id →
# Auto Counter, name → Variable Length Text, uuid → UUID); a unit-based mode →
# Decimal. The kind's FormatString ({0}=EntityName, {1}=ModeName) names the value type.
_REFERENCE_MODES = {
    "id": ("Auto Counter", "Popular"),   "Id": ("Auto Counter", "Popular"),
    "ID": ("Auto Counter", "Popular"),   "nr": ("Auto Counter", "Popular"),
    "Nr": ("Auto Counter", "Popular"),   "code": ("Variable Length Text", "Popular"),
    "Code": ("Variable Length Text", "Popular"), "name": ("Variable Length Text", "Popular"),
    "Name": ("Variable Length Text", "Popular"), "uuid": ("UUID", "Popular"),
    "UUID": ("UUID", "Popular"),         "Uuid": ("UUID", "Popular"),
}
_MODE_FORMAT = {"General": "{1}", "Popular": "{0}_{1}", "UnitBased": "{1}Value"}


def reference_mode(name, data_type=None, kind=None):
    """A NORMA reference mode: it implies the data type of the value type it creates
    and a kind (General/Popular/UnitBased). Intrinsic modes carry a portable type; a
    custom mode defaults to Decimal, UnitBased."""
    dt, k = _REFERENCE_MODES.get(name, ("Decimal", "UnitBased"))
    return _S(Atom("ReferenceMode"), Atom(name), Atom(data_type or dt), Atom(kind or k))

refmode_name = lambda m: m.xs[1].v
refmode_data_type = lambda m: m.xs[2].v
refmode_kind = lambda m: m.xs[3].v


def expand_reference_mode(entity_name, mode):
    """Expand a reference mode into its reference scheme (NORMA CreateReferenceMode):
    a value type named by the kind's FormatString (e.g. Person_id), a binary
    identifying fact type, and an internal uniqueness on the value-type role — the
    preferred identifier. So identity IS a uniqueness constraint, and the value
    type's data type becomes the identifying (primary-key) column's type."""
    vtn = _MODE_FORMAT[refmode_kind(mode)].format(entity_name, refmode_name(mode))
    vt = value_type(vtn, refmode_data_type(mode))
    ft = fact_type(entity_name + " has " + vtn, [entity_name, vtn], "has")
    uc = uniqueness([ft_roles(ft)[1]])                       # unique on the value-type role
    return _S(Atom("ReferenceScheme"), vt, ft, uc)

refscheme_value_type = lambda rs: rs.xs[1]
refscheme_fact_type = lambda rs: rs.xs[2]
refscheme_identifier = lambda rs: rs.xs[3]                   # the preferred-identifier uniqueness


# ======================= data type → column (RMAP hop) ========================
# "Defining a data type defines a column type in a table": a column's type IS its
# value type's data type — the same portable type, Length and Scale flow straight
# through (NORMA ColumnProperties: the Column.DataType getter returns
# valueType.DataType, with Length/Scale redirected to the same value type).
def column(value_type_obj, mandatory=True):
    """A relational column whose type is a value type's data type + length + scale."""
    return _S(Atom("Column"), Atom(vt_name(value_type_obj)), vt_data_type(value_type_obj),
              Atom("mandatory" if mandatory else "optional"))

col_name = lambda c: c.xs[1].v
col_data_type = lambda c: c.xs[2]                            # the ⟨DataType, name, len, scale⟩


def identifier_column(reference_scheme):
    """An entity's primary-key column: RMAP absorbs the reference-scheme value type,
    so the identifying value type's data type becomes the PK column's type."""
    return column(refscheme_value_type(reference_scheme))


# ============================ state machine definitions =======================
# "A state machine is itself a set of facts (a status, its transitions, and the
# trigger fact type of each)" (§2). So a machine is an ORM schema built from the
# primitives above; advancing it is the AST step (Prop. 3), and RMAP maps these
# transition facts to the machine's rows.
_name = reference_scheme("Popular", "Variable Length Text")  # identified by a supplied name
SMDefinition = entity_type("State Machine Definition", _name)
StatusType = entity_type("Status", _name)
TransitionType = entity_type("Transition", _name)

SMD_is_for_Noun = fact_type("SMD is for Noun", ["State Machine Definition", "Noun"], "is for")
Status_is_initial_in_SMD = fact_type("Status is initial", ["Status", "State Machine Definition"], "is initial in")
Transition_is_from_Status = fact_type("Transition is from", ["Transition", "Status"], "is from")
Transition_is_to_Status = fact_type("Transition is to", ["Transition", "Status"], "is to")
Transition_is_triggered_by = fact_type("Transition triggered by", ["Transition", "Fact Type"], "is triggered by")


# ==================== remaining constraint families over θ₁ ====================
_LT = Atom("lt")
_LE = Atom("le")
_GT = Atom("gt")
_GE = Atom("ge")
_AND = Atom("and")
_COND = Atom("COND")


def ValueComparison(r1, r2, op):
    """Role r1 `op` role r2 on each fact; V = the facts where it fails.
    op ∈ {lt, le, gt, ge, eq, ne} (NORMA ValueComparisonOperator)."""
    keys = _cons(_sel(r1), _sel(r2))
    hold = _comp(_NOT, _EQ, keys) if op == "ne" else _comp(Atom(op), keys)   # r1 op r2 holds
    return Filter(_comp(_NOT, hold))                                          # keep where it fails


def Frequency(L, lo, hi):
    """Each L-key combination occurs a count in [lo, hi]; V = facts outside it."""
    keyL = _cons(*tuple(_sel(i) for i in L))
    same = _comp(_EQ, _cons(_comp(keyL, _1), _comp(keyL, _2)))
    count = _comp(_LEN, Filter(same), _DISTL)
    inrange = _comp(_AND, _cons(_comp(_GE, _cons(count, _S(_CONST, Atom(lo)))),
                                _comp(_LE, _cons(count, _S(_CONST, Atom(hi))))))
    return _comp(_S(_ALPHA, _1), Filter(_comp(_NOT, inrange)), _DISTR, _cons(_ID, _ID))


def Value(r, allowed):
    """Role r's value must lie in `allowed` (a value set); V = facts outside it."""
    notin = _comp(_NULL, Filter(_EQ), _DISTL)                # r:t ∉ allowed
    return Filter(_comp(notin, _cons(_sel(r), _S(_CONST, allowed))))


# --- value ranges & enumeration (NORMA ValueConstraint / ValueRange) ----------
# A value constraint holds a set of ValueRanges. RangeInclusion ∈ {NotSet
# (unbounded, empty bound), Open (endpoint excluded), Closed (included)}. A single
# discrete value is a degenerate range with MinValue == MaxValue — so an enumeration
# {a, b, c} is three point ranges. Continuous types take open bounds as-is;
# discontinuous (integral) types snap an open bound to the nearest value.
_OR = Atom("or")


def value_range(lo, hi=None, lo_incl="Closed", hi_incl="Closed"):
    """A ValueRange ⟨lo, hi, loInclusion, hiInclusion⟩. `hi` defaults to `lo` (a
    single value — the enumeration case); None on a bound means unbounded (NotSet)."""
    if hi is None and lo is not None:
        hi = lo                                              # single value: MinValue == MaxValue
    return _S(Atom("ValueRange"),
              Atom("" if lo is None else lo), Atom("" if hi is None else hi),
              Atom("NotSet" if lo is None else lo_incl), Atom("NotSet" if hi is None else hi_incl))


def _range_text(vr):
    """One range's reading: `v` (single), `[lo..hi]`, `(lo..hi)`, `[lo..`, `..hi]`."""
    lo, hi, li, hi_i = (x.v for x in vr.xs[1:5])
    if lo == hi and li != "NotSet":
        return str(lo)                                       # single value — no delimiter
    left = "" if li == "NotSet" else ("(" if li == "Open" else "[")
    right = "" if hi_i == "NotSet" else (")" if hi_i == "Open" else "]")
    return "{0}{1}..{2}{3}".format(left, lo, hi, right)


def _range_pred(vr):
    """FFP predicate on a value v: v lies in this range (open bound ⇒ strict)."""
    lo, hi, li, hi_i = (x.v for x in vr.xs[1:5])
    parts = []
    if li != "NotSet":
        parts.append(_comp(_GT if li == "Open" else _GE, _cons(_ID, _S(_CONST, Atom(lo)))))
    if hi_i != "NotSet":
        parts.append(_comp(_LT if hi_i == "Open" else _LE, _cons(_ID, _S(_CONST, Atom(hi)))))
    if not parts:
        return _S(_CONST, Atom("T"))                         # a fully unbounded range admits everything
    pred = parts[0]
    for p in parts[1:]:
        pred = _comp(_AND, _cons(pred, p))                   # v ↦ (v ≥ lo) ∧ (v ≤ hi)
    return pred


def InRanges(r, ranges):
    """Role r's value must lie in some ValueRange; V = facts outside every range.
    Enumeration is the all-point-ranges case. Uses ordered comparison (ge/le),
    which holds within a data type's domain (NORMA DataType.Compare)."""
    admitted = _range_pred(ranges[0])
    for vr in ranges[1:]:
        admitted = _comp(_OR, _cons(admitted, _range_pred(vr)))    # ⋃ ranges
    return Filter(_comp(_NOT, admitted, _sel(r)))            # keep facts no range admits


def Cardinality(lo, hi):
    """Population count in [lo, hi]; V = ⟨X⟩ (the population as one offending binding)
    when it is not, else φ. Wrapping in a singleton makes V_c ≠ φ even when the count
    is 0 — so a min-cardinality (lo≥1) rejects an empty population (Def. Violation)."""
    inrange = _comp(_AND, _cons(_comp(_GE, _cons(_LEN, _S(_CONST, Atom(lo)))),
                                _comp(_LE, _cons(_LEN, _S(_CONST, Atom(hi))))))
    return _S(_COND, inrange, _S(_CONST, PHI), _S(_CONS, _ID))   # in range → φ ; else → ⟨X⟩


# ring: over the projected ⟨r1, r2⟩ pairs (NORMA RingConstraintType — base types;
# conjunctive kinds like AcyclicIntransitive compose these).
_notmem_swap = _comp(_NULL, Filter(_EQ), _DISTL, _cons(_comp(_swap, _1), _2))     # swap(p) ∉ R
_mem_swap = _comp(_NOT, _NULL, Filter(_EQ), _DISTL, _cons(_comp(_swap, _1), _2))  # swap(p) ∈ R
_a_ne_b = _comp(_NOT, _EQ, _cons(_comp(_1, _1), _comp(_2, _1)))                   # first ≠ last on ⟨p,R⟩
_over = lambda pred: _comp(_S(_ALPHA, _1), Filter(pred), _DISTR, _cons(_ID, _ID))
_RING = {
    "Irreflexive": Filter(_comp(_EQ, _cons(_1, _2))),                    # some ⟨a,a⟩
    "Asymmetric": _over(_mem_swap),                                      # ⟨a,b⟩ with ⟨b,a⟩ present
    "Symmetric": _over(_notmem_swap),                                    # ⟨a,b⟩ with ⟨b,a⟩ absent
    "Antisymmetric": _over(_comp(_AND, _cons(_mem_swap, _a_ne_b))),      # ⟨a,b⟩,⟨b,a⟩ with a≠b
}

# closure-based ring types (NORMA): transitive / intransitive / acyclic / reflexive need
# the relation's composition R∘S = {⟨a,c⟩: ⟨a,b⟩∈R ∧ ⟨b,c⟩∈S} and its transitive closure —
# a fixpoint over the pair relation, on the same lfp engine as derive (Lemma finiteness).
_APNDL, _APNDR, _WH, _INS = Atom("apndl"), Atom("apndr"), Atom("WHILE"), Atom("INSERT")
_append_phi = _comp(_APNDR, _cons(_ID, _S(_CONST, PHI)))                # X → ⟨x1..xn, φ⟩ (fold seed)
_dedup = _comp(_S(_INS, _S(_COND, _member, _2, _APNDL)), _append_phi)   # remove duplicate rows (set)
_compose = _comp(Project([1, 3]), NatJoin(2))                           # ⟨R, S⟩ → R∘S
_self2 = _comp(_compose, _cons(_ID, _ID))                               # P → P∘P
# TC(P) = lfp of Q ↦ Q ∪ (P∘Q) from Q=P, threaded over ⟨P, Q⟩; the closed Q is the closure
_growQ = _cons(_1, _comp(_dedup, _CAT, _cons(_2, _compose)))            # ⟨P,Q⟩ → ⟨P, dedup(Q ++ P∘Q)⟩
_moreQ = _comp(_NOT, _NULL, setminus, _cons(_compose, _2))             # (P∘Q) ∖ Q ≠ φ ?
_TC = _comp(_2, _S(_WH, _moreQ, _growQ), _cons(_ID, _ID))              # TC:P = 2:(while grow):⟨P,P⟩
_diag = _comp(_S(_ALPHA, _1), Filter(_comp(_EQ, _cons(_1, _2))))       # P → the a with ⟨a,a⟩∈P
_domain = _comp(_dedup, _CAT, _cons(_S(_ALPHA, _1), _S(_ALPHA, _2)))   # P → the values occurring in P
_RING.update({
    "Reflexive": _comp(setminus, _cons(_domain, _diag)),               # a ∈ dom(P) lacking ⟨a,a⟩
    "PurelyReflexive": Filter(_comp(_NOT, _EQ, _cons(_1, _2))),         # ⟨a,b⟩ with a≠b
    "Transitive": _comp(setminus, _cons(_self2, _ID)),                 # (P∘P) ∖ P (a missing composite pair)
    "Intransitive": _comp(intersection, _cons(_ID, _self2)),           # P ∩ (P∘P)
    "StronglyIntransitive": _comp(intersection, _cons(_ID, _comp(_compose, _cons(_ID, _TC)))),   # P ∩ (P∘TC(P))
    "Acyclic": _comp(Filter(_comp(_EQ, _cons(_1, _2))), _TC),           # a self-loop of TC(P) = a cycle
})


def Ring(r1, r2, kind):
    """Ring constraint on roles r1,r2 (same object type); V = the offending pairs."""
    return _comp(_RING[kind], _S(_ALPHA, _cons(_sel(r1), _sel(r2))))     # kind ∘ α[r1,r2]


# ==================== role-centric constraint front (NORMA) ====================
# Every constraint is over a ConstraintRoleSequence of Role objects; it compiles
# to one of the θ₁ evaluators above and carries its modality (alethic → reject,
# deontic → warn). The positional forms above are the evaluators these bind to.
_positions = lambda role_seq: [role_position(r) + 1 for r in role_seq]
_INSERT = Atom("INSERT")
_union = _comp(_CAT, _S(_CONS, _1, _comp(setminus, _swap)))            # A ∪ B = A ++ (B∖A)
_symdiff = _comp(_CAT, _S(_CONS, setminus, _comp(setminus, _swap)))     # (A∖B) ∪ (B∖A)


def mandatory(role_seq, modality="alethic"):
    """Simple mandatory (one role) or disjunctive/inclusive-or (several roles): each
    instance of the object type must play at least one constrained role.
    V = O ∖ ⋃ᵢ (players of role i), with O the object-type population."""
    ot = role_player(role_seq[0])
    if len(role_seq) == 1:
        vc = Mandatory(role_position(role_seq[0]) + 1)
        arrange = _S(_CONS, _rel(ot), _rel(_ft_of(role_seq)))               # ⟨O, R_ft⟩
    else:
        vc = _comp(setminus, _S(_CONS, _1, _comp(_S(_INSERT, _union), _2)))  # O ∖ ⋃ᵢ pvᵢ  (n-ary ior)
        pvs = _S(_CONS, *[_comp(Project([role_position(r) + 1]), _rel(role_fact_type(r)))
                          for r in role_seq])
        arrange = _S(_CONS, _rel(ot), pvs)                                  # ⟨O, ⟨pv₁..pvₙ⟩⟩
    return _constraint("MandatoryConstraint", [role_seq], modality, vc, arrange)


def frequency(role_seq, lo, hi, modality="alethic"):
    return _constraint("FrequencyConstraint", [role_seq], modality,                # lo, hi in extra
                       Frequency(_positions(role_seq), lo, hi), _rel(_ft_of(role_seq)), Atom(lo), Atom(hi))


def ring(role_seq, kind, modality="alethic"):
    a, b = _positions(role_seq)
    return _constraint("RingConstraint", [role_seq], modality,                     # kind in extra
                       Ring(a, b, kind), _rel(_ft_of(role_seq)), Atom(kind))


def value_comparison(role_a, role_b, op, modality="alethic"):
    a, b = role_position(role_a) + 1, role_position(role_b) + 1
    return _constraint("ValueComparisonConstraint", [[role_a, role_b]], modality,   # op in extra
                       ValueComparison(a, b, op), _rel(role_fact_type(role_a)), Atom(op))


def _set_comparison(kind, op, seq_a, seq_b, modality):
    vc = _comp(op, _S(_CONS, _comp(Project(_positions(seq_a)), _1),
                             _comp(Project(_positions(seq_b)), _2)))    # op:⟨π_a(A), π_b(B)⟩
    arrange = _S(_CONS, _rel(_ft_of(seq_a)), _rel(_ft_of(seq_b)))       # ⟨A, B⟩
    return _constraint(kind, [seq_a, seq_b], modality, vc, arrange)


def subset(seq_a, seq_b, modality="alethic"):
    return _set_comparison("SubsetConstraint", setminus, seq_a, seq_b, modality)


def equality(seq_a, seq_b, modality="alethic"):
    return _set_comparison("EqualityConstraint", _symdiff, seq_a, seq_b, modality)


def exclusion(seq_a, seq_b, modality="alethic"):
    return _set_comparison("ExclusionConstraint", intersection, seq_a, seq_b, modality)


def value(role, spec, modality="alethic"):
    """A role value constraint (NORMA RoleValueConstraint): the role's values must
    lie in `spec` — a value set (enumeration, checked by membership) or a list of
    ValueRanges. It carries its ranges so it can verbalize as `{a, b, c}` / `[lo..hi]`
    and so the reading round-trips. Must be compatible with the role's data type."""
    r = role_position(role) + 1
    if isinstance(spec, Seq):                                # enumeration by membership
        ranges, vc = [value_range(a.v) for a in spec.xs], Value(r, spec)
    else:                                                    # explicit ValueRanges
        ranges, vc = list(spec), InRanges(r, spec)
    return _constraint("ValueConstraint", [[role]], modality, vc,
                       _rel(role_fact_type(role)), _S(*ranges))


def value_constraint_reading(constraint, player_name=None):
    """The reading of a value constraint (Halpin & Curland): `The possible values of
    X are {a, b, c}` / `[20..270]`. The ranges list is the constraint's last part."""
    role = c_roles(constraint).xs[0].xs[0]
    subject = player_name or role_player(role)
    items = [_range_text(vr) for vr in c_extra(constraint)[-1].xs]
    listing = items[0] if len(items) == 1 else "{" + ", ".join(items) + "}"   # bare single; braced set
    return "The possible values of {0} are {1}.".format(subject, listing)


def cardinality(role_seq, lo, hi, modality="alethic"):
    return _constraint("CardinalityConstraint", [role_seq], modality,               # lo, hi in extra
                       Cardinality(lo, hi), _rel(_ft_of(role_seq)), Atom(lo), Atom(hi))


# ========================= objectification & subtyping ========================
def objectification(nesting_type_name, nested_fact_type, implied=False):
    """Objectification (NORMA): an object type whose instances are the tuples of a
    nested fact type — NestingType ↔ NestedFactType. It can play roles, and its
    identity is a spanning uniqueness over the nested fact type's roles."""
    pid = uniqueness(list(ft_roles(nested_fact_type)))          # spanning ⇒ internal
    return _S(Atom("Objectification"), Atom(nesting_type_name), nested_fact_type,
              Atom("implied") if implied else Atom("explicit"), pid)


def subtype_fact(subtype, supertype, provides_preferred_identifier=True):
    """A subtype fact (NORMA SubtypeFact): subtype IsA supertype. Population:
    subtype ⊆ supertype; V = subtype instances not among supertype instances."""
    return _S(Atom("SubtypeFact"), Atom(subtype), Atom(supertype),
              Atom("primary") if provides_preferred_identifier else Atom("secondary"),
              setminus)                                          # V_c = subtype ∖ supertype


# ================================ role paths (NORMA) ==========================
# A RolePath navigates fact types via joins — NORMA PathedRolePurpose (StartRole,
# SameFactType, PostInnerJoin, PostOuterJoin), with a PathObjectUnifier unifying the
# joined object. A derivation rule is a role path projected onto a head (Def.
# Derive); an external constraint is a constraint over the path's projection. This
# builds the linear inner-join core (fact types sharing an object); outer joins,
# branches (RoleSubPath), calculated/aggregate values, and anaphora (excluded from
# the AREST fragment R) extend it.
def pathed_role(role, purpose):
    """A step in a role path (purpose ∈ StartRole/SameFactType/PostInnerJoin/PostOuterJoin)."""
    return _S(Atom("PathedRole"), role, Atom(purpose))


def role_path(pathed_roles):
    return _S(Atom("RolePath"), _S(*pathed_roles))


# evaluate a linear inner-join path over populations sharing an entity (role 1):
# ⟨FT₁,…,FTₙ⟩ → the joined relation.  /⋈₁ — insert natural join.
path_join = _S(Atom("INSERT"), NatJoin(1))


def derivation(head_positions):
    """A derivation rule: the joined role path projected onto the head roles
    (Def. Derive — a role path projected onto a head; path variables here,
    calculated/aggregate values and constants extend it)."""
    return _comp(Project(head_positions), path_join)
