/**
 * Web Component adapter — register `<element>` instances as Component
 * facts (#494 Track KKKK).
 *
 * DDDD's #485 (commit 80f62ae) landed `readings/ui/components.md`
 * declaring 12 well-known Components × 4 Toolkits. The 'web-components'
 * tier has 9 ImplementationBinding rows (button, text-input,
 * date-picker, dialog, image, slider, combo-box, checkbox,
 * progress-bar). Static text alone doesn't populate the live SYSTEM
 * cells — at boot the BROWSER side (ui.do) needs to actually emit
 * Component, ImplementationBinding, ComponentProperty, ComponentEvent,
 * and ComponentTrait facts so the selection logic in #492 has cells
 * to query at runtime, AND so JJJJ's #493 select_component MCP verb
 * sees web-component candidates when the rendering host is the browser.
 *
 * This module mirrors `crates/arest-kernel/src/ui_apps/registry.rs`
 * (FFFF #486) on the browser side. The Rust side uses
 * `ast::cell_push` + `ast::fact_from_pairs` against the live SYSTEM
 * Object; here we construct equivalent ComponentFact records and
 * push them through `arestDataProvider.create()` — the worker's
 * /arest/{resource} surface routes a Component cell create through
 * the existing CRUD handler, so no worker-side change is needed.
 *
 * # Scanner approach
 *
 * Pure function `scanWebComponents()` returns a flat ComponentFact
 * list. The list is built by iterating the curated
 * `STANDARD_HTML_COMPONENTS` table (9 entries; mirrors DDDD's reading
 * line-by-line — each entry comments its source range). At runtime
 * we ALSO walk `customElements.get(name)` for each name in the
 * `customElementHooks` argument so a future track that registers
 * `<mdxui-card>` etc. just appends a hook entry.
 *
 * # Fact shape
 *
 * Each fact is a discriminated union over `cell`. Bindings mirror
 * FFFF's `fact_from_pairs(&[(role, value)])` — the same role names
 * the kernel-side adapter uses (`Component`, `ComponentRole`,
 * `Toolkit`, `ToolkitSymbol`, `PropertyName`, `PropertyType`,
 * `PropertyDefault`, `EventName`, `EventPayloadType`, `SlotName`,
 * `ComponentTrait`, `ImplementationBinding`).
 *
 * # Wiring
 *
 * `registerWebComponents(provider)` calls scan + fires one
 * `provider.create(cell, { data: fact })` per fact. Failures are
 * logged-and-skipped — re-boots WILL hit duplicate-key 409s because
 * the kernel-side adapter is intentionally NOT set-semantic
 * (mirrors FFFF's `cell_push` rather than `cell_push_unique`). The
 * runtime treats duplicates as observable rather than fatal.
 *
 * If `customElements` isn't defined at scan time (SSR, a preboot
 * environment), the scan still works for the standard HTML element
 * list — only the customElement hook resolution is skipped.
 */

import type { ArestDataProvider } from '../providers/types'

/** Toolkit slug — matches Toolkit Slug enumeration in components.md L76. */
export const TOOLKIT_WEB_COMPONENTS = 'web-components' as const

/**
 * One canonical Web Component spec. Internal shape used by
 * `scanWebComponents()` to build the fact stream.
 *
 * Mirrors the `ComponentSpec` struct in
 * `crates/arest-kernel/src/ui_apps/registry.rs::ComponentSpec`
 * (FFFF #486) field-for-field so the cross-toolkit invariants are
 * trivially auditable.
 */
export interface WebComponentSpec {
  /**
   * Component identifier (FORML role-instance name). Lowercase slug
   * matching `Component Role` enumeration in components.md L65.
   */
  name: string
  /** Component Role enumeration value. Equal to `name` for seeded entries. */
  role: string
  /** `display- Title` value (UI-visible label). */
  displayTitle: string
  /** `Description` value (one-line summary). */
  description: string
  /**
   * `Toolkit Symbol` for the web binding — the HTML tag string the
   * adapter resolves at instantiation time. Includes attribute
   * predicates for `<input type=...>` to discriminate variants.
   */
  toolkitSymbol: string
  /**
   * `(name, type, default)` triples for each FORML-canonical
   * property. The web adapter (#491 follow-up) will project these
   * onto the corresponding HTML attributes.
   */
  properties: ReadonlyArray<readonly [string, string, string]>
  /** `(name, payload_type)` pairs for each canonical event. */
  events: ReadonlyArray<readonly [string, string]>
  /** Slot names exposed (mirrors `Component has Slot` facts). */
  slots: ReadonlyArray<string>
  /** Component-level traits (toolkit-agnostic). */
  componentTraits: ReadonlyArray<string>
  /**
   * Web-binding-specific traits that ride the
   * `ImplementationBinding has Trait` ternary.
   */
  bindingTraits: ReadonlyArray<string>
}

/**
 * The 9 standard HTML Component bindings DDDD's #485 reading
 * declares for Toolkit 'web-components'. Each entry's traits +
 * properties + events are extracted from
 * `readings/ui/components.md` lines noted in the comment.
 *
 * The 3 Components in DDDD's reading without a web binding (card,
 * list, tab) are intentionally absent — DDDD's reading omits them
 * and so does this adapter; the gap-detection rule in #492 will
 * surface the absence at MCP-query time.
 */
export const STANDARD_HTML_COMPONENTS: ReadonlyArray<WebComponentSpec> = [
  // From components.md "### Component: Button" — `<button>` binding
  // line 422-427. `touch_optimized` rides on the .web binding
  // because mobile browsers honour touch-target sizing on native
  // <button>; `screen_reader_aware` because every screen reader
  // exposes the implicit ARIA `button` role; `hidpi_native` because
  // the browser handles DPR scaling for vector glyph rendering.
  {
    name: 'button',
    role: 'button',
    displayTitle: 'Button',
    description:
      'Plain push button — primary control for triggering an action.',
    toolkitSymbol: '<button>',
    properties: [
      ['text', 'string', ''],
      ['enabled', 'bool', 'true'],
      ['primary', 'bool', 'false'],
    ],
    events: [['clicked', 'none']],
    slots: ['leading', 'trailing'],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'hidpi_native', 'touch_optimized'],
  },
  // From components.md "### Component: TextInput" — `<input
  // type=text>` binding line 462-466. `touch_optimized` for mobile
  // soft-keyboards; `screen_reader_aware` for the implicit ARIA
  // `textbox` role.
  {
    name: 'text-input',
    role: 'text-input',
    displayTitle: 'Text Input',
    description: 'Single-line text entry field.',
    toolkitSymbol: '<input type=text>',
    properties: [
      ['text', 'string', ''],
      ['placeholder', 'string', ''],
      ['enabled', 'bool', 'true'],
      ['maxlength', 'int', '0'],
    ],
    events: [
      ['changed', 'string'],
      ['submitted', 'string'],
    ],
    slots: ['leading', 'trailing'],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'hidpi_native', 'touch_optimized'],
  },
  // From components.md "### Component: DatePicker" — `<input
  // type=date>` binding line 511-515. `touch_optimized` because
  // mobile browsers expose a native date wheel; `screen_reader_
  // aware` because the implicit ARIA role surfaces calendar-day
  // labels.
  {
    name: 'date-picker',
    role: 'date-picker',
    displayTitle: 'Date Picker',
    description:
      "Calendar-driven date selection. No Slint binding in this slice — #486 will surface the gap as a TODO once it scans MMM's actual surface (#436).",
    toolkitSymbol: '<input type=date>',
    properties: [
      ['value', 'string', ''],
      ['enabled', 'bool', 'true'],
    ],
    events: [['changed', 'string']],
    slots: [],
    componentTraits: ['keyboard_navigable'],
    bindingTraits: ['touch_optimized', 'screen_reader_aware', 'hidpi_native'],
  },
  // From components.md "### Component: Dialog" — `<dialog>`
  // binding line 581-584. `screen_reader_aware` for the implicit
  // ARIA `dialog` role; `hidpi_native` for DPR-clean focus
  // outlines.
  {
    name: 'dialog',
    role: 'dialog',
    displayTitle: 'Dialog',
    description:
      'Modal overlay window for transient interaction (confirm, alert, form-on-overlay).',
    toolkitSymbol: '<dialog>',
    properties: [
      ['title', 'string', ''],
      ['open', 'bool', 'false'],
    ],
    events: [
      ['closed', 'none'],
      ['confirmed', 'none'],
    ],
    slots: ['children', 'footer'],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'hidpi_native'],
  },
  // From components.md "### Component: Image" — `<img>` binding
  // line 612-614. Only `hidpi_native` (browser handles DPR via
  // srcset / DPR media queries). `screen_reader_aware` is
  // intentionally absent — <img> needs an explicit alt= for screen
  // readers, so the trait isn't universal.
  {
    name: 'image',
    role: 'image',
    displayTitle: 'Image',
    description:
      'Static raster or vector image. Qt 6 reuses QLabel + pixmap because QImage is the data type, not the widget.',
    toolkitSymbol: '<img>',
    properties: [
      ['source', 'image', ''],
      ['fit', 'enum', 'contain'],
    ],
    events: [],
    slots: [],
    componentTraits: ['theming_consumer'],
    bindingTraits: ['hidpi_native'],
  },
  // From components.md "### Component: Slider" — `<input
  // type=range>` binding line 643-645. `touch_optimized` for the
  // mobile native range thumb; `hidpi_native` for the track
  // rendering. `screen_reader_aware` is omitted in DDDD's reading.
  {
    name: 'slider',
    role: 'slider',
    displayTitle: 'Slider',
    description:
      'Continuous numeric value selection along a track. Slint binding name is the expected MMM #436 surface; #486 will TODO if missing.',
    toolkitSymbol: '<input type=range>',
    properties: [
      ['value', 'int', '0'],
      ['minimum', 'int', '0'],
      ['maximum', 'int', '100'],
    ],
    events: [['changed', 'int']],
    slots: [],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['touch_optimized', 'hidpi_native'],
  },
  // From components.md "### Component: ComboBox" — `<select>`
  // binding line 673-676. `screen_reader_aware` for the implicit
  // ARIA `combobox` role; `touch_optimized` for the mobile native
  // option-sheet.
  {
    name: 'combo-box',
    role: 'combo-box',
    displayTitle: 'Combo Box',
    description:
      "Dropdown selection from a closed list. No Slint binding in this slice — #486 will surface the gap.",
    toolkitSymbol: '<select>',
    properties: [
      ['items', 'string', ''],
      ['selected', 'int', '-1'],
    ],
    events: [['selection-changed', 'int']],
    slots: [],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'touch_optimized', 'hidpi_native'],
  },
  // From components.md "### Component: CheckBox" — `<input
  // type=checkbox>` binding line 766-770. `screen_reader_aware`
  // for the implicit ARIA `checkbox` role; `touch_optimized` for
  // mobile hit targets.
  {
    name: 'checkbox',
    role: 'checkbox',
    displayTitle: 'Check Box',
    description: 'Bistate (or tristate) toggle bound to a label.',
    toolkitSymbol: '<input type=checkbox>',
    properties: [
      ['checked', 'bool', 'false'],
      ['label', 'string', ''],
      ['enabled', 'bool', 'true'],
    ],
    events: [['toggled', 'bool']],
    slots: [],
    componentTraits: ['keyboard_navigable', 'theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'touch_optimized', 'hidpi_native'],
  },
  // From components.md "### Component: ProgressBar" — `<progress>`
  // binding line 723-725. `screen_reader_aware` for the implicit
  // ARIA `progressbar` role.
  {
    name: 'progress-bar',
    role: 'progress-bar',
    displayTitle: 'Progress Bar',
    description:
      'Linear progress indicator with optional indeterminate mode.',
    toolkitSymbol: '<progress>',
    properties: [
      ['value', 'int', '0'],
      ['maximum', 'int', '100'],
      ['indeterminate', 'bool', 'false'],
    ],
    events: [],
    slots: [],
    componentTraits: ['theming_consumer'],
    bindingTraits: ['screen_reader_aware', 'hidpi_native'],
  },
] as const

/**
 * Cell name discriminator. One of the FORML cell-name slugs FFFF's
 * `register_slint_components` uses (kernel side); we mirror them
 * verbatim so the worker's CRUD router and the kernel's
 * `cell_push` see identical cell names.
 */
export type ComponentCell =
  | 'Component_has_ComponentRole'
  | 'Component_has_displayTitle'
  | 'Component_has_Description'
  | 'Component_is_implemented_by_Toolkit_at_ToolkitSymbol'
  | 'ImplementationBinding_pivots_Component_is_implemented_by_Toolkit'
  | 'Component_has_Property_of_PropertyType_with_PropertyDefault'
  | 'Component_emits_Event_with_EventPayloadType'
  | 'Component_has_Slot'
  | 'Component_has_Trait'
  | 'ImplementationBinding_has_Trait'

/**
 * One FORML fact ready to push through `arestDataProvider.create`.
 *
 * Mirrors FFFF's `cell_push(cell_name, fact_from_pairs(&[(role,
 * value)]), state)` shape: `cell` is the cell-name slug; `bindings`
 * is the role -> value map. The worker's CRUD surface unwraps
 * `bindings` into the cell's noun fields.
 */
export interface ComponentFact {
  cell: ComponentCell
  bindings: Readonly<Record<string, string>>
}

/**
 * Optional argument shape for `scanWebComponents()`. Future tracks
 * extend by adding `customElementHooks` entries — each entry binds a
 * `customElements`-registered tag name into the FORML fact stream
 * with a per-element WebComponentSpec.
 */
export interface ScanOptions {
  /**
   * Curated list of (tag, spec) pairs to surface from the
   * `customElements` registry. Empty by default — mdxui's custom
   * elements have no public FORML role mapping yet.
   *
   * If a hook entry's `tag` isn't actually defined in
   * `customElements`, the scanner skips it silently (the hook is
   * declarative; runtime presence is independent).
   */
  customElementHooks?: ReadonlyArray<{ tag: string; spec: WebComponentSpec }>
  /** Test-only override for `globalThis.customElements`. */
  customElementsRegistry?: CustomElementRegistry | null
}

/**
 * Build the canonical `<component>.web` slug used as the
 * `ImplementationBinding` reference-mode value. Matches DDDD's
 * reading convention (e.g. components.md line 422 — `'button.web'
 * pivots Component 'button'`).
 */
export function componentBindingId(componentName: string): string {
  return `${componentName}.web`
}

/**
 * Build the FORML fact stream for one Component spec. Order mirrors
 * FFFF's `push_component()` so cross-toolkit auditors can read the
 * Slint adapter and this side line-for-line.
 *
 *   1. Component_has_ComponentRole
 *   2. Component_has_displayTitle
 *   3. Component_has_Description
 *   4. Component_is_implemented_by_Toolkit_at_ToolkitSymbol
 *   5. ImplementationBinding_pivots_Component_is_implemented_by_Toolkit
 *   6. Component_has_Property_of_PropertyType_with_PropertyDefault (xN)
 *   7. Component_emits_Event_with_EventPayloadType (xM)
 *   8. Component_has_Slot (xK)
 *   9. Component_has_Trait (xT)
 *   10. ImplementationBinding_has_Trait (xT)
 */
function specToFacts(spec: WebComponentSpec): ComponentFact[] {
  const bindingId = componentBindingId(spec.name)
  const out: ComponentFact[] = []

  // 1.
  out.push({
    cell: 'Component_has_ComponentRole',
    bindings: { Component: spec.name, ComponentRole: spec.role },
  })
  // 2.
  out.push({
    cell: 'Component_has_displayTitle',
    bindings: { Component: spec.name, displayTitle: spec.displayTitle },
  })
  // 3.
  out.push({
    cell: 'Component_has_Description',
    bindings: { Component: spec.name, Description: spec.description },
  })
  // 4.
  out.push({
    cell: 'Component_is_implemented_by_Toolkit_at_ToolkitSymbol',
    bindings: {
      Component: spec.name,
      Toolkit: TOOLKIT_WEB_COMPONENTS,
      ToolkitSymbol: spec.toolkitSymbol,
    },
  })
  // 5.
  out.push({
    cell: 'ImplementationBinding_pivots_Component_is_implemented_by_Toolkit',
    bindings: {
      ImplementationBinding: bindingId,
      Component: spec.name,
      Toolkit: TOOLKIT_WEB_COMPONENTS,
    },
  })
  // 6.
  for (const [name, ty, def] of spec.properties) {
    out.push({
      cell: 'Component_has_Property_of_PropertyType_with_PropertyDefault',
      bindings: {
        Component: spec.name,
        PropertyName: name,
        PropertyType: ty,
        PropertyDefault: def,
      },
    })
  }
  // 7.
  for (const [name, payload] of spec.events) {
    out.push({
      cell: 'Component_emits_Event_with_EventPayloadType',
      bindings: {
        Component: spec.name,
        EventName: name,
        EventPayloadType: payload,
      },
    })
  }
  // 8.
  for (const slot of spec.slots) {
    out.push({
      cell: 'Component_has_Slot',
      bindings: { Component: spec.name, SlotName: slot },
    })
  }
  // 9.
  for (const t of spec.componentTraits) {
    out.push({
      cell: 'Component_has_Trait',
      bindings: { Component: spec.name, ComponentTrait: t },
    })
  }
  // 10.
  for (const t of spec.bindingTraits) {
    out.push({
      cell: 'ImplementationBinding_has_Trait',
      bindings: { ImplementationBinding: bindingId, ComponentTrait: t },
    })
  }

  return out
}

/**
 * Resolve the registry to consult for `customElements`. Defaults to
 * `globalThis.customElements`; tests inject explicit nulls / fakes.
 */
function resolveRegistry(
  override: ScanOptions['customElementsRegistry'],
): CustomElementRegistry | null {
  if (override !== undefined) return override
  if (typeof globalThis.customElements === 'undefined') return null
  return globalThis.customElements
}

/**
 * Scan the web side and build the FORML ComponentFact list.
 *
 * Pure function — the caller decides whether to push to the worker
 * via `registerWebComponents` or to inspect (the test harness).
 *
 * Always emits facts for every spec in `STANDARD_HTML_COMPONENTS`;
 * those tags are universally present in any HTML environment.
 *
 * Walks `customElements` only for the tags listed in
 * `opts.customElementHooks`. Unrecognised hook tags are skipped
 * silently (the hook is declarative; presence is independent).
 */
export function scanWebComponents(opts: ScanOptions = {}): ComponentFact[] {
  const out: ComponentFact[] = []

  for (const spec of STANDARD_HTML_COMPONENTS) {
    out.push(...specToFacts(spec))
  }

  const registry = resolveRegistry(opts.customElementsRegistry)
  for (const hook of opts.customElementHooks ?? []) {
    // Only emit facts when the custom element is actually defined,
    // so a hook list that drifts ahead of the actual mdxui
    // surface doesn't push phantom facts.
    if (registry && registry.get(hook.tag) === undefined) continue
    out.push(...specToFacts(hook.spec))
  }

  return out
}

/** Result of `registerWebComponents()` — tally for the boot log. */
export interface RegisterReport {
  attempted: number
  succeeded: number
  failed: number
}

/**
 * Provider surface used by `registerWebComponents`. Narrowed to just
 * `create` so tests don't have to satisfy the full ArestDataProvider
 * shape (and so the call site can pass a thin shim if it ever needs
 * to).
 */
export interface CreateOnlyProvider {
  create: (
    resource: string,
    params: { data: ComponentFact },
  ) => Promise<{ data: ComponentFact }>
}

/**
 * Push every scanned ComponentFact through the data provider's
 * `create()` surface. Failures are logged-and-skipped: re-boots
 * WILL hit duplicate-key 409s because the kernel-side adapter is
 * intentionally not set-semantic (FFFF's `cell_push` rather than
 * `cell_push_unique`). Treating duplicates as fatal would silently
 * orphan the rest of the registration on every reload.
 *
 * Returns a tally suitable for boot logs / dev-tools display.
 */
export async function registerWebComponents(
  provider: CreateOnlyProvider | ArestDataProvider,
  opts: ScanOptions = {},
): Promise<RegisterReport> {
  const facts = scanWebComponents(opts)
  let succeeded = 0
  let failed = 0
  for (const fact of facts) {
    try {
      await provider.create(fact.cell, { data: fact })
      succeeded += 1
    } catch (err) {
      failed += 1
      // Browser-only — console is always present in jsdom + browser.
      // eslint-disable-next-line no-console
      console.warn(
        `[ui.do] registerWebComponents: ${fact.cell} push failed`,
        fact.bindings,
        err,
      )
    }
  }
  return { attempted: facts.length, succeeded, failed }
}

/**
 * Boot-time entry: kicks off `registerWebComponents` against
 * `globalThis.customElements`, deferring the call to the next
 * animation frame so any custom elements registered by mdxui /
 * file-browser modules land first.
 *
 * If `requestAnimationFrame` isn't present (SSR / test env), runs
 * the registration synchronously.
 */
export function scanAndRegisterWebComponents(
  provider: CreateOnlyProvider | ArestDataProvider,
  opts: ScanOptions = {},
): Promise<RegisterReport> | void {
  // Defer to the next paint so any module that does
  // `customElements.define(...)` at import time gets a chance to
  // run first. The standard HTML element specs don't need this —
  // they're hardcoded — but the customElement hooks do.
  if (typeof globalThis.requestAnimationFrame === 'function') {
    return new Promise<RegisterReport>((resolve) => {
      globalThis.requestAnimationFrame(() => {
        resolve(registerWebComponents(provider, opts))
      })
    })
  }
  return registerWebComponents(provider, opts)
}
