import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { ReferenceLabel, ReferencePicker } from '../ReferencePicker'

const baseUrl = 'https://ui.auto.dev/arest'

function stubFetch(handler: (url: string) => Response): string[] {
  const urls: string[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    urls.push(url)
    return handler(url)
  })
  return urls
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

describe('ReferencePicker', () => {
  it('GETs /arest/{slug} for the referenced noun and renders options', async () => {
    const urls = stubFetch(() => json({
      data: [
        { id: 'acme', name: 'Acme Corp' },
        { id: 'globex', name: 'Globex' },
      ],
      _links: {},
    }))

    const onChange = vi.fn()
    render(wrap(
      <ReferencePicker
        noun="Organization"
        value=""
        onChange={onChange}
        baseUrl={baseUrl}
        testId="picker"
      />,
    ))

    await waitFor(() => {
      const opts = (screen.getByTestId('picker') as HTMLSelectElement).options
      expect(opts.length).toBeGreaterThan(1)
    })

    expect(urls[0]).toContain('/arest/organizations')

    const select = screen.getByTestId('picker') as HTMLSelectElement
    expect(select.options[0].value).toBe('') // "— none —"
    expect(select.options[1].textContent).toBe('Acme Corp')
    expect(select.options[2].textContent).toBe('Globex')

    fireEvent.change(select, { target: { value: 'globex' } })
    expect(onChange).toHaveBeenCalledWith('globex')
  })

  it('falls back to id when no label field matches', async () => {
    stubFetch(() => json({ data: [{ id: 'x' }], _links: {} }))
    render(wrap(
      <ReferencePicker noun="Thing" value="" onChange={vi.fn()} baseUrl={baseUrl} testId="picker" />,
    ))
    await waitFor(() => {
      const opts = (screen.getByTestId('picker') as HTMLSelectElement).options
      expect(opts.length).toBeGreaterThan(1)
    })
    expect((screen.getByTestId('picker') as HTMLSelectElement).options[1].textContent).toBe('x')
  })
})

describe('ReferenceLabel', () => {
  it('fetches the entity and renders its name', async () => {
    stubFetch(() => json({
      id: 'acme',
      type: 'Organization',
      data: { name: 'Acme Corp' },
      _links: {},
    }))
    render(wrap(<ReferenceLabel noun="Organization" id="acme" baseUrl={baseUrl} testId="label" />))
    await waitFor(() => expect(screen.getByTestId('label').textContent).toBe('Acme Corp'))
  })
})
