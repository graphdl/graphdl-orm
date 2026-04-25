/**
 * useFileSelection tests (#406).
 *
 * Covers the click semantics (bare/shift/meta), the master-checkbox
 * indeterminate logic, and the visible-window range-select that
 * mirrors desktop file-manager behaviour.
 */
import { describe, expect, it } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { useFileSelection } from '../useFileSelection'

describe('useFileSelection', () => {
  it('starts empty with no anchor', () => {
    const { result } = renderHook(() => useFileSelection())
    expect(result.current.size).toBe(0)
    expect(result.current.has('a')).toBe(false)
    expect(result.current.allVisibleSelected).toBe(false)
    expect(result.current.someVisibleSelected).toBe(false)
  })

  it('bare click replaces selection with just that id', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.click('a', {}))
    expect(result.current.size).toBe(1)
    expect(result.current.has('a')).toBe(true)
    act(() => result.current.click('b', {}))
    expect(result.current.size).toBe(1)
    expect(result.current.has('a')).toBe(false)
    expect(result.current.has('b')).toBe(true)
  })

  it('ctrl/meta-click toggles individual ids without clearing existing', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.click('a', {}))
    act(() => result.current.click('b', { meta: true }))
    expect(result.current.size).toBe(2)
    expect(result.current.has('a')).toBe(true)
    expect(result.current.has('b')).toBe(true)
    // Toggle back off.
    act(() => result.current.click('a', { meta: true }))
    expect(result.current.size).toBe(1)
    expect(result.current.has('a')).toBe(false)
  })

  it('shift-click selects the contiguous range against visible order', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b', 'c', 'd', 'e']))
    act(() => result.current.click('b', {}))
    act(() => result.current.click('d', { shift: true }))
    expect(result.current.size).toBe(3)
    expect(result.current.has('b')).toBe(true)
    expect(result.current.has('c')).toBe(true)
    expect(result.current.has('d')).toBe(true)
    expect(result.current.has('a')).toBe(false)
    expect(result.current.has('e')).toBe(false)
  })

  it('shift-click without an anchor falls back to single-select', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b', 'c']))
    act(() => result.current.click('c', { shift: true }))
    expect(result.current.size).toBe(1)
    expect(result.current.has('c')).toBe(true)
  })

  it('checkbox toggle re-anchors and preserves other selections', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b', 'c', 'd']))
    act(() => result.current.click('a', {}))
    act(() => result.current.toggle('c'))
    expect(result.current.has('a')).toBe(true)
    expect(result.current.has('c')).toBe(true)
    // Anchor moved to c — shift-click 'd' fills [c…d].
    act(() => result.current.click('d', { shift: true }))
    expect(result.current.has('d')).toBe(true)
    // Sanity: 'b' was never picked.
    expect(result.current.has('b')).toBe(false)
  })

  it('master checkbox: allVisibleSelected and someVisibleSelected reflect state', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b', 'c']))
    expect(result.current.allVisibleSelected).toBe(false)
    expect(result.current.someVisibleSelected).toBe(false)
    act(() => result.current.click('a', {}))
    expect(result.current.allVisibleSelected).toBe(false)
    expect(result.current.someVisibleSelected).toBe(true)
    act(() => result.current.set(['a', 'b', 'c']))
    expect(result.current.allVisibleSelected).toBe(true)
    expect(result.current.someVisibleSelected).toBe(false)
  })

  it('toggleAllVisible adds every visible row when not all selected', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b', 'c']))
    act(() => result.current.toggleAllVisible())
    expect(result.current.size).toBe(3)
    // Toggling again clears the visible window (any off-window selections stay).
    act(() => result.current.toggleAllVisible())
    expect(result.current.size).toBe(0)
  })

  it('clear empties the selection and drops the anchor', () => {
    const { result } = renderHook(() => useFileSelection())
    act(() => result.current.setVisibleIds(['a', 'b']))
    act(() => result.current.click('a', {}))
    act(() => result.current.click('b', { meta: true }))
    expect(result.current.size).toBe(2)
    act(() => result.current.clear())
    expect(result.current.size).toBe(0)
    // After clear, shift-click on a row falls back to single-select.
    act(() => result.current.click('b', { shift: true }))
    expect(result.current.size).toBe(1)
  })
})
