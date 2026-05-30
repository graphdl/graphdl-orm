# AREST UI: Collection-List View Derivation — task-934-2

> **Status: task-934-2 — LIVE. These two shared-frontier skolem rules compile
> (real parser+compiler) through the join-skolem path (≥2 antecedents, shared
> entity-typed frontier), registered in `lib.rs` UI_READINGS after
> `view-menu`. Full ~593-FT metamodel compiles GREEN (no hang, `*`-View lazy
> only). Proven by
> `collection_list_view_derivation_compiled_from_authored_reading` in
> `compile_explicit_derivation_tests.rs`.**

## Overview

A collection View of a Noun lists its instances. Each row is a `ViewElement`
that renders one `Resource` instance of that Noun. The derivation is lazy
(resolved at fetch time via `resolve_view`) and uses a SKOLEM head variable so
the `ViewElement` identity is deterministic and idempotent across re-reads.

This is design-doc §3.1/§4.6 (collection rows) instantiated as a predicate
reading. The join is simpler than the menu (3-antecedent chain vs 5-antecedent
chain), but uses the same join-skolem mechanism proven in task-934-3.

## The Derivation (Predicate Reading Form)

The rule shape (using the `(E)` parenthesised existential syntax from
`skolem-head-design.md` §5):

```
* ViewElement (E) renders Resource (R) iff
    View is for Noun
    and View has View Kind 'collection'
    and Resource (R) is instance of Noun.
```

and the companion rule (same frontier → same `E`):

```
* ViewElement (E) has Component Role 'list' iff
    View is for Noun
    and View has View Kind 'collection'
    and Resource (R) is instance of Noun.
```

Both rules carry `*` (lazy, `View` materialization policy — never enters the
eager forward chain). The `View has View Kind 'collection'` antecedent is a
literal-pinned filter: the parser records it as `AntecedentRoleLiteral`
(role = "View Kind", value = "collection") and the join compiler applies it as
a per-antecedent predicate filter over the `View_has_View_Kind` cell.

## Fact Types

`ViewElement has Component Role` is already declared `*` (fully-derived) in
`view-projection.md`. The collection-specific `renders Resource` link is
declared here. The `*` suffix marks it View-materialized so the forward chain
never eager-evaluates the join over the ~593-FT metamodel.

ViewElement renders Resource. *

## Derivation Rules

The two shared-frontier skolem rules (single-line registration form of the
prose above). The `(E)` head variable is fresh (existential); the parser
records a `SkolemHeadRole` and promotes the 3-way `and`-chain to a Join whose
skolem frontier is the entity-typed antecedent nouns — `View`, `Noun`,
`Resource` (in order of first antecedent-FT-role occurrence). `View is for
exactly one Noun` (UC in view-projection.md), so the (View, Noun, Resource)
frontier effectively gives one ViewElement per (View, Resource) binding.

Component Role 'list' is chosen because §3.1 of the design doc maps each
collection row instance to the `list` Component (the `list` value is already
in the `components.md` enum — no new value needed). The `list` role labels
the row cell in the list view surface, matching iFactr's `IContentCell` shape.

* ViewElement (E) renders Resource (R) iff View is for Noun and View has View Kind 'collection' and Resource (R) is instance of Noun.
* ViewElement (E) has Component Role 'list' iff View is for Noun and View has View Kind 'collection' and Resource (R) is instance of Noun.

## Metamodel Fact-Type Names (Verified)

The following cell names have been verified against `readings/ui/view-projection.md`
and `readings/core/instances.md`:

| FORML 2 reading text               | Cell name                      |
|------------------------------------|-------------------------------|
| View is for Noun                   | `View_is_for_Noun`            |
| View has View Kind 'collection'    | `View_has_View_Kind`          |
| Resource is instance of Noun       | `Resource_is_instance_of_Noun`|

`View_is_for_Noun` and `View_has_View_Kind` are declared in `view-projection.md`.
`Resource_is_instance_of_Noun` is declared in `readings/core/instances.md`.
`View Kind` is a value type (not entity-typed) — it is excluded from the
entity-typed frontier (only `View`, `Noun`, `Resource` enter the hash seed).

## Join Chain

```
View is for Noun                          (View → Noun_N)
  ⋈ View has View Kind 'collection'      (View → View_Kind, filtered to 'collection')
  ⋈ Resource is instance of Noun         (Resource → Noun_N)     [join on Noun_N]
```

Join keys (shared by ≥2 antecedents): `View` (appears in FTs 1+2), `Noun`
(appears in FTs 1+3).
Frontier (entity-typed antecedent nouns, in order): `View`, `Noun`, `Resource`.
Frontier hash seed: `fnv1a64(View + "|" + Noun + "|" + Resource)` → `ve_<16 hex>`.

Because each View is for exactly one Noun (UC from view-projection.md), the
`(View, Noun)` pair collapses to `(View)` as the discriminating prefix, so
the effective granularity is one ViewElement per (View, Resource) pair —
exactly the design-doc §3.1 intent.

## Skolem Head Properties

- **Deterministic**: `ve_<fnv>` is a pure function of `(View, Noun, Resource)`.
  Re-reading the same population reproduces the same ids.
- **Idempotent**: same frontier → same id → no duplicate `ViewElement` across
  re-read passes (semi-oblivious / Skolem chase correctness).
- **Lazy**: both rules emit `view:{cell}` defs, never `derivation:{cell}` defs.
  Resolved via `resolve_view` at `Func::Fetch` / `Func::FetchOrPhi` time.
- **Filter-correct**: only Views with View Kind = 'collection' produce
  ViewElements — the literal pin is applied as an antecedent predicate in
  `compile_join_derivation` (the `antecedent_role_literals` path, #814b).

## Remaining Work

### (1) Instance-detail view (§3.2)
The per-Noun instance detail/form view (one ViewElement per Fact Type of the
Noun, keyed by value type → Component Role) — deferred to task-934-2b. Requires
a 4-antecedent join over `View_is_for_Noun` × `View_has_View_Kind 'instance'`
× `Fact_Type_has_Role` × `Role_is_played_by_Noun`, plus the §4.2 value-type →
Component Role mapping chain.

### (2) Row caption (reference-scheme value)
Design §3.1: `IContentCell.TextLabel` = the instance's reference-scheme value
(the absorbing ref-scheme FT value). Requires joining on `Resource_has_Reference`
or `Noun_has_Reference_Scheme` and reading the identity value — deferred.

### (3) Guard-negation filtering
Design §4.6: suppressing draft/archived items requires negative antecedents
(parser negation, not yet available as a user-authoring surface in FORML 2).

## Test Coverage

`crates/arest/src/compile_explicit_derivation_tests.rs`:
- `collection_list_view_derivation_compiled_from_authored_reading` — GREEN:
  proves (a) 3 VEs for a Noun with 3 instances + a collection View,
  (b) 0 VEs for a Noun with 0 instances, (c) deterministic `ve_<fnv>` ids,
  (d) idempotent across 2 passes, (e) Resource carried through,
  (f) shared frontier → same VE id in both rules (renders + Component Role),
  (g) no eager `derivation:` def, (h) literal filter — 'instance'-kind View
  produces zero VEs.
