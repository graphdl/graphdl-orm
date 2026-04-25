/**
 * FilePreview — right-column collapsible details + preview pane.
 *
 * When a file id is supplied, fetches the File entity via useArestOne
 * and renders a mime-typed preview block (image / pdf / text via the
 * /file/{id}/content endpoint) plus a metadata strip and action
 * buttons (Download, Open, Delete). When no file is selected the
 * pane renders a slim placeholder.
 *
 * The actual delete button only fires the supplied callback — wiring
 * the mutation to the data provider lives at the parent, so this
 * component stays presentation-only and unit-testable.
 */
import { useEffect, useState, type ReactElement } from 'react'
import { useArestOne } from '../../hooks/useArestResource'
import { cn } from '../../lib/utils'
import { Download, ExternalLink, Trash, X } from '../../lib/icons'
import { pickFileIcon, type FileRow } from './FileList'

export interface FilePreviewProps {
  /** AREST worker base URL. */
  baseUrl: string
  /** File id to preview, or null when no file is selected. */
  fileId: string | null
  /** Close handler — typically clears the file id from the URL. */
  onClose: () => void
  /** Optional delete callback — when omitted, the button is hidden. */
  onDelete?: (id: string) => void
}

export function FilePreview({ baseUrl, fileId, onClose, onDelete }: FilePreviewProps): ReactElement {
  const query = useArestOne<FileRow>('File', fileId ?? '', { baseUrl })
  const file = query.data?.data ?? null

  if (!fileId) {
    return (
      <aside data-testid="file-preview" data-empty="true" className="hidden" aria-hidden="true" />
    )
  }

  return (
    <aside
      data-testid="file-preview"
      data-empty={file ? 'false' : 'true'}
      aria-label="File details"
      className="h-full flex flex-col bg-neutral-100 border-l border-border overflow-y-auto"
    >
      <header className="flex items-center justify-between px-md py-sm border-b border-border">
        <h2 className="text-h3 font-medium truncate">{file?.name ?? 'Loading…'}</h2>
        <button
          type="button"
          aria-label="Close preview"
          onClick={onClose}
          data-testid="file-preview-close"
          className="inline-flex items-center justify-center rounded-sm p-xs text-text-muted hover:text-text-primary hover:bg-neutral-200 transition-colors duration-fast"
        >
          <X size={16} />
        </button>
      </header>

      <div className="flex-1 px-md py-md space-y-md">
        <PreviewBody baseUrl={baseUrl} file={file} />
        {file ? <Metadata file={file} /> : null}
      </div>

      <footer className="px-md py-sm border-t border-border flex items-center gap-xs">
        <a
          href={file ? `${baseUrl}/file/${encodeURIComponent(file.id)}/content` : '#'}
          download={file?.name ?? undefined}
          data-testid="file-preview-download"
          className={cn(
            'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button border border-border',
            'text-text-primary bg-surface hover:bg-neutral-200 transition-colors duration-fast',
          )}
        >
          <Download size={14} />
          <span>Download</span>
        </a>
        <a
          href={file ? `${baseUrl}/file/${encodeURIComponent(file.id)}/content` : '#'}
          target="_blank"
          rel="noreferrer"
          data-testid="file-preview-open"
          className={cn(
            'inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button border border-border',
            'text-text-primary bg-surface hover:bg-neutral-200 transition-colors duration-fast',
          )}
        >
          <ExternalLink size={14} />
          <span>Open</span>
        </a>
        {onDelete && file ? (
          <button
            type="button"
            data-testid="file-preview-delete"
            onClick={() => onDelete(file.id)}
            className={cn(
              'ml-auto inline-flex items-center gap-xs rounded-sm px-sm py-xs text-button',
              'border border-danger/40 text-danger hover:bg-danger/10 transition-colors duration-fast',
            )}
          >
            <Trash size={14} />
            <span>Delete</span>
          </button>
        ) : null}
      </footer>
    </aside>
  )
}

interface PreviewBodyProps {
  baseUrl: string
  file: FileRow | null
}

function PreviewBody({ baseUrl, file }: PreviewBodyProps): ReactElement {
  const [textPreview, setTextPreview] = useState<string | null>(null)
  const mime = file?.mime_type ?? null
  const isImage = mime?.startsWith('image/') === true
  const isPdf = mime === 'application/pdf'
  const isText = mime?.startsWith('text/') === true

  useEffect(() => {
    let cancelled = false
    setTextPreview(null)
    if (!file || !isText) return
    const url = `${baseUrl}/file/${encodeURIComponent(file.id)}/content`
    fetch(url, { credentials: 'include' })
      .then((r) => (r.ok ? r.text() : Promise.reject(new Error(`HTTP ${r.status}`))))
      .then((text) => {
        if (!cancelled) setTextPreview(text.slice(0, 4000))
      })
      .catch(() => {
        if (!cancelled) setTextPreview(null)
      })
    return () => { cancelled = true }
  }, [baseUrl, file, isText])

  if (!file) {
    return (
      <div data-testid="file-preview-body-empty" className="text-text-muted text-body-sm">
        Loading file…
      </div>
    )
  }

  const url = `${baseUrl}/file/${encodeURIComponent(file.id)}/content`

  if (isImage) {
    return (
      <div className="rounded-sm overflow-hidden border border-border bg-neutral-200">
        <img
          src={url}
          alt={file.name}
          data-testid="file-preview-image"
          className="block w-full h-auto max-h-[420px] object-contain"
        />
      </div>
    )
  }
  if (isPdf) {
    return (
      <embed
        src={url}
        type="application/pdf"
        data-testid="file-preview-pdf"
        className="w-full h-[420px] rounded-sm border border-border bg-neutral-200"
      />
    )
  }
  if (isText) {
    return (
      <pre
        data-testid="file-preview-text"
        className="rounded-sm border border-border bg-neutral-200 p-sm text-code overflow-auto max-h-[420px] whitespace-pre-wrap"
      >
        {textPreview ?? 'Loading…'}
      </pre>
    )
  }

  // Generic fallback — show the mime icon centred.
  const Icon = pickFileIcon(mime)
  return (
    <div
      data-testid="file-preview-generic"
      className="flex flex-col items-center justify-center gap-sm rounded-sm border border-border bg-neutral-200 p-lg text-text-muted"
    >
      <Icon size={48} />
      <span className="text-body-sm">Preview unavailable for {mime ?? 'this file type'}.</span>
    </div>
  )
}

function Metadata({ file }: { file: FileRow }): ReactElement {
  const items: Array<{ label: string; value: string | null | undefined }> = [
    { label: 'Mime', value: file.mime_type ?? '—' },
    { label: 'Size', value: formatBytes(file.size) },
    { label: 'Parent', value: file.parent_id ?? 'root' },
  ]
  return (
    <dl data-testid="file-preview-metadata" className="grid grid-cols-[auto_1fr] gap-x-md gap-y-xs text-body-sm">
      {items.map((item) => (
        <div className="contents" key={item.label}>
          <dt className="text-text-muted">{item.label}</dt>
          <dd className="text-text-primary truncate">{item.value ?? '—'}</dd>
        </div>
      ))}
      {file.tags && file.tags.length > 0 ? (
        <div className="contents">
          <dt className="text-text-muted">Tags</dt>
          <dd className="flex flex-wrap gap-xs">
            {file.tags.map((t) => (
              <span
                key={t}
                className="inline-flex items-center rounded-full border border-border bg-neutral-100 px-sm py-xs text-label text-text-muted"
              >
                {t}
              </span>
            ))}
          </dd>
        </div>
      ) : null}
    </dl>
  )
}

function formatBytes(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = n
  let i = 0
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024
    i += 1
  }
  return `${value < 10 && i > 0 ? value.toFixed(1) : Math.round(value)} ${units[i]}`
}

FilePreview.displayName = 'FilePreview'
