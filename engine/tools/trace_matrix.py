"""trace_matrix.py — the DO-178C batch trace for an app: requirement -> lambda
term -> evidence, emitted as one reviewable Markdown document.

The refinement chain IS the trace (Cor. Verbalization: compile is injective on
readings), and the Registry's `explain` verb already answers the derivation
chain live, per entity id. This tool is the BATCH form: one row per
REQUIREMENT — every fact-type reading, every constraint, every derivation
rule the compiled model M holds — traced forward to

  * the DERIVED artifacts: the definition cells the compile stored into the
    app's own D (Backus §13.3.5 — a constraint's cid, its attachment objects
    `<cid>_e` / `<cid>_df` / `<cid>_do` / `<cid>_a` / `<cid>_b` / `<cid>@<ft>`,
    a rule's cid and its semi-naive `<cid>~d<i>` delta variants), which ARE
    the lambda terms,
  * the canonical vocabulary those terms carry: the compile-time builder the
    compiler applies per M-fact kind (constraints:*, system:compile_rule*)
    and the canon names surviving inside the stored term (theta:*,
    constraints:*, system:*, ast:* — what the term applies through rho at
    evaluation),
  * the POPULATION evidence: row counts off the store, read partition-aware
    through the same path the Registry serves (ft_view for absorbed fact
    types, the cell otherwise),
  * the DOWNSTREAM trace for derived fact types: which rules feed them
    (ruleDerives) and read them (ruleReads) — explain's walk, batched,
  * the VERIFICATION column: which canon twin tests (engine/tests/
    test_*_canon.py) name the canonical terms a row's lambda carries. File
    names are found by scanning the test sources at generation time — listed,
    never fabricated.

Binary-side evidence is CITED, not fabricated: the lambda terms are the
certified artifacts; each host kernel (python, rust, csharp, java) consumes
them as data, and the cross-host differential + host-twin tests are the
object-code-level evidence this repo actually has. Section "Binary-side
evidence" lists exactly the differential test files present on disk and
states plainly what is not yet traced (no per-requirement object-code /
bitstream trace exists).

Usage:
    python engine/tools/trace_matrix.py --apps-dir <dir> <app> [-o trace.md]
        [--base none|default|<path>]   readings preloaded under the app
                                       (default none — cli.py's bare
                                       Registry; the live MCP server passes
                                       `default`)
        [--compile auto|always|never]  auto (default) compiles only when the
                                       app has no store yet
        [--verify]                     append Registry.verify's stored-vs-
                                       recomputed checks to the downstream
                                       trace (re-evaluates every audited rule)
        [--validate]                   append the settled-store violation
                                       annex (Registry.validate)

Read-only by design: the only writes are the app's own compile artifacts
(when compilation is needed) and the output document.
"""
import argparse
import datetime
import importlib.util
import json
import os
import re
import subprocess
import sys

# UTF-8 on both streams regardless of the console codepage (the cli.py
# lesson, 2026-07-09: readings and canon names carry non-cp1252 bytes).
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8")
    except Exception:
        pass

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # engine/

if "pyarest" not in sys.modules:
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(_ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(_ROOT, "python")])
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = mod
    spec.loader.exec_module(mod)

import pyarest.prims  # noqa: E402,F401
from pyarest import apps, defs, system  # noqa: E402
from pyarest.kernel import from_lam  # noqa: E402

_CANON = re.compile(r"^(theta|constraints|system|ast):[A-Za-z0-9_~]+$")
_MODALITIES = ("alethic", "deontic")


# ---------------------------------------------------------------- helpers
def _canon_atoms(tree, acc):
    """Every canon-namespaced name a from_lam object tree references."""
    if isinstance(tree, tuple):
        for x in tree:
            _canon_atoms(x, acc)
    elif isinstance(tree, str) and _CANON.match(tree):
        acc.add(tree)
    return acc


def _esc(text):
    """One Markdown table cell: pipes escaped, newlines collapsed."""
    return str(text).replace("|", "\\|").replace("\n", " ").strip()


def _code(name):
    return "`" + str(name).replace("|", "\\|") + "`"


def _codes(names, empty="—"):
    return ", ".join(_code(n) for n in names) if names else empty


def _git(args):
    try:
        out = subprocess.run(["git", "-C", _ROOT] + args, capture_output=True,
                             text=True, timeout=15)
        return out.stdout.strip() if out.returncode == 0 else None
    except Exception:
        return None


def _engine_version():
    try:
        text = open(os.path.join(_ROOT, "pyproject.toml"), encoding="utf-8").read()
        m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
        return m.group(1) if m else None
    except OSError:
        return None


# ------------------------------------------------- verification index
def _load_tests(tests_dir):
    """Two tiers of test sources: the canon twin tests (test_*_canon.py, the
    primary certification evidence) and every other test_*.py (secondary —
    listed in the index when it names a term the twins do not)."""
    canon, other = {}, {}
    if not os.path.isdir(tests_dir):
        return canon, other
    for fn in sorted(os.listdir(tests_dir)):
        if not (fn.startswith("test_") and fn.endswith(".py")):
            continue
        try:
            text = open(os.path.join(tests_dir, fn), encoding="utf-8").read()
        except OSError:
            continue
        (canon if fn.endswith("_canon.py") else other)[fn] = text
    return canon, other


def _tests_for(term, sources, memo):
    """Test files naming `term`: the full canonical name verbatim, or a call
    of its bare tail (the host binding — e.g. `uniqueness(` for
    constraints:uniqueness). Listed only when actually present in the file."""
    hit = memo.get(term)
    if hit is not None:
        return hit
    tail = term.split(":", 1)[1] if ":" in term else term
    call = re.compile(r"(?<![A-Za-z0-9_:])" + re.escape(tail) + r"\s*\(")
    files = [fn for fn, text in sources.items()
             if term in text or call.search(text)]
    memo[term] = files
    return files


_DIFFERENTIAL_TESTS = (
    "test_shared_scenarios.py", "test_polyglot.py", "test_csharp_kernel.py",
    "test_java_kernel.py", "test_canon_native.py",
    "test_derive_differential.py", "test_oracle.py",
)

_HOST_DIRS = ("python", "rust", "csharp", "java")


# ------------------------------------------------- model extraction
def _rows(D, name):
    return [tuple(r) if isinstance(r, tuple) else (r,)
            for r in system._pop_rows(D, name)]


def _pop_count(D, name, partition):
    """Row count through the Registry's own read discipline: an absorbed fact
    type reassembles via ft_view (the RMAP column), an own-cell one reads its
    cell (tools.read_pop's partition test)."""
    try:
        if partition.get(name, name) != name:
            return len(system.ft_view(D, name, partition))
        return len(system._pop_rows(D, name))
    except Exception:
        return None


def _scope_parts(crow):
    """⟨fact types, literal values⟩ a constraint row scopes. Scope columns sit
    between kind and modality (validate()'s walk); the exclusion family scopes
    a clause TUPLE of fact types, while a deontic row's tuple column carries
    the quoted VALUES of the operator (compiler._plan's deontic transform) —
    those are values, never fact types."""
    scope = crow[2:-1] if crow and crow[-1] in _MODALITIES else crow[2:]
    deontic = str(crow[1]).startswith("deontic") if len(crow) > 1 else False
    fts, values = [], []
    for i, part in enumerate(scope):
        for t in (part if isinstance(part, tuple) else (part,)):
            if not isinstance(t, str):
                continue
            if deontic and i > 0:
                values.append(t)
            else:
                fts.append(t)
    return fts, values


def _scope_fts(crow):
    return _scope_parts(crow)[0]


def _constraint_def_cells(cid, scope, defcells):
    """The definition cells one constraint stored into D: the cid itself plus
    the attachment names the compiler mints (mandatory's `_e`, deontic's
    `_df`/`_do`, equality's `_a`/`_b`, the scoped `@<ft>` family)."""
    cands = [cid, cid + "_e", cid + "_df", cid + "_do", cid + "_a", cid + "_b"]
    cands += [cid + "@" + ft for ft in scope]
    return [n for n in cands if n in defcells]


def _rule_def_cells(rid, natoms, defcells):
    cands = [rid] + [f"{rid}~d{i}" for i in range(1, max(natoms, 0) + 1)]
    return [n for n in cands if n in defcells]


def _canonical_reading(crow, players_of, value_specs):
    """The constraint's canonical verbalization, sourced the way the engine
    itself verbalizes (generator_cells' UC/MC wording; a deontic cid IS the
    original sentence; a value constraint's spec is its enumeration/range).
    Anything else renders as the canonical form kind(scope)."""
    cid, kind = crow[0], crow[1]
    scope = _scope_fts(crow)
    if str(kind).startswith("deontic"):
        return str(cid) if not str(cid).endswith(".") else str(cid)
    if kind == "value" and scope:
        spec = value_specs.get(scope[0])
        if spec:
            return f"The possible values of {scope[0]} are {spec}."
    players = players_of.get(scope[0], []) if scope else []
    if kind in ("uniqueness",) and len(players) >= 2:
        return f"Each {players[0]} has at most one {players[1]}."
    if kind == "mandatory" and len(players) >= 2:
        return f"Each {players[0]} has some {players[1]}."
    if kind == "subtype" and len(scope) >= 1:
        return f"Subtype link: {cid.replace('_', ' ')}."
    return f"{kind} over {', '.join(scope) if scope else cid}"


_VALUE_RANGE = re.compile(r"\.\.|\bat (least|most)\b")


def _constraint_family(crow, term, latest, value_specs):
    """The canonical builder the compiler applies for this constraint kind,
    named only when that definition exists in the loaded canon (guarded —
    never a guess). Deontic population form is the id primitive by design."""
    kind = str(crow[1])
    if kind.startswith("deontic"):
        if term == "id":
            return ["(id — population form)"]
        name = ("constraints:deontic_forbidden" if kind == "deontic_forbidden"
                else "constraints:deontic_obligatory_value")
        return [name] if name in latest else []
    if kind in ("uniqueness", "spanning_uniqueness"):
        return ["constraints:uniqueness"]
    if kind == "mandatory":
        return [n for n in ("constraints:scoped_mandatory_entities",
                            "constraints:scoped_mandatory_facts") if n in latest]
    if kind == "value":
        vt = crow[2] if len(crow) > 2 else None
        spec = value_specs.get(vt, "")
        name = ("constraints:value_range"
                if _VALUE_RANGE.search(str(spec or ""))
                else "constraints:value_enumeration")
        return [name] if name in latest else []
    name = "constraints:" + kind
    return [name] if name in latest else []


# ------------------------------------------------- document assembly
def build_matrix(reg, app, verify=False, validate=False, tests_dir=None):
    D = reg._load(app)
    partition = system.rmap_partition(D)
    defcells = defs._cells_of(D)

    fact_types = {}                       # ft -> reading template
    for f in _rows(D, "factType"):
        if f:
            fact_types[f[0]] = f[1] if len(f) > 1 else ""
    roles = {}
    for r in _rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    players_of = {ft: [p for (_i, p) in sorted(ps)] for ft, ps in roles.items()}
    derivation = {r[0]: r[1] for r in _rows(D, "derivation") if len(r) >= 2}
    constraints = [c for c in _rows(D, "constraint") if len(c) >= 2]
    value_specs = {r[0]: r[1] for r in _rows(D, "valueConstraint")
                   if len(r) >= 2}
    spans = {}
    for r in _rows(D, "spans"):
        if len(r) >= 2:
            spans.setdefault(r[0], []).append(r[1])
    derives = [r for r in _rows(D, "ruleDerives") if len(r) >= 2]
    head_of = {rid: head for (rid, head) in derives}
    rules_feeding = {}
    for rid, head in derives:
        rules_feeding.setdefault(head, []).append(rid)
    reads_of, rules_reading = {}, {}
    for r in _rows(D, "ruleReads"):
        if len(r) >= 2:
            reads_of.setdefault(r[0], []).append(r[1])
            rules_reading.setdefault(r[1], []).append(r[0])
    atom_counts = {}
    for r in _rows(D, "ruleAtom"):
        if len(r) >= 2:
            atom_counts[r[0]] = max(atom_counts.get(r[0], 0), int(r[1]))
    agg = {r[0] for r in _rows(D, "ruleAgg") if r}
    neg = {r[0] for r in _rows(D, "ruleNeg") if r}
    skolem = {r[0] for r in _rows(D, "ruleSkolem") if r}
    copies = {r[0] for r in _rows(D, "ruleCopies") if r}
    diags = {r[0]: r[1] for r in _rows(D, "ruleDiag") if len(r) >= 2}
    cons_on = {}
    for c in constraints:
        for ft in _scope_fts(c):
            cons_on.setdefault(ft, []).append(c[0])

    counts = {}

    def count_of(name):
        """Row count for ANY cell name (fact type, noun, value type),
        partition-aware, memoized."""
        if name not in counts:
            counts[name] = _pop_count(D, name, partition)
        return counts[name]

    def with_count(name):
        n = count_of(name)
        return _code(name) + ("" if n is None else f" ({n})")

    latest = defs.latest

    canon_tests, other_tests = _load_tests(
        tests_dir or os.path.join(_ROOT, "tests"))
    canon_memo, other_memo = {}, {}

    def reading_text(ft):
        tpl = str(fact_types.get(ft, ""))
        try:
            return tpl.format(*players_of.get(ft, []))
        except (IndexError, KeyError):
            return tpl

    def tree_of(name):
        try:
            return from_lam(defcells[name])
        except Exception:
            return None

    used_terms = {}                       # canon term -> requirement count

    def note_terms(terms):
        for t in terms:
            used_terms[t] = used_terms.get(t, 0) + 1

    def tests_cell(terms):
        files = sorted({fn for t in terms
                        for fn in _tests_for(t, canon_tests, canon_memo)})
        return _codes(files, empty="— (see §5)")

    lines = []
    w = lines.append

    # ---- header -------------------------------------------------------
    commit = _git(["rev-parse", "--short", "HEAD"]) or "unknown"
    dirty = bool(_git(["status", "--porcelain"]) or "") if commit != "unknown" else False
    branch = _git(["rev-parse", "--abbrev-ref", "HEAD"]) or "unknown"
    version = _engine_version() or "unknown"
    entries = []
    try:
        entries = reg._sink(app).read()
    except Exception:
        pass
    now = datetime.datetime.now(datetime.timezone.utc)

    w(f"# Trace Matrix — `{app}`")
    w("")
    w("DO-178C-style end-to-end trace: requirement (FORML2 reading in M) → "
      "lambda term (the definition cells the compile stored into the app's "
      "own D) → population and verification evidence. The refinement chain "
      "is the trace; this document is the batch form of the Registry's "
      "`explain` verb.")
    w("")
    w("| | |")
    w("|---|---|")
    w(f"| App | `{app}` (apps dir `{_esc(reg.root)}`) |")
    w(f"| Generated | {now.strftime('%Y-%m-%d %H:%M:%S')} UTC |")
    w(f"| Engine | pyarest {version}, commit `{commit}`"
      f"{' (dirty worktree)' if dirty else ''} on `{branch}` |")
    base_note = _esc(reg.base_dir) if reg.base_dir else "none"
    w(f"| Base readings preloaded | {base_note} |")
    w(f"| Store cells | {len(defcells)} |")
    w(f"| Requirements | {len(fact_types)} fact types, {len(constraints)} "
      f"constraints, {len(head_of)} derivation rules |")
    w(f"| Event log (audit trail) | {len(entries)} entries |")
    w("")
    w("Every artifact named below was read from the compiled store or the "
      "repository at generation time. Nothing in the lambda-term or "
      "verification columns is inferred from documentation alone; absent "
      "evidence renders as —.")
    w("")

    # ---- 1. fact types --------------------------------------------------
    derived_fts = [ft for ft in fact_types if ft in derivation]
    w("## 1. Requirements — fact-type readings")
    w("")
    w(f"{len(fact_types)} fact types ({len(derived_fts)} derived). Rows are "
      "store populations read partition-aware (an absorbed fact type counts "
      "its RMAP column via `ft_view`, exactly the Registry's query path). "
      "“Derived by” / “read by” are the `ruleDerives` / `ruleReads` M-facts "
      "— the same cells `explain` walks per id.")
    w("")
    w("| Fact type (requirement id) | Reading | Storage kind | Rows | "
      "Constraints on it | Derived by (rules) | Read by (rules) |")
    w("|---|---|---|---|---|---|---|")
    for ft in sorted(fact_types):
        kind = derivation.get(ft, "asserted")
        n = count_of(ft)
        w("| {} | {} | {} | {} | {} | {} | {} |".format(
            _code(ft), _esc(reading_text(ft)), _esc(kind),
            "?" if n is None else n,
            _codes(sorted(cons_on.get(ft, []))[:6]) +
            (f" (+{len(cons_on[ft]) - 6})" if len(cons_on.get(ft, [])) > 6 else ""),
            _codes(sorted(rules_feeding.get(ft, []))),
            _codes(sorted(rules_reading.get(ft, [])))))
    w("")

    # ---- 2. constraints --------------------------------------------------
    w("## 2. Requirements — constraints")
    w("")
    found_c = 0
    rows_c = []
    for c in sorted(constraints, key=lambda c: (str(c[1]), str(c[0]))):
        cid, kind = c[0], c[1]
        modality = c[-1] if c[-1] in _MODALITIES else ""
        scope, values = _scope_parts(c)
        cells = _constraint_def_cells(cid, scope, defcells)
        if cells:
            found_c += 1
        term_refs = set()
        first_tree = None
        for name in cells:
            t = tree_of(name)
            if first_tree is None:
                first_tree = t
            _canon_atoms(t, term_refs)
        family = _constraint_family(c, first_tree, latest, value_specs)
        terms = [f for f in family if ":" in f] + sorted(term_refs)
        note_terms(set(terms))
        scope_txt = ", ".join(with_count(ft) for ft in scope) or "—"
        if values:
            scope_txt += "; values: " + ", ".join(f"'{v}'" for v in values)
        span_txt = ("roles " + ",".join(str(p) for p in spans[cid])
                    if cid in spans else "")
        canon_cell = "; ".join(
            x for x in (("family: " + ", ".join(_code(f) for f in family))
                        if family else "",
                        ("term refs: " + _codes(sorted(term_refs)))
                        if term_refs else "") if x) or "—"
        rows_c.append("| {} | {} | {} | {} | {} | {} | {} | {} |".format(
            _code(cid), _esc(str(kind) + (" " + span_txt if span_txt else "")),
            modality or "—", scope_txt,
            _esc(_canonical_reading(c, players_of, value_specs)),
            _codes(cells, empty="— (not stored)"),
            canon_cell, tests_cell(terms)))
    w(f"{len(constraints)} constraints; lambda terms found in DEFS for "
      f"{found_c} of them. The cid IS the definition-cell name (Backus "
      "§13.3.5: a stored cell is `Def name ≡ ρ obj`); attachment objects "
      "(`_e`, `_df`, `_do`, `_a`, `_b`, `@<clause>`) are listed beside the "
      "cid. “family” names the canonical builder the compiler applies at "
      "ingest (it reduces away); “term refs” are the canon names the stored "
      "term still applies through rho at evaluation.")
    w("")
    w("| Constraint (cid) | Kind | Modality | Scope fact types (rows) | "
      "Canonical reading | Lambda terms in DEFS | Canonical builders | "
      "Twin tests |")
    w("|---|---|---|---|---|---|---|---|")
    lines.extend(rows_c)
    w("")

    # ---- 3. rules --------------------------------------------------------
    w("## 3. Requirements — derivation rules")
    w("")
    rows_r = []
    found_r = 0
    for rid in sorted(head_of):
        head = head_of[rid]
        natoms = atom_counts.get(rid, 0)
        cells = _rule_def_cells(rid, natoms, defcells)
        if cells:
            found_r += 1
        term_refs = set()
        for name in cells:
            _canon_atoms(tree_of(name), term_refs)
        flavor, builders = [], []
        if rid in agg:
            flavor.append("aggregate")
            builders.append("system:compile_agg_rule")
        elif rid in neg:
            flavor.append("negation")
            builders += ["system:compile_rule", "system:anti_wrap",
                         "theta:Project"]
        else:
            builders.append("system:compile_rule")
        if rid in skolem:
            flavor.append("existential (skolem)")
        if rid in copies:
            flavor.append("copy")
        if any(n.startswith(rid + "~d") for n in cells):
            builders.append("system:compile_rule_delta")
        if not flavor:
            flavor.append("positive")
        if rid in diags:
            flavor.append(f"UNCOMPILED — {diags[rid]}")
        terms = builders + sorted(term_refs)
        note_terms(set(terms))
        reads = sorted(set(reads_of.get(rid, [])))
        reads_txt = ", ".join(with_count(ft) for ft in reads) or "—"
        n_head = count_of(head)
        canon_cell = "; ".join(
            x for x in ("applied: " + _codes(builders),
                        ("term refs: " + _codes(sorted(term_refs)))
                        if term_refs else "") if x)
        rows_r.append("| {} | {} | {} | {} | {} | {} | {} | {} |".format(
            _code(rid), _code(head), "?" if n_head is None else n_head,
            reads_txt, _esc("; ".join(flavor) + f"; atoms: {natoms or '?'}"),
            _codes(cells, empty="— (not stored)"),
            canon_cell, tests_cell(terms)))
    w(f"{len(head_of)} rules; lambda terms found in DEFS for {found_r}. A "
      "rule's base object evaluates over D; its `~d<i>` variants are the "
      "semi-naive delta twins (one per body atom). “applied” names the "
      "canonical compile builder per the rule's own M-facts (`ruleAgg` → "
      "aggregate, `ruleNeg` → stratified negation, a `~d` cell → the delta "
      "builder); “term refs” are canon names inside the stored term.")
    w("")
    w("| Rule (cid) | Derives (head) | Head rows | Reads (fact types, rows) "
      "| Shape | Lambda terms in DEFS | Canonical builders | Twin tests |")
    w("|---|---|---|---|---|---|---|---|")
    lines.extend(rows_r)
    w("")

    # ---- 4. downstream trace ---------------------------------------------
    w("## 4. Downstream trace — derived fact types")
    w("")
    w("The requirement-to-requirement closure: for every derived fact type, "
      "the rules that feed it and the rules and constraints that consume it. "
      "This is the recomputation frontier the engine itself reads off "
      "`ruleReads`/`ruleDerives` (Cor. streaming).")
    w("")
    verify_col = {}
    if verify:
        try:
            for chk in reg.verify(app).get("checks", []):
                verify_col[chk["head"]] = chk
        except Exception as e:
            w(f"*verify unavailable: {type(e).__name__}: {e}*")
            w("")
    head_cols = ("| Derived fact type | Kind | Rows | Fed by (rules) | "
                 "Read by (rules) | Constraints on it |")
    if verify:
        head_cols += " Reproduced (verify) |"
    w(head_cols)
    w("|---|---|---|---|---|---|" + ("---|" if verify else ""))
    for ft in sorted(derived_fts):
        feeders = sorted(rules_feeding.get(ft, []))
        row = "| {} | {} | {} | {} | {} | {} |".format(
            _code(ft), _esc(derivation.get(ft, "")),
            count_of(ft) if count_of(ft) is not None else "?",
            _codes(feeders, empty="**— (no compiled rule: trace gap)**"),
            _codes(sorted(rules_reading.get(ft, []))),
            _codes(sorted(cons_on.get(ft, []))))
        if verify:
            chk = verify_col.get(ft)
            row += (" {} (stored {}, recomputed {}) |".format(
                "MATCH" if chk["match"] else "MISMATCH",
                chk["stored"], chk["recomputed"]) if chk else " n/a |")
        w(row)
    w("")
    if verify:
        w("*`verify` audits the heads whose recompute is destructive or owned "
          "(the sweep / dred / agg-whole classes plus owned keyed heads — "
          "`Registry.verify`'s audit set); other derived heads read n/a here "
          "and are covered by the fixpoint semantics itself (set-union "
          "merge is idempotent).*")
        w("")
    w("A derived fact type whose “fed by” column is a trace gap declares a "
      "derivation the compiler stored no rule object for — a requirement "
      "with no implementation artifact (a semi-derived head remains manually "
      "assertable, but its derivation half is missing). These rows demand "
      "review.")
    w("")

    # ---- 5. verification index --------------------------------------------
    w("## 5. Verification index — canonical terms to twin tests")
    w("")
    w("Each canonical term the matrix names, with the tests that pin it, "
      "scanned from engine/tests at generation time (a file is listed when "
      "it names the term verbatim or calls its host binding). Primary "
      "evidence is the canon twin suite (test_*_canon.py — the twin "
      "contract: a host's optimized form is held observationally equal to "
      "the canonical definition); the last column lists other engine tests "
      "naming the term. A term with neither is exercised only indirectly "
      "(through every compile and differential run) — an explicit twin "
      "test is the recommended close-out.")
    w("")
    w("| Canonical term | Named by requirements | Twin tests "
      "(test_*_canon.py) | Other engine tests |")
    w("|---|---|---|---|")
    for term in sorted(used_terms):
        cf = _tests_for(term, canon_tests, canon_memo)
        of = _tests_for(term, other_tests, other_memo)
        w(f"| {_code(term)} | {used_terms[term]} | {_codes(sorted(cf))} | "
          f"{_codes(sorted(of)[:6])}"
          f"{f' (+{len(of) - 6})' if len(of) > 6 else ''} |")
    w("")

    # ---- 6. binary-side evidence -------------------------------------------
    w("## 6. Binary-side evidence — host twins and the differential")
    w("")
    hosts = [h for h in _HOST_DIRS if os.path.isdir(os.path.join(_ROOT, h))]
    diffs = [t for t in _DIFFERENTIAL_TESTS
             if os.path.exists(os.path.join(_ROOT, "tests", t))]
    w("The lambda terms above are the certified artifacts. They are DATA: "
      "each host kernel reduces the same terms, and equivalence across hosts "
      "is held by differential tests over the shared scenario table, not by "
      "per-host reimplementation of the semantics.")
    w("")
    w(f"- Host kernels present in this checkout: "
      f"{', '.join(_code('engine/' + h) for h in hosts)}.")
    w(f"- Differential / host-twin suites present: {_codes(diffs)}.")
    w("- The sidecar `" + app + ".store.json` beside the .db carries the "
      "store and every compiled process definition as the wire scenario the "
      "Rust resident boots from — the same terms this matrix traced.")
    w("")
    w("**Not traced here (honest gap):** there is no per-requirement "
      "object-code or bitstream trace. The binary column of a full DO-178C "
      "table A-7 would require compiler-output evidence per host; what this "
      "repo certifies today is source-level canon equality (twin tests) plus "
      "cross-host observational equality (the differential suites named "
      "above).")
    w("")

    # ---- 7. optional violation annex ----------------------------------------
    if validate:
        w("## 7. Violation annex — the settled store under its own constraints")
        w("")
        try:
            v = reg.validate(app).get("violations", [])
            if not v:
                w("Registry.validate reports a clean bill: no non-empty "
                  "violation set over the settled store.")
            else:
                w("| Fact type | Constraint kinds | Alethic? | Offenders |")
                w("|---|---|---|---|")
                for item in v:
                    w("| {} | {} | {} | {} |".format(
                        _code(item.get("fact_type", "?")),
                        _esc(", ".join(item.get("kinds", []))),
                        "yes" if item.get("alethic") else "no (deontic drift)",
                        len(item.get("offenders", []))))
        except Exception as e:
            w(f"*validate unavailable: {type(e).__name__}: {e}*")
        w("")

    w("---")
    w(f"*Generated by `engine/tools/trace_matrix.py` — batch form of the "
      f"`explain` verb; per-id derivation chains remain available live via "
      f"`explain {app} <id>`.*")
    w("")
    return "\n".join(lines)


def main(argv):
    ap = argparse.ArgumentParser(
        prog="trace_matrix.py",
        description="Emit the DO-178C-style requirement→lambda→evidence "
                    "trace matrix for a compiled app, as Markdown.")
    ap.add_argument("--apps-dir", required=True)
    ap.add_argument("app")
    ap.add_argument("-o", "--out", default=None,
                    help="output file (default stdout)")
    ap.add_argument("--base", default="none",
                    help="'none' (bare Registry, cli.py's default), "
                         "'default' (the vendored shared/base, the live MCP "
                         "server's parity default), or a directory path")
    ap.add_argument("--compile", dest="compile_mode", default="auto",
                    choices=("auto", "always", "never"))
    ap.add_argument("--verify", action="store_true",
                    help="append stored-vs-recomputed rule checks (slow)")
    ap.add_argument("--validate", action="store_true",
                    help="append the settled-store violation annex (slow)")
    ap.add_argument("--tests-dir", default=None,
                    help="twin-test directory (default engine/tests)")
    args = ap.parse_args(argv[1:])

    if args.base == "none":
        base = None
    elif args.base == "default":
        base = apps.default_base()
    else:
        base = args.base
    reg = apps.Registry(args.apps_dir, base_dir=base)

    if args.compile_mode == "always":
        reg.compile(args.app)
    elif args.compile_mode == "auto" and not reg._storage(args.app).exists():
        reg.compile(args.app)

    try:
        doc = build_matrix(reg, args.app, verify=args.verify,
                           validate=args.validate, tests_dir=args.tests_dir)
    except FileNotFoundError as e:
        sys.stderr.write(f"trace_matrix: {e} (run with --compile auto)\n")
        return 1
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(doc)
        sys.stderr.write(f"wrote {args.out}\n")
    else:
        sys.stdout.write(doc)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
