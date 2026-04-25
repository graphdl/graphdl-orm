/**
 * MoveDialog — destination-directory picker for bulk Move (#406).
 *
 * Reuses the DirectoryTree as the picker UI: clicking a directory
 * stages it as the target. Confirm fires `onConfirm(targetDirectoryId)`
 * — the parent walks the selected File set and emits per-File
 * `update({ data: { parent_id: targetId } })` calls (the AREST data
 * provider has no bulk endpoint today; updateMany sequences single
 * updates internally).
 *
 * If the underlying constraint surface evolves so that "File is in
 * Directory" is exposed as a discrete `Directory_has_File` mutation
 * rather than a parent_id field on File, the parent's mutation lambda
 * is the only swap point — this dialog stays unchanged.
 */
import { useState } from 'react'
import { cn } from '../../../lib/utils'
import { Move } from '../../../lib/icons'
import { BaseDialog } from './BaseDialog'
import { DirectoryTree } from '../DirectoryTree'

export interface MoveDialogProps {
  open: boolean
  count: number
  baseUrl: string
  onClose: () => void
  /** Called with the chosen directory id; await to keep dialog open while pending. */
  onConfirm: (targetDirectoryId: string) => Promise<void> | void
}

export function MoveDialog({ open, count, baseUrl, onClose, onConfirm }: MoveDialogProps) {
  const [target, setTarget] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  const handleConfirm = async () => {
    if (!target) return
    setPending(true)
    try {
      await onConfirm(target)
      setTarget(null)
      onClose()
    } finally {
      setPending(false)
    }
  }

  const handleClose = () => {
    if (pending) return
    setTarget(null)
    onClose()
  }

  return (
    <BaseDialog
      open={open}
      onClose={handleClose}
      title={`Move ${count} ${count === 1 ? 'file' : 'files'}`}
      testid="move-dialog"
      footer={
        <>
          <button
            type="button"
            data-testid="move-dialog-cancel"
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
            data-testid="move-dialog-submit"
            onClick={handleConfirm}
            disabled={!target || pending}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'bg-accent text-surface hover:bg-accent/90 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            <Move size={14} aria-hidden="true" />
            <span>{pending ? 'Moving…' : 'Move'}</span>
          </button>
        </>
      }
    >
      <p className="text-body-sm text-text-muted mb-sm" data-testid="move-dialog-copy">
        Choose a destination directory.
      </p>
      <div
        data-testid="move-dialog-picker"
        data-target={target ?? ''}
        className="border border-border rounded-sm bg-neutral-100 max-h-[320px] overflow-auto"
      >
        <DirectoryTree baseUrl={baseUrl} selectedId={target} onSelect={setTarget} />
      </div>
    </BaseDialog>
  )
}

MoveDialog.displayName = 'MoveDialog'
