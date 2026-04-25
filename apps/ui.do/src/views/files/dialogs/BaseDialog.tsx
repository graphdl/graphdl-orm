/**
 * BaseDialog — modal scaffold shared by the bulk-action dialogs (#406).
 *
 * Renders a full-page backdrop + a centered surface with a title bar,
 * a body slot, and a footer slot. No portal — the dialog renders in
 * place so tests can locate it via `screen.getByTestId(testid)` without
 * extra container plumbing. (A portal-based variant lands when more
 * than one dialog can stack; today only one bulk-op modal opens at a
 * time so in-tree mount is fine.)
 *
 * Pure presentation — does not own any business state. Esc cancels via
 * `onClose`; Enter does NOT submit (each dialog wires its own primary
 * button explicitly so destructive ops require a deliberate click).
 */
import { useEffect, type ReactNode } from 'react'
import { cn } from '../../../lib/utils'
import { X } from '../../../lib/icons'

export interface BaseDialogProps {
  /** True when the dialog should be visible. */
  open: boolean
  /** Called when the user dismisses (backdrop click, X, or Esc). */
  onClose: () => void
  /** Heading text shown in the title bar. */
  title: string
  /** Body content — typically a description + form controls. */
  children: ReactNode
  /** Footer content — typically a cancel + primary button pair. */
  footer?: ReactNode
  /** Custom data-testid for the outer surface. Default: 'base-dialog'. */
  testid?: string
  /** Width preset — 'sm' for confirmations, 'md' for pickers. */
  size?: 'sm' | 'md'
}

export function BaseDialog({
  open,
  onClose,
  title,
  children,
  footer,
  testid = 'base-dialog',
  size = 'md',
}: BaseDialogProps) {
  useEffect(() => {
    if (!open) return undefined
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose])

  if (!open) return null

  return (
    <div
      data-testid={`${testid}-backdrop`}
      role="presentation"
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${testid}-title`}
        data-testid={testid}
        onClick={(e) => e.stopPropagation()}
        className={cn(
          'relative bg-surface text-text-primary border border-border rounded-sm shadow-lg',
          'flex flex-col max-h-[80vh]',
          size === 'sm' ? 'w-[360px]' : 'w-[480px]',
        )}
      >
        <header className="flex items-center justify-between px-md py-sm border-b border-border">
          <h2 id={`${testid}-title`} className="text-h3 font-medium truncate">
            {title}
          </h2>
          <button
            type="button"
            aria-label="Close"
            data-testid={`${testid}-close`}
            onClick={onClose}
            className={cn(
              'inline-flex items-center justify-center rounded-sm p-xs text-text-muted',
              'hover:text-text-primary hover:bg-neutral-200 transition-colors duration-fast',
            )}
          >
            <X size={16} />
          </button>
        </header>

        <div className="flex-1 px-md py-md overflow-y-auto">{children}</div>

        {footer ? (
          <footer className="px-md py-sm border-t border-border flex items-center justify-end gap-xs">
            {footer}
          </footer>
        ) : null}
      </div>
    </div>
  )
}

BaseDialog.displayName = 'BaseDialog'
