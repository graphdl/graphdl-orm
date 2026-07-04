"""Encryption per the platform arc: field-level by construction, sensitivity derived
from DATA TYPES, mode derived from CONSTRAINTS (uniqueness or reference participation
forces deterministic sealing so equality survives; everything else randomizes), the key
scope a tenant concern, and the cipher a boundary def (test-grade here, loudly marked).
The sqlite driver seals the derived roles at rest and unseals on load."""
import os
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam
from pyarest import ast, forml, persist
from pyarest import persist as seal
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


MODEL = """Person(.SSN) is an entity type.
SSN is a value type.
Note is a value type.
Salary is a value type.
Alias is a value type.
Person has Note.
Person earns Salary.
Person hides Alias.
For each Alias, at most one Person hides that Alias.
Data Type: SSN is SensitiveText.
Data Type: Note is SensitiveText.
Data Type: Alias is SensitiveText.
Data Type: Salary is Money.
"""


def test_sensitivity_and_mode_derive_from_the_schema():
    D, rep = forml.compile_model(MODEL)
    assert rep["unparsed"] == []
    p = seal.seal_plan(D)
    assert p["roles"][("Person_has_Note", 2)] == "randomized"     # sensitive, unconstrained
    assert ("Person_earns_Salary", 2) not in p["roles"]           # Money is not sensitive
    assert p["ids"]["Person"] == "deterministic"                  # the identifier IS equality
    # the inverse UC sits ON the Alias role, so equality must survive sealing there
    assert p["roles"][("Person_hides_Alias", 2)] == "deterministic"


def test_deterministic_sealing_preserves_equality_and_randomized_does_not():
    k = b"tenant-key"
    a1 = seal.seal(k, "123-45-6789", deterministic=True)
    a2 = seal.seal(k, "123-45-6789", deterministic=True)
    assert a1 == a2 and a1 != "123-45-6789"                       # NATEQ on ciphertexts
    r1 = seal.seal(k, "123-45-6789", deterministic=False)
    r2 = seal.seal(k, "123-45-6789", deterministic=False)
    assert r1 != r2
    assert seal.unseal(k, a1) == seal.unseal(k, r1) == "123-45-6789"


def test_sqlite_seals_the_derived_roles_at_rest(tmp_path):
    D, _ = forml.compile_model(MODEL)
    D = apply(ast.Store("Person_has_Note"), S(to_lam((("p1", "the-secret-note"),)), D))
    path = os.path.join(str(tmp_path), "sealed.db")
    persist.save_sqlite(D, path, seal_key=b"tenant-key")
    raw = open(path, "rb").read()
    assert b"the-secret-note" not in raw                          # sealed at rest
    D2 = persist.load_sqlite(path, seal_key=b"tenant-key")
    rows = [c[2] for c in from_lam(D2)
            if isinstance(c, tuple) and len(c) == 3 and c[1] == "Person_has_Note"]
    assert rows and ("p1", "the-secret-note") in set(rows[0])     # unsealed on load
