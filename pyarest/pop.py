"""Three-valued population semantics (Def. Population).

A ground fact g is *true* in P if g ∈ P, *false* if its paired negation fact
⟨¬, …g⟩ ∈ P, and *unknown* otherwise; under the closed-world assumption on a noun,
unknown collapses to false. Falsity is asserted into P as a negation fact and is
never inferred from absence, so the derivation of truths and of falsities is
positive (Lemma finiteness undisturbed). Membership is the characteristic function
of P — g ∈ P and P g are one act.
"""
from .objects import Atom, Seq
from .reduce import apply
from .theta import Filter

_S = lambda *xs: Seq(xs)
_A = Atom
_NEG = _A("¬")

_member = _S(_A("COMP"), _A("not"), _A("null"), Filter(_A("eq")), _A("distl"))   # ⟨x, Y⟩ → x ∈ Y
_neg = _S(_A("COMP"), _A("apndl"), _S(_A("CONS"), _S(_A("CONST"), _NEG), _A("id")))   # g → ⟨¬, …g⟩


def negate(fact):
    """The paired negation fact ⟨¬, …g⟩ (verbalized "it is known to be false that …")."""
    return apply(_neg, fact)


# truth:⟨g, P⟩ → 'true' | 'false' | 'unknown'  — open-world, three-valued (Def. Population)
_in_P = _S(_A("COMP"), _member, _S(_A("CONS"), _A("1"), _A("2")))                      # g ∈ P?
_neg_in_P = _S(_A("COMP"), _member, _S(_A("CONS"), _S(_A("COMP"), _neg, _A("1")), _A("2")))  # ¬g ∈ P?
truth = _S(_A("COND"), _in_P, _S(_A("CONST"), _A("true")),
           _S(_A("COND"), _neg_in_P, _S(_A("CONST"), _A("false")), _S(_A("CONST"), _A("unknown"))))


def truth_of(g, P, closed_world=False):
    """The three-valued truth of ground fact g in population P (Def. Population). Under
    the closed-world assumption on the noun, an unknown collapses to false."""
    t = apply(truth, _S(g, P)).v
    return "false" if (closed_world and t == "unknown") else t
