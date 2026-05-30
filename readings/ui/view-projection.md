# AREST UI: View Projection (modeled derivation) — task-934

> ⚠️ **task-934 FINDING (2026-05-26): the eager `iff`-rule approach in this file DOES NOT WORK.**
> Registering these rules and compiling caused a *pathological forward-chain* — the LFP
> eagerly materializes the `FactType ⋈ Role ⋈ Noun ⋈ Format` join over all ~593 metamodel
> fact types on every compile, hanging the metamodel-compile (attempted + reverted, binary
> restored). **The projection must be LAZY** — defined as on-demand views via the task-927
> `resolve_view` / `FetchOrPhi` machinery (`ast.rs:2327,4717`, `compile.rs:439`), computed
> when `get`/`actions` runs, NOT as eager derivation cells. The §4.2 mapping and the verified
> metamodel fact-type references below remain correct; redo them as lazy views, not iff-rules.
>
> Promotes `readings/ui/view-projection-design.md` §4.2 (the value-type →
> widget rules) into an additive, compilable FORML 2 reading.
>
> **Status: SLICE 1 — written, NOT YET REGISTERED.** This file is the
> promoted foundation (the `View` / `ViewElement` fact types + the
> negation-free value-type → `Component Role` projection). It is
> deliberately *not* yet wired into `metamodel_readings()` (lib.rs): the
> engine has a single build target (`crates/arest/target`, the binary the
> MCP uses — there is no workspace-root Cargo.toml), so a reading with a
> checker error would break the live metamodel compile. The register +
> rebuild + iterate-to-checker-clean step is intentional and separate.
>
> **Next steps (in order):**
>   1. Register: add `("view-projection", include_str!(".../view-projection.md"))`
>      to the metamodel list in `crates/arest/src/lib.rs` (after `render`,
>      ~line 1120), rebuild, and run the suite — fix any checker errors in
>      the rules below (the multi-hop anaphora / literal-consequent idioms
>      are the likely friction; resolve them the way `core.md:266,:321`
>      and `monoview.md` do).
>   2. Add the `at most one` UC on `ViewElement has Component Role` once
>      the Format/Enum exclusivity below is confirmed mutually exclusive
>      in practice (a Noun has at most one Format, and enum value types do
>      not carry `Format 'text'`, so the four rules should not overlap).
>
> **Deferred to follow-up slices** (they need the negation / existential-
> head idioms `view-projection-design.md` §4.3-4.6 + §5 flag):
>   - §4.3 constraint → `ViewElement is required` (`some Frequency Constraint spans ...`)
>   - §4.4 caption fallback (`no Fact Type has some Title`)
>   - §4.5 menu = Theorem 4 (current-Status filter + Guard negation)
>   - auto-derivation of one `ViewElement` per rendered Fact Type (a
>     skolem / existential rule head) and the per-Noun override guards (§5)
>   - per-role granularity for n-ary fact types (this slice keys off the
>     single value-type role, correct for unary/binary facts)
>
> Metamodel anchors (`readings/core/core.md`): `Noun has Object Type`
> (:99), `Noun has Format` (:105), `Noun has Enum Values` (:107),
> `Fact Type has Role` (:164), `Role is played by Noun` (:321 usage).
> `Component Role` value type + widget vocabulary: `components.md:63`.

## Entity Types

View(.Name) is an entity type.
ViewElement(.id) is an entity type.

## Value Types

View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.

## Fact Types

View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.

ViewElement belongs to View.
  Each ViewElement belongs to exactly one View.
ViewElement renders Fact Type.
  Each ViewElement renders at most one Fact Type.
ViewElement has Component Role. *
ViewElement has Order.
  Each ViewElement has at most one Order.

## Derivation Rules — value type drives the widget (design §4.2)

# Each rule keys a ViewElement's Component Role off the value-type Noun's
# metamodel facts, reached by joining the rendered Fact Type's Role to the
# Noun that plays it. The analogue of MonoTouch.Dialog's member-type →
# Element switch (Reflect.cs L247-395), but over Noun + Fact Type +
# constraints rather than a CLR Type. The Formats (text/date/boolean) are
# mutually exclusive (a Noun has at most one Format); the enum branch keys
# off Enum Values instead.

ViewElement has Component Role 'text-input' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Object Type 'value' and that Noun has Format 'text'.

ViewElement has Component Role 'date-picker' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Format 'date'.

ViewElement has Component Role 'checkbox' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Format 'boolean'.

ViewElement has Component Role 'combo-box' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has some Enum Values.
