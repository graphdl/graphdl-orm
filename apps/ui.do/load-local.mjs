/**
 * load-local.mjs — load the support.auto.dev MODEL (schema/readings only,
 * no business/seed data) into a LOCAL arest worker via POST /api/parse.
 *
 * Why /api/parse and not /api/load_reading?
 *   The local HATEOAS surface (GET /arest/…) resolves a domain through
 *   `loadDomainSchema`, which reads the `defs:{domain}` cell's
 *   `data.readings`. ONLY `POST /api/parse` writes that cell (parse.ts).
 *   `/api/load_reading` writes a separate `_loaded_reading:*` manifest
 *   cell that the /arest read path does not consult. So parse is the
 *   route that makes a domain resolve for ui.do.
 *
 * Dependency tiers (NORMA nouns are global; later tiers resolve names
 * defined earlier). Tier 1 (arest metamodel) is bundled in the worker
 * WASM, so we do not load it. We load law-core → us-law → auto.dev →
 * support so support's supertypes (Agent Chat, Customer, API, …) resolve.
 *
 * Usage:
 *   node load-local.mjs                 # default worker http://127.0.0.1:8788
 *   AREST_WORKER=http://127.0.0.1:8788 node load-local.mjs
 *   node load-local.mjs --support-only  # smallest set (support readings only)
 *
 * NOTE: each parse materializes entity cells (one encrypted DO each),
 * which is slow under local wrangler. Files are loaded sequentially with
 * a long per-request timeout. Be patient on the first run.
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { join, basename } from 'node:path'

const WORKER = process.env.AREST_WORKER ?? 'http://127.0.0.1:8788'
const REPOS = 'C:/Users/lippe/Repos'
const APPS = join(REPOS, 'apps')
const SUPPORT = join(APPS, 'support.auto.dev')
const SUPPORT_ONLY = process.argv.includes('--support-only')
const REQ_TIMEOUT_MS = Number(process.env.AREST_PARSE_TIMEOUT_MS ?? 300000)

function listMd(dir, { recursive = false } = {}) {
  if (!existsSync(dir)) return []
  const out = []
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, ent.name)
    if (ent.isDirectory()) {
      if (recursive) out.push(...listMd(p, { recursive }))
    } else if (ent.name.endsWith('.md')) {
      out.push(p)
    }
  }
  return out.sort()
}

async function parseOne(slug, text) {
  const ctrl = new AbortController()
  const timer = setTimeout(() => ctrl.abort(), REQ_TIMEOUT_MS)
  const t0 = Date.now()
  try {
    const res = await fetch(`${WORKER}/api/parse`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ domain: slug, text }),
      signal: ctrl.signal,
    })
    const body = await res.json().catch(() => ({}))
    const r = body?.domains?.[0]
    const secs = ((Date.now() - t0) / 1000).toFixed(1)
    if (!res.ok || !r) {
      console.log(`  x ${slug.padEnd(30)} HTTP ${res.status} ${JSON.stringify(body).slice(0, 160)} (${secs}s)`)
      return false
    }
    const errs = r.errors?.length ? ` errors=${r.errors.length}` : ''
    console.log(`  o ${slug.padEnd(30)} entities=${r.entities} nouns=${r.nouns} readings=${r.readings}${errs} (${secs}s)`)
    return true
  } catch (e) {
    const secs = ((Date.now() - t0) / 1000).toFixed(1)
    console.log(`  x ${slug.padEnd(30)} ${e?.name === 'AbortError' ? 'TIMEOUT' : String(e)} (${secs}s)`)
    return false
  } finally {
    clearTimeout(timer)
  }
}

function tier(label, files) {
  return { label, files }
}

const tiers = SUPPORT_ONLY
  ? [tier('support', listMd(join(SUPPORT, 'readings')))]
  : [
      tier('Tier 2 — law-core', listMd(join(REPOS, 'law-core', 'readings'))),
      tier('Tier 3 — us-law', [
        ...listMd(join(REPOS, 'us-law', 'readings')),
        ...listMd(join(REPOS, 'us-law', 'readings', 'states'), { recursive: true }),
      ]),
      tier('Tier 4 — auto.dev', listMd(join(APPS, 'auto.dev'))),
      tier('Tier 5 — support.auto.dev', [
        join(SUPPORT, 'app.md'),
        ...listMd(join(SUPPORT, 'readings')),
      ].filter(existsSync)),
    ]

console.log(`Loading support MODEL into ${WORKER} (support-only=${SUPPORT_ONLY})\n`)
let ok = 0
let fail = 0
for (const t of tiers) {
  console.log(`=== ${t.label} (${t.files.length} files) ===`)
  for (const p of t.files) {
    const slug = basename(p, '.md')
    const text = readFileSync(p, 'utf-8')
    if (!text.trim()) continue
    const good = await parseOne(slug, text)
    good ? ok++ : fail++
  }
}
console.log(`\nDone. parsed=${ok} failed=${fail}`)
process.exit(fail > 0 ? 1 : 0)
