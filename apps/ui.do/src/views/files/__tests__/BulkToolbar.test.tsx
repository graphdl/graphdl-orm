/**
 * BulkToolbar tests (#406).
 *
 * Pure presentation. We only need to verify:
 *   - threshold: count=0 → renders nothing
 *   - count display + button enablement
 *   - each button fires the matching callback
 *   - Clear button is rendered separately
 */
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { BulkToolbar } from '../BulkToolbar'

function setup(count: number) {
  const onMove = vi.fn()
  const onCopy = vi.fn()
  const onDelete = vi.fn()
  const onTag = vi.fn()
  const onUntag = vi.fn()
  const onClear = vi.fn()
  render(
    <BulkToolbar
      count={count}
      onMove={onMove}
      onCopy={onCopy}
      onDelete={onDelete}
      onTag={onTag}
      onUntag={onUntag}
      onClear={onClear}
    />,
  )
  return { onMove, onCopy, onDelete, onTag, onUntag, onClear }
}

describe('BulkToolbar', () => {
  it('renders nothing when count is zero', () => {
    setup(0)
    expect(screen.queryByTestId('bulk-toolbar')).toBeNull()
  })

  it('renders the toolbar with the selection count when count > 0', () => {
    setup(3)
    const toolbar = screen.getByTestId('bulk-toolbar')
    expect(toolbar.getAttribute('data-count')).toBe('3')
    expect(screen.getByTestId('bulk-toolbar-count').textContent).toBe('3 selected')
  })

  it('fires the matching callback for each action button', () => {
    const cbs = setup(2)
    fireEvent.click(screen.getByTestId('bulk-toolbar-move'))
    fireEvent.click(screen.getByTestId('bulk-toolbar-copy'))
    fireEvent.click(screen.getByTestId('bulk-toolbar-delete'))
    fireEvent.click(screen.getByTestId('bulk-toolbar-tag'))
    fireEvent.click(screen.getByTestId('bulk-toolbar-untag'))
    fireEvent.click(screen.getByTestId('bulk-toolbar-clear'))
    expect(cbs.onMove).toHaveBeenCalledOnce()
    expect(cbs.onCopy).toHaveBeenCalledOnce()
    expect(cbs.onDelete).toHaveBeenCalledOnce()
    expect(cbs.onTag).toHaveBeenCalledOnce()
    expect(cbs.onUntag).toHaveBeenCalledOnce()
    expect(cbs.onClear).toHaveBeenCalledOnce()
  })
})
