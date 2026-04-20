import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'

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
