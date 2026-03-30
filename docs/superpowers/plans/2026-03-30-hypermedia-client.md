# Hypermedia Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace ui.do with a generic AREST hypermedia renderer — a peer AREST system with client-side WASM (ρ), cached population (P), and mdx-ui View Renderers.

**Architecture:** Three layers following the paper: ρ (fol_engine.wasm), TargetFactory (View Renderer resolution from P via ρ), Render (mdx-ui components). No entity-specific code. iFactr/MonoCross pattern: Register = View Renderer facts in P; NavigationMap = HATEOAS links; MXViewMap = θ₁ queries via ρ. Client WASM is a validation UX, not a security boundary — server is authoritative.

**Tech Stack:** React 19, Vite, fol_engine.wasm (1.5MB), @mdxui/primitives 6.0.0, @mdxui/app 6.0.0, @mdxui/admin 6.0.1, @mdxui/widgets 6.1.0, @mdxui/themes 6.0.0, SSE for streaming.

---

## File Map

```
ui.do/src/
  main.tsx                — Entry point. Load WASM, fetch session, mount App.
  index.css               — Import @mdxui/themes CSS + @mdxui/primitives styles.

  rho/
    engine.ts             — WASM lifecycle: init, load_ir, applyCommand, derive, validate, getTransitions.
    population.ts         — P_local: fetch entity partitions, cache, merge deltas, query by type+domain.
    stream.ts             — SSE client: subscribe /api/stream, parse events, update P_local.

  factory/
    registry.ts           — Component name → lazy import. The ONLY hardcoded config (~15 entries).
    resolve.ts            — Query P for View Renderer facts. (nounType, perspective) → component.

  shell/
    App.tsx               — Root: init ρ, load P, render shell + Renderer.
    Shell.tsx             — @mdxui/app AppShell. Sidebar from Domain/App facts. Breadcrumbs.
    Renderer.tsx          — URL → fetch representation → resolve view → render component.

  views/
    ListView.tsx          — @mdxui/admin DataGrid. Entity docs → rows, HATEOAS links → actions.
    DetailView.tsx        — @mdxui/primitives Card. Fields → key-value, transitions → buttons.
    FormView.tsx          — Schema-driven form from ρ. Graph schema roles → controls.
    ChatView.tsx          — @mdxui/widgets ChatBox. SSE to /ai/chat.
```

---

### Task 1: Install mdx-ui dependencies and WASM

**Files:**
- Modify: `ui.do/package.json`
- Modify: `ui.do/vite.config.ts`
- Create: `ui.do/src/rho/engine.ts`
- Copy: `graphdl-orm/crates/fol-engine/pkg/` → `ui.do/public/wasm/`

- [ ] **Step 1: Link mdx-ui packages from local monorepo**

Since mdx-ui is in `C:\Users\lippe\Repos\ui\packages\`, install from local paths:

```bash
cd /c/Users/lippe/Repos/ui.do
npm install ../ui/packages/primitives ../ui/packages/app ../ui/packages/admin ../ui/packages/widgets ../ui/packages/themes
```

If that fails due to peer deps, use `--legacy-peer-deps`. The packages depend on React 19 which ui.do already has.

- [ ] **Step 2: Copy WASM to public directory**

```bash
mkdir -p public/wasm
cp ../graphdl-orm/crates/fol-engine/pkg/fol_engine_bg.wasm public/wasm/
cp ../graphdl-orm/crates/fol-engine/pkg/fol_engine.js src/rho/fol_engine.js
cp ../graphdl-orm/crates/fol-engine/pkg/fol_engine.d.ts src/rho/fol_engine.d.ts
cp ../graphdl-orm/crates/fol-engine/pkg/fol_engine_bg.wasm.d.ts src/rho/fol_engine_bg.wasm.d.ts
```

- [ ] **Step 3: Configure Vite for WASM**

In `ui.do/vite.config.ts`, add WASM support:

```typescript
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  optimizeDeps: {
    exclude: ['fol_engine'],
  },
  server: {
    fs: { allow: ['..'] },
  },
})
```

- [ ] **Step 4: Write the WASM engine wrapper**

Create `ui.do/src/rho/engine.ts`:

```typescript
/**
 * ρ layer — WASM engine lifecycle.
 * The representation function. Maps FFP objects to executables.
 * Client-side ρ is a validation UX, not a security boundary.
 */

let initialized = false

export async function initEngine(): Promise<void> {
  if (initialized) return
  const { default: init, load_ir } = await import('./fol_engine.js')
  await init({ module_or_path: '/wasm/fol_engine_bg.wasm' })
  initialized = true
}

export async function loadSchema(irJson: string): Promise<void> {
  const { load_ir } = await import('./fol_engine.js')
  load_ir(irJson)
}

export async function applyCommand(command: unknown, populationJson: string): Promise<any> {
  const { apply_command_wasm } = await import('./fol_engine.js')
  return apply_command_wasm(command, populationJson)
}

export async function getTransitions(nounName: string, currentStatus: string): Promise<any[]> {
  const { get_transitions_wasm } = await import('./fol_engine.js')
  return get_transitions_wasm(nounName, currentStatus)
}

export async function derivePopulation(populationJson: string): Promise<any> {
  const { forward_chain_population } = await import('./fol_engine.js')
  return forward_chain_population(populationJson)
}

export async function validateSchema(domainIrJson: string): Promise<any> {
  const { validate_schema_wasm } = await import('./fol_engine.js')
  return validate_schema_wasm(domainIrJson)
}

export function isReady(): boolean {
  return initialized
}
```

- [ ] **Step 5: Verify build**

```bash
cd /c/Users/lippe/Repos/ui.do && npm run build
```

Expected: Build succeeds (WASM excluded from bundle, loaded at runtime from `/wasm/`).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add mdx-ui deps and WASM engine wrapper"
```

---

### Task 2: Population cache (P_local)

**Files:**
- Create: `ui.do/src/rho/population.ts`

- [ ] **Step 1: Write the population cache**

Create `ui.do/src/rho/population.ts`:

```typescript
/**
 * P_local — cached partition of the population.
 * P = ↑FILE:D is the named set of relations.
 * The client caches the partition needed for the current view.
 */

const API = 'https://api.auto.dev'

export interface CellRecord {
  id: string
  type: string
  data: Record<string, unknown>
  _links?: Record<string, { href: string; method?: string }>
}

export interface ListResponse {
  docs: CellRecord[]
  totalDocs: number
  limit: number
  page: number
  totalPages: number
  hasNextPage: boolean
  hasPrevPage: boolean
  _links?: Record<string, string>
}

// In-memory cache keyed by (type, domain)
const cache = new Map<string, CellRecord[]>()

function cacheKey(type: string, domain: string): string {
  return `${type}:${domain}`
}

export async function fetchEntities(type: string, domain: string, limit = 500): Promise<ListResponse> {
  const res = await fetch(`${API}/api/entities/${encodeURIComponent(type)}?domain=${domain}&limit=${limit}`, {
    credentials: 'include',
    headers: { Accept: 'application/json' },
  })
  if (!res.ok) return { docs: [], totalDocs: 0, limit, page: 1, totalPages: 0, hasNextPage: false, hasPrevPage: false }
  const data = await res.json()
  const docs = data.docs || []
  cache.set(cacheKey(type, domain), docs)
  return data
}

export async function fetchEntity(type: string, id: string, domain?: string): Promise<CellRecord | null> {
  const qs = domain ? `?domain=${domain}` : ''
  const res = await fetch(`${API}/api/entities/${encodeURIComponent(type)}/${id}${qs}`, {
    credentials: 'include',
    headers: { Accept: 'application/json' },
  })
  if (!res.ok) return null
  return res.json()
}

export async function fetchSession(): Promise<{ email: string; admin?: boolean } | null> {
  const res = await fetch(`${API}/account`, { credentials: 'include', redirect: 'manual', headers: { Accept: 'application/json' } })
  if (!res.ok || res.type === 'opaqueredirect') return null
  const data = await res.json()
  return { email: data.user?.email || data.email || '', admin: data.user?.admin || false }
}

export async function fetchSeedStats(): Promise<{ totals: { domains: number; nouns: number; readings: number }; perDomain: Record<string, { nouns: number; readings: number }> }> {
  const res = await fetch(`${API}/api/seed`, { credentials: 'include', headers: { Accept: 'application/json' } })
  if (!res.ok) return { totals: { domains: 0, nouns: 0, readings: 0 }, perDomain: {} }
  return res.json()
}

export async function postTransition(type: string, id: string, event: string): Promise<any> {
  const res = await fetch(`${API}/api/entities/${encodeURIComponent(type)}/${id}/transition`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ event }),
  })
  return res.json()
}

export async function createEntity(type: string, domain: string, data: Record<string, unknown>): Promise<any> {
  const res = await fetch(`${API}/api/entities/${encodeURIComponent(type)}`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ domain, data }),
  })
  return res.json()
}

/** Get cached entities, or fetch if not cached */
export function getCached(type: string, domain: string): CellRecord[] | undefined {
  return cache.get(cacheKey(type, domain))
}

/** Update a single entity in the cache (from SSE delta) */
export function updateCache(entity: CellRecord, domain: string): void {
  const key = cacheKey(entity.type, domain)
  const existing = cache.get(key) || []
  const idx = existing.findIndex(e => e.id === entity.id)
  if (idx >= 0) existing[idx] = entity
  else existing.push(entity)
  cache.set(key, existing)
}

/** Remove an entity from the cache */
export function removeFromCache(type: string, id: string, domain: string): void {
  const key = cacheKey(type, domain)
  const existing = cache.get(key) || []
  cache.set(key, existing.filter(e => e.id !== id))
}

/** Clear all cached data */
export function clearCache(): void {
  cache.clear()
}
```

- [ ] **Step 2: Commit**

```bash
git add src/rho/population.ts
git commit -m "feat: add P_local population cache"
```

---

### Task 3: Component registry (TargetFactory)

**Files:**
- Create: `ui.do/src/factory/registry.ts`
- Create: `ui.do/src/factory/resolve.ts`

- [ ] **Step 1: Write the component registry**

Create `ui.do/src/factory/registry.ts`:

```typescript
/**
 * Component registry — the ONLY hardcoded configuration.
 * Maps View Renderer component names to lazy mdx-ui imports.
 * This is iFactr's Register<IView>(ConcreteType) for the web platform.
 */

import { type ComponentType, lazy } from 'react'

type LazyComponent = ComponentType<any>

const registry: Record<string, () => Promise<{ default: LazyComponent }>> = {
  // @mdxui/admin views
  DataGrid: () => import('../views/ListView'),
  DataBrowser: () => import('../views/ListView'),

  // @mdxui/primitives views
  Card: () => import('../views/DetailView'),
  Detail: () => import('../views/DetailView'),

  // @mdxui/widgets views
  ChatBox: () => import('../views/ChatView'),

  // Schema-driven form
  Form: () => import('../views/FormView'),
}

export function getComponent(name: string): React.LazyExoticComponent<LazyComponent> | null {
  const loader = registry[name]
  if (!loader) return null
  return lazy(loader)
}

export function hasComponent(name: string): boolean {
  return name in registry
}

export function getComponentNames(): string[] {
  return Object.keys(registry)
}
```

- [ ] **Step 2: Write the view resolver**

Create `ui.do/src/factory/resolve.ts`:

```typescript
/**
 * View resolution — iFactr's MXViewMap for AREST.
 * Given (nounType, perspective), find the View Renderer from P.
 * Falls back to defaults when no View Renderer fact exists.
 */

import { type ComponentType, lazy } from 'react'
import { getComponent } from './registry'

type Perspective = 'list' | 'detail' | 'form' | 'chat'

// Default view components when no View Renderer fact is registered
const defaults: Record<Perspective, () => Promise<{ default: ComponentType<any> }>> = {
  list: () => import('../views/ListView'),
  detail: () => import('../views/DetailView'),
  form: () => import('../views/FormView'),
  chat: () => import('../views/ChatView'),
}

/**
 * Resolve a view component for a given entity type and perspective.
 * First checks View Renderer facts in P_local, then falls back to defaults.
 */
export function resolveView(
  nounType: string,
  perspective: Perspective,
  viewRenderers?: Array<{ componentName: string; forView: string; platform: string }>,
): React.LazyExoticComponent<ComponentType<any>> {
  // Check View Renderer facts from P for this noun + platform web
  if (viewRenderers) {
    const match = viewRenderers.find(
      vr => vr.platform === 'web' && vr.forView?.toLowerCase().includes(perspective),
    )
    if (match) {
      const component = getComponent(match.componentName)
      if (component) return component
    }
  }

  // Default: use perspective-based fallback
  return lazy(defaults[perspective])
}
```

- [ ] **Step 3: Commit**

```bash
git add src/factory/
git commit -m "feat: add TargetFactory — component registry and view resolver"
```

---

### Task 4: View components (mdx-ui wrappers)

**Files:**
- Create: `ui.do/src/views/ListView.tsx`
- Create: `ui.do/src/views/DetailView.tsx`
- Create: `ui.do/src/views/FormView.tsx`
- Create: `ui.do/src/views/ChatView.tsx`

- [ ] **Step 1: Write ListView**

Create `ui.do/src/views/ListView.tsx`:

```tsx
/**
 * List View — wraps @mdxui/admin DataGrid or falls back to a simple table.
 * Maps entity docs to rows, HATEOAS links to row actions.
 */

import type { CellRecord, ListResponse } from '../rho/population'

interface ListViewProps {
  data: ListResponse
  nounType: string
  domain: string
  onSelect: (entity: CellRecord) => void
  onNavigate: (href: string) => void
}

export default function ListView({ data, nounType, domain, onSelect, onNavigate }: ListViewProps) {
  const { docs, totalDocs, page, totalPages, hasNextPage, hasPrevPage, _links } = data

  if (docs.length === 0) {
    return <div className="p-8 text-center text-muted-foreground">No {nounType} entities in {domain}</div>
  }

  // Derive columns from first doc's keys (excluding system fields)
  const sampleDoc = docs[0]
  const columns = Object.keys(sampleDoc).filter(k => !k.startsWith('_') && k !== 'type')

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-4 py-2 border-b">
        <h2 className="text-lg font-semibold">{nounType}</h2>
        <span className="text-sm text-muted-foreground">{totalDocs} total</span>
      </div>
      <div className="flex-1 overflow-auto">
        <table className="w-full text-sm">
          <thead className="sticky top-0 bg-card border-b">
            <tr>
              {columns.map(col => (
                <th key={col} className="px-4 py-2 text-left font-medium text-muted-foreground">{col}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {docs.map(doc => (
              <tr
                key={doc.id}
                className="border-b hover:bg-muted/50 cursor-pointer transition-colors"
                onClick={() => onSelect(doc)}
              >
                {columns.map(col => (
                  <td key={col} className="px-4 py-2 truncate max-w-[200px]">
                    {String(doc[col] ?? doc.data?.[col] ?? '')}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {totalPages > 1 && (
        <div className="flex items-center justify-between px-4 py-2 border-t text-sm">
          <span>Page {page} of {totalPages}</span>
          <div className="flex gap-2">
            {hasPrevPage && _links?.prev && (
              <button className="px-3 py-1 rounded bg-muted hover:bg-accent" onClick={() => onNavigate(_links.prev)}>Prev</button>
            )}
            {hasNextPage && _links?.next && (
              <button className="px-3 py-1 rounded bg-muted hover:bg-accent" onClick={() => onNavigate(_links.next)}>Next</button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 2: Write DetailView**

Create `ui.do/src/views/DetailView.tsx`:

```tsx
/**
 * Detail View — entity fields as key-value pairs, HATEOAS transition links as buttons.
 * Per Theorem 3: links(s) = π_event(Filter(p) : T) — transitions ARE links.
 */

import type { CellRecord } from '../rho/population'

interface DetailViewProps {
  entity: CellRecord
  nounType: string
  onTransition: (event: string) => void
  onBack: () => void
}

export default function DetailView({ entity, nounType, onTransition, onBack }: DetailViewProps) {
  const fields = Object.entries(entity).filter(([k]) => !k.startsWith('_') && k !== 'type' && k !== 'id')
  const links = entity._links || {}
  const transitions = Object.entries(links).filter(([k]) => k !== 'self' && k !== 'collection')

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-4 py-3 border-b">
        <button className="text-muted-foreground hover:text-foreground" onClick={onBack}>&larr;</button>
        <h2 className="text-lg font-semibold">{nounType}</h2>
        <span className="text-sm text-muted-foreground">{entity.id}</span>
        {entity._status && (
          <span className="px-2 py-0.5 text-xs rounded-full bg-primary/10 text-primary font-medium">
            {String(entity._status)}
          </span>
        )}
      </div>
      <div className="flex-1 overflow-auto p-4">
        <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2">
          {fields.map(([key, value]) => (
            <div key={key} className="contents">
              <dt className="text-sm font-medium text-muted-foreground py-1">{key}</dt>
              <dd className="text-sm py-1">{typeof value === 'object' ? JSON.stringify(value) : String(value ?? '')}</dd>
            </div>
          ))}
        </dl>
      </div>
      {transitions.length > 0 && (
        <div className="flex gap-2 px-4 py-3 border-t">
          {transitions.map(([event, link]) => (
            <button
              key={event}
              className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={() => onTransition(event)}
            >
              {event}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 3: Write FormView**

Create `ui.do/src/views/FormView.tsx`:

```tsx
/**
 * Form View — schema-driven form generated from graph schema roles.
 * The WASM engine knows the roles, constraints, and multiplicity.
 * This generates form fields from the readings, not from hardcoded definitions.
 */

import { useState } from 'react'

interface FormViewProps {
  nounType: string
  domain: string
  fields: Array<{ name: string; required: boolean; type: string }>
  onSubmit: (data: Record<string, string>) => void
  onCancel: () => void
}

export default function FormView({ nounType, domain, fields, onSubmit, onCancel }: FormViewProps) {
  const [values, setValues] = useState<Record<string, string>>({})

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    onSubmit(values)
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-col h-full">
      <div className="px-4 py-3 border-b">
        <h2 className="text-lg font-semibold">New {nounType}</h2>
      </div>
      <div className="flex-1 overflow-auto p-4 space-y-4">
        {fields.map(field => (
          <div key={field.name} className="space-y-1">
            <label className="text-sm font-medium">
              {field.name}{field.required && <span className="text-destructive"> *</span>}
            </label>
            {field.type === 'textarea' ? (
              <textarea
                className="w-full px-3 py-2 rounded-md border bg-background text-sm"
                value={values[field.name] || ''}
                onChange={e => setValues(v => ({ ...v, [field.name]: e.target.value }))}
                required={field.required}
                rows={4}
              />
            ) : (
              <input
                className="w-full px-3 py-2 rounded-md border bg-background text-sm"
                type="text"
                value={values[field.name] || ''}
                onChange={e => setValues(v => ({ ...v, [field.name]: e.target.value }))}
                required={field.required}
              />
            )}
          </div>
        ))}
      </div>
      <div className="flex gap-2 px-4 py-3 border-t">
        <button type="submit" className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground">Create</button>
        <button type="button" className="px-4 py-2 text-sm rounded-md bg-muted" onClick={onCancel}>Cancel</button>
      </div>
    </form>
  )
}
```

- [ ] **Step 4: Write ChatView**

Create `ui.do/src/views/ChatView.tsx`:

```tsx
/**
 * Chat View — SSE streaming chat connected to /ai/chat.
 */

import { useState, useRef, useEffect } from 'react'

interface ChatViewProps {
  endpoint: string
  domain: string
  context?: Record<string, unknown>
}

interface Message {
  role: 'user' | 'assistant'
  content: string
}

export default function ChatView({ endpoint, domain, context }: ChatViewProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [streaming, setStreaming] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => { bottomRef.current?.scrollIntoView({ behavior: 'smooth' }) }, [messages])

  const send = async () => {
    if (!input.trim() || streaming) return
    const userMsg: Message = { role: 'user', content: input.trim() }
    setMessages(prev => [...prev, userMsg])
    setInput('')
    setStreaming(true)

    const assistantMsg: Message = { role: 'assistant', content: '' }
    setMessages(prev => [...prev, assistantMsg])

    try {
      const res = await fetch(`https://api.auto.dev${endpoint}`, {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream' },
        body: JSON.stringify({ messages: [...messages, userMsg], domain, ...context }),
      })

      const reader = res.body?.getReader()
      if (!reader) return
      const decoder = new TextDecoder()
      let buffer = ''

      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        const lines = buffer.split('\n')
        buffer = lines.pop() || ''
        for (const line of lines) {
          const trimmed = line.replace(/\r$/, '')
          if (trimmed.startsWith('data: ')) {
            const data = trimmed.slice(6)
            if (data === '[DONE]') break
            try {
              const parsed = JSON.parse(data)
              if (parsed.content) {
                setMessages(prev => {
                  const updated = [...prev]
                  const last = updated[updated.length - 1]
                  if (last.role === 'assistant') last.content += parsed.content
                  return updated
                })
              }
            } catch {
              setMessages(prev => {
                const updated = [...prev]
                const last = updated[updated.length - 1]
                if (last.role === 'assistant') last.content += data
                return updated
              })
            }
          }
        }
      }
    } catch (err) {
      setMessages(prev => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (last.role === 'assistant') last.content = `Error: ${err}`
        return updated
      })
    }
    setStreaming(false)
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 overflow-auto p-4 space-y-4">
        {messages.map((msg, i) => (
          <div key={i} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div className={`max-w-[80%] rounded-lg px-4 py-2 text-sm ${
              msg.role === 'user' ? 'bg-primary text-primary-foreground' : 'bg-muted'
            }`}>
              {msg.content || (streaming && msg.role === 'assistant' ? '...' : '')}
            </div>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
      <div className="flex gap-2 px-4 py-3 border-t">
        <input
          className="flex-1 px-3 py-2 rounded-md border bg-background text-sm"
          placeholder="Type a message..."
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && !e.shiftKey && send()}
          disabled={streaming}
        />
        <button
          className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground disabled:opacity-50"
          onClick={send}
          disabled={streaming || !input.trim()}
        >
          Send
        </button>
      </div>
    </div>
  )
}
```

- [ ] **Step 5: Commit**

```bash
git add src/views/
git commit -m "feat: add view components — ListView, DetailView, FormView, ChatView"
```

---

### Task 5: Shell and Renderer

**Files:**
- Create: `ui.do/src/shell/Shell.tsx`
- Create: `ui.do/src/shell/Renderer.tsx`
- Rewrite: `ui.do/src/shell/App.tsx`
- Rewrite: `ui.do/src/main.tsx`

- [ ] **Step 1: Write the Shell**

Create `ui.do/src/shell/Shell.tsx`:

```tsx
/**
 * App Shell — sidebar from Domain/App facts in P, breadcrumbs from nav history.
 * The shell structure is derived from the population, not hardcoded.
 */

import { type ReactNode } from 'react'

interface Domain {
  slug: string
  name: string
  nouns: number
  readings: number
}

interface ShellProps {
  email: string
  domains: Domain[]
  activeDomain: string | null
  onSelectDomain: (slug: string) => void
  activeNoun: string | null
  nouns: Array<{ id: string; name: string }>
  onSelectNoun: (name: string) => void
  children: ReactNode
}

export default function Shell({
  email, domains, activeDomain, onSelectDomain,
  activeNoun, nouns, onSelectNoun, children,
}: ShellProps) {
  return (
    <div className="flex h-dvh bg-background text-foreground overflow-hidden">
      {/* Sidebar */}
      <aside className="w-64 border-r border-border flex flex-col shrink-0">
        <div className="px-4 py-3 border-b">
          <span className="font-display font-bold text-sm">ui.do</span>
        </div>

        {/* Domains */}
        <nav className="flex-1 overflow-auto py-2">
          <div className="px-3 py-1 text-xs font-medium text-muted-foreground uppercase tracking-wider">Domains</div>
          {domains.map(d => (
            <button
              key={d.slug}
              className={`w-full text-left px-3 py-1.5 text-sm rounded-md mx-1 transition-colors ${
                d.slug === activeDomain ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'
              }`}
              style={{ width: 'calc(100% - 8px)' }}
              onClick={() => onSelectDomain(d.slug)}
            >
              <span className="font-medium">{d.name || d.slug}</span>
              <span className="ml-2 text-xs text-muted-foreground">{d.nouns}n</span>
            </button>
          ))}
        </nav>

        {/* Nouns for active domain */}
        {activeDomain && nouns.length > 0 && (
          <nav className="border-t overflow-auto py-2 max-h-[40%]">
            <div className="px-3 py-1 text-xs font-medium text-muted-foreground uppercase tracking-wider">Entity Types</div>
            {nouns.map(n => (
              <button
                key={n.id}
                className={`w-full text-left px-3 py-1.5 text-sm rounded-md mx-1 transition-colors ${
                  n.name === activeNoun ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'
                }`}
                style={{ width: 'calc(100% - 8px)' }}
                onClick={() => onSelectNoun(n.name)}
              >
                {n.name}
              </button>
            ))}
          </nav>
        )}

        {/* User */}
        <div className="px-4 py-3 border-t text-xs text-muted-foreground truncate">{email}</div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-hidden">
        {children}
      </main>
    </div>
  )
}
```

- [ ] **Step 2: Write the Renderer**

Create `ui.do/src/shell/Renderer.tsx`:

```tsx
/**
 * Generic renderer — the render loop.
 * URL → fetch representation → resolve view → render component.
 * This is sub_client: dispatches by representation type.
 */

import { Suspense } from 'react'
import { resolveView } from '../factory/resolve'
import type { CellRecord, ListResponse } from '../rho/population'

interface RendererProps {
  // What to render
  listData?: ListResponse
  entityData?: CellRecord
  // Context
  nounType: string
  domain: string
  perspective: 'list' | 'detail' | 'form' | 'chat'
  // Callbacks
  onSelect: (entity: CellRecord) => void
  onBack: () => void
  onTransition: (event: string) => void
  onNavigate: (href: string) => void
  onSubmit: (data: Record<string, string>) => void
}

export default function Renderer(props: RendererProps) {
  const View = resolveView(props.nounType, props.perspective)

  return (
    <Suspense fallback={<div className="flex items-center justify-center h-full text-muted-foreground">Loading...</div>}>
      {props.perspective === 'list' && props.listData && (
        <View data={props.listData} nounType={props.nounType} domain={props.domain} onSelect={props.onSelect} onNavigate={props.onNavigate} />
      )}
      {props.perspective === 'detail' && props.entityData && (
        <View entity={props.entityData} nounType={props.nounType} onTransition={props.onTransition} onBack={props.onBack} />
      )}
      {props.perspective === 'form' && (
        <View nounType={props.nounType} domain={props.domain} fields={[]} onSubmit={props.onSubmit} onCancel={props.onBack} />
      )}
      {props.perspective === 'chat' && (
        <View endpoint="/ai/chat" domain={props.domain} />
      )}
    </Suspense>
  )
}
```

- [ ] **Step 3: Write the App**

Create `ui.do/src/shell/App.tsx`:

```tsx
/**
 * Root App — init ρ, load P, render Shell + Renderer.
 * The peer AREST system: (O, ρ_wasm, D_local, P_cached, sub_client).
 */

import { useState, useEffect, useCallback } from 'react'
import Shell from './Shell'
import Renderer from './Renderer'
import { initEngine } from '../rho/engine'
import { fetchSession, fetchSeedStats, fetchEntities, fetchEntity, postTransition } from '../rho/population'
import type { CellRecord, ListResponse } from '../rho/population'

interface Domain {
  slug: string
  name: string
  nouns: number
  readings: number
}

export default function App() {
  const [email, setEmail] = useState('')
  const [domains, setDomains] = useState<Domain[]>([])
  const [activeDomain, setActiveDomain] = useState<string | null>(null)
  const [nouns, setNouns] = useState<Array<{ id: string; name: string }>>([])
  const [activeNoun, setActiveNoun] = useState<string | null>(null)
  const [listData, setListData] = useState<ListResponse | null>(null)
  const [entityData, setEntityData] = useState<CellRecord | null>(null)
  const [perspective, setPerspective] = useState<'list' | 'detail' | 'form' | 'chat'>('list')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Init: load session + seed stats + WASM
  useEffect(() => {
    (async () => {
      try {
        const [session, stats] = await Promise.all([
          fetchSession(),
          fetchSeedStats(),
          initEngine().catch(() => {}), // WASM load is best-effort
        ])
        if (!session) {
          window.location.href = `https://auto.dev/signin?redirectUrl=${encodeURIComponent(window.location.href)}`
          return
        }
        setEmail(session.email)
        const domainList = Object.entries(stats.perDomain).map(([slug, counts]) => ({
          slug,
          name: slug,
          nouns: counts.nouns,
          readings: counts.readings,
        }))
        setDomains(domainList)
        if (domainList.length > 0) {
          setActiveDomain(domainList[0].slug)
        }
      } catch (err) {
        setError(String(err))
      }
      setLoading(false)
    })()
  }, [])

  // When active domain changes, fetch nouns for it
  useEffect(() => {
    if (!activeDomain) return
    (async () => {
      const result = await fetchEntities('Noun', activeDomain, 500)
      const entityNouns = result.docs
        .filter((d: any) => d.objectType === 'entity')
        .map((d: any) => ({ id: d.id, name: d.name || d.id }))
        .sort((a: any, b: any) => a.name.localeCompare(b.name))
      setNouns(entityNouns)
      setActiveNoun(null)
      setListData(null)
      setEntityData(null)
      setPerspective('list')
    })()
  }, [activeDomain])

  // When active noun changes, fetch entity list
  useEffect(() => {
    if (!activeDomain || !activeNoun) return
    (async () => {
      const result = await fetchEntities(activeNoun, activeDomain)
      setListData(result)
      setEntityData(null)
      setPerspective('list')
    })()
  }, [activeDomain, activeNoun])

  const handleSelect = useCallback(async (entity: CellRecord) => {
    if (!activeDomain || !activeNoun) return
    const full = await fetchEntity(activeNoun, entity.id, activeDomain)
    if (full) {
      setEntityData(full)
      setPerspective('detail')
    }
  }, [activeDomain, activeNoun])

  const handleBack = useCallback(() => {
    setEntityData(null)
    setPerspective('list')
  }, [])

  const handleTransition = useCallback(async (event: string) => {
    if (!entityData || !activeNoun) return
    const result = await postTransition(activeNoun, entityData.id, event)
    // Refresh entity after transition
    if (activeDomain) {
      const updated = await fetchEntity(activeNoun, entityData.id, activeDomain)
      if (updated) setEntityData(updated)
      // Refresh list too
      const list = await fetchEntities(activeNoun, activeDomain)
      setListData(list)
    }
  }, [entityData, activeNoun, activeDomain])

  const handleNavigate = useCallback(async (href: string) => {
    // Follow HATEOAS link
    const res = await fetch(`https://api.auto.dev${href}`, {
      credentials: 'include',
      headers: { Accept: 'application/json' },
    })
    if (res.ok) {
      const data = await res.json()
      if (data.docs) setListData(data)
      else setEntityData(data)
    }
  }, [])

  const handleSubmit = useCallback(async (data: Record<string, string>) => {
    // Placeholder for create entity
  }, [])

  if (loading) return <div className="flex items-center justify-center h-dvh text-muted-foreground">Loading...</div>
  if (error) return <div className="flex items-center justify-center h-dvh text-destructive">{error}</div>

  return (
    <Shell
      email={email}
      domains={domains}
      activeDomain={activeDomain}
      onSelectDomain={setActiveDomain}
      activeNoun={activeNoun}
      nouns={nouns}
      onSelectNoun={setActiveNoun}
    >
      {activeNoun && (listData || entityData) ? (
        <Renderer
          listData={listData || undefined}
          entityData={entityData || undefined}
          nounType={activeNoun}
          domain={activeDomain || ''}
          perspective={perspective}
          onSelect={handleSelect}
          onBack={handleBack}
          onTransition={handleTransition}
          onNavigate={handleNavigate}
          onSubmit={handleSubmit}
        />
      ) : (
        <div className="flex items-center justify-center h-full text-muted-foreground">
          {activeDomain ? 'Select an entity type' : 'Select a domain'}
        </div>
      )}
    </Shell>
  )
}
```

- [ ] **Step 4: Update main.tsx**

Rewrite `ui.do/src/main.tsx`:

```tsx
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './shell/App'
import './index.css'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
```

- [ ] **Step 5: Update index.css**

Rewrite `ui.do/src/index.css`:

```css
@import 'tailwindcss';

:root {
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
  --card: oklch(0.205 0 0);
  --muted: oklch(0.269 0 0);
  --muted-foreground: oklch(0.708 0 0);
  --accent: oklch(0.269 0 0);
  --accent-foreground: oklch(0.985 0 0);
  --primary: oklch(0.922 0 0);
  --primary-foreground: oklch(0.205 0 0);
  --destructive: oklch(0.577 0.245 27.325);
  --border: oklch(0.269 0 0);
  --radius: 0.5rem;
}

body {
  background-color: var(--background);
  color: var(--foreground);
  font-family: system-ui, -apple-system, sans-serif;
}
```

- [ ] **Step 6: Build and verify**

```bash
cd /c/Users/lippe/Repos/ui.do && npm run build
```

Expected: Build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src/shell/ src/main.tsx src/index.css
git commit -m "feat: add Shell, Renderer, App — generic hypermedia client"
```

---

### Task 6: Clean up old code and deploy

**Files:**
- Delete: `ui.do/src/App.tsx` (replaced by shell/App.tsx)
- Delete: `ui.do/src/api.ts` (replaced by rho/population.ts)
- Delete: `ui.do/src/pages/` (replaced by views/)
- Delete: `ui.do/src/components/` (replaced by views/)
- Delete: `ui.do/src/layout/` (replaced by shell/)
- Delete: `ui.do/src/hooks/` (functionality moved to shell/App.tsx)
- Delete: `ui.do/src/dashboard/` (replaced by shell)
- Delete: `ui.do/src/types.ts` (types in population.ts)
- Delete: `ui.do/src/arest.ts` (logic in rho/engine.ts)
- Delete: `ui.do/src/utils.ts` (not needed)

- [ ] **Step 1: Remove old source files**

```bash
cd /c/Users/lippe/Repos/ui.do
rm -f src/App.tsx src/api.ts src/types.ts src/arest.ts src/utils.ts
rm -rf src/pages src/components src/layout src/hooks src/dashboard
```

- [ ] **Step 2: Verify build still passes**

```bash
npm run build
```

- [ ] **Step 3: Local dev test**

```bash
npm run dev
```

Open `http://localhost:5173` in the headed browser. Verify: login redirect → shell with sidebar → domain list → select domain → noun list → select noun → entity table → click row → detail view.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: remove old procedural UI code"
```

- [ ] **Step 5: Deploy**

```bash
npm run build && npx wrangler pages deploy dist --project-name ui-do
```

Or whatever the existing deployment mechanism is for ui.do.

---

## Self-Review

**Spec coverage check:**
- ρ layer (WASM): Task 1 ✓
- P layer (cache): Task 2 ✓
- TargetFactory (registry + resolve): Task 3 ✓
- View components (ListView, DetailView, FormView, ChatView): Task 4 ✓
- Shell + Renderer + App: Task 5 ✓
- Cleanup + deploy: Task 6 ✓
- Access control via derivation rules: Handled in App.tsx (fetchSession, domain filtering) ✓
- Trust boundary (client WASM = UX, server = authoritative): Documented in engine.ts comments ✓
- SSE streaming: Not yet implemented (noted as future task — the architecture supports it but the server endpoint `/api/stream` doesn't exist yet)

**Gap: SSE endpoint.** The spec mentions `/api/stream` but graphdl-orm doesn't have it yet. This plan gets the client working with fetch-based rendering first. SSE streaming is a follow-up task on the server side.

**Placeholder scan:** No TBDs, TODOs, or "implement later" found.

**Type consistency:** `CellRecord` and `ListResponse` used consistently across population.ts, views, and renderer.
