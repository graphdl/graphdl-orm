/**
 * FileBrowser — three-column shell for the /files surface (#405).
 *
 * Layout:
 *   ┌─────────────┬──────────────────────────┬─────────────────┐
 *   │ DirectoryTree │ TagFilter (above list)   │ FilePreview      │
 *   │  (~240px)    ├──────────────────────────┤  (~320px,        │
 *   │              │ FileList (flex-1)        │   collapsible)   │
 *   └─────────────┴──────────────────────────┴─────────────────┘
 *
 * The right pane collapses to width 0 when no file id is in the URL.
 * Routes:
 *   /files                            — root
 *   /files/:directoryId               — directory open
 *   /files/:directoryId/:fileId       — file selected (preview shown)
 *
 * Tag filter state lives here (not URL-encoded for now — a future
 * slice can lift it into a query param). Multiple selected tags
 * combine with AND semantics inside FileList.
 */
import { useCallback, useMemo, useState, type ReactElement } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { cn } from '../../lib/utils'
import { DirectoryTree } from './DirectoryTree'
import { FileList } from './FileList'
import { FilePreview } from './FilePreview'
import { TagFilter } from './TagFilter'

export interface FileBrowserProps {
  /** AREST worker base URL (passed through to all child queries). */
  baseUrl: string
}

export function FileBrowser({ baseUrl }: FileBrowserProps): ReactElement {
  const params = useParams<{ directoryId?: string; fileId?: string }>()
  const navigate = useNavigate()
  const directoryId = params.directoryId ?? ''
  const fileId = params.fileId ?? null

  const [selectedTags, setSelectedTags] = useState<Set<string>>(() => new Set())

  const onSelectDirectory = useCallback(
    (id: string) => {
      navigate(`/files/${encodeURIComponent(id)}`)
    },
    [navigate],
  )

  const onSelectFile = useCallback(
    (id: string) => {
      const dir = directoryId || 'root'
      navigate(`/files/${encodeURIComponent(dir)}/${encodeURIComponent(id)}`)
    },
    [navigate, directoryId],
  )

  const onClosePreview = useCallback(() => {
    if (directoryId) navigate(`/files/${encodeURIComponent(directoryId)}`)
    else navigate('/files')
  }, [navigate, directoryId])

  const onToggleTag = useCallback((id: string) => {
    setSelectedTags((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const previewOpen = Boolean(fileId)

  // Memoise the grid template so it changes only when the preview
  // visibility flips — avoids a layout thrash on tag toggles.
  const gridStyle = useMemo(
    () => ({ gridTemplateColumns: previewOpen ? '240px 1fr 320px' : '240px 1fr 0px' }),
    [previewOpen],
  )

  return (
    <div
      data-testid="file-browser"
      data-directory={directoryId || 'root'}
      data-file={fileId ?? ''}
      data-preview-open={previewOpen ? 'true' : 'false'}
      className={cn(
        'grid h-full min-h-[480px] w-full bg-surface text-text-primary',
        'border border-border rounded-sm overflow-hidden',
      )}
      style={gridStyle}
    >
      <section
        data-testid="file-browser-tree"
        className="border-r border-border bg-neutral-100 overflow-hidden"
        aria-label="Directories"
      >
        <DirectoryTree
          baseUrl={baseUrl}
          selectedId={directoryId || null}
          onSelect={onSelectDirectory}
        />
      </section>

      <section
        data-testid="file-browser-main"
        className="flex flex-col min-w-0 bg-surface overflow-hidden"
        aria-label="Files"
      >
        <TagFilter baseUrl={baseUrl} selected={selectedTags} onToggle={onToggleTag} />
        <FileList
          baseUrl={baseUrl}
          directoryId={directoryId}
          tagFilter={selectedTags}
          selectedFileId={fileId}
          onSelect={onSelectFile}
        />
      </section>

      <section
        data-testid="file-browser-preview"
        className={cn(
          'transition-[width] duration-normal overflow-hidden',
          previewOpen ? 'w-full' : 'w-0',
        )}
        aria-label="File preview"
        aria-hidden={previewOpen ? undefined : true}
      >
        <FilePreview baseUrl={baseUrl} fileId={fileId} onClose={onClosePreview} />
      </section>
    </div>
  )
}

FileBrowser.displayName = 'FileBrowser'
