"""State machines as VALUES fed into ONE lambda (Prop. onestep: machine = foldl transition).

A machine is not code — it IS its transition relation, a value. There is one runner, `run`,
an FFP object that folds a transition value over a sequence of inputs from an initial state,
applying the transition with the `apply` primitive (membership is application). RMAP and CSDP
are two such values passed into that same one lambda; the runner does not know which machine
it runs. That is the whole thing "expressible as values passed into one lambda."
"""
from . import lam as L
from .lam import atom as A
from .defs import define

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

_COMP, _CONS, _COND, _WHILE = A("COMP"), A("CONS"), A("COND"), A("WHILE")
_1, _2, _3 = A(1), A(2), A(3)
_TL, _NULL, _NOT, _APPLY, _APNDR, _EQ, _CONST = A("tl"), A("null"), A("not"), A("apply"), A("apndr"), A("eq"), A("CONST")

# ---- run: the one lambda. run:⟨t, ⟨acc0, inputs⟩⟩ = foldl(t, acc0, inputs). ----
# The state threads ⟨t, acc, remaining⟩ so the transition VALUE travels with the fold and is
# applied to ⟨acc, input⟩ each step via `apply`. One runner; the machine is the value `t`.
_input   = _S(_COMP, _1, _3)                                 # 1:(3:state) — the current input
_new_acc = _S(_COMP, _APPLY, _S(_CONS, _1, _S(_CONS, _2, _input)))   # apply:⟨t, ⟨acc, input⟩⟩
_new_rem = _S(_COMP, _TL, _3)                                # tl:(3:state)
_step    = _S(_CONS, _1, _new_acc, _new_rem)                 # ⟨t, acc', rem'⟩
_hasmore = _S(_COMP, _NOT, _NULL, _3)                        # remaining non-empty?
_loop    = _S(_WHILE, _hasmore, _step)
_init    = _S(_CONS, _1, _S(_COMP, _1, _2), _S(_COMP, _2, _2))       # ⟨t, acc0, inputs⟩
run = _S(_COMP, _2, _loop, _init)                            # 2:(loop:(init:arg)) = the final acc
define("run", run)


def run_machine(transition, acc0, inputs):
    """Fold a transition VALUE over `inputs` from `acc0` — via the one `run` lambda."""
    from .reduce import apply
    return apply(run, _S(transition, _S(acc0, inputs)))


# ---- RMAP as a value (Halpin §10.3): the two grouping rules, as a transition relation. ----
# Over fact-type facts ⟨factType, objectType, kind⟩, RMAP assigns each fact type a table:
#   functional role  → grouped ON the object type   (rule 2)
#   compound UC      → its OWN table                 (rule 1)
# The transition emits ⟨tableKey, factType⟩; folding it over the schema is the mapping — and
# "the decomposition into atomic facts IS the relational mapping": each key becomes a cell.
_kind = _S(_COMP, _3, _2)                                    # kind of the fact-type fact (2:arg)
_ot   = _S(_COMP, _2, _2)                                    # its object type
_ft   = _S(_COMP, _1, _2)                                    # its fact type
_is_functional = _S(_COMP, _EQ, _S(_CONS, _kind, _S(_CONST, A("functional"))))
_table_key = _S(_COND, _is_functional, _ot, _ft)            # rule 2 → object type ; rule 1 → own
_entry = _S(_CONS, _table_key, _ft)                         # ⟨tableKey, factType⟩
rmap = _S(_COMP, _APNDR, _S(_CONS, _1, _entry))            # apndr:⟨acc, ⟨tableKey, factType⟩⟩
define("rmap", rmap)


# ---- CSDP as a value: another transition, run by the SAME lambda. ----
# The CSDP populate step (Halpin §3.2) folds elementary example facts into the schema's fact
# set. Here that step is `apndr` (accumulate each verbalized fact) — a different value into `run`.
csdp = _APNDR
define("csdp", csdp)
