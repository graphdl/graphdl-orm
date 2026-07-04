"""The self-capturing metamodel M (spec §4.3; Halpin §13.7, read verbatim).

M is authored as FORML readings (M_READINGS) and ingested through compile_model like any
schema; Cor. closure means there is no special path for schema-about-schemas. Ingesting M
yields meta-cells that DESCRIBE M — the fixpoint: the instanceOf cell contains
("Object Type", "ObjectType"). M_MAP is M's own relational mapping, from each declared M
fact type to the runtime cell that stores its population; self_gate then runs the book's
test — "to test a full ORM metaschema, you should be able to populate it with itself" —
by validating every mapped cell with the constraints M declares for it.

The recomputation frontier (Cor. streaming) is read off the constraint, ruleReads, and
ruleDerives cells that ingestion wrote; nothing is tracked host-side.
"""
from . import lam as L
from .lam import atom as A, to_lam, from_lam
from . import canon as T
from .reduce import apply


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


# M's own readings, in the same fragment the compiler parses (Halpin §13.7's noun seed:
# ObjectType/OTkind, the predicate and its roles with positions and players, Constraint
# with its kind, plus the modality tag; 'specializes' phrases the subtype link as a plain
# fact type so the reading declares M's fact type rather than asserting a subtype fact).
M_READINGS = """Object Type is an entity type.
OT Kind is a value type.
Fact Type is an entity type.
Reading is a value type.
Role is an entity type.
Position is a value type.
Constraint is an entity type.
Constraint Kind is a value type.
Modality is a value type.
Object Type is of OT Kind.
Each Object Type is of exactly one OT Kind.
The possible values of OT Kind are 'ObjectType', 'ValueType'.
Fact Type has Reading.
Each Fact Type has at most one Reading.
Role is in Fact Type at Position played by Object Type.
Each Role is in at most one Fact Type at Position played by Object Type.
Constraint is of Constraint Kind about Fact Type with Modality.
Each Constraint is of at most one Constraint Kind about Fact Type with Modality.
Object Type specializes Object Type.
The possible values of Modality are 'alethic', 'deontic'.
"""

# M's rmap: declared M fact type -> the runtime cell holding its population. The bridge
# is explicit and dissolves when grammar-as-readings unifies cell naming (Stage 2/3).
M_MAP = {
    "Object_Type_is_of_OT_Kind": "instanceOf",
    "Fact_Type_has_Reading": "factType",
    "Role_is_in_Fact_Type_at_Position_played_by_Object_Type": "role",
    "Constraint_is_of_Constraint_Kind_about_Fact_Type_with_Modality": "constraint",
    "Object_Type_specializes_Object_Type": "subtype",
}


def initial_D():
    """The empty store seed a schema is compiled INTO (a FILE cell)."""
    from . import ast
    return L.SEQ(L.CONS(ast.cell("FILE", to_lam(())))(L.NIL))


def M_store():
    """Ingest M's own readings: (D, report) where D's meta-cells describe M itself."""
    from . import forml
    return forml.compile_model(M_READINGS)


def _rows(D, name):
    from . import ast
    rows = from_lam(apply(ast.FetchPop(name), D))
    return list(rows) if isinstance(rows, tuple) else []


def instances_of(D, kind):
    """θ₁ selection over the instanceOf cell: the names of `kind`'s instances."""
    from . import ast
    is_kind = _S(A("COMP"), A("eq"), _S(A("CONS"), A(2), _S(A("CONST"), A(kind))))
    sel = _S(A("COMP"), _S(A("ALPHA"), A(1)), T.Filter(is_kind), ast.FetchPop("instanceOf"))
    return set(from_lam(apply(sel, D)))


def self_gate(D):
    """The §13.7 gate: validate each mapped runtime cell with the constraints M declares
    for its fact type, within D's own step (the constraint objects live in D's DEFS)."""
    from . import forml, ast, defs
    report = {}
    for m_ft, cell in M_MAP.items():
        val = forml.validate_for(m_ft, D)
        pop = apply(ast.FetchPop(cell), D)
        with defs.step(D):
            _p, v, flag = from_lam(apply(val, _S(pop, D)))
        report[cell] = (tuple(v) if isinstance(v, tuple) else (v,), flag)
    return report


# ============================ the bounded-recomputation frontier ==============
# derive = lfp F_S and validate are INCREMENTAL and BOUNDED (Cor. streaming): a change to
# a fact type re-triggers only the constraints scoped to it and the rules that read it,
# then (transitively) what those rules derive — all read off M's cells.

def affected_constraints(D, fact_type):
    """The ONLY constraints validate must re-check when `fact_type` changes."""
    return tuple(f[0] for f in _rows(D, "constraint") if len(f) >= 3 and f[2] == fact_type)


def affected_rules(D, fact_type):
    """The ONLY derivation rules derive must re-fire when `fact_type` changes."""
    return tuple(r[0] for r in _rows(D, "ruleReads") if len(r) >= 2 and r[1] == fact_type)


def recompute_frontier(D, fact_type):
    """The bound on the lfp for a change to `fact_type`: constraints to re-check, rules to
    re-fire, and the fact types those rules derive (feeding the next incremental round).
    The rules→derives hop is a θ₁ natural join over the ruleDerives cell."""
    rules = affected_rules(D, fact_type)
    derives_rows = tuple(tuple(r) for r in _rows(D, "ruleDerives"))
    joined = _S(A("COMP"), _S(A("ALPHA"), A(2)), T.NatJoin(1))
    derives = from_lam(apply(joined, to_lam((tuple((r,) for r in rules), derives_rows))))
    return {"constraints": affected_constraints(D, fact_type), "rules": rules,
            "derives": tuple(derives)}
