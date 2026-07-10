"""#18: the GENERIC constraint translator system:h_crows — one canon body the family
shares (h_uniqueness / h_mandatory are name aliases). Cooked ⟨decl_rows, mid, obj_specs⟩:
mid ⟨c, tail⟩ entries complete with the modality (apndr over distr), ⟨w, row⟩ pass through,
obj specs apply their canonical builder through DEFS. Optionality = the empty sequence —
the exactly-one mandatory rider is just more elements, no COND."""
import pyarest.prims  # noqa: F401
from pyarest import canon, compiler
from pyarest.lam import from_lam, to_lam, atom as A
from pyarest.reduce import apply as R

canon.load_all()

DECL = (("factType", ("User_has_Email", "{0} has {1}")),
        ("role", ("User_has_Email.1", "User_has_Email", 1, "User")),
        ("role", ("User_has_Email.2", "User_has_Email", 2, "Email")))

# uniqueness WITHOUT the rider (at most one)
UC = (DECL,
      (("c", ("User_has_Email_uc", "uniqueness", "User_has_Email")),
       ("w", ("spans", ("User_has_Email_uc", 1)))),
      (("User_has_Email_uc", "constraints:uniqueness", (1,)),))

# uniqueness WITH the exactly-one mandatory rider — more elements, same shape
UCM = (DECL,
       UC[1] + (("c", ("User_has_Email_mand", "mandatory", "User_has_Email", "User")),
                ("w", ("spans", ("User_has_Email_mand", 1)))),
       UC[2] + (("User_has_Email_mand", "constraints:scoped_mandatory_entities", "User"),
                ("User_has_Email_mand_e", "constraints:scoped_mandatory_facts", "User_has_Email")))

# bare mandatory (no decl — declared ft), empty decl exercises cat on phi
MAND = ((),
        (("c", ("R_mand", "mandatory", "R", "S")), ("w", ("spans", ("R_mand", 1)))),
        (("R_mand", "constraints:scoped_mandatory_entities", "S"),
         ("R_mand_e", "constraints:scoped_mandatory_facts", "R")))


def _canon(name, groups):
    r = R(A(name), to_lam((groups, (), "alethic")))
    asserts = [(x[0], tuple(x[1])) for x in from_lam(R(A(1), r))]
    return asserts, R(A(2), r)


def _host(groups):
    a, o = compiler._h_crows(groups, None, "alethic")
    return a, o


def test_crows_asserts_twin_host():
    for name, g in (("system:h_crows", UC), ("system:h_uniqueness", UCM),
                    ("system:h_mandatory", MAND)):
        assert _canon(name, g)[0] == _host(g)[0], name


def test_crows_shape():
    ca, _ = _canon("system:h_uniqueness", UCM)
    assert ca[0] == ("factType", ("User_has_Email", "{0} has {1}"))
    assert ca[3] == ("constraint", ("User_has_Email_uc", "uniqueness",
                                    "User_has_Email", "alethic"))
    assert ca[4] == ("spans", ("User_has_Email_uc", 1))
    assert ca[5] == ("constraint", ("User_has_Email_mand", "mandatory",
                                    "User_has_Email", "User", "alethic"))
    assert ca[6] == ("spans", ("User_has_Email_mand", 1))


def test_crows_uniqueness_obj_extensional():
    cobjs = _canon("system:h_crows", UC)[1]
    hobjs = _host(UC)[1]
    assert from_lam(R(A(1), R(A(1), cobjs))) == hobjs[0][0] == "User_has_Email_uc"
    pop = (("a", "x"), ("a", "y"), ("b", "z"))            # duplicate key "a"
    c_ans = from_lam(R(R(A(2), R(A(1), cobjs)), to_lam(pop)))
    h_ans = from_lam(R(hobjs[0][1], to_lam(pop)))
    assert c_ans == h_ans
    assert c_ans, "the duplicated key must be flagged (non-vacuous)"


def test_crows_cooks():
    # the cooks produce the crows shape from raw production groups
    class K2(set):
        pass
    # uniqueness cook needs a Known-like: use the real one via a tiny corpus
    from pyarest.compiler import _cook_mandatory, _Known
    k = _Known({"User", "Email"}, {}, set(), set())
    g = _cook_mandatory(("Each User", "has some Email"), k)
    assert g[1][0][0] == "c" and g[1][1][0] == "w"
    assert g[2][0][1] == "constraints:scoped_mandatory_entities"
