import type { ReactElement } from 'react'
import { AREST_BASE_URL } from './env'
import {
  createArestAuthProvider,
  createArestDataProvider,
  createArestNavigationProvider,
} from './providers'

/**
 * Placeholder App — wires the three AREST providers now that #122 has
 * landed. The mdxui <App /> shell and per-resource views land when the
 * TanStack Query + SSE bridge is in place (#123); for now we export
 * the providers through the module closure so a dev tool can poke at
 * them from the console.
 */
export const providers = {
  data: createArestDataProvider({ baseUrl: AREST_BASE_URL }),
  auth: createArestAuthProvider({ baseUrl: AREST_BASE_URL }),
  navigation: createArestNavigationProvider({ baseUrl: AREST_BASE_URL }),
}

export function App(): ReactElement {
  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', padding: '2rem' }}>
      <h1>ui.do</h1>
      <p>AREST front-end, talking to <code>{AREST_BASE_URL}</code>.</p>
      <p>
        Providers wired (data / auth / navigation). TanStack Query +
        SSE bridge land with #123.
      </p>
    </main>
  )
}

export default App
