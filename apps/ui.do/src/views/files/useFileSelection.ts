/**
 * useFileSelection — selection state for the FileList multi-select UX
 * (#406). Owns a `Set<FileId>` plus a `lastAnchor: FileId | null` so
 * shift-click can fill a contiguous range against the visible row
 * order supplied by the FileList.
 *
 * Selection semantics (mirror desktop file-manager convention):
 *   - bare click  → set selection to exactly {id}, anchor := id
 *   - shift-click → select [anchor … id] from the visible order
 *                   (preserves any rows already selected outside the
 *                   range — we union, not replace, to match Finder/
 *                   Explorer behaviour)
 *   - ctrl/meta   → toggle membership of {id}, anchor := id
 *
 * Visible-order is supplied via setVisibleIds so shift-range is
 * computed against the same sort/filter the user sees, not the raw
 * server payload. Callers update visibleIds inside a memo when their
 * row list changes.
 *
 * The hook is presentation-agnostic — it does not own the selected-
 * file-id (preview pin), which lives in the URL via FileBrowser.
 */
import { useCallback, useMemo, useRef, useState } from 'react'

export type FileId = string

export interface FileSelectionState {
  selected: Set<FileId>
  anchor: FileId | null
}

export interface FileSelectionApi {
  /** Read-only accessors. */
  selected: Set<FileId>
  size: number
  has: (id: FileId) => boolean
  /**
   * Click handler. Pass the modifier flags from the DOM event so the
   * hook stays detached from React event types — anything truthy on
   * `shift` or `meta` is honoured.
   */
  click: (id: FileId, modifiers: { shift?: boolean; meta?: boolean }) => void
  /** Toggle a single id without changing the anchor (checkbox col). */
  toggle: (id: FileId) => void
  /** Replace the selection with the supplied id set. */
  set: (ids: Iterable<FileId>) => void
  /** Empty selection. */
  clear: () => void
  /**
   * Tell the hook the current visible row order so shift-range can
   * resolve [anchor … id]. Stable across re-renders unless the order
   * actually changes.
   */
  setVisibleIds: (ids: ReadonlyArray<FileId>) => void
  /** All visible ids selected? — drives the master checkbox state. */
  allVisibleSelected: boolean
  /** ≥1 visible selected, but not all — indeterminate master state. */
  someVisibleSelected: boolean
  /** Toggle master: all-on if any unselected, all-off otherwise. */
  toggleAllVisible: () => void
}

/**
 * Build a selection API. Initial state is empty / anchor null.
 */
export function useFileSelection(): FileSelectionApi {
  const [state, setState] = useState<FileSelectionState>(() => ({
    selected: new Set<FileId>(),
    anchor: null,
  }))

  // Visible-order ref so shift-range resolves against the rendered
  // row order without forcing a re-render when the order changes.
  const visibleRef = useRef<ReadonlyArray<FileId>>([])

  const setVisibleIds = useCallback((ids: ReadonlyArray<FileId>) => {
    visibleRef.current = ids
  }, [])

  const has = useCallback((id: FileId) => state.selected.has(id), [state.selected])

  const click = useCallback(
    (id: FileId, modifiers: { shift?: boolean; meta?: boolean }) => {
      setState((prev) => {
        if (modifiers.meta) {
          // Ctrl/Meta-click — toggle membership of just this id and
          // re-anchor on the clicked row (so a follow-up shift-click
          // grows from here).
          const next = new Set(prev.selected)
          if (next.has(id)) next.delete(id)
          else next.add(id)
          return { selected: next, anchor: id }
        }
        if (modifiers.shift && prev.anchor != null) {
          // Range select against current visible order. Anchor stays
          // put — repeated shift-clicks pivot off the same anchor,
          // matching Finder/Explorer.
          const visible = visibleRef.current
          const a = visible.indexOf(prev.anchor)
          const b = visible.indexOf(id)
          if (a === -1 || b === -1) {
            // Anchor or target not in visible window — fall back to
            // single-select on the target.
            return { selected: new Set([id]), anchor: id }
          }
          const lo = Math.min(a, b)
          const hi = Math.max(a, b)
          const next = new Set(prev.selected)
          for (let i = lo; i <= hi; i += 1) next.add(visible[i])
          return { selected: next, anchor: prev.anchor }
        }
        // Bare click — replace selection with just this id.
        return { selected: new Set([id]), anchor: id }
      })
    },
    [],
  )

  const toggle = useCallback((id: FileId) => {
    setState((prev) => {
      const next = new Set(prev.selected)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      // Toggle re-anchors so a subsequent shift-click grows from here,
      // matching the checkbox-column UX in Gmail/Drive.
      return { selected: next, anchor: id }
    })
  }, [])

  const setAll = useCallback((ids: Iterable<FileId>) => {
    setState({ selected: new Set(ids), anchor: null })
  }, [])

  const clear = useCallback(() => {
    setState({ selected: new Set<FileId>(), anchor: null })
  }, [])

  const allVisibleSelected = useMemo(() => {
    const ids = visibleRef.current
    if (ids.length === 0) return false
    for (const id of ids) if (!state.selected.has(id)) return false
    return true
    // visibleRef is a ref so we depend on selected only; callers
    // that care about all-selected re-render when selected mutates.
  }, [state.selected])

  const someVisibleSelected = useMemo(() => {
    const ids = visibleRef.current
    if (ids.length === 0) return false
    let any = false
    let all = true
    for (const id of ids) {
      if (state.selected.has(id)) any = true
      else all = false
    }
    return any && !all
  }, [state.selected])

  const toggleAllVisible = useCallback(() => {
    const ids = visibleRef.current
    setState((prev) => {
      // If every visible is already selected, clear visible — otherwise
      // add every visible to the existing selection so off-screen rows
      // (e.g. behind a tag filter the user just toggled on) stay put.
      let everyVisibleOn = ids.length > 0
      for (const id of ids) {
        if (!prev.selected.has(id)) {
          everyVisibleOn = false
          break
        }
      }
      const next = new Set(prev.selected)
      if (everyVisibleOn) {
        for (const id of ids) next.delete(id)
      } else {
        for (const id of ids) next.add(id)
      }
      return { selected: next, anchor: prev.anchor }
    })
  }, [])

  return {
    selected: state.selected,
    size: state.selected.size,
    has,
    click,
    toggle,
    set: setAll,
    clear,
    setVisibleIds,
    allVisibleSelected,
    someVisibleSelected,
    toggleAllVisible,
  }
}
