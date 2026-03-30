# Hypermedia Client Design

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the procedural ui.do with a generic hypermedia renderer that follows the AREST whitepaper exactly. The client is a peer AREST system with client-side WASM (ρ), cached population (P), and mdx-ui components as View Renderers.

**Architecture:** Three layers — ρ (WASM engine), TargetFactory (View Renderer resolution from P via ρ), Render (mdx-ui components). No entity-specific code. The MonoCross/iFactr pattern applied to AREST: `Register<IView>(ConcreteType)` becomes View Renderer facts in P; NavigationMap becomes HATEOAS links; MXViewMap becomes θ₁ queries over P via ρ.

**Tech Stack:** React 19, Vite, fol_engine.wasm, @mdxui/primitives, @mdxui/app, @mdxui/admin, @mdxui/widgets, @mdxui/themes, SSE for streaming.

---

## 1. The Client as a Peer AREST System

The AREST tuple `(O, ρ, D, P, sub)` applies to the client:

- **O** — FFP objects (readings, constraints, derivation rules) fetched from the server as entity cells
- **ρ** — `fol_engine.wasm` loaded in the browser. Same engine as the server. Evaluates constraints, derives facts, compiles readings, runs state machine transitions.
- **D_local** — Cached cells in browser memory. Partitioned by RMAP from the schema's uniqueness constraints.
- **P_cached** — The local population. Fetched via `/api/entities/:type?domain=X`, updated via SSE deltas.
- **sub_client** — The render loop. Dispatches by representation type: is-list → DataGrid, is-entity → DetailView, is-transition → POST + optimistic update.

## 2. Data Flow

### Initial Load
1. Load `fol_engine.wasm` (async, cached by browser/service worker)
2. Fetch `/api/seed` for domain list
3. Fetch View Renderer facts: `/api/entities/ViewRenderer?domain=ui`
4. Load readings into WASM via `load_ir()` — compiled schema cached in browser
5. Fetch partition for current URL → render

### Navigation
- User clicks HATEOAS link → `fetch(link.href)` → new partition → re-render
- Master/detail: click entity in list → fetch single entity → render detail
- Transitions: click transition link → WASM validates locally → POST → update P_local

### Streaming (SSE)
- Client subscribes to `/api/stream?domain=X`
- Server sends: `{ type: 'created'|'updated'|'deleted', entity: CellContents }`
- Client updates P_local → re-derives via WASM → re-renders affected views

### Optimistic Mutations
- Client runs `create = emit ∘ validate ∘ derive ∘ resolve` via WASM
- No alethic violations → render optimistically, POST to server
- Server confirms → done. Server rejects → rollback P_local, show violation

### Trust Boundary
Client-side WASM is a **validation UX, not a security boundary**. The server runs the full `create` pipeline for every mutation independently. A client can bypass WASM and POST directly to the API — the server must reject invalid transitions and constraint violations on its own. Client ρ provides instant feedback and optimistic rendering; server ρ is authoritative.

## 3. TargetFactory: View Resolution from P

Following iFactr's `Register<IView>(ConcreteType)` pattern:

**Registration** = View Renderer facts in the population P:
```
View Renderer "support-list" is for List View.
View Renderer "support-list" is on Platform 'web'.
View Renderer "support-list" has component Name 'DataGrid'.
```

**Resolution** = θ₁ query over P via ρ:
Given `(nounType, perspective)`, query: "which View Renderer is for a View that displays Noun X on Platform 'web'?" This is `Filter(eq ∘ [s_noun, x̄]) : P_viewRenderers` — a standard restriction.

**Pairing** = component name → lazy import:
```typescript
const loaders: Record<string, () => Promise<Component>> = {
  DataGrid: () => import('@mdxui/admin/data-grid'),
  DataBrowser: () => import('@mdxui/admin/data-browser'),
  ChatBox: () => import('@mdxui/widgets/chatbox'),
  AppShell: () => import('@mdxui/app'),
  Card: () => import('@mdxui/primitives').then(m => m.Card),
  // ~15 more entries
}
```

This is the ONLY hardcoded configuration. Everything else comes from readings.

**Defaults** when no View Renderer fact exists:
- Entity list → DataGrid
- Single entity → Card with key-value pairs
- Entity with `_status` → Badge + transition buttons
- Entity with `chatEndpoint` → ChatBox

## 4. File Structure

```
src/
  rho/
    engine.ts       — WASM loader: loadSchema(), evaluate(), derive(), applyCommand()
    population.ts   — P_local cache: fetch partitions, merge SSE deltas, query()
    stream.ts       — SSE client for /api/stream
  factory/
    registry.ts     — Component name → lazy import (the only hardcoded config)
    resolve.ts      — Query ρ for View Renderer facts, return component loader
  shell/
    App.tsx          — AppShell, sidebar from Domain/App facts, breadcrumbs
    Renderer.tsx     — Generic render loop: URL → fetch → resolve view → render
    defaults.ts      — Default renderers for unregistered entity types
  views/
    ListView.tsx     — Wraps @mdxui/admin DataGrid. Maps docs to rows, links to actions.
    DetailView.tsx   — Wraps @mdxui/primitives Card. Fields to key-value, transitions to buttons.
    ChatView.tsx     — Wraps @mdxui/widgets ChatBox. Connects to /ai/chat SSE.
    FormView.tsx     — Schema-driven form from graph schema roles + constraints via WASM.
```

~10 files, each under 100 lines. Complexity lives in ρ and P, not client code.

## 5. Access Control

Already modeled in `organizations.md` readings:

- "User has Org Role in Organization"
- "User can access Domain iff User has Org Role in Organization AND Domain belongs to Organization"
- "User can access Domain if Domain has Visibility 'public'"
- "User can view only own Resource in App iff User has Org Role 'member'"

The client evaluates these derivation rules via WASM on login. Sidebar shows only accessible domains/apps.

**Routing by hostname:**
- `ui.auto.dev` → admin shell, all domains for user's org
- `support.auto.dev` → support app, only support domain's navigable views

Same client, same WASM, same P. Hostname determines the initial App fact. App fact determines navigable domains. Derivation rules determine visible resources.

## 6. Dependencies

**Add to ui.do:**
- `@mdxui/primitives` `@mdxui/app` `@mdxui/admin` `@mdxui/widgets` `@mdxui/themes` (from dot-do/ui monorepo, link or publish)
- `fol_engine` WASM (copy from graphdl-orm build output)

**Server-side requirement:**
- SSE endpoint `/api/stream?domain=X` on graphdl-orm (new — entity change events)
- View Renderer instance facts seeded in the `ui` domain (new — register mdx-ui components)
