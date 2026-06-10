/**
 * ViewForm — §5.2 viewproj-client-render, the ui.do half.
 *
 * Renders the ENGINE-EMITTED View projection (`ViewProjection`, the
 * Theorem-4 view layer riding the get-one envelope) as a read-only
 * widget form: one row per ViewElement, the widget chosen by the
 * element's `componentRole` (§4.2 value-type → widget mapping), the
 * value joined from the entity record. The web sibling of the
 * kernel's `crates/arest-kernel/src/view_form.rs` + UnifiedRepl.slint
 * ViewField form — same projection, different Render Target.
 *
 * Widget bindings follow the components.md 'web-components' tier
 * (src/components/registry.ts): text-input → <input type=text>,
 * date-picker → <input type=date>, checkbox → <input type=checkbox>,
 * combo-box → <select>. All readOnly/disabled — this is the SHOW
 * surface; the edit affordance stays with GenericEditView until the
 * mutation half of the contract lands.
 *
 * Label + value derivation mirrors view_form.rs::view_fields_for:
 * strip the `{Noun}_has_` prefix off the rendered Fact Type id for
 * the label (underscores → spaces), then read the record field by
 * the spaced, underscored, and raw keys (fact role keys may be
 * spaced or underscored depending on the writer).
 */
import type { ReactElement } from 'react'
import type { ViewProjection } from '../providers/types'

export interface ViewFormProps {
  /** The engine-emitted projection (get-one envelope `view`). */
  view: ViewProjection
  /** The fetched entity's flat field record. */
  record: Record<string, unknown>
  /** Noun, for the `{Noun}_has_` label-prefix strip. */
  noun: string
}

/** Label + lookup keys for one rendered Fact Type. */
function fieldKeys(factType: string, noun: string): { label: string; keys: string[] } {
  const prefix = `${noun.replace(/ /g, '_')}_has_`
  const attr = factType.startsWith(prefix) ? factType.slice(prefix.length) : factType
  const spaced = attr.replace(/_/g, ' ')
  return { label: spaced, keys: [spaced, attr, factType] }
}

function valueOf(record: Record<string, unknown>, keys: string[]): string {
  for (const k of keys) {
    const v = record[k]
    if (v === null || v === undefined) continue
    return typeof v === 'string' ? v : String(v)
  }
  return ''
}

function Widget({ role, value, label }: { role: string; value: string; label: string }): ReactElement {
  switch (role) {
    case 'checkbox':
      return (
        <input
          type="checkbox"
          checked={value === 'true'}
          readOnly
          disabled
          aria-label={label}
        />
      )
    case 'date-picker':
      return <input type="date" value={value} readOnly aria-label={label} />
    case 'combo-box':
      // Read-only show surface: the closed list isn't carried by the
      // projection (Enum Values ride the schema), so the select holds
      // just the current value.
      return (
        <select value={value} disabled aria-label={label}>
          <option value={value}>{value}</option>
        </select>
      )
    case 'text-input':
    default:
      return <input type="text" value={value} readOnly aria-label={label} />
  }
}

export function ViewForm({ view, record, noun }: ViewFormProps): ReactElement {
  // Deterministic order — the engine sorts elements by Fact Type; keep
  // that order verbatim (the projection is the source of truth).
  return (
    <div
      data-testid="view-form"
      data-view={view.view}
      data-view-source={view.source}
      style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', rowGap: '0.5rem', columnGap: '1rem' }}
    >
      {view.elements.map((el) => {
        const { label, keys } = fieldKeys(el.factType, noun)
        const value = valueOf(record, keys)
        return (
          <div key={el.id} style={{ display: 'contents' }}>
            <label style={{ fontWeight: 600 }}>{label}</label>
            <span data-testid={`view-field-${el.factType}`} data-widget={el.componentRole}>
              <Widget role={el.componentRole} value={value} label={label} />
            </span>
          </div>
        )
      })}
    </div>
  )
}

ViewForm.displayName = 'ViewForm'
