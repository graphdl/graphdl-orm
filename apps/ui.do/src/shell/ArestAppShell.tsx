/**
 * ArestAppShell — the top-level AREST application shell.
 *
 * Wraps @mdxui/app's <AppShell> and wires it to AREST concerns:
 *   - navigation is auto-derived from the app-scoped OpenAPI doc
 *     (useArestResources from #126 — every entity schema becomes a
 *     sidebar entry, skipping value types).
 *   - user / identity comes from the arestAuthProvider.getIdentity
 *     probe (sent through @mdxui/app's UserIdentity shape).
 *   - optional contextual footer renders the Overworld menu (#11)
 *     when viewing a specific entity, so SM transitions and HATEOAS
 *     navigation links are always one click away.
 *   - page header slot holds breadcrumbs; caller can supply their own
 *     or use the default path-walking element.
 *
 * Presentation-only: data fetches run through the providers and
 * TanStack Query cache already mounted by src/App.tsx.
 */
import type { ReactElement, ReactNode } from 'react'
import { AppShell } from '@mdxui/app'
import type { NavGroup, NavItem, UserIdentity } from '@mdxui/app'
import { useArestResources } from '../resources'
import { EntityOverworldMenu } from '../views'

export interface ArestAppShellProps {
  /** AREST worker base URL. */
  baseUrl: string
  /** App scope for /api/openapi.json?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** App display config passed to @mdxui/app's AppShell. */
  config: { name: string; description?: string; logo?: ReactNode; basePath?: string }
  /** Current user, if any. Shown in the sidebar footer / user menu. */
  user?: UserIdentity
  /**
   * When provided, the sidebar renders an EntityOverworldMenu below the
   * resource nav so the user always sees the SM transitions + HATEOAS
   * links available from the current entity.
   */
  currentEntity?: { noun: string; id: string }
  /** Page header content (breadcrumbs or similar). */
  pageHeader?: ReactNode
  /** Custom footer. Overrides the default (user block + overworld). */
  footer?: ReactNode
  /** Main content. */
  children: ReactNode
}

/**
 * Build a single NavGroup from the auto-discovered resources. Each
 * entry's `url` is basePath + /slug so routing libraries can mount on
 * the same tree.
 */
function resourcesToNavGroup(
  resources: ReturnType<typeof useArestResources>['resources'],
  basePath: string,
): NavGroup {
  const prefix = basePath.endsWith('/') ? basePath.slice(0, -1) : basePath
  const items: NavItem[] = resources
    .filter((r) => !r.options?.hideFromMenu)
    .map((r) => ({
      title: r.options?.label ?? r.name,
      url: `${prefix}/${r.name}`,
    }))
  return { label: 'Resources', items }
}

export function ArestAppShell(props: ArestAppShellProps): ReactElement {
  const { baseUrl, app, config, user, currentEntity, pageHeader, footer, children } = props
  const { resources, isLoading } = useArestResources({ baseUrl, app })
  const basePath = config.basePath ?? ''
  const navigation: NavGroup[] = [resourcesToNavGroup(resources, basePath)]

  const defaultFooter = currentEntity ? (
    <EntityOverworldMenu
      noun={currentEntity.noun}
      id={currentEntity.id}
      baseUrl={baseUrl}
    />
  ) : null

  return (
    <AppShell
      config={config}
      navigation={navigation}
      user={user}
      isLoading={isLoading}
      pageHeader={pageHeader}
      nav={<ArestSidebarNavList group={navigation[0]} />}
      footer={footer ?? defaultFooter}
    >
      {children}
    </AppShell>
  )
}

ArestAppShell.displayName = 'ArestAppShell'

/**
 * Simple sidebar nav list. Uses plain anchors so the app can route
 * with whatever router is mounted (react-router, next-router, etc.);
 * callers who want a custom LinkComponent can replace this with their
 * own and drop it into AppShell's `nav` slot directly.
 */
export function ArestSidebarNavList({ group }: { group: NavGroup }): ReactElement {
  return (
    <ul data-testid="arest-sidebar-nav" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
      {group.items.map((item) => (
        <li key={item.url}>
          <a href={item.url} data-testid={`nav-${item.url}`}
             style={{ display: 'block', padding: '0.5rem 1rem', color: 'inherit', textDecoration: 'none' }}>
            {item.title}
          </a>
        </li>
      ))}
    </ul>
  )
}
