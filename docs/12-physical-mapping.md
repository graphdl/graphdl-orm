# 12 · Canonical Physical Mapping

AREST's state `D` is a sequence of cells. The physical mapping question is: where does each cell live when the engine runs distributed? This document states the canonical answer, ties it to the paper (§5.2, §5.4, Definition 2), and anchors the worker-side code that implements it.

## One DO per cell

**The physical mapping is one Durable Object per cell.** Each entity is a cell (paper §5.2). One DO instance holds one entity's 3NF row — the complete set of facts that depend on its reference scheme.

This is not an implementation detail we're free to change. It's the direct operational form of Definition 2 (Cell Isolation): "For each cell `⟨CELL, n, c⟩` in D, at most one μ application that writes to n may be in progress at any time." A distributed system realises that by giving each cell its own writer — one DO per cell, with the DO runtime serialising concurrent requests to the same instance.

The older per-(scope, domain) form — one DO per domain in the registry — still exists, but only as an **index**: `RegistryDB` holds the population index and schema cache per (scope, domain), and it IS per-scope, not per-cell. The per-cell layer is `EntityDB` — one DO per entity.

## Cell naming

Cells are named by `(nounType, entityId)`. The worker computes the DO key through `cellKey()` (`src/api/cell-key.ts`):

```typescript
cellKey('Organization', 'org-1')  // => 'Organization:org-1'
cellKey('Event', '2026-04-18T12:00:00Z')  // compound keys survive
```

Prefixing the noun type scopes the DO namespace: two entities of different types with the same id never collide onto the same DO. That matches RMAP — each 3NF table has its own primary-key space — and matches Definition 2 — distinct cells get distinct writers.

`parseCellKey(key)` is the inverse, returning `null` for legacy raw-UUID keys so callers know when to fall back to a Registry lookup for the type.

## Signal-delivery demux

Event subscribers receive a per-cell substream via paper Eq. 11:

```
E_n = Filter(eq ∘ [RMAP, n̄]) : E
```

`BroadcastDO` implements this filter: every published `CellEvent` carries its canonical `cellKey`, and `SubscriptionFilter.cellKey` is an O(1) match axis. A client that subscribes with `?cellKey=Order:ord-7` sees exactly that cell's substream — no more, no less. The `noun` / `entityId` pair stays for historical callers that filter loosely, but new code should prefer `cellKey` since it is the single string identity for a cell.

## Why not one DO per domain?

Earlier iterations kept a single `RegistryDB` per scope, with all entity writes flowing through it. That architecture works but violates Definition 2: two writers on disjoint entities in the same domain contend on the scope's single lock. As soon as the deployment grew beyond a handful of entities per scope, throughput saturated.

The migration to per-cell routing was tracked through the following commits (see `git log --grep=cellKey`):

| Task | Delivered                                                                    |
|------|------------------------------------------------------------------------------|
| #205 | Move the registry to per-(scope, domain). Cut per-scope contention first.    |
| #217 | Thread `cellKey(nounType, entityId)` through DO routing. Make the RMAP rule explicit in one place. |
| #220 | Per-cell event demux: `CellEvent.cellKey` + `SubscriptionFilter.cellKey`.    |
| #221 | This document.                                                               |

Per-domain registries still exist for the scope-level concerns (population index, schema cache, snapshot storage). Those are appropriate to that granularity — a population index IS per-domain. What's per-cell is every entity write.

## Query path

List queries (`GET /${plural}`) fan out across the matching entity DOs through the Registry's `entity_index`. Each entity DO responds with its own 3NF row. The worker merges and returns the full collection.

The recent OpenAPI surface (#218) advertises sort + order query parameters enumerated over each noun's RMAP columns, so a client can request `?sort=createdAt&order=desc` and the worker applies that ordering across the fanned-out reads. The sort is a post-fanout merge step — each DO has no awareness of sibling cells.

## Consequences

- **Horizontal scaling**: adding cells to the cluster scales SYSTEM without changing its definition (paper Eq. 12). Cloudflare's scheduler places DOs across the edge; AREST reaches all of them through the single `cellKey` primitive.
- **Browser peers**: a browser runtime can carry its own set of cells locally, fold events against them at zero latency, and sync asynchronously — the paper's §distributed evaluation. The same `cellKey` function runs in the browser to locate the local cell.
- **FPGA peers**: per-cell BRAM per #166 matches the per-DO model exactly — one memory unit per cell, one state machine per cell, all addressed by the same cell name.

See also: `08-federation.md` for how external systems appear as federated cells alongside local ones.
