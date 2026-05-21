# AREST UI: View-Projection Contract (design)

> **Status: DESIGN.** This reading is a design deliverable for tasks-app
> #934. It specifies *how a typed FORML 2 entity becomes a view* — the
> data→UI **view-projection contract**, AREST's analogue of the
> MonoTouch.Dialog (MT.D) reflection-driven UI. It contains the mapping
> table, derivation-rule **shapes** (not yet a checker-clean reading),
> the default-convention/override model, the monoview.md ⇄ contract.ts
> reconciliation note, and the convention choices that need the user's
> (iFactr expert) input. A follow-up slice turns the rule shapes in
> §4 / `view-projection.draft.md` into an additive FORML 2 reading.
>
> **Do not** treat the rule blocks here as compilable yet — they cite
> real metamodel fact types (`readings/core/core.md`,
> `readings/core/state.md`) and real UI fact types
> (`readings/ui/components.md`, `readings/ui/monoview.md`) so the
> follow-up can lift them with minimal edits, but the negation /
> cardinality idioms still need the same care the monoview.md and
> wine.md authors document.

## 0. Thesis: a view is a projection, not a hand-written form

AREST's whitepaper Theorem 4 — **HATEOAS as Projection** — says the
`_links` block on an entity response is not authored; it is *projected*
from `P` (the fact population): the links are exactly the transitions
whose `from` status matches the entity's current status
(`readings/core/state.md`, `docs/04-state-machines.md`). The agent sees
its valid next moves because they fall out of the state-machine fold,
not because someone wrote a menu.

This reading **extends that projection from actions to whole views.**
If the *action menu* of an entity is a projection of its state machine,
then by the same argument:

- the **list/collection view** of a Noun is a projection of its
  reference scheme + the small set of facts that identify/summarise an
  instance;
- the **detail/form view** of an instance is a projection of the Noun's
  Fact Types (by value type), constraints (mandatory / uniqueness /
  value-enum), and reference scheme;
- the **menu/actions view** is *literally* Theorem 4 — the SM
  transitions available from the current Status.

So the whole UI is `ρ`-applied over `P`: **`view(entity) = project(P, entity)`**.
Nothing about the view is hard-coded against a Noun's identity — the
same way `readings/ui/monoview.md` made pane mode a reading instead of a
property baked into each `.slint` file. MT.D gets there by reflecting a
C# object's fields + attributes; AREST gets there by *deriving* over a
strictly richer model (Noun + Fact Types + constraints + reference
scheme + state machine), so the projection can be smarter than
reflection.

## 1. The convention reference: MonoTouch.Dialog

MT.D (`MonoTouch.Dialog/Reflect.cs`, Miguel de Icaza) is the canonical
"reflection-driven UI from a data type." `BindingContext.Populate`
walks an object's members and maps **member type + attributes** →
**Element**, grouped into **Section**s, under a **RootElement**. The
conventions we steal:

### 1.1 Member **type** → Element (the default, attribute-free mapping)

| C# member type | MT.D Element | Notes (`Reflect.cs` line) |
| --- | --- | --- |
| `string` | `StringElement` (read-only label) | default for string (L292) |
| `string` + `[Entry]` | `EntryElement` (text field) | placeholder/keyboard from attr (L286) |
| `string` + `[Password]` | `EntryElement(secret)` | (L284) |
| `string` + `[Multiline]` | `MultilineElement` | (L288) |
| `string` + `[Html]` | `HtmlElement` (tappable→browser) | (L290) |
| `bool` | `BooleanElement` (switch) | (L328) |
| `bool` + `[Checkbox]` | `CheckboxElement` | (L325) |
| `float` | `FloatElement` (slider) | `[Range(lo,hi)]` sets bounds (L305) |
| `DateTime` | `DateTimeElement` | `[Date]`→`DateElement`, `[Time]`→`TimeElement` (L329) |
| `enum` | `RootElement` + `Section` of `RadioElement` | one radio per enum field; selected = current (L346) |
| `IEnumerable` (after an `int` `[RadioSelection]`) | `RootElement` + radio `Section` | runtime-populated radio group (L366) |
| `UIImage` | `ImageElement` | (L364) |
| nested object | nested `RootElement` (drill-in) | recursion via `Populate` (L388) |

### 1.2 **Attributes** that shape sections / captions / behavior

| Attribute | Effect |
| --- | --- |
| `[Section("cap","footer")]` | starts a new `Section`; caption+footer chrome |
| `[Caption("…")]` | overrides the element label (else `MakeCaption` splits CamelCase / `_`) |
| `[Skip]` (and compiler-generated) | member omitted from the view |
| `[Entry]` / `[Password]` / `[Multiline]` / `[Html]` | string sub-kind (above) |
| `[Checkbox]` / `[Date]` / `[Time]` / `[Range]` | element sub-kind for bool/DateTime/float |
| `[RadioSelection("target")]` | marks the `int` index member that drives the next `IEnumerable` radio group |
| `[OnTap("Method")]` | makes a `StringElement` tappable → invokes a callback (this is MT.D's *action* affordance) |
| `[Alignment]` | text alignment on a `StringElement` |

### 1.3 The three structural facts to carry over

1. **Caption convention.** Absent `[Caption]`, the label is derived
   from the member name (`MakeCaption`: CamelCase / `_` → spaced
   Title Case). AREST's analogue is the Fact Type **reading** /
   Noun **Title**, which is *better* than a name-split.
2. **Type drives the widget; an attribute refines it.** The default is
   a pure function of the member's type; attributes are *overrides*.
   AREST's analogue: the Fact Type's **value-type Noun** (its
   `Object Type`, `Format`, `Enum Values`) drives the widget; a per-
   reading override refines it.
3. **Two passes: build + fetch.** `Populate` builds the element tree;
   `Fetch` reads edited values back into the object (`Reflect.cs`
   L434). AREST's analogue: project → render → on submit, the edited
   element values become `apply`/`create` facts (the form is the
   inverse projection).

**What MT.D *cannot* express (and AREST can):** required-ness (MT.D has
no `[Required]`), uniqueness, enum *value constraints* distinct from C#
`enum`, an identifying caption derived from a reference scheme, or an
action menu derived from a workflow. AREST has all of these as
first-class facts, so the projection is richer than reflection.

## 2. Target vocabularies (what we project *into*)

Two existing AREST vocabularies are the projection targets; the new
reading does **not** restate their populations.

- **`readings/ui/components.md`** — the toolkit-agnostic `Component`
  registry (Role ∈ {button, text-input, list, date-picker, dialog,
  image, slider, combo-box, progress-bar, checkbox, tab, menu, card}),
  each with `Property`/`Event`/`Slot`/`Trait` facts and per-toolkit
  `ImplementationBinding`s. This is the **widget vocabulary** the
  projection emits. The component *selection* (which toolkit) is
  already a solved problem there (the `is preferred for MonoView`
  derivation rules score bindings against MonoView constraints).
- **`readings/ui/monoview.md`** — the per-app **surface**: `MonoView`,
  `Region` (slot/role/surface-tier), `Pane Mode`. This is the
  **layout vocabulary** — *where* the projected components land
  (sidebar/content/detail/command-bar).
- **`apps/ui.do/src/ifactr/contract.ts`** — the iFactr serialized-object
  TS contract: the **wire shape** a HATEOAS thin client (ui.do)
  deserializes. The projection's output, serialized, must be faithful
  to these shapes (`iLayer`/`iItem`/`Link`/`Button` for the legacy
  graph; `IListView`/`Section`/`ICell`/`IMenu` for the modern view
  model).

## 3. The mapping table (FORML 2 → MT.D Element → Component / iFactr)

Three core views. Columns: the FORML 2 source (cited against
`core.md` / `state.md` fact types), the MT.D-style element it is the
analogue of, the `components.md` Component Role, and the
`contract.ts` shape.

### 3.1 Collection / list view  (`get({noun})` — list all instances)

Projected from the Noun's **reference scheme** + a small summary
projection (the identifying fact + at most one or two summary facts).

| FORML 2 source | MT.D analogue | components.md | contract.ts |
| --- | --- | --- | --- |
| the Noun itself | `RootElement(title)` / a screen | — (the surface) | `IListView` (`ViewKind:"ListView"`), `Title` = Noun `Plural` |
| each instance | a `StringElement` row (drill-in) | `list` Component, one row each | `Section.Cells[]` of `IContentCell` **or** legacy `iList.Items[]` of `iItem` |
| Noun **reference scheme** (`Noun has Reference Scheme Noun`) → the identifying value | the row's caption | row primary text | `IContentCell.TextLabel` / `iItem.Text` |
| a designated **summary** Fact Type (see §6 Q3) | row subtitle | row subtext | `IContentCell.SubtextLabel` / `SubtextItem.Subtext` |
| drill-in to the instance | tappable row → nested `RootElement` | `list` selection event | `IContentCell.NavigationLink` (`Link.Address` = instance URL) |
| SM transitions on the Noun that are *creational* (no `from` Status) | top "+" button | `button` (primary) | `iLayer.ActionButtons[]` `Button{Action:Add}` / `IMenu` button |
| `Status` value (if the Noun has an SM) | trailing badge | (text/`image`) | `IContentCell.ValueLabel` |

### 3.2 Instance / detail + form view  (`get({noun,id})`)

Projected from the Noun's **Fact Types** (one element per fact-type
role that this Noun fills as the *non-identifying* side), with the
**value type** choosing the widget and the **constraints** choosing
required/edit behavior. This is the direct analogue of MT.D's
`Populate`.

| FORML 2 source | MT.D analogue | components.md | contract.ts |
| --- | --- | --- | --- |
| Fact Type whose value type Noun has `Object Type 'value'`, `Format` text/none | `EntryElement` | `text-input` | `iItem`/field cell + edit control |
| value type with `Format 'date'` | `DateElement` | `date-picker` | edit cell (date) |
| value type with `Object Type 'value'`, numeric (`Minimum`/`Maximum` set) | `FloatElement` (`[Range]`) | `slider` (bounded) or `text-input` (unbounded) | edit cell + min/max |
| value type that **is an enum** (`Noun has Enum Values`) | `enum` → radio `Section` | `combo-box` (n large) / radio set (n small) | `iList` of radio `iItem` / `<select>` |
| Fact Type to a `bool`-typed value type | `BooleanElement` / `[Checkbox]`→`CheckboxElement` | `checkbox` | toggle cell |
| Fact Type whose **value type is another Noun** (entity ref) | nested `RootElement` (drill-in) **or** picker | `combo-box` (pick existing) / `card` (embedded) | `IContentCell.NavigationLink` to the related instance |
| **mandatory** fact (`Frequency Constraint`, `Min Occurrence ≥ 1`, family `mandatory`) | (MT.D can't) → element flagged required | required marker on the Component; submit-blocking | `IListView.ValidationErrors` keyed by control |
| **uniqueness** constraint (family `uniqueness`) over the role | (MT.D can't) → inline-validated field | async-validated `text-input` | `ValidationErrors` entry on conflict |
| Noun `Title` / Fact Type **reading** | element `[Caption]` | Component `display- Title` | section header / cell label |
| facts grouped by sub-aspect (e.g. one SM's-worth) | `[Section]` | `card` (one per group) | `Section` (header/footer) |
| the instance's identifying caption (reference scheme) | screen title | view `Title` | `iLayer.Title` / `IView.Title` |
| submit (write the edited facts back via `apply`/`create`) | MT.D `Fetch()` | `button` (primary, `Submit`) | `SubmitButton` (`Action:Submit`) → POST |

### 3.3 Menu / actions view  (Theorem 4, verbatim)

Projected from the **state machine** — the transitions whose `from`
Status is the instance's current Status. **This already exists** as the
`actions` MCP verb / `_links` block; the view projection just renders
it.

| FORML 2 source | MT.D analogue | components.md | contract.ts |
| --- | --- | --- | --- |
| `Transition` with `from Status` = current Status (`state.md`) | `StringElement` + `[OnTap]` | `button` per transition / `menu` of buttons | `IMenu.Buttons[]` `IMenuButton{NavigationLink}` / `iLayer.ActionButtons[]` |
| the Transition's name / reading | the action caption | button `text` | `Button.Text` (or derived from `Action`) |
| Transition's **target** Status | (n/a — informational) | tooltip | `Link.ConfirmationText` (optional) |
| a `Guard` that prevents the transition (`state.md`) | element omitted/disabled | button `enabled=false` | link omitted (Theorem 4: not in `_links`) |
| a Verb performed *in* the current Status (Moore) | `[OnTap]` standing action | `button` | `iLayer.ActionButtons[]` |
| transition POST target | `[OnTap]` callback | button `clicked` event | `Link{Address:"…/transition", RequestType:Async, Parameters:{event}}` |
| terminal Status (`links(s)=∅`) | empty section | no menu | `_links: {}` (Corollary: Deletion) |

The **MXController binding** = the inverse direction: a button's
`clicked`/`OnTap` (a `Component` Event from components.md) carries the
transition name as the `Link.Parameters.event`; the renderer POSTs to
`…/transition`; the SM fold advances; the re-projected view's `_links`
reflect the new Status. The controller is not bespoke per Noun — it is
the generic "fire the event named by the link, re-fetch the projection"
loop, exactly the iFactr `IMXController`/`iLayer` round-trip in
`contract.ts`.

## 4. The projection AS derivations (rule shapes)

Per the AREST thesis the projection must be **modeled, not hand-coded**.
Below are the rule *shapes* — they join the metamodel fact types
(`readings/core/core.md`, `readings/core/state.md`) to the UI fact
types (`readings/ui/components.md`). They introduce a thin set of new
fact types (the "view-element" layer) the follow-up reading will own.
The concrete one-Noun worked example is in
`readings/ui/view-projection.draft.md`.

### 4.1 New fact types the projection populates (sketch)

```
View(.Name) is an entity type.
ViewElement(.id) is an entity type.

View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.

View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.

ViewElement belongs to View.
  Each ViewElement belongs to exactly one View.
ViewElement renders Fact Type.            -- the source fact (form/detail)
ViewElement renders Transition.           -- the source transition (menu)
ViewElement has Component Role.           -- the components.md Role to instantiate
  Each ViewElement has exactly one Component Role.
ViewElement is required.                  -- from a mandatory constraint
ViewElement has display- Title.           -- the caption (from reading/Title)
ViewElement has Order.
```

The renderer then runs the **component-selection** rules already in
`components.md` (`ImplementationBinding is preferred for MonoView …`) to
pick a toolkit binding per `ViewElement`'s `Component Role`, and drops
each element into a `Region` per `monoview.md`. So the projection is a
*pipeline of derivations*: Noun → ViewElements → Component Role →
ImplementationBinding → Region.

### 4.2 Value-type → Component Role (the MT.D §1.1 analogue, modeled)

```
+ ViewElement (E) has Component Role 'text-input'
    if ViewElement (E) renders Fact Type (FT)
    and FT has Role played by Noun (V)
    and V has Object Type 'value'
    and V has Format 'text'.        -- (or: V has no Format)

+ ViewElement (E) has Component Role 'date-picker'
    if ViewElement (E) renders Fact Type (FT)
    and FT has Role played by Noun (V)
    and V has Format 'date'.

+ ViewElement (E) has Component Role 'checkbox'
    if ViewElement (E) renders Fact Type (FT)
    and FT has Role played by Noun (V)
    and V has Format 'boolean'.

+ ViewElement (E) has Component Role 'combo-box'
    if ViewElement (E) renders Fact Type (FT)
    and FT has Role played by Noun (V)
    and V has some Enum Values.     -- enum value type → picker (cf. MT.D enum→radio)

+ ViewElement (E) has Component Role 'slider'
    if ViewElement (E) renders Fact Type (FT)
    and FT has Role played by Noun (V)
    and V has some Minimum
    and V has some Maximum.         -- bounded numeric → slider (cf. MT.D [Range])
```

These mirror MT.D's type switch (`Reflect.cs` L247-395) but key off the
*value type Noun's* metamodel facts instead of a CLR `Type`. Convention
choice Q1 (§6) is whether enum→combo-box or enum→radio is the default.

### 4.3 Constraint → required / validation (the thing MT.D lacks)

```
+ ViewElement (E) is required
    if ViewElement (E) renders Fact Type (FT)
    and some Frequency Constraint (C) spans some Role of FT
    and C has Min Occurrence (n) and n >= 1.

  -- equivalently keyed off Constraint Kind Family 'mandatory'
  -- (core.md: Constraint Kind Family ∈ {…, 'mandatory', 'uniqueness', …}).
```

Uniqueness constraints (`family 'uniqueness'`) project to an
async-validated control; on conflict the renderer populates
`IListView.ValidationErrors` (contract.ts). Value constraints
(`Minimum`/`Maximum`/`Pattern`/`Enum Values` on the value-type Noun)
project to the widget's own bounds (slider min/max, input pattern,
combo-box item set).

### 4.4 Caption (the MT.D `MakeCaption` analogue, but better)

```
+ ViewElement (E) has display- Title (T)
    if ViewElement (E) renders Fact Type (FT)
    and FT has Title (T).                 -- prefer the Fact Type's authored Title

+ ViewElement (E) has display- Title (T)
    if ViewElement (E) renders Fact Type (FT)
    and no FT has some Title
    and T is the reading of FT.           -- fall back to the FT reading text
```

AREST never has to CamelCase-split a member name (MT.D's `MakeCaption`)
because the reading/Title is authored prose already.

### 4.5 Menu = Theorem 4 (extend the existing actions projection)

```
+ ViewElement (E) renders Transition (Tr)
    and ViewElement (E) has Component Role 'button'
    if View (Vw) is for Noun (N)
    and Vw has View Kind 'menu'
    and State Machine Definition (SM) is for Noun (N)
    and Transition (Tr) is defined in SM
    and Transition (Tr) is from Status (S)
    and <entity's current Status is S>.   -- the Theorem-4 link filter

  -- A Guard that prevents Tr removes the element (the contract.ts
  -- `_links` omission). Expressed as a negative antecedent — same
  -- negation idiom the state.md authors flag (parser-side negation
  -- is a known follow-up).
```

This rule is the *generalisation point*: today the SM fold projects
`_links`; this rule projects the same set as `ViewElement`s of a `menu`
View. The action-menu view and the HATEOAS `_links` are the **same
projection** viewed at two fidelities.

### 4.6 Collection rows from the reference scheme

```
+ ViewElement (E) has Component Role 'list'
    if View (Vw) is for Noun (N)
    and Vw has View Kind 'collection'.

+ <row caption> is the reference-scheme value of the instance
    if N has Reference Scheme Noun (R)
    and <instance's R-valued fact>.        -- row primary text = identifying value
```

## 5. Defaults vs overrides (MT.D-style, never hard-coded)

MT.D's discipline: **type → element is the default; an attribute
overrides it.** AREST keeps the discipline but moves both halves into
facts so neither is hard-coded against a Noun.

- **Default convention** = the §4 derivation rules. They are universal:
  they fire for *every* Noun off its value types / constraints / SM.
  No rule branches on a Noun's identity (same property the
  `components.md` selection rules guarantee — Role is "an opaque key").
- **Override** = an asserted `ViewElement has Component Role …` (or a
  `View … is hidden` / `ViewElement has Order …`) fact in a *domain
  reading*. Because asserted facts win over derived ones in the
  chainer (the derivation only fires when the override is absent — the
  same `no … is such that` guard `monoview.md`'s effective-Pane-Mode
  rules use), a reading author overrides the projection for one Fact
  Type without touching the rules. This is the FORML 2 analogue of
  MT.D's `[Entry]`/`[Caption]`/`[Section]` attributes: an attribute is
  "an override fact attached to the member."
- **Skip** (MT.D `[Skip]`) = a `ViewElement is suppressed` /
  `Fact Type is not projected` fact; the renderer drops it.
- **Sectioning** (MT.D `[Section]`) = grouping `ViewElement`s under a
  `card` Component / `Section` by a `ViewElement belongs to Group`
  fact, defaulting to one section per SM-aspect or per reference-scheme
  partition.

The override surface is therefore: per-Noun *view* facts and per-Fact-
Type *element* facts asserted in a domain reading, exactly the way an
app asserts a `PanePreference` to override pane mode in `monoview.md`.

## 6. Open questions / convention choices for the user (iFactr expert)

1. **enum default widget.** MT.D maps a C# `enum` to a *radio group*
   (one `RadioElement` per value). AREST has `combo-box` and (via the
   `list`) radio-style sets. Default enum → `combo-box`, or →
   radio-set when the value count is small (≤ N)? What N?
2. **entity-valued fact (FK) default.** When a Fact Type's value type
   is another **Noun**, MT.D's only analogue is a nested drill-in
   `RootElement`. Options: (a) `combo-box` picking an existing
   instance, (b) `card` embedding a summary projection of the related
   instance + a `NavigationLink`, (c) drill-in only. iFactr's
   `iItem.Link` / `IContentCell.NavigationLink` supports (c) cleanly —
   is drill-in the default with combo-box as the *edit* affordance?
3. **summary fact for the collection subtitle.** The reference scheme
   gives the row's *primary* text. What picks the **subtitle** (the
   `SubtextLabel`)? Options: a designated `Fact Type is summary for
   Noun` fact (explicit, MT.D-free), or a heuristic (first mandatory
   text-valued fact). Recommend explicit; confirm.
4. **list vs grid for the collection.** contract.ts has both
   `IListView` and `IGridView`. Default collection → `IListView`
   always, or → `IGridView` when the Noun has ≥ K summary facts
   (card-grid)? iFactr's `ColumnMode`/`TwoColumns` is the middle
   ground.
5. **modern vs legacy iFactr layer as the wire target.** contract.ts
   carries *both* the modern `IListView`/`Section`/`ICell` model and
   the legacy serialized `iLayer`/`iItem`/`iList` graph. Which is the
   projection's canonical output — the modern view model (cells via
   the projection inlined into `Sections[].Cells`), or the legacy
   serialized layer graph (`iLayer.Items[]`)? (See §7 — this is also
   the monoview.md ⇄ contract.ts reconciliation question.)
6. **form submit semantics.** MT.D `Fetch()` writes back to the same
   object. AREST's inverse projection turns edited elements into
   `apply`/`create` facts. Confirm: a `SubmitButton` POSTs the whole
   changed fact set as one `create`/`apply` (transactional), and
   per-field uniqueness/mandatory failures come back as
   `IListView.ValidationErrors` keyed by the originating Fact Type.
7. **Moore actions placement.** A `Verb performed in Status` (Moore)
   is a standing action of the *current state*, not a transition. Does
   it render in the same `menu`/`IMenu` as the transitions, or as a
   distinct command-bar `Region`?
8. **identity of the override grain.** Is the override attached to the
   `(Noun, Fact Type)` pair (one element), to the `(View, Fact Type)`
   (per-view), or to a `(MonoView, Noun, Fact Type)` triple (per app
   surface)? This decides whether the same Noun can project
   differently in two apps.

## 7. Reconciliation note: monoview.md ⇄ contract.ts

The task flagged a likely overlap. They are **not** duplicates; they are
two ends of the same pipe, and the new view-projection layer is what
joins them:

- **`readings/ui/monoview.md`** is the **AREST-native surface model**:
  `MonoView` + `Region` (slot / role / surface-tier / pane-mode
  visibility). It answers *"where does content go on this app's
  screen, under this pane mode, at this density?"* It is toolkit- and
  wire-agnostic and already feeds the `components.md` selection rules.
- **`apps/ui.do/src/ifactr/contract.ts`** is the **iFactr wire shape**:
  `IView`/`IListView`/`IGridView` (+ `Section`/`ICell`/`IMenu`) for the
  modern model and `iLayer`/`iItem`/`Link` for the legacy serialized
  graph. It answers *"what JSON/XML does a ui.do thin client
  deserialize?"* It is iFactr-specific.

**The overlap is real but shallow:** both describe "a view with regions
of cells and a menu." The concepts line up cleanly —

| monoview.md | contract.ts | view-projection role |
| --- | --- | --- |
| `MonoView` | `IView` / `iLayer` | the projected screen frame |
| `Region` (slot `content`) | `IListView.Sections` / `iLayer.Items` | where rows/fields land |
| `Region` (slot `detail`) | detail pane / `DetailLink` | instance view target |
| `Region` (role `action`, slot `command-bar`) | `IMenu` / `iLayer.ActionButtons` | the menu projection (§3.3) |
| `Pane Mode` `master-detail` | composite/`DetailLink` layout | list↔detail relationship |

**Recommended reconciliation (for the user / the ui.do agent — NOT done
here):**

1. Keep **monoview.md as the source of truth** for the *surface*
   (regions, pane mode, density, a11y). It is AREST-native and
   already wired to selection.
2. Treat **contract.ts as a serialization target** the projection
   emits *into* — i.e. `Region(role action) → IMenu`,
   `Region(slot content) + collection View → IListView.Sections`,
   `instance View → iLayer` (or modern `IListView` per Q5). The
   `view-projection-design` layer owns this mapping; contract.ts stays
   a faithful iFactr extraction.
3. Avoid restating either population in the new reading (same
   discipline components.md/monoview.md already follow). The new
   reading *joins*, it does not copy.
4. One concrete divergence to resolve with the ui.do agent: contract.ts
   `IView.Metadata`/`BackgroundColor` chrome vs monoview.md
   `Surface Tier`/design-token (#432) chrome — these are two spellings
   of the same thing and should map, not coexist.

## 8. What a follow-up implements

1. Promote §4 + the draft into an additive FORML 2 reading
   (`readings/ui/view-projection.md`) with the new `View`/`ViewElement`
   fact types and the derivation rules, checker-clean (resolving the
   negation/cardinality idioms the way state.md/monoview.md document).
2. Wire the `get`/`actions` MCP verbs to emit the projected `View` as
   either a `components.md` composition or a contract.ts wire shape
   (per Q5).
3. Settle Q1-Q8 with the user and bake the chosen defaults as the
   `+`-derivation defaults; leave the override facts as the escape
   hatch.

## References

- `MonoTouch.Dialog/Reflect.cs` (Miguel de Icaza) — reflection→Element
  conventions (cloned read-only for study; not vendored).
- `readings/core/core.md` — Noun / Fact Type / Role / Constraint /
  reference scheme / value-type metamodel.
- `readings/core/state.md`, `docs/04-state-machines.md` — State
  Machine, Transition, Guard; Theorem 4 (HATEOAS as Projection).
- `readings/ui/components.md` — Component registry + selection rules.
- `readings/ui/monoview.md` — MonoView surface (regions / pane mode).
- `apps/ui.do/src/ifactr/contract.ts` (+ `contract.md`) — iFactr
  serialized-object wire contract.
