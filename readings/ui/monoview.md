# AREST UI: MonoView Surface

This reading defines the per-app render surface — what the user calls
the "MonoView" — as FORML 2 fact types and instances. Pre-#457 the
pane mode of each app (master-detail vs single-pane vs popover) was a
hard-coded property of each `.slint` file, and downstream consumers
(ui.do React surface, terminal renderer) had no way to introspect or
override it without round-tripping through Slint source. After #457,
the entire surface — pane mode, regions, transitions, density,
interaction mode, surface tier, accessibility profile — is a reading,
the same way design tokens already are (#432, `readings/ui/design.md`).

The benefits are the FORML 2 dividend: derivations (touch interaction
implies spacious density without anyone restating it), validation via
the readings checker (a region cannot belong to two MonoViews), and
the same MCP / HATEOAS introspection over UI surface that already
works over data. A user-installed app advertises its preferences as a
`PanePreference` fact; the kernel projects them through the
derivation graph and the renderer reads the resulting facts. There is
no UI code that branches on app identity.

This reading is additive over `readings/ui/ui.md` (the platform-
agnostic view hierarchy) and `readings/ui/design.md` (the token
layer). The Slint kernel surface and the ui.do React frontend are
the two consumers; both follow as separate tracks (#457b/c).

## Entity Types

MonoView(.Name) is an entity type.

Region(.Name) is an entity type.

PanePreference(.Name) is an entity type.

App Role(.Name) is an entity type.
  HateoasBrowser is a subtype of App Role.
  REPL is a subtype of App Role.
  FileBrowser is a subtype of App Role.
  Settings is a subtype of App Role.
  {HateoasBrowser, REPL, FileBrowser, Settings} are mutually exclusive subtypes of App Role.

## Value Types

Pane Mode is a value type.
  The possible values of Pane Mode are
    'master-detail', 'popover', 'single-pane', 'tabs', 'stack'.

Region Role is a value type.
  The possible values of Region Role are
    'navigation', 'context', 'detail', 'status', 'action'.

Region Slot is a value type.
  The possible values of Region Slot are
    'sidebar', 'content', 'detail', 'footer', 'command-bar',
    'header', 'overlay'.

Transition Style is a value type.
  The possible values of Transition Style are
    'slide', 'fade', 'swap', 'stack', 'none'.

Density Scale is a value type.
  The possible values of Density Scale are
    'compact', 'regular', 'spacious'.

Interaction Mode is a value type.
  The possible values of Interaction Mode are
    'pointer', 'keyboard', 'touch'.

Surface Tier is a value type.
  The possible values of Surface Tier are
    'backdrop', 'panel', 'overlay', 'drop-shadow'.

A11y Profile is a value type.
  The possible values of A11y Profile are
    'high-contrast', 'reduced-motion', 'screen-reader-aware'.

Override Source is a value type.
  The possible values of Override Source are
    'app-default', 'user', 'platform-policy', 'derived'.

Hit Target Size is a value type.

Z Index is a value type.

## Fact Types

### MonoView

MonoView is for App Role.
  Each MonoView is for exactly one App Role.

MonoView has display- Title.
  Each MonoView has at most one display- Title.

MonoView has default Pane Mode.
  Each MonoView has exactly one default Pane Mode.

MonoView has default Density Scale.
  Each MonoView has exactly one default Density Scale.

MonoView has default Interaction Mode.
  Each MonoView has exactly one default Interaction Mode.

MonoView has default A11y Profile.

MonoView has Description.
  Each MonoView has at most one Description.

### Region

Region belongs to MonoView.
  Each Region belongs to exactly one MonoView.

Region has Region Slot.
  Each Region has exactly one Region Slot.

Region has Region Role.
  Each Region has exactly one Region Role.

Region has Transition Style.
  Each Region has at most one Transition Style.

Region has Surface Tier.
  Each Region has exactly one Surface Tier.

Region has Z Index.
  Each Region has at most one Z Index.

Region has display- Title.
  Each Region has at most one display- Title.

Region is visible in Pane Mode.

### PanePreference

PanePreference is for App Role.
  Each PanePreference is for exactly one App Role.

PanePreference has Pane Mode.
  Each PanePreference has exactly one Pane Mode.

PanePreference has Override Source.
  Each PanePreference has exactly one Override Source.

PanePreference has Description.
  Each PanePreference has at most one Description.

### Density / Interaction cross-product

Interaction Mode implies minimum Hit Target Size.
  Each Interaction Mode implies at most one minimum Hit Target Size.

Density Scale has row- Pixels.
  Each Density Scale has exactly one row- Pixels.

### Accessibility

A11y Profile suppresses Transition Style.

A11y Profile pins Density Scale.
  Each A11y Profile pins at most one Density Scale.

## Constraints

Each MonoView is for at most one App Role.
No two MonoViews are for the same App Role.

Each Region belongs to at most one MonoView.
No two Regions belonging to the same MonoView share the same Region Slot.

Each PanePreference is for at most one App Role.
No two PanePreferences with Override Source 'app-default' are for the same App Role.

Each Hit Target Size used by an Interaction Mode is a non-negative multiple of 4.
Each Pixels value used as row- Pixels by a Density Scale is a non-negative multiple of 4.

## Deontic Constraints

It is obligatory that each MonoView has some Region with Region Role 'navigation'
  or that MonoView has default Pane Mode 'single-pane'.
It is obligatory that each MonoView has some Region with Region Slot 'content'.
It is obligatory that each App Role has some PanePreference with Override Source 'app-default'.
It is obligatory that each Density Scale has some row- Pixels.
It is obligatory that each Interaction Mode has some minimum Hit Target Size.

## Derivation Rules

* Density Scale 'compact'  has row- Pixels 24.
* Density Scale 'regular'  has row- Pixels 32.
* Density Scale 'spacious' has row- Pixels 44.

* Interaction Mode 'pointer'  implies minimum Hit Target Size 24.
* Interaction Mode 'keyboard' implies minimum Hit Target Size 24.
* Interaction Mode 'touch'    implies minimum Hit Target Size 44.

+ MonoView has default Density Scale 'spacious'
    if MonoView has default Interaction Mode 'touch'.

+ MonoView has default A11y Profile 'reduced-motion'
    if MonoView has default Interaction Mode 'touch'
    and MonoView has default Density Scale 'spacious'.

+ Region has Transition Style 'none'
    if Region belongs to MonoView
    and MonoView has default A11y Profile 'reduced-motion'.

+ PanePreference has Pane Mode 'single-pane'
    if PanePreference is for App Role
    and that App Role is a REPL
    and PanePreference has Override Source 'app-default'.

+ MonoView has effective Pane Mode if MonoView is for App Role
    and some PanePreference is for that App Role
    and that PanePreference has Override Source 'user'
    and effective Pane Mode is that PanePreference's Pane Mode.

+ MonoView has effective Pane Mode if MonoView is for App Role
    and some PanePreference is for that App Role
    and that PanePreference has Override Source 'app-default'
    and no PanePreference is for that App Role with Override Source 'user'
    and effective Pane Mode is that PanePreference's Pane Mode.

## Instance Facts

Domain 'ui' has Description 'MonoView surface — the per-app render surface as FORML 2 facts. Pane mode, regions, transitions, density, interaction, surfaces, and accessibility are all readings so derivations + validation + MCP introspection apply uniformly.'.

### Density / Interaction defaults

Density Scale 'compact'  has row- Pixels 24.
Density Scale 'regular'  has row- Pixels 32.
Density Scale 'spacious' has row- Pixels 44.

Interaction Mode 'pointer'  implies minimum Hit Target Size 24.
Interaction Mode 'keyboard' implies minimum Hit Target Size 24.
Interaction Mode 'touch'    implies minimum Hit Target Size 44.

### A11y defaults

A11y Profile 'reduced-motion' suppresses Transition Style 'slide'.
A11y Profile 'reduced-motion' suppresses Transition Style 'fade'.
A11y Profile 'reduced-motion' suppresses Transition Style 'stack'.
A11y Profile 'reduced-motion' pins Density Scale 'regular'.
A11y Profile 'high-contrast'  pins Density Scale 'regular'.

### App: HateoasBrowser

App Role 'hateoas-browser' is a HateoasBrowser.

MonoView 'hateoas' is for App Role 'hateoas-browser'.
MonoView 'hateoas' has display- Title 'HATEOAS Browser'.
MonoView 'hateoas' has default Pane Mode 'master-detail'.
MonoView 'hateoas' has default Density Scale 'regular'.
MonoView 'hateoas' has default Interaction Mode 'pointer'.
MonoView 'hateoas' has default A11y Profile 'screen-reader-aware'.
MonoView 'hateoas' has Description 'Resources column on the left, Instances in the center, Detail on the right. Master-detail under pointer, collapses to stack under touch.'.

Region 'hateoas.resources' belongs to MonoView 'hateoas'.
Region 'hateoas.resources' has Region Slot 'sidebar'.
Region 'hateoas.resources' has Region Role 'navigation'.
Region 'hateoas.resources' has Transition Style 'slide'.
Region 'hateoas.resources' has Surface Tier 'panel'.
Region 'hateoas.resources' has Z Index 10.
Region 'hateoas.resources' has display- Title 'Resources'.
Region 'hateoas.resources' is visible in Pane Mode 'master-detail'.
Region 'hateoas.resources' is visible in Pane Mode 'tabs'.

Region 'hateoas.instances' belongs to MonoView 'hateoas'.
Region 'hateoas.instances' has Region Slot 'content'.
Region 'hateoas.instances' has Region Role 'context'.
Region 'hateoas.instances' has Transition Style 'fade'.
Region 'hateoas.instances' has Surface Tier 'backdrop'.
Region 'hateoas.instances' has Z Index 0.
Region 'hateoas.instances' has display- Title 'Instances'.
Region 'hateoas.instances' is visible in Pane Mode 'master-detail'.
Region 'hateoas.instances' is visible in Pane Mode 'single-pane'.
Region 'hateoas.instances' is visible in Pane Mode 'tabs'.
Region 'hateoas.instances' is visible in Pane Mode 'stack'.

Region 'hateoas.detail' belongs to MonoView 'hateoas'.
Region 'hateoas.detail' has Region Slot 'detail'.
Region 'hateoas.detail' has Region Role 'detail'.
Region 'hateoas.detail' has Transition Style 'swap'.
Region 'hateoas.detail' has Surface Tier 'panel'.
Region 'hateoas.detail' has Z Index 20.
Region 'hateoas.detail' has display- Title 'Detail'.
Region 'hateoas.detail' is visible in Pane Mode 'master-detail'.
Region 'hateoas.detail' is visible in Pane Mode 'popover'.
Region 'hateoas.detail' is visible in Pane Mode 'stack'.

Region 'hateoas.command-bar' belongs to MonoView 'hateoas'.
Region 'hateoas.command-bar' has Region Slot 'command-bar'.
Region 'hateoas.command-bar' has Region Role 'action'.
Region 'hateoas.command-bar' has Transition Style 'fade'.
Region 'hateoas.command-bar' has Surface Tier 'overlay'.
Region 'hateoas.command-bar' has Z Index 100.

PanePreference 'hateoas.default' is for App Role 'hateoas-browser'.
PanePreference 'hateoas.default' has Pane Mode 'master-detail'.
PanePreference 'hateoas.default' has Override Source 'app-default'.
PanePreference 'hateoas.default' has Description 'Three-column master-detail is the canonical HATEOAS browse surface.'.

### App: REPL

App Role 'repl' is a REPL.

MonoView 'repl' is for App Role 'repl'.
MonoView 'repl' has display- Title 'REPL'.
MonoView 'repl' has default Pane Mode 'single-pane'.
MonoView 'repl' has default Density Scale 'regular'.
MonoView 'repl' has default Interaction Mode 'keyboard'.
MonoView 'repl' has default A11y Profile 'screen-reader-aware'.
MonoView 'repl' has Description 'Transcript stream above, prompt below — single pane is the only sensible default for a REPL.'.

Region 'repl.transcript' belongs to MonoView 'repl'.
Region 'repl.transcript' has Region Slot 'content'.
Region 'repl.transcript' has Region Role 'context'.
Region 'repl.transcript' has Transition Style 'none'.
Region 'repl.transcript' has Surface Tier 'backdrop'.
Region 'repl.transcript' has Z Index 0.
Region 'repl.transcript' has display- Title 'Transcript'.
Region 'repl.transcript' is visible in Pane Mode 'single-pane'.

Region 'repl.prompt' belongs to MonoView 'repl'.
Region 'repl.prompt' has Region Slot 'footer'.
Region 'repl.prompt' has Region Role 'action'.
Region 'repl.prompt' has Transition Style 'none'.
Region 'repl.prompt' has Surface Tier 'panel'.
Region 'repl.prompt' has Z Index 10.
Region 'repl.prompt' has display- Title 'Prompt'.
Region 'repl.prompt' is visible in Pane Mode 'single-pane'.

Region 'repl.status' belongs to MonoView 'repl'.
Region 'repl.status' has Region Slot 'header'.
Region 'repl.status' has Region Role 'status'.
Region 'repl.status' has Transition Style 'fade'.
Region 'repl.status' has Surface Tier 'panel'.
Region 'repl.status' has Z Index 5.

PanePreference 'repl.default' is for App Role 'repl'.
PanePreference 'repl.default' has Pane Mode 'single-pane'.
PanePreference 'repl.default' has Override Source 'app-default'.
PanePreference 'repl.default' has Description 'A REPL is a transcript + prompt; multi-pane chrome would be noise.'.

### App: FileBrowser

App Role 'file-browser' is a FileBrowser.

MonoView 'file-browser' is for App Role 'file-browser'.
MonoView 'file-browser' has display- Title 'File Browser'.
MonoView 'file-browser' has default Pane Mode 'master-detail'.
MonoView 'file-browser' has default Density Scale 'regular'.
MonoView 'file-browser' has default Interaction Mode 'pointer'.
MonoView 'file-browser' has default A11y Profile 'screen-reader-aware'.
MonoView 'file-browser' has Description 'Tree on the left, listing in the center, preview/metadata on the right.'.

Region 'file-browser.tree' belongs to MonoView 'file-browser'.
Region 'file-browser.tree' has Region Slot 'sidebar'.
Region 'file-browser.tree' has Region Role 'navigation'.
Region 'file-browser.tree' has Transition Style 'slide'.
Region 'file-browser.tree' has Surface Tier 'panel'.
Region 'file-browser.tree' has Z Index 10.
Region 'file-browser.tree' has display- Title 'Folders'.
Region 'file-browser.tree' is visible in Pane Mode 'master-detail'.
Region 'file-browser.tree' is visible in Pane Mode 'tabs'.

Region 'file-browser.listing' belongs to MonoView 'file-browser'.
Region 'file-browser.listing' has Region Slot 'content'.
Region 'file-browser.listing' has Region Role 'context'.
Region 'file-browser.listing' has Transition Style 'fade'.
Region 'file-browser.listing' has Surface Tier 'backdrop'.
Region 'file-browser.listing' has Z Index 0.
Region 'file-browser.listing' has display- Title 'Files'.
Region 'file-browser.listing' is visible in Pane Mode 'master-detail'.
Region 'file-browser.listing' is visible in Pane Mode 'single-pane'.
Region 'file-browser.listing' is visible in Pane Mode 'stack'.

Region 'file-browser.preview' belongs to MonoView 'file-browser'.
Region 'file-browser.preview' has Region Slot 'detail'.
Region 'file-browser.preview' has Region Role 'detail'.
Region 'file-browser.preview' has Transition Style 'swap'.
Region 'file-browser.preview' has Surface Tier 'panel'.
Region 'file-browser.preview' has Z Index 20.
Region 'file-browser.preview' has display- Title 'Preview'.
Region 'file-browser.preview' is visible in Pane Mode 'master-detail'.
Region 'file-browser.preview' is visible in Pane Mode 'popover'.
Region 'file-browser.preview' is visible in Pane Mode 'stack'.

Region 'file-browser.status' belongs to MonoView 'file-browser'.
Region 'file-browser.status' has Region Slot 'footer'.
Region 'file-browser.status' has Region Role 'status'.
Region 'file-browser.status' has Transition Style 'fade'.
Region 'file-browser.status' has Surface Tier 'panel'.
Region 'file-browser.status' has Z Index 5.

PanePreference 'file-browser.default' is for App Role 'file-browser'.
PanePreference 'file-browser.default' has Pane Mode 'master-detail'.
PanePreference 'file-browser.default' has Override Source 'app-default'.
PanePreference 'file-browser.default' has Description 'Master-detail with the detail column on the right; collapses to stack on small viewports.'.

### App: Settings

App Role 'settings' is a Settings.

MonoView 'settings' is for App Role 'settings'.
MonoView 'settings' has display- Title 'Settings'.
MonoView 'settings' has default Pane Mode 'master-detail'.
MonoView 'settings' has default Density Scale 'regular'.
MonoView 'settings' has default Interaction Mode 'pointer'.
MonoView 'settings' has default A11y Profile 'screen-reader-aware'.
MonoView 'settings' has Description 'Categories on the left, panel on the right.'.

Region 'settings.categories' belongs to MonoView 'settings'.
Region 'settings.categories' has Region Slot 'sidebar'.
Region 'settings.categories' has Region Role 'navigation'.
Region 'settings.categories' has Transition Style 'slide'.
Region 'settings.categories' has Surface Tier 'panel'.
Region 'settings.categories' has Z Index 10.
Region 'settings.categories' has display- Title 'Categories'.
Region 'settings.categories' is visible in Pane Mode 'master-detail'.
Region 'settings.categories' is visible in Pane Mode 'tabs'.

Region 'settings.panel' belongs to MonoView 'settings'.
Region 'settings.panel' has Region Slot 'content'.
Region 'settings.panel' has Region Role 'detail'.
Region 'settings.panel' has Transition Style 'fade'.
Region 'settings.panel' has Surface Tier 'backdrop'.
Region 'settings.panel' has Z Index 0.
Region 'settings.panel' has display- Title 'Settings'.
Region 'settings.panel' is visible in Pane Mode 'master-detail'.
Region 'settings.panel' is visible in Pane Mode 'single-pane'.
Region 'settings.panel' is visible in Pane Mode 'stack'.

PanePreference 'settings.default' is for App Role 'settings'.
PanePreference 'settings.default' has Pane Mode 'master-detail'.
PanePreference 'settings.default' has Override Source 'app-default'.
PanePreference 'settings.default' has Description 'Two-column categories + panel; collapses to stack on small viewports.'.
