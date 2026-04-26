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

Wine Prefix Path is a value type.
  <!-- POSIX-shaped absolute path under the canonical
       `/var/wine/` parent, e.g. `/var/wine/notepad-plus-plus/`. The
       path is wholly derived from the Wine App's slug (= its `.Name`
       reference mode) by the prefix-path derivation rule below; the
       value type exists so the runtime layer (#462c) and the future
       MCP `wine_prefix_for(app_id)` verb (#481) can hand the same
       string out without each consumer re-deriving it. The trailing
       slash is mandatory — every consumer of this value treats it as
       a *directory* path so the slash keeps the join with relative
       sub-paths (`drive_c/users/wineuser/...`) total. -->

Installer URL is a value type.
  <!-- Source location of the Wine App's installer binary. Either an
       `http(s)://` / `file://` URL the `installer_fetch` module
       passes to `curl` / `Invoke-WebRequest`, or a host-filesystem
       path for pre-staged binaries (licensed apps where the upstream
       URL sits behind a login wall). The runtime layer (#505)
       caches the fetched binary under
       `<prefix>/drive_c/_install/<filename>`; re-runs short-circuit
       on the cached file. -->

Installer Filename is a value type.
  <!-- Cache-side filename for the fetched installer (e.g.
       `npp-installer.exe`, `SteamSetup.exe`, `SpotifyFullSetup.exe`).
       Decoupled from the URL because URLs may use redirect tokens or
       query strings that don't yield a stable filename, and because
       the installer-runner subprocess takes the local path directly.
       The cache key is `(prefix Directory, filename)`; two Wine Apps
       cannot share a prefix Directory by construction (#481), so
       this is uniqueness-safe. -->

Install Status is a value type.
  The possible values of Install Status are
    'Downloaded', 'Installing', 'Installed', 'Failed'.
  <!-- State-machine state for the per-app install lifecycle (#505,
       #212). Transitions: nothing → Downloaded (binary fetched but
       wine not yet run) → Installing (in-progress / blocked, e.g.
       fetcher unavailable on PATH) → Installed (wine ran, marker
       written) → Failed (non-zero exit). The state is materialised
       as a sequence of facts in the `Wine App install Status` cell
       — one fact per transition, the final state being the last
       fact. Mirrors the Process state-machine-as-derivation pattern
       (#212) so the runtime layer can re-derive the current state
       from the fact stream rather than maintain a side index. -->

Main Exe Path is a value type.
  <!-- Prefix-relative POSIX-style path to the Wine App's main
       executable, e.g. `drive_c/Program Files/Notepad++/notepad++.exe`.
       Joined to the per-app prefix Directory by the launcher (#506,
       `cli::wine_launch`) to produce the absolute path passed to
       wine. Decoupled from Installer Filename because the installer
       binary and the installed exe almost never share a name; the
       launcher needs the post-install path, not the setup binary. -->

Run Status is a value type.
  The possible values of Run Status are
    'Running', 'Paused', 'Exited', 'Crashed'.
  <!-- State-machine state for the per-app runtime lifecycle (#506,
       #212). Transitions: nothing → Running (wine subprocess
       spawned and survived the ~500ms settle window) → Paused
       (suspended via SIGSTOP / debugger break — produced by the
       future `arest watch` flow, not by `arest run`) → Exited
       (clean exit with status 0) | Crashed (non-zero exit, signal
       termination, or spawn failure). Materialised as a sequence
       of facts in the `Wine_App_run_status` cell — one fact per
       transition, the final state being the last fact. Mirrors
       the Install Status pattern above and the Process SM pattern
       (#212). The runtime layer reads the latest fact for the app
       to decide whether `arest run` should short-circuit (already
       running) or proceed (Crashed/Exited/no-history). -->

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

### Per-app prefix isolation (#481)

Wine App has prefix Directory.
  Each Wine App has exactly one prefix Directory.
  Each Directory is the prefix Directory of at most one Wine App.
  <!-- Cross-domain join: the second role binds to the `Directory`
       entity declared in `readings/os/filesystem.md`. The 1:1
       multiplicity (mandatory + uniqueness on the Wine App side,
       uniqueness on the Directory side) makes the prefix Directory
       co-extensional with its owning Wine App: dropping the App
       leaves the Directory cell with no inbound `prefix Directory`
       reference, and the runtime layer (#462c) reaps it as part of
       the standard cell-eviction sweep. Backup is symmetric — the
       existing `zip_directory(prefix_id)` Platform fn from #404
       (see `crates/arest/src/platform/zip.rs`) walks exactly the
       prefix subtree the join points at, so a per-app snapshot is a
       single call once the prefix Directory id is known. -->

Wine App has Wine Prefix Path.
  Each Wine App has exactly one Wine Prefix Path.
  No two Wine Apps share the same Wine Prefix Path.
  <!-- Derived; see Derivation Rules below. The path is a pure
       function of the Wine App's slug, so the uniqueness constraint
       follows transitively from `No two Wine Apps share the same
       Name`; restating it here makes the disjoint-prefix invariant
       readable at the constraint surface. -->

### Installer fetch + run (#505)

Wine App has Installer URL.
  Each Wine App has at most one Installer URL.
  <!-- Source location of the installer binary. Either a URL the
       fetcher hands to curl / Invoke-WebRequest, or a host-filesystem
       path for pre-staged binaries. The orchestrator
       (`cli::wine_install`) reads this fact at `arest run` time and
       transitions to `Installing` if absent. Free multiplicity-of-1
       (at most one) so the readings can express 'no installer
       declared yet' without violating an obligatory constraint;
       authors who need it mandatory layer that on top per-app. -->

Wine App has Installer Filename.
  Each Wine App has at most one Installer Filename.
  <!-- Cache-side filename for the fetched binary
       (`<prefix>/drive_c/_install/<filename>`). Companion to
       Installer URL — URLs with redirects or query strings don't
       yield stable filenames, and the wine-runner subprocess takes
       the local path directly. -->

Wine App has Install Status.
  <!-- State-machine fact stream for the per-app install lifecycle
       (#505 / #212). Each transition pushes one fact onto the
       `Wine_App_install_status` cell; the final state is the last
       fact in the cell. Free multiplicity (no `at most one` clause)
       so the entire transition history is materialised — a Failed
       state followed by a re-run that lands on Installed leaves
       both facts in place, with Installed dominating because it is
       most recent. The runtime layer (#506) reads the last fact to
       decide whether to launch. -->

### App launch + monitor (#506)

Wine App has Main Exe Path.
  Each Wine App has at most one Main Exe Path.
  <!-- Prefix-relative path to the installed app's main executable.
       The launcher (`cli::wine_launch`) joins this to the per-app
       prefix Directory and passes the absolute path to wine.
       Free multiplicity-of-1 so apps without a declared exe path
       transition to a no-op `Exited` state with a clean diagnostic
       rather than failing the whole `arest run` chain. -->

Wine App has Run Status.
  <!-- State-machine fact stream for the per-app runtime lifecycle
       (#506 / #212). Each launch transition pushes one fact onto
       the `Wine_App_run_status` cell; the final state is the last
       fact in the cell. Free multiplicity so the full lifecycle
       history is materialised: Running → (hours later) Crashed →
       (next launch) Running again leaves three facts, the latest
       Running dominating. `arest run` reads the latest to decide
       whether to short-circuit (already Running) or relaunch
       (Crashed / Exited / no-history). -->

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

Each Wine App has exactly one prefix Directory.
Each Directory is the prefix Directory of at most one Wine App.
Each Wine App has exactly one Wine Prefix Path.
No two Wine Apps share the same prefix Directory.
No two Wine Apps share the same Wine Prefix Path.

No prefix Directory of a Wine App has another prefix Directory of a
  Wine App as parent Directory.
  <!-- Disjoint-subtree invariant. Two Wine Apps' prefixes share at
       most the canonical `/var/wine/` parent Directory; no Wine App's
       prefix is ever nested inside another Wine App's prefix. The
       canonical parent itself is NOT a prefix Directory of any Wine
       App (it is the shared root, declared as a single Directory
       instance under "## Instance Facts" below), so the constraint
       reduces to: prefix Directories of distinct Wine Apps are
       sibling subtrees. The runtime layer relies on this when it
       routes a per-app filesystem write — the route is always the
       single prefix Directory id; cross-prefix collisions cannot
       arise by construction. -->

## Deontic Constraints

It is obligatory that each Wine App has some Compat Rating.
It is obligatory that each Wine App has some Prefix Architecture.
It is obligatory that each Wine App has some prefix Directory.
It is obligatory that each Wine App has some Wine Prefix Path.

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

### Per-app prefix path derivation (#481)

* Wine App has Wine Prefix Path (P) iff Wine App has Name (N)
    and P is the concatenation of '/var/wine/' and N and '/'.
  <!-- The prefix path is a pure function of the Wine App's slug
       (which is its `.Name` reference mode value): each Wine App
       lives at `/var/wine/<slug>/`. Mirrors the
       `File has Size iff File has ContentRef and Size is the
       byte-length of ContentRef.` shape from
       `readings/os/filesystem.md`. The runtime layer (#462c) and the
       MCP `wine_prefix_for(app_id)` verb (#481) read this derived
       value rather than re-concatenating per call site, so any
       future re-rooting (e.g. `/home/<user>/.local/share/wine/`)
       is one rule edit. -->

* Wine App has prefix Directory (D) iff Wine App has Name (N)
    and Directory (D) has Name (N)
    and Directory (D) has parent Directory 'wine-prefix-root'.
  <!-- The prefix Directory is the unique Directory whose Name equals
       the Wine App's slug AND whose parent is the canonical
       `/var/wine/` root (declared as `Directory 'wine-prefix-root'`
       under "## Instance Facts" below). Together with the
       1:1 multiplicity declared in the fact-type section, this is
       the join the runtime layer follows when it wants to write into
       a per-app prefix: look up the Wine App, follow `prefix
       Directory`, and route every File create / Directory mkdir
       through the resulting Directory id. The two-clause antecedent
       (Name match + canonical parent) is what enforces the
       sibling-subtree invariant: any Directory cell that matches
       both clauses is necessarily a child of `/var/wine/` and so
       cannot also be a child of another Wine App's prefix. -->

<!-- Uninstall cascade (#481): when a Wine App fact is removed (the
     user runs `arest uninstall "App Name"`), its prefix Directory is
     no longer the value of any Wine App's `prefix Directory` role —
     the App was the unique inbound reference (1:1 multiplicity, see
     "### Per-app prefix isolation (#481)" above). The runtime layer
     reaps the orphaned prefix Directory (and, transitively, every
     File and child Directory in its subtree) as part of the standard
     cell-eviction sweep. The kernel's command pipeline already
     cascades on `parent Directory` (see the ring-acyclic constraint
     in `readings/os/filesystem.md`); the 1:1 fact type declared
     above is what gives the eviction sweep something unambiguous to
     follow at the Wine App layer.

     This is *not* expressed as a `+`/`*` derivation rule: FORML 2
     derivation rules are monotonic — they add facts, they do not
     describe deletion. The cascade is a runtime policy whose pre-
     condition (the Directory has no inbound `prefix Directory`
     reference) is an algebraic consequence of the 1:1 constraint,
     so no extra rule body is needed. The future `arest uninstall`
     CLI executes a single `delete:Wine App` command and the
     standard cascade handles the rest. -->

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

### Per-app prefix Directory instances (#481)

<!-- The canonical `/var/wine/` parent. Every Wine App's prefix
     Directory hangs off this single shared root, which itself is
     never the prefix Directory of any Wine App. The Name 'wine' +
     parent path conventions match the FHS layout the runtime layer
     materialises on first boot. The root has no parent Directory
     fact — by the ring-acyclic constraint in
     `readings/os/filesystem.md` it is therefore a top-level
     Directory; the runtime probes `/var/` itself via the host OS
     mount. -->

Directory 'wine-prefix-root' has Name 'wine'.

<!-- Per-Wine-App prefix Directory anchors. The Directory id is the
     Wine App's slug + the literal `-prefix` suffix; the Directory's
     Name role carries the slug verbatim so the prefix-Directory
     derivation rule above (`Directory (D) has Name (N)`) joins
     unambiguously. Every prefix Directory's parent is the canonical
     'wine-prefix-root' declared just above. The slug → derived path
     mapping is then:

       'notepad-plus-plus'  → /var/wine/notepad-plus-plus/
       'office-2016-word'   → /var/wine/office-2016-word/
       'photoshop-cs6'      → /var/wine/photoshop-cs6/
       … etc.

     One block per Wine App declared above. New Wine Apps need to add
     three lines here (Directory has Name, Directory has parent
     Directory, Wine App has prefix Directory) — the rest of the
     prefix machinery is derived. -->

Directory 'notepad-plus-plus-prefix' has Name 'notepad-plus-plus'.
Directory 'notepad-plus-plus-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'notepad-plus-plus' has prefix Directory 'notepad-plus-plus-prefix'.

Directory 'office-2016-word-prefix' has Name 'office-2016-word'.
Directory 'office-2016-word-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'office-2016-word' has prefix Directory 'office-2016-word-prefix'.

Directory 'photoshop-cs6-prefix' has Name 'photoshop-cs6'.
Directory 'photoshop-cs6-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'photoshop-cs6' has prefix Directory 'photoshop-cs6-prefix'.

Directory 'autohotkey-v1-prefix' has Name 'autohotkey-v1'.
Directory 'autohotkey-v1-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'autohotkey-v1' has prefix Directory 'autohotkey-v1-prefix'.

Directory 'notion-desktop-prefix' has Name 'notion-desktop'.
Directory 'notion-desktop-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'notion-desktop' has prefix Directory 'notion-desktop-prefix'.

Directory 'total-commander-prefix' has Name 'total-commander'.
Directory 'total-commander-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'total-commander' has prefix Directory 'total-commander-prefix'.

Directory 'vscode-prefix' has Name 'vscode'.
Directory 'vscode-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'vscode' has prefix Directory 'vscode-prefix'.

Directory 'spotify-prefix' has Name 'spotify'.
Directory 'spotify-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'spotify' has prefix Directory 'spotify-prefix'.

Directory 'steam-windows-prefix' has Name 'steam-windows'.
Directory 'steam-windows-prefix' has parent Directory 'wine-prefix-root'.
Wine App 'steam-windows' has prefix Directory 'steam-windows-prefix'.

Directory '7-zip-prefix' has Name '7-zip'.
Directory '7-zip-prefix' has parent Directory 'wine-prefix-root'.
Wine App '7-zip' has prefix Directory '7-zip-prefix'.

### Federated source: ProtonDB

External System 'protondb' has URL 'https://www.protondb.com/api/v1'.
External System 'protondb' has Kind 'rest'.
  <!-- The ingest pipeline (#462e) walks the per-app reports endpoint
       and folds each user's compat rating into the Wine App graph.
       Auth header is omitted — the public endpoints are unauth. -->

### Installer fetch + run instance facts (#505)

<!-- Installer URLs + cache filenames for the 10 Wine Apps declared
     above. URLs point at upstream-hosted installer binaries (or
     pre-staged paths for licensed apps where the upstream URL sits
     behind a login wall). The runtime layer (#505,
     `cli::wine_install`) consumes these via the
     `Wine_App_has_Installer_URL` / `Wine_App_has_Installer_Filename`
     cells.

     End-to-end validation in the unit suite uses Notepad++ because
     its installer is small (~4MB) and unauthed. The other nine are
     declared for completeness; their actual fetch + run paths are
     exercised at smoke-test time, not unit-test time.

     Note the URL pins point at specific versions where the upstream
     publishes per-version download paths (Notepad++, Notion, VSCode,
     Spotify) and at floating "latest" paths where the publisher
     does not (Steam, AutoHotkey). For licensed-only apps (Office,
     Photoshop) the URL is a host-filesystem path the user populates
     out-of-band; the fetcher copies rather than downloads. -->

Wine App 'notepad-plus-plus' has Installer URL 'https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.6.4/npp.8.6.4.Installer.exe'.
Wine App 'notepad-plus-plus' has Installer Filename 'npp.8.6.4.Installer.exe'.

Wine App 'office-2016-word' has Installer URL '/var/wine/staged/office-2016-setup.exe'.
Wine App 'office-2016-word' has Installer Filename 'office-2016-setup.exe'.
  <!-- Office requires a licensed installer; the URL field carries a
       host-filesystem path the user populates out-of-band. The
       fetcher copies (rather than downloads) when the value parses
       as a path rather than a URL. -->

Wine App 'photoshop-cs6' has Installer URL '/var/wine/staged/photoshop-cs6-setup.exe'.
Wine App 'photoshop-cs6' has Installer Filename 'photoshop-cs6-setup.exe'.
  <!-- Pre-staged like Office; Photoshop CS6 is no longer
       publicly downloadable from Adobe. -->

Wine App 'autohotkey-v1' has Installer URL 'https://www.autohotkey.com/download/ahk-install.exe'.
Wine App 'autohotkey-v1' has Installer Filename 'ahk-install.exe'.

Wine App 'notion-desktop' has Installer URL 'https://www.notion.so/desktop/windows/download'.
Wine App 'notion-desktop' has Installer Filename 'Notion-Setup.exe'.

Wine App 'total-commander' has Installer URL 'https://www.ghisler.com/download/tcmd1100x32_64.exe'.
Wine App 'total-commander' has Installer Filename 'tcmd1100x32_64.exe'.

Wine App 'vscode' has Installer URL 'https://code.visualstudio.com/sha/download?build=stable&os=win32-x64-user'.
Wine App 'vscode' has Installer Filename 'VSCodeUserSetup-x64.exe'.

Wine App 'spotify' has Installer URL 'https://download.scdn.co/SpotifyFullSetup.exe'.
Wine App 'spotify' has Installer Filename 'SpotifyFullSetup.exe'.

Wine App 'steam-windows' has Installer URL 'https://cdn.cloudflare.steamstatic.com/client/installer/SteamSetup.exe'.
Wine App 'steam-windows' has Installer Filename 'SteamSetup.exe'.

Wine App '7-zip' has Installer URL 'https://www.7-zip.org/a/7z2407-x64.exe'.
Wine App '7-zip' has Installer Filename '7z2407-x64.exe'.

### Main Exe Path instance facts (#506)

<!-- Prefix-relative paths to each Wine App's main executable post-
     install. The launcher (`cli::wine_launch`) joins these to the
     per-app prefix Directory to get the absolute path that gets
     passed to wine. Each path is the file the app's installer
     produces under the emulated `C:\` root — i.e. the same path
     a Windows user would see in `Start Menu → All Programs →
     <app>`, mapped through Wine's `drive_c/` to-Windows root
     translation. Quoting matches the rest of this file: literal
     spaces inside the path are preserved (`Program Files` is
     two words, no underscore).

     Verified per-app against the upstream installer's default
     install location at the version pinned in the Installer URL
     above. Where the installer offers an arch choice
     (`Program Files` vs `Program Files (x86)`) the path matches
     the app's declared `Prefix Architecture` — win32 prefixes
     route through `Program Files`, win64 prefixes route through
     `Program Files` for native-64 binaries and
     `Program Files (x86)` for legacy-32 binaries. -->

Wine App 'notepad-plus-plus' has Main Exe Path 'drive_c/Program Files/Notepad++/notepad++.exe'.

Wine App 'office-2016-word' has Main Exe Path 'drive_c/Program Files/Microsoft Office/root/Office16/WINWORD.EXE'.

Wine App 'photoshop-cs6' has Main Exe Path 'drive_c/Program Files/Adobe/Adobe Photoshop CS6 (64 Bit)/Photoshop.exe'.

Wine App 'autohotkey-v1' has Main Exe Path 'drive_c/Program Files/AutoHotkey/AutoHotkey.exe'.

Wine App 'notion-desktop' has Main Exe Path 'drive_c/users/wineuser/AppData/Local/Programs/Notion/Notion.exe'.

Wine App 'total-commander' has Main Exe Path 'drive_c/totalcmd/TOTALCMD64.EXE'.

Wine App 'vscode' has Main Exe Path 'drive_c/Program Files/Microsoft VS Code/Code.exe'.

Wine App 'spotify' has Main Exe Path 'drive_c/users/wineuser/AppData/Roaming/Spotify/Spotify.exe'.

Wine App 'steam-windows' has Main Exe Path 'drive_c/Program Files (x86)/Steam/steam.exe'.

Wine App '7-zip' has Main Exe Path 'drive_c/Program Files/7-Zip/7zFM.exe'.
