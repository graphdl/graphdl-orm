"""THE COVERAGE GATE (Samuel, 2026-07-08: "Is there a test that makes
sure that all functionality is available in the shared canon?"): every
name a host kernel dispatches is one of exactly three things —

  (1) the FP BASE vocabulary: the substrate the canon is written in
      (Backus's algebra: selectors, sequence ops, logic, arithmetic);
  (2) a declared D5 BOUNDARY transducer: transduction only, no policy
      (the lex family, cellkey, escape_html, skolem, strip_prefix;
      stage1_fields by the 2026-07-07 ruling — "a canonical composition
      is not owed at the boundary, exactly as lex itself");
  (3) CANON-NAMED: a certified-equal override for speed whose DEF of
      record must exist in shared/*.canon, twinned by its own pin.

A host op that fits none of these fails here: functionality lands in
the shared lambda source first, or it does not land. The semantic half
of the discipline stays with the per-override twin pins (classify,
render, vb_fetch, entity_view, ...); this gate is the structural half —
nothing can even be NAMED host-side without a canon story."""
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# the term grammar's combinator tags (evaluator internals, not ops; a
# host may or may not dispatch them by name — DEFS, for one, is
# structural in the java/C# reducers)
TAGS = {"ALPHA", "BU", "COMP", "COND", "CONS", "CONST", "INSERT",
        "WHILE", "DEFS"}
BASE = {"id", "tl", "atom", "null", "eq", "apndl", "apndr", "distl",
        "distr", "length", "reverse", "cat", "not", "and", "or", "1r",
        "tlr", "rotl", "rotr", "trans", "+", "-", "*", "div", "ge",
        "gt", "le", "lt", "apply"}
D5 = {"cellkey", "lex", "slug", "implode", "escape_html", "skolem",
      "strip_prefix", "stage1_fields",
      # the JSON view emitter (2026-07-09): the react/Worker target
      # consumes the element TREE itself — its "render" is the tree's
      # JSON spelling, pure format transduction (the implode class)
      "render:json"}
# certified-equal overrides: the host name -> the canon DEF of record
OVERRIDES = {
    "render:html": "system:render_html",
    "system:vb_fetch": "system:vb_fetch",
    "system:entity_view": "system:entity_view",
    "system:ev_cols": "system:ev_cols",
    "get_view": "system:entity_view",
    "_classify_heads": "system:classify_heads",
    "verify": "system:verify_store",
    # theta join/dedup primitives: canon DEFs (arest.canon) with certified-equal
    # native overrides in Rust `fn prim`, each gated by the `theta_arms_off` kill
    # switch (flip it and the differential oracle falls back to the canon DEF) —
    # the same certified-twin pattern as system:ev_cols above, the "fast override
    # per platform" for the hot join/dedup path (the store-twin / join slices).
    "theta:append_phi": "theta:append_phi",
    "theta:dedup": "theta:dedup",
    "theta:flatten": "theta:flatten",
    "theta:join_combine": "theta:join_combine",
    "theta:member": "theta:member",
}


def _canon_defs():
    names = set()
    shared = os.path.join(ROOT, "shared")
    for f in os.listdir(shared):
        if f.endswith(".canon"):
            src = open(os.path.join(shared, f), encoding="utf-8").read()
            names |= set(re.findall(r'DEF\("([^"]+)"', src))
    return names


def _src(*parts):
    return open(os.path.join(ROOT, *parts), encoding="utf-8").read()


def _rust_ops():
    src = _src("rust", "src", "main.rs")
    ops = set(re.findall(r'register\("([^"]+)"', src))
    # DIRECT arms of fn prim's dispatch match only, by BRACE DEPTH
    # twice over (indent and next-method heuristics both break: prim
    # bodies nest matches and tuple data — "unary"/"ref" inside
    # entity_view are values — and top-level helper fns sit between
    # methods). First bound fn prim at ITS closing brace, then walk the
    # dispatch block collecting arms entered at depth 1.
    i = src.find("fn prim(&self")
    depth = 0
    end = i
    opened = False
    for off, ch in enumerate(src[i:], start=i):
        if ch == "{":
            depth += 1
            opened = True
        elif ch == "}":
            depth -= 1
            if opened and depth == 0:
                end = off
                break
    body = src[i:end]
    k = body.find("match s {")
    depth = 0
    for line in body[k:].splitlines():
        if depth == 1 and "=>" in line and re.match(r'\s*"', line):
            for lit in re.findall(r'"([^"]+)"', line.split("=>")[0]):
                ops.add(lit)
        depth += line.count("{") - line.count("}")
    return ops


def _java_ops():
    return set(re.findall(r'name\.equals\("([^"]+)"\)',
                          _src("java", "Reducer.java")))


def _csharp_ops():
    return set(re.findall(r'case "([^"]+)":', _src("csharp", "Reducer.cs")))


def _python_ops():
    return set(re.findall(r'register\("([^"]+)"',
                          _src("python", "engine.py")))


def test_every_kernel_dispatches_the_shared_vocabulary():
    # the intersection contract's op half: the same base + boundary
    # vocabulary in all four kernels (python's BASE is structural in
    # prims.py, so python owes only the boundary set here)
    want = BASE | D5
    for name, ops in (("rust", _rust_ops()), ("java", _java_ops()),
                      ("csharp", _csharp_ops())):
        missing = want - ops
        assert not missing, f"{name} kernel lacks shared ops: {sorted(missing)}"
    missing = D5 - _python_ops()
    assert not missing, f"python host lacks boundary ops: {sorted(missing)}"


def test_no_host_op_escapes_the_discipline():
    allowed = BASE | D5 | TAGS | set(OVERRIDES)
    for name, ops in (("rust", _rust_ops()), ("java", _java_ops()),
                      ("csharp", _csharp_ops()), ("python", _python_ops())):
        stray = ops - allowed
        assert not stray, (
            f"{name} dispatches ops with no canon story: {sorted(stray)} — "
            "define the meaning in shared/*.canon (canon-named override) "
            "or declare the D5 transducer here with its ruling")


def test_canon_named_overrides_have_their_defs():
    defs = _canon_defs()
    for host_name, canon_name in OVERRIDES.items():
        assert canon_name in defs, (
            f"override {host_name!r} names {canon_name!r} but shared/*.canon "
            "carries no such DEF — the meaning must land in canon first")
