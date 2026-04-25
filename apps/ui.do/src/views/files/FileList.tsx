/**
 * FileList — center column of the file browser.
 *
 * Lists Files inside the currently-selected Directory. Columns:
 * Name (with mime-typed icon), Size, Modified, Tags (chips). Sortable
 * by name / size / modified via the column headers. Filtered by the
 * AND-combined selected tag set passed in by the parent FileBrowser.
 *
 * Pure tokens-based styling — no inline colors, no inline px outside
 * the SPACING grid (we lean on the `xs` / `sm` / `md` / `lg` Tailwind
 * scale that mirrors SpacingToken).
 */
import { useMemo, useState, type ComponentType, type ReactElement } from 'react'
import { useArestList } from '../../hooks/useArestResource'
import { cn } from '../../lib/utils'
import {
  File as FileIcon,
  FileCode,
  FileImage,
  FilePdf,
  FileText,
  FileVideo,
  FileAudio,
  SortAsc,
  SortDesc,
} from '../../lib/icons'

export interface FileRow {
  id: string
  name: string
  size?: number | null
  mime_type?: string | null
  modified_at?: string | null
  parent_id?: string | null
  tags?: string[] | null
}

export type SortField = 'name' | 'size' | 'modified'
export type SortOrder = 'asc' | 'desc'
export interface SortState { field: SortField; order: SortOrder }

export interface FileListProps {
  /** AREST worker base URL. */
  baseUrl: string
  /** Currently-open directory id; '' means root. */
  directoryId: string
  /** Selected tag ids — file must carry every id in this set to appear. */
  tagFilter: Set<string>
  /** Currently-selected file id (highlights the row). */
  selectedFileId?: string | null
  /** Click handler — receives the clicked file id. */
  onSelect: (id: string) => void
}

type IconComponent = ComponentType<{ size?: number; className?: string }>

/** Map a mime-type prefix or extension hint onto a Lucide icon role. */
export function pickFileIcon(mime: string | null | undefined): IconComponent {
  if (!mime) return FileIcon
  if (mime.startsWith('image/')) return FileImage
  if (mime.startsWith('video/')) return FileVideo
  if (mime.startsWith('audio/')) return FileAudio
  if (mime === 'application/pdf') return FilePdf
  if (mime.startsWith('text/')) return FileText
  if (mime.includes('json') || mime.includes('xml') || mime.includes('javascript') || mime.includes('typescript')) {
    return FileCode
  }
  return FileIcon
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

function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '—'
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
}

export function FileList({ baseUrl, directoryId, tagFilter, selectedFileId, onSelect }: FileListProps): ReactElement {
  const [sort, setSort] = useState<SortState>({ field: 'name', order: 'asc' })

  // Server-side filter on parent_id when a directory is selected. The
  // worker may or may not honour this filter today; if it doesn't, we
  // also apply it client-side below for safety.
  const params = useMemo(
    () => ({
      pagination: { page: 1, perPage: 500 },
      ...(directoryId ? { filter: { parent_id: directoryId } } : {}),
    }),
    [directoryId],
  )
  const list = useArestList<FileRow>('File', params, { baseUrl })
  const allRows = list.data?.data ?? []

  const rows = useMemo(() => {
    const dirFiltered = directoryId
      ? allRows.filter((r) => (r.parent_id ?? null) === directoryId)
      : allRows
    const tagFiltered = tagFilter.size === 0
      ? dirFiltered
      : dirFiltered.filter((r) => {
          const set = new Set(r.tags ?? [])
          for (const t of tagFilter) if (!set.has(t)) return false
          return true
        })
    const sorted = [...tagFiltered].sort((a, b) => {
      let cmp = 0
      if (sort.field === 'name') cmp = a.name.localeCompare(b.name)
      else if (sort.field === 'size') cmp = (a.size ?? 0) - (b.size ?? 0)
      else cmp = (a.modified_at ?? '').localeCompare(b.modified_at ?? '')
      return sort.order === 'asc' ? cmp : -cmp
    })
    return sorted
  }, [allRows, directoryId, tagFilter, sort])

  const toggleSort = (field: SortField) => {
    setSort((prev) =>
      prev.field === field
        ? { field, order: prev.order === 'asc' ? 'desc' : 'asc' }
        : { field, order: 'asc' },
    )
  }

  return (
    <div data-testid="file-list" className="h-full overflow-y-auto">
      <table className="w-full border-collapse text-body">
        <thead className="sticky top-0 bg-neutral-100 border-b border-border">
          <tr>
            <SortHeader label="Name" field="name" sort={sort} onClick={toggleSort} />
            <SortHeader label="Size" field="size" sort={sort} onClick={toggleSort} align="right" />
            <SortHeader label="Modified" field="modified" sort={sort} onClick={toggleSort} />
            <th className="text-left px-md py-sm text-label text-text-muted font-medium">Tags</th>
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={4} className="px-md py-lg text-center text-text-muted text-body-sm" data-testid="file-list-empty">
                {list.isLoading ? 'Loading…' : 'No files in this directory.'}
              </td>
            </tr>
          ) : (
            rows.map((row) => {
              const Icon = pickFileIcon(row.mime_type)
              const isSelected = row.id === selectedFileId
              return (
                <tr
                  key={row.id}
                  data-testid={`file-row-${row.id}`}
                  data-selected={isSelected ? 'true' : 'false'}
                  onClick={() => onSelect(row.id)}
                  className={cn(
                    'cursor-pointer border-b border-border transition-colors duration-fast',
                    'hover:bg-neutral-200',
                    isSelected && 'bg-accent/10',
                  )}
                >
                  <td className="px-md py-sm">
                    <span className="inline-flex items-center gap-sm">
                      <Icon size={16} className="text-text-muted shrink-0" />
                      <span className="truncate">{row.name}</span>
                    </span>
                  </td>
                  <td className="px-md py-sm text-right tabular-nums text-text-muted">
                    {formatBytes(row.size)}
                  </td>
                  <td className="px-md py-sm text-text-muted">
                    {formatDate(row.modified_at)}
                  </td>
                  <td className="px-md py-sm">
                    <span className="inline-flex flex-wrap gap-xs">
                      {(row.tags ?? []).map((t) => (
                        <span
                          key={t}
                          className="inline-flex items-center rounded-full border border-border bg-neutral-100 px-sm py-xs text-label text-text-muted"
                        >
                          {t}
                        </span>
                      ))}
                    </span>
                  </td>
                </tr>
              )
            })
          )}
        </tbody>
      </table>
    </div>
  )
}

interface SortHeaderProps {
  label: string
  field: SortField
  sort: SortState
  onClick: (field: SortField) => void
  align?: 'left' | 'right'
}

function SortHeader({ label, field, sort, onClick, align = 'left' }: SortHeaderProps): ReactElement {
  const active = sort.field === field
  const Glyph = active && sort.order === 'desc' ? SortDesc : SortAsc
  return (
    <th
      scope="col"
      className={cn(
        'px-md py-sm text-label font-medium text-text-muted select-none cursor-pointer',
        align === 'right' ? 'text-right' : 'text-left',
      )}
      data-testid={`file-sort-${field}`}
      onClick={() => onClick(field)}
    >
      <span className={cn('inline-flex items-center gap-xs', align === 'right' && 'flex-row-reverse')}>
        <span>{label}</span>
        <Glyph size={12} className={cn('transition-opacity duration-fast', active ? 'opacity-100' : 'opacity-30')} />
      </span>
    </th>
  )
}

FileList.displayName = 'FileList'
