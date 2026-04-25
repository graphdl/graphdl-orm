/**
 * Bulk-action dialog tests (#406).
 *
 * Covers the four dialog components that the BulkToolbar fires open
 * — DeleteConfirmDialog (most-used), TagDialog, UntagDialog, and a
 * smoke for MoveDialog (DirectoryTree integration). Copy mirrors
 * Move so we trust the shared structure rather than duplicate.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { DeleteConfirmDialog } from '../dialogs/DeleteConfirmDialog'
import { TagDialog } from '../dialogs/TagDialog'
import { UntagDialog } from '../dialogs/UntagDialog'
import { MoveDialog } from '../dialogs/MoveDialog'

const baseUrl = 'https://ui.auto.dev/arest'

interface Recorded { url: string; method: string }

function stubFetch(handler: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    const req = { url, method }
    recorded.push(req)
    return handler(req)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

describe('DeleteConfirmDialog', () => {
  it('renders nothing when open is false', () => {
    render(
      <DeleteConfirmDialog
        open={false}
        count={2}
        onClose={() => {}}
        onConfirm={async () => {}}
      />,
    )
    expect(screen.queryByTestId('delete-confirm-dialog')).toBeNull()
  })

  it('shows the count in body + button label', () => {
    render(
      <DeleteConfirmDialog
        open
        count={3}
        onClose={() => {}}
        onConfirm={async () => {}}
      />,
    )
    expect(screen.getByTestId('delete-confirm-copy').textContent).toContain('Delete 3 files')
    expect(screen.getByTestId('delete-confirm-submit').textContent).toContain('Delete 3')
  })

  it('fires onConfirm and then onClose on submit', async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    const onClose = vi.fn()
    render(
      <DeleteConfirmDialog
        open
        count={1}
        onClose={onClose}
        onConfirm={onConfirm}
      />,
    )
    fireEvent.click(screen.getByTestId('delete-confirm-submit'))
    await waitFor(() => expect(onConfirm).toHaveBeenCalledOnce())
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
  })

  it('fires only onClose on cancel', () => {
    const onConfirm = vi.fn()
    const onClose = vi.fn()
    render(
      <DeleteConfirmDialog
        open
        count={1}
        onClose={onClose}
        onConfirm={onConfirm}
      />,
    )
    fireEvent.click(screen.getByTestId('delete-confirm-cancel'))
    expect(onClose).toHaveBeenCalledOnce()
    expect(onConfirm).not.toHaveBeenCalled()
  })
})

describe('TagDialog', () => {
  it('selecting an existing tag chip enables submit and forwards the id', async () => {
    stubFetch((req) => {
      if (/\/tags(\?|$)/.test(req.url)) {
        return json({ data: [
          { id: 't1', name: 'invoice' },
          { id: 't2', name: 'archived' },
        ] })
      }
      return json({ data: [] })
    })
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    render(wrap(
      <TagDialog
        open
        count={2}
        baseUrl={baseUrl}
        onClose={() => {}}
        onConfirm={onConfirm}
      />,
    ))
    const chip = await screen.findByTestId('tag-dialog-chip-t1')
    fireEvent.click(chip)
    fireEvent.click(screen.getByTestId('tag-dialog-submit'))
    await waitFor(() =>
      expect(onConfirm).toHaveBeenCalledWith({ kind: 'existing', tagId: 't1' }),
    )
  })

  it('typing a new-tag name enables submit and forwards as kind:new', async () => {
    stubFetch(() => json({ data: [] }))
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    render(wrap(
      <TagDialog
        open
        count={1}
        baseUrl={baseUrl}
        onClose={() => {}}
        onConfirm={onConfirm}
      />,
    ))
    const input = await screen.findByTestId('tag-dialog-new-input')
    fireEvent.change(input, { target: { value: 'urgent' } })
    fireEvent.click(screen.getByTestId('tag-dialog-submit'))
    await waitFor(() =>
      expect(onConfirm).toHaveBeenCalledWith({ kind: 'new', name: 'urgent' }),
    )
  })
})

describe('UntagDialog', () => {
  it('shows union options and forwards the chosen tag id', async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    render(
      <UntagDialog
        open
        count={3}
        options={[
          { id: 't1', name: 'invoice' },
          { id: 't2', name: 'archived' },
        ]}
        onClose={() => {}}
        onConfirm={onConfirm}
      />,
    )
    const chip = screen.getByTestId('untag-dialog-chip-t2')
    fireEvent.click(chip)
    fireEvent.click(screen.getByTestId('untag-dialog-submit'))
    await waitFor(() => expect(onConfirm).toHaveBeenCalledWith('t2'))
  })

  it('renders an empty placeholder when no tags are present on the selection', () => {
    render(
      <UntagDialog
        open
        count={1}
        options={[]}
        onClose={() => {}}
        onConfirm={vi.fn()}
      />,
    )
    expect(screen.getByTestId('untag-dialog-empty')).toBeDefined()
    const submit = screen.getByTestId('untag-dialog-submit') as HTMLButtonElement
    expect(submit.disabled).toBe(true)
  })
})

describe('MoveDialog', () => {
  it('integrates with DirectoryTree and forwards the chosen target id', async () => {
    stubFetch((req) => {
      if (/\/directorys(\?|$)/.test(req.url)) {
        return json({ data: [
          { id: 'd1', name: 'Inbox', parent_id: null },
          { id: 'd2', name: 'Archive', parent_id: null },
        ] })
      }
      return json({ data: [] })
    })
    const onConfirm = vi.fn().mockResolvedValue(undefined)
    render(wrap(
      <MoveDialog
        open
        count={4}
        baseUrl={baseUrl}
        onClose={() => {}}
        onConfirm={onConfirm}
      />,
    ))
    const picker = await screen.findByTestId('move-dialog-picker')
    const node = await within(picker).findByTestId('dir-node-d2')
    fireEvent.click(node)
    await waitFor(() => expect(picker.getAttribute('data-target')).toBe('d2'))
    fireEvent.click(screen.getByTestId('move-dialog-submit'))
    await waitFor(() => expect(onConfirm).toHaveBeenCalledWith('d2'))
  })

  it('keeps submit disabled until a destination is picked', () => {
    stubFetch(() => json({ data: [] }))
    render(wrap(
      <MoveDialog
        open
        count={1}
        baseUrl={baseUrl}
        onClose={() => {}}
        onConfirm={vi.fn()}
      />,
    ))
    const submit = screen.getByTestId('move-dialog-submit') as HTMLButtonElement
    expect(submit.disabled).toBe(true)
  })
})
