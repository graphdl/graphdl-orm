/**
 * BulkToolbar — sticky action bar shown above the FileList while one
 * or more files are selected (#406).
 *
 * Pure presentation. Owns no business state — all action callbacks
 * are wired by FileBrowser, which holds the selection set + dialog
 * open flags. The toolbar disappears when `count === 0` (return null)
 * so it does not occupy any layout space at rest.
 *
 * Sticky positioning sits the toolbar at the top of the centre-column
 * scroll container; z-index=20 keeps it above the FileList's own
 * sticky-thead row (which has no explicit z) without obscuring the
 * preview pane on the right.
 */
import type { ReactElement } from 'react'
import { cn } from '../../lib/utils'
import { Move, Copy, Trash, Tag as TagIcon, Tags as TagsIcon, X } from '../../lib/icons'

export interface BulkToolbarProps {
  count: number
  onMove: () => void
  onCopy: () => void
  onDelete: () => void
  onTag: () => void
  onUntag: () => void
  onClear: () => void
}

export function BulkToolbar({
  count,
  onMove,
  onCopy,
  onDelete,
  onTag,
  onUntag,
  onClear,
}: BulkToolbarProps): ReactElement | null {
  if (count === 0) return null
  return (
    <div
      data-testid="bulk-toolbar"
      data-count={count}
      role="toolbar"
      aria-label="Bulk file actions"
      className={cn(
        'sticky top-0 z-20 flex flex-wrap items-center gap-xs px-md py-sm',
        'border-b border-border bg-accent/10 text-text-primary',
      )}
    >
      <span data-testid="bulk-toolbar-count" className="text-label font-medium mr-sm">
        {count} selected
      </span>
      <ToolbarButton testid="bulk-toolbar-move" onClick={onMove} icon={<Move size={14} aria-hidden="true" />}>
        Move…
      </ToolbarButton>
      <ToolbarButton testid="bulk-toolbar-copy" onClick={onCopy} icon={<Copy size={14} aria-hidden="true" />}>
        Copy…
      </ToolbarButton>
      <ToolbarButton
        testid="bulk-toolbar-delete"
        onClick={onDelete}
        icon={<Trash size={14} aria-hidden="true" />}
        variant="danger"
      >
        Delete
      </ToolbarButton>
      <ToolbarButton testid="bulk-toolbar-tag" onClick={onTag} icon={<TagIcon size={14} aria-hidden="true" />}>
        Tag…
      </ToolbarButton>
      <ToolbarButton testid="bulk-toolbar-untag" onClick={onUntag} icon={<TagsIcon size={14} aria-hidden="true" />}>
        Untag…
      </ToolbarButton>
      <button
        type="button"
        data-testid="bulk-toolbar-clear"
        onClick={onClear}
        className={cn(
          'ml-auto inline-flex items-center gap-xs rounded-sm px-sm py-xs text-label',
          'text-text-muted hover:text-text-primary hover:bg-neutral-200',
          'transition-colors duration-fast',
        )}
      >
        <X size={12} aria-hidden="true" />
        <span>Clear</span>
      </button>
    </div>
  )
}

interface ToolbarButtonProps {
  testid: string
  onClick: () => void
  icon: ReactElement
  variant?: 'default' | 'danger'
  children: string
}

function ToolbarButton({ testid, onClick, icon, variant = 'default', children }: ToolbarButtonProps): ReactElement {
  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      className={cn(
        'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button border',
        'transition-colors duration-fast',
        variant === 'danger'
          ? 'border-danger/40 text-danger bg-surface hover:bg-danger/10'
          : 'border-border text-text-primary bg-surface hover:bg-neutral-200',
      )}
    >
      {icon}
      <span>{children}</span>
    </button>
  )
}

BulkToolbar.displayName = 'BulkToolbar'
