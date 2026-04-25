/**
 * FileBrowser smoke tests — three-column shell, navigation, and tag
 * AND-filter semantics. Backed by stubbed fetch since the AREST worker
 * may not yet serve `/api/file`, `/api/directory`, `/api/tag`. The
 * stub deliberately returns deterministic shapes that the views can
 * render without depending on the live API.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { FileBrowser } from '../FileBrowser'

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

function wrap(node: ReactNode, initialPath = '/files') {
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

describe('FileBrowser', () => {
  it('renders the three-column shell (tree, list, preview)', async () => {
    stubFetch(() => json({ data: [], _links: {} }))
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('file-browser')).toBeDefined())
    expect(screen.getByTestId('directory-tree')).toBeDefined()
    expect(screen.getByTestId('file-list')).toBeDefined()
    // Preview is collapsed when no file is selected — column wrapper
    // still exists but is marked aria-hidden / data-collapsed.
    const browser = screen.getByTestId('file-browser')
    expect(browser.getAttribute('data-preview-open')).toBe('false')
  })

  it('toggles tag filter chips so multiple tags AND-combine into the filter set', async () => {
    // nounToSlug('Tag') === 'tags' — the data provider hits /arest/tags?…
    stubFetch((req) => {
      if (/\/tags(\?|$)/.test(req.url)) {
        return json({ data: [
          { id: 't1', name: 'invoice' },
          { id: 't2', name: 'archived' },
        ] })
      }
      return json({ data: [] })
    })
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('tag-filter')).toBeDefined())

    const chip1 = await screen.findByTestId('tag-chip-t1')
    const chip2 = await screen.findByTestId('tag-chip-t2')

    fireEvent.click(chip1)
    await waitFor(() =>
      expect(screen.getByTestId('tag-filter').getAttribute('data-selected')).toBe('t1'),
    )

    fireEvent.click(chip2)
    await waitFor(() => {
      // AND semantics — both ids appear, comma-separated, sorted lexicographically.
      const selected = screen.getByTestId('tag-filter').getAttribute('data-selected')
      expect(selected).toBe('t1,t2')
    })

    // Clicking chip1 again toggles it off — only t2 remains.
    fireEvent.click(chip1)
    await waitFor(() =>
      expect(screen.getByTestId('tag-filter').getAttribute('data-selected')).toBe('t2'),
    )
  })

  it('navigates to /files/:directoryId when a directory is clicked', async () => {
    // nounToSlug('Directory') === 'directorys' (naive pluralisation).
    stubFetch((req) => {
      if (/\/directorys(\?|$)/.test(req.url)) {
        return json({ data: [
          { id: 'd1', name: 'Inbox', parent_id: null },
          { id: 'd2', name: 'Archive', parent_id: null },
        ] })
      }
      return json({ data: [] })
    })
    render(wrap(<FileBrowser baseUrl={baseUrl} />))
    const tree = await screen.findByTestId('directory-tree')
    const node = await within(tree).findByTestId('dir-node-d2')
    fireEvent.click(node)
    await waitFor(() =>
      expect(screen.getByTestId('file-browser').getAttribute('data-directory')).toBe('d2'),
    )
  })
})
