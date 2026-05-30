# AREST UI: Menu-View Derivation — task-934-3

> **Status: task-934-3 part (a) — LIVE. The `(E)` skolem-head surface syntax
> is wired through the parser + join compiler, and these two rules are
> REGISTERED in `lib.rs` UI_READINGS; the full ~593-FT metamodel compiles
> GREEN (no hang, `*`-View lazy only). Proven by
> `menu_view_derivation_compiled_from_authored_reading_reproduces_proven_func`
> (the COMPILED authored reading reproduces the hand-built target func) and
> `menu_view_derivation_via_skolem_head_lazy_idempotent` (the hand-built
> target) in `compile_explicit_derivation_tests.rs`.**

## Overview

A Noun's action menu is a DERIVED view. Each menu item is a `ViewElement`
that renders a `Transition` — specifically, every transition that is legal from
the entity's CURRENT status. The derivation is lazy (resolved at fetch time
via `resolve_view`) and uses a SKOLEM head variable so the `ViewElement`
identity is deterministic and idempotent across re-reads.

This is design-doc §4.5 (Theorem 4 as a view) instantiated as a predicate reading.

## The Derivation (Predicate Reading Form)

The rule shape (using the `(E)` parenthesised existential syntax from
`skolem-head-design.md` §5 — the parser for this is not yet landed):

```
* ViewElement (E) renders Transition (Tr) iff
    Resource is currently in Status (S)
    and Transition (Tr) is from Status (S)
    and Transition (Tr) is defined in State Machine Definition (D)
    and State Machine Definition (D) is for Noun (N)
    and Resource is instance of Noun (N).
```

and the companion rule (same frontier → same `E`):

```
* ViewElement (E) has Component Role 'button' iff
    Resource is currently in Status (S)
    and Transition (Tr) is from Status (S)
    and Transition (Tr) is defined in State Machine Definition (D)
    and State Machine Definition (D) is for Noun (N)
    and Resource is instance of Noun (N).
```

Both rules carry `*` (lazy, `View` materialization policy — never enters the
eager forward chain that caused the task-934 metamodel hang).

## Fact Types

`ViewElement has Component Role` is already declared `*` (fully-derived) in
`view-projection.md`; only the menu-specific `renders Transition` link is
declared here. The `*` suffix marks it View-materialized so the
forward chain never eager-evaluates the 5-way join over the ~593-FT metamodel.

ViewElement renders Transition. *

## Derivation Rules

The two shared-frontier skolem rules (single-line registration form of the
prose above). The `(E)` head variable is fresh (existential); the parser
records a `SkolemHeadRole` and promotes the 5-way `and`-chain to a Join whose
skolem frontier is the entity-typed antecedent nouns `(Resource, Transition,
State Machine Definition, Noun)` — identical across both rules, so the invented
`ve_<fnv>` id is shared. `Transition (Tr)` carries the rendered transition.

* ViewElement (E) renders Transition (Tr) iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.
* ViewElement (E) has Component Role 'button' iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.

## Metamodel Fact-Type Names (Verified)

The following cell names have been verified against `readings/core/state.md`,
`readings/core/instances.md`, and `readings/core/core.md`:

| FORML 2 reading text                         | Cell name                                          |
|----------------------------------------------|----------------------------------------------------|
| State Machine Definition is for Noun         | `State_Machine_Definition_is_for_Noun`             |
| Transition is defined in State Machine Def.  | `Transition_is_defined_in_State_Machine_Definition`|
| Transition is from Status                    | `Transition_is_from_Status`                        |
| State Machine is currently in Status         | `State_Machine_is_currently_in_Status`             |
| State Machine is for Resource                | `State_Machine_is_for_Resource`                    |
| Resource is instance of Noun                 | `Resource_is_instance_of_Noun`                     |
| Resource is currently in Status              | `Resource_is_currently_in_Status`                  |

`Resource is currently in Status` is the bridge projection declared in
`readings/core/instances.md` and populated per-app by the SM-for-Resource ×
SM-currently-in-Status join (e.g. `apps/tasks/readings/app.md`). The
menu-view derivation should join on the general-level cells above (the 6-way
join) to work across ALL Nouns+SMs, not just the tasks domain.

## Join Chain

```
Resource is currently in Status            (Resource → Status_S)
  ⋈ Transition is from Status             (Transition → Status_S)  [join on Status_S]
  ⋈ Transition is defined in SMD          (Transition → SMD_D)
  ⋈ State Machine Definition is for Noun  (SMD_D → Noun_N)
  ⋈ Resource is instance of Noun          (Resource → Noun_N)      [join on Noun_N]
```

The frontier after this join is `(Resource, Transition)`.
Frontier hash seed: `fnv1a64(Resource + "|" + Transition)` → `ve_<16 hex>` id.

## Skolem Head Properties

- **Deterministic**: `ve_<fnv>` is a pure function of `(Resource, Transition)`.
  Re-reading the same population reproduces the same ids.
- **Idempotent**: same frontier → same id → no duplicate `ViewElement` across
  re-read passes (semi-oblivious / Skolem chase correctness).
- **Lazy**: both rules emit `view:{cell}` defs, never `derivation:{cell}` defs.
  Resolved via `resolve_view` at `Func::Fetch` / `Func::FetchOrPhi` time.
- **Terminal-safe**: an entity in a terminal status (no departing transitions)
  produces zero frontier rows → zero ViewElements. Proven in the test.

## Remaining Work

### (1) Parser surface syntax (skolem-head-design.md §5) — DONE
The `(E)` parenthesised existential variable is supported:
`resolve_derivation_rule` records a `SkolemHeadRole` and promotes a
multi-antecedent `and`-chain to a Join (`compile_join_derivation` emits the
`Compose(Platform("skolem"), Construction[frontier extractors])`). For a JOIN
skolem head the frontier is the entity-typed antecedent nouns (here
`Resource, Transition, State Machine Definition, Noun`), which is identical
across the two sibling rules so the invented `ve_<fnv>` matches — the
"shared frontier → shared entity" invariant. `spec_skolem_head_authored_in_forml2_resolves_lazily`
(2-antecedent) and `menu_view_derivation_compiled_from_authored_reading_reproduces_proven_func`
(5-way) both pass through the real parser+compiler.

### (2) Guard-filtering negation
Design §4.5: `Guard prevents Transition → omit the ViewElement`. This requires
the parser-negation idiom (`no Guard prevents Tr` or AbsenceOf in the antecedent)
which is not yet available as a user-authoring surface in FORML 2.
The basic menu (all legal transitions, no guard filter) is what is proven here.

### (3) Registration in UI_READINGS (lib.rs) — DONE
`("view-menu", include_str!("../../../readings/ui/view-menu.md"))` is
registered after `view-projection`. The full metamodel compiles green (no
hang, no checker errors) — `handle_isolation_tests::create_impl_loads_metamodel`
exercises the full `compile_to_defs_state` over the registered reading. The
`*` View policy on `ViewElement renders Transition` keeps both rules out of
the eager forward chain (`view:` defs only, no `derivation:` def).

### (4) Collection-list and detail views (934-2)
`readings/ui/view-projection-design.md` §4.6 (collection rows) and §3.2
(instance detail) are deferred to the 934-2 slice.

## Test Coverage

`crates/arest/src/compile_explicit_derivation_tests.rs`:
- `menu_view_derivation_via_skolem_head_lazy_idempotent` — GREEN, test-only:
  proves (a) 2 VEs for pending entity, (b) 0 VEs for terminal entity,
  (c) deterministic `ve_<fnv>` ids, (d) idempotent across 2 passes,
  (e) Transition carried through, (f) shared frontier → same VE id in both
  rules, (g) no eager `derivation:` def.
- `menu_view_derivation_metamodel_ft_name_audit` — GREEN, audit-only:
  documents the verified metamodel FT names.

Both `skolem_head_resolve_view_invents_one_idempotent_entity_per_binding` and
`platform_skolem_is_deterministic_and_frontier_keyed` (from ast.rs) remain green
and cover the underlying mechanism. The menu test adds the menu-specific
semantic proof on top.
