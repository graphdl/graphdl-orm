"""The self-capturing metamodel M (spec §4.3).

M describes the ORM constructs — ObjectType, FactType, Role, Constraint — and each is an
FFP object in M's population, reflected back by rho to the ORM layer as the meta-object.
M's own constructs are among what it describes: ObjectType is an ObjectType (the fixpoint
of self-capture), and "each Role is in exactly one FactType" is a Constraint whose FFP
violation object, reflected, checks M's own Role population. So M is a population that
populates itself, validated by the same mu and the same constraint objects as any schema.
"""
from . import lam as L
from .lam import atom as A, to_lam, from_lam
from . import theta as T
from . import constraints as C
from .reduce import apply

def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)

# the meta object types (atoms that name M's own object types)
OBJECTTYPE, FACTTYPE, ROLE, CONSTRAINT = "ObjectType", "FactType", "Role", "Constraint"

# M's population — facts describing the ORM constructs, INCLUDING M's own (self-capture).
# instanceOf ⟨x, meta-type⟩ : x is an instance of a meta object type.
_INSTANCE_OF = (
    (OBJECTTYPE, OBJECTTYPE),        # ObjectType is an ObjectType — the self-capture fixpoint
    (FACTTYPE,   OBJECTTYPE),         # FactType, Role, Constraint are ObjectTypes too
    (ROLE,       OBJECTTYPE),
    (CONSTRAINT, OBJECTTYPE),
)
# roleIn ⟨role, factType, position⟩ : which fact type each role belongs to (M's own roles)
_ROLE_IN = (
    ("r_ot_has_name", "ObjectType_has_Name", 1),
    ("r_name_of_ot",  "ObjectType_has_Name", 2),
    ("r_role_in_ft",  "Role_in_FactType", 1),
    ("r_ft_of_role",  "Role_in_FactType", 2),
)


def instance_of_population():
    return to_lam(_INSTANCE_OF)


def role_in_population():
    return to_lam(_ROLE_IN)


def initial_D():
    """M's population as cells in a state D — the substrate a schema is compiled INTO. Compiling
    a reading is a create command over one of these cells (D3 / Cor. closure)."""
    from . import ast
    return L.SEQ(L.CONS(ast.cell("instanceOf", instance_of_population()))(
                 L.CONS(ast.cell("roleIn", role_in_population()))(L.NIL)))


def instances_of(meta_type):
    """rho-reflect the extent of a meta object type: theta1 selection of M's instanceOf
    facts whose type is `meta_type`. instances_of('ObjectType') contains ObjectType itself."""
    is_type = _S(A("COMP"), A("eq"), _S(A("CONS"), A(2), _S(A("CONST"), A(meta_type))))
    return apply(T.Filter(is_type), instance_of_population())


# "each Role is in exactly one FactType" — a uniqueness Constraint over role (column 1) of
# roleIn. Its FFP violation object, reflected, validates M's own Role population.
role_in_one_facttype = C.uniqueness([1])


def validate_roles(population=None):
    """(rho role_in_one_facttype) : Role-population — M checking itself with its own constraint."""
    return apply(role_in_one_facttype, role_in_population() if population is None else population)


# ============================ the bounded-recomputation frontier ==============
# derive = lfp F_S and validate are INCREMENTAL and BOUNDED (Cor. streaming): a change to a
# fact type only re-triggers the constraints scoped to it and the rules that read it, then
# (transitively) what those rules derive. The frontier is a theta1 query over M — the
# metamodel records the dependencies, so the bound is read off M, not tracked host-side.

# constraintScope ⟨constraint, factType⟩ : the fact type a constraint guards
_CONSTRAINT_SCOPE = (
    ("uc_role_in_ft", "Role_in_FactType"),        # role-uniqueness guards Role_in_FactType
    ("uc_name",       "ObjectType_has_Name"),      # a different constraint, a different scope
)
# ruleReads ⟨rule, factType⟩ / ruleDerives ⟨rule, factType⟩ : a derivation rule's in/out fact types
_RULE_READS = (("r_inherit", "Role_in_FactType"),)
_RULE_DERIVES = (("r_inherit", "Inherited_Role"),)


def _keyed_on(pos, value):
    """selection predicate: keep tuples whose role `pos` equals `value`."""
    return _S(A("COMP"), A("eq"), _S(A("CONS"), A(pos), _S(A("CONST"), A(value))))


def _ids_where(population_tuples, pos, value):
    """theta1: ids (role 1) of the facts whose role `pos` = value — the frontier projection.
    alpha(1) projects each matching tuple to its id, so the result is already the id tuple."""
    frontier = _S(A("COMP"), _S(A("ALPHA"), A(1)), T.Filter(_keyed_on(pos, value)))
    return from_lam(apply(frontier, to_lam(population_tuples)))


def affected_constraints(fact_type):
    """The ONLY constraints validate must re-check when `fact_type` changes."""
    return _ids_where(_CONSTRAINT_SCOPE, 2, fact_type)


def affected_rules(fact_type):
    """The ONLY derivation rules derive must re-fire when `fact_type` changes."""
    return _ids_where(_RULE_READS, 2, fact_type)


def recompute_frontier(fact_type):
    """The bound on the lfp for a change to `fact_type`: the constraints to re-check and the
    rules to re-fire — and the fact types those rules derive, which feed the next incremental
    round (the transitive closure that makes the bounded lfp reach its fixpoint). The
    rules→derives hop is a theta1 natural join over M, like every other frontier read."""
    rules = affected_rules(fact_type)
    joined = _S(A("COMP"), _S(A("ALPHA"), A(2)), T.NatJoin(1))       # π_ft(rules ⋈ ruleDerives)
    derives = from_lam(apply(joined, to_lam((tuple((r,) for r in rules), _RULE_DERIVES))))
    return {"constraints": affected_constraints(fact_type), "rules": rules, "derives": tuple(derives)}
