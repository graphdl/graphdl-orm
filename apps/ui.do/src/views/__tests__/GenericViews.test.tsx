/**
 * Generic schema-driven views — integration tests.
 *
 * Stubs globalThis.fetch to respond to the two endpoints the views
 * hit: /api/openapi.json?app=<name> (schema) and /arest/{slug}[...]
 * (data). Each test renders inside a QueryClientProvider with no
 * retries so failures surface immediately.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'

// Mock @mdxui/admin's container primitives so tests don't drag in the
// full dist graph (which includes @emoji-mart/data, a JSON module that
// trips Node ESM's strict JSON import attribute). The real layout wins
// at build time; tests assert the children — which is what the
// schema-driven work is actually producing.
vi.mock('@mdxui/admin', () => ({
  ListView: ({ title, actions, loading, empty, pagination, children }: any) =>
    (
      <div data-testid="list-container">
        {title && <h1>{title}</h1>}
        {actions}
        {loading ? <span>Loading…</span> : (children || empty)}
        {pagination}
      </div>
    ),
  ShowView: ({ title, loading, children, actions, aside }: any) => (
    <div data-testid="show-container">
      {title && <h1>{title}</h1>}
      {actions}
      {loading ? <span>Loading…</span> : children}
      {aside}
    </div>
  ),
  EditView: ({ title, loading, children }: any) => (
    <div data-testid="edit-container">
      {title && <h1>{title}</h1>}
      {loading ? <span>Loading…</span> : children}
    </div>
  ),
  CreateView: ({ title, children }: any) => (
    <div data-testid="create-container">
      {title && <h1>{title}</h1>}
      {children}
    </div>
  ),
}))

import { GenericListView } from '../GenericListView'
import { GenericShowView } from '../GenericShowView'
import { GenericEditView } from '../GenericEditView'
import { GenericCreateView } from '../GenericCreateView'

const baseUrl = 'https://ui.auto.dev/arest'

const openapi = {
  openapi: '3.1.0',
  components: {
    schemas: {
      Organization: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string', title: 'Legal Name' },
          tier: { type: 'string', enum: ['Starter', 'Pro', 'Enterprise'] },
          active: { type: 'boolean' },
        },
        required: ['name'],
      },
    },
  },
}

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

describe('GenericListView', () => {
  it('renders schema-derived headers and row cells for /arest/{slug}', async () => {
    stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      // /arest/organizations collection
      return json({
        data: [
          { id: 'acme', name: 'Acme', tier: 'Pro', active: true },
          { id: 'globex', name: 'Globex', tier: 'Starter', active: false },
        ],
        _links: {},
      })
    })

    render(wrap(<GenericListView noun="Organization" baseUrl={baseUrl} />))

    await waitFor(() => expect(screen.getByTestId('generic-list-table')).toBeDefined())

    // "Legal Name" title wins over humanize("name")
    expect(screen.getByText('Legal Name')).toBeDefined()
    expect(screen.getByText('Tier')).toBeDefined()
    expect(screen.getByText('Active')).toBeDefined()
    expect(screen.getByText('Acme')).toBeDefined()
    expect(screen.getByText('Globex')).toBeDefined()
    // Boolean values render via SchemaDisplay as ✓ Yes / ✗ No.
    expect(screen.getByText(/Yes/)).toBeDefined()
    expect(screen.getByText(/No/)).toBeDefined()
  })

  it('shows the empty state when the collection is empty', async () => {
    stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      return json({ data: [], _links: {} })
    })

    render(wrap(<GenericListView noun="Organization" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeDefined())
  })
})

describe('GenericShowView', () => {
  it('renders <dl> with schema-labeled field/value pairs', async () => {
    stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      // /arest/organizations/acme
      return json({
        data: { id: 'acme', name: 'Acme', tier: 'Pro', active: true },
        _links: {},
      })
    })

    render(wrap(<GenericShowView noun="Organization" id="acme" baseUrl={baseUrl} />))

    await waitFor(() => expect(screen.getByTestId('generic-show-dl')).toBeDefined())
    expect(screen.getByTestId('field-name').textContent).toBe('Acme')
    expect(screen.getByTestId('field-tier').textContent).toBe('Pro')
    // SchemaDisplay: booleans get a ✓ / ✗ glyph.
    expect(screen.getByTestId('field-active').textContent).toMatch(/Yes/)
  })
})

describe('GenericEditView', () => {
  it('hydrates inputs from the fetched record and PATCHes on submit', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      if (req.method === 'GET') {
        return json({
          data: { id: 'acme', name: 'Acme', tier: 'Pro', active: true },
          _links: {},
        })
      }
      // PATCH
      return json({
        data: { id: 'acme', name: 'Acme Rebrand', tier: 'Enterprise', active: true },
        _links: {},
      })
    })

    let saved: Record<string, unknown> | null = null
    render(wrap(<GenericEditView
      noun="Organization"
      id="acme"
      baseUrl={baseUrl}
      onSaved={(next) => { saved = next }}
    />))

    await waitFor(() => expect(screen.getByTestId('input-name')).toBeDefined())

    const nameInput = screen.getByTestId('input-name') as HTMLInputElement
    expect(nameInput.value).toBe('Acme')
    // Tier has 3 options (Starter/Pro/Enterprise) — below the default
    // radio-vs-dropdown threshold of 4, so it renders as a radio group.
    const tierGroup = screen.getByTestId('input-tier')
    expect(tierGroup.tagName).toBe('FIELDSET')
    expect(tierGroup.getAttribute('data-widget')).toBe('radio-group')

    // Change name and submit — fireEvent.change drives React's synthetic
    // handler correctly (direct .value mutation bypasses the React setter).
    await act(async () => {
      fireEvent.change(nameInput, { target: { value: 'Acme Rebrand' } })
    })

    const form = screen.getByTestId('generic-edit-form') as HTMLFormElement
    await act(async () => {
      fireEvent.submit(form)
    })

    await waitFor(() => expect(saved).not.toBeNull())

    const patch = recorded.find((r) => r.method === 'PATCH')
    expect(patch).toBeDefined()
    expect(patch!.url).toBe('https://ui.auto.dev/arest/organizations/acme')
    expect((patch!.body as { name?: string }).name).toBe('Acme Rebrand')
  })
})

describe('GenericCreateView', () => {
  it('POSTs /arest/{slug} with schema-driven form values', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      return json({ data: { id: 'new-org', name: 'New Org', tier: 'Starter', active: true }, _links: {} }, 201)
    })

    let created: Record<string, unknown> | null = null
    render(wrap(<GenericCreateView
      noun="Organization"
      baseUrl={baseUrl}
      onCreated={(r) => { created = r }}
    />))

    await waitFor(() => expect(screen.getByTestId('input-name')).toBeDefined())

    const nameInput = screen.getByTestId('input-name') as HTMLInputElement
    await act(async () => {
      fireEvent.change(nameInput, { target: { value: 'New Org' } })
    })

    const form = screen.getByTestId('generic-create-form') as HTMLFormElement
    await act(async () => {
      fireEvent.submit(form)
    })

    await waitFor(() => expect(created).not.toBeNull())
    const post = recorded.find((r) => r.method === 'POST')
    expect(post!.url).toBe('https://ui.auto.dev/arest/organizations')
    expect((post!.body as { name?: string }).name).toBe('New Org')
  })

  it('surfaces a 422 violation reading as the form error', async () => {
    stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(openapi)
      return json({
        data: null,
        violations: [{
          reading: 'Each Organization has exactly one legal name.',
          constraintId: 'uc-legal',
          modality: 'alethic',
          detail: 'name is required',
        }],
      }, 422)
    })

    render(wrap(<GenericCreateView noun="Organization" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('generic-create-form')).toBeDefined())

    const form = screen.getByTestId('generic-create-form') as HTMLFormElement
    await act(async () => {
      fireEvent.submit(form)
    })

    await waitFor(() => expect(screen.getByTestId('create-error').textContent).toMatch(/name is required/))
  })
})
