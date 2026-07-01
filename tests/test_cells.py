from pyarest.objects import Atom, Seq, DEFAULT
from pyarest.cells import cell, fetch, store_, pop, purge


def test_fetch_returns_first_match_else_default():
    s = Seq((cell(Atom("A"), Atom(1)), cell(Atom("B"), Atom(2)), cell(Atom("A"), Atom(9))))
    assert fetch(Atom("A"), s) == Atom(1)     # first match wins
    assert fetch(Atom("B"), s) == Atom(2)
    assert fetch(Atom("Z"), s) == DEFAULT     # absent → default


def test_store_prepends_and_shadows_old():
    s = Seq((cell(Atom("A"), Atom(1)),))
    s2 = store_(Atom("A"), Atom(7), s)
    assert fetch(Atom("A"), s2) == Atom(7)    # new value shadows
    assert len(s2.items) == 1                 # pop removed the stale cell


def test_pop_and_purge():
    s = Seq((cell(Atom("A"), Atom(1)), cell(Atom("A"), Atom(2)), cell(Atom("B"), Atom(3))))
    assert fetch(Atom("A"), pop(Atom("A"), s)) == Atom(2)   # first A removed
    assert fetch(Atom("A"), purge(Atom("A"), s)) == DEFAULT  # all A removed
