/**
 * CopyDialog — destination-directory picker for bulk Copy (#406).
 *
 * Mirrors MoveDialog's UX (same picker, same flow) but the parent's
 * onConfirm fires `create({ data: { ...source, parent_id: targetId } })`
 * per source File rather than `update`. The source's content_ref is
 * passed through as-is — server-side may eventually duplicate or COW
 * the storage; the client just emits the create call.
 *
 * Kept as a thin wrapper around BaseDialog + DirectoryTree (deliberately
 * not extending MoveDialog so the labels / button text / testids stay
 * unambiguous — useful when more than one bulk-op test runs in the
 * same render).
 */
import { useState } from 'react'
import { cn } from '../../../lib/utils'
import { Copy } from '../../../lib/icons'
import { BaseDialog } from './BaseDialog'
import { DirectoryTree } from '../DirectoryTree'

export interface CopyDialogProps {
  open: boolean
  count: number
  baseUrl: string
  onClose: () => void
  /** Called with the chosen directory id; await to keep dialog open while pending. */
  onConfirm: (targetDirectoryId: string) => Promise<void> | void
}

export function CopyDialog({ open, count, baseUrl, onClose, onConfirm }: CopyDialogProps) {
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
      title={`Copy ${count} ${count === 1 ? 'file' : 'files'}`}
      testid="copy-dialog"
      footer={
        <>
          <button
            type="button"
            data-testid="copy-dialog-cancel"
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
            data-testid="copy-dialog-submit"
            onClick={handleConfirm}
            disabled={!target || pending}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'bg-accent text-surface hover:bg-accent/90 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            <Copy size={14} aria-hidden="true" />
            <span>{pending ? 'Copying…' : 'Copy'}</span>
          </button>
        </>
      }
    >
      <p className="text-body-sm text-text-muted mb-sm" data-testid="copy-dialog-copy">
        Choose a destination directory. The source content reference is reused — the
        server may duplicate or copy-on-write the underlying region.
      </p>
      <div
        data-testid="copy-dialog-picker"
        data-target={target ?? ''}
        className="border border-border rounded-sm bg-neutral-100 max-h-[320px] overflow-auto"
      >
        <DirectoryTree baseUrl={baseUrl} selectedId={target} onSelect={setTarget} />
      </div>
    </BaseDialog>
  )
}

CopyDialog.displayName = 'CopyDialog'
