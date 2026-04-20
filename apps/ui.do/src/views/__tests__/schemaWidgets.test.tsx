/**
 * Unit tests for SchemaInput and SchemaDisplay — the per-kind
 * widget layer that GenericEditView / GenericListView / GenericShowView
 * use to pick input and display components.
 */
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { SchemaInput } from '../schemaInputs'
import { SchemaDisplay } from '../schemaDisplay'
import type { FieldDef } from '../../schema'

function field(overrides: Partial<FieldDef>): FieldDef {
  return {
    name: 'f',
    kind: 'string',
    required: false,
    label: 'F',
    ...overrides,
  }
}

describe('SchemaInput', () => {
  describe('enum widget selection', () => {
    it('renders a radio group when the enum has <= 4 options', () => {
      const onChange = vi.fn()
      render(
        <SchemaInput
          field={field({ kind: 'enum', enum: ['Starter', 'Pro', 'Enterprise'] })}
          value=""
          onChange={onChange}
        />,
      )
      const widget = screen.getByTestId('input-f')
      expect(widget.tagName).toBe('FIELDSET')
      expect(widget.getAttribute('data-widget')).toBe('radio-group')

      fireEvent.click(screen.getByLabelText('Pro'))
      expect(onChange).toHaveBeenCalledWith('Pro')
    })

    it('renders a <select> when the enum has > 4 options', () => {
      render(
        <SchemaInput
          field={field({ kind: 'enum', enum: ['a', 'b', 'c', 'd', 'e'] })}
          value=""
          onChange={vi.fn()}
        />,
      )
      const widget = screen.getByTestId('input-f')
      expect(widget.tagName).toBe('SELECT')
      expect(widget.getAttribute('data-widget')).toBe('select')
    })

    it('respects the enumAsRadioThreshold override', () => {
      render(
        <SchemaInput
          field={field({ kind: 'enum', enum: ['a', 'b'] })}
          value=""
          onChange={vi.fn()}
          enumAsRadioThreshold={1}
        />,
      )
      // Threshold 1 forces a select even for 2 options.
      expect(screen.getByTestId('input-f').tagName).toBe('SELECT')
    })
  })

  describe('numeric widget', () => {
    it('uses <input type="number"> and forwards min/max/step from the schema', () => {
      render(
        <SchemaInput
          field={field({ kind: 'integer', min: 1, max: 99, step: 1 })}
          value={50}
          onChange={vi.fn()}
        />,
      )
      const input = screen.getByTestId('input-f') as HTMLInputElement
      expect(input.type).toBe('number')
      expect(input.min).toBe('1')
      expect(input.max).toBe('99')
      expect(input.step).toBe('1')
    })

    it('defaults integer step to 1 when schema omits multipleOf', () => {
      render(
        <SchemaInput field={field({ kind: 'integer' })} value={0} onChange={vi.fn()} />,
      )
      const input = screen.getByTestId('input-f') as HTMLInputElement
      expect(input.step).toBe('1')
    })

    it('coerces numeric onChange to Number (or null on empty)', () => {
      const onChange = vi.fn()
      render(
        <SchemaInput field={field({ kind: 'number' })} value={0} onChange={onChange} />,
      )
      fireEvent.change(screen.getByTestId('input-f'), { target: { value: '42' } })
      expect(onChange).toHaveBeenLastCalledWith(42)
      fireEvent.change(screen.getByTestId('input-f'), { target: { value: '' } })
      expect(onChange).toHaveBeenLastCalledWith(null)
    })
  })

  describe('iFactr-parallel widgets (textarea / password / slider / switch / time)', () => {
    it('kind=textarea renders <textarea>', () => {
      render(<SchemaInput field={field({ kind: 'textarea' })} value="abc" onChange={vi.fn()} />)
      const w = screen.getByTestId('input-f')
      expect(w.tagName).toBe('TEXTAREA')
      expect(w.getAttribute('data-widget')).toBe('textarea')
    })

    it('kind=password renders <input type="password">', () => {
      render(<SchemaInput field={field({ kind: 'password' })} value="" onChange={vi.fn()} />)
      const w = screen.getByTestId('input-f') as HTMLInputElement
      expect(w.type).toBe('password')
    })

    it('kind=slider renders <input type="range"> with min/max/step', () => {
      const onChange = vi.fn()
      render(
        <SchemaInput
          field={field({ kind: 'slider', min: 0, max: 10, step: 1 })}
          value={5}
          onChange={onChange}
        />,
      )
      const w = screen.getByTestId('input-f') as HTMLInputElement
      expect(w.type).toBe('range')
      expect(w.min).toBe('0')
      expect(w.max).toBe('10')
      expect(w.step).toBe('1')
      fireEvent.change(w, { target: { value: '7' } })
      expect(onChange).toHaveBeenLastCalledWith(7)
    })

    it('kind=switch renders a checkbox with role="switch"', () => {
      render(<SchemaInput field={field({ kind: 'switch' })} value={true} onChange={vi.fn()} />)
      const w = screen.getByTestId('input-f') as HTMLInputElement
      expect(w.type).toBe('checkbox')
      expect(w.getAttribute('role')).toBe('switch')
    })

    it('kind=time renders <input type="time">', () => {
      render(<SchemaInput field={field({ kind: 'time' })} value="09:30" onChange={vi.fn()} />)
      const w = screen.getByTestId('input-f') as HTMLInputElement
      expect(w.type).toBe('time')
    })
  })

  describe('string widgets', () => {
    it('forwards minLength / maxLength / pattern to the HTML5 attrs', () => {
      render(
        <SchemaInput
          field={field({ kind: 'string', minLength: 3, maxLength: 20, pattern: '^[a-z]+$' })}
          value=""
          onChange={vi.fn()}
        />,
      )
      const input = screen.getByTestId('input-f') as HTMLInputElement
      expect(input.minLength).toBe(3)
      expect(input.maxLength).toBe(20)
      expect(input.pattern).toBe('^[a-z]+$')
    })

    it.each([
      ['email', 'email'],
      ['url', 'url'],
      ['date', 'date'],
      ['datetime', 'datetime-local'],
    ] as const)('kind=%s uses <input type="%s">', (kind, type) => {
      render(
        <SchemaInput field={field({ kind })} value="" onChange={vi.fn()} />,
      )
      const input = screen.getByTestId('input-f') as HTMLInputElement
      expect(input.type).toBe(type)
    })
  })
})

describe('SchemaDisplay', () => {
  it('renders numbers with locale formatting', () => {
    render(<SchemaDisplay field={field({ kind: 'number' })} value={1234567} locale="en-US" />)
    expect(screen.getByTestId('display-f').textContent).toBe('1,234,567')
  })

  it('renders integers with locale formatting', () => {
    render(<SchemaDisplay field={field({ kind: 'integer' })} value={1500} locale="en-US" />)
    expect(screen.getByTestId('display-f').textContent).toBe('1,500')
  })

  it('renders dates with Intl.DateTimeFormat', () => {
    render(<SchemaDisplay field={field({ kind: 'date' })} value="2026-04-20" locale="en-US" />)
    // Intl output varies by platform; assert it's human-readable.
    const text = screen.getByTestId('display-f').textContent ?? ''
    expect(text).toMatch(/2026/)
    expect(text).toMatch(/Apr/)
  })

  it('renders booleans as ✓ Yes / ✗ No', () => {
    const { rerender } = render(<SchemaDisplay field={field({ kind: 'boolean' })} value={true} />)
    expect(screen.getByTestId('display-f').textContent).toMatch(/✓/)
    expect(screen.getByTestId('display-f').textContent).toMatch(/Yes/)
    rerender(<SchemaDisplay field={field({ kind: 'boolean' })} value={false} />)
    expect(screen.getByTestId('display-f').textContent).toMatch(/✗/)
    expect(screen.getByTestId('display-f').textContent).toMatch(/No/)
  })

  it('renders emails as mailto: anchors', () => {
    render(<SchemaDisplay field={field({ kind: 'email' })} value="sam@driv.ly" />)
    const link = screen.getByTestId('display-f') as HTMLAnchorElement
    expect(link.tagName).toBe('A')
    expect(link.href).toBe('mailto:sam@driv.ly')
  })

  it('renders urls as external anchors with noopener noreferrer', () => {
    render(<SchemaDisplay field={field({ kind: 'url' })} value="https://example.com" />)
    const link = screen.getByTestId('display-f') as HTMLAnchorElement
    expect(link.tagName).toBe('A')
    expect(link.target).toBe('_blank')
    expect(link.rel).toContain('noopener')
    expect(link.rel).toContain('noreferrer')
  })

  it('renders nullish values as an em-dash', () => {
    render(<SchemaDisplay field={field({ kind: 'string' })} value={null} />)
    expect(screen.getByTestId('display-f').textContent).toBe('—')
  })
})
