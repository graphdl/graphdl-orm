/**
 * OutcomesFeed — live-updating list view for the outcomes domain
 * (see readings/outcomes.md). Renders Violations and Failures
 * grouped by Severity / Failure Type so operators can spot issues
 * at a glance.
 *
 * The outcomes domain is AREST's canonical "failures as facts"
 * representation (whitepaper §8 alignment): every evaluation
 * path returns valid claims, violation facts, failure facts, or a
 * combination — no silent paths. This widget is the operator-side
 * view of that log.
 *
 * Live updates: useArestList subscribes via the TanStack Query
 * cache key the SSE bridge invalidates on, so any new Violation
 * or Failure mutation streamed from the worker re-renders the feed
 * within the 500ms broadcast budget.
 */
import type { ReactElement } from 'react'
import { useArestList } from '../hooks/useArestResource'
import type { ArestResourceOptions } from '../hooks/useArestResource'

export type Severity = 'error' | 'warning' | 'info'
export type FailureType = 'extraction' | 'evaluation' | 'transition' | 'parse' | 'induction'

export interface Violation {
  id: string
  severity?: Severity
  text?: string
  constraintId?: string
  domainId?: string
  timestamp?: string
  batchId?: string
  [key: string]: unknown
}

export interface Failure {
  id: string
  failureType?: FailureType
  severity?: Severity
  reasonText?: string
  inputText?: string
  domainId?: string
  timestamp?: string
  [key: string]: unknown
}

export interface OutcomesFeedOptions extends ArestResourceOptions {
  /** Optional domain filter — corresponds to the Violation belongs to Domain fact. */
  domain?: string
  /**
   * Field name used when applying the `domain` filter. The AREST
   * compiler produces JSON field names from fact-role readings (e.g.
   * `Violation belongs to Domain` -> `belongsToDomain` or `domain`
   * depending on the generator). If the default doesn't match your
   * app's OpenAPI schema, override here. Defaults to `'domain'`.
   */
  domainField?: string
  /** Maximum rows per section. */
  perSection?: number
}

function groupBy<T, K extends string>(rows: T[], key: (r: T) => K | undefined): Record<K, T[]> {
  const out = {} as Record<K, T[]>
  for (const row of rows) {
    const k = key(row)
    if (!k) continue
    if (!out[k]) out[k] = []
    out[k].push(row)
  }
  return out
}

function sortByTimestampDesc<T extends { timestamp?: string }>(rows: T[]): T[] {
  return [...rows].sort((a, b) => (b.timestamp ?? '').localeCompare(a.timestamp ?? ''))
}

// ── ViolationsFeed ─────────────────────────────────────────────────

export interface ViolationsFeedProps extends OutcomesFeedOptions {
  /** Header above the section. Defaults to "Violations". */
  title?: string
}

export function ViolationsFeed(props: ViolationsFeedProps): ReactElement {
  const { title = 'Violations', domain, domainField = 'domain', perSection = 5, ...opts } = props
  const filter = domain ? { [domainField]: domain } : undefined
  const query = useArestList<Violation>(
    'Violation',
    filter ? { filter } : undefined,
    opts,
  )
  const rows = sortByTimestampDesc(query.data?.data ?? [])
  const bySeverity = groupBy(rows, (r) => r.severity)
  const order: Severity[] = ['error', 'warning', 'info']

  return (
    <section data-testid="violations-feed">
      <h2>{title}</h2>
      {query.isLoading && <p>Loading…</p>}
      {!query.isLoading && rows.length === 0 && (
        <p data-testid="violations-empty">No violations — every path is a valid claim.</p>
      )}
      {order.map((sev) => {
        const group = bySeverity[sev] ?? []
        if (group.length === 0) return null
        return (
          <div key={sev} data-testid={`violations-${sev}`}>
            <h3>{sev}</h3>
            <ul>
              {group.slice(0, perSection).map((v) => (
                <li key={v.id} data-testid={`violation-${v.id}`}>
                  <strong>{v.constraintId ?? v.id}</strong>
                  {v.text && <> — {v.text}</>}
                  {v.timestamp && <small> ({v.timestamp})</small>}
                </li>
              ))}
            </ul>
          </div>
        )
      })}
    </section>
  )
}
ViolationsFeed.displayName = 'ViolationsFeed'

// ── FailuresFeed ───────────────────────────────────────────────────

export interface FailuresFeedProps extends OutcomesFeedOptions {
  title?: string
}

export function FailuresFeed(props: FailuresFeedProps): ReactElement {
  const { title = 'Failures', domain, domainField = 'domain', perSection = 5, ...opts } = props
  const filter = domain ? { [domainField]: domain } : undefined
  const query = useArestList<Failure>(
    'Failure',
    filter ? { filter } : undefined,
    opts,
  )
  const rows = sortByTimestampDesc(query.data?.data ?? [])
  const byType = groupBy(rows, (r) => r.failureType)
  const order: FailureType[] = ['parse', 'extraction', 'induction', 'evaluation', 'transition']

  return (
    <section data-testid="failures-feed">
      <h2>{title}</h2>
      {query.isLoading && <p>Loading…</p>}
      {!query.isLoading && rows.length === 0 && (
        <p data-testid="failures-empty">No failures recorded.</p>
      )}
      {order.map((ft) => {
        const group = byType[ft] ?? []
        if (group.length === 0) return null
        return (
          <div key={ft} data-testid={`failures-${ft}`}>
            <h3>{ft}</h3>
            <ul>
              {group.slice(0, perSection).map((f) => (
                <li key={f.id} data-testid={`failure-${f.id}`}>
                  <strong>{f.reasonText ?? f.id}</strong>
                  {f.inputText && <> — input: <code>{f.inputText}</code></>}
                </li>
              ))}
            </ul>
          </div>
        )
      })}
    </section>
  )
}
FailuresFeed.displayName = 'FailuresFeed'

// ── Composite ──────────────────────────────────────────────────────

export interface OutcomesBoardProps extends OutcomesFeedOptions {
  /** Optional section titles. */
  violationsTitle?: string
  failuresTitle?: string
}

export function OutcomesBoard(props: OutcomesBoardProps): ReactElement {
  const { violationsTitle, failuresTitle, ...rest } = props
  return (
    <div data-testid="outcomes-board" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '1rem' }}>
      <ViolationsFeed title={violationsTitle} {...rest} />
      <FailuresFeed title={failuresTitle} {...rest} />
    </div>
  )
}
OutcomesBoard.displayName = 'OutcomesBoard'
