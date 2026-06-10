# AREST UI: Render Target — render functions in DEFS (whitepaper §5.2)

This reading reifies the §5.2 Platform Binding render seam as facts. A
Render Target is one registered render function: a `Func::Platform`
entry in DEFS (the `externals.rs` pattern — `register_runtime_fn` binds
the name, `install_platform_fn` attaches the callable at boot) that
turns the ViewProjection emitted by `view_via_rho`
(`readings/ui/view-projection.md`) plus an entity population into a
target-native widget tree. This reading declares WHICH targets exist
and which DEFS name each answers to, so render dispatch is a fact walk
— ρ over the Render Target population — not a hard-coded match in Rust.
A new target platform is one new Render Target instance plus one
installed function; no per-app glue, and a renderer that must know
about a specific app has leaked the seam.

The composition direction follows `readings/ui/render.md`: a Render
Target produces the CONTENT that lands in some Surface as Frames; the
Display / Surface / Frame substrate carries the bytes and never learns
the widget vocabulary, and this reading never restates geometry. The
widget vocabulary a render function consumes is the closed `Component
Role` catalog from `readings/ui/components.md`, resolved per value
type by the Format-else-CDT layer in `readings/ui/view-projection.md`.

## Entity Types

Render Target(.Name) is an entity type.
  <!-- One registered render function. The .Name reference mode is a
       stable slug — 'html' for the engine's reference markup
       renderer, 'slint' for the in-kernel surface, 'ui-do' for the
       hosted TS worker. The slug names the TARGET PLATFORM, not the
       app being rendered: every app renders through every installed
       target. -->

## Value Types

Platform Function Name is a value type.
  <!-- The DEFS binding the runtime installs the callable under, e.g.
       'render:html'. The 'render:' prefix is the namespace convention
       for render functions; the engine applies the bound Func to
       [view facts, entity data, affordances] when the target's
       function has an installed body, and skips the target gracefully
       (Object::Bottom discipline) when no body is installed. -->

MimeType is a value type.
  <!-- Media type of the rendered output, e.g. 'text/html'. Same value
       type the filesystem reading declares for File; declared here
       too so the ui-readings scope stands alone. Absent on targets
       whose output is not byte-addressed (an in-process widget tree
       handed straight to a toolkit). -->

## Fact Types

### Render Target

Render Target has Platform Function Name.
  Each Render Target has exactly one Platform Function Name.

Render Target emits MimeType.
  Each Render Target emits at most one MimeType.

Render Target has display- Title.
  Each Render Target has at most one display- Title.

Render Target has Description.
  Each Render Target has at most one Description.

## Constraints

No two Render Targets share the same Platform Function Name.

## Deontic Constraints

It is obligatory that each Render Target has some Platform Function Name.

## Instance Facts

### Render Target: the reference HTML renderer

Render Target 'html' has Platform Function Name 'render:html'.
Render Target 'html' emits MimeType 'text/html'.
Render Target 'html' has display- Title 'Reference HTML renderer'.
Render Target 'html' has Description 'Engine-installed reference render function: walks the ViewProjection elements in Order, emits one labelled widget per Component Role (text-input, date-picker, checkbox, combo-box) and one rel=transition anchor per HATEOAS affordance. Pure function of its input; knows nouns and widgets, never apps.'.
