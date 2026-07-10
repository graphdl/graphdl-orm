# theta:dedup native twin — characterized, 90% landed, one edge case open (#20)

Design/handoff record, 2026-07-10. The measured next canon-perf win after `ast:Pop` (4bcbdbcf).
Do NOT re-derive this — it is done bar one consumer edge case and a full gate.

## Why

Instrumented DEF resolutions over a full `identity` compile (wrap `_d_store()` in a counting
dict): **`theta:keep_eq` resolves 964k times, ~60x the next theta op.** It traces to
`theta:dedup` (theta.canon, Codd 2.1.2 duplicate elimination):

    dedup = INSERT(COND(member, N2, apndl)) ∘ append_phi     # fold: skip a row if already present
    member = not ∘ null ∘ filter_eq ∘ distl                  # re-filters the WHOLE accumulator, O(n)

So dedup is **O(n²)** — n membership checks, each O(n) — and drives the 964k `keep_eq`. The pure
lambda cannot hash, so per the directive ("preferably by canon, otherwise a DEFS override; register
fast overrides per platform") the fix is a native hash twin. Codd: the logical operator's meaning
stays the canon; the physical hash is the host twin.

## Exact semantics (pinned empirically, dedup_probe)

Canon `dedup` keeps each row's **LAST occurrence, in list order** (right-fold-prepend):

    (a,b,a)     -> (b,a)
    (a,b,c,a)   -> (b,c,a)
    (a,a,b)     -> (a,b)
    (b,a,b,a)   -> (b,a)

## The twin (kernel.py, before register_base; call after register_base())

    def _theta_dedup(mu, o):
        native = o if type(o) is tuple else from_lam(o)
        if type(native) is not tuple:
            return o
        try:
            pos = {}
            for i, r in enumerate(native):
                pos[r] = i
            out = tuple(r for i, r in enumerate(native) if pos[r] == i)
        except TypeError:                                    # unhashable row: same last-occ, O(n²)
            out = tuple(r for i, r in enumerate(native) if not any(r == y for y in native[i + 1:]))
        return out if type(o) is tuple else to_lam(out)

    override("theta:dedup", _theta_dedup)     # NOT register(): override is the (mu, operand)->value
                                              # fast-carrier twin; register() is curried/Scott and
                                              # bridges wrong (the first attempt's hang).

Two gotchas already solved: (1) `override`, not `register` (right table / calling convention);
(2) decode-both — the operand arrives native (tuple) on the delta carrier AND Scott on the reduce
path, so decode via `from_lam` when it is not a tuple and re-encode via `to_lam` (same cons-list
the canon apndl-fold builds, so fixpoint convergence holds).

## Test status — RESOLVED (this session, fully traced)

The twin is CORRECT. Nothing about it diverges.

- PASS: `test_theta`, `test_csharp_kernel` + `test_java_kernel` (the closure fixpoint, byte-parity
  Python-twin vs C#/Java canon), all 165 `-k canon` tests, and — crucially — `test_intersection`'s
  own reduction, INCLUDING the Rust differential, once the recursion limit is raised.
- The "hang" was NOT a bug and NOT builder-equivalence. It is **`FETCH` recursion depth**
  (kernel.py:247, the Y-recursive Scott-store lookup, one Python frame per `_store` entry). The
  `override` call bumps `version`, which makes the differential test's repeated `canon.load_all`
  regrow `_store`, so `FETCH` walks ~4000 deep and blows Python's DEFAULT-1000 limit. Measured:
  RecursionError at limit 1200/2000, PASS at 4000/8000/200000 (actual depth ~4000, no segfault).
  Isolated by running each consumer alone: only `test_intersection` (repeated load_all) trips it;
  `test_delta/entity_view/showui/skolem` pass. It is finite-but-deep, not a loop.

## The real conclusion — the Python twin is the wrong host

A Python `override` lands only in `defs.fast`, which is Python-process-local. `export_scenario`
(tools.py:259) ships `defs.latest` (the compiled CANON), NOT `fast`, so the Rust/C#/Java kernels
still reduce the O(n²) canon dedup. **This twin speeds only the Python reference host.** Per the
directive (target the Rust CLI and the canon, not the reference), that is not the production win,
and it is not worth a C-stack-adjacent global recursion bump to land it.

The canon dedup is irreducibly O(n²) in the pure lambda (no hashing). The fix is a per-host native
twin. This document has already done the hard, portable parts: the exact last-occurrence semantics
(byte-exact, dedup_probe), the certification (oracle + 165 canon at limit >=4000), and the two
mechanism gotchas. What remains is per host.

## CLOSED — no Rust twin needed; production already dedups natively

Verified in the Rust engine: `op_run_rules` (main.rs:3066) outputs rules "deduplicated by row key
in FIRST-seen order" — a native `HashSet`/`BTreeSet` dedup, O(n). The Rust runtime does NOT reduce
the canon `theta:dedup`; it dedups in host code. So there is no Rust twin to build — production is
already O(n). (Note: Rust keeps first-seen, the canon keeps last-seen; identical as a SET, which is
all set-semantic dedup needs — the two are different code paths, the canon reduction only exercised
by the polyglot differential.)

The canon `theta:dedup`'s O(n²) is therefore reduced by exactly ONE host: the Python REFERENCE. Per
the directive (don't optimize the reference), that cost is off-target by design. **Nothing to do for
production dedup.** The whole thread reduces to: the reference is slow on dedup; the reference is not
the target; the target (Rust) is already fast.

If the Python reference compile is ever wanted fast for dev-iteration (explicitly off the production
path), the Python twin above lands with `sys.setrecursionlimit(~8000)` at kernel.py import — but that
is a reference convenience, not a #20 production item. `FETCH` (kernel.py:247) going iterative would
similarly harden the reference against store-size recursion depth. Neither is production work.

Lesson for #20: the Rust engine already carries the native optimizations (dedup, store) that make it
fast; the Python reference's O(n²) costs are reference-specific. The production compile-perf win is
COMPLETING the Rust-native compile (it delegates to Python today), not optimizing the canon or Python.
