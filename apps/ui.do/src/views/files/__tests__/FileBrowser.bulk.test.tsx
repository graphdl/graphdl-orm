/**
 * FileBrowser bulk-ops integration tests (#406).
 *
 * Exercises the full path: click rows → toolbar appears → confirm
 * dialog → per-File API calls → invalidation. Backed by stubbed
 * fetch — see FileBrowser.test.tsx for the wire shape AREST returns
 * (the data provider normalises both /docs/ and /data/ envelopes).
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { FileBrowser } from '../FileBrowser'

const baseUrl = 'https://ui.auto.dev/arest'

interface Recorded {
  url: string
  method: string
  body?: unknown
}

function stubFetch(handler: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    let body: unknown
    if (init?.body != null) {
      try { body = JSON.parse(init.body as string) } catch { body = init.body }
    }
    const req: Recorded = { url, method, body }
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

const fileRows = [
  { id: 'f1', name: 'alpha.txt', parent_id: 'd1', mime_type: 'text/plain', tags: ['t1'] },
  { id: 'f2', name: 'bravo.txt', parent_id: 'd1', mime_type: 'text/plain', tags: ['t2'] },
  { id: 'f3', name: 'charlie.txt', parent_id: 'd1', mime_type: 'text/plain', tags: [] },
]

const tagRows = [
  { id: 't1', name: 'invoice' },
  { id: 't2', name: 'archived' },
]

const dirRows = [
  { id: 'd1', name: 'Inbox', parent_id: null },
  { id: 'd2', name: 'Archive', parent_id: null },
]

function defaultHandler(req: Recorded): Response {
  if (/\/files(\?|$)/.test(req.url) && req.method === 'GET') {
    return json({ data: fileRows })
  }
  if (/\/tags(\?|$)/.test(req.url) && req.method === 'GET') {
    return json({ data: tagRows })
  }
  if (/\/directorys(\?|$)/.test(req.url) && req.method === 'GET') {
    return json({ data: dirRows })
  }
  return json({ data: [] })
}

function wrap(node: ReactNode, initialPath = '/files/d1') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[initialPath]}>
        <Routes>
          <Route path="/files" element={node} />
          <Route path="/files/:directoryId" element={node} />
          <Route path="/files/:directoryId/:fileId" element={node} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  )
}

afterEach(() => { vi.unstubAllGlobals() })

describe('FileBrowser — bulk selection + toolbar', () => {
  it('checkbox click toggles selection and shows the toolbar at >=1 selected', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')

    expect(screen.queryByTestId('bulk-toolbar')).toBeNull()

    const cb = screen.getByTestId('file-row-checkbox-f1') as HTMLInputElement
    fireEvent.click(cb)
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('1'),
    )
    expect(screen.getByTestId('bulk-toolbar')).toBeDefined()
    expect(screen.getByTestId('bulk-toolbar-count').textContent).toBe('1 selected')

    // Toggle a second row via the master? No — toggle a second checkbox.
    fireEvent.click(screen.getByTestId('file-row-checkbox-f2'))
    await waitFor(() =>
      expect(screen.getByTestId('bulk-toolbar-count').textContent).toBe('2 selected'),
    )
  })

  it('shift-click selects a contiguous range of rows', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')

    const row1 = screen.getByTestId('file-row-f1')
    const row3 = screen.getByTestId('file-row-f3')
    fireEvent.click(row1) // bare click sets anchor at f1
    fireEvent.click(row3, { shiftKey: true })
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('3'),
    )
  })

  it('Esc keyboard clears the selection', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')
    fireEvent.click(screen.getByTestId('file-row-checkbox-f1'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('1'),
    )
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('0'),
    )
    expect(screen.queryByTestId('bulk-toolbar')).toBeNull()
  })

  it('Ctrl+A selects every visible row when focus sits inside the browser', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')
    const browser = screen.getByTestId('file-browser')
    fireEvent.keyDown(browser, { key: 'a', ctrlKey: true })
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('3'),
    )
  })

  it('Delete key opens the delete-confirm dialog', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')
    fireEvent.click(screen.getByTestId('file-row-checkbox-f1'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('1'),
    )
    fireEvent.keyDown(window, { key: 'Delete' })
    await screen.findByTestId('delete-confirm-dialog')
    expect(screen.getByTestId('delete-confirm-copy').textContent).toContain('Delete 1 file')
  })

  it('delete confirm flow fires DELETE per selected file then clears selection', async () => {
    const recorded = stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')

    fireEvent.click(screen.getByTestId('file-row-checkbox-f1'))
    fireEvent.click(screen.getByTestId('file-row-checkbox-f3'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('2'),
    )

    fireEvent.click(screen.getByTestId('bulk-toolbar-delete'))
    const dialog = await screen.findByTestId('delete-confirm-dialog')
    fireEvent.click(within(dialog).getByTestId('delete-confirm-submit'))

    await waitFor(() => {
      const deletes = recorded.filter((r) => r.method === 'DELETE')
      const ids = deletes.map((r) => r.url.split('/').pop())
      expect(ids).toEqual(expect.arrayContaining(['f1', 'f3']))
    })
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('0'),
    )
  })

  it('tag-add flow PATCHes each selected file with the new tag id', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'PATCH' && /\/files\/[a-z0-9]+$/.test(req.url)) {
        return json({ data: { id: req.url.split('/').pop(), tags: ['t2'] } })
      }
      return defaultHandler(req)
    })
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f3')

    fireEvent.click(screen.getByTestId('file-row-checkbox-f3'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('1'),
    )

    fireEvent.click(screen.getByTestId('bulk-toolbar-tag'))
    const dialog = await screen.findByTestId('tag-dialog')
    const chip = await within(dialog).findByTestId('tag-dialog-chip-t2')
    fireEvent.click(chip)
    fireEvent.click(within(dialog).getByTestId('tag-dialog-submit'))

    await waitFor(() => {
      const patches = recorded.filter((r) => r.method === 'PATCH' && /\/files\//.test(r.url))
      // Patched f3 specifically, with the new tag included.
      const targeted = patches.find((r) => r.url.endsWith('/f3'))
      expect(targeted).toBeDefined()
      const body = targeted?.body as { tags?: string[] }
      expect(body?.tags).toEqual(expect.arrayContaining(['t2']))
    })
  })

  it('clicking the Clear button on the toolbar empties the selection', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')
    fireEvent.click(screen.getByTestId('file-row-checkbox-f1'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('1'),
    )
    fireEvent.click(screen.getByTestId('bulk-toolbar-clear'))
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('0'),
    )
  })

  it('master checkbox toggles every visible row on/off', async () => {
    stubFetch(defaultHandler)
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await screen.findByTestId('file-row-f1')
    const master = screen.getByTestId('file-list-master-checkbox') as HTMLInputElement
    fireEvent.click(master)
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('3'),
    )
    fireEvent.click(master)
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-selection-count')).toBe('0'),
    )
  })
})
