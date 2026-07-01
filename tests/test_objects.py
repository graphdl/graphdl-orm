from pyarest.objects import Atom, Seq, BOTTOM, PHI, T, F, seq, is_atom, is_seq, is_bottom


def test_atom_equality_by_value():
    assert Atom(2) == Atom(2)
    assert Atom("T") == T
    assert Atom(2) != Atom("2")


def test_seq_constructor_is_bottom_preserving():
    assert seq(Atom(1), Atom(2)) == Seq((Atom(1), Atom(2)))
    assert seq(Atom(1), BOTTOM) is BOTTOM


def test_phi_is_both_atom_and_sequence():
    assert PHI == Seq(())
    assert is_seq(PHI) and is_atom(PHI)


def test_predicates():
    assert is_atom(Atom(5)) and not is_seq(Atom(5))
    assert is_seq(Seq((Atom(1),))) and not is_atom(Seq((Atom(1),)))
    assert is_bottom(BOTTOM)
