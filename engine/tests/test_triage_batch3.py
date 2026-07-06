"""Third triage batch from the old engine's e2e scenarios.

subtype_metamodel_rule_e2e: an instance fact authored via the SUBTYPE (`Car '1' has
Color 'red'`) must land in the SUPERTYPE-declared fact type's population — subtype
instances ARE supertype instances (Halpin: a subtype inherits its supertypes' roles;
the fact type lives once). The old engine synthesized per-(sub, sup, ft) rules; here
the instance READING resolves up the subtype closure, the same lift rule clauses get.

transitivity_metamodel_rule_e2e: pins an ABSENCE the old engine converged to
(task-969 removed its eager `_transitive_*` materialisation as unconsumed): the
closure of R_S derives AUTHORED rules only; run_rules founds no cells beyond the
declared populations.

aggregate_min_over_recursive_closure_e2e (non-recursive halves): the min folds the
NAMED derived source, not a same-signature sibling (our sources are name-keyed
FetchPop, so signature confusion cannot arise — pinned anyway), across union heads
(two rules, one head = the disjunction reading of multiple rules per head); and a
three-role group folds per group. The recursive cost-summing halves (`Cost1 plus
Cost2 is Cost3` mid-chain) need the non-linear join compiler — recorded, not executed
(the old engine could not run them in-harness either; its own fix was verified on the
deployed path only).

ss_autofill_metamodel_rule_e2e: `subset_autofill` has no FORML surface (the old file
says so itself) — the canonical expression of auto-fill is a SEMI-DERIVED fact type
(`+`: asserted AND derived) with a plain derivation rule, and the subset constraint
stays a check. The rule-based fill and the semi-derived coexistence are executed
here; the check-against-derived-population seam is recorded in the triage doc.

valuetyped_join_key_projection_e2e: a rule joining two fact types on a VALUE-TYPED
role projects the second antecedent's other role into the head (Halpin: a conceptual
join asserts one instance plays both roles, so the joined role propagates). The old
engine's bug was case-INsensitive noun sniffing inside verb phrases (`confidence`
matched a declared `Confidence`), inflating the role set until resolution missed; our
resolver matches whole readings with case-sensitive role tokens, so the collision
class is structurally absent — pinned with the colliding noun declared."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def _cell_names(Dpy):
    return {c[1] for c in Dpy if isinstance(c, tuple) and len(c) == 3 and c[0] == "CELL"}


def test_instance_fact_via_subtype_lands_in_supertype_declared_ft():
    MODEL = """Vehicle(.id) is an entity type.
Color is a value type.
Car is an entity type.
Car is a subtype of Vehicle.
Vehicle has Color.
Car '1' has Color 'red'.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    # the fact lives ONCE, in the supertype-declared cell; no sibling ft is founded
    assert _cell(Dpy, "Vehicle_has_Color") == {("1", "red")}
    assert "Car_has_Color" not in _cell_names(Dpy)


def test_run_rules_founds_no_cells_beyond_the_authored_model():
    MODEL = """Person(.id) is an entity type.
City(.id) is an entity type.
Country(.id) is an entity type.
Person has City.
City is in Country.
Person 'p1' has City 'c1'.
City 'c1' is in Country 'us'.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    before = from_lam(D)
    after = from_lam(system.run_rules(D))
    assert _cell_names(after) == _cell_names(before)          # no engine-invented cells
    assert not any("transitive" in n for n in _cell_names(after))
    assert _cell(after, "Person_has_City") == _cell(before, "Person_has_City")


MIN_MODEL = """Node(.Id) is an entity type.
Cost is a value type.
Node moves to Node at Cost.
Node hops to Node at Cost.
Node reaches Node at Cost.
Node shortest reaches Node at Cost.
Node1 reaches Node2 at Cost1 if Node1 moves to Node2 at Cost1.
Node1 reaches Node2 at Cost1 if Node1 hops to Node2 at Cost1.
Node1 shortest reaches Node2 at Cost3 if Node1 reaches Node2 at Cost2 and Cost3 is the min of Cost2.
"""


def test_min_folds_the_named_derived_source_across_union_heads():
    D, rep = forml.compile_model(MIN_MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Node_moves_to_Node_at_Cost"), S(to_lam((("a", "c", 3),)), D))
    D = apply(ast.Store("Node_hops_to_Node_at_Cost"), S(to_lam((("a", "c", 2),)), D))
    D = system.run_rules(D)
    Dpy = from_lam(D)
    # two rules, one head: the union reading of multiple rules per head
    assert _cell(Dpy, "Node_reaches_Node_at_Cost") == {("a", "c", 3), ("a", "c", 2)}
    # the min folds the NAMED source `reaches` ({2,3} -> 2), never a sibling's {3}
    assert _cell(Dpy, "Node_shortest_reaches_Node_at_Cost") == {("a", "c", 2)}


def test_min_three_role_group_folds_per_group():
    MODEL = """Node(.Id) is an entity type.
Feature(.Id) is an entity type.
Cost is a value type.
Node moves to Node for Feature at Cost.
Node hops to Node for Feature at Cost.
Node reaches Node for Feature at Cost.
Node best reaches Node for Feature at Cost.
Node1 reaches Node2 for Feature1 at Cost1 if Node1 moves to Node2 for Feature1 at Cost1.
Node1 reaches Node2 for Feature1 at Cost1 if Node1 hops to Node2 for Feature1 at Cost1.
Node1 best reaches Node2 for Feature1 at Cost3 if Node1 reaches Node2 for Feature1 at Cost2 and Cost3 is the min of Cost2.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Node_moves_to_Node_for_Feature_at_Cost"),
              S(to_lam((("a", "c", "loc", 3),)), D))
    D = apply(ast.Store("Node_hops_to_Node_for_Feature_at_Cost"),
              S(to_lam((("a", "c", "loc", 2),)), D))
    D = system.run_rules(D)
    assert _cell(from_lam(D), "Node_best_reaches_Node_for_Feature_at_Cost") == \
        {("a", "c", "loc", 2)}


def test_subset_autofill_is_a_semi_derived_rule():
    MODEL = """Academic(.id) is an entity type.
Department(.id) is an entity type.
Academic heads Department.
Academic works for Department +.
Academic1 works for Department1 if Academic1 heads Department1.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    D = apply(ast.Store("Academic_heads_Department"), S(to_lam((("A1", "D1"),)), D))
    D = apply(ast.Store("Academic_works_for_Department"), S(to_lam((("A2", "D2"),)), D))
    D = system.run_rules(D)
    Dpy = from_lam(D)
    # semi-derived (+): the asserted row COEXISTS with the rule-filled one
    assert _cell(Dpy, "Academic_works_for_Department") == {("A1", "D1"), ("A2", "D2")}
    assert ("Academic_works_for_Department", "semi-derived") in _cell(Dpy, "derivation")


def test_valuetyped_join_projects_all_roles_despite_verb_noun_collision():
    MODEL = """Problem(.id) is an entity type.
Count(.id) is an entity type.
Confidence is a value type.
Shape is a value type.
The possible values of Shape are 'gather', 'relabel'.
Problem wins by Shape.
Shape has confidence Count.
Problem ranks Shape at Count.
Problem1 ranks Shape1 at Count1 if Problem1 wins by Shape1 and Shape1 has confidence Count1.
Problem 'p1' wins by Shape 'gather'.
Shape 'gather' has confidence Count '3'.
"""
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    assert rep["rule_diagnostics"] == []
    Dpy = from_lam(D)
    # the verb word `confidence` never matches the declared noun `Confidence`:
    # the declaration stays BINARY and the instance fact fills exactly two roles
    assert _cell(Dpy, "Shape_has_confidence_Count") == {("gather", "3")}
    D = system.run_rules(D)
    # the value-typed join key propagates and the second antecedent's other
    # role (Count) projects into the head
    assert _cell(from_lam(D), "Problem_ranks_Shape_at_Count") == {("p1", "gather", "3")}
