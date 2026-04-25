# AREST UI: Design System (Modern Minimal)

This reading defines the modern-minimal design system as FORML 2 fact
types and instances. It is the single source of truth consumed by both
the Slint kernel UI surface and the ui.do React frontend, so a token
change here propagates uniformly to every rendering target.

The system is restrained on purpose: one neutral 5-shade scale, one
accent, four semantic colors, two type families (Inter sans, JetBrains
Mono mono), a strict 4px spacing grid, and only two motion durations
(150ms / 250ms). Icons come exclusively from the Lucide icon set so the
icon bake at #434 has a closed enumeration to compile against.

## Entity Types

Theme(.Name) is an entity type.
  Dark Theme is a subtype of Theme.
  Light Theme is a subtype of Theme.
  {Dark Theme, Light Theme} are mutually exclusive subtypes of Theme.

ColorToken(.Name) is an entity type.

TypographyScale(.Name) is an entity type.

SpacingToken(.Name) is an entity type.

MotionToken(.Name) is an entity type.

IconToken(.Name) is an entity type.

FontFamily(.Name) is an entity type.
  Sans Family is a subtype of FontFamily.
  Mono Family is a subtype of FontFamily.
  {Sans Family, Mono Family} are mutually exclusive subtypes of FontFamily.

## Value Types

Hex Color is a value type.
Color Role is a value type.
  The possible values of Color Role are
    'primary', 'secondary', 'surface',
    'text-primary', 'text-muted',
    'accent', 'success', 'warning', 'danger', 'info',
    'neutral-50', 'neutral-100', 'neutral-200',
    'neutral-700', 'neutral-900',
    'border', 'overlay'.

Font Weight is a value type.
  The possible values of Font Weight are
    '300', '400', '500', '600', '700'.

Type Role is a value type.
  The possible values of Type Role are
    'display', 'h1', 'h2', 'h3', 'body', 'body-sm', 'caption', 'code',
    'label', 'button'.

Pixels is a value type.

Spacing Step is a value type.
  The possible values of Spacing Step are
    'xs', 'sm', 'md', 'lg', 'xl', '2xl', '3xl'.

Milliseconds is a value type.

Easing is a value type.

Motion Role is a value type.
  The possible values of Motion Role are
    'fast', 'normal', 'enter', 'exit', 'emphasized'.

Lucide Name is a value type.

Icon Role is a value type.

Theme Mode is a value type.
  The possible values of Theme Mode are 'dark', 'light'.

Token Group is a value type.
  The possible values of Token Group are
    'color', 'typography', 'spacing', 'motion', 'icon'.

## Fact Types

### Theme

Theme has Theme Mode.
  Each Theme has exactly one Theme Mode.

Theme is the default Theme.
  At most one Theme is the default Theme.

Theme has display- Title.
  Each Theme has at most one display- Title.

### ColorToken

ColorToken belongs to Theme.
  Each ColorToken belongs to exactly one Theme.

ColorToken has Color Role.
  Each ColorToken has exactly one Color Role.

ColorToken has Hex Color.
  Each ColorToken has exactly one Hex Color.

ColorToken has Description.
  Each ColorToken has at most one Description.

### TypographyScale

TypographyScale has Type Role.
  Each TypographyScale has exactly one Type Role.

TypographyScale uses FontFamily.
  Each TypographyScale uses exactly one FontFamily.

TypographyScale has Font Weight.
  Each TypographyScale has exactly one Font Weight.

TypographyScale has Pixels as font size.
  Each TypographyScale has exactly one Pixels as font size.

TypographyScale has Pixels as line height.
  Each TypographyScale has exactly one Pixels as line height.

TypographyScale has Pixels as letter spacing.
  Each TypographyScale has at most one Pixels as letter spacing.

### FontFamily

FontFamily has Name.
  Each FontFamily has exactly one Name.

FontFamily has fallback- Name.

### SpacingToken

SpacingToken has Spacing Step.
  Each SpacingToken has exactly one Spacing Step.

SpacingToken has Pixels.
  Each SpacingToken has exactly one Pixels.

### MotionToken

MotionToken has Motion Role.
  Each MotionToken has exactly one Motion Role.

MotionToken has Milliseconds.
  Each MotionToken has exactly one Milliseconds.

MotionToken has Easing.
  Each MotionToken has exactly one Easing.

### IconToken

IconToken has Lucide Name.
  Each IconToken has exactly one Lucide Name.

IconToken has Icon Role.
  Each IconToken has at most one Icon Role.

IconToken has Description.
  Each IconToken has at most one Description.

## Constraints

Each Theme has at most one ColorToken per Color Role.
Each TypographyScale name maps to exactly one Type Role.
Each SpacingToken name maps to exactly one Spacing Step.
No two SpacingTokens share the same Spacing Step.
No two MotionTokens share the same Motion Role.
No two IconTokens share the same Lucide Name.

Each Pixels value used by a SpacingToken is a non-negative multiple of 4.
Each Pixels value used as font size by a TypographyScale is a multiple of 1.
Each Pixels value used as line height by a TypographyScale is a multiple of 4.

## Deontic Constraints

It is obligatory that each Theme has some ColorToken with Color Role 'primary'.
It is obligatory that each Theme has some ColorToken with Color Role 'surface'.
It is obligatory that each Theme has some ColorToken with Color Role 'text-primary'.
It is obligatory that each Theme has some ColorToken with Color Role 'accent'.
It is obligatory that each Theme has some ColorToken with Color Role 'success'.
It is obligatory that each Theme has some ColorToken with Color Role 'warning'.
It is obligatory that each Theme has some ColorToken with Color Role 'danger'.
It is obligatory that each Theme has some ColorToken with Color Role 'info'.

## Derivation Rules

* SpacingToken 'xs' has Pixels 4.
* SpacingToken 'sm' has Pixels 8.
* SpacingToken 'md' has Pixels 16.
* SpacingToken 'lg' has Pixels 24.
* SpacingToken 'xl' has Pixels 32.
* SpacingToken '2xl' has Pixels 48.
* SpacingToken '3xl' has Pixels 64.

+ ColorToken belongs to default Theme if ColorToken belongs to Theme and that Theme is the default Theme.

## Instance Facts

Domain 'ui' has Description 'Design system tokens (colors, typography, spacing, motion, icons) shared across Slint kernel UI and ui.do React frontend.'.

### Font families

FontFamily 'Inter' is a Sans Family.
FontFamily 'Inter' has fallback- Name 'system-ui'.
FontFamily 'Inter' has fallback- Name '-apple-system'.
FontFamily 'Inter' has fallback- Name 'Segoe UI'.
FontFamily 'Inter' has fallback- Name 'Roboto'.
FontFamily 'Inter' has fallback- Name 'sans-serif'.

FontFamily 'JetBrains Mono' is a Mono Family.
FontFamily 'JetBrains Mono' has fallback- Name 'SF Mono'.
FontFamily 'JetBrains Mono' has fallback- Name 'Menlo'.
FontFamily 'JetBrains Mono' has fallback- Name 'Consolas'.
FontFamily 'JetBrains Mono' has fallback- Name 'monospace'.

### Spacing tokens (4px grid)

SpacingToken 'xs' has Spacing Step 'xs'.
SpacingToken 'xs' has Pixels 4.

SpacingToken 'sm' has Spacing Step 'sm'.
SpacingToken 'sm' has Pixels 8.

SpacingToken 'md' has Spacing Step 'md'.
SpacingToken 'md' has Pixels 16.

SpacingToken 'lg' has Spacing Step 'lg'.
SpacingToken 'lg' has Pixels 24.

SpacingToken 'xl' has Spacing Step 'xl'.
SpacingToken 'xl' has Pixels 32.

SpacingToken '2xl' has Spacing Step '2xl'.
SpacingToken '2xl' has Pixels 48.

SpacingToken '3xl' has Spacing Step '3xl'.
SpacingToken '3xl' has Pixels 64.

### Motion tokens

MotionToken 'fast' has Motion Role 'fast'.
MotionToken 'fast' has Milliseconds 150.
MotionToken 'fast' has Easing 'cubic-bezier(0.4, 0.0, 0.2, 1)'.

MotionToken 'normal' has Motion Role 'normal'.
MotionToken 'normal' has Milliseconds 250.
MotionToken 'normal' has Easing 'cubic-bezier(0.4, 0.0, 0.2, 1)'.

MotionToken 'enter' has Motion Role 'enter'.
MotionToken 'enter' has Milliseconds 250.
MotionToken 'enter' has Easing 'cubic-bezier(0.0, 0.0, 0.2, 1)'.

MotionToken 'exit' has Motion Role 'exit'.
MotionToken 'exit' has Milliseconds 150.
MotionToken 'exit' has Easing 'cubic-bezier(0.4, 0.0, 1, 1)'.

MotionToken 'emphasized' has Motion Role 'emphasized'.
MotionToken 'emphasized' has Milliseconds 250.
MotionToken 'emphasized' has Easing 'cubic-bezier(0.2, 0.0, 0, 1)'.

### Typography scale

TypographyScale 'display' has Type Role 'display'.
TypographyScale 'display' uses FontFamily 'Inter'.
TypographyScale 'display' has Font Weight '600'.
TypographyScale 'display' has Pixels 36 as font size.
TypographyScale 'display' has Pixels 44 as line height.

TypographyScale 'h1' has Type Role 'h1'.
TypographyScale 'h1' uses FontFamily 'Inter'.
TypographyScale 'h1' has Font Weight '600'.
TypographyScale 'h1' has Pixels 28 as font size.
TypographyScale 'h1' has Pixels 36 as line height.

TypographyScale 'h2' has Type Role 'h2'.
TypographyScale 'h2' uses FontFamily 'Inter'.
TypographyScale 'h2' has Font Weight '600'.
TypographyScale 'h2' has Pixels 22 as font size.
TypographyScale 'h2' has Pixels 28 as line height.

TypographyScale 'h3' has Type Role 'h3'.
TypographyScale 'h3' uses FontFamily 'Inter'.
TypographyScale 'h3' has Font Weight '500'.
TypographyScale 'h3' has Pixels 18 as font size.
TypographyScale 'h3' has Pixels 24 as line height.

TypographyScale 'body' has Type Role 'body'.
TypographyScale 'body' uses FontFamily 'Inter'.
TypographyScale 'body' has Font Weight '400'.
TypographyScale 'body' has Pixels 14 as font size.
TypographyScale 'body' has Pixels 20 as line height.

TypographyScale 'body-sm' has Type Role 'body-sm'.
TypographyScale 'body-sm' uses FontFamily 'Inter'.
TypographyScale 'body-sm' has Font Weight '400'.
TypographyScale 'body-sm' has Pixels 13 as font size.
TypographyScale 'body-sm' has Pixels 20 as line height.

TypographyScale 'caption' has Type Role 'caption'.
TypographyScale 'caption' uses FontFamily 'Inter'.
TypographyScale 'caption' has Font Weight '400'.
TypographyScale 'caption' has Pixels 12 as font size.
TypographyScale 'caption' has Pixels 16 as line height.

TypographyScale 'label' has Type Role 'label'.
TypographyScale 'label' uses FontFamily 'Inter'.
TypographyScale 'label' has Font Weight '500'.
TypographyScale 'label' has Pixels 12 as font size.
TypographyScale 'label' has Pixels 16 as line height.

TypographyScale 'button' has Type Role 'button'.
TypographyScale 'button' uses FontFamily 'Inter'.
TypographyScale 'button' has Font Weight '500'.
TypographyScale 'button' has Pixels 14 as font size.
TypographyScale 'button' has Pixels 20 as line height.

TypographyScale 'code' has Type Role 'code'.
TypographyScale 'code' uses FontFamily 'JetBrains Mono'.
TypographyScale 'code' has Font Weight '400'.
TypographyScale 'code' has Pixels 13 as font size.
TypographyScale 'code' has Pixels 20 as line height.

### Theme: Dark (default)

Theme 'dark' is a Dark Theme.
Theme 'dark' has Theme Mode 'dark'.
Theme 'dark' has display- Title 'Modern Minimal Dark'.
Theme 'dark' is the default Theme.

ColorToken 'dark.neutral-50' belongs to Theme 'dark'.
ColorToken 'dark.neutral-50' has Color Role 'neutral-50'.
ColorToken 'dark.neutral-50' has Hex Color '#0A0A0B'.

ColorToken 'dark.neutral-100' belongs to Theme 'dark'.
ColorToken 'dark.neutral-100' has Color Role 'neutral-100'.
ColorToken 'dark.neutral-100' has Hex Color '#111114'.

ColorToken 'dark.neutral-200' belongs to Theme 'dark'.
ColorToken 'dark.neutral-200' has Color Role 'neutral-200'.
ColorToken 'dark.neutral-200' has Hex Color '#1A1A1F'.

ColorToken 'dark.neutral-700' belongs to Theme 'dark'.
ColorToken 'dark.neutral-700' has Color Role 'neutral-700'.
ColorToken 'dark.neutral-700' has Hex Color '#A0A0AB'.

ColorToken 'dark.neutral-900' belongs to Theme 'dark'.
ColorToken 'dark.neutral-900' has Color Role 'neutral-900'.
ColorToken 'dark.neutral-900' has Hex Color '#F5F5F7'.

ColorToken 'dark.surface' belongs to Theme 'dark'.
ColorToken 'dark.surface' has Color Role 'surface'.
ColorToken 'dark.surface' has Hex Color '#111114'.
ColorToken 'dark.surface' has Description 'App background; same as neutral-100.'.

ColorToken 'dark.primary' belongs to Theme 'dark'.
ColorToken 'dark.primary' has Color Role 'primary'.
ColorToken 'dark.primary' has Hex Color '#F5F5F7'.

ColorToken 'dark.secondary' belongs to Theme 'dark'.
ColorToken 'dark.secondary' has Color Role 'secondary'.
ColorToken 'dark.secondary' has Hex Color '#A0A0AB'.

ColorToken 'dark.text-primary' belongs to Theme 'dark'.
ColorToken 'dark.text-primary' has Color Role 'text-primary'.
ColorToken 'dark.text-primary' has Hex Color '#F5F5F7'.

ColorToken 'dark.text-muted' belongs to Theme 'dark'.
ColorToken 'dark.text-muted' has Color Role 'text-muted'.
ColorToken 'dark.text-muted' has Hex Color '#A0A0AB'.

ColorToken 'dark.border' belongs to Theme 'dark'.
ColorToken 'dark.border' has Color Role 'border'.
ColorToken 'dark.border' has Hex Color '#26262C'.

ColorToken 'dark.overlay' belongs to Theme 'dark'.
ColorToken 'dark.overlay' has Color Role 'overlay'.
ColorToken 'dark.overlay' has Hex Color '#00000099'.

ColorToken 'dark.accent' belongs to Theme 'dark'.
ColorToken 'dark.accent' has Color Role 'accent'.
ColorToken 'dark.accent' has Hex Color '#7C5CFF'.
ColorToken 'dark.accent' has Description 'Single accent — restrained palette discipline.'.

ColorToken 'dark.success' belongs to Theme 'dark'.
ColorToken 'dark.success' has Color Role 'success'.
ColorToken 'dark.success' has Hex Color '#34D399'.

ColorToken 'dark.warning' belongs to Theme 'dark'.
ColorToken 'dark.warning' has Color Role 'warning'.
ColorToken 'dark.warning' has Hex Color '#F59E0B'.

ColorToken 'dark.danger' belongs to Theme 'dark'.
ColorToken 'dark.danger' has Color Role 'danger'.
ColorToken 'dark.danger' has Hex Color '#EF4444'.

ColorToken 'dark.info' belongs to Theme 'dark'.
ColorToken 'dark.info' has Color Role 'info'.
ColorToken 'dark.info' has Hex Color '#38BDF8'.

### Theme: Light

Theme 'light' is a Light Theme.
Theme 'light' has Theme Mode 'light'.
Theme 'light' has display- Title 'Modern Minimal Light'.

ColorToken 'light.neutral-50' belongs to Theme 'light'.
ColorToken 'light.neutral-50' has Color Role 'neutral-50'.
ColorToken 'light.neutral-50' has Hex Color '#FFFFFF'.

ColorToken 'light.neutral-100' belongs to Theme 'light'.
ColorToken 'light.neutral-100' has Color Role 'neutral-100'.
ColorToken 'light.neutral-100' has Hex Color '#FAFAFB'.

ColorToken 'light.neutral-200' belongs to Theme 'light'.
ColorToken 'light.neutral-200' has Color Role 'neutral-200'.
ColorToken 'light.neutral-200' has Hex Color '#F0F0F2'.

ColorToken 'light.neutral-700' belongs to Theme 'light'.
ColorToken 'light.neutral-700' has Color Role 'neutral-700'.
ColorToken 'light.neutral-700' has Hex Color '#52525B'.

ColorToken 'light.neutral-900' belongs to Theme 'light'.
ColorToken 'light.neutral-900' has Color Role 'neutral-900'.
ColorToken 'light.neutral-900' has Hex Color '#0A0A0B'.

ColorToken 'light.surface' belongs to Theme 'light'.
ColorToken 'light.surface' has Color Role 'surface'.
ColorToken 'light.surface' has Hex Color '#FFFFFF'.

ColorToken 'light.primary' belongs to Theme 'light'.
ColorToken 'light.primary' has Color Role 'primary'.
ColorToken 'light.primary' has Hex Color '#0A0A0B'.

ColorToken 'light.secondary' belongs to Theme 'light'.
ColorToken 'light.secondary' has Color Role 'secondary'.
ColorToken 'light.secondary' has Hex Color '#52525B'.

ColorToken 'light.text-primary' belongs to Theme 'light'.
ColorToken 'light.text-primary' has Color Role 'text-primary'.
ColorToken 'light.text-primary' has Hex Color '#0A0A0B'.

ColorToken 'light.text-muted' belongs to Theme 'light'.
ColorToken 'light.text-muted' has Color Role 'text-muted'.
ColorToken 'light.text-muted' has Hex Color '#52525B'.

ColorToken 'light.border' belongs to Theme 'light'.
ColorToken 'light.border' has Color Role 'border'.
ColorToken 'light.border' has Hex Color '#E4E4E7'.

ColorToken 'light.overlay' belongs to Theme 'light'.
ColorToken 'light.overlay' has Color Role 'overlay'.
ColorToken 'light.overlay' has Hex Color '#00000033'.

ColorToken 'light.accent' belongs to Theme 'light'.
ColorToken 'light.accent' has Color Role 'accent'.
ColorToken 'light.accent' has Hex Color '#5B3FE6'.

ColorToken 'light.success' belongs to Theme 'light'.
ColorToken 'light.success' has Color Role 'success'.
ColorToken 'light.success' has Hex Color '#059669'.

ColorToken 'light.warning' belongs to Theme 'light'.
ColorToken 'light.warning' has Color Role 'warning'.
ColorToken 'light.warning' has Hex Color '#D97706'.

ColorToken 'light.danger' belongs to Theme 'light'.
ColorToken 'light.danger' has Color Role 'danger'.
ColorToken 'light.danger' has Hex Color '#DC2626'.

ColorToken 'light.info' belongs to Theme 'light'.
ColorToken 'light.info' has Color Role 'info'.
ColorToken 'light.info' has Hex Color '#0284C7'.

### Lucide icon set

Canonical list consumed by the icon bake at #434. Names match the
official Lucide registry exactly so the bake script can resolve them
without aliasing. Roles tag the canonical use site; the same icon may
be reused elsewhere without a new IconToken.

# File browser

IconToken 'file' has Lucide Name 'file'.
IconToken 'file' has Icon Role 'file-browser'.
IconToken 'file' has Description 'Generic file leaf node in the file browser tree.'.

IconToken 'file-text' has Lucide Name 'file-text'.
IconToken 'file-text' has Icon Role 'file-browser'.

IconToken 'file-code' has Lucide Name 'file-code'.
IconToken 'file-code' has Icon Role 'file-browser'.

IconToken 'folder' has Lucide Name 'folder'.
IconToken 'folder' has Icon Role 'file-browser'.

IconToken 'folder-open' has Lucide Name 'folder-open'.
IconToken 'folder-open' has Icon Role 'file-browser'.

IconToken 'folder-plus' has Lucide Name 'folder-plus'.
IconToken 'folder-plus' has Icon Role 'file-browser'.

IconToken 'upload' has Lucide Name 'upload'.
IconToken 'upload' has Icon Role 'file-browser'.

IconToken 'download' has Lucide Name 'download'.
IconToken 'download' has Icon Role 'file-browser'.

# REPL

IconToken 'terminal' has Lucide Name 'terminal'.
IconToken 'terminal' has Icon Role 'repl'.

IconToken 'play' has Lucide Name 'play'.
IconToken 'play' has Icon Role 'repl'.

IconToken 'square' has Lucide Name 'square'.
IconToken 'square' has Icon Role 'repl'.
IconToken 'square' has Description 'Stop button glyph.'.

IconToken 'rotate-ccw' has Lucide Name 'rotate-ccw'.
IconToken 'rotate-ccw' has Icon Role 'repl'.
IconToken 'rotate-ccw' has Description 'Reset / restart REPL session.'.

IconToken 'copy' has Lucide Name 'copy'.
IconToken 'copy' has Icon Role 'repl'.

# HATEOAS browser

IconToken 'link' has Lucide Name 'link'.
IconToken 'link' has Icon Role 'hateoas'.

IconToken 'external-link' has Lucide Name 'external-link'.
IconToken 'external-link' has Icon Role 'hateoas'.

IconToken 'arrow-left' has Lucide Name 'arrow-left'.
IconToken 'arrow-left' has Icon Role 'hateoas'.

IconToken 'arrow-right' has Lucide Name 'arrow-right'.
IconToken 'arrow-right' has Icon Role 'hateoas'.

IconToken 'home' has Lucide Name 'home'.
IconToken 'home' has Icon Role 'hateoas'.

IconToken 'globe' has Lucide Name 'globe'.
IconToken 'globe' has Icon Role 'hateoas'.

# Common controls

IconToken 'search' has Lucide Name 'search'.
IconToken 'search' has Icon Role 'common'.

IconToken 'x' has Lucide Name 'x'.
IconToken 'x' has Icon Role 'common'.
IconToken 'x' has Description 'Close / dismiss.'.

IconToken 'check' has Lucide Name 'check'.
IconToken 'check' has Icon Role 'common'.

IconToken 'plus' has Lucide Name 'plus'.
IconToken 'plus' has Icon Role 'common'.

IconToken 'minus' has Lucide Name 'minus'.
IconToken 'minus' has Icon Role 'common'.

IconToken 'trash' has Lucide Name 'trash'.
IconToken 'trash' has Icon Role 'common'.

IconToken 'pencil' has Lucide Name 'pencil'.
IconToken 'pencil' has Icon Role 'common'.

IconToken 'save' has Lucide Name 'save'.
IconToken 'save' has Icon Role 'common'.

IconToken 'settings' has Lucide Name 'settings'.
IconToken 'settings' has Icon Role 'common'.

IconToken 'menu' has Lucide Name 'menu'.
IconToken 'menu' has Icon Role 'common'.

IconToken 'more-horizontal' has Lucide Name 'more-horizontal'.
IconToken 'more-horizontal' has Icon Role 'common'.

IconToken 'more-vertical' has Lucide Name 'more-vertical'.
IconToken 'more-vertical' has Icon Role 'common'.

IconToken 'chevron-right' has Lucide Name 'chevron-right'.
IconToken 'chevron-right' has Icon Role 'common'.

IconToken 'chevron-left' has Lucide Name 'chevron-left'.
IconToken 'chevron-left' has Icon Role 'common'.

IconToken 'chevron-down' has Lucide Name 'chevron-down'.
IconToken 'chevron-down' has Icon Role 'common'.

IconToken 'chevron-up' has Lucide Name 'chevron-up'.
IconToken 'chevron-up' has Icon Role 'common'.

IconToken 'filter' has Lucide Name 'filter'.
IconToken 'filter' has Icon Role 'common'.

IconToken 'sort-asc' has Lucide Name 'arrow-up-narrow-wide'.
IconToken 'sort-asc' has Icon Role 'common'.

IconToken 'sort-desc' has Lucide Name 'arrow-down-narrow-wide'.
IconToken 'sort-desc' has Icon Role 'common'.

# Status / semantic

IconToken 'info' has Lucide Name 'info'.
IconToken 'info' has Icon Role 'status'.

IconToken 'alert-triangle' has Lucide Name 'alert-triangle'.
IconToken 'alert-triangle' has Icon Role 'status'.

IconToken 'alert-circle' has Lucide Name 'alert-circle'.
IconToken 'alert-circle' has Icon Role 'status'.

IconToken 'check-circle' has Lucide Name 'check-circle'.
IconToken 'check-circle' has Icon Role 'status'.

IconToken 'x-circle' has Lucide Name 'x-circle'.
IconToken 'x-circle' has Icon Role 'status'.

IconToken 'loader' has Lucide Name 'loader'.
IconToken 'loader' has Icon Role 'status'.
IconToken 'loader' has Description 'Spinner glyph; pair with motion token "normal" for rotation.'.

# Auth / user

IconToken 'user' has Lucide Name 'user'.
IconToken 'user' has Icon Role 'auth'.

IconToken 'log-in' has Lucide Name 'log-in'.
IconToken 'log-in' has Icon Role 'auth'.

IconToken 'log-out' has Lucide Name 'log-out'.
IconToken 'log-out' has Icon Role 'auth'.

IconToken 'lock' has Lucide Name 'lock'.
IconToken 'lock' has Icon Role 'auth'.

IconToken 'unlock' has Lucide Name 'unlock'.
IconToken 'unlock' has Icon Role 'auth'.

# Theme switcher

IconToken 'sun' has Lucide Name 'sun'.
IconToken 'sun' has Icon Role 'theme'.

IconToken 'moon' has Lucide Name 'moon'.
IconToken 'moon' has Icon Role 'theme'.

IconToken 'palette' has Lucide Name 'palette'.
IconToken 'palette' has Icon Role 'theme'.
