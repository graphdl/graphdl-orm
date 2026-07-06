"""The polyglot differential: the SAME values — base-op probes, θ₁ joins, compiled
constraints, and the whitepaper machine's whole create handler — reduce under the
Python Scott mu and the Rust Scott mu (rust/src/main.rs, closures and Y all the way
down), and must agree exactly, bottoms included. No machine code exists in Rust: the
machine that advances there is the exported M-driven FFP object. Skipped cleanly when
the Rust binary has not been built (rust/ + cargo build --release)."""
import pytest
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import atom as A, to_lam, from_lam
from pyarest import ast, forml, system
from pyarest import canon as T
from pyarest import polyglot

pytestmark = pytest.mark.skipif(not polyglot.rust_available(),
                                reason="rust kernel not built (cd rust; cargo build --release)")


def S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _diff(D, cases):
    got = polyglot.run_rust(polyglot.export_scenario(D, cases))
    want = polyglot.python_ground_truth(D, cases)
    for (case, g, w) in zip(cases, got, want):
        assert g == w, f"kernel divergence on {from_lam(case[0])!r}: rust={g!r} python={w!r}"


def test_base_ops_agree_across_kernels():
    D = L.SEQ(L.NIL)
    pop = to_lam((("a", 1), ("b", 2), ("a", 3)))
    comp = S(A("COMP"), A(1), A("tl"))
    consx = S(A("CONS"), A(2), A(1), S(A("CONST"), A("k")))
    cond = S(A("COND"), A("null"), S(A("CONST"), A("empty")), A("length"))
    ins = S(A("INSERT"), A("+"))
    alpha = S(A("ALPHA"), A(2))
    spin = S(A("WHILE"), S(A("CONST"), A("T")), A("id"))
    countdown = S(A("WHILE"),
                  S(A("COMP"), A("gt"), S(A("CONS"), A("id"), S(A("CONST"), A(0)))),
                  S(A("COMP"), A("-"), S(A("CONS"), A("id"), S(A("CONST"), A(1)))))
    cases = [
        (A(2), to_lam(("x", "y", "z")), None),
        (A(5), to_lam(("x", "y")), None),                     # short: ⊥ both sides
        (A("tl"), to_lam(("x",)), None),
        (A("tl"), A("atomic"), None),                         # ⊥
        (A("eq"), to_lam((1, 1.0)), None),                    # int vs float: F (NATEQ)
        (A("eq"), to_lam((1, 1)), None),
        (A("+"), to_lam((2, 3)), None),                       # int stays int
        (A("+"), to_lam((2, 1.5)), None),                     # promotes to float
        (A("+"), to_lam(("120", "30")), None),                # LEXICAL ints coerce (150)
        (A("+"), to_lam(("2", 1.5)), None),                   # lexical + float: 3.5
        (A("+"), to_lam(("a", "3")), None),                   # non-numeric: ⊥ both
        (A("lt"), to_lam(("9", "10")), None),                 # comparators COERCE: 9 < 10
        (A("lt"), to_lam((4997, "11000")), None),             # mixed int/lexical (claude's totals)
        (A("ge"), to_lam(("305", "1190")), None),             # multi-digit text orders numerically
        (A("div"), to_lam((4, 2)), None),                     # Python /: float
        (A("div"), to_lam((4, 0)), None),                     # ÷0 = ⊥
        (A("lt"), to_lam(("apple", "pear")), None),           # string ordering
        (A("trans"), to_lam(((1, 2), (3, 4), (5, 6))), None),
        (A("trans"), to_lam(((1, 2), (3,))), None),           # ragged: ⊥
        (A("apndl"), to_lam(("h", ("a", "b"))), None),
        (A("distr"), to_lam((("a", "b"), "y")), None),
        (A("rotr"), to_lam((1, 2, 3)), None),
        (A("1r"), to_lam((1, 2, 3)), None),
        (A("tlr"), to_lam(()), None),                         # tlr:φ = ⊥
        (comp, to_lam(("a", "b", "c")), None),
        (consx, to_lam(("p", "q")), None),
        (cond, to_lam(()), None),
        (cond, to_lam((1, 2, 3)), None),
        (ins, to_lam((1, 2, 3, 4)), None),
        (alpha, pop, None),
        (A("apply"), S(A(1), to_lam(("only",))), None),       # dynamic selection
        (spin, A(7), 3000),                                   # fuel bottoms both kernels
        (countdown, L.atom(5), 200000),                       # terminating WHILE agrees
    ]
    _diff(D, cases)


def test_theta_and_constraints_agree_across_kernels():
    from pyarest import constraints as C
    D = L.SEQ(L.NIL)
    pop = to_lam((("o1", "c1"), ("o2", "c2"), ("o1", "c3")))
    other = to_lam((("o1", "x"), ("o3", "y")))
    cases = [
        (T.Project([2, 1]), pop, None),
        (T.Filter(S(A("COMP"), A("eq"), S(A("CONS"), A(1), S(A("CONST"), A("o1"))))), pop, None),
        (T.NatJoin(1), S(pop, other), None),
        (T.JoinOn(((1, 1),), (2,)), S(pop, other), None),     # the general Codd join:
        (T.JoinOn(((1, 1), (2, 2)), ()), S(pop, pop), None),  # multi-column, semijoin,
        (T.JoinOn((), (1,)), S(pop, other), None),            # and the cross product —
        (C.uniqueness([1]), pop, None),                       # same prims, certified
        (T.member, S(A("o2"), to_lam(("o1", "o2"))), None),
    ]
    _diff(D, cases)


ORDER = """Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Customer places Order.
Customer ships Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
"""


SFT = "Order_is_currently_in_Status"


def _order_setup():
    """The whitepaper store with status(e) on its RMAP column: o1 In Cart, plus
    the machine slot ⟨table, col, width⟩ computed off the partition."""
    from pyarest.reduce import apply
    D, _ = forml.compile_model(ORDER)
    D = system.layout_cells(system.status_facts(D))
    D = apply(A(2), system.create(D, SFT, to_lam(("o1", "In Cart"))))
    part = system.rmap_partition(D)
    scols = system.table_columns(part, "Order")
    return D, ("Order", 2 + scols.index(SFT), 1 + len(scols))


def test_the_machine_advances_identically_on_rust_closures():
    # the killer case: the whitepaper create handler — commit, machine advance, links —
    # is ONE exported FFP object; Rust runs it with zero machine code of its own
    D, slot = _order_setup()
    trans_of = system.transitions_of(to_lam(system.sm_triples(D)), 2)
    handler = ast.build_system(
        cell_name="Customer_places_Order",
        machine=(slot, system.machine_step("Customer_places_Order"), 2),
        mealy_obj=system.mealy_step("Customer_places_Order"),
        links_obj=trans_of)
    x = S(to_lam(("c1", "o1")), D)
    _diff(D, [(handler, x, None)])


def test_rules_resolve_through_the_step_frame_on_both_kernels():
    SUPER = """Party is an entity type.
Person is an entity type.
Person is a subtype of Party.
State Machine Definition 'Party' is for Noun 'Party'.
"""
    D, _ = forml.compile_model(SUPER)
    D = system.governance_rules(D)
    # f is an ATOM: it must resolve through the step frame's DEFS in both kernels
    _diff(D, [(A("governedBy_rule_base"), D, None)])


def test_overrides_and_canonical_terms_agree():
    # the universal interface's contract, cross-host: the override twins (native
    # containers) and the canonical combinator terms answer identically
    D = L.SEQ(L.NIL)
    cases = [
        (A("apndr"), to_lam((("a", "b"), "c")), None),
        (A("distl"), to_lam(("x", ("p", "q", "r"))), None),
        (A("trans"), to_lam(((1, 2, 3), (4, 5, 6))), None),
        (A("rotr"), to_lam((1, 2, 3, 4)), None),
        (A("tlr"), to_lam((1, 2, 3)), None),
        (S(A("COMP"), A("reverse"), A("cat")), S(to_lam((1, 2)), to_lam((3, 4))), None),
    ]
    sc = polyglot.export_scenario(D, cases)
    on = polyglot.run_rust(sc)
    sc["overrides"] = 0
    off = polyglot.run_rust(sc)
    want = polyglot.python_ground_truth(D, cases)
    assert on == off == want


def test_benchmark_the_flex(capsys):
    # correctness-asserted timing: the whitepaper machine step, N times, three ways
    import time
    from pyarest.reduce import apply as fast_apply
    D, slot = _order_setup()
    handler = ast.build_system(
        cell_name="Customer_places_Order",
        machine=(slot, system.machine_step("Customer_places_Order"), 2))
    x = S(to_lam(("c1", "o1")), D)
    N = 10
    N_TRUTH = 2                                               # ground truth is deliberately the
    cases = [(handler, x, None)] * N                          # slow reference; two runs prove
    cases_truth = cases[:N_TRUTH]                             # stability, ten would prove patience

    t0 = time.perf_counter()
    want_truth = polyglot.python_ground_truth(D, cases_truth)
    t_scott_py = time.perf_counter() - t0
    assert len(set(map(repr, want_truth))) == 1               # identical cases, identical truth
    want = [want_truth[0]] * N

    from pyarest import defs as _d
    t0 = time.perf_counter()
    for _ in range(N):
        with _d.step(D):
            fast_apply(handler, x)
    t_delta_py = time.perf_counter() - t0

    sc = polyglot.export_scenario(D, cases)
    t0 = time.perf_counter()
    on = polyglot.run_rust(sc)
    t_rust_on = time.perf_counter() - t0
    sc["overrides"] = 0
    t0 = time.perf_counter()
    off = polyglot.run_rust(sc)
    t_rust_off = time.perf_counter() - t0

    ses = polyglot.RustSession()
    try:
        ses.set_store(D)
        fact = to_lam(("c1", "o1"))
        ses.run_facts(handler, [fact])                        # warm
        t0 = time.perf_counter()
        resident = ses.run_facts(handler, [fact] * N)
        t_rust_res = time.perf_counter() - t0
        ses.run_facts(handler, [fact], engine="native")       # warm
        t0 = time.perf_counter()
        native = ses.run_facts(handler, [fact] * N, engine="native")
        t_rust_native = time.perf_counter() - t0
    finally:
        ses.close()
    assert on == off == want                                  # correctness before speed
    assert resident == want                                   # the resident runner agrees
    assert native == want                                     # the native carrier agrees
    with capsys.disabled():
        print(f"\n[bench] machine step x{N}: "
              f"py-scott(x{N_TRUTH})={t_scott_py:.2f}s py-delta={t_delta_py:.2f}s "
              f"rust-canonical={t_rust_off:.2f}s rust-overrides={t_rust_on:.2f}s "
              f"rust-resident={t_rust_res:.3f}s rust-native={t_rust_native:.3f}s "
              f"(one-shot rust pays spawn + D serialization; resident retains the store)")


def test_the_native_carrier_agrees_three_ways():
    # the deepest override: the native-carrier machine (delta.py's analog) must equal
    # the Scott closures must equal Python — same scenarios, engine selected per request
    D, slot = _order_setup()
    handler = ast.build_system(
        cell_name="Customer_places_Order",
        machine=(slot, system.machine_step("Customer_places_Order"), 2),
        mealy_obj=system.mealy_step("Customer_places_Order"))
    spin = S(A("WHILE"), S(A("CONST"), A("T")), A("id"))
    cases = [
        (handler, S(to_lam(("c1", "o1")), D), None),
        (S(A("INSERT"), A("+")), to_lam((1, 2, 3, 4)), None),
        (S(A("COND"), A("null"), S(A("CONST"), A("e")), A("length")), to_lam((1, 2)), None),
        (A("trans"), to_lam(((1, 2), (3,))), None),           # ragged: ⊥
        (spin, A(7), 3000),                                   # fuel bottoms
        (A("governedBy_rule_base"), D, None),                 # step-frame resolution
    ]
    want = polyglot.python_ground_truth(D, cases)
    sc = polyglot.export_scenario(D, cases)
    scott = polyglot.run_rust(sc)
    sc["engine"] = "native"
    native = polyglot.run_rust(sc)
    assert scott == want
    assert native == want


def test_the_resident_serves_an_absorbed_apply_natively(tmp_path):
    """The write-path flip's first landing, rust-side: with delegation
    DISABLED (a bogus --python interpreter), the resident commits an ABSORBED
    fact — the stored create:<ft> handler computes its fact-dependent cell
    (cellkey) at reduce time, the machine leg advances the status column, the
    native derive runs, and the sidecar persists — all without the compiler
    host. A committed receipt here PROVES the native path."""
    import json
    import os
    import subprocess
    import pytest as _pytest
    from pyarest import apps as _apps, canon as _canon
    exe = _canon.rust_bin("arestlam")
    if not os.path.exists(exe):
        _pytest.skip("rust kernel not built")
    root = str(tmp_path / "apps")
    d = os.path.join(root, "flow", "readings")
    os.makedirs(d)
    with open(os.path.join(d, "app.md"), "w", encoding="utf-8") as f:
        f.write("""Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Order is paid by Customer.
Each Order is paid by exactly one Customer.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'pay' is from Status 'In Cart'.
Transition 'pay' is to Status 'Paid'.
Transition 'pay' is triggered by Fact Type 'Order is paid by Customer'.
""")
    reg = _apps.Registry(root, cache_dir=str(tmp_path / "fz"))
    reg.compile("flow")                                       # sidecar for the resident
    reg.apply("flow", "Order_is_currently_in_Status", ("o1", "In Cart"))
    proc = subprocess.Popen(
        [exe, "--mcp", "--apps-dir", root, "--python", "no-such-interpreter"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        encoding="utf-8")

    def rpc(mid, method, params=None):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": mid,
                                     "method": method,
                                     "params": params or {}}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    try:
        rpc(1, "initialize", {"protocolVersion": "2024-11-05"})
        rpc(2, "tools/call", {"name": "apps_use",
                              "arguments": {"name": "flow"}})
        ap = rpc(3, "tools/call", {"name": "apply", "arguments": {
            "app": "flow", "fact_type": "Order_is_paid_by_Customer",
            "fact": ["o1", "c1"]}})
        receipt = json.loads(ap["result"]["content"][0]["text"])
        assert receipt["committed"] is True                   # native, or it failed
        q = rpc(4, "tools/call", {"name": "query", "arguments": {
            "app": "flow", "fact_type": "Order_is_paid_by_Customer"}})
        rows = json.loads(q["result"]["content"][0]["text"])["rows"]
        assert ["o1", "c1"] in [list(r) for r in rows]
    finally:
        proc.kill()
    # the resident persisted to the SOURCE OF TRUTH (the event log + the
    # sidecar; the .db is disposable): a python recompile replays the
    # resident-written event through the same create, machine advance included
    reg.compile("flow")
    D = reg._load("flow")
    from pyarest import system as _sys
    assert ("o1", "Paid") in _sys.ft_view(
        D, "Order_is_currently_in_Status", _sys.rmap_partition(D))
