/**
 * TagFilter — chip strip rendered above the FileList.
 *
 * Loads all tags via the data provider and exposes a controlled set
 * of selected tag ids. Multiple selected chips combine with AND
 * semantics — the FileList consumes the resulting Set<string> and
 * filters its rows so only files carrying every selected tag remain.
 *
 * The component is presentation-only: state lives in the parent
 * (FileBrowser) so it can be reflected into the URL or query keys.
 */
import type { ReactElement } from 'react'
import { useArestList } from '../../hooks/useArestResource'
import { cn } from '../../lib/utils'
import { Check, Filter } from '../../lib/icons'

export interface TagRow {
  id: string
  name: string
  color?: string | null
}

export interface TagFilterProps {
  /** AREST worker base URL. */
  baseUrl: string
  /** Currently selected tag ids (AND-combined). */
  selected: Set<string>
  /** Toggle handler — receives the clicked tag id. */
  onToggle: (id: string) => void
}

export function TagFilter({ baseUrl, selected, onToggle }: TagFilterProps): ReactElement {
  const list = useArestList<TagRow>('Tag', { pagination: { page: 1, perPage: 200 } }, { baseUrl })
  const tags = list.data?.data ?? []
  // Sorted, comma-joined for the data attribute the tests inspect.
  const selectedAttr = Array.from(selected).sort().join(',')

  return (
    <div
      data-testid="tag-filter"
      data-selected={selectedAttr}
      className="flex flex-wrap items-center gap-xs px-md py-sm border-b border-border bg-neutral-100"
    >
      <Filter size={14} className="text-text-muted shrink-0" aria-hidden="true" />
      <span className="text-label text-text-muted mr-xs">Tags:</span>
      {tags.length === 0 ? (
        <span data-testid="tag-filter-empty" className="text-body-sm text-text-muted">
          No tags
        </span>
      ) : (
        tags.map((tag) => {
          const isOn = selected.has(tag.id)
          return (
            <button
              type="button"
              key={tag.id}
              data-testid={`tag-chip-${tag.id}`}
              aria-pressed={isOn}
              onClick={() => onToggle(tag.id)}
              className={cn(
                'inline-flex items-center gap-xs rounded-full px-sm py-xs text-label',
                'border transition-colors duration-fast',
                isOn
                  ? 'border-accent bg-accent/20 text-accent'
                  : 'border-border bg-surface text-text-muted hover:text-text-primary hover:border-text-muted',
              )}
            >
              {isOn ? <Check size={12} aria-hidden="true" /> : null}
              <span>{tag.name}</span>
            </button>
          )
        })
      )}
    </div>
  )
}

TagFilter.displayName = 'TagFilter'
