/**
 * OverworldMenu — navigation surface composed of Theorem-4
 * affordances: state-machine transitions (things you can do from
 * here) and HATEOAS navigation links (places you can go from here).
 *
 * Named for the overworld-menu idiom from adventure games: the menu
 * always surfaces the full set of exits and actions available at the
 * current location. In AREST terms: at an entity (a Cell in state s),
 *   links_full(e, n, s) = nav(e, n) ∪ links(s)
 * is the complete, derivable set of next-step options — no hidden
 * routes, no documentation to hallucinate through.
 *
 * Two component layers:
 *   - <OverworldMenu>        — presentational; takes sections, fires
 *                              onAction / onNavigate callbacks.
 *   - <EntityOverworldMenu>  — composed; given (noun, id) it wires
 *                              useActions + useEntityLinks into
 *                              OverworldMenu.
 */
import type { ReactElement } from 'react'
import { useActions } from '../hooks/useActions'
import { useEntityLinks } from '../hooks/useEntityLinks'

export interface OverworldActionItem {
  kind: 'action'
  /** Event name (used as dispatch argument). */
  name: string
  /** Human-readable label. */
  label: string
  /** Target status if the SM transition has one. */
  to?: string
}

export interface OverworldNavItem {
  kind: 'nav'
  /** Relation (menu key). */
  rel: string
  /** Destination path (typically starts with /arest/...). */
  href: string
  /** Human-readable label. */
  label: string
  /** Fact-type id the link was projected over, if applicable. */
  factType?: string
}

export type OverworldMenuItem = OverworldActionItem | OverworldNavItem

export interface OverworldMenuSection {
  label: string
  items: OverworldMenuItem[]
}

export interface OverworldMenuProps {
  title?: string
  /** Current status of the entity, if any. Surfaced as a sub-title. */
  status?: string
  sections: OverworldMenuSection[]
  onAction?: (actionName: string) => Promise<void> | void
  onNavigate?: (href: string) => void
  /** Loading state disables all interactions. */
  loading?: boolean
}

export function OverworldMenu(props: OverworldMenuProps): ReactElement {
  const { title, status, sections, onAction, onNavigate, loading } = props

  return (
    <nav data-testid="overworld-menu" aria-label="Entity affordances"
         style={{ display: 'flex', flexDirection: 'column', gap: '1rem', padding: '1rem', border: '1px solid #ddd', borderRadius: 6 }}>
      {(title || status) && (
        <header>
          {title && <h2 style={{ margin: 0 }}>{title}</h2>}
          {status && <p data-testid="overworld-status" style={{ margin: 0, color: '#666' }}>Status: {status}</p>}
        </header>
      )}
      {loading && <p>Loading…</p>}
      {!loading && sections.filter((s) => s.items.length > 0).map((section) => (
        <section key={section.label} data-testid={`overworld-section-${section.label.toLowerCase().replace(/\s+/g, '-')}`}>
          <h3 style={{ margin: '0 0 0.25rem 0', fontSize: '0.875rem', textTransform: 'uppercase', letterSpacing: '0.05em', color: '#444' }}>{section.label}</h3>
          <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: '0.25rem' }}>
            {section.items.map((item, i) => (
              <li key={item.kind === 'action' ? `a-${item.name}-${i}` : `n-${item.rel}-${item.href}`}>
                {item.kind === 'action' ? (
                  <button
                    type="button"
                    data-testid={`overworld-action-${item.name}`}
                    onClick={() => onAction?.(item.name)}
                    style={{ width: '100%', textAlign: 'left', padding: '0.5rem', cursor: 'pointer' }}
                  >
                    {item.label}{item.to && <span style={{ color: '#888' }}> → {item.to}</span>}
                  </button>
                ) : (
                  <button
                    type="button"
                    data-testid={`overworld-nav-${item.rel}`}
                    onClick={() => onNavigate?.(item.href)}
                    style={{ width: '100%', textAlign: 'left', padding: '0.5rem', cursor: 'pointer' }}
                  >
                    {item.label}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}

OverworldMenu.displayName = 'OverworldMenu'

export interface EntityOverworldMenuProps {
  noun: string
  id: string
  baseUrl: string
  /** Override the default section labels. */
  actionsLabel?: string
  navLabel?: string
  /** Fires on SM transition click. Defaults to dispatching via useActions. */
  onAction?: (actionName: string) => Promise<void> | void
  /** Fires on nav link click. No default — supply a router navigator. */
  onNavigate?: (href: string) => void
  /** Title to display — defaults to a simple `${noun} ${id}`. */
  title?: string
  /** Current status to display below the title. Optional. */
  status?: string
}

export function EntityOverworldMenu(props: EntityOverworldMenuProps): ReactElement {
  const { noun, id, baseUrl, actionsLabel, navLabel, onAction, onNavigate, title, status } = props
  const { actions, dispatch, isLoading: actionsLoading } = useActions(noun, id, { baseUrl })
  const { links, isLoading: linksLoading } = useEntityLinks(noun, id, { baseUrl })

  const sections: OverworldMenuSection[] = [
    {
      label: actionsLabel ?? 'Actions',
      items: actions.map((a) => ({
        kind: 'action' as const,
        name: a.name,
        label: a.label,
        to: a.to,
      })),
    },
    {
      label: navLabel ?? 'Go to',
      items: links.map((l) => ({
        kind: 'nav' as const,
        rel: l.rel,
        href: l.href,
        label: l.label,
        factType: l.factType,
      })),
    },
  ]

  return (
    <OverworldMenu
      title={title ?? `${noun} ${id}`}
      status={status}
      sections={sections}
      loading={actionsLoading || linksLoading}
      onAction={onAction ?? dispatch}
      onNavigate={onNavigate}
    />
  )
}

EntityOverworldMenu.displayName = 'EntityOverworldMenu'
