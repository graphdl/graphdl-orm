import { afterEach, describe, expect, it, vi } from 'vitest'
import { renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { usePermissions } from '../usePermissions'

const baseUrl = 'https://ui.auto.dev/arest'

function wrap(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

afterEach(() => { vi.unstubAllGlobals() })

describe('usePermissions', () => {
  it('returns permissions flattened from /arest/ _links.organizations', async () => {
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({
      type: 'User',
      id: 'sam@driv.ly',
      data: { email: 'sam@driv.ly' },
      _links: {
        organizations: [
          { href: '/arest/organizations/acme', title: 'Acme', factType: 'User_owns_Organization' },
          { href: '/arest/organizations/globex', title: 'Globex', factType: 'User_belongs_to_Organization' },
        ],
      },
    }), { status: 200, headers: { 'Content-Type': 'application/json' } }))

    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
    const { result } = renderHook(() => usePermissions({ baseUrl }), { wrapper: wrap(client) })

    await waitFor(() => expect(result.current.isLoading).toBe(false))
    expect(result.current.permissions).toEqual([
      'User_owns_Organization:acme',
      'User_belongs_to_Organization:globex',
    ])
    expect(result.current.has('User_owns_Organization:acme')).toBe(true)
    expect(result.current.has('User_owns_Organization:globex')).toBe(false)
    // Wildcard on the fact-type side: matches any acme permission.
    expect(result.current.has('User_*')).toBe(true)
    expect(result.current.has('Admin_*')).toBe(false)
    // `*` alone — "any permission".
    expect(result.current.has('*')).toBe(true)
  })
})
