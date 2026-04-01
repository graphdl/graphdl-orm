# Federation Light-Loading Design

## Context

The AREST system has 39 seeded domains with instance facts browsable in the UI. External Systems (ClickHouse, Edmunds, auto.dev product API, auth.vin, etc.) are declared in readings with Base URLs and Secret References. The engine needs to resolve backed nouns from these External Systems on demand without caching or indexing live data.

The paper defines the system function as:

```
SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
```

The entity is fetched from D. ρ resolves it based on its type and the definitions in DEFS. The operation (read, create, update, delete) is applied as the operand. The entity handles dispatch, not the system function. A local entity applies read by selecting attributes from P. A backed entity applies read by calling httpFetch.

## Architecture

### List Request (light-loading)

The UI sends GET /arest/entities/:noun?domain=X&page=1&limit=20.

1. The engine looks up the noun in the IR. If the noun is backed by an External System, resolve calls httpFetch with the External System's Base URL, the noun path, and page/limit params.
2. The External System returns a page of results. These populate P transiently for this request only. No store.
3. The representation is emitted with pagination metadata (totalDocs, page, totalPages, hasNextPage, hasPrevPage) and HATEOAS links.
4. The response is discarded after emit. The next request fetches fresh.

### Detail Request (full detail)

The UI sends GET /arest/entities/:noun/:id?domain=X.

1. The engine resolves the entity. For a backed noun, httpFetch calls the External System's detail endpoint for the specific ID.
2. The full entity data populates P transiently. The representation includes all fields, constraints, HATEOAS links, and view metadata.
3. The response is discarded after emit.

### Acting on External Data

If the user wants to act on an external entity (create an incident, attach a note, trigger a state transition), the operation is create or update. Now store fires. The data enters D permanently as a local cell. The entity becomes a local fact that the engine can evaluate constraints and derivation rules over.

This is the boundary between browsing and acting. Browse = fetch, display, discard. Act = fetch, store, derive.

### Derivation Traces

The paper says derivation chains are recorded and available on demand as part of the representation. When the forward chainer evaluates to lfp, the trace of which rules fired, in what order, producing which facts, is queryable. This is not a transient debug log. It is a first-class part of the system.

For the support app, this means the engine can show how it reached a conclusion. "External System 'edmunds' has Service Health Status 'degraded' because Error Rate (47 in 5 minutes) exceeded Error Threshold (10)." The LLM consumes conclusions. The human reads the trace to verify them.

The trace is a sequence of rule applications over P, each a ρ-application (Theorem 5). The UI renders it like any other entity data.

## What Determines "Backed"

The readings declare it:

```
Noun is backed by External System.
External System has Base URL.
Domain connects to External System with Secret Reference.
```

The entity's ρ resolves differently based on whether its noun is backed. The IR includes the backed_by relationship. The engine checks it during resolve. A local noun's ρ reads from the cell in D. A backed noun's ρ calls httpFetch.

## Pagination

The External System handles pagination. The UI sends page and limit as query params. These pass through as part of the input I in SYSTEM:x. The resolve step forwards them to the httpFetch call. The response includes totalDocs, hasNextPage, hasPrevPage. The UI renders pagination controls from this metadata.

No indexing of live data. No registry entries for external entities. The list is fetched fresh each time.

## No Cache

Browse = fetch, display, discard. The data is in P only for the duration of the request. No stale cache. No index of live data. If the user wants to pin the data (for analysis, incident creation, or derivation), they act on it and it becomes a local fact.

## Current State vs Target

### What the paper says the UI should be

The paper describes a browser runtime that registers render into DEFS. Facts bind to render via ρ. (ρ fact):render produces widgets. When a cell updates via store, the ρ-application re-evaluates and the bound widget fires. Event streaming is re-evaluation of ρ over updated D, not a separate subscription mechanism. The fol_engine WASM runs client-side for validation and derivation.

### What the UI does today

The UI is a React SPA that calls fetch() directly. It renders JSON from the API using procedural React components. It works as a thin client over the API, but it does not follow the paper's architecture.

| Paper says | Today |
|---|---|
| Facts bind to render via ρ | React components call fetch() and setState() |
| Navigation from HATEOAS links | Hardcoded route structure in App.tsx |
| View metadata from _view in representation | Partially works, component selection is procedural |
| Event streaming is ρ re-evaluation | No streaming, manual refresh |
| Browser runtime registers functions into DEFS | No WASM engine in browser, no DEFS |
| Derivation traces queryable in UI | Not implemented |

### Path forward

The API already follows the paper. The representations include _view, _links, and _nav metadata. The UI can migrate incrementally from procedural React to fact-bound rendering by:

1. Loading fol_engine WASM in the browser
2. Registering render functions into DEFS
3. Binding facts to widgets via ρ
4. Replacing fetch()/setState() with cell subscriptions
5. Rendering derivation traces as entity data

This is a separate effort from federation. Federation works today through the API layer regardless of how the UI renders.

## What Needs to Change

### Rust/WASM Engine (crates/fol-engine/)

1. NounDef needs backed_by field in types.rs
2. Parser recognizes "Noun is backed by External System" and sets backed_by on the NounDef
3. The IR includes the backing relationship so the engine knows which nouns resolve externally

### TypeScript Runtime (src/)

1. When a query targets a backed noun, the runtime calls httpFetch with the External System's Base URL, path, auth headers from Secret Reference, and page/limit params
2. The response is normalized to the entity format (id, type, data) and returned as the list or detail representation
3. No store for browse-only requests. Store only when an operation creates or updates

### Router (src/api/router.ts)

1. The entity listing endpoint checks if the noun is backed. If so, it calls the External System instead of the registry
2. The entity detail endpoint does the same for single-entity fetches
3. Pagination params (page, limit) pass through to the External System

### Readings

The External System entities and "backed by" facts are already seeded. The readings in core.md already declare the mechanism. No new readings needed for the federation infrastructure. Domain-specific readings (which nouns are backed by which systems) are declared per domain.

## What NOT to Build

- No cache layer for external data
- No registry index for external entities
- No background sync or polling
- No separate federation config. Readings drive everything
- No UI changes for federation. The API returns the same response shape
