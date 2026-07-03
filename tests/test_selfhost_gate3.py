"""Self-host, gate three: the statement translators are DEFS-REGISTERED NAMES. M
declares the dispatch (Classification has Translator, ingested from the grammar file)
and each translator name resolves through rho to a registered definition — the same
boundary as the federation connectors (DEFS is the DI container; swapping is
re-registering). The engine keeps NO translator dispatch table: the old
_KINDS_BY_TRANSLATOR map is dissolved into the registered impls' own bindings, so a
new surface lands by (a) declaring Classification/Translator readings in the grammar
and (b) registering the impl — no engine-table edits. A name M declares that this
host has not registered is skipped gracefully (universal override interface: graceful
absence), and the seed-equivalence battery (test_selfhost_compile) holds unchanged."""
import pyarest.prims  # noqa: F401
from pyarest import defs, forml


def test_translators_are_defs_registered_names_and_the_map_is_gone():
    forml.register_translators()                              # idempotent
    for name in ("translate_nouns", "translate_derivation_rules",
                 "translate_set_constraints", "translate_state_machines"):
        assert defs.latest.get(name, ("",))[0] == "registered"
    assert not hasattr(forml, "_KINDS_BY_TRANSLATOR")         # the map is dissolved


def test_selfhost_dispatch_resolves_through_rho_not_a_python_table():
    calls = []

    def probe(mu):
        def g(operand):
            calls.append("hit")
            from pyarest.reduce import apply as _apply
            from pyarest.lam import atom as A
            return _apply(A(4), operand)                      # D unchanged (4th component)
        return g

    defs.register("translate_nouns", probe)                  # swap = re-register
    try:
        _D, rep = forml.compile_model_selfhost("Person is an entity type.\n")
        assert calls, "dispatch must reach the re-registered translator through rho"
        assert rep["unclassified"] == []
    finally:
        forml.register_translators()                          # restore the real bindings
    # restored: the real translator lands the noun again
    D2, _rep = forml.compile_model_selfhost("Person is an entity type.\n")
    from pyarest.lam import from_lam
    rows = [c[2] for c in from_lam(D2)
            if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", "instanceOf")]
    assert rows and ("Person", "ObjectType") in set(rows[0])


def test_a_declared_but_unregistered_translator_name_degrades_gracefully():
    # M may name translators a host has not registered (the grammar already names
    # translate_partitions with no binding here): the dispatch skips them without
    # error instead of dying — graceful absence, the boundary's contract
    real = defs.latest.pop("translate_finality", None)
    try:
        _D, rep = forml.compile_model_selfhost("Order is an entity type.\n"
                                               "Order becomes final at depth 6.\n")
        assert rep["unclassified"] == []                      # classified, then skipped
    finally:
        if real is not None:
            forml.register_translators()
