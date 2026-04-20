/**
 * ArestAppShell tests.
 *
 * Mocks @mdxui/app's AppShell (and @mdxui/admin's useCurrentResource
 * and container primitives, consumed transitively by the Overworld
 * menu's composed ResourceDefinition-style rendering). The test
 * exercises the wiring, not the styling.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'

vi.mock('@mdxui/app', () => ({
  AppShell: ({ config, navigation, user, pageHeader, nav, footer, children, isLoading }: any) => (
    <div data-testid="app-shell">
      <header data-testid="app-shell-header">
        <span data-testid="app-shell-name">{config.name}</span>
        {user && <span data-testid="app-shell-user">{user.fullName ?? user.email ?? user.id}</span>}
      </header>
      <aside data-testid="app-shell-sidebar">
        <div data-testid="app-shell-nav-groups">
          {navigation?.map((g: any, i: number) => (
            <div key={i} data-testid={`nav-group-${g.label.toLowerCase()}`}>{g.label}: {g.items.length}</div>
          ))}
        </div>
        <div data-testid="app-shell-nav">{nav}</div>
        <div data-testid="app-shell-footer">{footer}</div>
      </aside>
      <div data-testid="page-header">{pageHeader}</div>
      <main data-testid="app-shell-main">
        {isLoading ? <span>shell-loading</span> : children}
      </main>
    </div>
  ),
}))

vi.mock('@mdxui/admin', () => ({
  useCurrentResource: () => ({ resourceName: undefined, recordId: undefined, action: undefined }),
}))

import { ArestAppShell } from '../ArestAppShell'

const baseUrl = 'https://ui.auto.dev/arest'

const sampleDoc = {
  openapi: '3.1.0',
  components: {
    schemas: {
      Organization: {
        type: 'object',
        properties: { id: { type: 'string' }, name: { type: 'string' } },
      },
      SupportRequest: {
        type: 'object',
        properties: { id: { type: 'string' }, title: { type: 'string' } },
      },
    },
  },
}

function stubFetch(responder: (url: string) => Response): string[] {
  const urls: string[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    urls.push(url)
    return responder(url)
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

describe('ArestAppShell', () => {
  it('renders config, user, and the main content area', async () => {
    stubFetch(() => json(sampleDoc))

    render(wrap(
      <ArestAppShell
        baseUrl={baseUrl}
        app="ui.do"
        config={{ name: 'ui.do' }}
        user={{ id: 'sam@driv.ly', email: 'sam@driv.ly' }}
      >
        <p data-testid="main-content">Hello</p>
      </ArestAppShell>,
    ))

    await waitFor(() => expect(screen.getByTestId('app-shell-name').textContent).toBe('ui.do'))
    expect(screen.getByTestId('app-shell-user').textContent).toBe('sam@driv.ly')
    // Content renders only once isLoading flips false — wait for it.
    await waitFor(() => expect(screen.getByTestId('main-content').textContent).toBe('Hello'))
  })

  it('derives sidebar nav items from useArestResources', async () => {
    stubFetch(() => json(sampleDoc))
    render(wrap(
      <ArestAppShell baseUrl={baseUrl} config={{ name: 'ui.do' }}>
        <p>content</p>
      </ArestAppShell>,
    ))

    await waitFor(() => expect(screen.getByTestId('nav-group-resources').textContent).toContain('2'))
    // Links render as anchors with testids derived from their URL.
    expect(screen.getByTestId('nav-/organizations')).toBeDefined()
    expect(screen.getByTestId('nav-/support-requests')).toBeDefined()
  })

  it('respects basePath when building nav URLs', async () => {
    stubFetch(() => json(sampleDoc))
    render(wrap(
      <ArestAppShell baseUrl={baseUrl} config={{ name: 'ui.do', basePath: '/admin' }}>
        <p>content</p>
      </ArestAppShell>,
    ))
    await waitFor(() => expect(screen.getByTestId('nav-/admin/organizations')).toBeDefined())
  })

  it('renders the EntityOverworldMenu in the footer when currentEntity is provided', async () => {
    // Two endpoints are hit when the overworld is rendered: /actions
    // (useActions) and /arest/slug/id (useEntityLinks). Both get empty
    // valid responses so we can assert the component renders without
    // throwing.
    stubFetch((url) => {
      if (url.includes('/api/openapi.json')) return json(sampleDoc)
      if (url.endsWith('/actions')) return json({ data: { transitions: [] } })
      return json({ data: {}, _links: {} })
    })

    render(wrap(
      <ArestAppShell
        baseUrl={baseUrl}
        config={{ name: 'ui.do' }}
        currentEntity={{ noun: 'Organization', id: 'acme' }}
      >
        <p>content</p>
      </ArestAppShell>,
    ))

    await waitFor(() => expect(screen.getByTestId('overworld-menu')).toBeDefined())
  })

  it('forwards a caller-supplied pageHeader (breadcrumbs slot)', async () => {
    stubFetch(() => json(sampleDoc))
    render(wrap(
      <ArestAppShell
        baseUrl={baseUrl}
        config={{ name: 'ui.do' }}
        pageHeader={<span data-testid="crumbs">Home / Orgs / Acme</span>}
      >
        <p>content</p>
      </ArestAppShell>,
    ))
    await waitFor(() => expect(screen.getByTestId('crumbs')).toBeDefined())
  })
})
