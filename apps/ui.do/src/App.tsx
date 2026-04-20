import { useEffect, useMemo, useState } from 'react'
import type { ReactElement } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import { AREST_BASE_URL } from './env'
import {
  createArestAuthProvider,
  createArestDataProvider,
  createArestNavigationProvider,
} from './providers'
import {
  createArestQueryBridge,
  createArestQueryClient,
} from './query'

/**
 * App shell — owns the QueryClient, the providers, and the
 * SSE-to-cache bridge. The bridge is started on mount and torn down
 * on unmount so hot-reload doesn't leak EventSource connections.
 *
 * The domain defaults to "organizations" — AREST's bootstrap domain
 * (see src/api/router.ts handleArestRoute). A user-facing selector
 * can override this once the first resource views land.
 */
export function App(): ReactElement {
  // QueryClient and providers are stable for the app's lifetime.
  const queryClient = useMemo(() => createArestQueryClient(), [])
  const providers = useMemo(
    () => ({
      data: createArestDataProvider({ baseUrl: AREST_BASE_URL }),
      auth: createArestAuthProvider({ baseUrl: AREST_BASE_URL }),
      navigation: createArestNavigationProvider({ baseUrl: AREST_BASE_URL }),
    }),
    [],
  )

  const [domain] = useState('organizations')

  useEffect(() => {
    // Only open the SSE bridge when EventSource is available (browser
    // runtime). SSR / build-time has no such global, so we skip.
    if (typeof globalThis.EventSource !== 'function') return undefined

    const bridge = createArestQueryBridge({
      baseUrl: AREST_BASE_URL,
      domain,
      queryClient,
    })
    return () => bridge.close()
  }, [queryClient, domain])

  return (
    <QueryClientProvider client={queryClient}>
      <main style={{ fontFamily: 'system-ui, sans-serif', padding: '2rem' }}>
        <h1>ui.do</h1>
        <p>AREST front-end, talking to <code>{AREST_BASE_URL}</code>.</p>
        <p>
          Providers wired (data / auth / navigation); TanStack Query +
          SSE cache-invalidation bridge active on domain{' '}
          <code>{domain}</code>.
        </p>
        <p>
          Resource views wire into these providers + useQuery hooks in
          the next round; all infrastructure for #121 / #122 / #123 is
          landed.
        </p>
        {/* The providers are exposed on the module for dev-console pokes
            until the resource views wire into them directly. */}
        <ProvidersDebug providers={providers} />
      </main>
    </QueryClientProvider>
  )
}

interface DebugProps {
  providers: {
    data: ReturnType<typeof createArestDataProvider>
    auth: ReturnType<typeof createArestAuthProvider>
    navigation: ReturnType<typeof createArestNavigationProvider>
  }
}

function ProvidersDebug({ providers }: DebugProps): ReactElement {
  // Stash on window in dev so a person can poke at providers.data.getList('...')
  // from the console. No-op in SSR.
  useEffect(() => {
    if (typeof window === 'undefined') return
    ;(window as unknown as { __arest__?: unknown }).__arest__ = providers
  }, [providers])

  return (
    <details style={{ marginTop: '2rem' }}>
      <summary>providers (dev)</summary>
      <ul>
        <li><code>window.__arest__.data</code> — DataProvider</li>
        <li><code>window.__arest__.auth</code> — AuthProvider</li>
        <li><code>window.__arest__.navigation</code> — NavigationProvider</li>
      </ul>
    </details>
  )
}

export default App
