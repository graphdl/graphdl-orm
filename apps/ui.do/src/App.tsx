import type { ReactElement } from 'react'
import { AREST_BASE_URL } from './env'

/**
 * Placeholder App — the mdxui shell and providers land in subsequent
 * commits (#122 wires data/auth/navigation providers, #123 adds the
 * TanStack Query cache + SSE bridge).
 */
export function App(): ReactElement {
  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', padding: '2rem' }}>
      <h1>ui.do</h1>
      <p>AREST front-end, talking to <code>{AREST_BASE_URL}</code>.</p>
      <p>
        This is the task-#121 placeholder — providers wire up in #122 and
        the TanStack Query + SSE bridge in #123.
      </p>
    </main>
  )
}

export default App
