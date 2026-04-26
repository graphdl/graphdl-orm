# AREST on Cloudflare — Getting Started

AREST runs as a Cloudflare Worker. The engine is a WASM module; per-tenant
state lives in three Durable Object classes:

| DO class    | Granularity          | What it holds                              |
|-------------|----------------------|--------------------------------------------|
| `EntityDB`  | one per entity id    | a single cell — Definition 2 isolation     |
| `RegistryDB`| one per scope        | population index, schema cache, secrets    |
| `BroadcastDO` | one per scope      | SSE subscriber registry                    |

The Worker exposes the full v1.0 verb set as REST endpoints, an OpenAPI 3.1
manifest, an SSE event stream, and an MCP endpoint at `/mcp` (also `/sse`).

## Prerequisites

- Node 18+ and `yarn` (or npm)
- A Cloudflare account (free tier is fine)
- Rust nightly + `wasm-pack` for building the engine WASM

```bash
# Rust nightly (only needed once)
curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain nightly -y
cargo install wasm-pack
```

## 5-minute deploy

```bash
git clone https://github.com/graphdl/arest
cd arest
yarn install
yarn build:wasm        # compiles crates/arest → pkg/ as a WASM bundle
yarn dev               # local Worker on http://127.0.0.1:8787
```

Deploy to Cloudflare:

```bash
wrangler login         # one-time browser auth
yarn deploy            # build:wasm + wrangler deploy
```

The first `wrangler deploy` provisions all three DO classes + applies the v1
and v2 migrations declared in `wrangler.jsonc`.

## Adding readings

Drop FORML 2 markdown files under `readings/` (or any scoped subfolder you
configure) and re-deploy:

```bash
cp my-app.md readings/
yarn deploy
```

A live tenant can also load readings at runtime via `POST /arest/:scope/load_reading`
without redeploying — see `readings/core/evolution.md` for the governance gate.

## Using a deployed worker

Replace `https://arest.example.workers.dev` with your worker URL. All
endpoints are listed in `/api/openapi.json`.

```bash
# Schema introspection
curl https://arest.example.workers.dev/api/openapi.json | jq

# Create an entity (compile-derived endpoint, generated from your readings)
curl -X POST https://arest.example.workers.dev/arest/default/Order \
  -H "Content-Type: application/json" \
  -d '{"id": "ord-1", "customer": "acme"}'

# State-machine transition
curl -X POST https://arest.example.workers.dev/arest/default/Order/ord-1/transition \
  -H "Content-Type: application/json" \
  -d '{"transition": "place"}'

# Read with HATEOAS links
curl https://arest.example.workers.dev/arest/default/Order/ord-1

# List
curl https://arest.example.workers.dev/arest/default/Order

# Live event stream (SSE)
curl -N https://arest.example.workers.dev/api/events
```

## Using the checker

Constraint violations come back inline on every mutation. To check arbitrary
text or a candidate fact set without writing it:

```bash
curl -X POST https://arest.example.workers.dev/api/verify \
  -H "Content-Type: application/json" \
  -d '{"text": "Alice placed Order ord-2 on 2026-04-26."}'
```

Returns the extracted facts plus any deontic / alethic violations they would
trigger against the current population.

## MCP from a deployed worker

Point any MCP-aware agent (Claude Desktop, Claude Code) at the worker:

```json
{
  "mcpServers": {
    "arest": {
      "url": "https://arest.example.workers.dev/mcp"
    }
  }
}
```

Or with bearer auth:

```bash
export AREST_API_KEY=...
curl -H "Authorization: Bearer $AREST_API_KEY" \
     https://arest.example.workers.dev/mcp/...
```

## Where next

- `docs/02-writing-readings.md` — FORML 2 syntax
- `docs/09-mcp-verbs.md` — verb set the worker exposes
- `docs/12-physical-mapping.md` — why per-cell DO sharding (Theorem 5)
- [`docs/cli.md`](cli.md) — same engine, no Cloudflare
- [`docs/mcp.md`](mcp.md) — agent-facing surface
- [`crates/arest-kernel/README.md`](../crates/arest-kernel/README.md) — same engine as a UEFI kernel
