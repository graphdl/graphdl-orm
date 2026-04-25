# AREST Compat: Wine Application Compatibility

This reading defines Wine application compatibility as FORML 2 fact
types and instances. Wine on Linux is hard not because it doesn't
work, but because configuring it per-application is unstructured
tribal knowledge — winetricks recipes, ProtonDB user reports, ad-hoc
DLL overrides, registry tweaks, environment-variable incantations.
FORML normalizes that knowledge: each Wine app's compat needs become
facts; common per-app overrides become derivation rules; the future
`arest run "App Name"` (#462c) compiles the FORML state into a Wine
prefix automatically.

This reading is the foundation slice of the Windows-via-Wine epic
(#462). Sibling slices: #462b ports a Mono-flavoured runtime layer
that consumes these facts; #462c lands the `arest run` CLI; #462e
ingests the public ProtonDB corpus through the existing federation
pipeline (the `External System 'protondb'` instance fact at the end
of this file is the federation handle that ingest will write
against).

The corpus reuses the External System noun from `readings/core/core.md`
and the Domain noun from the same file; no new metamodel entities are
introduced beyond the compat-specific nouns below.

## Entity Types

Wine App(.Name) is an entity type.

DXVK Version(.Name) is an entity type.

vkd3d-proton Version(.Name) is an entity type.

Required Component Anchor(.Recipe) is an entity type.
  <!-- Per-winetricks-recipe anchor noun. The Required Component value
       type carries the recipe identifier as a string; the anchor entity
       lets us hang descriptions / arch variants / one-off metadata off
       a single FORML resource so derivation rules have something to
       reference when they fan a recipe out. The (.Recipe) reference
       mode mirrors the Required Component value verbatim. -->

GPU Vendor(.Slug) is an entity type.
  <!-- Hardware-compat anchor. Vulkan-≥-1.3 escalation rules produce a
       hardware requirement against this noun; the runtime layer
       (#462c) cross-references it against `lspci`/DRM probe output to
       refuse-to-launch on unsupported GPUs. -->

Notice(.Slug) is an entity type.
  <!-- A short user-facing advisory that the runtime layer surfaces
       at prefix-build time (e.g. 'unsupported in current Wine',
       'expect occasional GPU-process crashes'). Decoupled from
       Compat Rating so derivation rules can fan ratings out into
       human-readable warnings the CLI prints during `arest run`. -->

## Value Types

Wine Version is a value type.
  <!-- Free-form because the upstream version space is not a closed
       enumeration: mainline ('8.0', '9.0'), Staging ('8.0-staging'),
       TKG ('9.0-tkg'), Proton ('proton-experimental', 'proton-7.0-6'),
       Glorious Eggroll ('proton-ge-9-x'), and one-off forks. The
       checker validates the value is a string; the runtime layer
       (#462c) parses the prefix to dispatch the install path. -->

DLL Behavior is a value type.
  The possible values of DLL Behavior are
    'native', 'builtin', 'native-then-builtin', 'builtin-then-native',
    'disabled'.
  <!-- Mirrors the WINEDLLOVERRIDES grammar. 'native' loads the
       Windows DLL out of the prefix; 'builtin' uses Wine's
       reimplementation; the hyphenated variants set fallback order;
       'disabled' refuses to load the DLL at all. -->

DLL Name is a value type.
  <!-- e.g. 'msvcr120.dll', 'msxml6.dll', 'd3d11.dll'. Conventional
       lowercase with a '.dll' suffix; the parser does not enforce
       that, but consumers (#462c) normalize before lookup. -->

Registry Path is a value type.
  <!-- e.g. 'HKCU\\Software\\Wine\\Direct3D'. Backslash-separated, root
       key as the first segment ('HKCU' / 'HKLM' / 'HKCR' / 'HKU').
       The runtime layer expands these into `regedit /S` script
       fragments at prefix-build time. -->

Registry Value is a value type.
  <!-- The string value to write at Registry Path. REG_SZ only in this
       slice; REG_DWORD / REG_BINARY land in #462c when the runtime
       layer needs to write GPU-vendor and renderer-mode keys. -->

Env Var Name is a value type.
  <!-- e.g. 'WINEDLLOVERRIDES', 'DXVK_HUD', 'PROTON_NO_ESYNC',
       'WINEDEBUG'. Conventionally upper-snake-case but not
       enforced. -->

Env Var Value is a value type.
  <!-- Free-form string; the runtime layer concatenates these into the
       per-app launch wrapper's environment. -->

Prefix Architecture is a value type.
  The possible values of Prefix Architecture are 'win32', 'win64'.

Required Component is a value type.
  <!-- A winetricks recipe identifier, e.g. 'vcrun2019', 'dotnet48',
       'corefonts', 'mfc140', 'd3dcompiler_47', 'gdiplus', 'msxml6',
       'riched20'. Validated at runtime against the winetricks
       verb list shipped by the host distribution. -->

Compat Rating is a value type.
  The possible values of Compat Rating are
    'platinum', 'gold', 'silver', 'bronze', 'borked'.
  <!-- ProtonDB-aligned ordinal. 'platinum' = runs perfectly out of
       the box; 'gold' = runs perfectly after tweaks; 'silver' = runs
       with minor issues; 'bronze' = runs but with major issues;
       'borked' = does not run. -->

Vulkan Version is a value type.
  <!-- Semver triple as a string, e.g. '1.3', '1.2', '1.1'. The
       transitive-resolution rule below treats this as totally
       ordered by lexicographic comparison after a numeric split. -->

DLL Override Family is a value type.
  The possible values of DLL Override Family are
    'msvc-runtime', 'mfc-runtime', 'dotnet-clr', 'msxml',
    'd3d-compiler', 'directx-2d', 'directx-text', 'directx-gi'.
  <!-- FORML 2 set-membership stand-in for the regex matches the
       chainer would otherwise want ('msvcr*.dll', 'msxml[0-9]+.dll',
       etc.). Each DLL Name belongs to at most one family via the
       `DLL Name belongs to DLL Override Family` fact type below; the
       derivation rules then fire on `belongs to` rather than on a
       string-pattern literal. ORM 2 does not admit literal regex in
       a derivation antecedent — every membership has to be an
       enumerated fact — so each new family member is one instance
       fact, not a pattern edit. -->

GPU Driver Version is a value type.
  <!-- Free-form version string keyed by the GPU Vendor it belongs to;
       e.g. '525' for an NVIDIA driver, 'rdna' for an AMD generation,
       'xe' for an Intel architecture. The runtime layer (#462c)
       parses these against the host's driver probe output. -->

Dotnet Framework Version is a value type.
  The possible values of Dotnet Framework Version are
    '2.0', '3.0', '3.5', '4.0', '4.5', '4.6', '4.7', '4.8'.
  <!-- The closed enumeration of Microsoft .NET Framework releases
       Wine's mscoree shim recognises. Each value maps 1:1 to a
       winetricks `dotnet<short>` recipe via the derivation rule
       below; '4.8' -> 'dotnet48', '4.7' -> 'dotnet472', etc. .NET 5+
       are .NET Core / .NET (no dot, no 'Framework') and are out of
       scope for this slice — they self-contain. -->

Notice Text is a value type.
  <!-- Short human-readable string, the body of a Notice instance. -->

## Fact Types

### Wine App

Wine App has display- Title.
  Each Wine App has at most one display- Title.

Wine App has Description.
  Each Wine App has at most one Description.

Wine App has Compat Rating.
  Each Wine App has exactly one Compat Rating.

Wine App has Prefix Architecture.
  Each Wine App has exactly one Prefix Architecture.

Wine App requires Wine Version.
  <!-- Free-multiplicity: a single app may name multiple known-good
       Wine versions; the runtime picks the first one available on the
       host. The `arest run` resolver walks the list in declared
       order. -->

Wine App requires DXVK Version.
  Each Wine App requires at most one DXVK Version.

Wine App requires vkd3d-proton Version.
  Each Wine App requires at most one vkd3d-proton Version.

Wine App requires Required Component.
  <!-- Many-per-app: a typical Office install needs several
       winetricks recipes. -->

### DLL Override

Wine App requires DLL Override of DLL Name with DLL Behavior.
  Each Wine App, DLL Name combination occurs at most once in the
    population of Wine App requires DLL Override.
  <!-- Ternary: the third role pins the override behaviour for the
       (App, DLL) pair. The (App, DLL) uniqueness keeps the
       WINEDLLOVERRIDES map injective per app, matching Wine's own
       last-write-wins parser semantics. -->

### Registry Key

Wine App requires Registry Key at Registry Path with Registry Value.
  Each Wine App, Registry Path combination occurs at most once in the
    population of Wine App requires Registry Key.
  <!-- Ternary again — Path is the key, Value is the data. The
       per-app uniqueness mirrors the registry's own one-value-per-key
       semantics for REG_SZ. -->

### Environment Variable

Wine App requires Environment Variable with Env Var Name and Env Var Value.
  Each Wine App, Env Var Name combination occurs at most once in the
    population of Wine App requires Environment Variable.

### DXVK / vkd3d-proton

DXVK Version requires Vulkan Version.
  Each DXVK Version requires exactly one Vulkan Version.

vkd3d-proton Version requires Vulkan Version.
  Each vkd3d-proton Version requires exactly one Vulkan Version.

Wine App requires Vulkan Version.
  Each Wine App requires at most one Vulkan Version.
  <!-- Derived; see Derivation Rules below. The transitive resolution
       lifts the Vulkan requirement from the chosen DXVK / vkd3d-proton
       version onto the app, so the runtime layer can ask one
       question ('does the host support Vulkan X?') instead of
       walking the dependency chain at every launch. -->

### Required Component anchors

Required Component Anchor has Recipe of Required Component.
  Each Required Component Anchor has exactly one Recipe.
  <!-- Pivots a Required Component value into the entity space so
       descriptions / arch variants attach somewhere. -->

Required Component Anchor has Description.
  Each Required Component Anchor has at most one Description.

Required Component Anchor has win64- Recipe of Required Component.
  Each Required Component Anchor has at most one win64- Recipe.
  <!-- A few winetricks recipes (notably the VC++ runtimes) ship a
       distinct verb for the win64 prefix — e.g. 'vcrun2019' on win32
       has the 'vcrun2019_x64' counterpart. The architecture-
       transitivity rule below uses this mapping. -->

### DLL Override family membership

DLL Name belongs to DLL Override Family.
  Each DLL Name belongs to at most one DLL Override Family.
  <!-- The set-membership surface that replaces regex in derivation
       antecedents. New DLLs are admitted to a family by an instance
       fact (see Instance Facts below), so the check-readings layer
       can validate the member set without parsing regex literals. -->

### .NET Framework

Wine App is .NET Framework version Dotnet Framework Version.
  Each Wine App is at most one .NET Framework version.
  <!-- Authors with apps that ship a known .NET Framework target
       (Office add-ins, LINQPad, paint.net) declare the version
       directly; the derivation rule fans it out into the matching
       'dotnet<N>' Required Component automatically. -->

### Notices

Notice has Notice Text.
  Each Notice has exactly one Notice Text.

Wine App requires Notice.
  <!-- Many-per-app: the borked-rating rule and the Electron-quirk
       rule both contribute to the same app's notice list. -->

### Hardware compatibility

GPU Vendor has display- Title.
  Each GPU Vendor has exactly one display- Title.

GPU Vendor has minimum GPU Driver Version.
  Each GPU Vendor has at most one minimum GPU Driver Version.
  <!-- The driver floor below which the vendor's stack lacks the
       Vulkan ≥ 1.3 features the chained DXVK builds expect. -->

Wine App requires GPU Vendor.
  <!-- Derived from the Vulkan-version escalation rule. Free
       multiplicity: a single app can be compatible with several
       vendors so the host-probe can pick whichever is present. -->

## Constraints

Each Wine App has exactly one Compat Rating.
Each Wine App has exactly one Prefix Architecture.

No two Wine Apps share the same Name.
No two DXVK Versions share the same Name.
No two vkd3d-proton Versions share the same Name.

Each Wine App, DLL Name combination occurs at most once in the
  population of Wine App requires DLL Override.
Each Wine App, Registry Path combination occurs at most once in the
  population of Wine App requires Registry Key.
Each Wine App, Env Var Name combination occurs at most once in the
  population of Wine App requires Environment Variable.

## Deontic Constraints

It is obligatory that each Wine App has some Compat Rating.
It is obligatory that each Wine App has some Prefix Architecture.

## Derivation Rules

+ Wine App requires Vulkan Version (V)
    if Wine App requires DXVK Version (X)
    and DXVK Version (X) requires Vulkan Version (V).

+ Wine App requires Vulkan Version (V)
    if Wine App requires vkd3d-proton Version (X)
    and vkd3d-proton Version (X) requires Vulkan Version (V).

+ Wine App requires Required Component 'msxml6'
    if Wine App has Prefix Architecture 'win32'
    and Wine App requires DLL Override of DLL Name 'msxml6.dll'
    with DLL Behavior 'native'.
  <!-- Common winetricks expansion: declaring `msxml6.dll = native`
       without first installing the redistributable produces a
       silent load failure. The expansion folds the install step in
       automatically so the user never has to remember it. -->

+ Wine App requires Required Component 'corefonts'
    if Wine App requires DLL Override of DLL Name 'riched20.dll'
    with DLL Behavior 'native'.
  <!-- Office-family apps that pin riched20.dll to native almost
       always also need the Microsoft core fonts to render correctly;
       this expansion is a noisy default but documented in
       Office's WineHQ AppDB entry. -->

### MSVC / MFC runtime expansion

+ Wine App requires Required Component 'vcrun2013'
    if Wine App requires DLL Override of DLL Name 'msvcr120.dll'
    with DLL Behavior 'native'.
  <!-- Visual C++ 2013 runtime. msvcr120 is the canonical marker; the
       app authors who declare it almost universally forget to also
       list `vcrun2013` because winetricks installs the redistributable
       silently in their normal workflow. -->

+ Wine App requires Required Component 'vcrun2019'
    if Wine App requires DLL Override of DLL Name 'msvcr140.dll'
    with DLL Behavior 'native'.
  <!-- Visual C++ 2015-2022 share msvcr140 (the runtime is API-stable
       across that whole window); the winetricks recipe is named for
       2019 because that was the active version when the verb landed. -->

+ Wine App requires Required Component 'vcrun2019'
    if Wine App requires DLL Override of DLL Name 'mfc140.dll'
    with DLL Behavior 'native'.
  <!-- MFC 14.0 ships with the same VC++ 14 runtime as msvcr140; one
       expansion handles both DLL pins. -->

### .NET Framework inference

+ Wine App requires Required Component 'dotnet48'
    if Wine App requires DLL Override of DLL Name 'mscoree.dll'
    with DLL Behavior 'native'.
  <!-- mscoree is the .NET CLR shim. Pinning it to native without
       installing a .NET Framework runtime produces the classic
       'CLR not found' silent crash. dotnet48 is the safest default —
       the latest 4.x is binary-compatible with everything from 4.0
       up. Apps with a tighter floor declare `is .NET Framework
       version` explicitly and the rule below picks the matching
       recipe instead. -->

+ Wine App requires Required Component 'dotnet48'
    if Wine App is .NET Framework version '4.8'.

+ Wine App requires Required Component 'dotnet472'
    if Wine App is .NET Framework version '4.7'.

+ Wine App requires Required Component 'dotnet462'
    if Wine App is .NET Framework version '4.6'.

+ Wine App requires Required Component 'dotnet45'
    if Wine App is .NET Framework version '4.5'.

+ Wine App requires Required Component 'dotnet40'
    if Wine App is .NET Framework version '4.0'.
  <!-- Five small rules rather than one parameterised rule because
       FORML 2 derivation antecedents do not admit string concatenation
       on the consequent side — the recipe slug ('dotnet48' vs
       'dotnet472') is not a clean function of the version literal.
       Each version maps to its own fixed winetricks verb and the
       enumeration is closed at five values for this slice. -->

### DLL family expansion

+ Wine App requires DXVK Version 'dxvk-2.3'
    if Wine App requires DLL Override of DLL Name (D)
       with DLL Behavior 'native'
    and DLL Name (D) belongs to DLL Override Family 'd3d-compiler'.
  <!-- Electron / Chromium apps fan out d3dcompiler_47 (and the
       handful of older d3dcompiler_43/46 variants) to native; the
       canvas / WebGL paths then need DXVK underneath them or the
       app silently loses GPU acceleration. dxvk-2.3 is the latest-
       stable pin per the DXVK Version anchor instances below. -->

+ Wine App requires DXVK Version 'dxvk-2.3'
    if Wine App requires DLL Override of DLL Name (D)
       with DLL Behavior 'native'
    and DLL Name (D) belongs to DLL Override Family 'directx-2d'.

+ Wine App requires DXVK Version 'dxvk-2.3'
    if Wine App requires DLL Override of DLL Name (D)
       with DLL Behavior 'native'
    and DLL Name (D) belongs to DLL Override Family 'directx-text'.

+ Wine App requires DXVK Version 'dxvk-2.3'
    if Wine App requires DLL Override of DLL Name (D)
       with DLL Behavior 'native'
    and DLL Name (D) belongs to DLL Override Family 'directx-gi'.
  <!-- d2d1 / dwrite / dxgi cover the Direct2D, DirectWrite and DXGI
       paths respectively. All three require a Vulkan-backed
       implementation when running under Wine; pinning any of them to
       native with no DXVK underneath drops to the GDI / OSMesa
       slowpath, so the chainer auto-selects DXVK. -->

+ Wine App requires Required Component 'msxml3'
    if Wine App requires DLL Override of DLL Name 'msxml3.dll'
    with DLL Behavior 'native'.

+ Wine App requires Required Component 'msxml4'
    if Wine App requires DLL Override of DLL Name 'msxml4.dll'
    with DLL Behavior 'native'.
  <!-- msxml<N> family — winetricks ships recipes for N in {3, 4, 6};
       msxml6 already has the explicit win32-prefix rule above (it
       fires only on win32 because msxml6 ships natively in win64
       prefixes). msxml3 / msxml4 fire on every architecture. The
       three rules are spelled separately rather than via a parametric
       join because FORML 2 derivation rules cannot synthesise the
       recipe name from the DLL name — each (DLL, recipe) pair is its
       own enumerated mapping. -->
  <!-- (msxml6 expansion above covers the third member.) -->

### Architecture transitivity

+ Wine App requires Required Component (R64)
    if Wine App has Prefix Architecture 'win64'
    and Wine App requires Required Component (R)
    and Required Component Anchor has Recipe (R)
    and Required Component Anchor has win64- Recipe (R64).
  <!-- VC++ runtimes (and a handful of others) need the explicit
       `_x64` verb on win64 prefixes; the join through the anchor's
       win64- Recipe role yields the variant. If no win64- Recipe is
       declared on the anchor the rule contributes nothing — single
       recipe covers both arches in that case. -->

### Vulkan -> GPU vendor escalation

+ Wine App requires GPU Vendor (V)
    if Wine App requires Vulkan Version '1.3'
    and GPU Vendor (V) has minimum GPU Driver Version (D).
  <!-- Hardware-compat escalation: any Vulkan-1.3-requiring app
       inherits a hardware floor from the GPU Vendor anchors below.
       The runtime layer (#462c) walks the resulting set and refuses
       to launch on hosts whose probed driver falls below any
       vendor's minimum. The Vulkan version itself was already lifted
       transitively from DXVK / vkd3d-proton via the rules above, so
       declaring DXVK 2.3 on an app implicitly produces the GPU
       requirement. -->

### Compat-rating notices

+ Wine App requires Notice 'unsupported-current-wine'
    if Wine App has Compat Rating 'borked'.
  <!-- Borked apps are the ones the runtime should refuse to launch
       (or launch with a loud warning). Routing the rating into a
       Notice surface lets the CLI render the same human-readable
       string regardless of which path produced it (rating, app-
       specific override, or a future quirk-detector). -->

+ Wine App requires Notice 'electron-gpu-process-quirks'
    if Wine App has Compat Rating 'silver'
    and Wine App requires DLL Override of DLL Name (D)
       with DLL Behavior 'native'
    and DLL Name (D) belongs to DLL Override Family 'd3d-compiler'.
  <!-- Heuristic for Chromium-based apps (Electron, CEF). Silver +
       d3dcompiler family pin is the canonical Electron-on-Wine
       fingerprint; emit the standard advisory so users know to
       expect occasional GPU-process restarts. -->

## Instance Facts

Domain 'compat' has Description 'Wine application compatibility — per-app DLL overrides, registry tweaks, environment variables, winetricks recipes, and ProtonDB-aligned compat ratings expressed as FORML facts. Substrate for `arest run "App Name"` and the ProtonDB ingest pipeline.'.

### DXVK / vkd3d-proton baseline versions

DXVK Version 'dxvk-2.3' has Name '2.3'.
DXVK Version 'dxvk-2.3' requires Vulkan Version '1.3'.

DXVK Version 'dxvk-2.0' has Name '2.0'.
DXVK Version 'dxvk-2.0' requires Vulkan Version '1.3'.

DXVK Version 'dxvk-1.10' has Name '1.10'.
DXVK Version 'dxvk-1.10' requires Vulkan Version '1.1'.

vkd3d-proton Version 'vkd3d-proton-2.12' has Name '2.12'.
vkd3d-proton Version 'vkd3d-proton-2.12' requires Vulkan Version '1.3'.

### Wine App: Notepad++

Wine App 'notepad-plus-plus' has display- Title 'Notepad++'.
Wine App 'notepad-plus-plus' has Description 'Lightweight text editor; one of the cleanest Wine targets — no DirectX, no .NET, just plain Win32. Platinum on every Wine version since 5.0.'.
Wine App 'notepad-plus-plus' has Compat Rating 'gold'.
Wine App 'notepad-plus-plus' has Prefix Architecture 'win32'.
Wine App 'notepad-plus-plus' requires Wine Version '8.0'.

### Wine App: Microsoft Office 2016 (Word)

Wine App 'office-2016-word' has display- Title 'Microsoft Word 2016'.
Wine App 'office-2016-word' has Description 'Office 2016 Word component. Gold-rated with corefonts + gdiplus + riched20 native. Activation requires a manual product key step outside the prefix.'.
Wine App 'office-2016-word' has Compat Rating 'gold'.
Wine App 'office-2016-word' has Prefix Architecture 'win64'.
Wine App 'office-2016-word' requires Wine Version '8.0-staging'.
Wine App 'office-2016-word' requires Required Component 'corefonts'.
Wine App 'office-2016-word' requires Required Component 'gdiplus'.
Wine App 'office-2016-word' requires DLL Override of DLL Name 'riched20.dll' with DLL Behavior 'native'.

### Wine App: Adobe Photoshop CS6

Wine App 'photoshop-cs6' has display- Title 'Adobe Photoshop CS6'.
Wine App 'photoshop-cs6' has Description 'Last perpetual-license Photoshop. Gold on Wine 8.0+; needs the VC++ 2013 runtime (msvcr120.dll = native) and DXVK for the canvas-rendering acceleration paths.'.
Wine App 'photoshop-cs6' has Compat Rating 'gold'.
Wine App 'photoshop-cs6' has Prefix Architecture 'win64'.
Wine App 'photoshop-cs6' requires Wine Version '8.0-staging'.
Wine App 'photoshop-cs6' requires DXVK Version 'dxvk-2.3'.
Wine App 'photoshop-cs6' requires Required Component 'vcrun2013'.
Wine App 'photoshop-cs6' requires DLL Override of DLL Name 'msvcr120.dll' with DLL Behavior 'native'.

### Wine App: AutoHotkey 1.x

Wine App 'autohotkey-v1' has display- Title 'AutoHotkey 1.x'.
Wine App 'autohotkey-v1' has Description 'Pure Win32 scripting host. Platinum on every Wine version; no special config needed beyond a stock win32 prefix.'.
Wine App 'autohotkey-v1' has Compat Rating 'platinum'.
Wine App 'autohotkey-v1' has Prefix Architecture 'win32'.
Wine App 'autohotkey-v1' requires Wine Version '8.0'.

### Wine App: Notion (Windows desktop)

Wine App 'notion-desktop' has display- Title 'Notion'.
Wine App 'notion-desktop' has Description 'Electron-based collaboration app. Silver on Wine 8.0+ — runs but with the canonical Electron-on-Wine quirks (occasional GPU-process crashes, font fallback issues). DXVK is the difference between usable and unusable.'.
Wine App 'notion-desktop' has Compat Rating 'silver'.
Wine App 'notion-desktop' has Prefix Architecture 'win64'.
Wine App 'notion-desktop' requires Wine Version '9.0'.
Wine App 'notion-desktop' requires DXVK Version 'dxvk-2.3'.
Wine App 'notion-desktop' requires Environment Variable with Env Var Name 'WINEDLLOVERRIDES' and Env Var Value 'libglesv2=b'.
Wine App 'notion-desktop' requires Environment Variable with Env Var Name 'DXVK_HUD' and Env Var Value '0'.

### Wine App: Total Commander

Wine App 'total-commander' has display- Title 'Total Commander'.
Wine App 'total-commander' has Description 'Veteran two-pane file manager. Gold on every Wine version — runs as a single Win32 binary with no library deps.'.
Wine App 'total-commander' has Compat Rating 'gold'.
Wine App 'total-commander' has Prefix Architecture 'win32'.
Wine App 'total-commander' requires Wine Version '8.0'.

### Wine App: Visual Studio Code

Wine App 'vscode' has display- Title 'Visual Studio Code'.
Wine App 'vscode' has Description 'Electron-based editor. Gold on Wine 9.0+ with DXVK; common Electron pattern of disabling libglesv2 to force the software canvas path. Native Linux build is preferred where available; this entry exists for users on locked-down hosts that ship only the Windows installer.'.
Wine App 'vscode' has Compat Rating 'gold'.
Wine App 'vscode' has Prefix Architecture 'win64'.
Wine App 'vscode' requires Wine Version '9.0'.
Wine App 'vscode' requires DXVK Version 'dxvk-2.3'.
Wine App 'vscode' requires Environment Variable with Env Var Name 'WINEDLLOVERRIDES' and Env Var Value 'libglesv2=b'.

### Wine App: Spotify

Wine App 'spotify' has display- Title 'Spotify'.
Wine App 'spotify' has Description 'Streaming-music client (CEF/Chromium under the hood). Silver — playback works; Connect-to-device handoff is flaky. Disable the in-app crash reporter (it loops).'.
Wine App 'spotify' has Compat Rating 'silver'.
Wine App 'spotify' has Prefix Architecture 'win64'.
Wine App 'spotify' requires Wine Version '9.0'.
Wine App 'spotify' requires DXVK Version 'dxvk-2.0'.
Wine App 'spotify' requires Environment Variable with Env Var Name 'WINEDLLOVERRIDES' and Env Var Value 'libglesv2=b'.
Wine App 'spotify' requires Registry Key at Registry Path 'HKCU\\Software\\Spotify\\CrashReporter' with Registry Value 'disabled'.

### Wine App: Steam (Windows client — ironic)

Wine App 'steam-windows' has display- Title 'Steam (Windows client)'.
Wine App 'steam-windows' has Description 'The Windows Steam client running on Wine on Linux — useful for accessing the Workshop or family library features that have no Linux-native parity. Silver on Wine 9.0+; needs a sizeable override set including dwrite + msxml + the VC++ runtimes.'.
Wine App 'steam-windows' has Compat Rating 'silver'.
Wine App 'steam-windows' has Prefix Architecture 'win64'.
Wine App 'steam-windows' requires Wine Version '9.0-staging'.
Wine App 'steam-windows' requires Required Component 'vcrun2019'.
Wine App 'steam-windows' requires Required Component 'corefonts'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'dwrite.dll' with DLL Behavior 'disabled'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'msxml3.dll' with DLL Behavior 'native'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'msxml6.dll' with DLL Behavior 'native'.
Wine App 'steam-windows' requires Environment Variable with Env Var Name 'STEAM_RUNTIME' and Env Var Value '0'.

### Wine App: 7-Zip

Wine App '7-zip' has display- Title '7-Zip'.
Wine App '7-zip' has Description 'Cross-architecture archive utility. Platinum on every Wine version — runs as a stock Win32 binary, the GUI uses only common controls.'.
Wine App '7-zip' has Compat Rating 'platinum'.
Wine App '7-zip' has Prefix Architecture 'win32'.
Wine App '7-zip' requires Wine Version '8.0'.

### Required Component anchors

Required Component Anchor 'vcrun2013' has Recipe 'vcrun2013'.
Required Component Anchor 'vcrun2013' has Description 'Microsoft Visual C++ 2013 Redistributable (msvcr120 / msvcp120 / mfc120).'.
Required Component Anchor 'vcrun2013' has win64- Recipe 'vcrun2013_x64'.

Required Component Anchor 'vcrun2019' has Recipe 'vcrun2019'.
Required Component Anchor 'vcrun2019' has Description 'Microsoft Visual C++ 2015-2022 Redistributable (msvcr140 / msvcp140 / mfc140).'.
Required Component Anchor 'vcrun2019' has win64- Recipe 'vcrun2019_x64'.

Required Component Anchor 'dotnet48' has Recipe 'dotnet48'.
Required Component Anchor 'dotnet48' has Description 'Microsoft .NET Framework 4.8 — latest 4.x; binary-compatible back to 4.0.'.

Required Component Anchor 'dotnet472' has Recipe 'dotnet472'.
Required Component Anchor 'dotnet472' has Description 'Microsoft .NET Framework 4.7.2.'.

Required Component Anchor 'dotnet462' has Recipe 'dotnet462'.
Required Component Anchor 'dotnet462' has Description 'Microsoft .NET Framework 4.6.2.'.

Required Component Anchor 'dotnet45' has Recipe 'dotnet45'.
Required Component Anchor 'dotnet45' has Description 'Microsoft .NET Framework 4.5.'.

Required Component Anchor 'dotnet40' has Recipe 'dotnet40'.
Required Component Anchor 'dotnet40' has Description 'Microsoft .NET Framework 4.0.'.

Required Component Anchor 'corefonts' has Recipe 'corefonts'.
Required Component Anchor 'corefonts' has Description 'Microsoft TrueType core fonts (Arial, Times New Roman, Courier New, Verdana, Comic Sans MS, Trebuchet MS, Webdings, Andale Mono, Impact, Georgia).'.

Required Component Anchor 'gdiplus' has Recipe 'gdiplus'.
Required Component Anchor 'gdiplus' has Description 'Microsoft GDI+ redistributable; required by Office and most Photoshop-family apps for image rasterisation.'.

Required Component Anchor 'msxml3' has Recipe 'msxml3'.
Required Component Anchor 'msxml3' has Description 'Microsoft XML Core Services 3.0.'.

Required Component Anchor 'msxml4' has Recipe 'msxml4'.
Required Component Anchor 'msxml4' has Description 'Microsoft XML Core Services 4.0.'.

Required Component Anchor 'msxml6' has Recipe 'msxml6'.
Required Component Anchor 'msxml6' has Description 'Microsoft XML Core Services 6.0.'.

### DLL Override family memberships

DLL Name 'msvcr120.dll' belongs to DLL Override Family 'msvc-runtime'.
DLL Name 'msvcr140.dll' belongs to DLL Override Family 'msvc-runtime'.
DLL Name 'msvcp120.dll' belongs to DLL Override Family 'msvc-runtime'.
DLL Name 'msvcp140.dll' belongs to DLL Override Family 'msvc-runtime'.

DLL Name 'mfc120.dll' belongs to DLL Override Family 'mfc-runtime'.
DLL Name 'mfc140.dll' belongs to DLL Override Family 'mfc-runtime'.

DLL Name 'mscoree.dll' belongs to DLL Override Family 'dotnet-clr'.
DLL Name 'mscorlib.dll' belongs to DLL Override Family 'dotnet-clr'.

DLL Name 'msxml3.dll' belongs to DLL Override Family 'msxml'.
DLL Name 'msxml4.dll' belongs to DLL Override Family 'msxml'.
DLL Name 'msxml6.dll' belongs to DLL Override Family 'msxml'.

DLL Name 'd3dcompiler_43.dll' belongs to DLL Override Family 'd3d-compiler'.
DLL Name 'd3dcompiler_46.dll' belongs to DLL Override Family 'd3d-compiler'.
DLL Name 'd3dcompiler_47.dll' belongs to DLL Override Family 'd3d-compiler'.

DLL Name 'd2d1.dll' belongs to DLL Override Family 'directx-2d'.
DLL Name 'dwrite.dll' belongs to DLL Override Family 'directx-text'.
DLL Name 'dxgi.dll' belongs to DLL Override Family 'directx-gi'.

### GPU Vendor anchors

GPU Vendor 'nvidia' has display- Title 'NVIDIA GeForce / RTX'.
GPU Vendor 'nvidia' has minimum GPU Driver Version '525'.

GPU Vendor 'amd-rdna' has display- Title 'AMD Radeon (RDNA / RDNA2 / RDNA3)'.
GPU Vendor 'amd-rdna' has minimum GPU Driver Version 'rdna'.

GPU Vendor 'intel-xe' has display- Title 'Intel Arc / Xe'.
GPU Vendor 'intel-xe' has minimum GPU Driver Version 'xe'.

### Notice anchors

Notice 'unsupported-current-wine' has Notice Text 'This app is rated `borked` on the current Wine version; `arest run` will refuse to launch unless --force is passed.'.

Notice 'electron-gpu-process-quirks' has Notice Text 'Electron / Chromium app under Wine; expect occasional GPU-process crashes that the app auto-recovers from.'.

### Federated source: ProtonDB

External System 'protondb' has URL 'https://www.protondb.com/api/v1'.
External System 'protondb' has Kind 'rest'.
  <!-- The ingest pipeline (#462e) walks the per-app reports endpoint
       and folds each user's compat rating into the Wine App graph.
       Auth header is omitted — the public endpoints are unauth. -->
