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

### Federated source: ProtonDB

External System 'protondb' has URL 'https://www.protondb.com/api/v1'.
External System 'protondb' has Kind 'rest'.
  <!-- The ingest pipeline (#462e) walks the per-app reports endpoint
       and folds each user's compat rating into the Wine App graph.
       Auth header is omitted — the public endpoints are unauth. -->
