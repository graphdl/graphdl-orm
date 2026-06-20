# AREST UI: Render Target instances — the installed render-function registry

This reading carries the Render Target INSTANCE population — the rows
`render_via_targets` (`command.rs`) walks to dispatch a rendering. It was
split out of the always-loaded schema reading `render-target.md` for
view-tree-shaking (2026-06): the Render Target NOUN + its fact types stay
in the base (every app can SEE the schema), but the instances ride this
per-app OVERLAY (`UI_VIEW_READINGS` in lib.rs) so they land ONLY for an
app that declares `App '<slug>' uses Render Surface '<surface>'`.

A UI-less agent (tasks / claude / arc-agi-3) never loads this file, so
`Render_Target_has_Platform_Function_Name` is empty and
`render_via_targets` returns no representations — the pure-CRM no-op the
engine already implements. Adding a target platform is still one new
Render Target instance here plus one installed function; no per-app glue.

The schema this populates lives in `render-target.md` (the Render Target
noun, `Render Target has Platform Function Name`, `… emits MimeType`,
`… has display- Title`, `… has Description`).

## Instance Facts

### Render Target: the reference HTML renderer

Render Target 'html' has Platform Function Name 'render:html'.
Render Target 'html' emits MimeType 'text/html'.
Render Target 'html' has display- Title 'Reference HTML renderer'.
Render Target 'html' has Description 'Engine-installed reference render function: walks the ViewProjection elements in Order, emits one labelled widget per Component Role (text-input, date-picker, checkbox, combo-box) and one rel=transition anchor per HATEOAS affordance. Pure function of its input; knows nouns and widgets, never apps.'.
