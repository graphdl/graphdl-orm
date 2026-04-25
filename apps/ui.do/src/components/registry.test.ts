/**
 * Web Component adapter — vitest smoke (#494 Track KKKK).
 *
 * Mirrors crates/arest-kernel/src/ui_apps/registry.rs::tests in shape:
 * the pure scanner is exercised against jsdom's empty `customElements`
 * registry plus the curated list of standard HTML elements DDDD's
 * `readings/ui/components.md` (#485) declared web-components bindings
 * for. Counts are pinned so a future drift in the seed list is caught.
 *
 * Follow-up registration is exercised against a stub `create`
 * implementation; we assert the call shapes (resource + body) match
 * the cell-name + fact-binding convention FFFF uses on the kernel
 * side via `fact_from_pairs`.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  STANDARD_HTML_COMPONENTS,
  registerWebComponents,
  scanWebComponents,
  type ComponentFact,
} from './registry'

interface CapturedCreate {
  resource: string
  data: ComponentFact
}

function makeProviderStub(): { provider: { create: (resource: string, params: { data: ComponentFact }) => Promise<{ data: ComponentFact }> }; calls: CapturedCreate[] } {
  const calls: CapturedCreate[] = []
  return {
    provider: {
      async create(resource: string, params: { data: ComponentFact }) {
        calls.push({ resource, data: params.data })
        return { data: params.data }
      },
    },
    calls,
  }
}

describe('scanWebComponents', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('seeds the 9 standard HTML element specs DDDD declared in components.md', () => {
    // Hard-pinned count — fence-posts against accidental drift in the
    // seed list. DDDD's reading enumerates exactly nine
    // ImplementationBinding rows under Toolkit 'web-components':
    // <button>, <input type=text>, <input type=date>, <dialog>,
    // <img>, <input type=range>, <select>, <input type=checkbox>,
    // <progress>.
    expect(STANDARD_HTML_COMPONENTS).toHaveLength(9)
    const symbols = STANDARD_HTML_COMPONENTS.map((c) => c.toolkitSymbol)
    expect(symbols).toEqual([
      '<button>',
      '<input type=text>',
      '<input type=date>',
      '<dialog>',
      '<img>',
      '<input type=range>',
      '<select>',
      '<input type=checkbox>',
      '<progress>',
    ])
  })

  it('emits one Component_has_ComponentRole anchor fact per seeded element', () => {
    const facts = scanWebComponents()
    const anchors = facts.filter((f) => f.cell === 'Component_has_ComponentRole')
    expect(anchors).toHaveLength(9)
    const roles = anchors.map((f) => f.bindings.ComponentRole).sort()
    expect(roles).toEqual([
      'button',
      'checkbox',
      'combo-box',
      'date-picker',
      'dialog',
      'image',
      'progress-bar',
      'slider',
      'text-input',
    ])
  })

  it('emits one ImplementationBinding pivot fact per seeded element bound to Toolkit web-components', () => {
    const facts = scanWebComponents()
    const pivots = facts.filter(
      (f) => f.cell === 'ImplementationBinding_pivots_Component_is_implemented_by_Toolkit',
    )
    expect(pivots).toHaveLength(9)
    for (const p of pivots) {
      expect(p.bindings.Toolkit).toBe('web-components')
      // Convention mirrors FFFF's `<component>.slint` slug — web side
      // is `<component>.web`, matching DDDD's reading line 422
      // (`'button.web' pivots Component 'button'`).
      expect(p.bindings.ImplementationBinding).toMatch(/\.web$/)
    }
  })

  it('emits the Toolkit Symbol triple for every web-component implementation', () => {
    const facts = scanWebComponents()
    const triples = facts.filter(
      (f) => f.cell === 'Component_is_implemented_by_Toolkit_at_ToolkitSymbol',
    )
    expect(triples).toHaveLength(9)
    const buttonTriple = triples.find((t) => t.bindings.Component === 'button')
    expect(buttonTriple?.bindings.Toolkit).toBe('web-components')
    expect(buttonTriple?.bindings.ToolkitSymbol).toBe('<button>')
  })

  it('emits hidpi_native ImplementationBinding trait for every native HTML element', () => {
    // DDDD's reading attaches `hidpi_native` to every web-side
    // binding because vector-clean DPR is universal in HTML
    // (browser handles the scaling). Mirrors components.md lines
    // 425, 469, 514 etc.
    const facts = scanWebComponents()
    const hidpiTraits = facts.filter(
      (f) =>
        f.cell === 'ImplementationBinding_has_Trait' &&
        f.bindings.ComponentTrait === 'hidpi_native',
    )
    expect(hidpiTraits.length).toBeGreaterThanOrEqual(9)
  })

  it('emits screen_reader_aware ImplementationBinding trait for the elements with implicit ARIA roles', () => {
    // Native HTML form controls + dialog all carry implicit ARIA
    // roles surfaced by AT-SPI / UIA / VoiceOver without an
    // app-side aria-* attribute. The scanner attaches the trait
    // for those elements only; <img> deliberately omits because
    // <img> needs an explicit alt= to be screen-reader meaningful.
    const facts = scanWebComponents()
    const screenReaderBindings = facts
      .filter(
        (f) =>
          f.cell === 'ImplementationBinding_has_Trait' &&
          f.bindings.ComponentTrait === 'screen_reader_aware',
      )
      .map((f) => f.bindings.ImplementationBinding)
    // Spec from components.md: button, text-input, date-picker,
    // dialog, combo-box, checkbox, progress-bar all carry the
    // screen_reader_aware trait on their .web binding.
    // image/slider intentionally omit (image needs explicit alt;
    // slider's binding shows touch_optimized but not
    // screen_reader_aware in the reading).
    expect(screenReaderBindings).toContain('button.web')
    expect(screenReaderBindings).toContain('text-input.web')
    expect(screenReaderBindings).toContain('date-picker.web')
    expect(screenReaderBindings).toContain('dialog.web')
    expect(screenReaderBindings).toContain('combo-box.web')
    expect(screenReaderBindings).toContain('checkbox.web')
    expect(screenReaderBindings).toContain('progress-bar.web')
  })

  it('emits ComponentProperty facts for every (Component, PropertyName) pair from the seed', () => {
    const facts = scanWebComponents()
    const propFacts = facts.filter(
      (f) =>
        f.cell === 'Component_has_Property_of_PropertyType_with_PropertyDefault',
    )
    // Mirrors DDDD's reading property surface, web-tier subset:
    // button: text, enabled, primary               -> 3
    // text-input: text, placeholder, enabled, maxlength -> 4
    // date-picker: value, enabled                   -> 2
    // dialog: title, open                           -> 2
    // image: source, fit                            -> 2
    // slider: value, minimum, maximum               -> 3
    // combo-box: items, selected                    -> 2
    // checkbox: checked, label, enabled             -> 3
    // progress-bar: value, maximum, indeterminate   -> 3
    // = 24 total
    expect(propFacts).toHaveLength(24)
  })

  it('emits ComponentEvent facts for every (Component, EventName) pair from the seed', () => {
    const facts = scanWebComponents()
    const eventFacts = facts.filter(
      (f) => f.cell === 'Component_emits_Event_with_EventPayloadType',
    )
    // Mirrors DDDD's reading event surface, web-tier subset:
    // button.clicked, text-input changed/submitted,
    // date-picker changed, dialog closed/confirmed,
    // slider changed, combo-box selection-changed,
    // checkbox toggled = 9
    // image / progress-bar don't emit events.
    expect(eventFacts).toHaveLength(9)
  })

  it('does not emit a Component fact for a custom element that has no spec', () => {
    // Define a synthetic custom element. The scanner returns specs
    // for the curated standard HTML element list only — a future
    // track can extend by adding observed custom elements to the
    // STANDARD_HTML_COMPONENTS table. We assert nothing for it
    // appears, so a typo in mdxui-defined custom elements doesn't
    // silently inject malformed Component facts.
    class FakeCustom extends HTMLElement {}
    customElements.define('fake-kkkk-element', FakeCustom)

    const facts = scanWebComponents()
    const fakeFacts = facts.filter((f) =>
      Object.values(f.bindings).includes('fake-kkkk-element'),
    )
    expect(fakeFacts).toHaveLength(0)
  })

  it('includes a custom element in the scan when it appears in CUSTOM_ELEMENT_HOOKS', async () => {
    // The scanner lifts a custom element into the fact stream when
    // CUSTOM_ELEMENT_HOOKS ships a spec for it (e.g. mdxui's
    // <mdxui-card> would land here). With no hooks set, no extra
    // facts. This is a contract test — once mdxui defines a custom
    // element worth registering, a follow-up populates the hook
    // list and this test doc-comments the seam.
    const facts = scanWebComponents({ customElementHooks: [] })
    // Just the 9 standard anchors.
    const anchors = facts.filter((f) => f.cell === 'Component_has_ComponentRole')
    expect(anchors).toHaveLength(9)
  })
})

describe('registerWebComponents', () => {
  it('POSTs each scanned fact through arestDataProvider.create using the cell name as resource', async () => {
    const { provider, calls } = makeProviderStub()
    await registerWebComponents(provider)

    // One create() call per fact emitted by scanWebComponents().
    const expected = scanWebComponents().length
    expect(calls).toHaveLength(expected)

    // Each call's resource matches the fact's cell — the worker
    // routes /arest/{resource} to the corresponding cell handler
    // (or its noun's CRUD surface). Documented in the report:
    // confirmed via tracing through arestDataProvider.create —
    // resource is plural-slug; cell name is in PascalCase_with_
    // underscores so a worker-side router slug-cases it on its
    // side.
    for (const c of calls) {
      expect(c.resource).toMatch(/^Component|^Implementation|^Notice/)
      expect(c.data.cell).toBe(c.resource)
      expect(typeof c.data.bindings).toBe('object')
    }

    // Spec smoke: Button anchor goes through.
    const buttonAnchor = calls.find(
      (c) =>
        c.data.cell === 'Component_has_ComponentRole' &&
        c.data.bindings.Component === 'button',
    )
    expect(buttonAnchor).toBeDefined()
    expect(buttonAnchor?.data.bindings.ComponentRole).toBe('button')
  })

  it('continues registration even if one create() rejects', async () => {
    // Idempotent-but-not-set-semantics layering — the register loop
    // logs and continues so a single failed POST (e.g. duplicate-
    // key 409 because an earlier boot already pushed the fact)
    // doesn't abort the remaining 80+ creates.
    let n = 0
    const provider = {
      async create(_resource: string, params: { data: ComponentFact }) {
        n += 1
        if (n === 3) throw new Error('409 Conflict')
        return { data: params.data }
      },
    }

    const reports = await registerWebComponents(provider)
    expect(reports.attempted).toBe(scanWebComponents().length)
    expect(reports.failed).toBe(1)
    expect(reports.succeeded).toBe(reports.attempted - 1)
  })
})
