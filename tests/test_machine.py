"""State machines as values into one lambda: one `run` folds any transition value (Prop.
onestep). RMAP and CSDP are just two of those values."""
from pyarest import to_lam, from_lam
from pyarest.lam import atom as A
import pyarest.prims  # noqa: F401
from pyarest import machine as M


def run(t, acc0, inputs):
    return from_lam(M.run_machine(t, to_lam(acc0), to_lam(inputs)))


def test_one_runner_many_machines():
    # the SAME run lambda, different transition VALUES — the machine is the value, not the code
    assert run(A("+"), 0, (1, 2, 3)) == 6                    # a summing machine
    assert run(A("apndr"), (), ("a", "b", "c")) == ("a", "b", "c")  # a collecting machine

def test_rmap_is_a_value_into_run():
    # RMAP's two grouping rules as a transition value: functional → object-type table,
    # compound → own table. Folding it over the schema is the relational mapping.
    schema = (("has_name", "Person", "functional"),
              ("has_age", "Person", "functional"),
              ("enrolled", "Enrollment", "compound"))
    assert run(M.rmap, (), schema) == (("Person", "has_name"),
                                       ("Person", "has_age"),
                                       ("enrolled", "enrolled"))

def test_csdp_is_a_value_into_the_same_run():
    # CSDP's populate step (a different value) run by the SAME lambda
    examples = (("Person", "has", "Name"), ("Person", "has", "Age"))
    assert run(M.csdp, (), examples) == examples
