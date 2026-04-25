import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { scanAndRegisterWebComponents } from './components/registry'
import { AREST_BASE_URL } from './env'
import { createArestDataProvider } from './providers'
import './styles/globals.css'

/**
 * Boot entry for ui.do. Mounts the placeholder <App /> into #root.
 * Subsequent tasks (#122, #123) wrap this with mdxui providers.
 */
const rootNode = document.getElementById('root')
if (!rootNode) throw new Error('ui.do: #root element missing from index.html')

ReactDOM.createRoot(rootNode).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)

// Track KKKK (#494): mirror FFFF's #486 / GGGG's #487 / IIII's #488
// Component registration on the BROWSER side. After mount (deferred
// inside scanAndRegisterWebComponents to the next animation frame so
// any module that does `customElements.define(...)` at import time
// lands first), push the 9 standard HTML web-component facts through
// the existing arestDataProvider.create() surface. Failures are
// logged-and-skipped because the kernel-side adapter intentionally
// uses cell_push (not cell_push_unique), so re-boots will hit
// duplicate-key 409s on already-seeded facts.
void scanAndRegisterWebComponents(createArestDataProvider({ baseUrl: AREST_BASE_URL }))
