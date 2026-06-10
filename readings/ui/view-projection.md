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
ViewElement renders Fact Type. *
  Each ViewElement renders at most one Fact Type.
  <!-- `*` (fully-derived / View-materialized): the renders link is
       derived lazily by view-detail.md's skolem rule. This FIRST
       declaration must carry the star — view-detail.md re-declares the
       FT with `*`, but duplicate declarations dedupe to the first one,
       dropping the re-declaration's Derivation Mode marker; without the
       star here the rule compiles Stored, never gets its
       `view:ViewElement_renders_Fact_Type` def, and `resolve_view`
       returns None for every instance view (the pb-zero-glue-acceptance
       blocker, 2026-06-10). -->
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

## Conceptual-Data-Type / Format widget layer (Format-on-CDT, Phase 1)

> Additive over the §4.2 rules above. Those rules key a ViewElement's
> widget DIRECTLY off the legacy per-Noun Format literal. This layer
> makes the widget a *modeled property of the data type itself*: a base
> widget per Conceptual Data Type (`readings/core/core.md`), refined by a
> per-Format widget when a value type declares a Format. It is the
> presentation analogue of the JSON-Schema / Abstract-SQL projections the
> CDT catalog already carries — the widget falls out of the type, not out
> of a Noun-by-Noun rule.
>
> `Component Role` is the closed widget vocabulary declared in
> `readings/ui/components.md` (a UI-feature noun); this layer lives here
> in the `ui-readings` scope rather than in `core.md` because the readings
> checker rejects an undeclared role-noun in the core slice.
>
> Metamodel anchors (`readings/core/core.md`): `Conceptual Data Type`
> (:631 region), `Format` (:633 region), `Noun has Conceptual Data Type`
> (the value-type -> CDT link), `Noun has Format` (the value-type ->
> Format refinement link), `Format is built on Conceptual Data Type`.

### Fact Types

Conceptual Data Type implies Component Role.
  Each Conceptual Data Type implies at most one Component Role.
  <!-- BASE widget. Keyed per CDT leaf (a Data-Type-Group-level default
       can be layered later by deriving this from `Conceptual Data Type
       is in Data Type Group`). Mirrors the form of `Interaction Mode
       implies minimum Hit Target Size` in monoview.md. -->

Format implies Component Role.
  Each Format implies at most one Component Role.
  <!-- REFINEMENT widget. A Format overrides the base CDT widget for the
       value types that declare it. -->

Noun has effective Component Role. *
  <!-- The RESOLVED widget for a value-type Noun: its Format's implied
       Component Role when it declares a Format (refinement), ELSE its
       base Conceptual Data Type's implied Component Role (base). Lazy
       (`*`, View materialization) so the resolution never eager-folds
       over the whole metamodel — the same discipline as the §4.2 rules. -->

### Derivation Rules — effective widget resolves Format-else-CDT

# Modeled on the effective-Pane-Mode override/fallback idiom
# (`readings/ui/monoview.md`: a PanePreference overrides a MonoView's
# default Pane Mode). The REFINEMENT rule fires for value types that
# declare a Format; the BASE rule supplies the Conceptual-Data-Type
# default. FORML 2 negation was removed as an antecedent kind
# (readings/core/derivation.md, 2026-05-19), so the "else" is NOT a
# negation guard: where a value type carries both a Format and a base
# CDT, both rules populate `Noun has effective Component Role`, and the
# Format-sourced row is the refinement the projection/renderer prefers
# (the same specific-wins-at-resolve-time discipline the post-negation
# codebase uses, and the same place a PanePreference override is picked
# over a MonoView default). A first-class suppression operator that makes
# the base row drop automatically when a refinement exists is a Phase-2
# need (see below).

* Noun has effective Component Role (CR)
    if Noun has some Format
    and that Format implies Component Role (CR).

* Noun has effective Component Role (CR)
    if Noun has some Conceptual Data Type
    and that Conceptual Data Type implies Component Role (CR).

## Instance Facts — seed the four legacy widget Formats (Phase 1)

> Reproduces the current widget vocabulary EXACTLY by seeding the four
> legacy Formats as `Format` instances built on the matching existing CDT
> leaf, each carrying its `Component Role`. `text`/`boolean`/`enum` impose
> no JSON-Schema `format` keyword, so they carry no `Format has JSON
> Format` (the binary is `at most one`); their effective JSON Format falls
> back to the base CDT's (also none). `date` refines to JSON Format
> 'date', matching its base CDT 'date' (`Conceptual Data Type 'date' has
> JSON Format 'date'` in core.md) so the projection is identical whether
> resolved via the Format or the base CDT.
>
> Widget reproduction (current behavior, unchanged):
>   text    -> text-input
>   date    -> date-picker
>   boolean -> checkbox
>   enum    -> combo-box

Format 'text'    is built on Conceptual Data Type 'text'.
Format 'date'    is built on Conceptual Data Type 'date'.
Format 'boolean' is built on Conceptual Data Type 'boolean'.
Format 'enum'    is built on Conceptual Data Type 'text'.

Format 'date' has JSON Format 'date'.

Format 'text'    implies Component Role 'text-input'.
Format 'date'    implies Component Role 'date-picker'.
Format 'boolean' implies Component Role 'checkbox'.
Format 'enum'    implies Component Role 'combo-box'.
