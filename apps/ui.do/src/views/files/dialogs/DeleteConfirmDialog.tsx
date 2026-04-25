/**
 * DeleteConfirmDialog — bulk-delete confirmation modal (#406).
 *
 * Shown by the BulkToolbar when the user fires "Delete". Renders a
 * count + a warning copy + a destructive primary button. The actual
 * mutation is fired by the parent's onConfirm — this component just
 * collects assent.
 *
 * The body explicitly states "This cannot be undone." per the spec
 * — no soft-delete escape hatch in the worker today, so the warning
 * is honest.
 */
import { useState } from 'react'
import { cn } from '../../../lib/utils'
import { Trash } from '../../../lib/icons'
import { BaseDialog } from './BaseDialog'

export interface DeleteConfirmDialogProps {
  open: boolean
  count: number
  onClose: () => void
  /** Async confirm — caller fires per-File deletes; we await + close. */
  onConfirm: () => Promise<void> | void
}

export function DeleteConfirmDialog({ open, count, onClose, onConfirm }: DeleteConfirmDialogProps) {
  const [pending, setPending] = useState(false)

  const handleConfirm = async () => {
    setPending(true)
    try {
      await onConfirm()
      onClose()
    } finally {
      setPending(false)
    }
  }

  return (
    <BaseDialog
      open={open}
      onClose={onClose}
      title="Delete files"
      testid="delete-confirm-dialog"
      size="sm"
      footer={
        <>
          <button
            type="button"
            data-testid="delete-confirm-cancel"
            onClick={onClose}
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
            data-testid="delete-confirm-submit"
            onClick={handleConfirm}
            disabled={pending}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'bg-danger text-surface hover:bg-danger/90 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            <Trash size={14} aria-hidden="true" />
            <span>{pending ? 'Deleting…' : `Delete ${count}`}</span>
          </button>
        </>
      }
    >
      <p className="text-body" data-testid="delete-confirm-copy">
        Delete {count} {count === 1 ? 'file' : 'files'}? This cannot be undone.
      </p>
    </BaseDialog>
  )
}

DeleteConfirmDialog.displayName = 'DeleteConfirmDialog'
