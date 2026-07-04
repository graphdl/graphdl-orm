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
def validate_of(constraints, alethic=None, scoped=(), scoped_alethic=None):
    """validate_S : ⟨P, D⟩ ↦ ⟨P, V, alethicViolated⟩ (Def. Command / Violation). `constraints`
    consume the target population P (cell-local — composed with the selector); `scoped` consume
    ⟨P, D⟩ whole (cross-cell — they fetch sibling cells from the frozen D). `alethic` /
    `scoped_alethic` are the commit-blocking subsets (default: all of each). The canonical
    builder (shared/system.py) applied to ⟨local, alethic?, scoped, scoped_alethic?⟩; an
    absent subset is the empty slot, a provided one wraps (deliberately-deontic empties
    stay distinct from absence)."""
    from .reduce import apply as _apply

    def lst(objs):
        out = L.NIL
        for o in reversed(list(objs)):
            out = L.CONS(o)(out)
        return L.SEQ(out)

    def slot(v):
        return _S() if v is None else _S(lst(v))

    rec = _S(lst(constraints), slot(alethic), lst(scoped), slot(scoped_alethic))
    return _apply(A("system:validate_of"), rec)


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
    """One round: F_S(P) = P ∪ ⋃_rules rule(P) (the canonical system:F_of applied
    to the rule sequence; empty rules answer the identity)."""
    from .reduce import apply as _apply
    return _apply(A("system:F_of"), _S(*rules) if rules else _S())


def derive_of(rules):
    """derive_S = lfp(F_S, ·): Backus `while` iterating F_S until (F_S:P) ∖ P = φ
    (the canonical system:derive_of applied to the rule sequence). Pass only the
    frontier's affected rules to keep the lfp bounded to the touched fragment
    (Cor. streaming)."""
    from .reduce import apply as _apply
    return _apply(A("system:derive_of"), _S(*rules) if rules else _S())


# --- role-path -> F_S: a derivation rule is a Datalog rule q(..) <- p1(..), .., pm(..) (ORM ->
# Datalog): a conjunctive query whose body atoms join on shared variables and project to the head
# (the role path). A recursive head (ancestor <- link ; link, ancestor) is resolved by derive_of's
# least fixed point. Each compiled rule is an FFP object P -> its derived head facts. ---
def join_rule(join_role, head_cols):
    """A two-atom SELF-referential role-path rule over one fact type: join the fact type to itself
    on `join_role` (R.join_role = R'.1) and project to `head_cols`. This is the recursive body,
    e.g. ancestor(x,z) <- link(x,y), ancestor(y,z) with head_cols=[1,3], join_role=2 — feed it to
    derive_of for the least fixed point (transitive closure). The canonical
    system:join_rule applied to ⟨join_role, head_cols⟩."""
    from .lam import to_lam
    from .reduce import apply as _apply
    return _apply(A("system:join_rule"), to_lam((join_role, tuple(head_cols))))


def join_rule2(join_role, head_cols):
    """A two-atom role-path rule over two fact types: input ⟨A, B⟩; join A.join_role = B.1 and
    project to head_cols over the combined tuple (e.g. FastCarDriver(x) <- drives(x,y), isFast(y)).
    The canonical system:join_rule2 applied to ⟨join_role, head_cols⟩."""
    from .lam import to_lam
    from .reduce import apply as _apply
    return _apply(A("system:join_rule2"), to_lam((join_role, tuple(head_cols))))


# the storage half of a NORMA */**/+/++ marker: whether the derived facts are materialized (stored)
# vs recomputed on demand. (* and + recompute; ** and ++ store.) The derivation half is the rule
# above, fed to derive_of; the create pipeline runs it as the `derive` stage over the fact's cell.
_MATERIALIZE = {"fully-derived": False, "derived-and-stored": True,
                "semi-derived": False, "partially-derived-and-stored": True}


def materialize(marker):
    """True if the marker means store the derived facts (** / ++), False if compute on demand (* / +)."""
    return _MATERIALIZE.get(marker, False)


# --- resolve with auto-counter minting (Def. Command: mint iff the ref scheme auto-generates) ---
def mint_next(col):
    """P ↦ 1 + the greatest value in column `col` of P (or 1 if empty): the auto-counter's
    next id — successor of a max-fold over the id column (one surrogate per guarded
    step). The canonical system:mint_next applied to the column selector."""
    from .reduce import apply as _apply
    return _apply(A("system:mint_next"), A(col))


def resolve_minting(col):
    """resolve for an auto-generating entity: mint the next id and prepend ⟨id, …I⟩
    to P. The canonical system:resolve_minting applied to the column selector."""
    from .reduce import apply as _apply
    return _apply(A("system:resolve_minting"), A(col))


# --- emit: HATEOAS — the representation carries its own links (Thm. hateoas) ---
# links(e) = nav(e) ∪ transitions(status(e)): the related resources plus the actions available
# from the entity's current state. Both are theta1 selections — nav over P, transitions over a
# state machine value; the representation is self-describing, no link table maintained.
def nav_of(key_pos):
    """nav(e): the facts of P sharing the affected entity's key (role `key_pos` of the head fact).
        α(1) ∘ Filter(key(f) = headKey) ∘ distr ∘ [id, key∘1]
    The canonical builder applied to the key selector (shared/system.py)."""
    from .reduce import apply as _apply
    return _apply(A("system:nav_of"), A(key_pos))


def transitions_of(sm, status_pos):
    """transitions(status(e)): the state-machine transitions available from the head fact's
    status. `sm` is a value ⟨⟨from, trigger, to⟩…⟩; a transition fires when from = status(head).
        α(1) ∘ Filter(from(t) = status) ∘ distr ∘ [sm̄, status∘1]
    The canonical builder applied to ⟨sm, pos⟩ (shared/system.py)."""
    from .reduce import apply as _apply
    return _apply(A("system:transitions_of"), _S(sm, A(status_pos)))


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
def cmp_filter(op, col, lit=None, col2=None):
    """The comparator PREDICATE over a joined row: col `op` lit, or col `op` col2
    (the cross-antecedent form). compile_rule Filter-wraps it after the joins. The
    canonical builders (shared/system.py)."""
    from .reduce import apply as _apply
    from .lam import to_lam
    if col2 is not None:
        return _apply(A("system:cmp_filter_col"), _S(A(op), A(col), A(col2)))
    return _apply(A("system:cmp_filter_lit"), _S(A(op), A(col), to_lam(lit)))


def _rule_atoms(atom_fts, widths, joins):
    """⟨⟨ft, width, join?⟩…⟩ — the shared atom-spec encoding for the rule builders."""
    from .lam import to_lam
    ws = list(widths) if widths else [2] * len(atom_fts)
    js = list(joins) if joins else [None] * (len(atom_fts) - 1)
    js = [None] + js                                          # first atom never joins
    atoms = L.NIL
    for ftn, w, j in reversed(list(zip(atom_fts, ws, js))):
        spec = to_lam(()) if j is None else _S(to_lam(tuple(tuple(p) for p in j[0])),
                                               to_lam(tuple(j[1])))
        atoms = L.CONS(_S(A(ftn), to_lam(w), spec))(atoms)
    return L.SEQ(atoms)


def _obj_list(objs):
    out = L.NIL
    for o in reversed(list(objs or ())):
        out = L.CONS(o)(out)
    return L.SEQ(out)


def compile_rule(atom_fts, head_positions, widths=None, filters=None, joins=None):
    """A rule's body as one FFP object over D: the populations of the clause fact types,
    each fetched from its own cell, joined linearly by default and by the general Codd
    join where `joins` says so, with the head's variable positions projected. The
    canonical WHILE-over-atoms builder (shared/system.py) applied to
    ⟨⟨ft, width, join?⟩…, head, filters⟩. Cross-cell by construction:
    store-on-derive's read side."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(head_positions)),
             _obj_list(filters))
    return _apply(A("system:compile_rule"), rec)


def compile_rule_neg(atom_fts, head_positions, ncols, widths, filters, joins, negs):
    """The positive body wrapped in stratified anti-joins: per negation group
    (neg_atom_fts, neg_key_proj, neg_widths, neg_filters, neg_joins, anti_key),
    a running tuple survives iff its anti_key columns are NOT among the group's
    projected keys (theta:AntiRestrict — Restrict's mirror with the membership
    negated). Full recompute above the closure, exactly like aggregates:
    semi-naive deltas are unsound under negation-as-failure."""
    from .reduce import apply as _apply
    from .lam import to_lam
    obj = compile_rule(atom_fts, list(range(1, ncols + 1)), widths, filters,
                       joins)
    for (nfts, nproj, nwidths, nfilters, njoins, anti_key) in negs:
        neg = compile_rule(nfts, nproj, nwidths, nfilters, njoins)
        # the composition shape is CANONICAL (system:anti_wrap, shared base);
        # this wrapper only marshals the group spec — the wrapper doctrine
        obj = _apply(A("system:anti_wrap"),
                     _S(obj, neg, to_lam((tuple(anti_key),
                                          tuple(range(1, len(nproj) + 1))))))
    head = _apply(A("theta:Project"), to_lam(tuple(head_positions)))
    return _S(A("COMP"), head, obj)


def compile_rule_delta(atom_fts, head_positions, delta_at, widths=None, filters=None,
                       joins=None):
    """The rule body with atom `delta_at` (0-based) reading the round's DELTA instead of
    its cell: an FFP object over ⟨Δ, D⟩ — semi-naive evaluation's inner join
    (Bancilhon–Ramakrishnan 1986). The canonical builder (shared/system.py) applied to
    ⟨atoms, head, filters, delta_at+1⟩; every non-delta fetch composes with selector 2
    of the pair."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(head_positions)),
             _obj_list(filters), to_lam(delta_at + 1))
    return _apply(A("system:compile_rule_delta"), rec)


def class_rule(clauses, head_const):
    """A grammar recognizer as one FFP object over D (the parser is the file): each
    clause ⟨field_ft, literal-or-None⟩ selects the Statements whose field cell holds
    the literal (or holds anything, when literal-less — the existence test); clauses
    intersect; the head pairs each surviving Statement with the constant
    classification."""
    from . import ast

    def subj(ftb, lit):
        pop = ast.FetchPop(ftb)
        if lit is not None:
            pop = _S(_COMP, T.Filter(_S(_COMP, _EQ, _S(_CONS, A(2), _S(_CONST, A(lit))))), pop)
        return _S(_COMP, _S(_ALPHA, A(1)), pop)

    s = subj(*clauses[0])
    for (ftb, lit) in clauses[1:]:
        keep = _S(_COMP, T.member, _S(_CONS, _1, _2))
        s = _S(_COMP, _S(_ALPHA, _1), T.Filter(keep), _DISTR, _S(_CONS, s, subj(ftb, lit)))
    row = _S(_CONS, _ID, _S(_CONST, A(head_const)))
    return _S(_COMP, _S(_ALPHA, row), T.dedup, s)


def compile_agg_rule(atom_fts, group_positions, over_position, op,
                     widths=None, filters=None, joins=None):
    """An aggregate rule (Def. derive: 'an aggregate reducing a finite bag to one
    scalar'): joins and filters as compile_rule, then per GROUP (the non-aggregated
    head variables) the fold of `op` over the aggregated column. Stratified above the
    positive closure; the head REPLACES on recompute — an aggregate head is functional
    per group, so union-merge would preserve stale folds (the misfold the old engine
    documented). The canonical builder applied to ⟨atoms, group, over, op, filters⟩."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rec = _S(_rule_atoms(atom_fts, widths, joins), to_lam(tuple(group_positions)),
             to_lam(over_position), A(op), _obj_list(filters))
    return _apply(A("system:compile_agg_rule"), rec)


# FAST twins for classifier rules (stratum 4): keyed by rule cid, fn(D) -> row
# set. Speed as registration under the canonical name — the canonical object in
# D stays the meaning; run_rules consults the twin before generic evaluation.
# Twins rebuild FROM M (classSpec facts freeze with the store), so the thawed
# grammar path gets them too.
rule_twins = {}


def _rowsort(rows):
    """Deterministic row ordering that survives MIXED-TYPE cells: a migrated
    lexical row ('150') and a coerced-arithmetic derivation (150) coexist under
    NATEQ, and bare sorted() has no int-str ordering (the claude rehearsal
    crashed exactly there). Type name then lexical value, per element."""
    return tuple(sorted(rows, key=lambda r: tuple(
        (type(x).__name__, str(x)) for x in r) if isinstance(r, tuple)
        else ((type(r).__name__, str(r)),)))


def rebuild_class_twins(D):
    """Reconstruct class-rule twins from the store's classSpec facts:
    ⟨rid, field_ft, literal-or-empty, head-classification⟩ per clause. The twin
    is the contract from system.class_rule — filter each field cell on column 2
    (or existence), intersect statement ids, pair with the head constant."""
    specs = {}
    for r in _pop_rows(D, "classSpec"):
        if len(r) >= 4:
            specs.setdefault(r[0], ("", []))
            head, clauses = specs[r[0]]
            specs[r[0]] = (r[3], clauses + [(r[1], r[2] or None)])
    for rid, (head, clauses) in specs.items():
        def _twin(Dx, _cl=tuple(clauses), _head=head):
            sids = None
            for (ftb, lit) in _cl:
                rows = _pop_rows(Dx, ftb)
                s = {r[0] for r in rows
                     if r and (lit is None or (len(r) >= 2 and r[1] == lit))}
                sids = s if sids is None else (sids & s)
                if not sids:
                    return set()
            return {(sid, _head) for sid in (sids or ())}
        rule_twins[rid] = _twin
    return len(specs)


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
    aggids = {r[0] for r in _pop_rows(D, "ruleAgg") if r}
    all_rules = [(r[0], r[1]) for r in _pop_rows(D, "ruleDerives")]
    rules = [(rid, h) for (rid, h) in all_rules if rid not in aggids]
    frontier = None if changed is None else set(changed)
    closure_changed = set()
    delta, rnd = None, 0
    while True:
        rnd += 1
        fired, next_delta = False, {}
        for rule_cid, head in rules:
            if delta is None:                                # round one: full bodies
                if frontier is not None and not (reads.get(rule_cid, set()) & frontier):
                    continue
                tw = rule_twins.get(rule_cid)
                if tw is not None:                           # the FAST twin: same rows,
                    new_rows = set(tw(D))                    # no generic evaluation
                else:
                    with defs.step(D):
                        outs = from_lam(_ap(_A(rule_cid), D))
                    if not isinstance(outs, tuple):
                        continue                             # rule not compiled (M-facts only)
                    new_rows = {tuple(r) for r in outs if isinstance(r, tuple)}
                if stats is not None:
                    stats.append({"round": rnd, "rule": rule_cid, "mode": "full"})
            else:
                hits = [(p, ft) for (p, ft) in atomsof.get(rule_cid, ()) if ft in delta]
                if hits:
                    new_rows = set()
                    for (p, ft) in hits:
                        drows = _rowsort(delta[ft])
                        with defs.step(D):
                            o = from_lam(_ap(_A(f"{rule_cid}~d{p}"), _S(to_lam(drows), D)))
                        if isinstance(o, tuple):
                            new_rows |= {tuple(r) for r in o if isinstance(r, tuple)}
                        if stats is not None:
                            stats.append({"round": rnd, "rule": rule_cid, "mode": "delta",
                                          "pos": p, "in": len(drows),
                                          "base": len(_pop_rows(D, ft))})
                elif rule_cid not in atomsof and (reads.get(rule_cid, set()) & set(delta)):
                    tw = rule_twins.get(rule_cid)
                    if tw is not None:
                        new_rows = set(tw(D))
                    else:
                        with defs.step(D):                   # legacy rule: full fallback
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
                D = _ap(ast.Store(head), _S(to_lam(_rowsort(old | add)), D))
                fired = True
                next_delta.setdefault(head, set()).update(add)
        if not fired:
            break
        delta = next_delta
        closure_changed.update(next_delta)
    # THE UPPER STRATA, iterated to a JOINT fixpoint. Three passes sit above
    # the positive closure, and each can invalidate the others' work through
    # the dependency graph (loads settle, ranks rederives over them, the peak
    # refolds over ranks), so they repeat until a full sweep changes nothing:
    #
    #   agg   — aggregate heads supersede PER GROUP (functional per group;
    #           union would keep stale folds, whole-cell replace clobbered the
    #           corpus's paired zero-supply rows — the count-of-empty lesson);
    #   keyed — heads whose fact type carries a uniqueness over a role prefix
    #           (the old engine's task-955 upsert) re-evaluate over the
    #           settled store and supersede PER KEY; asserted rows whose key
    #           the rules did not produce survive;
    #   sweep — DELETE-AND-REDERIVE (Gupta-Mumick-Subrahmanian 1993, in the
    #           library): for a FULLY-derived plain head the stored cell is
    #           materialization of the expressible set (Codd 1970 §1.5), never
    #           ground truth, so it re-evaluates whole and REPLACES — which
    #           both propagates this invocation's supersessions and converges
    #           staleness inherited from earlier stores (frozen caches, replay
    #           history), making derive idempotent. Whole-cell rederivation is
    #           the paper's overestimate-then-rederive at cell granularity,
    #           sound exactly because no row is asserted. Self-supporting
    #           heads (reachable from themselves through derived-head reads)
    #           stay out: their overestimate can rederive itself through the
    #           cycle, and cleaning them needs the delta form of the paper.
    kindmap = {r[0]: r[1] for r in _pop_rows(D, "derivation") if len(r) >= 2}
    spans_of = {}
    for r in _pop_rows(D, "spans"):
        if len(r) >= 2:
            spans_of.setdefault(r[0], set()).add(r[1])
    keyspans = {}
    for c in _pop_rows(D, "constraint"):
        if len(c) >= 3 and c[1] in ("uniqueness", "spanning_uniqueness"):
            ps = spans_of.get(c[0], set())
            if ps:
                keyspans.setdefault(c[2], set()).update(ps)
    agg_rules = [(rid, head) for (rid, head) in all_rules if rid in aggids]
    agg_heads = {head for (_rid, head) in agg_rules}
    keyed_of = {}
    for (rid, head) in all_rules:
        if rid not in aggids and head in keyspans:
            keyed_of.setdefault(head, []).append(rid)
    plain_of = {}
    for (rid, head) in all_rules:
        if rid not in aggids:
            plain_of.setdefault(head, []).append(rid)
    reach = {h: {ft for rid in rids for ft in reads.get(rid, set())}
             for h, rids in plain_of.items()}
    derived_heads = set(agg_heads) | set(plain_of)

    def _self_supporting(h):
        seen, stack = set(), [x for x in reach.get(h, ()) if x in derived_heads]
        while stack:
            x = stack.pop()
            if x == h:
                return True
            if x in seen:
                continue
            seen.add(x)
            stack.extend(y for y in reach.get(x, ()) if y in derived_heads)
        return False

    sweep = sorted(h for h in plain_of
                   if kindmap.get(h) == "fully-derived"
                   and h not in agg_heads and h not in keyed_of
                   and not _self_supporting(h))

    def _eval_rules(rids, Dx):
        outs = set()
        for rid in rids:
            tw = rule_twins.get(rid)
            if tw is not None:
                outs |= set(tw(Dx))
                continue
            with defs.step(Dx):
                o = from_lam(_ap(_A(rid), Dx))
            if isinstance(o, tuple):
                outs |= {tuple(r) for r in o if isinstance(r, tuple)}
        return outs

    # Dirty-set filtering keeps the fixpoint's cost proportional to what
    # actually changed: round one evaluates agg and keyed passes whole (their
    # pre-fixpoint status quo) but sweeps only heads whose reads intersect the
    # dirty set — the asserted frontier plus everything the closure and the
    # earlier passes stored this call. A FULL call (changed=None) sweeps every
    # eligible head once, which is where the idempotence guarantee lives; the
    # per-batch delta path pays only for its own ripple. Later rounds filter
    # all three passes the same way, so iteration runs exactly as deep as the
    # dependency chain that moved.
    dirty = None if changed is None else (set(changed) | closure_changed)
    for _outer in range(12):
        settled = True
        round_changed = set()

        def _touched(read_set):
            if dirty is None:
                return True
            return bool(read_set & dirty) or bool(read_set & round_changed)
        for (rid, head) in agg_rules:
            if (_outer or dirty is not None) and not _touched(reads.get(rid, set())):
                continue
            with defs.step(D):
                out = from_lam(_ap(_A(rid), D))
            if isinstance(out, tuple):
                agg_rows = {tuple(r) for r in out if isinstance(r, tuple)}
                keys = {r[:-1] for r in agg_rows}
                before = {tuple(r) for r in _pop_rows(D, head)}
                merged = agg_rows | {r for r in before if r[:-1] not in keys}
                if merged != before:
                    settled = False
                    round_changed.add(head)
                    D = _ap(ast.Store(head), _S(to_lam(_rowsort(merged)), D))
        for head in sorted(keyed_of):
            if (_outer or dirty is not None) and not _touched(
                    {ft for rid in keyed_of[head] for ft in reads.get(rid, set())}):
                continue
            key_pos = sorted(keyspans[head])
            outs = _eval_rules(keyed_of[head], D)

            def key(r, _kp=key_pos):
                return tuple(r[p - 1] for p in _kp if p <= len(r))
            keys = {key(r) for r in outs}
            kept = {tuple(r) for r in _pop_rows(D, head)
                    if key(tuple(r)) not in keys}
            merged = _rowsort(outs | kept)
            current = _rowsort({tuple(r) for r in _pop_rows(D, head)})
            if merged != current:
                settled = False
                round_changed.add(head)
                D = _ap(ast.Store(head), _S(to_lam(merged), D))
        for head in sweep:
            if not _touched(reach.get(head, set())):
                continue
            outs = _eval_rules(plain_of[head], D)
            current = {tuple(r) for r in _pop_rows(D, head)}
            if outs != current:
                settled = False
                round_changed.add(head)
                D = _ap(ast.Store(head), _S(to_lam(_rowsort(outs)), D))
        if settled:
            break
        dirty = round_changed
    return D


# --- the state machine read off M (whitepaper §1): a machine IS a set of facts ---
# smFrom ⟨t, from⟩ ⋈ smTrigger ⟨t, trigger⟩ ⋈ smTo ⟨t, to⟩, projected to ⟨from, trigger, to⟩ —
# assembling the machine is a theta1 join over M's cells (the canonical defs
# system:sm_join / system:sm_join_named), not a second interpreter. The host
# only fetches and converts: the thin-runner posture, per the Operating Rule
# defs-override-glue-framework.
def sm_triples(D):
    """The machine's ⟨from, trigger, to⟩ triples, joined from M's smFrom/smTrigger/smTo
    cells by the canonical system:sm_join."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(A("system:sm_join"), pops)))


def sm_triples_named(D):
    """⟨transition, from, trigger, to⟩ — the named form, so per-transition facts
    (guards, Mealy emissions) can key in."""
    from . import ast
    from .reduce import apply as _ap
    from .lam import from_lam
    pops = _S(*[_ap(ast.FetchPop(n), D) for n in ("smFrom", "smTrigger", "smTo")])
    return tuple(from_lam(_ap(A("system:sm_join_named"), pops)))


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
    partitioned = {tuple(r[:2]) for r in _pop_rows(D, "subtypePartition") if len(r) >= 2}

    def _top(o):                                             # RMAP step 0 (§10.3): absorb
        seen = set()                                         # subtypes into their top
        while o in subs and o not in seen:                   # supertype — STOPPING at a
            if (o, subs[o]) in partitioned:                  # partitioned edge (Halpin's
                break                                        # partition mapping: mutually
            seen.add(o)                                      # exclusive families keep
            o = subs[o]                                      # their own tables)
        return o

    arity = {}
    for r in _pop_rows(D, "role"):
        if len(r) >= 3:
            arity[r[1]] = max(arity.get(r[1], 0), r[2])
    subject = {r[1]: _top(r[3]) for r in reversed(_pop_rows(D, "role")) if r[2] == 1}
    role2 = {r[1]: _top(r[3]) for r in reversed(_pop_rows(D, "role")) if r[2] == 2}
    spans = {}
    for r in _pop_rows(D, "spans"):
        if len(r) == 2:
            spans.setdefault(r[0], set()).add(r[1])
    ucpos, mand = {}, {}
    for c in cons:
        if len(c) >= 3 and c[1] == "uniqueness":
            ucpos.setdefault(c[2], set()).update(spans.get(c[0], {1}))
        if len(c) >= 4 and c[1] == "mandatory":
            mand.setdefault(c[2], set()).add(_top(c[3]))

    def _side(ft):
        # 1:1 grouping favors fewer nulls (Halpin §10.3): a doubly-functional fact type
        # absorbs into the MANDATORY side — every instance there plays, so no # holes
        s1 = subject.get(ft, ft)
        if {1, 2} <= ucpos.get(ft, set()):
            s2 = role2.get(ft)
            if s2 and s2 in mand.get(ft, set()) and s1 not in mand.get(ft, set()):
                return s2
        return s1

    # a UNARY is functional by definition (its internal UC spans its one role —
    # Halpin's boolean-column mapping; NORMA auto-creates that UC), so it absorbs
    # with no declaration at all
    triples = tuple((ft, _side(ft),
                     "functional" if (arity.get(ft) == 1 or
                                      (ft in functional and ft not in spanning))
                     else "spanning")
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


def row_resolve(col, width, unary=False):
    """resolve for an entity-cell write: ⟨I, row⟩ → row′, where I = ⟨key, value⟩ and the
    cell holds the entity's 3NF row (a fresh entity gets holes, the default object #). A
    conflicting functional write makes the column ⊥, the row collapses (§11.2.1), and the
    step's transition rule refuses it atomically: absorption makes the UC structural."""
    key = _S(_COMP, _1, _1)
    val = _S(_CONST, A("T")) if unary else _S(_COMP, _2, _1)   # unary: the boolean column
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


def create_routed(D, ft, fact, partition, machine=None, mealy_obj=None, validate_obj=None):
    """Route a create through the RMAP partition (spec §4.4: the partition IS the layout).
    An absorbed fact type writes the entity's own cell `table:key`, the write unit of
    Def. iso, updating its column of the row; an own-table fact type creates into its
    per-fact-type cell unchanged. The table's index cell records the key IN THE SAME
    STEP (index_cell rides the commit chain like the machine slot), so a refused write
    leaves the index untouched. A machine (the row form) advances within the routed
    step; a validate (row_validate: step 5's constraint mapping) refuses within it."""
    from . import ast
    from .lam import from_lam
    table = partition.get(ft, ft)
    if table == ft:
        return ast.run(fact, D, cell_name=ft, machine=machine, mealy_obj=mealy_obj,
                       validate_obj=validate_obj)
    cols = table_columns(partition, table)
    col = 2 + cols.index(ft)
    key = from_lam(fact)[0]
    unary = max((r[2] for r in _pop_rows(D, "role") if len(r) >= 3 and r[1] == ft),
                default=2) == 1
    return ast.run(fact, D, cell_name=f"{table}:{key}",
                   resolve_obj=row_resolve(col, 1 + len(cols), unary),
                   machine=machine, mealy_obj=mealy_obj, validate_obj=validate_obj,
                   index_cell=table, append_cell=ft)


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
    unary = max((r[2] for r in _pop_rows(D, "role") if len(r) >= 3 and r[1] == ft),
                default=2) == 1
    key = _S(_COMP, _1, _1)                                  # of ⟨⟨key⟩, D⟩: the key scalar
    name = _S(_COMP, A("cellkey"), _S(_CONS, _S(_CONST, A(table)), key))
    row = _S(_COMP, ast.DynFetch(), _S(_CONS, name, _2))
    pair = _S(_CONS, key, _S(_COMP, A(col), row))
    nonhole = _S(_COMP, A("not"), _EQ, _S(_CONS, A(2), _S(_CONST, A("#"))))
    expr = _S(_COMP, T.Filter(nonhole), _S(_ALPHA, pair), _DISTR)
    idx = _ap(ast.FetchPop(table), D)
    pairs = set(from_lam(_ap(expr, _S(idx, D))))
    if unary:
        return {(k,) for (k, v) in pairs if v == "T"}         # the boolean column, back
    return pairs


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
    from .reduce import apply as _apply
    from .lam import to_lam
    rc = to_lam(()) if row_col is None else _S(to_lam(row_col))
    return _apply(A("system:machine_step"), _S(A(trigger_ft), rc))


def mealy_step(trigger_ft, row_col=None):
    """Mealy output on the SAME step: for each entity whose transition fires, the
    transition's named definition (smEmit, read from M in-step like everything else) is
    resolved by ρ from D's own cells (definitions are ordinary cells, §13.3.5) and
    applied to ⟨e, from, to⟩; the emissions ⟨⟨e, result⟩ …⟩ join the representation o.
    Silent transitions, absent definitions, and unfired machines emit nothing."""
    from .reduce import apply as _apply
    from .lam import to_lam
    rc = to_lam(()) if row_col is None else _S(to_lam(row_col))
    return _apply(A("system:mealy_step"), _S(A(trigger_ft), rc))


def _governed_player(D, ft):
    """The player of `ft` whose noun a machine governs (directly via smDef, or through
    the derived governedBy closure), and so whose status cell the trigger advances."""
    gov = {r[1] for r in _pop_rows(D, "smDef")} | {r[0] for r in _pop_rows(D, "governedBy")}
    for (_rid, f, _p, otype) in _pop_rows(D, "role"):
        if f == ft and otype in gov:
            return otype
    return None


_AUTH_FT = "User_is_authorized_for_Operation_on_Resource"


def create(D, fact_type, fact, fuel=None, actor=None, operation="create"):
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
    # authorization (the access module, when ingested): the actor must hold the derived
    # ⟨user, operation, resource⟩ where the resource is the RMAP table the write lands
    # in; refusal answers ⟨ERROR, unchanged D⟩. Absent module: ungoverned (graceful).
    if actor is not None and any(r and r[0] == _AUTH_FT for r in _pop_rows(D, "factType")):
        allowed = {tuple(r) for r in _pop_rows(D, _AUTH_FT)}
        if (actor, operation, table) not in allowed:
            return _S(A("ERROR"), D)
    row_col = None
    if table != fact_type:
        row_col = 2 + table_columns(part, table).index(fact_type)
    machine = mealy = links = None
    if any(r[1] == fact_type for r in _pop_rows(D, "smTrigger")):
        noun = _governed_player(D, fact_type)
        if noun is not None:
            role_pos = next((r[2] for r in _pop_rows(D, "role")
                             if len(r) >= 4 and r[1] == fact_type and r[3] == noun), None)
            machine = (noun + "_status", machine_step(fact_type, row_col), role_pos)
            mealy = mealy_step(fact_type, row_col)
            if table == fact_type and role_pos is not None:
                # Thm. hateoas at the ORM level: the representation offers exactly the
                # next transitions from the POST-step status, no caller wiring
                from .lam import to_lam
                links = transitions_of(to_lam(sm_triples(D)), 2)
    if table != fact_type:
        return create_routed(D, fact_type, fact, part, machine=machine, mealy_obj=mealy,
                             validate_obj=row_validate(D, fact_type, part))
    return ast.run(fact, D, cell_name=fact_type, machine=machine, mealy_obj=mealy,
                   links_obj=links, fuel=fuel)


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


def ftpop_expr(ft, partition):
    """The fact type's population as one FFP expression over D, whatever the layout:
    an own-table fact type reads its cell; an absorbed one reassembles ⟨key, value⟩
    through the index and the dynamic fetch (ft_view's expression, composed over D).
    The seam the RMAP plan recorded: scoped constraints read through the VIEW."""
    from . import ast
    table = partition.get(ft, ft)
    if table == ft:
        return ast.FetchPop(ft)
    col = 2 + table_columns(partition, table).index(ft)
    key = _S(_COMP, _1, _1)
    hole = _S(_CONST, A("#"))
    name = _S(_COMP, A("cellkey"), _S(_CONS, _S(_CONST, A(table)), key))
    row = _S(_COMP, ast.DynFetch(), _S(_CONS, name, _2))
    # an index entry whose entity cell is absent contributes a hole (outer-join style),
    # which the filter drops — never ⊥, since validate runs over arbitrary D
    pair = _S(_COND, _S(_COMP, A("atom"), row), _S(_CONS, key, hole),
              _S(_CONS, key, _S(_COMP, A(col), row)))
    nonhole = _S(_COMP, A("not"), _EQ, _S(_CONS, A(2), hole))
    inner = _S(_COMP, T.Filter(nonhole), _S(_ALPHA, pair), _DISTR)
    return _S(_COMP, inner, _S(_CONS, ast.FetchPop(table), _ID))


def row_validate(D, ft, partition):
    """Step 5's constraint mapping (Halpin §10.3): schema constraints move with the
    partitioned layout. A value constraint on an absorbed fact type's value player
    checks the ROW's column on the routed write — the named vc object (already a
    definition, resolved by ρ in-step) applied to the singleton ⟨row[col]⟩, skipped
    while the column is a # hole (fresh rows), the flag alethic per the constraint's
    modality. None when nothing maps."""
    from .lam import PHI
    table = partition.get(ft, ft)
    if table == ft:
        return None
    col = 2 + table_columns(partition, table).index(ft)
    players = [r[3] for r in _pop_rows(D, "role") if len(r) >= 4 and r[1] == ft]
    vcs = {r[0]: r for r in _pop_rows(D, "valueConstraint") if len(r) >= 3}
    hits = [vcs[p] for p in players if p in vcs]
    if not hits:
        return None
    vt, _spec, modality = hits[0][0], hits[0][1], hits[0][2]
    vcell = _S(_COMP, A(col), _1)
    is_hole = _S(_COMP, _EQ, _S(_CONS, vcell, _S(_CONST, A("#"))))
    V = _S(_COND, is_hole, _S(_CONST, PHI),
           _S(_COMP, A(vt + "_vc"), _S(_CONS, _S(_CONS, vcell))))
    flag = _S(_COMP, A("not"), A("null"), V) if modality == "alethic" else _S(_CONST, A("F"))
    return _S(_CONS, _1, V, flag)


def facts_about(D, entity):
    """Every fact mentioning `entity`, VERBALIZED: scan the flat fact cells for rows
    containing it and render each through its fact type's reading template (Prop.
    spec's verbalize direction, applied to instances). Returns (ft, row, sentence)."""
    from .lam import from_lam
    readings = {f[0]: f[1] for f in _pop_rows(D, "factType") if len(f) >= 2}
    players = {}
    for r in _pop_rows(D, "role"):
        if len(r) >= 4:
            players.setdefault(r[1], {})[r[2]] = r[3]
    out = []
    for c in from_lam(D):
        if not (isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"):
            continue
        rows = c[2]
        if not (isinstance(rows, tuple) and all(
                isinstance(r, tuple) and all(not isinstance(x, tuple) for x in r)
                for r in rows)):
            continue
        template = readings.get(c[1])
        for r in rows:
            if entity in r:
                if template and template.count("{") == len(r):
                    # NORMA instance verbalization keeps the role player's type name
                    filled = [f"{players.get(c[1], {}).get(i + 1, '')} '{v}'".strip()
                              for i, v in enumerate(r)]
                    sentence = template.format(*filled) + "."
                else:
                    sentence = f"{c[1]}{r}"
                out.append((c[1], r, sentence))
    return out


def describe(D, noun):
    """What the system can say about a noun, from its own M-facts (a read view in the
    ft_view style): kind, supertypes and subtypes, the fact types it plays roles in
    (with their reading templates and this noun's position), reference mode, the
    machine governing it (if any), and federation provenance."""
    readings = {f[0]: f[1] for f in _pop_rows(D, "factType") if len(f) >= 2}
    roles = [(r[1], r[2], readings.get(r[1], ""))
             for r in _pop_rows(D, "role") if len(r) >= 4 and r[3] == noun]
    return {
        "noun": noun,
        "kind": sorted({r[1] for r in _pop_rows(D, "instanceOf")
                        if len(r) >= 2 and r[0] == noun}),
        "supertypes": sorted({b for (a, b) in
                              (r[:2] for r in _pop_rows(D, "subtype") if len(r) >= 2)
                              if a == noun}),
        "subtypes": sorted({a for (a, b) in
                            (r[:2] for r in _pop_rows(D, "subtype") if len(r) >= 2)
                            if b == noun}),
        "roles": sorted(roles),
        "ref_mode": sorted({r[1] for r in _pop_rows(D, "refMode")
                            if len(r) >= 2 and r[0] == noun}),
        "machine": machine_for(D, noun),
        "federated_from": sorted({r[1] for r in _pop_rows(D, "federatedFrom")
                                  if len(r) >= 2 and r[0] == noun}),
    }


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


