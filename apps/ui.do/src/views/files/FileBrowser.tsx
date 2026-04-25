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
 *
 * Bulk-ops surface (#406):
 *   - Multi-selection state lives in useFileSelection (here, so the
 *     toolbar + dialogs share it across components).
 *   - BulkToolbar renders sticky above the FileList when ≥1 row
 *     selected; its actions open the matching dialog.
 *   - Dialogs call arestDataProvider per File (no bulk endpoints
 *     today; the data provider's updateMany / deleteMany sequence
 *     individual calls internally).
 *   - Keyboard: Esc clears, Delete fires the confirm flow,
 *     Ctrl/Cmd+A selects every visible row.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { useQueryClient } from '@tanstack/react-query'
import { cn } from '../../lib/utils'
import { DirectoryTree } from './DirectoryTree'
import { FileList, type FileRow } from './FileList'
import { FilePreview } from './FilePreview'
import { TagFilter } from './TagFilter'
import { BulkToolbar } from './BulkToolbar'
import { useFileSelection } from './useFileSelection'
import { MoveDialog } from './dialogs/MoveDialog'
import { CopyDialog } from './dialogs/CopyDialog'
import { DeleteConfirmDialog } from './dialogs/DeleteConfirmDialog'
import { TagDialog } from './dialogs/TagDialog'
import { UntagDialog, type UntagOption } from './dialogs/UntagDialog'
import { createArestDataProvider } from '../../providers'
import { nounToSlug } from '../../query'

export interface FileBrowserProps {
  /** AREST worker base URL (passed through to all child queries). */
  baseUrl: string
}

type DialogKind = null | 'move' | 'copy' | 'delete' | 'tag' | 'untag'

export function FileBrowser({ baseUrl }: FileBrowserProps): ReactElement {
  const params = useParams<{ directoryId?: string; fileId?: string }>()
  const navigate = useNavigate()
  const directoryId = params.directoryId ?? ''
  const fileId = params.fileId ?? null

  const [selectedTags, setSelectedTags] = useState<Set<string>>(() => new Set())
  const selection = useFileSelection()
  const [dialog, setDialog] = useState<DialogKind>(null)

  const queryClient = useQueryClient()
  const provider = useMemo(() => createArestDataProvider({ baseUrl }), [baseUrl])

  // Mirror of the visible row payload — the bulk-action handlers need
  // it to look up source entities for copy + tag-union for untag, and
  // to surface the raw row object during optimistic mutations.
  const visibleRowsRef = useRef<ReadonlyArray<FileRow>>([])
  const onVisibleRowsChange = useCallback((rows: ReadonlyArray<FileRow>) => {
    visibleRowsRef.current = rows
  }, [])

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

  const fileResource = useMemo(() => nounToSlug('File'), [])
  const tagResource = useMemo(() => nounToSlug('Tag'), [])

  const invalidateFiles = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['arest', 'list', fileResource] })
  }, [queryClient, fileResource])

  // ── Bulk-op handlers ───────────────────────────────────────────────
  // Each handler walks the current selection set and fires per-File
  // calls against the data provider. The data provider does not
  // expose bulk endpoints today (server-side bulk routes don't exist
  // yet), so we emit individual calls; updateMany / deleteMany on the
  // provider already do this internally for callers that prefer the
  // helper signature. After mutations, the File-list query is
  // invalidated so the visible rows re-render.

  const handleMoveConfirm = useCallback(
    async (targetDirectoryId: string) => {
      const ids = Array.from(selection.selected)
      // Per-File update — patches `parent_id` (the projection of
      // "File is in Directory" the worker exposes today). If the
      // worker later splits this into a discrete `Directory_has_File`
      // mutation, swap the lambda body — the call site stays put.
      await Promise.all(
        ids.map((id) => provider.update<FileRow>(fileResource, { id, data: { parent_id: targetDirectoryId } })),
      )
      await invalidateFiles()
      selection.clear()
    },
    [selection, provider, fileResource, invalidateFiles],
  )

  const handleCopyConfirm = useCallback(
    async (targetDirectoryId: string) => {
      const ids = Array.from(selection.selected)
      const byId = new Map(visibleRowsRef.current.map((r) => [r.id, r]))
      await Promise.all(
        ids.map((id) => {
          const src = byId.get(id)
          // Carry forward enough fields for the server to materialise
          // a duplicate. content_ref is reused as-is — server may
          // duplicate or COW the underlying region.
          const data: Partial<FileRow> & Record<string, unknown> = src
            ? {
                name: src.name,
                mime_type: src.mime_type ?? null,
                size: src.size ?? null,
                tags: src.tags ?? [],
                parent_id: targetDirectoryId,
                source_id: id,
              }
            : { parent_id: targetDirectoryId, source_id: id }
          return provider.create<FileRow>(fileResource, { data })
        }),
      )
      await invalidateFiles()
      selection.clear()
    },
    [selection, provider, fileResource, invalidateFiles],
  )

  const handleDeleteConfirm = useCallback(async () => {
    const ids = Array.from(selection.selected)
    await Promise.all(ids.map((id) => provider.delete<FileRow>(fileResource, { id })))
    await invalidateFiles()
    selection.clear()
  }, [selection, provider, fileResource, invalidateFiles])

  const handleTagConfirm = useCallback(
    async (chosen: { kind: 'existing'; tagId: string } | { kind: 'new'; name: string }) => {
      let tagId: string
      if (chosen.kind === 'new') {
        const created = await provider.create<{ id: string; name: string }>(tagResource, {
          data: { name: chosen.name },
        })
        tagId = created.data.id
        await queryClient.invalidateQueries({ queryKey: ['arest', 'list', tagResource] })
      } else {
        tagId = chosen.tagId
      }
      const ids = Array.from(selection.selected)
      const byId = new Map(visibleRowsRef.current.map((r) => [r.id, r]))
      // Add the tag to each File's tag set. The worker accepts a
      // `tags: string[]` patch on File today (the same projection
      // FileList consumes); a future `Tag is on File` create endpoint
      // would replace this lambda body.
      await Promise.all(
        ids.map((id) => {
          const src = byId.get(id)
          const existing = new Set<string>(src?.tags ?? [])
          existing.add(tagId)
          return provider.update<FileRow>(fileResource, {
            id,
            data: { tags: Array.from(existing) },
          })
        }),
      )
      await invalidateFiles()
      selection.clear()
    },
    [selection, provider, fileResource, tagResource, invalidateFiles, queryClient],
  )

  const handleUntagConfirm = useCallback(
    async (tagId: string) => {
      const ids = Array.from(selection.selected)
      const byId = new Map(visibleRowsRef.current.map((r) => [r.id, r]))
      await Promise.all(
        ids.map((id) => {
          const src = byId.get(id)
          const existing = src?.tags ?? []
          if (!existing.includes(tagId)) return Promise.resolve()
          const next = existing.filter((t) => t !== tagId)
          return provider.update<FileRow>(fileResource, {
            id,
            data: { tags: next },
          })
        }),
      )
      await invalidateFiles()
      // Clear selection so the toolbar matches the now-changed row state.
      selection.clear()
    },
    [selection, provider, fileResource, invalidateFiles],
  )

  // Tag-union across the selected file set drives the UntagDialog
  // option list. Recomputed on render; cheap because both the
  // selection and visible-row sets are small (≤ 500 rows).
  const untagOptions: ReadonlyArray<UntagOption> = useMemo(() => {
    if (selection.size === 0) return []
    const byId = new Map(visibleRowsRef.current.map((r) => [r.id, r]))
    const seen = new Map<string, string>()
    for (const id of selection.selected) {
      const src = byId.get(id)
      for (const t of src?.tags ?? []) {
        if (!seen.has(t)) seen.set(t, t) // id == display label until Tag entities are looked up
      }
    }
    return Array.from(seen.entries()).map(([id, name]) => ({ id, name }))
  }, [selection.selected, selection.size])

  // ── Keyboard shortcuts ──────────────────────────────────────────
  // Esc → clear selection (always). Delete → open delete-confirm if
  // ≥1 selected. Ctrl/Cmd+A → select every visible row, but only
  // when the focus is inside the file-browser (we don't want to
  // hijack global Select-All in form inputs).
  const browserRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Skip when typing in an input / textarea / contenteditable.
      const tag = (e.target as HTMLElement | null)?.tagName?.toLowerCase()
      const editable = tag === 'input' || tag === 'textarea' || (e.target as HTMLElement | null)?.isContentEditable
      if (e.key === 'Escape' && selection.size > 0 && !editable && dialog === null) {
        selection.clear()
      } else if (e.key === 'Delete' && selection.size > 0 && !editable && dialog === null) {
        e.preventDefault()
        setDialog('delete')
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a' && !editable) {
        // Only intercept when focus sits inside the file-browser surface.
        if (browserRef.current && browserRef.current.contains(e.target as Node)) {
          e.preventDefault()
          const ids = visibleRowsRef.current.map((r) => r.id)
          selection.set(ids)
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [selection, dialog])

  return (
    <div
      ref={browserRef}
      data-testid="file-browser"
      data-directory={directoryId || 'root'}
      data-file={fileId ?? ''}
      data-preview-open={previewOpen ? 'true' : 'false'}
      data-selection-count={selection.size}
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
        <BulkToolbar
          count={selection.size}
          onMove={() => setDialog('move')}
          onCopy={() => setDialog('copy')}
          onDelete={() => setDialog('delete')}
          onTag={() => setDialog('tag')}
          onUntag={() => setDialog('untag')}
          onClear={() => selection.clear()}
        />
        <FileList
          baseUrl={baseUrl}
          directoryId={directoryId}
          tagFilter={selectedTags}
          selectedFileId={fileId}
          onSelect={onSelectFile}
          selection={selection}
          onVisibleRowsChange={onVisibleRowsChange}
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

      <MoveDialog
        open={dialog === 'move'}
        count={selection.size}
        baseUrl={baseUrl}
        onClose={() => setDialog(null)}
        onConfirm={handleMoveConfirm}
      />
      <CopyDialog
        open={dialog === 'copy'}
        count={selection.size}
        baseUrl={baseUrl}
        onClose={() => setDialog(null)}
        onConfirm={handleCopyConfirm}
      />
      <DeleteConfirmDialog
        open={dialog === 'delete'}
        count={selection.size}
        onClose={() => setDialog(null)}
        onConfirm={handleDeleteConfirm}
      />
      <TagDialog
        open={dialog === 'tag'}
        count={selection.size}
        baseUrl={baseUrl}
        onClose={() => setDialog(null)}
        onConfirm={handleTagConfirm}
      />
      <UntagDialog
        open={dialog === 'untag'}
        count={selection.size}
        options={untagOptions}
        onClose={() => setDialog(null)}
        onConfirm={handleUntagConfirm}
      />
    </div>
  )
}

FileBrowser.displayName = 'FileBrowser'
