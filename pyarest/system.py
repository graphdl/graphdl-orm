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


# --- the state machine read off M (whitepaper §1): a machine IS a set of facts ---
# smFrom ⟨t, from⟩ ⋈ smTrigger ⟨t, trigger⟩ ⋈ smTo ⟨t, to⟩, projected to ⟨from, trigger, to⟩ —
# assembling the machine is a theta1 join over M's cells, not a second interpreter.
def _sm_join():
    j1 = _S(_COMP, T.NatJoin(1), _S(_CONS, _1, _2))          # ⟨t, from, trigger⟩
    j2 = _S(_COMP, T.NatJoin(1), _S(_CONS, j1, A(3)))        # ⟨t, from, trigger, to⟩
    return _S(_COMP, T.Project([2, 3, 4]), j2)


def sm_triples(D):
    """The machine's ⟨from, trigger, to⟩ triples, joined from M's smFrom/smTrigger/smTo
    cells by the theta1 expression above (host code only fetches and converts)."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(_sm_join(), pops)))


def machine_wiring(D, trigger_ft, noun):
    """Read `trigger_ft`'s wiring off M: the (from, to) pairs of the transitions it fires,
    and the addressed noun's role position in the trigger fact type (from M's role facts)."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pairs = tuple((f, t) for (f, tr, t) in sm_triples(D) if tr == trigger_ft)
    roles = from_lam(_ap(ast.FetchPop("role"), D))
    pos = next((p for (_rid, ft, p, otype) in roles if ft == trigger_ft and otype == noun), 1)
    return pairs, pos


# --- Phase 4 opening: RMAP read off M, driving D's layout (spec §4.4; §Cells) ---
def _pop_rows(D, name):
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    rows = from_lam(_ap(ast.FetchPop(name), D))
    return list(rows) if isinstance(rows, tuple) else []


def rmap_partition(D):
    """M-facts → the cell partition {fact type: table key}, by the RMAP grouping rules run
    as the one machine fold. The kind of each fact type is READ OFF M's constraint facts:
    a spanning UC (or no UC at all, the m:n case) gives the fact type its own table (rule 1);
    a single-role UC makes it functional, absorbed into its role-1 player's table (rule 2)."""
    from .machine import rmap as rmap_value, run_machine
    from .lam import to_lam, from_lam
    fts = [f[0] for f in reversed(_pop_rows(D, "factType"))]          # declaration order
    cons = _pop_rows(D, "constraint")
    spanning = {c[2] for c in cons if len(c) >= 3 and c[1] == "spanning_uniqueness"}
    functional = {c[2] for c in cons if len(c) >= 3 and c[1] == "uniqueness"}
    subject = {r[1]: r[3] for r in reversed(_pop_rows(D, "role")) if r[2] == 1}
    triples = tuple((ft, subject.get(ft, ft),
                     "functional" if (ft in functional and ft not in spanning) else "spanning")
                    for ft in fts)
    pairs = from_lam(run_machine(rmap_value, to_lam(()), to_lam(triples)))
    return {ft: key for (key, ft) in pairs}


def absorb_rows(D, table_key, partition):
    """The 3NF row population of one RMAP table: the θ₁ natural join on the key (role 1)
    of the fact types absorbed into `table_key` (spec §4.4: functional roles on the same
    object type give one cell keyed on its id). Entities missing a functional fact drop
    from the joined rows; the optional-column (outer join) refinement is a later step."""
    from . import theta as T
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    import pyarest.lam as L
    fts = [ft for ft, key in partition.items() if key == table_key and ft != table_key]
    if not fts:
        return []
    acc = to_lam(tuple(tuple(r) for r in _pop_rows(D, fts[0])))
    for ft in fts[1:]:
        nxt = to_lam(tuple(tuple(r) for r in _pop_rows(D, ft)))
        acc = _ap(T.NatJoin(1), L.SEQ(L.CONS(acc)(L.CONS(nxt)(L.NIL))))
    return list(from_lam(acc))


# The cell-naming boundary op, as the reference TS engine computes it in the worker
# (12-physical-mapping.md: cellKey('Order','org-1') gives 'Order:org-1'). Strings are
# outside the algebra, so joining a name is a registered value op (spec D5).
def _cellkey_impl(mu):
    from . import defs as _d
    import pyarest.lam as L

    def g(o):
        it = _d._items(L._list(o))
        if len(it) != 2:
            return L.BOT
        a, b = _d._aval(it[0]), _d._aval(it[1])
        if a is None or b is None or isinstance(a, tuple) or isinstance(b, tuple):
            return L.BOT
        return L.atom(f"{a}:{b}")
    return g


def _register_cellkey():
    from .defs import register
    register("cellkey", _cellkey_impl)


_register_cellkey()


def table_columns(partition, table):
    """The fact types absorbed into `table`, in declaration order; column j of the 3NF row
    ⟨key, v1, v2, …⟩ holds the (1+j)th entry's value."""
    return [ft for ft, key in partition.items() if key == table and ft != table]


def row_resolve(col, width):
    """resolve for an entity-cell write: ⟨I, row⟩ → row′, where I = ⟨key, value⟩ and the
    cell holds the entity's 3NF row (a fresh entity gets holes, the default object #). A
    conflicting functional write makes the column ⊥, the row collapses (§11.2.1), and the
    step's transition rule refuses it atomically: absorption makes the UC structural."""
    key, val = _S(_COMP, _1, _1), _S(_COMP, _2, _1)
    hole = _S(_CONST, A("#"))
    bot = _S(_COMP, _1, _S(_CONST, A("x")))                  # a selector on an atom is ⊥
    fresh = _S(_CONS, key, *[val if j == col else hole for j in range(2, width + 1)])
    old = _S(_COMP, A(col), _2)
    ok = _S(_COMP, A("or"), _S(_CONS,
             _S(_COMP, _EQ, _S(_CONS, old, hole)),
             _S(_COMP, _EQ, _S(_CONS, old, val))))
    upd = _S(_CONS, _S(_COMP, A(1), _2),
             *[_S(_COND, ok, val, bot) if j == col else _S(_COMP, A(j), _2)
               for j in range(2, width + 1)])
    return _S(_COND, _S(_COMP, A("null"), _2), fresh, upd)


def create_routed(D, ft, fact, partition):
    """Route a create through the RMAP partition (spec §4.4: the partition IS the layout).
    An absorbed fact type writes the entity's own cell `table:key`, the write unit of
    Def. iso, updating its column of the row; an own-table fact type creates into its
    per-fact-type cell unchanged. After a commit the table's index cell (named by the
    noun, the RegistryDB analog of the TS engine) records the key."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    from . import defs as _d
    import pyarest.lam as L
    table = partition.get(ft, ft)
    if table == ft:
        return ast.run(fact, D, cell_name=ft)
    cols = table_columns(partition, table)
    col = 2 + cols.index(ft)
    key = from_lam(fact)[0]
    res = ast.run(fact, D, cell_name=f"{table}:{key}",
                  resolve_obj=row_resolve(col, 1 + len(cols)))
    o, D2 = _d._items(L._list(res))
    if from_lam(o) != "ERROR":
        idx = from_lam(_ap(ast.FetchPop(table), D2))
        if (key,) not in idx:
            newidx = _ap(A("apndl"), _S(to_lam((key,)), to_lam(tuple(idx))))
            D2 = _ap(ast.Store(table), _S(newidx, D2))
    return _S(o, D2)


def ft_view(D, ft, partition):
    """Reassemble an absorbed fact type's ⟨key, value⟩ population from the entity cells:
    a θ₁ expression over ⟨index, D⟩ pairing each key with its row's column through the
    dynamic fetch and the cellkey op, then dropping the # holes. An own-table fact type
    reads its own cell."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    from . import theta as T
    table = partition.get(ft, ft)
    if table == ft:
        return set(from_lam(_ap(ast.FetchPop(ft), D)))
    col = 2 + table_columns(partition, table).index(ft)
    key = _S(_COMP, _1, _1)                                  # of ⟨⟨key⟩, D⟩: the key scalar
    name = _S(_COMP, A("cellkey"), _S(_CONS, _S(_CONST, A(table)), key))
    row = _S(_COMP, ast.DynFetch(), _S(_CONS, name, _2))
    pair = _S(_CONS, key, _S(_COMP, A(col), row))
    nonhole = _S(_COMP, A("not"), _EQ, _S(_CONS, A(2), _S(_CONST, A("#"))))
    expr = _S(_COMP, T.Filter(nonhole), _S(_ALPHA, pair), _DISTR)
    idx = _ap(ast.FetchPop(table), D)
    return set(from_lam(_ap(expr, _S(idx, D))))


def install_entity_cells(D, noun, rows):
    """Each entity its own cell (whitepaper §Cells; the TS engine's one-DO-per-cell):
    ⟨CELL, noun:id, row⟩ per 3NF row, addressed as the reference engine addresses it and
    the write unit Def. iso isolates."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    import pyarest.lam as L
    for row in rows:
        key = f"{noun}:{row[0]}"
        D = _ap(ast.Store(key), L.SEQ(L.CONS(to_lam(tuple(row)))(L.CONS(D)(L.NIL))))
    return D


def sm_step(pairs, entity_role):
    """The live machine step (Prop. onestep) as one FFP object: ⟨statusPop, P″⟩ → statusPop′.
    An entity advances iff a trigger fact naming it (at `entity_role`) entered P″ and a
    transition leaves its current status; everything else is unchanged. `pairs` are this
    trigger fact type's (from, to) pairs, quoted as data."""
    from .lam import to_lam
    trs = _S(_CONST, to_lam(tuple(pairs)))
    e = _S(_COMP, _1, _1)                                    # of ⟨⟨e,s⟩, P″⟩
    s = _S(_COMP, _2, _1)
    fact_names_e = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, A(entity_role), _1), _2))   # ⟨fact,e⟩
    fired = _S(_COMP, A("not"), A("null"), T.Filter(fact_names_e), _DISTR, _S(_CONS, _2, e))
    from_is_s = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, _1, _1), _2))                  # ⟨⟨f,t⟩,s⟩
    nexts = _S(_COMP, _S(_ALPHA, _1), T.Filter(from_is_s), _DISTR, _S(_CONS, trs, s))
    hasnext = _S(_COMP, A("not"), A("null"), nexts)
    to = _S(_COMP, A(2), _1, nexts)                          # the first matching pair's to
    both = _S(_COMP, A("and"), _S(_CONS, fired, hasnext))
    upd = _S(_COND, both, _S(_CONS, e, to), _1)              # ⟨e,to⟩, else the old ⟨e,s⟩
    return _S(_COMP, _S(_ALPHA, upd), _DISTR)                # over each ⟨⟨e,s⟩, P″⟩
