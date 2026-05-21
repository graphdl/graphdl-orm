# ui.do — local dev recipe (support.auto.dev, no Cloudflare deploy)

Runs ui.do against a **local** Wrangler worker, rendering the
`support.auto.dev` app from the loaded MODEL (schema only — no seed /
business data). Auth (auth.vin / auto.dev in prod) is **bypassed**
locally: the worker has no upstream `apis.` proxy, so `GET /arest/`
answering 200 is sufficient for the SPA's cookie-less auth probe.

Ports (chosen to avoid clashing with other local servers):
- worker (Wrangler dev): **8788**
- ui.do (Vite): **5174**

## 0. One-time build / install

```pwsh
# from the arest repo root
yarn install
yarn build:wasm        # compiles crates/arest -> crates/arest/pkg (~4-5 min)

# ui.do deps
cd apps/ui.do
pnpm install
```

## 1. Local worker secrets (`.dev.vars`, repo root, gitignored)

EntityDB cell storage refuses to run without a tenant master seed
(see `src/entity-do.ts::getMaster`). For local dev, create
`<repo-root>/.dev.vars`:

```
TENANT_MASTER_SEED=local-dev-seed-not-a-real-secret-0123456789abcdef
ENVIRONMENT=development
AREST_ALLOW_PLAINTEXT=1
```

`.dev.vars` is loaded automatically by `wrangler dev`. It is gitignored;
never commit a real seed.

## 2. Start the worker

```pwsh
# from the arest repo root
npx wrangler dev --port 8788 --local
```

Wait for `Ready on http://127.0.0.1:8788`.

## 3. Load the support MODEL (schema/readings only)

The local HATEOAS surface (`GET /arest/…`) resolves a domain through
`loadDomainSchema`, which reads the `defs:{domain}` cell written by
`POST /api/parse`. Load the dependency tiers then support:

```pwsh
# from apps/ui.do (a copy also lives in apps/support.auto.dev/)
node ./load-local.mjs
# smallest set first:
#   node ./load-local.mjs --support-only
```

Tier order (NORMA nouns are global; later tiers reference earlier names):
law-core -> us-law -> auto.dev -> support. Tier 1 (arest metamodel) is
bundled in the worker WASM and is NOT loaded.

Confirm a GET no longer says "No schema loaded":

```pwsh
curl "http://127.0.0.1:8788/arest/?ir_domain=support"
```

> KNOWN BLOCKER (2026-05): any EntityDB cell write (`POST /api/parse`,
> `POST /api/load_reading`) HANGS indefinitely under local `wrangler
> dev` — a single 1-entity write never returns (HTTP 000 after 60-120s)
> and blocks the single-threaded worker. Loading the model requires
> writing the `defs:{domain}` cell, so until this is fixed `/arest/*`
> stays at "No schema loaded".
>
> Root cause (isolated end-to-end):
>   * Pure in-process WASM is healthy. Measured directly in Node against
>     the built pkg: `create()` ~1.95s, `compile` ~131ms, `debug` ~6ms.
>   * `EntityDB.put` -> `persistEngineState()` calls `freezeHandle()`,
>     which serialises the WHOLE engine state to a hex string. With the
>     bundled metamodel that string is ~9.2 MB, and it is written with a
>     single un-chunked `ctx.storage.put(ENGINE_STATE_STORAGE_KEY, hex)`
>     PER cell write. Cloudflare DO storage values are capped at 128 KiB;
>     a 9.2 MB value blows past that and wedges miniflare's local SQLite.
>   * A secondary hang lives in `getMaster()` (HKDF via crypto.subtle):
>     with `TENANT_MASTER_SEED` bound, the first write hangs in key
>     derivation. The plaintext path (this recipe — no seed, just
>     `AREST_ALLOW_PLAINTEXT=1`) avoids that, but the 9.2 MB freeze blob
>     still wedges the write.
>
> Both live in the engine / worker cell-persistence path
> (`crates/arest` freeze size + `src/entity-do.ts` un-chunked put), out
> of scope for ui.do. Fix options for whoever owns that path: chunk the
> freeze blob across DO storage keys (or store it in R2/KV), and/or make
> `persistEngineState` skip persisting the bundled metamodel.

## 4. Auth stub

No code stub is required: `arestAuthProvider` only does `GET /arest/`
and treats any 200 as "signed in" (auth is enforced by the prod `apis.`
proxy, which is absent locally). Once step 3 makes `/arest/` return 200,
the SPA is "authenticated".

## 5. ui.do env (`apps/ui.do/.env.local`, gitignored)

Copy `.env.local.example` to `.env.local`:

```
VITE_AREST_BASE_URL=/arest
VITE_AREST_HOST=support.auto.dev
```

- `VITE_AREST_BASE_URL=/arest` makes the providers call **same-origin**
  paths. `vite.config.ts` proxies `/arest` and `/api` to the worker on
  8788, so the browser never makes a cross-origin (CORS) request. The
  worker itself sets no CORS headers (the prod `apis.` proxy does), so
  the proxy is what makes local cross-port work.
- `VITE_AREST_HOST=support.auto.dev` makes `useBranding` resolve the
  support branding (app slug `support.do`, support-domain noun-scope
  filter) even though the browser is on `localhost`.

## 6. Start ui.do + screenshot

```pwsh
cd apps/ui.do
pnpm dev            # serves on http://localhost:5174 (port set in vite.config.ts)
```

Screenshot with the agent-browser CLI:

```pwsh
agent-browser open http://127.0.0.1:5174
agent-browser wait 4000
agent-browser screenshot ./ui-do-local.png
agent-browser get text body
```

> NOTE: in the worktree where this was developed, agent-browser's
> bundled Chromium had no usable network (it could not reach
> `127.0.0.1:5174` NOR `https://example.com` — both timed out / refused),
> so an automated screenshot could not be captured here. `vite.config.ts`
> sets `host: true` precisely so a working headless browser can reach the
> dev server over IPv4 (the Windows default `localhost` bind is IPv6-only
> and yields ECONNREFUSED). The SPA itself is served correctly (verified
> via curl of `/` and `/src/main.tsx`).

Even before any model loads, the shell renders non-blank: header
(app name + Domain switcher) and a "Quick" nav group with a Files
link. Once the support model resolves, the sidebar gains the
support-domain resources derived from `GET /api/openapi.json?app=...`.

### OpenAPI sidebar note

`useArestResources` builds the sidebar from
`GET /api/openapi.json?app=<app>`, which 404s unless the app's readings
declare `App '<app>' uses Generator 'openapi'.` (no support reading
declares it today). Without it the sidebar shows only the static Quick
group — still non-blank. To populate model-derived resources, add that
assertion to the support app readings and re-run step 3.
