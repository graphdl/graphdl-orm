# AREST: Lazy Existential / Skolem Derivation Head (design) — task-970

> **Status: DESIGN + MINIMAL INCREMENT.** This reading specifies the
> existential-rule (TGD) derivation *head* with deterministic Skolem
> value-invention — the capability the view-projection follow-on slices
> (934-2 list/detail, 934-3 menu) need. It grounds
> `readings/ui/view-projection-design.md` §4.5 / §4.6 and the 934-1
> SLICE-1 lazy-view mechanism (`readings/ui/view-projection.md`).
>
> The companion code lands the **mechanism** (the `skolem` Platform
> primitive + the compiler's skolem-binding emission in the 1-antecedent
> fanout) plus a precise `#[ignore]`d spec test in
> `compile_explicit_derivation_tests.rs`. The FORML 2 *surface syntax*
> for declaring a fresh head variable is the remaining parser work,
> spelled out in §5.

## 1. The capability (what is genuinely new)

Today an AREST derivation rule head binds only nouns that appear as
roles in the ANTECEDENT (directly, or renamed via a `that X` / `X is Y`
anaphora clause). Every consequent role's value is *projected from* an
antecedent binding (`role_value_by_name`, compile.rs ~5188) or pinned to
a literal (`consequent_role_literals`).

The view-projection menu rule (design §4.5) needs a head whose
CONSEQUENT introduces a **fresh entity** not in the antecedent — one
`ViewElement` per matching `(View, Transition)` binding:

```
ViewElement(E) renders Transition(Tr)
and ViewElement(E) has Component Role 'button'
  IF  View(Vw) is for Noun(N)
  and Vw has View Kind 'menu'
  and Transition(Tr) is defined in the State Machine for N
  and Tr is from the current Status.
```

`E` appears **only** in the head. This is a **tuple-generating
dependency (TGD)** with an existential head variable — the standard
"existential rule" of the chase. Naively the existential is satisfied by
inventing a *new* labelled null each pass, which never terminates
(the oblivious chase). AREST instead uses the **semi-oblivious / Skolem
chase**: `E`'s identity is a deterministic function of the frontier
(the head variables that DO occur in the antecedent), so:

> **same frontier binding → same Skolem id → the SAME `ViewElement`
> across passes → NO duplicate.**

Idempotence is the correctness crux. It is what makes a lazy re-derivation
(every `get`/`actions` read recomputes the view) safe and stable.

## 2. The Skolem id scheme

`E.id = "ve_" ++ fnv1a64_hex( skolem_seed )` where `skolem_seed` is the
ordered tuple of the **frontier values** — the antecedent-bound role
values that distinguish one head instance from another. For the §4.5
menu rule the frontier is `(View.Name, Transition.id)`; concretely the
seed is the concatenation of those two atom values with a `|` separator,
hashed with the **same FNV-1a-64** the forward-chain dedup
(`evaluate::fact_key`) and `ast::synthesize_fact_id` already use, so the
id is stable across runs and across code paths.

Properties:

- **Deterministic** — pure function of atom values, no clock / counter /
  RNG. Re-derivation reproduces the id exactly.
- **Collision-resistant** — 64-bit FNV-1a; at view scales (≤10⁴
  elements) P(collision) ≈ 10⁻¹², same bound `evaluate.rs:550` documents.
- **Frontier-keyed** — two distinct `(View, Transition)` pairs get
  distinct ids; the SAME pair (re-read, or re-derived in a later pass)
  gets the SAME id. This is exactly the semi-oblivious chase's
  "one labelled null per frontier tuple" rule.

The hash is computed at **resolve time** inside a new
`Func::Platform("skolem")` leaf, so it is:

- introspectable (round-trips through `func_to_object` as
  `platform:skolem`, like every other Platform op — ast.rs:5800/5644),
- runtime-pluggable / FPGA-synthesizable (each runtime supplies its own
  `skolem` circuit; cf. the `tc` / `rmap` / `project` Platform ops),
- total (Bottom on shape mismatch; never panics).

## 3. The lazy resolution flow (LAZY ONLY — no eager forward chain)

The skolem head MUST use the **same `*`-View MaterializationPolicy path**
934-1 established, never an eager forward-chain rule. Eager
materialization of the `FactType ⋈ Role ⋈ Noun` join over the ~593-FT
metamodel HANGS the compile (proven by task-934; banner at the top of
`readings/ui/view-projection.md`).

Flow (all machinery already exists except the two starred steps):

1. The consequent FT is marked fully-derived (`*` suffix →
   `Fact Type has Derivation Mode 'fully-derived'`), so
   `compile.rs:3702-3712` sets the rule's `materialization = View`.
2. `compile.rs:1647-1654` SKIPS View rules from eager `derivation:{id}`
   emission. The forward chain never fires the rule — no metamodel hang.
3. `compile.rs:1664-1682` (`view_by_cell` fold) groups View rules by
   consequent cell and emits one `view:{cell}` def. ★ The skolem rule's
   func (built in step ★ below) is just another entry in this fold.
4. At read time, `Func::Fetch` / `Func::FetchOrPhi` on the consequent
   cell → `resolve_view` (ast.rs:4844) → `apply(view_func, encoded_pop)`
   → unwrap `[ft_id, reading, bindings]` envelopes to bindings.
5. ★ The view_func, for a skolem rule, emits one envelope per antecedent
   binding whose `bindings` list includes the synthesized
   `<HEAD_ENTITY_ROLE, skolem_id>` pair. Because the id is a pure
   function of the frontier, two `resolve_view` passes over the same
   population produce byte-identical bindings — idempotent.

★ = the two steps this task implements (the `skolem` primitive +
the compiler emission). Steps 1-4 are 934-1 / task-927 machinery reused
verbatim.

## 4. The implementation (functions / lines)

### 4.1 `Func::Platform("skolem")` — the value-invention leaf

`crates/arest/src/ast.rs`, `apply_platform` (~2781): add arm

```
"skolem" => platform_skolem(x),
```

and `fn platform_skolem(x: &Object) -> Object` near `synthesize_fact_id`
(ast.rs:5488). Contract: input `x` is a sequence of atom values (the
frontier); output is `Atom("ve_" ++ fnv1a64_hex)` computed with the
shared FNV-1a-64 constants. A non-seq / non-atom input → Bottom.
(No serde needed, so it is NOT gated behind `std-deps` like `rmap`; it
is available on every non-`no_std` target, and `no_std` already returns
Bottom for all `Func::Platform`.)

The id PREFIX (`ve_`) and the SEPARATOR are taken as a parameter so the
one primitive serves any future skolem head; minimal version hard-codes
`ve_` + `|` and documents the generalisation.

### 4.2 Compiler: emit the skolem binding in the 1-antecedent fanout

`crates/arest/src/types.rs`, `DerivationRuleDef`: add

```
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub skolem_head_roles: Vec<SkolemHeadRole>,
```

where `SkolemHeadRole { role: String, frontier: Vec<String> }` names the
fresh consequent role and the ordered antecedent role names whose values
seed the hash. Default empty → every existing rule is byte-identical
through the canonical-JSON writer (the `skip_serializing_if` keeps the
wire format unchanged; mirror the writer in `types.rs` ~781+).

`crates/arest/src/compile.rs`, `compile_explicit_derivation`, the
1-antecedent `bindings_func` (the `rule.consequent_role_literals.is_empty()`
branch, ~5059, AND the literal branch ~5153 since the menu rule pins
`Component Role 'button'`): after the normal pairs are built, for each
`SkolemHeadRole` APPEND a pair

```
<role_name, Platform("skolem"):[ role_value_by_name(frontier₀), … ]>
```

i.e. `Func::construction([ Constant(role_name),
Compose(Platform("skolem"), Construction([role_value_by_name(f) for f in frontier])) ])`.
The frontier `role_value_by_name` reads off the **antecedent fact under
`apply_to_all`** — exactly the same input the other consequent pairs
read. So the skolem id is per-binding, deterministic, and computed lazily
inside the view func.

Because the head entity role is now bound, the per-fact `required_keys`
guard (compile.rs ~5230) must NOT list the skolem role (it is not read
off the antecedent). Gate it out: `required_keys` already only collects
by-name-lookup roles; the skolem role is excluded by construction since
it is appended after, but add an explicit `skolem_head_roles` exclusion
when computing `required_keys` for safety.

No other compiler path changes: `derivation_dep_metadata` still reports
the consequent cell; `view_by_cell` still groups by it; `resolve_view`
still unwraps envelopes.

### 4.3 Parser (the remaining work — see §5)

The parser change to RECOGNISE a fresh head variable and populate
`skolem_head_roles` is deferred. The minimal increment constructs the
`DerivationRuleDef` directly in the spec test (the same way the engine's
synthetic rules — subtype inheritance, CWA negation — are built without
parser surface syntax), proving the compile + resolve_view + idempotence
mechanism end-to-end.

## 5. Remaining work — the FORML 2 surface syntax (parser)

`crates/arest/src/parse_forml2.rs`, `resolve_derivation_rule` (~1282),
consequent-resolution block (~1490-1533):

1. After `resolve_consequent_strict`, detect head role variables that
   (a) are roles of the resolved consequent FT and (b) do NOT appear as
   an antecedent role nor as the LHS of an `X is Y` rename clause. Such a
   role is a **skolem head role**. (The `(E)` / `(Tr)` parenthesised
   role-variable surface form in design §4.5 is the disambiguator: a
   parenthesised variable that is otherwise unbound is existential.)
2. Compute its frontier = the antecedent-bound role variables the head
   co-occurs with in the consequent FT clause group (for the menu rule:
   the View and Transition the same head ViewElement renders / belongs
   to). Minimal heuristic: frontier = ALL antecedent-bound roles that
   appear in the SAME consequent FT as the skolem role, in declaration
   order. Record `SkolemHeadRole { role, frontier }`.
3. The two-consequent-FT head (`E renders Tr AND E has Component Role
   'button'`) means a single rule writes TWO consequent cells with a
   SHARED skolem `E`. That needs the multi-consequent-head split
   (one `DerivationRuleDef` per consequent FT, both carrying the SAME
   `SkolemHeadRole` so the shared id matches). This is the larger parser
   sub-task; the minimal increment proves the single-consequent-FT case
   first.

A `re_resolve_rules` pass (parse_forml2, called from compile.rs:3687)
re-runs `resolve_derivation_rule`, so `skolem_head_roles` must be
cleared-then-rebuilt there exactly like `consequent_role_literals`
(compile.rs path, parse_forml2.rs:1500) to avoid duplicate accumulation.

## 6. Test (spec)

`crates/arest/src/compile_explicit_derivation_tests.rs`: an
`#[ignore]`d (until the parser lands) spec test
`skolem_head_derives_one_fresh_entity_per_binding_idempotent` that:

1. Builds a `DerivationRuleDef` for a menu-shaped skolem rule directly
   (View materialization, one antecedent FT `View_is_for_Noun`-style,
   `skolem_head_roles = [{role:"ViewElement", frontier:["View","Transition"]}]`).
2. Compiles it via `compile_explicit_derivation`, asserts the func is
   grouped under `view:{cell}` (no `derivation:` def).
3. Runs `resolve_view` over a 2-binding population → asserts exactly 2
   fresh `ViewElement` facts, each with a deterministic `id` =
   `ve_<fnv>` of its frontier.
4. Runs `resolve_view` a SECOND time → asserts the id set is
   byte-identical (idempotent; no new/duplicate ViewElement).
5. Asserts the metamodel-hang guard: the rule emits NO `derivation:` def
   (it never enters the eager forward chain).

A `#[test]` unit test `platform_skolem_is_deterministic_and_frontier_keyed`
covers the primitive directly (same input → same id; distinct input →
distinct id), GREEN immediately (no parser dependency).

## References

- `readings/ui/view-projection-design.md` §4.5 (menu = Theorem 4),
  §4.6 (collection rows), §5 (defaults vs overrides).
- `readings/ui/view-projection.md` — 934-1 SLICE-1 lazy-view mechanism +
  the eager-hangs-the-metamodel banner.
- `compile.rs:1647-1682` — Stored-vs-View def emission + `view_by_cell`
  fold (934-1).
- `ast.rs:4844 resolve_view`, `Func::Fetch`/`FetchOrPhi` (task-927/930).
- `ast.rs:5488 synthesize_fact_id`, `evaluate.rs:559 fnv_mix` — the
  shared FNV-1a-64 the Skolem id reuses.
- Cali et al., "Taming the Infinite Chase" — semi-oblivious / Skolem
  chase termination for existential rules (TGDs).
