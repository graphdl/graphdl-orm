"""The book's derivation-rule surface (Halpin ch. 2, exercise 4, D1 verbatim):
'Person1 is a grandparent of Person2 if Person1 is a parent of some Person3 and
Person3 is a parent of Person2' — numbered variables, `some` existentials, ` and `
conjunction, and multiple rules with one head giving disjunction (CWA closure). The
compiler resolves each clause to its fact type, joins the linear chain on shared
variables, projects the head's variables, and the cross-cell runner derives to the lfp.
The corpus's `**` after the period is the NORMA storage marker, not a rule form."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, system
from pyarest.reduce import apply


MODEL = """Person is an entity type.
Person is a parent of Person.
Person1 is a grandparent of Person2 if Person1 is a parent of some Person3 and Person3 is a parent of Person2.
Person1 is an ancestor of Person2 if Person1 is a parent of Person2.
Person1 is an ancestor of Person2 if Person1 is a parent of some Person3 and Person3 is an ancestor of Person2.
"""


def _with_pop(D, name, pop):
    return apply(ast.Store(name), L.SEQ(L.CONS(to_lam(pop))(L.CONS(D)(L.NIL))))


def _cell(D, name):
    for c in from_lam(D):
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_rule_if_parses_and_registers_reads_and_derives():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    # rule ids carry a stable body digest (one head may have several rules: disjunction)
    reads = _cell(D, "ruleReads")
    assert any(r.startswith("Person_is_a_grandparent_of_Person_rule")
               and ft == "Person_is_a_parent_of_Person" for (r, ft) in reads)
    derives = _cell(D, "ruleDerives")
    assert any(r.startswith("Person_is_a_grandparent_of_Person_rule")
               and h == "Person_is_a_grandparent_of_Person" for (r, h) in derives)
    heads = [h for (_r, h) in derives if h == "Person_is_an_ancestor_of_Person"]
    assert len(heads) == 2                                    # two rules, one head


def test_grandparent_derives_by_the_chain_join():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Person_is_a_parent_of_Person", (("a", "b"), ("b", "c"), ("c", "d")))
    D = system.run_rules(D)
    assert _cell(D, "Person_is_a_grandparent_of_Person") == {("a", "c"), ("b", "d")}


def test_recursive_ancestor_reaches_the_least_fixed_point():
    D, _ = forml.compile_model(MODEL)
    D = _with_pop(D, "Person_is_a_parent_of_Person", (("a", "b"), ("b", "c"), ("c", "d")))
    D = system.run_rules(D)
    assert _cell(D, "Person_is_an_ancestor_of_Person") == {
        ("a", "b"), ("b", "c"), ("c", "d"), ("a", "c"), ("b", "d"), ("a", "d")}


def test_corpus_marker_after_the_period_is_the_storage_marker():
    # the corpus writes 'Fact Type has Format. **' — statement + trailing NORMA marker
    D, rep = forml.compile_model("Fact Type is an entity type.\nFormat is a value type.\n"
                                 "Fact Type has Format. **")
    assert rep["unparsed"] == []
    assert ("Fact_Type_has_Format", "derived-and-stored") in _cell(D, "derivation")
