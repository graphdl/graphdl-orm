/**
 * UntagDialog — bulk Tag-remove modal (#406).
 *
 * Shows the union of tags present on ANY selected file. Selecting a
 * tag removes it from every selected file that has it (no-op for
 * those that don't). Mirrors the TagDialog UX but skips the new-tag
 * input — you can only remove tags that already exist on the
 * selection.
 */
import { useState } from 'react'
import { cn } from '../../../lib/utils'
import { Tags as TagsIcon, Check } from '../../../lib/icons'
import { BaseDialog } from './BaseDialog'

export interface UntagOption {
  /** Tag id used by the parent's mutation lambda. */
  id: string
  /** Human-readable tag name. */
  name: string
}

export interface UntagDialogProps {
  open: boolean
  count: number
  /** Union of tag ids/names present across the selected file set. */
  options: ReadonlyArray<UntagOption>
  onClose: () => void
  /** Called with the chosen tag id; await to keep dialog open while pending. */
  onConfirm: (tagId: string) => Promise<void> | void
}

export function UntagDialog({ open, count, options, onClose, onConfirm }: UntagDialogProps) {
  const [pickedTagId, setPickedTagId] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  const handleClose = () => {
    if (pending) return
    setPickedTagId(null)
    onClose()
  }

  const handleConfirm = async () => {
    if (!pickedTagId) return
    setPending(true)
    try {
      await onConfirm(pickedTagId)
      setPickedTagId(null)
      onClose()
    } finally {
      setPending(false)
    }
  }

  return (
    <BaseDialog
      open={open}
      onClose={handleClose}
      title={`Remove tag from ${count} ${count === 1 ? 'file' : 'files'}`}
      testid="untag-dialog"
      footer={
        <>
          <button
            type="button"
            data-testid="untag-dialog-cancel"
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
            data-testid="untag-dialog-submit"
            onClick={handleConfirm}
            disabled={!pickedTagId || pending}
            className={cn(
              'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'bg-danger text-surface hover:bg-danger/90 transition-colors duration-fast',
              'disabled:opacity-50 disabled:cursor-not-allowed',
            )}
          >
            <TagsIcon size={14} aria-hidden="true" />
            <span>{pending ? 'Removing…' : 'Remove'}</span>
          </button>
        </>
      }
    >
      <p className="text-body-sm text-text-muted mb-sm" data-testid="untag-dialog-copy">
        Select a tag to remove from any selected file that carries it.
      </p>
      {options.length === 0 ? (
        <p data-testid="untag-dialog-empty" className="text-body-sm text-text-muted">
          The selected files have no tags to remove.
        </p>
      ) : (
        <div className="flex flex-wrap gap-xs" data-testid="untag-dialog-options">
          {options.map((opt) => {
            const isOn = pickedTagId === opt.id
            return (
              <button
                type="button"
                key={opt.id}
                data-testid={`untag-dialog-chip-${opt.id}`}
                aria-pressed={isOn}
                onClick={() => setPickedTagId(isOn ? null : opt.id)}
                className={cn(
                  'inline-flex items-center gap-xs rounded-full px-sm py-xs text-label',
                  'border transition-colors duration-fast',
                  isOn
                    ? 'border-danger bg-danger/20 text-danger'
                    : 'border-border bg-surface text-text-muted hover:text-text-primary hover:border-text-muted',
                )}
              >
                {isOn ? <Check size={12} aria-hidden="true" /> : null}
                <span>{opt.name}</span>
              </button>
            )
          })}
        </div>
      )}
    </BaseDialog>
  )
}

UntagDialog.displayName = 'UntagDialog'
