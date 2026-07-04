"""Canon at boot, toolchain on the canon. Importing pyarest loads the intersection
files into DEFS (like the translator and federation registrations), so canonical
names resolve in any step frame without an explicit load. The toolchain modules then
STOP owning the definitions: python/theta.py's closed objects ARE the canon values
(name-reference-bearing trees), and its constructors apply the canonical builders
through the reducer, so there is exactly one source of truth, the shared files, and
the host module is a binding of it. The migrated objects carry theta: references by
construction, asserted here so equality tests cannot silently pass on a fork."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import to_lam, from_lam, atom as A
from pyarest import constraints as C, defs
from pyarest import canon as theta
from pyarest.reduce import apply


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _refs(tree):
    if isinstance(tree, tuple):
        return any(_refs(x) for x in tree)
    return isinstance(tree, str) and (tree.startswith("theta:")
                                      or tree.startswith("constraints:"))


def test_canonical_names_resolve_at_boot_without_an_explicit_load():
    D = L.SEQ(L.NIL)
    with defs.step(D):
        got = from_lam(apply(A("theta:member"), S(to_lam(("a", 1)),
                                                  to_lam((("a", 1), ("b", 2))))))
    assert got == "T"


def test_the_toolchain_objects_are_the_canon_not_a_fork():
    # the closed objects carry canonical references: they were LOADED, not rebuilt
    assert _refs(from_lam(theta.dedup))
    assert _refs(from_lam(theta.setminus))
    assert _refs(from_lam(C.mandatory()))


def test_the_constructors_apply_the_canonical_builders():
    p = A("eq")                                               # keep the equal pairs
    f = theta.Filter(p)
    assert _refs(from_lam(f))                                 # built THROUGH the canon
    D = L.SEQ(L.NIL)
    with defs.step(D):
        got = from_lam(apply(f, to_lam((("x", "x"), ("x", "y")))))
    assert got == (("x", "x"),)
    j = theta.NatJoin(2)
    with defs.step(D):
        out = from_lam(apply(j, S(to_lam((("a", 1),)), to_lam(((1, "z"),)))))
    assert out == (("a", 1, "z"),)


def test_joinon_and_restrict_come_from_the_canon():
    D = L.SEQ(L.NIL)
    j = theta.JoinOn(((3, 1),), (2, 3))
    assert _refs(from_lam(j))
    R = to_lam(((1, "x", 5), (1, "x", 7)))
    Sp = to_lam(((5, "u", "keep5"), (7, "v", "keep7"), (9, "w", "no")))
    with defs.step(D):
        out = from_lam(apply(j, S(R, Sp)))
    assert set(out) == {(1, "x", 5, "u", "keep5"), (1, "x", 7, "v", "keep7")}
    # the degenerate cases ride the same COND builder: cross product and semijoin
    with defs.step(D):
        cross = from_lam(apply(theta.JoinOn((), (1,)),
                               S(to_lam((("a",),)), to_lam((("z",),)))))
    assert set(cross) == {("a", "z")}
    with defs.step(D):
        semi = from_lam(apply(theta.JoinOn(((1, 1),), ()),
                              S(to_lam((("a", 1), ("b", 2))), to_lam((("a",),)))))
    assert set(semi) == {("a", 1)}
    r = theta.Restrict([1], [2])
    assert _refs(from_lam(r))
    with defs.step(D):
        kept = from_lam(apply(r, S(to_lam((("a", 1), ("b", 2))),
                                   to_lam(((9, "a"),)))))
    assert set(kept) == {("a", 1)}


def test_uniqueness_and_exclusion_come_from_the_canon():
    u = C.uniqueness([1])
    assert _refs(from_lam(u))
    D = L.SEQ(L.NIL)
    with defs.step(D):
        v = from_lam(apply(u, to_lam((("a", 1), ("a", 2), ("b", 3)))))
    assert set(v) == {("a", 1), ("a", 2)}
    with defs.step(D):
        v2 = from_lam(apply(C.exclusion(), to_lam((("e1", "f1"), ("e1", "f2")))))
    assert set(v2) == {("e1", "f1"), ("e1", "f2")}


def _wrapper_is_the_canonical_application(built, name, param, x, expected):
    """The strict authorship gate: the canonical NAME applied to the encoded
    parameter must produce the SAME behavior as the wrapper, and the absolute
    result must be right. A wrapper composing canon pieces host-side cannot pass
    this: the name itself must be defined in the shared file."""
    D = L.SEQ(L.NIL)
    with defs.step(D):
        via_name = apply(A(name), param) if param is not None else apply(A(name), to_lam(()))
        got = from_lam(apply(via_name, x))
        wrapped = from_lam(apply(built, x))
    assert set(got) == expected and set(wrapped) == expected, name


def test_the_value_families_come_from_the_canon():
    _wrapper_is_the_canonical_application(
        C.value_enumeration(1, ("A", "B")), "constraints:value_enumeration",
        S(A(1), to_lam(("A", "B"))),
        to_lam((("A",), ("F",), ("B",))), {("F",)})
    _wrapper_is_the_canonical_application(
        C.value_range(2, lo=1, hi=5), "constraints:value_range",
        S(A(2), S(to_lam(1), A("F")), S(to_lam(5), A("F"))),
        to_lam((("x", 0), ("y", 3), ("z", 9))), {("x", 0), ("z", 9)})
    _wrapper_is_the_canonical_application(
        C.value_range(1, lo=2, lo_open=True), "constraints:value_range",
        S(A(1), S(to_lam(2), A("T")), to_lam(())),
        to_lam(((2,), (3,))), {(2,)})


def test_frequency_and_exclusive_or_come_from_the_canon():
    pop = (("a", 1), ("a", 2), ("b", 3), ("c", 4), ("c", 5), ("c", 6))
    _wrapper_is_the_canonical_application(
        C.frequency([1], lo=2, hi=2), "constraints:frequency",
        S(to_lam((1,)), to_lam((2,)), to_lam((2,))),
        to_lam(pop), {("b", 3), ("c", 4), ("c", 5), ("c", 6)})
    universe = to_lam((("e1",), ("e2",), ("e3",)))
    participation = to_lam((("e1", "c1"), ("e3", "c1"), ("e3", "c2")))
    D = L.SEQ(L.NIL)
    with defs.step(D):
        got = from_lam(apply(A("constraints:exclusive_or"), S(universe, participation)))
        wrapped = from_lam(apply(C.exclusive_or(), S(universe, participation)))
    assert set(got) == {("e2",), ("e3",)} and set(wrapped) == {("e2",), ("e3",)}
