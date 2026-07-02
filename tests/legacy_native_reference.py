"""The substrate reduces (metacomposition + Backus's combining forms), and the
AREST system function `create` (eq. create) reduces as one composition."""
import pyarest                                   # loads substrate + system into DEFS
from pyarest.objects import Atom, Seq, PHI
from pyarest.reduce import apply


def A(v):
    return Atom(v)


def S(*xs):
    return Seq(xs)


# --- substrate: metacomposition and the combining forms ---
def test_selector_composition():
    assert apply(A("2"), S(A("a"), A("b"), A("c"))) == A("b")     # 2 ≡ 1∘tl
    assert apply(A("3"), S(A("a"), A("b"), A("c"))) == A("c")     # 3 ≡ 1∘tl∘tl


def test_construction():
    # [1, tl] : ⟨a,b,c⟩ = ⟨a, ⟨b,c⟩⟩
    f = S(A("CONS"), A("1"), A("tl"))
    assert apply(f, S(A("a"), A("b"), A("c"))) == S(A("a"), S(A("b"), A("c")))


def test_condition():
    # (null → φ̄ ; 1) : x
    f = S(A("COND"), A("null"), S(A("CONST"), PHI), A("1"))
    assert apply(f, S(A("a"), A("b"))) == A("a")     # not null → 1:x = a
    assert apply(f, PHI) == PHI                       # null → φ


def test_apply_to_all():
    # α tl : ⟨⟨a,b⟩,⟨c,d⟩⟩ = ⟨⟨b⟩,⟨d⟩⟩
    f = S(A("ALPHA"), A("tl"))
    assert apply(f, S(S(A("a"), A("b")), S(A("c"), A("d")))) == S(S(A("b")), S(A("d")))


def test_insert():
    # /+ : ⟨1,2,3⟩ = 6      (Backus's insert over the primitive +)
    f = S(A("INSERT"), A("+"))
    assert apply(f, S(A(1), A(2), A(3))) == A(6)


def test_condition_requires_a_boolean():
    # (id → 1 ; tl) : ⟨a,b⟩ — id:x = ⟨a,b⟩ ∉ {T,F}, so Backus gives ⊥, not tl:x
    from pyarest.objects import BOT
    f = S(A("COND"), A("id"), A("1"), A("tl"))
    assert apply(f, S(A("a"), A("b"))) is BOT


# --- Codd θ₁ over FILE: restriction as an FFP object (no query language) ---
def test_restriction_is_a_filter_ffp_object():
    from pyarest.theta import Filter
    # Filter(atom) : ⟨a, ⟨x,y⟩, c⟩ = ⟨a, c⟩   — keep the atoms, drop the sequence
    f = Filter(A("atom"))
    assert apply(f, S(A("a"), S(A("x"), A("y")), A("c"))) == S(A("a"), A("c"))


def test_projection_over_typed_values():
    from pyarest.theta import Project
    # atoms carry their raw data type for storage; projection dedups by value+type
    alice, bob = Atom("alice", "String"), Atom("bob", "String")
    R = S(S(alice, Atom(1, "Unsigned Integer")),
          S(bob, Atom(2, "Unsigned Integer")),
          S(alice, Atom(3, "Unsigned Integer")))   # ⟨String, Unsigned Integer⟩ tuples
    out = apply(Project([1]), R)                    # project column 1, deduped
    assert set(out.xs) == {S(alice), S(bob)}        # alice's two orders collapse to one


def test_data_type_is_part_of_identity():
    # an atom stores value + raw data type — a NORMA PortableDataType name, not a
    # value-type name ("Variable Length Text", not "String" or "OrderId")
    assert Atom("1", "Variable Length Text") != Atom(1, "Unsigned Integer")   # different data types
    assert Atom(1, "Unsigned Integer") == Atom(1, "Unsigned Integer")


def test_cat_is_a_derived_ffp_object():
    # cat ≡ (/apndl)∘apndr — built from Backus primitives, not a host primitive
    assert apply(A("cat"), S(S(A("a"), A("b")), S(A("c"), A("d")))) == S(A("a"), A("b"), A("c"), A("d"))


def test_natural_join():
    from pyarest.theta import NatJoin
    o1, o2 = A("o1"), A("o2")
    R = S(S(o1, A("alice")), S(o2, A("bob")))            # ⟨order, cust⟩
    St = S(S(o1, A("shipped")), S(o2, A("pending")))     # ⟨order, status⟩
    out = apply(NatJoin(1), S(R, St))                    # join on order (R.1 = S.1)
    assert out == S(S(o1, A("alice"), A("shipped")), S(o2, A("bob"), A("pending")))


def test_tie():
    from pyarest.theta import Tie
    # γ: keep tuples whose first = last, drop the last (degree 3 → 2)
    R = S(S(A("a"), A("b"), A("a")), S(A("c"), A("d"), A("e")), S(A("x"), A("y"), A("x")))
    assert apply(Tie, R) == S(S(A("a"), A("b")), S(A("x"), A("y")))


# --- ORM constraint primitives: V_c = (ρc):X as FFP objects over θ₁ ---
def test_uniqueness_constraint():
    from pyarest.orm import Unique
    # role 1 (the key) is not unique: 'a' appears twice
    X = S(S(A("a"), A("1")), S(A("b"), A("2")), S(A("a"), A("3")))
    Vc = apply(Unique([1]), X)
    assert set(Vc.xs) == {S(A("a"), A("1")), S(A("a"), A("3"))}   # the offending tuples
    # a unique relation offends nothing
    assert apply(Unique([1]), S(S(A("a"), A("1")), S(A("b"), A("2")))) == PHI


def test_mandatory_constraint():
    from pyarest.orm import Mandatory
    pop = S(A("o1"), A("o2"), A("o3"))                 # every Order...
    X = S(S(A("o1"), A("shipped")), S(A("o2"), A("pending")))   # ...but o3 plays no role 1
    assert apply(Mandatory(1), S(pop, X)) == S(A("o3"))         # o3 violates mandatory


def test_fact_type_has_role_objects():
    from pyarest.orm import fact_type, ft_roles, role_player, role_position, role_fact_type
    ft = fact_type("places", ["Order", "Customer"], "is placed by")
    r0, r1 = ft_roles(ft)                       # first-class Role objects
    assert role_player(r0) == "Order" and role_position(r0) == 0 and role_fact_type(r0) == "places"
    assert role_player(r1) == "Customer" and role_position(r1) == 1


def test_external_uniqueness_for_identity():
    from pyarest.orm import (fact_type, ft_roles, uniqueness, preferred_identifier,
                             entity_type, c_extra, c_evaluator)
    # Room is identified by Building + RoomNumber — roles across two fact types
    in_building = fact_type("Room is in Building", ["Room", "Building"], "is in")
    has_number = fact_type("Room has RoomNumber", ["Room", "RoomNumber"], "has")
    uc = uniqueness([ft_roles(in_building)[1], ft_roles(has_number)[1]])   # spans ⇒ external
    assert c_extra(uc)[0] == A("external")
    # two rooms sharing ⟨Building, RoomNumber⟩ violate identity (join is θ₁)
    vc = c_evaluator(uc)
    pop_building = S(S(A("r1"), A("B1")), S(A("r2"), A("B1")))
    pop_number = S(S(A("r1"), A(101)), S(A("r2"), A(101)))
    assert set(apply(vc, S(pop_building, pop_number)).xs) == {
        S(A("r1"), A("B1"), A(101)), S(A("r2"), A("B1"), A(101))}
    # identity IS that external uniqueness constraint
    assert entity_type("Room", preferred_identifier(uc)).xs[2] == uc


def test_set_comparison_constraints():
    from pyarest.orm import Subset, Exclusion, Equality
    P = S(A("a"), A("b"), A("c"))
    Q = S(A("b"), A("c"), A("d"))
    assert apply(Subset, S(P, Q)) == S(A("a"))                     # A∖B = ⟨a⟩ ⇒ not ⊆
    assert set(apply(Exclusion, S(P, Q)).xs) == {A("b"), A("c")}   # A∩B ⇒ not disjoint
    assert set(apply(Equality, S(P, Q)).xs) == {A("a"), A("d")}    # symmetric difference
    assert apply(Equality, S(S(A("x")), S(A("x")))) == PHI         # equal ⇒ no violation


def test_state_machine_is_a_set_of_facts():
    from pyarest import orm
    from pyarest.theta import Filter
    # the state-machine vocabulary is fact types with first-class roles
    ft = orm.Transition_is_from_Status
    r0, r1 = orm.ft_roles(ft)
    assert orm.role_player(r0) == "Transition"     # role 1 played by Transition
    assert orm.role_player(r1) == "Status"         # role 2 played by Status
    # the Order machine's 'is from' facts, queried by θ₁: transitions from 'In Cart'
    from_facts = S(S(A("place"), A("In Cart")), S(A("ship"), A("Placed")))
    p = S(A("COMP"), A("eq"), S(A("CONS"), A("2"), S(A("CONST"), A("In Cart"))))  # eq∘[2, "In Cart"̄]
    assert apply(Filter(p), from_facts) == S(S(A("place"), A("In Cart")))


def test_value_comparison_constraint():
    from pyarest.orm import ValueComparison
    X = S(S(A(1), A(5)), S(A(3), A(2)))                       # ⟨from, to⟩; require from < to
    assert apply(ValueComparison(1, 2, "lt"), X) == S(S(A(3), A(2)))   # 3 < 2 fails


def test_frequency_constraint():
    from pyarest.orm import Frequency
    X = S(S(A("a"), A("1")), S(A("a"), A("2")), S(A("b"), A("3")))
    assert apply(Frequency([1], 2, 2), X) == S(S(A("b"), A("3")))      # 'b' occurs once, need 2


def test_value_constraint():
    from pyarest.orm import Value
    allowed = S(A("In Cart"), A("Placed"), A("Shipped"))
    X = S(S(A("In Cart")), S(A("Bogus")))
    assert apply(Value(1, allowed), X) == S(S(A("Bogus")))            # Bogus ∉ allowed


def test_cardinality_constraint():
    from pyarest.orm import Cardinality
    X = S(A("a"), A("b"), A("c"))
    assert apply(Cardinality(1, 5), X) == PHI                          # |X|=3 ∈ [1,5] ⇒ no violation
    assert apply(Cardinality(1, 2), X) == S(X)                         # |X|=3 ∉ [1,2] ⇒ ⟨X⟩ offends
    assert apply(Cardinality(1, 2), PHI) == S(PHI)                     # empty pop violates min 1 (F5)


def test_disjunctive_mandatory_is_n_ary():
    from pyarest.orm import mandatory, role, c_evaluator
    roles = [role("F", 1, "T"), role("G", 1, "T"), role("H", 1, "T")]   # 3 constrained roles
    vc = c_evaluator(mandatory(roles, "alethic"))                      # the inclusive-or evaluator
    O = S(A("a"), A("b"), A("c"), A("d"))
    pvs = S(S(A("a")), S(A("b")), S(A("c")))                           # players of each of the 3 roles
    assert apply(vc, S(O, pvs)) == S(A("d"))    # only d plays none of the three (was ⟨c,d⟩ before F3)


def test_unbounded_value_range_admits_everything():
    from pyarest.orm import value, value_range, role, c_evaluator
    vc = c_evaluator(value(role("F has X", 1, "X"), [value_range(None)]))   # both bounds NotSet (F7)
    assert apply(vc, S(S(A("f"), A(5)), S(A("f"), A(999)))) == PHI      # nothing offends


def test_ring_constraints():
    from pyarest.orm import Ring
    assert apply(Ring(1, 2, "Irreflexive"),
                 S(S(A("a"), A("a")), S(A("a"), A("b")))) == S(S(A("a"), A("a")))
    assert set(apply(Ring(1, 2, "Asymmetric"),
                     S(S(A("a"), A("b")), S(A("b"), A("a")))).xs) == {S(A("a"), A("b")), S(A("b"), A("a"))}
    assert apply(Ring(1, 2, "Symmetric"), S(S(A("a"), A("b")))) == S(S(A("a"), A("b")))


def test_closure_ring_constraints():
    from pyarest.orm import Ring
    pr = lambda *ps: S(*[S(A(a), A(b)) for a, b in ps])
    # Transitive: a→b, b→c but no a→c ⇒ the missing composite pair (P∘P)∖P offends
    assert apply(Ring(1, 2, "Transitive"), pr(("a", "b"), ("b", "c"))) == S(S(A("a"), A("c")))
    assert apply(Ring(1, 2, "Transitive"), pr(("a", "b"), ("b", "c"), ("a", "c"))) == PHI
    # Acyclic: a cycle a→b→a is a self-loop in the transitive closure
    assert apply(Ring(1, 2, "Acyclic"), pr(("a", "b"), ("b", "a"))).xs
    assert apply(Ring(1, 2, "Acyclic"), pr(("a", "b"), ("b", "c"))) == PHI
    # Intransitive: a→b, b→c and a direct a→c ⇒ a→c offends
    assert apply(Ring(1, 2, "Intransitive"), pr(("a", "b"), ("b", "c"), ("a", "c"))) == S(S(A("a"), A("c")))
    # PurelyReflexive: only self-loops permitted; Reflexive: every element needs one
    assert apply(Ring(1, 2, "PurelyReflexive"), pr(("a", "a"), ("a", "b"))) == S(S(A("a"), A("b")))
    assert apply(Ring(1, 2, "Reflexive"), pr(("a", "a"), ("a", "b"))) == S(A("b"))


def test_role_centric_constraint_front():
    from pyarest import orm
    from pyarest.orm import fact_type, ft_roles, c_kind, c_modality, c_evaluator
    # mandatory over a Role object (not a position) — carries modality
    ships = fact_type("Customer ships Order", ["Customer", "Order"], "ships")
    m = orm.mandatory([ft_roles(ships)[1]], "deontic")
    assert c_kind(m) == "MandatoryConstraint" and c_modality(m) == "deontic"
    orders, facts = S(A("o1"), A("o2"), A("o3")), S(S(A("c1"), A("o1")), S(A("c2"), A("o2")))
    assert apply(c_evaluator(m), S(orders, facts)) == S(A("o3"))        # o3 unshipped

    # value-comparison over two Role objects
    period = fact_type("has period", ["Start", "End"], "has")
    vc = orm.value_comparison(ft_roles(period)[0], ft_roles(period)[1], "lt")
    assert apply(c_evaluator(vc), S(S(A(1), A(5)), S(A(3), A(2)))) == S(S(A(3), A(2)))

    # subset across two fact types' role sequences
    a = fact_type("A", ["X"], "a")
    b = fact_type("B", ["X"], "b")
    sub = orm.subset([ft_roles(a)[0]], [ft_roles(b)[0]])
    assert apply(c_evaluator(sub), S(S(S(A("p")), S(A("q"))), S(S(A("q")))) ) == S(S(A("p")))


def test_objectification():
    from pyarest.orm import fact_type, objectification
    plays = fact_type("Person plays Sport", ["Person", "Sport"], "plays")
    playing = objectification("Playing", plays)          # objectify the fact type
    assert playing.xs[0] == A("Objectification") and playing.xs[1] == A("Playing")
    assert playing.xs[2] == plays                        # the nested fact type
    from pyarest.orm import c_kind, c_extra
    pid = playing.xs[4]                                  # identity = spanning uniqueness
    assert c_kind(pid) == "UniquenessConstraint" and c_extra(pid)[0] == A("internal")


def test_subtyping():
    from pyarest.orm import subtype_fact
    sf = subtype_fact("Manager", "Employee")             # Manager IsA Employee
    assert sf.xs[0] == A("SubtypeFact") and sf.xs[1] == A("Manager") and sf.xs[2] == A("Employee")
    # every Manager must be an Employee; V = managers absent from employees
    managers, employees = S(A("m1"), A("m2")), S(A("m1"), A("e3"))
    assert apply(sf.xs[4], S(managers, employees)) == S(A("m2"))


def test_role_path_and_derivation():
    from pyarest.orm import path_join, derivation
    # three fact types on Person; a role path joins them on the shared entity
    in_building = S(S(A("alice"), A("B1")), S(A("bob"), A("B2")))
    has_age = S(S(A("alice"), A(30)), S(A("bob"), A(25)))
    likes = S(S(A("alice"), A("red")), S(A("bob"), A("blue")))
    joined = apply(path_join, S(in_building, has_age, likes))
    assert joined == S(S(A("alice"), A("B1"), A(30), A("red")),
                       S(A("bob"), A("B2"), A(25), A("blue")))
    # a derivation rule: derive ⟨Person, Color⟩ from the path (head roles 1 and 4)
    d = derivation([1, 4])
    assert set(apply(d, S(in_building, has_age, likes)).xs) == {
        S(A("alice"), A("red")), S(A("bob"), A("blue"))}


def test_system_runs_as_one_function():
    from pyarest.ast import cell, run, Fetch
    from pyarest.objects import PHI
    D = S(cell("FILE", PHI))                          # AST state: FILE holds an empty population
    fact = S(A("places"), A("order1"), A("alice"))
    result = run(fact, D)                             # μ(SYSTEM:⟨fact, D⟩) = ⟨output, D'⟩
    output, D1 = result.xs[0], result.xs[1]
    assert output.xs[0] == S(fact) and output.xs[1] == PHI   # o = ⟨population, violations⟩
    assert apply(Fetch("FILE"), D1) == S(fact)               # committed (no alethic violation)
    # a second transition threads the state forward
    fact2 = S(A("places"), A("order2"), A("bob"))
    assert apply(Fetch("FILE"), run(fact2, D1).xs[1]) == S(fact2, fact)


def test_compile_forml_readings():
    from pyarest import forml
    from pyarest.orm import ft_roles, role_player
    # object-type declaration compiles to an entity type
    et = forml.compile(forml.parse("Order is an entity type."))
    assert et.xs[0] == A("EntityType") and et.xs[1] == A("Order")
    # an elementary fact-type reading compiles to a fact type with role objects
    S_ = forml.schema(["Order is an entity type.",
                       "Customer is an entity type.",
                       "Order is placed by Customer."])
    ft = S_[2]
    assert ft.xs[0] == A("FactType")
    r0, r1 = ft_roles(ft)
    assert role_player(r0) == "Order" and role_player(r1) == "Customer"


def test_compile_constraint_reading():
    from pyarest import forml
    from pyarest.orm import role_player, c_kind, c_roles
    S_ = forml.schema([
        "Order is an entity type.",
        "Customer is an entity type.",
        "Order is placed by Customer.",
        "Each Order is placed by exactly one Customer.",
    ])
    tag, cs = S_[3]                                    # the constraint reading compiled
    assert tag == "constraints"
    assert {c_kind(c) for c in cs} == {"UniquenessConstraint", "MandatoryConstraint"}
    uc = next(c for c in cs if c_kind(c) == "UniquenessConstraint")
    assert role_player(c_roles(uc).xs[0].xs[0]) == "Order"   # over the Order role, not a position


def test_constraint_verbalization_round_trip():
    from pyarest import forml
    ft = forml.compile(forml.parse("Order is placed by Customer."))
    fts = {"Order is placed by Customer": ft}
    # nf = verbalize∘compile∘parse is the identity on canonical readings (Prop. Spec)
    for reading in ["Each Order is placed by exactly one Customer.",     # mandatory + uniqueness
                    "Each Order is placed by at most one Customer.",     # uniqueness
                    "Each Order is placed by some Customer."]:           # mandatory
        _, cs = forml.compile(forml.parse(reading), {"fact_types": fts})
        assert forml.verbalize(cs, fts) == reading


def test_nf_round_trips_every_family():
    from pyarest import forml
    from pyarest.forml import _ft_key, _ftn_key
    fts = {}
    for r in ["Person is an entity type.", "Sport is an entity type.", "Car is an entity type.",
              "Person plays Sport.", "Person drives Car.", "Person owns Car.",
              "Person is ancestor of Person.", "Meeting is an entity type.", "Meeting has Start and End."]:
        p = forml.parse(r); o = forml.compile(p, {"fact_types": fts})
        if p[0] == "fact_type":
            fts[_ft_key(p[1], p[3], p[2])] = o
        elif p[0] == "fact_type_n":
            fts[_ftn_key(p[1], p[2], p[3])] = o
    # verbalize∘compile∘parse is the identity across every constraint family (total nf, Prop. Spec)
    for reading in ["Each Person plays at most 3 Sport.",                 # frequency
                    "There are at least 2 Person.",                      # cardinality
                    "Person is ancestor of Person is acyclic.",          # ring (closure)
                    "If Person drives Car then Person owns Car.",        # subset
                    "Person drives Car if and only if Person owns Car.",  # equality
                    "For each Meeting, Start is before End.",            # value comparison
                    "The possible values of Sport are {Tennis, Chess}."]:  # value
        _, cs = forml.compile(forml.parse(reading), {"fact_types": fts})
        assert forml.verbalize(cs, fts) == reading


# --- system: create is one composed FFP function (eq. create) ---
def test_create_is_one_composed_function():
    fact = S(A("places"), A("order1"), A("alice"))
    out = apply(A("create"), S(fact, PHI))
    assert out == S(fact)                             # emit∘validate∘derive∘resolve


def test_create_adds_to_population():
    f1 = S(A("places"), A("order1"), A("alice"))
    f2 = S(A("places"), A("order2"), A("bob"))
    out = apply(A("create"), S(f2, S(f1)))
    assert out == S(f2, f1)


def test_system_enforces_constraints_alethic_refuses_deontic_warns():
    # Theorem (completeness of state transfer): validate = ⋃_c (ρc):P, and the AST step
    # commits P'' to FILE iff V has no alethic violation, else leaves D unchanged (eq. create).
    from pyarest.ast import cell, run, Fetch
    from pyarest.orm import fact_type, ft_roles, uniqueness
    from pyarest.system import validate_of
    FT = "Order is placed by Customer"
    ft = fact_type(FT, ["Order", "Customer"], "is placed by")
    D = S(cell("FILE", S(S(A(FT), A("o1"), A("alice")))))      # o1 already placed by alice
    dup = S(A(FT), A("o1"), A("bob"))                          # placing o1 again ⇒ Order not unique

    # ALETHIC uniqueness: the duplicate is refused — V ≠ φ and D is unchanged
    out, D2 = run(dup, D, validate_of([uniqueness([ft_roles(ft)[0]], "alethic")])).xs
    assert out.xs[1] != PHI                                    # the violation is reported
    assert apply(Fetch("FILE"), D2) == S(S(A(FT), A("o1"), A("alice")))   # commit REFUSED (D unchanged)

    # a clean fact commits
    out3, D3 = run(S(A(FT), A("o2"), A("bob")), D,
                   validate_of([uniqueness([ft_roles(ft)[0]], "alethic")])).xs
    assert out3.xs[1] == PHI and len(apply(Fetch("FILE"), D3).xs) == 2     # committed: o1 + o2

    # the SAME violation under a DEONTIC constraint warns and commits
    out4, D4 = run(dup, D, validate_of([uniqueness([ft_roles(ft)[0]], "deontic")])).xs
    assert out4.xs[1] != PHI                                   # still reported (a warning)
    assert len(apply(Fetch("FILE"), D4).xs) == 2              # but COMMITTED (deontic warns, commits)


def test_resolve_mints_auto_counter_identifiers():
    from pyarest.system import resolve_minting, mint_next
    assert apply(mint_next(1), PHI) == A(1)                       # empty ⇒ first id is 1
    r = resolve_minting(1)                                        # id at column 1
    P1 = apply(r, S(S(A("alice")), PHI))                         # create → ⟨1, alice⟩
    P2 = apply(r, S(S(A("bob")), P1))                            # create → ⟨2, bob⟩ (fresh surrogate/step)
    assert P1.xs[0].xs[0] == A(1) and P2.xs[0].xs[0] == A(2)


def test_derive_reaches_least_fixed_point():
    from pyarest.system import derive_of
    rule = S(A("ALPHA"), S(A("CONS"), A("2"), A("1")))           # rule(P) = ⟨reverse each pair⟩
    out = apply(derive_of([rule]), S(S(A("a"), A("b")), S(A("b"), A("c"))))
    assert set(out.xs) == {S(A("a"), A("b")), S(A("b"), A("c")),  # F_S reaches the symmetric closure
                           S(A("b"), A("a")), S(A("c"), A("b"))}
    assert apply(derive_of([]), S(S(A("a"), A("b")))) == S(S(A("a"), A("b")))   # no rules ⇒ identity


def test_enumerable_boundary_is_the_registered_definitions():
    from pyarest.defs import boundary, DEFS
    surf = boundary()                                            # Filter(eq∘[s_origin, registered̄]):DEFS
    assert all(r.xs[3] == A("registered") for r in surf.xs)      # only registered defs
    assert len(surf.xs) == sum(1 for d in DEFS.values() if d.origin == "registered")
    names = {r.xs[0].v for r in surf.xs}
    assert "eq" in names and "apply" in names                    # primitives are on the informal surface
    assert "cat" not in names and "create" not in names          # compiled objects are decidable, not


def test_three_valued_population_truth():
    from pyarest.pop import truth_of, negate
    FT = "Order is placed by Customer"
    g = S(A(FT), A("o1"), A("alice"))
    P = S(g, negate(S(A(FT), A("o2"), A("bob"))))                # o1 asserted true; o2 asserted false
    assert truth_of(g, P) == "true"                              # g ∈ P
    assert truth_of(S(A(FT), A("o2"), A("bob")), P) == "false"   # ¬g ∈ P (asserted, not inferred)
    other = S(A(FT), A("o3"), A("carol"))
    assert truth_of(other, P) == "unknown"                      # neither (open world)
    assert truth_of(other, P, closed_world=True) == "false"     # CWA collapses unknown to false


def test_replay_orders_by_timestamp():
    from pyarest import machine as m
    T = [("A", "x", "B"), ("B", "y", "C")]
    # τ-ordered (y@1 then x@2): y no-ops at A, then x → B
    assert m.replay_ordered(T, "A", [(2, "x"), (1, "y")]) == A("B")
    # arrival order (x then y): x → B, y → C — a different result, so ordering matters (Prop. onestep)
    assert m.run_machine(T, "A", ["x", "y"]) == A("C")


def test_entity_handler_mints_and_derives_in_running_pipeline():
    from pyarest.ast import build_system, cell, Fetch
    from pyarest.system import validate_of, resolve_minting, derive_of
    reverse = S(A("ALPHA"), S(A("CONS"), A("2"), A("1")))
    # a handler whose resolve mints an auto-counter id and whose derive is lfp(F_S) of one rule
    h = build_system(validate_of([]), cell="R",
                     resolve_obj=resolve_minting(1), derive_obj=derive_of([reverse]))
    _, D1 = apply(h, S(S(A("a")), S(cell("R", PHI)))).xs         # create ⟨a⟩ — one eq. create step
    P = apply(Fetch("R"), D1)
    assert S(A(1), A("a")) in P.xs                              # resolve minted the identifier 1
    assert S(A("a"), A(1)) in P.xs                              # derive (lfp) added the reverse


def test_emit_carries_hateoas_links_over_p():
    from pyarest.ast import build_system, cell
    from pyarest.system import validate_of
    FT = "Order is placed by Customer"
    h = build_system(validate_of([]), cell="Order", links_key=2)   # emit builds o = ⟨P'', V, links⟩
    D = S(cell("Order", S(S(A(FT), A("o1"), A("shipped")), S(A(FT), A("o2"), A("zoe")))))
    out, _ = apply(h, S(S(A(FT), A("o1"), A("alice")), D)).xs
    P, V, links = out.xs                                            # three parts now, not two
    assert len(links.xs) == 2 and all(f.xs[1] == A("o1") for f in links.xs)   # only the affected entity's facts


# --- CSDP and RMAP as state machines (Prop. onestep; Halpin §3.2, §10.3) ---
def test_state_machine_engine_is_the_ast_step():
    from pyarest import machine as m
    # one AST step: an event fires the matching transition (Prop. onestep)
    assert m.step(m.CSDP, "Schema Trimmed", "add uniqueness") == A("Uniqueness Added")
    # an irrelevant event leaves the status unchanged
    assert m.step(m.CSDP, "Schema Trimmed", "add mandatory") == A("Schema Trimmed")


def test_csdp_is_a_state_machine():
    from pyarest import machine as m
    # replaying the seven CSDP step actions from Start reaches Finalized (foldl transition)
    assert m.run_machine(m.CSDP, m.CSDP_INITIAL, m.CSDP_EVENTS) == A("Finalized")
    assert m.run_machine(m.CSDP, m.CSDP_INITIAL, m.CSDP_EVENTS[:4]) == A("Uniqueness Added")
    # the machine IS a set of facts: its transitions are recovered from them by θ₁ (natural join)
    tt = m.transition_table(m.csdp_machine())
    assert set(tt.xs) == set(m.transition_relation(m.CSDP).xs)


def test_rmap_two_grouping_rules():
    from pyarest import machine as m
    from pyarest.orm import fact_type, ft_roles, uniqueness
    # rule 1: a compound uniqueness (m:n / n-ary) maps the fact type to a SEPARATE table
    likes = fact_type("Person likes Sport", ["Person", "Sport"], "likes")
    mn = uniqueness([ft_roles(likes)[0], ft_roles(likes)[1]])           # spans both roles
    assert m.rmap_group(likes, [mn]) == S(A("separate"), A("Person likes Sport"))
    # rule 2: a functional role GROUPS the fact type into the object type's table (its cell)
    placed = fact_type("Order is placed by Customer", ["Order", "Customer"], "is placed by")
    fn = uniqueness([ft_roles(placed)[0]])                             # each Order placed by ≤1 Customer
    assert m.rmap_group(placed, [fn]) == S(A("grouped"), A("Order"))


def test_rmap_is_a_state_machine():
    from pyarest import machine as m
    # a fact type advances from its elementary form to a mapped table scheme (the cell)
    assert m.run_machine(m.RMAP, m.RMAP_INITIAL, m.RMAP_EVENTS) == A("Mapped")


def test_rmap_produces_cells():
    from pyarest import machine as m
    from pyarest.orm import fact_type, ft_roles, uniqueness
    placed = fact_type("Order is placed by Customer", ["Order", "Customer"], "is placed by")
    dated = fact_type("Order has OrderDate", ["Order", "OrderDate"], "has")
    likes = fact_type("Person likes Sport", ["Person", "Sport"], "likes")
    ucs = [uniqueness([ft_roles(placed)[0]]), uniqueness([ft_roles(dated)[0]]),
           uniqueness([ft_roles(likes)[0], ft_roles(likes)[1]])]
    cells = m.rmap([placed, dated, likes], ucs)               # RMAP → D, a sequence of cells
    tables = {c.xs[1].v: sorted(x.v for x in c.xs[2].xs) for c in cells.xs}
    # Order's cell groups both functional fact types (its 3NF row); the m:n gets its own table
    assert tables["Order"] == ["Order has OrderDate", "Order is placed by Customer"]
    assert tables["Person likes Sport"] == ["Person likes Sport"]


def test_csdp_machine_drives_compilation():
    from pyarest import machine as m
    from pyarest.orm import c_kind
    readings = ["Order is an entity type.", "Customer is an entity type.",
                "Order is placed by Customer.", "Each Order is placed by exactly one Customer."]
    status, schema = m.csdp_compile(readings)                # the CSDP machine drives the compile
    assert status == A("Finalized")                          # it ran the whole procedure
    facts = [x for x in schema if isinstance(x, Seq) and x.xs[0] == A("FactType")]
    cons = [c for x in schema if isinstance(x, tuple) and x[0] == "constraints" for c in x[1]]
    assert len(facts) == 1                                    # the fact type was drawn (step 2)
    assert {c_kind(c) for c in cons} == {"UniquenessConstraint", "MandatoryConstraint"}  # steps 4/5


def test_rmap_cells_route_facts_to_their_tables():
    from pyarest import machine as m
    from pyarest.orm import fact_type, ft_roles, uniqueness
    from pyarest.ast import Fetch
    placed = fact_type("Order is placed by Customer", ["Order", "Customer"], "is placed by")
    likes = fact_type("Person likes Sport", ["Person", "Sport"], "likes")
    ucs = [uniqueness([ft_roles(placed)[0]]), uniqueness([ft_roles(likes)[0], ft_roles(likes)[1]])]
    D, groupings = m.rmap_cells([placed, likes], ucs)         # RMAP → the live cells (D)
    f1 = S(A("Order is placed by Customer"), A("o1"), A("alice"))
    f2 = S(A("Person likes Sport"), A("bob"), A("tennis"))
    D2 = m.store_fact(f2, m.store_fact(f1, D, groupings), groupings)   # routed on the RMAP table
    assert apply(Fetch("Order"), D2) == S(f1)                # functional fact → the Order cell
    assert apply(Fetch("Person likes Sport"), D2) == S(f2)   # m:n fact → its own table's cell


def test_rmap_builds_the_running_eqsys_system():
    # RMAP folds the cells into the running SYSTEM: each fact type's create is an eq. sys
    # step routed to its entity handler, validating and committing over that entity's cell.
    from pyarest import machine as m
    from pyarest.orm import fact_type, ft_roles, uniqueness
    from pyarest.ast import dispatch, Fetch
    placed = fact_type("Order is placed by Customer", ["Order", "Customer"], "is placed by")
    likes = fact_type("Person likes Sport", ["Person", "Sport"], "likes")
    ucs = [uniqueness([ft_roles(placed)[0]], "alethic"),
           uniqueness([ft_roles(likes)[0], ft_roles(likes)[1]], "alethic")]
    D, route = m.rmap_system([placed, likes], ucs)           # RMAP → running per-entity system
    f1 = S(A("Order is placed by Customer"), A("o1"), A("alice"))
    _, D2 = dispatch(route(f1), f1, D).xs                    # eq. sys step: route to Order's handler
    assert apply(Fetch("Order"), D2) == S(f1)                # committed to the Order cell
    assert apply(Fetch("Person likes Sport"), D2) == PHI     # the other entity's cell untouched (isolation)
    dup = S(A("Order is placed by Customer"), A("o1"), A("bob"))
    out3, D3 = dispatch(route(dup), dup, D2).xs              # a duplicate o1 …
    assert out3.xs[1] != PHI and apply(Fetch("Order"), D3) == S(f1)   # … refused by Order's alethic UC


def test_eqsys_threaded_D_and_tenant_unaddressability():
    from pyarest import machine as m
    from pyarest.orm import fact_type, ft_roles, uniqueness
    from pyarest.ast import dispatch_threaded, Fetch, cell
    from pyarest.objects import BOT
    placed = fact_type("Order is placed by Customer", ["Order", "Customer"], "is placed by")
    D, route = m.rmap_store([placed], [uniqueness([ft_roles(placed)[0]], "alethic")])   # self-contained store
    f1 = S(A("Order is placed by Customer"), A("o1"), A("alice"))
    _, D2 = dispatch_threaded(route(f1), f1, D).xs           # handler fetched from THIS D's cells (↑entity:D)
    assert apply(Fetch("Order"), D2) == S(f1)               # committed to the Order cell
    # a sibling tenant's store names no @Order cell → the address is unaddressable (⊥), not forbidden
    assert dispatch_threaded("@Order", f1, S(cell("Widget", PHI))) is BOT


def test_parse_state_machine_readings_and_rejects_out_of_r():
    import pytest
    from pyarest import forml
    # the paper's state-machine readings compile to the machine fact types
    p = forml.parse("Transition 'place' is from Status 'In Cart'.")
    assert p[0] == "machine_fact"
    assert forml.compile(p) == S(A("Transition is from"), A("place"), A("In Cart"))
    assert forml.compile(forml.parse("State Machine Definition 'Order' is for Noun 'Order'.")) \
        == S(A("SMD is for Noun"), A("Order"), A("Order"))
    # out-of-R readings (pronoun-correlated) are rejected, not silently mis-parsed (Def. Fragment)
    with pytest.raises(ValueError):
        forml.parse("Each Person who manages a Project also leads that Project.")


def test_parse_all_constraint_families():
    from pyarest import forml
    from pyarest.orm import c_kind
    S = forml.schema([
        "Person is an entity type.", "Sport is an entity type.", "Car is an entity type.",
        "Person plays Sport.", "Person drives Car.", "Person owns Car.",
        "Person is ancestor of Person.",
        "Each Person plays at most 3 Sport.",                    # frequency
        "There are at least 2 Person.",                          # cardinality
        "The possible values of Sport are {Tennis, Chess}.",     # value (enumeration)
        "Person is ancestor of Person is irreflexive.",          # ring
        "If Person drives Car then Person owns Car.",            # subset
        "Person drives Car if and only if Person owns Car.",     # equality
        "No Person both drives Car and owns Car.",               # exclusion
    ])
    kinds = {c_kind(c) for item in S if isinstance(item, tuple) and item[0] == "constraints"
             for c in item[1]}
    assert kinds == {"FrequencyConstraint", "CardinalityConstraint", "ValueConstraint",
                     "RingConstraint", "SubsetConstraint", "EqualityConstraint", "ExclusionConstraint"}


def test_parse_ternary_value_comparison_and_derivation():
    from pyarest import forml
    from pyarest.orm import c_kind
    from pyarest.system import derive_of
    # n-ary fact types of arbitrary trailing-conjunction arity (not only ternary)
    assert forml.parse("Recipe uses Flour and Sugar and Butter.") == \
        ("fact_type_n", "Recipe", "uses", ["Flour", "Sugar", "Butter"])
    # value comparison over an n-ary (ternary) fact type
    Sv = forml.schema(["Meeting is an entity type.", "Start is a value type.", "End is a value type.",
                       "Meeting has Start and End.", "For each Meeting, Start is before End."])
    vcs = [c for it in Sv if isinstance(it, tuple) and it[0] == "constraints" for c in it[1]]
    assert c_kind(vcs[0]) == "ValueComparisonConstraint"
    # all ten ring kinds now compile — the closure-based ones too
    Sr = forml.schema(["Person is an entity type.", "Person is ancestor of Person.",
                       "Person is ancestor of Person is acyclic."])
    assert c_kind([c for it in Sr if isinstance(it, tuple) and it[0] == "constraints"
                   for c in it[1]][0]) == "RingConstraint"
    # a projective derivation reading compiles to a rule; run through derive (lfp) it derives the head
    Sd = forml.schema(["Person is an entity type.", "Person is parent of Person.",
                       "Person is grandparent of Person is derived from "
                       "Person is parent of Person and Person is parent of Person."])
    rule = [it[1] for it in Sd if isinstance(it, tuple) and it[0] == "derivation"][0]
    P = S(S(A("Person is parent of Person"), A("al"), A("bo")),
          S(A("Person is parent of Person"), A("bo"), A("cy")))
    out = apply(derive_of([rule]), P)
    assert S(A("Person is grandparent of Person"), A("al"), A("cy")) in out.xs   # al → bo → cy


def test_hateoas_links_generated_from_p_and_s():
    from pyarest import machine as m
    P = S(S(A("Order is placed by Customer"), A("o1"), A("alice")),
          S(A("Order is placed by Customer"), A("o2"), A("bob")))
    trans = [("In Cart", "place", "Placed"), ("Placed", "ship", "Shipped")]
    nav, controls = m.links("o1", "Placed", P, trans).xs    # links(e) = nav(e) ∪ transitions(status(e))
    assert nav == S(S(A("Order is placed by Customer"), A("o1"), A("alice")))   # only o1's facts (θ₁)
    assert controls == S(S(A("ship"), A("Shipped")))        # only the control valid from 'Placed'


# --- NORMA-grounded: value typing, ranges/enumeration, referencing, columns ---
def test_data_type_facets_and_classification():
    from pyarest.orm import (value_type, vt_data_type, vt_auto,
                             dt_name, dt_length, dt_scale, dt_range_support)
    price = value_type("Price", "Decimal", length=9, scale=2)     # DataTypeLength / DataTypeScale
    dt = vt_data_type(price)
    assert dt_name(dt) == "Decimal" and dt_length(dt) == 9 and dt_scale(dt) == 2
    # range support is intrinsic to the data type (DataTypesGenerator RangeSupport)
    assert dt_range_support(vt_data_type(value_type("Qty", "Unsigned Integer"))) == "discontinuous"
    assert dt_range_support(vt_data_type(value_type("Note", "Variable Length Text"))) == "continuous"
    assert dt_range_support(vt_data_type(value_type("Key", "UUID"))) == "none"
    # Auto Counter is AutoGenerationSupport.Required → auto-generated; supplied text is not
    assert vt_auto(value_type("Seq", "Auto Counter")) and not vt_auto(value_type("Title"))
    # the whitepaper's auto-generating identifiers (Def. Schema): counter, UUID, timestamp, surrogate
    assert vt_auto(value_type("Key", "UUID")) and vt_auto(value_type("Row", "Row Id"))


def test_value_ranges_and_enumeration():
    from pyarest.orm import value, value_range, role, value_constraint_reading, c_evaluator
    height = role("Person has Height", 1, "Height")
    c = value(height, [value_range(20, 270)])                      # a numeric range [20..270]
    assert apply(c_evaluator(c), S(S(A("p1"), A(100)), S(A("p2"), A(300)))) == S(S(A("p2"), A(300)))
    assert value_constraint_reading(c) == "The possible values of Height are [20..270]."
    # enumeration = point ranges (MinValue == MaxValue); verbalizes as a braced set
    status = value(role("Order has Status", 1, "Status"), S(A("Placed"), A("Shipped")))
    assert value_constraint_reading(status) == "The possible values of Status are {Placed, Shipped}."
    assert apply(c_evaluator(status), S(S(A("o1"), A("Placed")), S(A("o2"), A("Bogus")))) == S(S(A("o2"), A("Bogus")))


def test_reference_mode_expands_to_identity_and_column():
    from pyarest.orm import (reference_mode, expand_reference_mode, refscheme_value_type,
                             refscheme_identifier, identifier_column, col_data_type,
                             vt_name, vt_data_type, dt_name, role_player, c_kind, c_roles)
    rs = expand_reference_mode("Order", reference_mode("id"))      # .id ⇒ Auto Counter, "Order_id"
    vt = refscheme_value_type(rs)
    assert vt_name(vt) == "Order_id" and dt_name(vt_data_type(vt)) == "Auto Counter"
    uc = refscheme_identifier(rs)                                  # identity IS a uniqueness constraint
    assert c_kind(uc) == "UniquenessConstraint" and role_player(c_roles(uc).xs[0].xs[0]) == "Order_id"
    # a unit-based mode ⇒ Decimal, value type named "{mode}Value"
    rs2 = expand_reference_mode("Package", reference_mode("kg"))
    assert vt_name(refscheme_value_type(rs2)) == "kgValue"
    assert dt_name(vt_data_type(refscheme_value_type(rs2))) == "Decimal"
    # defining the data type defines the primary-key column's type (RMAP hop)
    assert dt_name(col_data_type(identifier_column(rs))) == "Auto Counter"


def test_column_type_is_value_types_data_type():
    from pyarest.orm import value_type, column, col_data_type, dt_name, dt_length, dt_scale
    price = value_type("Price", "Decimal", length=9, scale=2)
    dt = col_data_type(column(price))                             # Column.DataType ← ValueType.DataType
    assert dt_name(dt) == "Decimal" and dt_length(dt) == 9 and dt_scale(dt) == 2


# --- source-faithfulness fixes: Backus primitives, general selector, Codd restriction ---
def test_general_selector_has_no_degree_ceiling():
    row = S(*[A(i) for i in range(1, 9)])                # degree 8 (Codd: degree 30 common)
    assert apply(A("7"), row) == A(7) and apply(A("8"), row) == A(8)   # selectors past 6 work


def test_more_backus_primitives():
    from pyarest.objects import BOT
    assert apply(A("-"), S(A(7), A(3))) == A(4)
    assert apply(A("×"), S(A(6), A(7))) == A(42)
    assert apply(A("÷"), S(A(9), A(3))) == A(3.0)
    assert apply(A("÷"), S(A(1), A(0))) is BOT                     # ÷ by zero ⇒ ⊥
    assert apply(A("reverse"), S(A("a"), A("b"), A("c"))) == S(A("c"), A("b"), A("a"))
    assert apply(A("rotl"), S(A(1), A(2), A(3))) == S(A(2), A(3), A(1))
    assert apply(A("rotr"), S(A(1), A(2), A(3))) == S(A(3), A(1), A(2))
    assert apply(A("trans"), S(S(A(1), A(2)), S(A(3), A(4)))) == S(S(A(1), A(3)), S(A(2), A(4)))


def test_codd_restriction_operator():
    from pyarest.theta import Restrict
    R = S(S(A("a"), A(1)), S(A("b"), A(2)), S(A("c"), A(3)))
    St = S(S(A(1), A("x")), S(A(2), A("y")))
    # Codd's R_{2|1}S: rows of R whose 2nd column occurs in π₁(S) = {1, 2}
    assert apply(Restrict([2], [1]), S(R, St)) == S(S(A("a"), A(1)), S(A("b"), A(2)))


def test_fetch_defaults_and_store_pops_first_not_purge():
    from pyarest.ast import Fetch, Store, cell
    D = S(cell("N", A("a")), cell("FILE", A("p")), cell("N", A("b")))   # two cells named N (a LIFO stack)
    assert apply(Fetch("MISSING"), D) == A("#")            # no cell named n ⇒ default #  (Backus §13.3.4)
    assert apply(Fetch("N"), D) == A("a")                  # contents of the first N cell
    D2 = apply(Store("N"), S(A("c"), D))                   # store: pop the first N, push new N=c
    assert apply(Fetch("N"), D2) == A("c")                 # new top of the N stack
    assert apply(Fetch("N"), apply(A("tl"), D2)) == A("b")  # the second N SURVIVES — pop, not purge
