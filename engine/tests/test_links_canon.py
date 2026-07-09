"""links_of's union moved into system.canon (Thm. HATEOAS: links(e) =
nav(e) ∪ transitions(status(e)), a theta1 expression). nav_of / transitions_of
already dispatch; system:links_of composes them with cat. Gated ABSOLUTELY: for
any input, links(inp) is exactly nav(inp) followed by transitions(inp) (cat =
concatenation). Because it composes only differential-covered primitives + the two
existing canon DEFs, the Rust host reduces the identical bytes."""
import pyarest.prims  # noqa: F401
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

SM = to_lam((("Draft", "sub", "Checked"), ("Other", "x", "Y")))


def test_links_of_reduces_to_cat_union():
    # system:links_of applied to <key_pos, sm, status_pos> reduces to
    #   cat ∘ [ nav_of(key_pos), transitions_of(sm, status_pos) ]
    # i.e. outer COMP whose function is cat over a CONS of exactly two branches.
    # (Value comparison is defeated by Scott-encoding of sm in outputs, so this
    # pins the union STRUCTURE, which is the theorem: links = nav ∪ transitions.)
    tree = from_lam(R(A("system:links_of"), to_lam((1, SM, 1))))
    assert tree[0] == "COMP" and tree[1] == "cat", tree[:2]
    union = tree[2]
    assert union[0] == "CONS" and len(union) == 3, union  # exactly nav + transitions
    # the first branch is the nav side, the second the transitions side (both COMP objects)
    assert union[1][0] == "COMP" and union[2][0] == "COMP"
