/**
 * TagDialog — bulk Tag-add modal (#406).
 *
 * Lists every existing Tag (via the Tag list endpoint) plus a free-text
 * "create new tag" input. The user picks an existing Tag chip OR types
 * a new name; on Confirm the parent's onConfirm fires per-File mutations
 * to attach the chosen Tag.
 *
 * Tag attachment is expressed as `File has-tag Tag` in the constraint
 * graph. Today the FileRow carries a `tags: string[]` projection of
 * the existing tag ids — the parent's onConfirm calls
 * `arestDataProvider.update('File', { id, data: { tags: [...next] } })`
 * for each selected File. If the worker later exposes a discrete
 * `Tag is on File` create endpoint, the mutation lambda swaps without
 * touching this component.
 */
import { useState } from 'react'
import { cn } from '../../../lib/utils'
import { Tag as TagIcon, Check, Plus } from '../../../lib/icons'
import { BaseDialog } from './BaseDialog'
import { useArestList } from '../../../hooks/useArestResource'
import type { TagRow } from '../TagFilter'

export interface TagDialogProps {
  open: boolean
  count: number
  baseUrl: string
  onClose: () => void
  /**
   * Called with the chosen tag identity. `kind` distinguishes:
   *   - 'existing' → tagRef is an existing Tag id
   *   - 'new'      → tagRef is the human-typed tag name (parent must
   *                  create the Tag entity then attach it)
   */
  onConfirm: (
    chosen: { kind: 'existing'; tagId: string } | { kind: 'new'; name: string },
  ) => Promise<void> | void
}

export function TagDialog({ open, count, baseUrl, onClose, onConfirm }: TagDialogProps) {
  const [pickedTagId, setPickedTagId] = useState<string | null>(null)
  const [newTagName, setNewTagName] = useState('')
  const [pending, setPending] = useState(false)

  const list = useArestList<TagRow>(
    'Tag',
    { pagination: { page: 1, perPage: 200 } },
    { baseUrl },
  )
  const tags = list.data?.data ?? []

  const reset = () => {
    setPickedTagId(null)
    setNewTagName('')
  }

  const handleClose = () => {
    if (pending) return
    reset()
    onClose()
  }

  const handleConfirm = async () => {
    const trimmedNew = newTagName.trim()
    if (!pickedTagId && !trimmedNew) return
    setPending(true)
    try {
      if (pickedTagId) await onConfirm({ kind: 'existing', tagId: pickedTagId })
      else await onConfirm({ kind: 'new', name: trimmedNew })
      reset()
      onClose()
    } finally {
      setPending(false)
    }
  }

  const canSubmit = (pickedTagId !== null || newTagName.trim().length > 0) && !pending

  return (
    <BaseDialog
      open={open}
      onClose={handleClose}
      title={`Tag ${count} ${count === 1 ? 'file' : 'files'}`}
      testid="tag-dialog"
      footer={
        <>
          <button
            type="button"
            data-testid="tag-dialog-cancel"
            onClick={handleClose}
            disabled={pending}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button border border-border',
              'text-text-primary bg-surface hover:bg-neutral-200 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="tag-dialog-submit"
            onClick={handleConfirm}
            disabled={!canSubmit}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'bg-accent text-surface hover:bg-accent/90 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            <TagIcon size={14} aria-hidden="true" />
            <span>{pending ? 'Tagging…' : 'Tag'}</span>
          </button>
        </>
      }
    >
      <div className="space-y-md">
        <div>
          <p className="text-label text-text-muted mb-xs">Existing tags</p>
          {tags.length === 0 ? (
            <p data-testid="tag-dialog-empty" className="text-body-sm text-text-muted">
              No tags defined yet.
            </p>
          ) : (
            <div className="flex flex-wrap gap-xs" data-testid="tag-dialog-existing">
              {tags.map((tag) => {
                const isOn = pickedTagId === tag.id
                return (
                  <button
                    type="button"
                    key={tag.id}
                    data-testid={`tag-dialog-chip-${tag.id}`}
                    aria-pressed={isOn}
                    onClick={() => {
                      setPickedTagId(isOn ? null : tag.id)
                      // Picking an existing chip clears the new-tag draft so
                      // the submit branch is unambiguous.
                      if (!isOn) setNewTagName('')
                    }}
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
              })}
            </div>
          )}
        </div>

        <div>
          <label
            htmlFor="tag-dialog-new-input"
            className="text-label text-text-muted mb-xs flex items-center gap-xs"
          >
            <Plus size={12} aria-hidden="true" />
            <span>Or create a new tag</span>
          </label>
          <input
            id="tag-dialog-new-input"
            type="text"
            data-testid="tag-dialog-new-input"
            value={newTagName}
            onChange={(e) => {
              setNewTagName(e.target.value)
              if (e.target.value.length > 0) setPickedTagId(null)
            }}
            placeholder="Tag name"
            className={cn(
              'w-full rounded-sm border border-border bg-surface px-sm py-xs text-body',
              'focus:outline-none focus:ring-2 focus:ring-accent/50',
            )}
          />
        </div>
      </div>
    </BaseDialog>
  )
}

TagDialog.displayName = 'TagDialog'
