/**
 * OutcomesFeed tests — verify the widgets route to
 * /arest/violations and /arest/failures, group rows by Severity /
 * FailureType, and render an empty-state message when the feed is
 * clear.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { FailuresFeed, OutcomesBoard, ViolationsFeed } from '../OutcomesFeed'

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

describe('ViolationsFeed', () => {
  it('fetches /arest/violations and groups rows by severity', async () => {
    const urls = stubFetch(() => json({
      data: [
        { id: 'v1', severity: 'error',   constraintId: 'c1', text: 'required missing', timestamp: '2026-04-20T10:00:00Z' },
        { id: 'v2', severity: 'warning', constraintId: 'c2', text: 'weak link',        timestamp: '2026-04-20T09:00:00Z' },
        { id: 'v3', severity: 'error',   constraintId: 'c3', text: 'cycle',            timestamp: '2026-04-20T11:00:00Z' },
        { id: 'v4', severity: 'info',    constraintId: 'c4', text: 'note',             timestamp: '2026-04-20T12:00:00Z' },
      ],
      _links: {},
    }))

    render(wrap(<ViolationsFeed baseUrl={baseUrl} />))
    // Wait for the severity groups to render — passing the loading state.
    await waitFor(() => expect(screen.getByTestId('violations-error')).toBeDefined())

    expect(urls[0]).toBe('https://ui.auto.dev/arest/violations')

    expect(screen.getByTestId('violations-warning')).toBeDefined()
    expect(screen.getByTestId('violations-info')).toBeDefined()

    // Newest-first within each severity bucket.
    const errorRows = screen.getByTestId('violations-error').querySelectorAll('li')
    expect(errorRows[0].textContent).toContain('c3') // 11:00 UTC
    expect(errorRows[1].textContent).toContain('c1') // 10:00 UTC
  })

  it('shows an empty-state message when no violations are recorded', async () => {
    stubFetch(() => json({ data: [], _links: {} }))
    render(wrap(<ViolationsFeed baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('violations-empty')).toBeDefined())
    expect(screen.getByTestId('violations-empty').textContent).toMatch(/every path is a valid claim/i)
  })

  it('forwards a domain filter to /arest/violations as filter[belongs to Domain]', async () => {
    const urls = stubFetch(() => json({ data: [], _links: {} }))
    render(wrap(<ViolationsFeed baseUrl={baseUrl} domain="organizations" />))
    await waitFor(() => expect(urls).toHaveLength(1))
    const url = new URL(urls[0])
    expect(url.searchParams.get('filter[belongs to Domain]')).toBe('organizations')
  })
})

describe('FailuresFeed', () => {
  it('fetches /arest/failures and groups rows by failureType', async () => {
    const urls = stubFetch(() => json({
      data: [
        { id: 'f1', failureType: 'parse',       reasonText: 'could not parse input'      },
        { id: 'f2', failureType: 'transition',  reasonText: 'guard rejected event'        },
        { id: 'f3', failureType: 'extraction',  reasonText: 'field not present in source' },
      ],
      _links: {},
    }))
    render(wrap(<FailuresFeed baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('failures-parse')).toBeDefined())
    expect(urls[0]).toBe('https://ui.auto.dev/arest/failures')

    expect(screen.getByTestId('failures-extraction')).toBeDefined()
    expect(screen.getByTestId('failures-transition')).toBeDefined()
  })
})

describe('OutcomesBoard', () => {
  it('renders both feeds side by side', async () => {
    stubFetch((url) => {
      if (url.includes('/arest/violations')) return json({ data: [{ id: 'v1', severity: 'error', constraintId: 'c1' }], _links: {} })
      if (url.includes('/arest/failures'))   return json({ data: [{ id: 'f1', failureType: 'parse', reasonText: 'x' }], _links: {} })
      return json({ data: [], _links: {} })
    })
    render(wrap(<OutcomesBoard baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('outcomes-board')).toBeDefined())
    expect(screen.getByTestId('violations-feed')).toBeDefined()
    expect(screen.getByTestId('failures-feed')).toBeDefined()
  })
})
