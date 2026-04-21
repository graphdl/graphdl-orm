/**
 * StateMachineEditor tests. Stubs the data-provider endpoints:
 *   GET    /arest/state-machine-definitions/Order
 *   GET    /arest/transitions?filter[stateMachineDefinition]=Order
 *   POST   /arest/transitions
 *   PATCH  /arest/transitions/<id>
 *   DELETE /arest/transitions/<id>
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { StateMachineEditor } from '../StateMachineEditor'

const baseUrl = 'https://ui.auto.dev/arest'

interface Recorded { url: string; method: string; body?: unknown }

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
  return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

const smdRespond = () => json({
  data: { id: 'Order', noun: 'Order', initial: 'In Cart' },
  _links: {},
})

const transitionsRespond = () => json({
  data: [
    { id: 'place', from: 'In Cart', to: 'Placed' },
    { id: 'ship',  from: 'Placed',  to: 'Shipped' },
  ],
  _links: {},
})

describe('StateMachineEditor', () => {
  it('renders states with initial / terminal markers and outgoing transitions', async () => {
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.url.includes('/arest/transitions')) return transitionsRespond()
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} />))

    await waitFor(() => expect(screen.getByTestId('state-machine-editor')).toBeDefined())
    expect(screen.getByTestId('sm-initial').textContent).toBe('In Cart')

    // In Cart is initial; Shipped is terminal (no outgoing transition
    // after just place + ship), Placed is intermediate.
    const inCart = screen.getByTestId('sm-state-In Cart')
    expect(inCart.getAttribute('data-initial')).toBe('true')
    expect(inCart.getAttribute('data-terminal')).toBeNull()
    expect(screen.getByTestId('sm-state-Shipped').getAttribute('data-terminal')).toBe('true')

    // Each transition is rendered with event + target.
    expect(screen.getByTestId('sm-transition-place').textContent).toContain('place')
    expect(screen.getByTestId('sm-transition-place').textContent).toContain('Placed')
  })

  it('emits a valid xstate config via onConfigChange', async () => {
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.url.includes('/arest/transitions')) return transitionsRespond()
      return json({}, 404)
    })

    const seen: Array<{ initial: string }> = []
    render(wrap(
      <StateMachineEditor
        smdId="Order"
        baseUrl={baseUrl}
        onConfigChange={(c) => seen.push(c)}
      />,
    ))
    await waitFor(() => expect(seen.length).toBeGreaterThan(0))
    const last = seen[seen.length - 1]
    expect(last.initial).toBe('In Cart')
  })

  it('POSTs to /arest/transitions when the user submits the add form', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.method === 'GET' && req.url.includes('/arest/transitions')) return transitionsRespond()
      if (req.method === 'POST') return json({ data: { id: 'cancel', from: 'Placed', to: 'Cancelled' } }, 201)
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('sm-add-form')).toBeDefined())

    fireEvent.change(screen.getByTestId('sm-input-id'),   { target: { value: 'cancel' } })
    fireEvent.change(screen.getByTestId('sm-input-from'), { target: { value: 'Placed' } })
    fireEvent.change(screen.getByTestId('sm-input-to'),   { target: { value: 'Cancelled' } })

    await act(async () => {
      fireEvent.submit(screen.getByTestId('sm-add-form'))
    })

    const post = recorded.find((r) => r.method === 'POST')
    expect(post).toBeDefined()
    expect(post!.url).toBe('https://ui.auto.dev/arest/transitions')
    const body = post!.body as { id: string; from: string; to: string; stateMachineDefinition: string }
    expect(body.id).toBe('cancel')
    expect(body.from).toBe('Placed')
    expect(body.to).toBe('Cancelled')
    expect(body.stateMachineDefinition).toBe('Order')
  })

  it('DELETEs a transition when the delete button is clicked', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.method === 'GET' && req.url.includes('/arest/transitions')) return transitionsRespond()
      if (req.method === 'DELETE') return json({ data: { id: 'ship' } })
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('sm-delete-ship')).toBeDefined())

    await act(async () => {
      fireEvent.click(screen.getByTestId('sm-delete-ship'))
    })

    const del = recorded.find((r) => r.method === 'DELETE')
    expect(del).toBeDefined()
    expect(del!.url).toBe('https://ui.auto.dev/arest/transitions/ship')
  })

  it('opens an inline edit form and PATCHes on save', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.method === 'GET' && req.url.includes('/arest/transitions')) return transitionsRespond()
      if (req.method === 'PATCH') return json({ data: { id: 'ship', from: 'Placed', to: 'Delivered' } })
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('sm-edit-ship')).toBeDefined())

    fireEvent.click(screen.getByTestId('sm-edit-ship'))
    await waitFor(() => expect(screen.getByTestId('sm-transition-edit-ship')).toBeDefined())

    // Scope query to the edit form — the add form also has an
    // sm-input-to field, and a raw getByTestId would be ambiguous.
    const editForm = within(screen.getByTestId('sm-transition-edit-ship'))
    fireEvent.change(editForm.getByTestId('sm-input-to'), { target: { value: 'Delivered' } })

    await act(async () => {
      fireEvent.click(editForm.getByTestId('sm-form-submit'))
    })

    const patch = recorded.find((r) => r.method === 'PATCH')
    expect(patch).toBeDefined()
    expect(patch!.url).toBe('https://ui.auto.dev/arest/transitions/ship')
    expect((patch!.body as { to?: string }).to).toBe('Delivered')
  })

  it('highlights the current state when currentStatus prop is set', async () => {
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.url.includes('/arest/transitions')) return transitionsRespond()
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} currentStatus="Placed" />))
    await waitFor(() => expect(screen.getByTestId('sm-current')).toBeDefined())
    expect(screen.getByTestId('sm-current').textContent).toBe('Placed')
    expect(screen.getByTestId('sm-state-Placed').getAttribute('data-current')).toBe('true')
    expect(screen.getByTestId('sm-state-In Cart').getAttribute('data-current')).toBeNull()
  })

  it('surfaces a Stately Studio deeplink', async () => {
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.url.includes('/arest/transitions')) return transitionsRespond()
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('sm-stately-deeplink')).toBeDefined())
    const href = (screen.getByTestId('sm-stately-deeplink') as HTMLAnchorElement).href
    expect(href.startsWith('https://stately.ai/viz?machine=')).toBe(true)
  })

  it('warns when a cycle has no exit transition', async () => {
    // Override the transitions response with a dead cycle: A↔B with
    // no exit.
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Loop')) {
        return json({ data: { id: 'Loop', noun: 'Loop', initial: 'A' }, _links: {} })
      }
      if (req.url.includes('/arest/transitions')) {
        return json({
          data: [
            { id: 'a-to-b', from: 'A', to: 'B' },
            { id: 'b-to-a', from: 'B', to: 'A' },
          ],
          _links: {},
        })
      }
      return json({}, 404)
    })
    render(wrap(<StateMachineEditor smdId="Loop" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('sm-dead-cycle-warning')).toBeDefined())
    expect(screen.getByTestId('sm-dead-cycle-warning').textContent).toMatch(/A/)
    expect(screen.getByTestId('sm-dead-cycle-warning').textContent).toMatch(/B/)
  })

  it('hides edit / delete / add-form when readOnly=true', async () => {
    stubFetch((req) => {
      if (req.url.includes('/arest/state-machine-definitions/Order')) return smdRespond()
      if (req.url.includes('/arest/transitions')) return transitionsRespond()
      return json({}, 404)
    })

    render(wrap(<StateMachineEditor smdId="Order" baseUrl={baseUrl} readOnly />))
    await waitFor(() => expect(screen.getByTestId('sm-transition-place')).toBeDefined())
    expect(screen.queryByTestId('sm-edit-place')).toBeNull()
    expect(screen.queryByTestId('sm-delete-place')).toBeNull()
    expect(screen.queryByTestId('sm-add-form')).toBeNull()
  })
})
