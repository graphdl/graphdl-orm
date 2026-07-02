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


# --- S1: membership is application, as a construction (Def. pop / eq. (1)) ---
def typed_fact(ft, values):
    """A fact carrying its type as its first element (the paper's ⟨CONS, s₁…sₙ⟩ shape),
    so the fact 'is resolved by looking up its type' (eq. 1)."""
    from .lam import to_lam
    return to_lam((ft,) + tuple(values))


def typed_population(ft, rows):
    """A population of typed facts: applying it metacomposes down to the TYPE's
    definition, which computes membership — P:g = T iff g ∈ P, one act."""
    from .lam import to_lam
    return to_lam(tuple((ft,) + tuple(r) for r in rows))


def membership_def():
    """The fact type's definition as an operator: metacomposition hands it ⟨f₁, ⟨P, g⟩⟩
    (eq. 1 twice: once on the population, once on the fact), and member:⟨g, P⟩ answers.
    Define this under the fact type's name and the population IS its characteristic
    function, with no new mechanism."""
    g = _S(_COMP, _2, _2)
    P = _S(_COMP, _1, _2)
    return _S(_COMP, T.member, _S(_CONS, g, P))


# --- the book's rule form compiled (Halpin ch.2 ex.4): linear chain join over D ---
def compile_rule(atom_fts, head_positions):
    """A rule's body as one FFP object over D: the populations of the clause fact types,
    each fetched from its own cell, joined linearly (each next atom joins the running
    tuple's last column to its column 1), with the head's variable positions projected.
    Cross-cell by construction: this is store-on-derive's read side."""
    from . import ast
    expr = ast.FetchPop(atom_fts[0])
    width = 2
    for ftn in atom_fts[1:]:
        expr = _S(_COMP, T.NatJoin(width), _S(_CONS, expr, ast.FetchPop(ftn)))
        width += 1
    return _S(_COMP, T.Project(head_positions), expr)


def compile_rule_delta(atom_fts, head_positions, delta_at):
    """The rule body with atom `delta_at` (0-based) reading the round's DELTA instead of
    its cell: an FFP object over ⟨Δ, D⟩ — semi-naive evaluation's inner join
    (Bancilhon–Ramakrishnan 1986). Same join shape as compile_rule; only the substituted
    atom differs."""
    from . import ast

    def pop(j, ftn):
        return _1 if j == delta_at else _S(_COMP, ast.FetchPop(ftn), _2)

    expr = pop(0, atom_fts[0])
    width = 2
    for j, ftn in enumerate(atom_fts[1:], start=1):
        expr = _S(_COMP, T.NatJoin(width), _S(_CONS, expr, pop(j, ftn)))
        width += 1
    return _S(_COMP, T.Project(head_positions), expr)


def run_rules(D, changed=None, stats=None):
    """Cross-cell derivation to the least fixed point, semi-naive (Bancilhon–
    Ramakrishnan 1986): round one evaluates full bodies, BOUNDED by the frontier
    (Cor. streaming — with `changed` given, only rules whose ruleReads intersect fire);
    every later round joins only each head's per-round delta through the stored ~d
    variants (one per atom position, from M's ruleAtom facts), so the join input shrinks
    as the fixpoint nears. Sound and complete because rules are positive and monotone
    and every genuinely new tuple uses at least one new row. Rules without atom facts
    fall back to full evaluation when their reads changed. Rule names resolve through
    D's own DEFS (ρ within the step); Knaster–Tarski gives the lfp and Lemma finiteness
    bounds the rounds. `stats`, when a list, collects per-evaluation records."""
    from . import ast, defs
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as _A
    reads, atomsof = {}, {}
    for r in _pop_rows(D, "ruleReads"):
        if len(r) >= 2:
            reads.setdefault(r[0], set()).add(r[1])
    for r in _pop_rows(D, "ruleAtom"):
        if len(r) >= 3:
            atomsof.setdefault(r[0], []).append((r[1], r[2]))
    rules = [(r[0], r[1]) for r in _pop_rows(D, "ruleDerives")]
    frontier = None if changed is None else set(changed)
    delta, rnd = None, 0
    while True:
        rnd += 1
        fired, next_delta = False, {}
        for rule_cid, head in rules:
            if delta is None:                                # round one: full bodies
                if frontier is not None and not (reads.get(rule_cid, set()) & frontier):
                    continue
                with defs.step(D):
                    outs = from_lam(_ap(_A(rule_cid), D))
                if not isinstance(outs, tuple):
                    continue                                 # rule not compiled (M-facts only)
                new_rows = {tuple(r) for r in outs if isinstance(r, tuple)}
                if stats is not None:
                    stats.append({"round": rnd, "rule": rule_cid, "mode": "full"})
            else:
                hits = [(p, ft) for (p, ft) in atomsof.get(rule_cid, ()) if ft in delta]
                if hits:
                    new_rows = set()
                    for (p, ft) in hits:
                        drows = tuple(sorted(delta[ft]))
                        with defs.step(D):
                            o = from_lam(_ap(_A(f"{rule_cid}~d{p}"), _S(to_lam(drows), D)))
                        if isinstance(o, tuple):
                            new_rows |= {tuple(r) for r in o if isinstance(r, tuple)}
                        if stats is not None:
                            stats.append({"round": rnd, "rule": rule_cid, "mode": "delta",
                                          "pos": p, "in": len(drows),
                                          "base": len(_pop_rows(D, ft))})
                elif rule_cid not in atomsof and (reads.get(rule_cid, set()) & set(delta)):
                    with defs.step(D):                       # legacy rule: full fallback
                        outs = from_lam(_ap(_A(rule_cid), D))
                    if not isinstance(outs, tuple):
                        continue
                    new_rows = {tuple(r) for r in outs if isinstance(r, tuple)}
                    if stats is not None:
                        stats.append({"round": rnd, "rule": rule_cid, "mode": "full"})
                else:
                    continue
            old = {tuple(r) for r in _pop_rows(D, head)}
            add = new_rows - old
            if add:
                D = _ap(ast.Store(head), _S(to_lam(tuple(sorted(old | add))), D))
                fired = True
                next_delta.setdefault(head, set()).update(add)
        if not fired:
            return D
        delta = next_delta


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


def _sm_join_named():
    j1 = _S(_COMP, T.NatJoin(1), _S(_CONS, _1, _2))          # ⟨t, from, trigger⟩
    return _S(_COMP, T.NatJoin(1), _S(_CONS, j1, A(3)))      # ⟨t, from, trigger, to⟩


def sm_triples_named(D):
    """⟨transition, from, trigger, to⟩ — the named form, so per-transition facts
    (guards, Mealy emissions) can key in."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(_sm_join_named(), pops)))


def machine_for(D, noun):
    """The State Machine Definition governing `noun`: bound directly, or through the
    subtype chain to a bound supertype (a machine tied to a resource SUPERTYPE governs
    its subtypes' instances). None when no machine binds."""
    bound = {r[1]: r[0] for r in _pop_rows(D, "smDef")}
    subs = {r[0]: r[1] for r in _pop_rows(D, "subtype")}
    n, seen = noun, set()
    while n not in bound and n in subs and n not in seen:
        seen.add(n)
        n = subs[n]
    return bound.get(n)


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
    subs = {r[0]: r[1] for r in _pop_rows(D, "subtype")}

    def _top(o):                                             # RMAP step 0 (§10.3 verbatim):
        seen = set()                                         # "Absorb subtypes into their
        while o in subs and o not in seen:                   # top supertype"
            seen.add(o)
            o = subs[o]
        return o

    subject = {r[1]: _top(r[3]) for r in reversed(_pop_rows(D, "role")) if r[2] == 1}
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


def create_routed(D, ft, fact, partition, machine=None, mealy_obj=None):
    """Route a create through the RMAP partition (spec §4.4: the partition IS the layout).
    An absorbed fact type writes the entity's own cell `table:key`, the write unit of
    Def. iso, updating its column of the row; an own-table fact type creates into its
    per-fact-type cell unchanged. After a commit the table's index cell (named by the
    noun, the RegistryDB analog of the TS engine) records the key. A machine (the row
    form: machine_step(ft, row_col=...)) advances within the routed step."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    from . import defs as _d
    import pyarest.lam as L
    table = partition.get(ft, ft)
    if table == ft:
        return ast.run(fact, D, cell_name=ft, machine=machine, mealy_obj=mealy_obj)
    cols = table_columns(partition, table)
    col = 2 + cols.index(ft)
    key = from_lam(fact)[0]
    res = ast.run(fact, D, cell_name=f"{table}:{key}",
                  resolve_obj=row_resolve(col, 1 + len(cols)),
                  machine=machine, mealy_obj=mealy_obj)
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


def moore_view(D, noun):
    """The Moore output function as a view: for each live instance whose status carries an
    emission, the ρ-application of the named definition to ⟨entity, status⟩ (outputs are
    ρ-applications; the definition resolves through D's own DEFS)."""
    from . import defs as _d
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam, atom as _A
    moore = {r[0]: r[1] for r in _pop_rows(D, "smMoore")}
    out = {}
    for row in _pop_rows(D, f"{noun}_status"):
        e, s = row[0], row[1]
        if s in moore:
            with _d.step(D):
                out[(e, s)] = from_lam(_ap(_A(moore[s]), to_lam((e, s))))
    return out


def process_table(D, noun):
    """The run queue as a VIEW (nothing is managed host-side): each state-machine
    instance whose status has outgoing transitions is a WAITING process, keyed to the
    trigger fact types it awaits (a subscription is a ρ-application not yet evaluated,
    Cor. stream); an instance whose status has none has terminated and leaves the table
    (links = φ, the paper's logical deletion)."""
    triples = sm_triples(D)
    out = {}
    for row in _pop_rows(D, f"{noun}_status"):
        e, s = row[0], row[1]
        awaits = tuple(tr for (f, tr, _t) in triples if f == s)
        if awaits:
            out[(e, s)] = awaits
    return out


def _machine_exprs(trigger_ft, row_col=None):
    """The shared in-step expressions of the M-driven machine, over ⟨statusPop, P″, D⟩:
    the transition rows ⟨from, to, guard-or-#, emit-or-#⟩ joined from M in-reduction,
    the governed player's role position, and the per-entity pieces (fired, guard
    verdict, target) over pairs ⟨⟨e,s⟩, ⟨P″, trs, pos, D⟩⟩. machine_step and mealy_step
    are two projections of this one machine. With `row_col`, P″ is an ABSORBED entity's
    3NF row instead of a fact population: fired means the trigger's column went non-hole
    and the addressed entity is the row's key (or the column's value when the governed
    noun plays role 2 — the position still read from M in-step)."""
    from . import ast
    trig = _S(_CONST, A(trigger_ft))
    hash_ = _S(_CONST, A("#"))
    popD = lambda name: _S(_COMP, ast.FetchPop(name), A(3))
    # the named transitions of THIS trigger, joined from M in-step
    trs4 = _S(_COMP, _sm_join_named(), _S(_CONS, popD("smFrom"), popD("smTrigger"), popD("smTo")))
    mine = _S(_COMP, T.Filter(_S(_COMP, _EQ, _S(_CONS, A(3), trig))), trs4)

    # left-join guards and Mealy emits by transition name; row context ⟨row, ⟨gp, ep⟩⟩
    def _named(pop_sel):
        hits = _S(_COMP, T.Filter(_S(_COMP, _EQ, _S(_CONS, _S(_COMP, _1, _1), _2))),
                  _DISTR, _S(_CONS, pop_sel, _S(_COMP, _1, _1)))
        return _S(_COND, _S(_COMP, A("null"), hits), hash_, _S(_COMP, A(2), _1, _1, hits))

    row_ftge = _S(_CONS, _S(_COMP, A(2), _1), _S(_COMP, A(4), _1),
                  _named(_S(_COMP, _1, _2)), _named(_S(_COMP, _2, _2)))
    trsGE = _S(_COMP, _S(_ALPHA, row_ftge), _DISTR,
               _S(_CONS, mine, _S(_CONS, popD("smGuard"), popD("smEmit"))))
    # the governed nouns: smDef bindings plus the derived closure (supertype governance)
    govnouns = _S(_COMP, _CAT, _S(_CONS, _S(_COMP, _S(_ALPHA, A(2)), popD("smDef")),
                                  _S(_COMP, _S(_ALPHA, A(1)), popD("governedBy"))))
    rmatch = _S(_COMP, A("and"), _S(_CONS,
                _S(_COMP, _EQ, _S(_CONS, _S(_COMP, A(2), _1), trig)),
                _S(_COMP, T.member, _S(_CONS, _S(_COMP, A(4), _1), _2))))
    posrows = _S(_COMP, _S(_ALPHA, _1), T.Filter(rmatch), _DISTR,
                 _S(_CONS, popD("role"), govnouns))
    pos = _S(_COMP, A(3), _1, posrows)                       # the governed player's position
    # per-entity pieces, with the read-once context ⟨P″, trs, pos, D⟩ riding along
    ctx = _S(_CONS, _2, trsGE, pos, A(3))
    e = _S(_COMP, _1, _1)
    s = _S(_COMP, _2, _1)
    P = _S(_COMP, _1, _2)
    trs = _S(_COMP, _2, _2)
    posv = _S(_COMP, A(3), _2)
    Dd = _S(_COMP, A(4), _2)
    if row_col is None:
        fmatch = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, A("apply"), _S(_CONS, _S(_COMP, _2, _2), _1)),
                                   _S(_COMP, _1, _2)))       # fact[pos] = e, dynamically
        fired = _S(_COMP, A("not"), A("null"), T.Filter(fmatch), _DISTR,
                   _S(_CONS, P, _S(_CONS, e, posv)))
    else:
        e_addr = _S(_COND, _S(_COMP, _EQ, _S(_CONS, posv, _S(_CONST, A(1)))),
                    _S(_COMP, A(1), P), _S(_COMP, A(row_col), P))
        nonhole = _S(_COMP, A("not"), _EQ, _S(_CONS, _S(_COMP, A(row_col), P), hash_))
        fired = _S(_COMP, A("and"), _S(_CONS,
                   _S(_COMP, _EQ, _S(_CONS, e, e_addr)), nonhole))
    from_is_s = _S(_COMP, _EQ, _S(_CONS, _S(_COMP, _1, _1), _2))
    nexts = _S(_COMP, _S(_ALPHA, _1), T.Filter(from_is_s), _DISTR, _S(_CONS, trs, s))
    first = _S(_COMP, _1, nexts)
    to = _S(_COMP, A(2), first)
    gname = _S(_COMP, A(3), first)
    em = _S(_COMP, A(4), first)
    gpop = _S(_COMP, ast.DynFetch(), _S(_CONS, gname, Dd))
    unguarded = _S(_COMP, _EQ, _S(_CONS, gname, hash_))
    satisfied = _S(_COMP, T.member, _S(_CONS, e, _S(_COMP, _S(_ALPHA, A(1)), gpop)))
    okg = _S(_COND, unguarded, _S(_CONST, A("T")),
             _S(_COND, _S(_COMP, A("atom"), gpop), _S(_CONST, A("F")), satisfied))
    hasnext = _S(_COMP, A("not"), A("null"), nexts)
    both = _S(_COMP, A("and"), _S(_CONS, fired, hasnext))
    return dict(ctx=ctx, e=e, s=s, to=to, em=em, okg=okg, both=both, hash_=hash_, Dd=Dd)


def machine_step(trigger_ft, row_col=None):
    """The machine that runs IS the M-facts: one FFP object over ⟨statusPop, P″, D⟩ that
    reads the transitions (smFrom ⋈ smTrigger ⋈ smTo), the guards (smGuard), and the
    addressed entity's role position (role facts joined with the governed nouns, smDef
    plus the derived governedBy closure) from D INSIDE the reduction, then advances each
    entity whose trigger fact entered P″ with its guard satisfied. Numbers are selectors,
    so the runtime role position selects dynamically via the apply primitive. Editing M
    redirects this step with no rewiring; `trigger_ft` is the handler's compile-time
    identity, exactly as cell_name is for build_system. `row_col` selects the absorbed
    (3NF-row) fired form."""
    x = _machine_exprs(trigger_ft, row_col)
    upd = _S(_COND, x["both"], _S(_COND, x["okg"], _S(_CONS, x["e"], x["to"]), _1), _1)
    return _S(_COMP, _S(_ALPHA, upd), _DISTR, _S(_CONS, _1, x["ctx"]))


def mealy_step(trigger_ft, row_col=None):
    """Mealy output on the SAME step: for each entity whose transition fires, the
    transition's named definition (smEmit, read from M in-step like everything else) is
    resolved by ρ from D's own cells (definitions are ordinary cells, §13.3.5) and
    applied to ⟨e, from, to⟩; the emissions ⟨⟨e, result⟩ …⟩ join the representation o.
    Silent transitions, absent definitions, and unfired machines emit nothing."""
    from . import ast
    x = _machine_exprs(trigger_ft, row_col)
    dcell = _S(_COMP, ast.DynFetch(), _S(_CONS, x["em"], x["Dd"]))
    has_em = _S(_COMP, A("not"), _EQ, _S(_CONS, x["em"], x["hash_"]))
    has_def = _S(_COMP, A("not"), A("atom"), dcell)
    result = _S(_COMP, A("apply"), _S(_CONS, x["em"], _S(_CONS, x["e"], x["s"], x["to"])))
    row = _S(_COND, x["both"], _S(_COND, x["okg"], _S(_COND, has_em,
             _S(_COND, has_def, _S(_CONS, x["e"], result), x["hash_"]), x["hash_"]),
             x["hash_"]), x["hash_"])
    return _S(_COMP, T.Filter(_S(_COMP, A("not"), A("atom"))), _S(_ALPHA, row),
              _DISTR, _S(_CONS, _1, x["ctx"]))


def _governed_player(D, ft):
    """The player of `ft` whose noun a machine governs (directly via smDef, or through
    the derived governedBy closure), and so whose status cell the trigger advances."""
    gov = {r[1] for r in _pop_rows(D, "smDef")} | {r[0] for r in _pop_rows(D, "governedBy")}
    for (_rid, f, _p, otype) in _pop_rows(D, "role"):
        if f == ft and otype in gov:
            return otype
    return None


def create(D, fact_type, fact, fuel=None):
    """THE ORM-level entry: the caller names only the fact. Whether a machine runs is the
    ORM layer's business, read off M — when `fact_type` is some transition's trigger
    (smTrigger) and one of its players is governed (smDef plus the derived governedBy
    closure), the M-driven machine step and its Mealy emissions are attached to that
    player's status cell; how it runs is the AST layer's (Prop. onestep: the one
    transition, the trigger fact entering P IS the firing). Absorbed fact types route to
    their RMAP table, the machine taking the row form (fired = the trigger's column went
    non-hole on the addressed entity's own 3NF row)."""
    from . import ast
    part = rmap_partition(D)
    table = part.get(fact_type, fact_type)
    row_col = None
    if table != fact_type:
        row_col = 2 + table_columns(part, table).index(fact_type)
    machine = mealy = None
    if any(r[1] == fact_type for r in _pop_rows(D, "smTrigger")):
        noun = _governed_player(D, fact_type)
        if noun is not None:
            machine = (noun + "_status", machine_step(fact_type, row_col))
            mealy = mealy_step(fact_type, row_col)
    if table != fact_type:
        return create_routed(D, fact_type, fact, part, machine=machine, mealy_obj=mealy)
    return ast.run(fact, D, cell_name=fact_type, machine=machine, mealy_obj=mealy, fuel=fuel)


def create_stamped(D, ft, fact, tx):
    """Bitemporal τ (Halpin §13.6): transaction time is when the SYSTEM records the fact
    — ⟨tx, …fact⟩ enters the ft@tx log beside the base fact; valid time is ordinary UoD
    data inside the fact itself. The platform's stream sequencer supplies tx: arrival
    order at a stream IS transaction time (writer model); the engine holds no clock."""
    from . import ast, defs as _d
    from .reduce import apply as _ap
    from .lam import to_lam, from_lam
    import pyarest.lam as L
    res = create(D, ft, fact)
    o, D2 = _d._items(L._list(res))
    if from_lam(o) == "ERROR":
        return D2
    rows = tuple(tuple(r) for r in _pop_rows(D2, ft + "@tx")) + \
        ((tx,) + tuple(from_lam(fact)),)
    return _ap(ast.Store(ft + "@tx"), _S(to_lam(rows), D2))


def as_of(D, ft, tx):
    """The population as of transaction time `tx`, reconstructed from the τ log —
    Prop. onestep's order_τ audit view: Filter(tx′ ≤ tx), then project the fact."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    keep = T.Filter(_S(_COMP, A("le"), _S(_CONS, _1, _S(_CONST, A(tx)))))
    expr = _S(_COMP, _S(_ALPHA, A("tl")), keep, ast.FetchPop(ft + "@tx"))
    return {tuple(r) for r in from_lam(_ap(expr, D))}


def subscribe(D, sub_id, cells, def_name):
    """Cor. stream: a subscription IS a ρ-application that has not yet been evaluated
    against the current D — `def_name` names an ordinary definition (a cell, §13.3.5);
    the subscription facts record which cells it reads. The pending set is data."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    rows = tuple(tuple(r) for r in _pop_rows(D, "subscription")) + \
        tuple((sub_id, c, def_name) for c in cells)
    return _ap(ast.Store("subscription"), _S(to_lam(rows), D))


def _changed_closure(D, changed):
    """`changed` closed transitively through the rule graph: a changed cell wakes what
    reads it, and what those rules derive changes in turn (the frontier's derives hop,
    iterated). A sound over-approximation: waking a subscription whose cell did not in
    fact change merely re-evaluates the deferred ρ-application, which is its meaning."""
    reads = [(r[0], r[1]) for r in _pop_rows(D, "ruleReads") if len(r) >= 2]
    derives = {}
    for r in _pop_rows(D, "ruleDerives"):
        if len(r) >= 2:
            derives.setdefault(r[0], set()).add(r[1])
    out = set(changed)
    while True:
        grown = set(out)
        for (rule, ft) in reads:
            if ft in grown:
                grown |= derives.get(rule, set())
        if grown == out:
            return out
        out = grown


def wake(D, changed):
    """Evaluate every subscription due on `changed` (transitively through the rule
    graph): the deferred ρ-applications, now applied to the current D. Returns
    {subscription id: value}."""
    from . import defs
    from .reduce import apply as _ap
    from .lam import from_lam, atom as _A
    cl = _changed_closure(D, changed)
    due = {}
    for r in _pop_rows(D, "subscription"):
        if len(r) >= 3 and r[1] in cl:
            due[r[0]] = r[2]
    out = {}
    with defs.step(D):
        for sid, dname in due.items():
            out[sid] = from_lam(_ap(_A(dname), D))
    return out


def step_and_wake(D, fact_type, fact):
    """The commit path, wired (Cor. stream): one ORM-level create, the semi-naive
    derivation of the affected fragment, then the subscriptions due on what changed.
    Returns (⟨o, D′⟩, wakes); a refused step (ERROR) derives and wakes nothing."""
    from . import defs as _d
    from .lam import from_lam
    import pyarest.lam as L
    res = create(D, fact_type, fact)
    o, D2 = _d._items(L._list(res))
    if from_lam(o) == "ERROR":
        return res, {}
    changed = {fact_type}
    if any(r[1] == fact_type for r in _pop_rows(D2, "smTrigger")):
        noun = _governed_player(D2, fact_type)
        if noun is not None:
            changed.add(noun + "_status")
    D2 = run_rules(D2, changed=changed)
    return _S(o, D2), wake(D2, changed)


def finality_modality(D, noun, depth):
    """The writer model's hardening rule, read off M's finality facts: below the noun's
    declared depth k a violation reports DEONTICALLY (optimistic acceptance, V as the
    repair obligation); at or beyond k it refuses ALETHICALLY. An undeclared noun is
    final immediately. Nakamoto §11 quantifies any chosen k."""
    ks = {r[0]: r[1] for r in _pop_rows(D, "finality") if len(r) == 2}
    k = ks.get(noun, 0)
    return "deontic" if depth < k else "alethic"


def declare_sig(D, name, dom, cod):
    """Def. reg's ⟨dom, cod⟩ as M facts: defSig rows ⟨name, position, objectType⟩ with
    cod at position 0 — DatalogLB-style typed-predicate constraints on the boundary."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    rows = tuple(tuple(r) for r in _pop_rows(D, "defSig")) + \
        tuple((name, i + 1, t) for i, t in enumerate(dom)) + ((name, 0, cod),)
    return _ap(ast.Store("defSig"), _S(to_lam(rows), D))


def checked_apply(name):
    """The typed boundary application (Def. reg dom/cod): ⟨args, D⟩ applies registered
    `name` iff every argument at a declared dom position is an instance of its object
    type (membership in the type's index cell, read from D in-step), else the ERROR atom
    the transition rule refuses (§14.3.1). Undeclared names apply unchecked (dom
    unconstrained); a sig naming an absent type cell fails closed (⊥ → ERROR). cod is
    declared at position 0 and enforced by the receiving cell's own constraints."""
    from . import ast
    named = _S(_CONST, A(name))
    sig = _S(_COMP, T.Filter(_S(_COMP, A("gt"), _S(_CONS, A(2), _S(_CONST, A(0))))),
             T.Filter(_S(_COMP, _EQ, _S(_CONS, _1, named))),
             ast.FetchPop("defSig"), _2)
    pairs = _S(_COMP, _DISTR, _S(_CONS, sig, _ID))           # ⟨⟨(n,p,t), ⟨args,D⟩⟩ …⟩
    arg = _S(_COMP, A("apply"), _S(_CONS, _S(_COMP, A(2), _1), _S(_COMP, _1, _2)))
    tpop = _S(_COMP, _S(_ALPHA, A(1)), ast.DynFetch(), _S(_CONS, _S(_COMP, A(3), _1),
                                                          _S(_COMP, _2, _2)))
    chk = _S(_COMP, T.member, _S(_CONS, arg, tpop))
    checks = _S(_COMP, _S(_ALPHA, chk), pairs)
    allok = _S(_COND, _S(_COMP, A("null"), checks), _S(_CONST, A("T")),
               _S(_COMP, _S(_INSERT, A("and")), checks))
    out = _S(_COMP, A("apply"), _S(_CONS, named, _1))
    return _S(_COND, allok, out, _S(_CONST, A("ERROR")))


def finiteness_check(D):
    """The static condition discharging Lemma finiteness' hypothesis: recursion through
    the rule dependency graph is admitted — heads are range-restricted by construction,
    so the fixpoint runs over a finite atom domain and terminates — but no dependency
    CYCLE may pass through value invention, a rule whose definition applies a registered
    (boundary, Cor. boundary) function and so can introduce individuals drawn from no
    stored population, unboundedly. Value invention is (a) anything registered beyond
    the formal base (bridges, cellkey, FFI — the boundary proper) and (b) the base's own
    value-constructing ops (arithmetic, length, dynamic apply), which mint new atoms just
    as surely. Acyclic invention stays admissible (a finite composition introduces
    finitely many individuals). Returns the offending rule names. Definitions are cells,
    so rule bodies are read from D like everything else."""
    from . import defs, prims
    from .lam import from_lam
    reads, derives = {}, {}
    for (r, ft) in _pop_rows(D, "ruleReads"):
        reads.setdefault(r, set()).add(ft)
    for (r, ft) in _pop_rows(D, "ruleDerives"):
        derives.setdefault(r, set()).add(ft)
    boundary = (set(defs._registered) - set(prims.BASE)) | {"+", "-", "*", "div", "length", "apply"}

    def _atoms(v):
        if isinstance(v, tuple):
            for x in v:
                yield from _atoms(x)
        elif isinstance(v, str):
            yield v

    cells = defs._cells_of(D)
    inventive = set()
    for r in set(reads) | set(derives):
        body = cells.get(r)
        if body is not None and any(a in boundary for a in _atoms(from_lam(body))):
            inventive.add(r)
    succ = {}
    for r in reads:                                          # ft-level dependency edges
        for src in reads[r]:
            succ.setdefault(src, set()).update(derives.get(r, ()))

    def _reaches(a, b):
        seen, stack = set(), [a]
        while stack:
            n = stack.pop()
            if n == b:
                return True
            if n not in seen:
                seen.add(n)
                stack.extend(succ.get(n, ()))
        return False

    return sorted(r for r in inventive
                  if any(_reaches(d, s) for d in derives.get(r, ()) for s in reads.get(r, ())))


def governance_rules(D):
    """Install the governedBy closure with the engine's own rule machinery: a noun is
    governed by the machine it is bound to, and by any machine governing a supertype.
    run_rules then derives the closure like any other rule."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import to_lam
    plans = (("governedBy_rule_base", ["smDef"], [2, 1]),
             ("governedBy_rule_step", ["subtype", "governedBy"], [1, 3]))
    atoms = []
    for (name, fts, head) in plans:
        D = _ap(ast.DefineIn(name, compile_rule(fts, head)), D)
        for i, ft in enumerate(fts):                         # semi-naive ~d variants
            D = _ap(ast.DefineIn(f"{name}~d{i + 1}", compile_rule_delta(fts, head, i)), D)
            atoms.append((name, i + 1, ft))
    derives = tuple(tuple(r) for r in _pop_rows(D, "ruleDerives")) + \
        (("governedBy_rule_base", "governedBy"), ("governedBy_rule_step", "governedBy"))
    D = _ap(ast.Store("ruleDerives"), _S(to_lam(derives), D))
    reads = tuple(tuple(r) for r in _pop_rows(D, "ruleReads")) + \
        (("governedBy_rule_base", "smDef"), ("governedBy_rule_step", "subtype"),
         ("governedBy_rule_step", "governedBy"))
    D = _ap(ast.Store("ruleReads"), _S(to_lam(reads), D))
    rows = tuple(tuple(r) for r in _pop_rows(D, "ruleAtom")) + tuple(atoms)
    return _ap(ast.Store("ruleAtom"), _S(to_lam(rows), D))


