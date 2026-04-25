/**
 * DirectoryTree — left-column collapsible tree of `Directory` rows.
 *
 * Pulls all directories via `useArestList<DirectoryRow>('Directory')`
 * and folds them into a parent → children map keyed on `parent_id`.
 * The current directory id (read from the URL by FileBrowser) is
 * highlighted; clicking any node calls `onSelect(id)` so the parent
 * can navigate via react-router.
 *
 * Pure presentation — no global state. Tokens-only styling: bg /
 * text colors come from Tailwind utilities backed by CSS variables.
 */
import { useMemo, useState, type ReactElement } from 'react'
import { useArestList } from '../../hooks/useArestResource'
import { cn } from '../../lib/utils'
import { ChevronDown, ChevronRight, Folder, FolderOpen } from '../../lib/icons'

export interface DirectoryRow {
  id: string
  name: string
  parent_id?: string | null
}

export interface DirectoryTreeProps {
  /** AREST worker base URL. */
  baseUrl: string
  /** Currently selected directory id; highlights the matching node. */
  selectedId?: string | null
  /** Click handler — receives the clicked directory id. */
  onSelect: (id: string) => void
}

interface TreeIndex {
  /** parent_id ('' = root) → children rows */
  byParent: Map<string, DirectoryRow[]>
  /** id → row */
  byId: Map<string, DirectoryRow>
}

function indexRows(rows: DirectoryRow[]): TreeIndex {
  const byParent = new Map<string, DirectoryRow[]>()
  const byId = new Map<string, DirectoryRow>()
  for (const row of rows) {
    byId.set(row.id, row)
    const key = row.parent_id ?? ''
    const list = byParent.get(key)
    if (list) list.push(row)
    else byParent.set(key, [row])
  }
  for (const list of byParent.values()) list.sort((a, b) => a.name.localeCompare(b.name))
  return { byParent, byId }
}

export function DirectoryTree({ baseUrl, selectedId, onSelect }: DirectoryTreeProps): ReactElement {
  const list = useArestList<DirectoryRow>('Directory', { pagination: { page: 1, perPage: 500 } }, { baseUrl })
  const rows = list.data?.data ?? []
  const tree = useMemo(() => indexRows(rows), [rows])

  return (
    <nav data-testid="directory-tree" aria-label="Directory tree" className="h-full overflow-y-auto p-sm">
      <DirectoryNodes
        parentId=""
        tree={tree}
        depth={0}
        selectedId={selectedId ?? null}
        onSelect={onSelect}
        defaultOpen
      />
    </nav>
  )
}

interface DirectoryNodesProps {
  parentId: string
  tree: TreeIndex
  depth: number
  selectedId: string | null
  onSelect: (id: string) => void
  defaultOpen?: boolean
}

function DirectoryNodes({ parentId, tree, depth, selectedId, onSelect, defaultOpen }: DirectoryNodesProps): ReactElement | null {
  const children = tree.byParent.get(parentId) ?? []
  if (children.length === 0) return null
  return (
    <ul role={depth === 0 ? 'tree' : 'group'} className="list-none m-0 p-0">
      {children.map((node) => (
        <DirectoryNode
          key={node.id}
          node={node}
          tree={tree}
          depth={depth}
          selectedId={selectedId}
          onSelect={onSelect}
          defaultOpen={defaultOpen}
        />
      ))}
    </ul>
  )
}

interface DirectoryNodeProps {
  node: DirectoryRow
  tree: TreeIndex
  depth: number
  selectedId: string | null
  onSelect: (id: string) => void
  defaultOpen?: boolean
}

function DirectoryNode({ node, tree, depth, selectedId, onSelect, defaultOpen }: DirectoryNodeProps): ReactElement {
  const hasChildren = (tree.byParent.get(node.id) ?? []).length > 0
  // Auto-expand the root level so the user sees first-level dirs by default.
  const [open, setOpen] = useState(Boolean(defaultOpen))
  const selected = node.id === selectedId
  const Chevron = open ? ChevronDown : ChevronRight
  const FolderGlyph = open ? FolderOpen : Folder

  return (
    <li role="treeitem" aria-selected={selected} aria-expanded={hasChildren ? open : undefined}>
      <button
        type="button"
        data-testid={`dir-node-${node.id}`}
        onClick={() => onSelect(node.id)}
        className={cn(
          'flex items-center gap-xs w-full text-left rounded-sm px-sm py-xs text-body',
          'transition-colors duration-fast hover:bg-neutral-200',
          selected && 'bg-accent/20 text-accent',
        )}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
      >
        {hasChildren ? (
          <span
            role="button"
            aria-label={open ? 'Collapse' : 'Expand'}
            data-testid={`dir-toggle-${node.id}`}
            onClick={(e) => {
              e.stopPropagation()
              setOpen((prev) => !prev)
            }}
            className="inline-flex items-center justify-center text-text-muted"
          >
            <Chevron size={14} />
          </span>
        ) : (
          <span className="inline-block w-[14px]" aria-hidden="true" />
        )}
        <FolderGlyph size={16} className="text-text-muted shrink-0" />
        <span className="truncate">{node.name}</span>
      </button>
      {hasChildren && open ? (
        <DirectoryNodes
          parentId={node.id}
          tree={tree}
          depth={depth + 1}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      ) : null}
    </li>
  )
}

DirectoryTree.displayName = 'DirectoryTree'
