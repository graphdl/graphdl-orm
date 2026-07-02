"""CSDP and RMAP as state machines.

"A state machine is itself a set of facts (a status, its transitions, and the
trigger fact type of each), and advancing it is one AST step" (paper, Prop.
onestep). The Conceptual Schema Design Procedure and the Relational Mapping
procedure are themselves ORM state-machine definitions in the substrate: sets of
Transition facts advanced by the same θ₁ step that advances a domain machine
(the Order machine of the paper). So compiling a schema (CSDP) and mapping it to
cells (RMAP) are runs of ordinary machines — self-similar with the workflows they
build.

Grounded in Halpin, *Information Modeling and Relational Databases*: CSDP is the
seven-step procedure of §3.2; Rmap is the two grouping rules of §10.3
(imrdb.txt:27770): (1) a fact type with a compound uniqueness constraint maps to
a separate table; (2) fact types with functional roles attached to the same
object type are grouped into one table, keyed on that object type's identifier.
"""
from .objects import Atom, Seq, PHI
from .reduce import apply
from .theta import Filter, NatJoin, Project
from .ast import Fetch, Store
from . import orm

_S = lambda *xs: Seq(xs)
_A = Atom
_COMP, _CONS, _COND, _WHILE, _INSERT = _A("COMP"), _A("CONS"), _A("COND"), _A("WHILE"), _A("INSERT")
_ALPHA, _DISTR = _A("ALPHA"), _A("distr")
_1, _2, _3 = _A("1"), _A("2"), _A("3")
_EQ, _AND, _NOT, _NULL, _TL = _A("eq"), _A("and"), _A("not"), _A("null"), _A("tl")
_comp = lambda *fs: _S(_COMP, *fs)
_cons = lambda *fs: _S(_CONS, *fs)


# ============================== the engine (Prop. onestep) ====================
# A machine's transitions are the relation T of ⟨fromStatus, trigger, toStatus⟩
# tuples. advance:⟨T, s, e⟩ = the toStatus of the transition leaving s on trigger e,
# or s itself if none — one AST step, entirely in Codd's θ₁ over the machine facts.
_key = _cons(_2, _3)                                           # ⟨s, e⟩ from ⟨T, s, e⟩
_rows = _comp(_DISTR, _cons(_1, _key))                        # ⟨T,s,e⟩ → ⟨⟨r, ⟨s,e⟩⟩ …⟩
_matches = _comp(_AND, _cons(                                 # on ⟨r, ⟨s,e⟩⟩: r.from=s ∧ r.trig=e
    _comp(_EQ, _cons(_comp(_1, _1), _comp(_1, _2))),
    _comp(_EQ, _cons(_comp(_2, _1), _comp(_2, _2)))))
_tos = _comp(_S(_ALPHA, _comp(_3, _1)), Filter(_matches), _rows)   # ⟨T,s,e⟩ → ⟨matching toStatuses⟩
advance = _S(_COND, _comp(_NOT, _NULL, _tos), _comp(_1, _tos), _2)  # some transition → its to ; else stay s

# replay:⟨T, s0, events⟩ = foldl advance — thread the status through the events. The
# machine's foldl is Backus's while (paper Prop. onestep: reconstruction for audit).
_more = _comp(_NOT, _NULL, _3)                                # events remain?
_step = _cons(_1, _comp(advance, _cons(_1, _2, _comp(_1, _3))), _comp(_TL, _3))  # ⟨T, advance:⟨T,s,1:E⟩, tl:E⟩
replay = _comp(_2, _S(_WHILE, _more, _step))                 # run to the empty event list, return the status


def transition_relation(transitions):
    """A machine's ⟨fromStatus, trigger, toStatus⟩ relation (a list of triples)."""
    return _S(*[_S(_A(f), _A(t), _A(to)) for (f, t, to) in transitions])


def step(transitions, status, event):
    """Advance a machine one step (Prop. onestep): the AST step over its transitions."""
    return apply(advance, _S(transition_relation(transitions), _A(status), _A(event)))


def run_machine(transitions, initial, events):
    """Replay a machine over an event sequence (foldl transition — migration / audit)."""
    return apply(replay, _S(transition_relation(transitions), _A(initial),
                            _S(*[_A(e) for e in events])))


def replay_ordered(transitions, initial, timestamped_events):
    """machine(s₀, E) = foldl transition s₀ (order_τ E) — the reconstruction orders events
    by their occurrence timestamp τ before folding (Prop. onestep), whereas the *live* step
    takes events in arrival order. `timestamped_events` are ⟨τ, event⟩ pairs."""
    ordered = [e for _tau, e in sorted(timestamped_events)]      # order_τ E
    return run_machine(transitions, initial, ordered)


# ================= a machine AS a set of facts (Def. state machine) ===========
_fact = lambda ft, a, b: _S(_A(ft), _A(a), _A(b))            # a tagged fact ⟨factType, a, b⟩


def machine_facts(name, noun, initial, transitions):
    """A state machine as its population of facts — the SMD, its initial status, and
    the Transition facts (`is from`, `is to`, `is triggered by`) — using the ORM
    state-machine fact types of orm.py. This IS the machine (the paper's §2)."""
    facts = [_fact("SMD is for Noun", name, noun), _fact("Status is initial", initial, name)]
    for i, (frm, trig, to) in enumerate(transitions):
        t = "{0}#{1}".format(name, i)
        facts += [_fact("Transition is from", t, frm),
                  _fact("Transition is to", t, to),
                  _fact("Transition triggered by", t, trig)]
    return _S(*facts)


def _relation(ft_name, sm_facts):
    """The untagged relation of fact type `ft_name` from a fact population (θ₁)."""
    tag_is = _comp(_EQ, _cons(_1, _S(_A("CONST"), _A(ft_name))))
    return apply(_comp(_S(_ALPHA, _TL), Filter(tag_is)), sm_facts)


def transition_table(sm_facts):
    """Recover ⟨fromStatus, trigger, toStatus⟩ from a machine's facts by θ₁: natural-
    join the `is from`, `is triggered by`, and `is to` facts on the Transition, then
    project. The transitions ARE facts; advancing is a θ₁ query over them."""
    frm = _relation("Transition is from", sm_facts)         # ⟨t, from⟩
    trg = _relation("Transition triggered by", sm_facts)    # ⟨t, trig⟩
    to = _relation("Transition is to", sm_facts)            # ⟨t, to⟩
    joined = apply(_S(_INSERT, NatJoin(1)), _S(frm, trg, to))    # ⟨t, from, trig, to⟩
    return apply(Project([2, 3, 4]), joined)                # ⟨from, trig, to⟩


# ================= CSDP: the design procedure as a state machine ==============
# Halpin §3.2 — the seven CSDP steps are the machine's statuses; each step is the
# transition that performs it, triggered by that step's action. Running the machine
# over the reading/action stream advances the schema through the whole procedure.
CSDP = [
    ("Start",                   "verbalize examples",    "Elementary Facts"),         # Step 1
    ("Elementary Facts",        "draw fact types",       "Fact Types Drawn"),         # Step 2
    ("Fact Types Drawn",        "trim schema",           "Schema Trimmed"),           # Step 3
    ("Schema Trimmed",          "add uniqueness",        "Uniqueness Added"),         # Step 4
    ("Uniqueness Added",        "add mandatory",         "Mandatory Added"),          # Step 5
    ("Mandatory Added",         "add value/set/subtype", "Value Constraints Added"),  # Step 6
    ("Value Constraints Added", "final checks",          "Finalized"),                # Step 7
]
CSDP_INITIAL = "Start"
CSDP_EVENTS = [t for (_f, t, _to) in CSDP]                  # the seven step actions, in order

csdp_machine = lambda: machine_facts("CSDP", "Schema", CSDP_INITIAL, CSDP)


# ================= RMAP: the relational mapping as a state machine ============
# Halpin §10.3 — the central Rmap steps are the machine's phases; a fact type moves
# from its elementary form to a mapped table scheme (the paper's per-entity cell).
RMAP = [
    ("Elementary",         "group",           "Grouped"),          # apply the two grouping rules
    ("Grouped",            "underline keys",  "Keyed"),            # primary key from the uniqueness
    ("Keyed",              "mark optional",   "Optional Marked"),  # mandatory → NOT NULL; optional → nullable
    ("Optional Marked",    "map constraints", "Constraints Mapped"),  # subset, value list, …
    ("Constraints Mapped", "map derivations", "Mapped"),           # derivation rules mapped down
]
RMAP_INITIAL = "Elementary"
RMAP_EVENTS = [t for (_f, t, _to) in RMAP]

rmap_machine = lambda: machine_facts("Rmap", "Fact Type", RMAP_INITIAL, RMAP)


def rmap_group(fact_type, uniqueness_constraints):
    """Rmap's two grouping rules (Halpin §10.3, imrdb.txt:27770):
      1. a fact type with a COMPOUND uniqueness constraint (spanning ≥2 of its roles)
         maps to a SEPARATE table (m:n binaries, n-aries) — PK = that constraint;
      2. otherwise its functional role (a single-role uniqueness) groups it into the
         table of the object type it is attached to, keyed on that type's identifier.
    Returns ⟨placement, anchor⟩: ⟨"separate", factType⟩ or ⟨"grouped", objectType⟩ —
    the target cell into which RMAP absorbs the fact type."""
    ft_name = fact_type.xs[1].v
    own = [uc for uc in uniqueness_constraints
           if all(orm.role_fact_type(r) == ft_name for r in orm.c_roles(uc).xs[0].xs)]
    for uc in own:                                          # rule 1: a compound UC ⇒ its own table
        if len(orm.c_roles(uc).xs[0].xs) >= 2:
            return _S(_A("separate"), _A(ft_name))
    for uc in own:                                          # rule 2: a functional role ⇒ group on its player
        if len(orm.c_roles(uc).xs[0].xs) == 1:
            return _S(_A("grouped"), _A(orm.role_player(orm.c_roles(uc).xs[0].xs[0])))
    return _S(_A("separate"), _A(ft_name))                 # no uniqueness ⇒ its own (existential) table


def rmap(fact_types, uniqueness_constraints):
    """Map a schema's fact types to relational cells (Halpin §10.3; paper Sec. Cells):
    group each fact type by rmap_group, then build one cell per anchor holding the fact
    types grouped there. The result is the paper's D — "a sequence of cells, each the
    3NF row of facts depending on its key" — the output of running RMAP over the schema."""
    groups = {}
    for ft in fact_types:
        _, anchor = (x.v for x in rmap_group(ft, uniqueness_constraints).xs)
        groups.setdefault(anchor, []).append(ft.xs[1].v)
    return _S(*[_S(_A("CELL"), _A(anchor), _S(*[_A(n) for n in fts]))
                for anchor, fts in groups.items()])         # ⟨CELL, table, ⟨grouped fact types⟩⟩


# ==================== CSDP drives compilation (schema via CSDP) ===============
def _csdp_step_of(parsed):
    """The CSDP step that introduces a reading (Halpin §3.2): object/fact-type
    declarations are drawn at step 2; uniqueness at step 4; mandatory at step 5."""
    kind = parsed[0]
    if kind in ("entity_type", "value_type", "fact_type"):
        return "draw fact types"                            # step 2
    if kind == "constraint":
        return "add uniqueness" if parsed[1] in ("exactly_one", "at_most_one") else "add mandatory"
    return "final checks"                                   # step 7


def csdp_compile(readings):
    """Run the CSDP machine over readings, compiling the schema step by step — the
    procedure-machine *drives* the compilation (paper: "schema via CSDP"). Object and
    fact-type readings are drawn (step 2), then uniqueness (step 4) and mandatory
    (step 5) constraints are added, advancing the machine to Finalized. Returns
    ⟨finalStatus, schema⟩ with schema the compiled FFP objects, in CSDP order."""
    from . import forml
    by_step = {}
    for r in readings:
        by_step.setdefault(_csdp_step_of(forml.parse(r)), []).append(r)
    status, schema, fact_types = CSDP_INITIAL, [], {}
    for (_frm, action, _to) in CSDP:                        # canonical CSDP step order
        for r in by_step.get(action, []):
            parsed = forml.parse(r)
            schema.append(forml.compile(parsed, {"fact_types": fact_types}))
            if parsed[0] == "fact_type":
                fact_types[forml._ft_key(parsed[1], parsed[3], parsed[2])] = schema[-1]
        status = step(CSDP, status, action).v               # the CSDP machine advances one step
    return _A(status), schema


# =================== RMAP produces the live cells (cells via RMAP) ============
def rmap_cells(fact_types, uniqueness_constraints):
    """RMAP maps the schema to the live state D (paper Sec. Cells): one cell per table,
    each holding that table's population (initially empty). Returns ⟨D, groupings⟩,
    where `groupings` maps each fact type to the table (cell) it routes into."""
    groupings, tables = {}, []
    for ft in fact_types:
        _, anchor = (x.v for x in rmap_group(ft, uniqueness_constraints).xs)
        groupings[ft.xs[1].v] = anchor
        if anchor not in tables:
            tables.append(anchor)
    D = _S(*[_S(_A("CELL"), _A(t), PHI) for t in tables])   # a cell per table, empty population
    return D, groupings


def store_fact(fact, D, groupings):
    """Store a fact into its RMAP cell — routed on the fact type's table, which is the
    eq. sys dispatch over cells (↑table(fact):D). The addressed cell's population gains
    the fact; the other cells are untouched (Def. cell isolation)."""
    table = groupings[fact.xs[0].v]                         # the fact type's RMAP table
    newpop = _S(fact, *apply(Fetch(table), D).xs)          # prepend the fact to that cell's population
    return apply(Store(table), _S(newpop, D))


def rmap_system(fact_types, constraints):
    """RMAP builds the *running* system (folds the cells into the AST step): one cell per
    table (in the returned D), and one entity handler per table — a compiled symbol
    '@table' installed in DEFS (the global definition cells), wired with that table's
    constraints, operating over that table's cell. Returns ⟨D, route⟩: D the initial
    per-entity cell state, and route(fact) the handler symbol addressing a fact's table
    (the eq. sys address). A create is then dispatch(route(fact), fact, D) — one eq. sys
    step that validates and commits per cell."""
    from .defs import define
    from .system import validate_of
    from .ast import build_system
    groupings, tables = {}, []
    for ft in fact_types:
        _, anchor = (x.v for x in rmap_group(ft, constraints).xs)
        groupings[ft.xs[1].v] = anchor
        if anchor not in tables:
            tables.append(anchor)
    by_table = {t: [] for t in tables}                     # each table's own constraints
    for c in constraints:
        ftn = orm.role_fact_type(orm.c_roles(c).xs[0].xs[0])
        if ftn in groupings:
            by_table[groupings[ftn]].append(c)
    for t in tables:                                       # install the entity handlers in DEFS
        define("@" + t, build_system(validate_of(by_table[t]), cell=t))
    D = _S(*[_S(_A("CELL"), _A(t), PHI) for t in tables])
    return D, (lambda fact: "@" + groupings[fact.xs[0].v])


def rmap_store(fact_types, constraints):
    """RMAP builds a *self-contained* store (Prop. tenant): the returned D holds both the
    population cells (one per table) AND the handler cells (@table, holding that entity's
    compiled handler over its cell). Because the handlers live in D itself, the store is
    addressable only from itself — a sibling tenant's store names no @table of this one, so
    its address is unaddressable. dispatch_threaded(route(fact), fact, D) is one eq. sys step."""
    from .system import validate_of
    from .ast import build_system
    groupings, tables = {}, []
    for ft in fact_types:
        _, anchor = (x.v for x in rmap_group(ft, constraints).xs)
        groupings[ft.xs[1].v] = anchor
        if anchor not in tables:
            tables.append(anchor)
    by_table = {t: [] for t in tables}
    for c in constraints:
        ftn = orm.role_fact_type(orm.c_roles(c).xs[0].xs[0])
        if ftn in groupings:
            by_table[groupings[ftn]].append(c)
    cells = [_S(_A("CELL"), _A(t), PHI) for t in tables]                    # the population cells
    cells += [_S(_A("CELL"), _A("@" + t), build_system(validate_of(by_table[t]), cell=t))
              for t in tables]                                             # the handler cells (in D)
    return _S(*cells), (lambda fact: "@" + groupings[fact.xs[0].v])


# ==================== HATEOAS: links generated from P and S (Thm. hateoas) ====
# links(e) = nav(e) ∪ transitions(status(e)) — a θ₁ expression over the population and
# the schema, complete: it contains every valid control and only valid controls. This is
# the paper's "novel step": the affordances are a function of P and S, not hand-written.
def transitions_of(status, transitions):
    """transitions(status(e)) — the controls (⟨trigger, toStatus⟩) of the outgoing
    transitions from `status`, a θ₁ Project∘Filter over the machine's transition relation."""
    from_is = _comp(_EQ, _cons(_1, _S(_A("CONST"), _A(status))))       # transition leaves `status`?
    return apply(_comp(Project([2, 3]), Filter(from_is)), transition_relation(transitions))


def nav_of(entity, P):
    """nav(e) — the facts of P that mention `entity` (a θ₁ restriction over the population;
    the peer and child controls reachable through e's fact types)."""
    mentions = _comp(_NOT, _NULL, Filter(_EQ), _A("distl"),
                     _cons(_S(_A("CONST"), _A(entity)), _A("tl")))     # entity ∈ tl(fact)?
    return apply(Filter(mentions), P)


def links(entity, status, P, transitions):
    """links(e) = nav(e) ∪ transitions(status(e)) (Thm. hateoas): the complete, current set
    of valid controls for entity `e`, generated as a θ₁ expression over P and the schema —
    the value emit adds to the representation (paper Def. Command: emit builds ⟨P, V, links⟩)."""
    return _S(nav_of(entity, P), transitions_of(status, transitions))
