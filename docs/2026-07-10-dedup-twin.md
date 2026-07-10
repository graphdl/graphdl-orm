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

## Test status (this session)

- PASS: `test_theta` (dedup semantics), `test_csharp_kernel` + `test_java_kernel` (the closure
  fixpoint — byte-parity Python-twin vs C#/Java canon), and all 165 `-k canon` tests.
- FAIL/HANG: exactly one consumer among `{test_delta, test_entity_view, test_intersection,
  test_showui, test_skolem_prim}` hangs (infinite loop) — NOT yet isolated (ran out of budget).
  Likely a row shape my hashing/`==` treats differently than canon `EQOBJ` (nested rows, or an
  atom whose native `==` diverges from NATEQ), or a dedup applied to a non-population operand.

## To finish (a focused hour)

1. Re-apply the twin. Isolate the hanging consumer (`pytest tests/test_<one>.py -x`, one file at a
   time — pytest-timeout is NOT installed here).
2. Fix the edge case: most likely normalize the hash key through the same equality the canon uses
   (compare/hash on the fully-decoded native form; fall back to the O(n²) `==` branch for any row
   that is not cleanly hashable), or guard the operand shape.
3. Full gate: `-k "canon or kernel or forml or compile or ported or intersection or delta or
   entity_view or theta"` plus a real app compile, and re-run def_profile to confirm the 964k
   `keep_eq` collapses. Then commit signed.

Expected payoff: the derive pipeline's dominant O(n²) becomes O(n) on every reducing host.
