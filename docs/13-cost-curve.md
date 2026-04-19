# 13 · Platform-Binding Cost Curve

This document defines the measurement methodology for validating the per-cell DO architecture (§5.4, Definition 2) at listings-scale volume (20M rows). Tracked as task #222.

## What the curve answers

Three operational questions:

1. **Write latency** under concurrent pressure: does the per-cell DO isolation really let disjoint-cell writers run without contention, all the way up to the tail?
2. **Cost per 1M writes** as the tenant grows: does fanning out to a per-cell DO add storage / Durable Object invocation overhead that undoes the parallelism win?
3. **Read-path crossover point**: at what volume does a fanned-out list query across per-cell DOs start losing to the analytics-backed populate (#219)?

Each is a line on the curve — write-latency, write-cost, and list-latency against (cells, concurrency).

## Fixtures

One tenant, one App, one canonical noun (`Listing`) with a realistic RMAP:

- 8 scalar columns (id, title, price, status, created_at, updated_at, seller_id, category).
- 2 foreign keys (Seller, Category) → separate DOs.
- 1 state machine (`Listing` with statuses `draft → pending → active → sold`).
- 2 derivation rules keyed off Listing (price bands, freshness band).

This matches the domain shape the framework is typically exercised against — intentionally not a micro-bench.

## Test volumes

- **1K cells**: sanity run, should complete in minutes.
- **100K cells**: CI-gated run, should finish in under an hour with real Workers.
- **1M cells**: scale run, Cloudflare-only (local SQLite at this size stops being representative).
- **20M cells**: the target volume per #222. Requires the production Workers + DO plan; not reachable from a free tier.

All four volumes run the same test program so the curve is continuous.

## Test program

For each volume N:

1. Seed — populate N `Listing` entities via `POST /api/listings`. Parallelise across the worker fleet. Record per-entity RPS and p50/p95/p99 latency.
2. Write fan — fire M concurrent PATCH requests, each to a distinct Listing. M varies: 1, 8, 64, 512, 4096. The per-entity latency should stay flat as M grows (Definition 2 — disjoint cells don't contend). If it degrades, the per-cell isolation is leaking.
3. Write collide — fire M concurrent PATCH requests all to the SAME Listing. Latency should grow linearly — that's the single-cell serialisation the paper requires. Measure the serialisation overhead per concurrent writer.
4. List walk — `GET /api/listings?sort=created_at&order=desc&limit=20` with cold cache; record p50/p95/p99. Then warm cache. The fanned-out read-path should saturate around ~10K cells; beyond that the analytics binding (#219) should win.
5. SSE scale — open K cellKey-scoped subscribers (#220), drive background writes, measure event-delivery p50/p99 and bytes/sec per subscriber as K grows to 10K.

## Success criteria

The curve validates the architecture when:

- Write-latency p99 at (20M cells, 4096 concurrent disjoint-cell writers) is within 2× of the 1-writer baseline. (True isolation.)
- Single-cell serialisation overhead is ≤ 1ms per queued writer. (DO runtime is not the bottleneck.)
- Fanned-out list latency at 20M cells is under 500ms p95 — OR the analytics binding beats it by ≥ 10×. (Whichever deployment mode the team chooses, one of the two works.)
- SSE at 10K subscribers with cellKey demux delivers events under 100ms p95.

Any of these failing points to real architectural work, not just tuning.

## Cost model

Cloudflare Workers billing has three axes at this scale:

- **Durable Object invocations**: per-cell DO = one invocation per write. 20M writes → 20M invocations. At the current plan rate, that's the dominant line item.
- **DO storage**: 3NF row per cell. An 8-column Listing is ~200 bytes JSON + DO overhead. 20M × 200 B ≈ 4 GB.
- **Workers duration**: each request's CPU time on the worker. Mostly negligible — the DO does the work.

The cost curve plots all three axes against N so a future deployment has data to project from.

## What this doc does NOT do

This is methodology, not measurement. Actual runs require:

- A dev account with sufficient DO allowance for 20M rows.
- A fleet of client machines (or Workers-as-clients) to drive the load.
- A dashboard that records the timers the test program emits.

When those are in place, the runbook here applies unchanged. Partial runs (1K / 100K / 1M) give a representative slope and should match the 20M extrapolation within a small factor.

## Pointers

- `src/broadcast-do.test.ts` — the demux correctness pins (#220). Load tests on top extend these.
- `docs/12-physical-mapping.md` — the architecture the cost curve validates.
- `docs/08-federation.md` §Federated analytics — the read-path fork-point.
