"""The AREST command pipeline as FFP objects (Def. Command, eq. create), on the kernel.

    create = emit ∘ validate ∘ derive ∘ resolve                         (eq. create)

Each stage is an FFP object reduced by the one mu; nothing is host code. `derive` is the
bounded least fixed point of the immediate-consequence operator (Def. Derive): iterate
F_S from the delta until nothing new is derived, the fixpoint test being set-theoretic —
(F_S:P) ∖ P = φ — since F_S is monotone over a finite fact space (Knaster-Tarski / Lemma
finiteness). Given only the frontier's affected rules (meta.affected_rules), the lfp runs
over the affected fragment, not the whole population (Cor. streaming). `validate` unions
the per-constraint violation sets (rho c):P and flags an alethic offender so the AST step
can refuse to commit (Def. Violation).
"""
from . import lam as L
from .lam import atom as A, PHI
from .defs import define
from . import theta as T

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS, _CONST, _COND = A("COMP"), A("CONS"), A("CONST"), A("COND")
_ALPHA, _INSERT, _WHILE = A("ALPHA"), A("INSERT"), A("WHILE")
_ID, _1, _2, _APNDL = A("id"), A(1), A(2), A("apndl")
_EQ, _DISTR, _CAT = A("eq"), A("distr"), A("cat")

# --- default (minimal) stages, as compiled defs ---
define("resolve", _APNDL)                                    # ⟨I,P⟩ → ⟨I, …P⟩  (add the input fact)
define("derive", _ID)                                        # lfp with no rules = id (0 steps)
define("validate", _S(_CONS, _ID, _S(_CONST, PHI)))         # P → ⟨P, φ⟩  (no constraints)
define("emit", _1)                                           # ⟨P,V⟩ → P
define("create", _S(_COMP, A("emit"), A("validate"), A("derive"), A("resolve")))  # eq. create


# --- validate: V = ⋃_c (rho c):P, with the alethic commit guard ---
def _violations(exprs):
    """X ↦ ⋃_c (rho c):X — flatten the per-constraint violation sets ⟨V_c1..V_cn⟩."""
    if not exprs:
        return _S(_CONST, PHI)
    return _S(_COMP, T.flatten, _S(_CONS, *exprs))


def validate_of(constraints, alethic=None, scoped=(), scoped_alethic=None):
    """validate_S : ⟨P, D⟩ ↦ ⟨P, V, alethicViolated⟩ (Def. Command / Violation). `constraints`
    consume the target population P (cell-local — composed with the selector); `scoped` consume
    ⟨P, D⟩ whole (cross-cell — they fetch sibling cells from the frozen D). `alethic` /
    `scoped_alethic` are the commit-blocking subsets (default: all of each)."""
    local = [_S(_COMP, c, _1) for c in constraints]
    la = local if alethic is None else [_S(_COMP, c, _1) for c in alethic]
    sc = list(scoped)
    sa = sc if scoped_alethic is None else list(scoped_alethic)
    flag = _S(_COMP, A("not"), A("null"), _violations(la + sa))   # any alethic offender?
    return _S(_CONS, _1, _violations(local + sc), flag)


def validate_modal(pairs, scoped_pairs=()):
    """validate over constraints tagged with modality: pairs = [(obj, modality)] cell-local,
    scoped_pairs likewise for ⟨P,D⟩ consumers. V is the union of ALL violations, but only the
    ALETHIC ones set the block-commit flag (AREST Def. Violation / eq. create). A deontic
    violation is reported in V yet never blocks commit — 'ought to be obeyed but may be
    violated' (the constraint verbalization paper's deontic o)."""
    return validate_of([o for o, _m in pairs],
                       alethic=[o for o, m in pairs if m == "alethic"],
                       scoped=[o for o, _m in scoped_pairs],
                       scoped_alethic=[o for o, m in scoped_pairs if m == "alethic"])


# --- derive = lfp(F_S): the immediate-consequence operator iterated to a fixed point ---
def F_of(rules):
    """One round: F_S(P) = P ∪ ⋃_rules rule(P), with set semantics (dedup ∘ flatten ∘ [id, rules])."""
    if not rules:
        return _ID
    body = _S(_CONS, _ID, *rules)                            # ⟨P, rule1(P), …, rulen(P)⟩
    return _S(_COMP, T.dedup, T.flatten, body)               # dedup(P ++ the derived heads)


def derive_of(rules):
    """derive_S = lfp(F_S, ·): Backus `while` iterating F_S until (F_S:P) ∖ P = φ. Pass only the
    frontier's affected rules to keep the lfp bounded to the touched fragment (Cor. streaming)."""
    if not rules:
        return _ID
    F = F_of(rules)
    new = _S(_COMP, T.setminus, _S(_CONS, F, _ID))          # (F:P) ∖ P — the newly derived facts
    grows = _S(_COMP, A("not"), A("null"), new)              # still deriving something?
    return _S(_WHILE, grows, F)


# --- role-path -> F_S: a derivation rule is a Datalog rule q(..) <- p1(..), .., pm(..) (ORM ->
# Datalog): a conjunctive query whose body atoms join on shared variables and project to the head
# (the role path). A recursive head (ancestor <- link ; link, ancestor) is resolved by derive_of's
# least fixed point. Each compiled rule is an FFP object P -> its derived head facts. ---
def join_rule(join_role, head_cols):
    """A two-atom SELF-referential role-path rule over one fact type: join the fact type to itself
    on `join_role` (R.join_role = R'.1) and project to `head_cols`. This is the recursive body,
    e.g. ancestor(x,z) <- link(x,y), ancestor(y,z) with head_cols=[1,3], join_role=2 — feed it to
    derive_of for the least fixed point (transitive closure). rule:P = Project ∘ NatJoin ∘ [id,id]."""
    return _S(_COMP, T.Project(head_cols), T.NatJoin(join_role), _S(_CONS, _ID, _ID))


def join_rule2(join_role, head_cols):
    """A two-atom role-path rule over two fact types: input ⟨A, B⟩; join A.join_role = B.1 and
    project to head_cols over the combined tuple (e.g. FastCarDriver(x) <- drives(x,y), isFast(y)).
    rule:⟨A,B⟩ = Project ∘ NatJoin."""
    return _S(_COMP, T.Project(head_cols), T.NatJoin(join_role))


# the storage half of a NORMA */**/+/++ marker: whether the derived facts are materialized (stored)
# vs recomputed on demand. (* and + recompute; ** and ++ store.) The derivation half is the rule
# above, fed to derive_of; the create pipeline runs it as the `derive` stage over the fact's cell.
_MATERIALIZE = {"fully-derived": False, "derived-and-stored": True,
                "semi-derived": False, "partially-derived-and-stored": True}


def materialize(marker):
    """True if the marker means store the derived facts (** / ++), False if compute on demand (* / +)."""
    return _MATERIALIZE.get(marker, False)


# --- resolve with auto-counter minting (Def. Command: mint iff the ref scheme auto-generates) ---
_max2 = _S(_COND, _S(_COMP, A("ge"), _S(_CONS, _1, _2)), _1, _2)   # the larger of a pair


def mint_next(col):
    """P ↦ 1 + the greatest value in column `col` of P (or 1 if empty): the auto-counter's
    next id — successor of a max-fold over the id column (one surrogate per guarded step)."""
    biggest = _S(_COMP, _S(_INSERT, _max2), _S(_ALPHA, A(col)))
    succ = _S(_COMP, A("+"), _S(_CONS, biggest, _S(_CONST, A(1))))
    return _S(_COND, A("null"), _S(_CONST, A(1)), succ)      # empty → 1 ; else max+1


def resolve_minting(col):
    """resolve for an auto-generating entity: mint the next id and prepend ⟨id, …I⟩ to P."""
    minted = _S(_COMP, mint_next(col), _2)                   # ⟨I,P⟩ → the fresh id (from P)
    fact = _S(_COMP, _APNDL, _S(_CONS, minted, _1))          # ⟨id, …I⟩
    return _S(_COMP, _APNDL, _S(_CONS, fact, _2))            # ⟨fact, …P⟩


# --- emit: HATEOAS — the representation carries its own links (Thm. hateoas) ---
# links(e) = nav(e) ∪ transitions(status(e)): the related resources plus the actions available
# from the entity's current state. Both are theta1 selections — nav over P, transitions over a
# state machine value; the representation is self-describing, no link table maintained.
def nav_of(key_pos):
    """nav(e): the facts of P sharing the affected entity's key (role `key_pos` of the head fact).
        α(1) ∘ Filter(key(f) = headKey) ∘ distr ∘ [id, key∘1]"""
    key = A(key_pos)
    keyed = _S(_CONS, _ID, _S(_COMP, key, _1))               # ⟨P, key(head)⟩
    match = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, key, _1), _2))  # key(f) = headKey?
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(match), _DISTR, keyed)


def transitions_of(sm, status_pos):
    """transitions(status(e)): the state-machine transitions available from the head fact's
    status. `sm` is a value ⟨⟨from, trigger, to⟩…⟩; a transition fires when from = status(head).
        α(1) ∘ Filter(from(t) = status) ∘ distr ∘ [sm̄, status∘1]"""
    keyed = _S(_CONS, _S(_CONST, sm), _S(_COMP, A(status_pos), _1))   # ⟨sm, status(head)⟩
    match = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, _1, _1), _2))          # from(t) = status?
    return _S(_COMP, _S(_ALPHA, _1), T.Filter(match), _DISTR, keyed)


def links_of(key_pos, sm=None, status_pos=None):
    """links(e) = nav(e) ∪ transitions(status(e))  (Thm. hateoas). Without a state machine,
    the links are just the navigation."""
    nav = nav_of(key_pos)
    if sm is None:
        return nav
    return _S(_COMP, _CAT, _S(_CONS, nav, transitions_of(sm, status_pos)))  # nav ∪ transitions
