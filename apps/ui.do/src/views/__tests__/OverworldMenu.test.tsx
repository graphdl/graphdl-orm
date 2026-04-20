/**
 * OverworldMenu / EntityOverworldMenu tests.
 *
 * The overworld idiom surfaces every next-step option (actions and
 * nav) derivable from the entity's current state. Tests assert both
 * halves and the integration shape (useActions + useEntityLinks).
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { EntityOverworldMenu, OverworldMenu } from '../OverworldMenu'

const baseUrl = 'https://ui.auto.dev/arest'

interface Recorded {
  url: string
  method: string
  body?: unknown
}

function stubFetch(handler: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    let body: unknown
    if (init?.body != null) {
      try { body = JSON.parse(init.body as string) } catch { body = init.body }
    }
    const req = { url, method, body }
    recorded.push(req)
    return handler(req)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

describe('OverworldMenu (presentational)', () => {
  it('renders action and nav sections with per-item buttons', () => {
    render(
      <OverworldMenu
        title="Order ord-1"
        status="Placed"
        sections={[
          { label: 'Actions', items: [{ kind: 'action', name: 'ship', label: 'Ship', to: 'Shipped' }] },
          { label: 'Go to', items: [{ kind: 'nav', rel: 'customer', href: '/arest/customers/acme', label: 'Acme' }] },
        ]}
      />,
    )

    expect(screen.getByTestId('overworld-menu')).toBeDefined()
    expect(screen.getByTestId('overworld-status').textContent).toContain('Placed')
    expect(screen.getByTestId('overworld-action-ship').textContent).toContain('Ship')
    expect(screen.getByTestId('overworld-action-ship').textContent).toContain('Shipped')
    expect(screen.getByTestId('overworld-nav-customer').textContent).toContain('Acme')
  })

  it('invokes onAction when an action button is clicked', async () => {
    const onAction = vi.fn()
    render(
      <OverworldMenu
        sections={[{ label: 'Actions', items: [{ kind: 'action', name: 'cancel', label: 'Cancel' }] }]}
        onAction={onAction}
      />,
    )
    fireEvent.click(screen.getByTestId('overworld-action-cancel'))
    expect(onAction).toHaveBeenCalledWith('cancel')
  })

  it('invokes onNavigate when a nav button is clicked', () => {
    const onNavigate = vi.fn()
    render(
      <OverworldMenu
        sections={[{ label: 'Go to', items: [{ kind: 'nav', rel: 'parent', href: '/arest/organizations/acme', label: 'Acme' }] }]}
        onNavigate={onNavigate}
      />,
    )
    fireEvent.click(screen.getByTestId('overworld-nav-parent'))
    expect(onNavigate).toHaveBeenCalledWith('/arest/organizations/acme')
  })

  it('hides empty sections to keep the overworld compact', () => {
    render(
      <OverworldMenu
        sections={[
          { label: 'Actions', items: [] },
          { label: 'Go to', items: [{ kind: 'nav', rel: 'x', href: '/arest/x', label: 'X' }] },
        ]}
      />,
    )
    expect(screen.queryByTestId('overworld-section-actions')).toBeNull()
    expect(screen.getByTestId('overworld-section-go-to')).toBeDefined()
  })
})

describe('EntityOverworldMenu (composed)', () => {
  it('wires useActions + useEntityLinks into a single menu for the entity', async () => {
    stubFetch((req) => {
      if (req.url.endsWith('/actions')) {
        return json({
          data: {
            entity: 'ord-1',
            noun: 'Order',
            status: 'Placed',
            transitions: [
              { event: 'ship', targetStatus: 'Shipped' },
              { event: 'cancel', targetStatus: 'Cancelled', label: 'Cancel order' },
            ],
          },
        })
      }
      // GET /arest/orders/ord-1 — entity body with HATEOAS links
      return json({
        data: { id: 'ord-1', total: 99 },
        _links: {
          self: { href: '/arest/orders/ord-1' },
          customer: { href: '/arest/customers/acme', title: 'Acme', factType: 'Order_by_Customer' },
          'line-items': [
            { href: '/arest/orders/ord-1/line-items/1', title: 'Line 1' },
            { href: '/arest/orders/ord-1/line-items/2', title: 'Line 2' },
          ],
        },
      })
    })

    const onNavigate = vi.fn()
    render(wrap(<EntityOverworldMenu noun="Order" id="ord-1" baseUrl={baseUrl} onNavigate={onNavigate} />))

    // Actions section pulls from /actions.
    await waitFor(() => expect(screen.getByTestId('overworld-action-ship')).toBeDefined())
    expect(screen.getByTestId('overworld-action-cancel').textContent).toContain('Cancel order')

    // Nav section pulls from _links on the entity body. Array-form
    // (line-items) produces one button per href.
    expect(screen.getByTestId('overworld-nav-customer').textContent).toContain('Acme')
    // Multiple line-items → all rendered under the same rel key.
    const lineButtons = screen.getAllByTestId('overworld-nav-line-items')
    expect(lineButtons).toHaveLength(2)

    fireEvent.click(screen.getByTestId('overworld-nav-customer'))
    expect(onNavigate).toHaveBeenCalledWith('/arest/customers/acme')
  })

  it('dispatches the clicked action through useActions', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.endsWith('/actions')) {
        return json({ data: { transitions: [{ event: 'place', targetStatus: 'Placed' }] } })
      }
      if (req.method === 'POST') {
        return json({ data: { id: 'ord-1', status: 'Placed' } })
      }
      // /arest/orders/ord-1 entity
      return json({ data: { id: 'ord-1' }, _links: {} })
    })

    render(wrap(<EntityOverworldMenu noun="Order" id="ord-1" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('overworld-action-place')).toBeDefined())

    await act(async () => {
      fireEvent.click(screen.getByTestId('overworld-action-place'))
    })

    const post = recorded.find((r) => r.method === 'POST')
    expect(post).toBeDefined()
    expect(post!.url).toBe('https://ui.auto.dev/arest/orders/ord-1/place')
  })
})
