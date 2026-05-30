# AREST UI: Instance-Detail (Form) View Derivation — task-934-2

> **Status: task-934-2 (instance-detail slice) — LIVE. Six shared-frontier
> skolem rules (four widget types: text-input, date-picker, checkbox,
> combo-box) compile through the join-skolem path, registered in `lib.rs`
> UI_READINGS after `view-list`. `Fact Type has Format` is an EAGER
> derived-and-stored (`**`) projection from the FT's value-type role's
> Noun's Format — the lazy widget rules see it materialized. Full ~593-FT
> metamodel compiles GREEN. Proven by
> `instance_detail_view_derivation_compiled_from_authored_reading` and
> `instance_detail_view_real_data_format_projection` in
> `compile_explicit_derivation_tests.rs`.**

## Overview

An instance/detail View of a Noun projects its Fact Types into a form
structure — one `ViewElement` per Fact Type the Noun participates in, with the
widget chosen by the Fact Type's value-type. The derivation is lazy (resolved
at fetch time via `resolve_view`) and uses a SKOLEM head variable so the
`ViewElement` identity is deterministic and idempotent across re-reads.

This is design-doc §3.2 (instance detail + form view) instantiated as a
predicate reading. The join is a 4-antecedent chain over View→Noun, Fact
Type→Role→Noun, yielding one ViewElement per (View, Fact Type) binding.

## The Derivation (Predicate Reading Form)

The rule shape (using the `(E)` parenthesised existential syntax from
`skolem-head-design.md` §5):

```
* ViewElement (E) renders Fact Type (FT) iff
    View is for Noun
    and View has View Kind 'instance'
    and Fact Type (FT) has Role
    and Role is played by Noun.
```

and the companion widget rules (same frontier → same `E`):

```
* ViewElement (E) has Component Role 'text-input' iff
    View is for Noun
    and View has View Kind 'instance'
    and Fact Type (FT) has Role
    and Role is played by Noun
    and Fact Type (FT) has Format 'text'.

* ViewElement (E) has Component Role 'date-picker' iff
    View is for Noun
    and View has View Kind 'instance'
    and Fact Type (FT) has Role
    and Role is played by Noun
    and Fact Type (FT) has Format 'date'.

* ViewElement (E) has Component Role 'checkbox' iff
    View is for Noun
    and View has View Kind 'instance'
    and Fact Type (FT) has Role
    and Role is played by Noun
    and Fact Type (FT) has Format 'boolean'.

* ViewElement (E) has Component Role 'combo-box' iff
    View is for Noun
    and View has View Kind 'instance'
    and Fact Type (FT) has Role
    and Role is played by Noun
    and Fact Type (FT) has some Enum Values.
```

All six rules carry `*` (lazy, `View` materialization policy — never enters
the eager forward chain). The `View has View Kind 'instance'` antecedent is a
literal-pinned filter. The six sibling rules share the identical entity-typed
frontier `(View, Noun, Fact Type, Role)` so the invented `ve_<fnv>` id is
shared across `renders Fact Type` and each `Component Role` head.

The combo-box rule uses the eagerly-projected `Fact Type has Enum Values` cell
(parallel to `Fact Type has Format`), keeping the same 5-antecedent structure
as the other widget rules.

## Fact Types

`ViewElement has Component Role` is already declared `*` (fully-derived) in
`view-projection.md`. `ViewElement renders Fact Type` is declared there too
(at most one, View-materialized via these rules). The `*` suffix marks them
View-materialized so the forward chain never eager-evaluates the join over the
~593-FT metamodel.

`Fact Type has Format` and `Fact Type has Enum Values` are declared `**`
(derived-and-stored, EAGER) here. Eager because: (a) each is a small per-FT
projection (~one row per value-typed FT — no combinatorial blowup), and
(b) the lazy widget rules above must see them materialized in the population
when `resolve_view` evaluates them — lazy `*` projections would require
lazy-on-lazy chaining which `resolve_view` does NOT do.

Fact Type has Format. **
  Each Fact Type has at most one Format.
Fact Type has Enum Values. **
  Each Fact Type has at most one Enum Values.

## Derivation Rules

### Eager projections (Stored / `**`)

Projects the value-type role's Noun Format and Enum Values onto the Fact Type.
Fire in the forward chain on every apply, before the lazy view rules evaluate.
One output row per value-typed Fact Type (linear, no combinatorial blowup).

** Fact Type has Format iff Fact Type has some Role and that Role is played by some Noun and that Noun has Object Type 'value' and that Noun has Format.
** Fact Type has Enum Values iff Fact Type has some Role and that Role is played by some Noun and that Noun has Object Type 'value' and that Noun has some Enum Values.

### Lazy view rules (View / `*`)

Six shared-frontier skolem rules (single-line registration form of the
prose above). The `(E)` head variable is fresh (existential); the parser
records a `SkolemHeadRole` and promotes the antecedent chain to a Join whose
skolem frontier is the entity-typed antecedent nouns — `View`, `Noun`,
`Fact Type`, `Role` — identical across all six rules so the invented
`ve_<fnv>` id is shared. `Fact Type (FT)` carries the rendered fact type.

* ViewElement (E) renders Fact Type (FT) iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun.
* ViewElement (E) has Component Role 'text-input' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'text'.
* ViewElement (E) has Component Role 'date-picker' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'date'.
* ViewElement (E) has Component Role 'checkbox' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'boolean'.
* ViewElement (E) has Component Role 'combo-box' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has some Enum Values.

## Metamodel Fact-Type Names (Verified)

The following cell names have been verified against `readings/ui/view-projection.md`
and `readings/core/core.md`:

| FORML 2 reading text                 | Cell name                        |
|--------------------------------------|----------------------------------|
| View is for Noun                     | `View_is_for_Noun`               |
| View has View Kind 'instance'        | `View_has_View_Kind`             |
| Fact Type has Role                   | `Fact_Type_has_Role`             |
| Role is played by Noun               | `Noun_plays_Role` (inverse read) |
| Noun has Object Type                 | `Noun_has_Object_Type`           |
| Noun has Format                      | `Noun_has_Format`                |
| Noun has Enum Values                 | `Noun_has_Enum_Values`           |
| Fact Type has Format                 | `Fact_Type_has_Format` (eager)   |
| Fact Type has Enum Values            | `Fact_Type_has_Enum_Values` (eager) |
| ViewElement renders Fact Type        | `ViewElement_renders_Fact_Type`  |
| ViewElement has Component Role       | `ViewElement_has_Component_Role` |

`View_is_for_Noun` and `View_has_View_Kind` are declared in `view-projection.md`.
`Fact_Type_has_Role`, `Noun_plays_Role`, `Noun_has_Object_Type`, `Noun_has_Format`,
and `Noun_has_Enum_Values` are declared in `readings/core/core.md`.
`Fact_Type_has_Format` is declared here with `**` (eager/derived-and-stored).

**Projection paths**: the eager rules materialize `Fact_Type_has_Format` and
`Fact_Type_has_Enum_Values` by following `FT → Role → Noun(value-type) →
Format` and `FT → Role → Noun(value-type) → Enum Values` respectively. In
production, `Noun_has_Object_Type`, `Noun_has_Format`, and
`Noun_has_Enum_Values` are reconstituted from the absorbed Noun-cell fields
via `FetchOrPhi`. In test mini-schemas they are pushed directly. Either way,
the eager derivations materialize both projection cells before the lazy widget
rules evaluate.

## Join Chain

### Eager projections (pre-populate `Fact_Type_has_Format` + `Fact_Type_has_Enum_Values`):
```
Fact Type has some Role              (FT → Role_R)
  ⋈ that Role is played by some Noun (Role_R → Noun_V, value-type)
  ⋈ Noun_V has Object Type 'value'  (filtered)
  ⋈ Noun_V has Format                (Noun_V → Format_F)
→ emits: (Fact Type=FT, Format=Format_F)

Fact Type has some Role              (FT → Role_R)
  ⋈ that Role is played by some Noun (Role_R → Noun_V, value-type)
  ⋈ Noun_V has Object Type 'value'  (filtered)
  ⋈ Noun_V has some Enum Values      (Noun_V → Enum_Values_E)
→ emits: (Fact Type=FT, Enum Values=Enum_Values_E)
```

### Base (renders) + widget rules:
```
View is for Noun                          (View → Noun_N, entity-type)
  ⋈ View has View Kind 'instance'        (View → View_Kind, filtered to 'instance')
  ⋈ Fact Type (FT) has Role              (FT → Role_R)
  ⋈ Role is played by Noun               (Role_R → Noun_N)     [join on Noun_N + Role_R]
```

For text/date/boolean widget rules, additionally:
```
  ⋈ Fact Type (FT) has Format 'text'     (FT → Format, filtered to 'text')
```
(Format and Enum Values are materialized eagerly; FetchOrPhi finds them directly.)

For combo-box, additionally:
```
  ⋈ Fact Type (FT) has some Enum Values  (FT → Enum Values, materialized eagerly)
```

Join keys: `View` (FTs 1+2), `Noun` (FTs 1+4), `Role` (FTs 3+4), `Fact Type` (FTs 3+5)
Frontier (entity-typed antecedent nouns, in order): `View`, `Noun`, `Fact Type`, `Role`
Frontier hash seed: `fnv1a64(View + "|" + Noun + "|" + Fact Type + "|" + Role)` → `ve_<16 hex>`.

Because each View is for exactly one Noun (UC from view-projection.md), and
each Role is played by exactly one Noun (UC from core.md), the
`(View, Noun, Role)` combination collapses to `(View, Role)` as the
discriminating prefix — one ViewElement per (View, Fact Type) pair.

## Skolem Head Properties

- **Deterministic**: `ve_<fnv>` is a pure function of `(View, Noun, Fact Type, Role)`.
  Re-reading the same population reproduces the same ids.
- **Idempotent**: same frontier → same id → no duplicate `ViewElement` across
  re-read passes (semi-oblivious / Skolem chase correctness).
- **Lazy**: all rules emit `view:{cell}` defs, never `derivation:{cell}` defs.
  Resolved via `resolve_view` at `Func::Fetch` / `Func::FetchOrPhi` time.
- **Filter-correct**: only Views with View Kind = 'instance' produce
  ViewElements — the literal pin is applied as an antecedent predicate.
- **Shared-frontier**: all six sibling rules share frontier `[View, Noun, Fact
  Type, Role]` so `renders Fact Type` and `Component Role` heads produce the
  same `ve_<fnv>` id per (View, FT) binding.

## Remaining Work

### (1) Row caption (reference-scheme value)
Design §3.2: screen title = instance's reference-scheme value.
Requires `Noun has Reference Scheme` traversal — deferred.

### (2) Part (b): iFactr instance values at render time
The form STRUCTURE (which fields exist, which widgets) is established here.
Filling in actual instance VALUES at render time is deferred (part b).

### (3) Guard-negation filtering / suppression
Design: suppressing fields at fetch layer is not a derivation concern
(negation removed from FORML2 per parse_forml2.rs). Deferred.

## Test Coverage

`crates/arest/src/compile_explicit_derivation_tests.rs`:
- `instance_detail_view_derivation_compiled_from_authored_reading` — GREEN:
  proves (a) one VE per FT the Noun participates in, (b) 0 VEs for
  collection-kind View (literal filter excludes it), (c) deterministic
  `ve_<fnv>` ids, (d) idempotent across 2 passes, (e) Fact Type carried
  through, (f) shared frontier → same VE id across renders + Component Role
  rules, (g) no eager `derivation:` def for View rules, (h) correct widget
  per Format ('text' → 'text-input', 'date' → 'date-picker', 'boolean' →
  'checkbox').
- `instance_detail_view_real_data_format_projection` — GREEN:
  proves that `Fact_Type_has_Format` is derived EAGERLY (no manual population),
  and that all four widget rules fire from REAL derived data:
  text → 'text-input', date → 'date-picker', boolean → 'checkbox',
  enum → 'combo-box'. The Format projection rule carries `**` (Stored/eager)
  materialization and materializes before `resolve_view` runs.
