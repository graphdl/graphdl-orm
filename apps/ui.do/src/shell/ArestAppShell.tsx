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
import { useArestResources, useExternalSystems } from '../resources'
import { EntityOverworldMenu } from '../views'
import { Folder } from '../lib/icons'

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
  /**
   * Optional noun filter — hides any noun for which the predicate
   * returns false. Sourced from hostname branding (#128) in production
   * so support.auto.dev narrows the sidebar to support-domain nouns.
   */
  nounScope?: (nounName: string) => boolean
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

/**
 * Mirror of `resourcesToNavGroup` for mounted External Systems
 * (#343). Rendered as its own nav group so vocabulary browsers
 * don't clutter the first-class Resources list.
 */
function externalResourcesToNavGroup(
  resources: ReturnType<typeof useExternalSystems>['resources'],
  basePath: string,
): NavGroup {
  const prefix = basePath.endsWith('/') ? basePath.slice(0, -1) : basePath
  const items: NavItem[] = resources
    .filter((r) => !r.options?.hideFromMenu)
    .map((r) => ({
      title: r.options?.label ?? r.name,
      url: `${prefix}/${r.name}`,
    }))
  return { label: 'External Systems', items }
}

export function ArestAppShell(props: ArestAppShellProps): ReactElement {
  const { baseUrl, app, config, user, currentEntity, nounScope, pageHeader, footer, children } = props
  const { resources, isLoading } = useArestResources({ baseUrl, app, filter: nounScope })
  const { resources: externalResources, systems: externalSystems } = useExternalSystems({ baseUrl, app })
  const basePath = config.basePath ?? ''
  const prefix = basePath.endsWith('/') ? basePath.slice(0, -1) : basePath
  // #405 — top-of-sidebar Quick group surfaces the bespoke /files
  // browser ahead of the auto-discovered Resources list.
  const quickGroup: NavGroup = {
    label: 'Quick',
    items: [{ title: 'Files', url: `${prefix}/files`, icon: Folder }],
  }
  const navigation: NavGroup[] = [quickGroup, resourcesToNavGroup(resources, basePath)]
  if (externalSystems.length > 0) {
    navigation.push(externalResourcesToNavGroup(externalResources, basePath))
  }

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
      nav={<ArestSidebarNav groups={navigation} />}
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

/**
 * Multi-group sidebar — renders every NavGroup with a heading. #343
 * introduced the second group (External Systems); future lanes may
 * add more (saved searches, etc.) without further edits here.
 */
export function ArestSidebarNav({ groups }: { groups: NavGroup[] }): ReactElement {
  return (
    <nav data-testid="arest-sidebar-nav-root">
      {groups.map((group, idx) => (
        <section key={`${group.label}-${idx}`} data-testid={`arest-sidebar-group-${group.label}`}>
          {group.label ? (
            <h3 style={{ margin: '0.75rem 1rem 0.25rem', fontSize: '0.8rem', opacity: 0.7 }}>
              {group.label}
            </h3>
          ) : null}
          <ArestSidebarNavList group={group} />
        </section>
      ))}
    </nav>
  )
}
