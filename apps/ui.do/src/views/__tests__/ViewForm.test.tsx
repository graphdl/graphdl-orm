/**
 * ViewForm — unit tests for the §5.2 projection renderer (pure
 * component; the GenericViews integration suite covers the provider
 * passthrough + GenericShowView wiring).
 *
 * Pins the label/value derivation contract shared with the kernel's
 * `view_form.rs::view_fields_for`: labels strip the `{Noun}_has_`
 * prefix (underscores → spaces); values resolve by spaced key first,
 * then underscored, then the raw fact-type id.
 */
import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { ViewForm } from '../ViewForm'
import type { ViewProjection } from '../../providers/types'

const view: ViewProjection = {
  view: 'instance-view-Support Request',
  kind: 'instance',
  source: 'synthesized',
  elements: [
    { id: 've_a', factType: 'Support_Request_has_Install_Date', componentRole: 'date-picker' },
    { id: 've_b', factType: 'Support_Request_has_Subject', componentRole: 'text-input' },
    { id: 've_c', factType: 'unprefixed-fact-type', componentRole: 'text-input' },
  ],
}

describe('ViewForm', () => {
  it('derives labels from fact types (spaced-noun prefix strip + underscores to spaces)', () => {
    render(<ViewForm view={view} record={{}} noun="Support Request" />)
    // `Support Request` → prefix `Support_Request_has_`.
    expect(screen.getByText('Install Date')).toBeDefined()
    expect(screen.getByText('Subject')).toBeDefined()
    // No prefix match → raw fact-type id as the label (graceful).
    expect(screen.getByText('unprefixed-fact-type')).toBeDefined()
  })

  it('resolves values by spaced key, underscored key, then raw fact type', () => {
    render(
      <ViewForm
        view={view}
        record={{
          'Install Date': '2026-05-11', // spaced wins
          Subject: 'spaced-miss-underscore-hit',
          'unprefixed-fact-type': 'raw-id-hit',
        }}
        noun="Support Request"
      />,
    )
    expect(
      (screen.getByTestId('view-field-Support_Request_has_Install_Date').querySelector('input') as HTMLInputElement)
        .value,
    ).toBe('2026-05-11')
    expect(
      (screen.getByTestId('view-field-Support_Request_has_Subject').querySelector('input') as HTMLInputElement).value,
    ).toBe('spaced-miss-underscore-hit')
    expect(
      (screen.getByTestId('view-field-unprefixed-fact-type').querySelector('input') as HTMLInputElement).value,
    ).toBe('raw-id-hit')
  })

  it('renders an unchecked disabled checkbox for non-true boolean values', () => {
    const boolView: ViewProjection = {
      view: 'v',
      kind: 'instance',
      source: 'authored',
      elements: [{ id: 've_x', factType: 'Task_has_Done', componentRole: 'checkbox' }],
    }
    render(<ViewForm view={boolView} record={{ Done: 'false' }} noun="Task" />)
    const box = screen.getByTestId('view-field-Task_has_Done').querySelector('input') as HTMLInputElement
    expect(box.type).toBe('checkbox')
    expect(box.checked).toBe(false)
    expect(box.disabled).toBe(true)
  })
})
