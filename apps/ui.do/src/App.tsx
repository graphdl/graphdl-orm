import { useEffect, useMemo, useState, type ReactElement } from 'react'
import { BrowserRouter } from 'react-router-dom'
import { QueryClientProvider } from '@tanstack/react-query'
import { Admin, AdminRouter, Resource } from '@mdxui/admin'
import { AREST_BASE_URL } from './env'
import { createArestAuthProvider, createArestDataProvider, createArestNavigationProvider } from './providers'
import { createArestQueryBridge, createArestQueryClient } from './query'
import { ArestAppShell } from './shell'
import { useBranding } from './branding'
import { useArestResources } from './resources'

/**
 * Production shell for ui.do / support.auto.dev.
 *
 * Layers (outer to inner):
 *   QueryClientProvider   — TanStack Query cache + SSE bridge
 *     BrowserRouter       — react-router v6 for AdminRouter
 *       ArestAppShell     — sidebar / nav / user / breadcrumbs /
 *                           overworld menu, pulling resources from
 *                           the app-scoped OpenAPI and branding
 *                           from the hostname.
 *         Admin +         — schema-driven Resource list mounts one
 *         AdminRouter       Resource per entity noun; the router
 *                           generates /:resource/[create|:id|:id/show]
 *                           routes automatically.
 *
 * The SSE bridge starts on mount (only when EventSource is
 * present — browser only, SSR skips) and tears down on unmount so
 * hot-reload doesn't leak connections.
 */
export function App(): ReactElement {
  const queryClient = useMemo(() => createArestQueryClient(), [])
  const branding = useBranding()

  // Providers are stable across the app's lifetime. Instantiated once
  // so the data provider's fetch closure identity stays consistent
  // across re-renders (important for TanStack Query memoisation).
  const providers = useMemo(
    () => ({
      data: createArestDataProvider({ baseUrl: AREST_BASE_URL }),
      auth: createArestAuthProvider({ baseUrl: AREST_BASE_URL }),
      navigation: createArestNavigationProvider({ baseUrl: AREST_BASE_URL, app: branding.app }),
    }),
    [branding.app],
  )

  // Domain controls the SSE bridge filter and defaults to the
  // branding's app slug. A future UI switcher can update this —
  // for now, it follows the hostname.
  const [domain, setDomain] = useState<string>(() => defaultDomainFor(branding.app))

  useEffect(() => {
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
      <BrowserRouter>
        <AdminRoot
          baseUrl={AREST_BASE_URL}
          app={branding.app}
          name={branding.name}
          nounScope={branding.nounScope}
          domain={domain}
          onDomainChange={setDomain}
          providersDebug={providers}
        />
      </BrowserRouter>
    </QueryClientProvider>
  )
}

interface AdminRootProps {
  baseUrl: string
  app: string
  name: string
  nounScope?: (n: string) => boolean
  domain: string
  onDomainChange: (next: string) => void
  providersDebug: {
    data: ReturnType<typeof createArestDataProvider>
    auth: ReturnType<typeof createArestAuthProvider>
    navigation: ReturnType<typeof createArestNavigationProvider>
  }
}

/**
 * Inside the router. Reads resources from the app-scoped OpenAPI
 * and hands them to @mdxui/admin's Admin component via <Resource>
 * children, which AdminRouter then generates routes from.
 */
function AdminRoot({ baseUrl, app, name, nounScope, domain, onDomainChange, providersDebug }: AdminRootProps): ReactElement {
  const { resources } = useArestResources({ baseUrl, app, filter: nounScope })

  return (
    <ArestAppShell
      baseUrl={baseUrl}
      app={app}
      config={{ name, basePath: '' }}
      pageHeader={<DomainSwitcher domain={domain} onChange={onDomainChange} />}
    >
      <Admin>
        {resources.map((r) => (
          <Resource
            key={r.name}
            name={r.name}
            list={r.list}
            create={r.create}
            edit={r.edit}
            show={r.show}
            icon={r.icon}
            options={r.options}
          />
        ))}
        <AdminRouter />
      </Admin>
      {/* Dev aid — expose providers on window for console pokes. */}
      <ProvidersDebug providers={providersDebug} />
    </ArestAppShell>
  )
}

/**
 * Small inline domain switcher in the page header. A proper picker
 * lands with Batch 6; this is enough to exercise the SSE bridge
 * across domains without rebooting the tab.
 */
function DomainSwitcher({ domain, onChange }: { domain: string; onChange: (next: string) => void }): ReactElement {
  return (
    <label style={{ fontSize: '0.875rem', display: 'inline-flex', alignItems: 'center', gap: '0.5rem' }}>
      Domain:
      <input
        type="text"
        data-testid="app-domain-switcher"
        value={domain}
        onChange={(e) => onChange(e.target.value)}
        style={{ padding: '0.25rem 0.5rem' }}
      />
    </label>
  )
}

function defaultDomainFor(app: string): string {
  // Heuristic: support.do watches the support domain; ui.do defaults
  // to 'organizations' which is AREST's bootstrap domain. Either way,
  // callers can override at runtime.
  if (app.startsWith('support')) return 'support'
  return 'organizations'
}

interface ProvidersDebugProps {
  providers: AdminRootProps['providersDebug']
}

function ProvidersDebug({ providers }: ProvidersDebugProps): ReactElement | null {
  useEffect(() => {
    if (typeof window === 'undefined') return
    ;(window as unknown as { __arest__?: unknown }).__arest__ = providers
  }, [providers])
  return null
}

export default App
