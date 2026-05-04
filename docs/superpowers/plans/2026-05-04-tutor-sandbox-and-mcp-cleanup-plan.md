# Tutor Sandbox + `.mcp.json` Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the in-tree AREST tutor work seamlessly from any active app scope by giving it a parallel, isolated engine handle backed by `tutor/domains/`, and collapse `.mcp.json` from three named entries to two now that `arest-tutor` is redundant.

**Architecture:** A single MCP server process holds two engine handles: the existing `_localHandle` (active app, mutated by bare `apply`/`compile`/`propose`) and a new `_sandboxHandle` (always loaded from `tutor/domains/`, mutated only by `tutor.*` tools). In WASM mode they are two handles into the same lazy-loaded `_localEngine` instance — `engine.system(handle, …)` already partitions D state per-handle, giving free isolation. In CLI-DB mode the sandbox uses a separate SQLite file at `$AREST_TUTOR_DB` (default `tutor/.sandbox/tutor.db`). Lesson predicates in the existing `tutor` tool grade against the sandbox.

**Tech Stack:** TypeScript, `@modelcontextprotocol/sdk`, vitest, the bundled AREST WASM engine at `crates/arest/pkg/`, and (in CLI mode) the `arest-cli` binary. PowerShell on Windows.

**Spec:** [`docs/superpowers/specs/2026-05-04-tutor-sandbox-and-mcp-cleanup-design.md`](../specs/2026-05-04-tutor-sandbox-and-mcp-cleanup-design.md)

---

## File Structure

| Path                                             | Status   | Responsibility |
|--------------------------------------------------|----------|----------------|
| `src/mcp/tutor-sandbox.ts`                       | NEW      | Owns the sandbox handle + DB lifecycle. Exports `getSandboxHandle`, `tutorSystemCall`, `resetSandbox`, `tutorDomainsDir`. |
| `src/mcp/tutor-sandbox.test.ts`                  | NEW      | Vitest suite for the sandbox module — isolation, reset, persistence. |
| `src/mcp/server.ts`                              | MODIFY   | Register the 8 new `tutor.*` tools. Refactor `evalExpectPredicate` to take a `systemCall` arg. Update `tutor` lesson handler to grade via `tutorSystemCall`. |
| `.mcp.json`                                      | MODIFY   | Drop `arest-tutor` entry. Update `arest` description. |
| `.gitignore`                                     | MODIFY   | Ignore `tutor/.sandbox/`. |
| `tutor/README.md`                                | MODIFY   | Note `arest-tutor` registration is deprecated; document the eight `tutor.*` tools and `$AREST_TUTOR_DB`. |

The new module is small and single-purpose; it does not subsume any existing responsibility from `server.ts`. The refactor of `evalExpectPredicate` is local to the existing `tutor` lesson section in `server.ts` (lines ~1497–1651).

---

## Task 1: Ignore the sandbox DB directory

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Add the ignore line**

Append to `.gitignore`:

```
# Tutor sandbox SQLite (per-developer, ephemeral)
tutor/.sandbox/
```

- [ ] **Step 2: Verify the path is ignored**

Run: `git check-ignore -v tutor/.sandbox/tutor.db`
Expected: prints a line ending in `tutor/.sandbox/` (a positive match).

- [ ] **Step 3: Commit**

```powershell
cd C:\Users\lippe\Repos\arest
git commit -m @'
chore: ignore tutor/.sandbox/ (per-dev sandbox DB lives here)

Prep for the in-tree tutor sandbox; the next commits add the module
that writes there.
'@ -- .gitignore
```

---

## Task 2: Sandbox module skeleton + WASM-mode handle

**Files:**
- Create: `src/mcp/tutor-sandbox.ts`
- Create: `src/mcp/tutor-sandbox.test.ts`

The sandbox lives in WASM mode for this task (in-process, lost on MCP restart). CLI persistence lands in Task 7. Tests are written first; they import the module's public functions and assert behaviour on the WASM path.

- [ ] **Step 1: Write the failing test**

`src/mcp/tutor-sandbox.test.ts`:

```ts
/**
 * Tutor sandbox module — unit tests.
 *
 * The sandbox is a second engine handle compiled from tutor/domains/.
 * Mutations through tutorSystemCall must not be visible to the active
 * app's handle, and vice versa.
 */
import { describe, it, expect, beforeEach } from 'vitest'
import { getSandboxHandle, tutorSystemCall, resetSandbox, tutorDomainsDir } from './tutor-sandbox.js'
import { compileDomainReadings, system } from '../api/engine.js'
import { existsSync } from 'fs'
import { resolve } from 'path'

describe('tutor sandbox — WASM mode', () => {
  beforeEach(async () => {
    await resetSandbox()
  })

  it('exposes the bundled tutor/domains directory', () => {
    const dir = tutorDomainsDir()
    expect(existsSync(dir)).toBe(true)
    expect(existsSync(resolve(dir, 'orders.md'))).toBe(true)
  })

  it('boots from tutor/domains/ on first call and exposes its noun catalog', async () => {
    const raw = await tutorSystemCall('list:Noun', '')
    const list = JSON.parse(raw)
    const names = list.map((n: any) => n.id ?? n.Name ?? n.name)
    expect(names).toContain('Order')
    expect(names).toContain('Customer')
  })

  it('isolates sandbox writes from a sibling local handle, in both directions', async () => {
    // Sandbox → local: write a Customer through the sandbox.
    await tutorSystemCall('create:Customer', '<<Name, alice-sandbox>>')

    // Build an unrelated active-app handle compiled from a tiny fixture
    // that also declares Customer.
    const localHandle = compileDomainReadings(
      'Customer(.Name) is an entity type.\nCustomer has Name.\n  Each Customer has exactly one Name.'
    )
    const localList = JSON.parse(system(localHandle, 'list:Customer', ''))
    expect(localList.find((c: any) => (c.id ?? c.Name) === 'alice-sandbox')).toBeUndefined()

    // Local → sandbox: write a Customer to the local handle and confirm
    // the sandbox cannot see it.
    system(localHandle, 'create:Customer', '<<Name, bob-local>>')
    const sandList = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(sandList.find((c: any) => (c.id ?? c.Name) === 'bob-local')).toBeUndefined()
    // Sandbox still sees its own write.
    expect(sandList.find((c: any) => (c.id ?? c.Name) === 'alice-sandbox')).toBeDefined()
  })

  it('returns the same numeric handle across calls until reset', async () => {
    const a = await getSandboxHandle()
    const b = await getSandboxHandle()
    expect(a).toBe(b)
    expect(a).toBeGreaterThanOrEqual(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts`
Expected: FAIL — module `./tutor-sandbox.js` cannot be resolved.

- [ ] **Step 3: Create the sandbox module (WASM path only)**

`src/mcp/tutor-sandbox.ts`:

```ts
/**
 * tutor-sandbox.ts — second engine handle bound to tutor/domains/.
 *
 * The MCP server keeps two parallel D states inside one process:
 *   • the active-app handle (managed by server.ts), and
 *   • the sandbox handle (managed here), always bootstrapped from
 *     tutor/domains/.
 *
 * Lesson predicates and tutor.* tools route to tutorSystemCall so a
 * learner can take lessons end-to-end without disturbing the active app.
 */
/// <reference types="node" />
import { readFileSync, readdirSync, existsSync, rmSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { fileURLToPath } from 'url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

let _sandboxHandle = -1
let _engine: typeof import('../api/engine.js') | null = null

export function tutorDomainsDir(): string {
  return resolve(__dirname, '..', '..', 'tutor', 'domains')
}

export function tutorSandboxDbPath(): string {
  return process.env.AREST_TUTOR_DB
    ?? resolve(__dirname, '..', '..', 'tutor', '.sandbox', 'tutor.db')
}

async function getEngine() {
  if (_engine) return _engine
  _engine = await import('../api/engine.js')
  return _engine
}

function loadTutorDomainReadings(): string[] {
  const dir = tutorDomainsDir()
  if (!existsSync(dir)) return []
  return readdirSync(dir)
    .filter(f => f.endsWith('.md'))
    .sort()
    .map(f => readFileSync(join(dir, f), 'utf-8'))
}

export async function getSandboxHandle(): Promise<number> {
  if (_sandboxHandle >= 0) return _sandboxHandle
  const engine = await getEngine()
  const readings = loadTutorDomainReadings()
  _sandboxHandle = engine.compileDomainReadings(...readings)
  return _sandboxHandle
}

export async function tutorSystemCall(key: string, input: string): Promise<string> {
  const engine = await getEngine()
  const handle = await getSandboxHandle()
  return engine.system(handle, key, input)
}

export async function resetSandbox(): Promise<void> {
  if (_sandboxHandle >= 0 && _engine) {
    try { _engine.release_domain?.(_sandboxHandle) } catch {}
  }
  _sandboxHandle = -1
  const dbPath = tutorSandboxDbPath()
  try { if (existsSync(dbPath)) rmSync(dbPath) } catch {}
}
```

- [ ] **Step 4: Run tests, expect pass**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts`
Expected: PASS — all four tests green.

- [ ] **Step 5: Commit**

```powershell
git commit -m @'
mcp: tutor-sandbox module — WASM-mode handle isolation

Adds a second engine handle compiled from tutor/domains/, so tutor
predicates and (later) tutor.* tools can read/mutate D without
touching the active app. State partitioning is free — engine.system
already keys on handle. CLI-mode persistence lands in a follow-up.
'@ -- src/mcp/tutor-sandbox.ts src/mcp/tutor-sandbox.test.ts
```

---

## Task 3: Lesson predicates grade against the sandbox

The existing `evalExpectPredicate` in `server.ts` (line ~1519) closes over the active-engine `systemCall`. Lift `systemCall` to a parameter so the lesson handler can pass `tutorSystemCall`. After this task, `tutor({ track:'easy', num:1 })` returns `check.ok === true` even when the active scope is empty.

**Files:**
- Modify: `src/mcp/server.ts:1519` (`evalExpectPredicate` signature) and `src/mcp/server.ts:1607` (lesson handler call site)
- Modify: `src/mcp/tutor-sandbox.test.ts` (add a regression test that goes through the predicate evaluator)

- [ ] **Step 1: Write the failing regression test**

Append to `src/mcp/tutor-sandbox.test.ts`:

```ts
import { evalExpectPredicate } from './server.js'

describe('tutor lesson predicate grading', () => {
  it('grades list-contains predicate against the sandbox', async () => {
    await resetSandbox()
    const result = await evalExpectPredicate(
      'list Noun contains {"id":"Order"}',
      tutorSystemCall,
    )
    expect(result.ok).toBe(true)
  })
})
```

- [ ] **Step 2: Run test, expect fail**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts -t 'grades list-contains'`
Expected: FAIL — `evalExpectPredicate` is not exported, or its signature does not accept a second argument.

- [ ] **Step 3: Refactor `evalExpectPredicate` to accept `systemCall`**

In `src/mcp/server.ts`, change the function signature and every internal `systemCall(...)` call to use the parameter. Replace the existing function (around line 1519) with:

```ts
export async function evalExpectPredicate(
  predicate: string,
  call: (key: string, input: string) => Promise<string> = systemCall,
): Promise<{ ok: boolean; detail: string }> {
  const p = predicate.replace(/\\\s/g, ' ').trim()
  if (!p) return { ok: false, detail: 'empty predicate' }
  const parseJson = (s: string): any => JSON.parse(s.trim())
  const safeJson = <T>(raw: string, fallback: T): T | any => {
    try { const v = JSON.parse(raw); return v ?? fallback } catch { return fallback }
  }

  // list NOUN contains <json>
  let m = p.match(/^list\s+([^\s{][^{]*?)\s+contains\s+(\{[\s\S]*\})$/)
  if (m) {
    const [, noun, jsonStr] = m
    const raw = await call(`list:${noun.trim()}`, '')
    const list = safeJson(raw, [])
    if (!Array.isArray(list)) return { ok: false, detail: `list:${noun.trim()} -> not an array` }
    const expected = parseJson(jsonStr)
    const ok = list.some((item: any) => matchesSubset(item, expected))
    return { ok, detail: ok ? 'found' : `no match in ${list.length} entries` }
  }

  // list NOUN count OP N
  m = p.match(/^list\s+(\S+(?:\s\S+)*?)\s+count\s+(==|>=|<=|>|<)\s+(\d+)$/)
  if (m) {
    const [, noun, op, nStr] = m
    const raw = await call(`list:${noun.trim()}`, '')
    const list = safeJson(raw, [])
    const len = Array.isArray(list) ? list.length : 0
    const ok = cmpNum(len, op, parseInt(nStr, 10))
    return { ok, detail: `count=${len} ${op} ${nStr}` }
  }

  // query FT contains <json>
  m = p.match(/^query\s+(\S+)\s+contains\s+(\{[\s\S]*\})$/)
  if (m) {
    const [, ft, jsonStr] = m
    const raw = await call(`query:${ft}`, '')
    const rows = safeJson(raw, [])
    const expected = parseJson(jsonStr)
    const ok = Array.isArray(rows) && rows.some((r: any) => matchesSubset(r, expected))
    return { ok, detail: ok ? 'found' : `no match in ${Array.isArray(rows) ? rows.length : 0} facts` }
  }

  // query FT count OP N
  m = p.match(/^query\s+(\S+)\s+count\s+(==|>=|<=|>|<)\s+(\d+)$/)
  if (m) {
    const [, ft, op, nStr] = m
    const raw = await call(`query:${ft}`, '')
    const rows = safeJson(raw, [])
    const len = Array.isArray(rows) ? rows.length : 0
    const ok = cmpNum(len, op, parseInt(nStr, 10))
    return { ok, detail: `count=${len} ${op} ${nStr}` }
  }

  // get NOUN ID equals <json>
  m = p.match(/^get\s+(\S+(?:\s\S+)*?)\s+(\S+)\s+equals\s+(\{[\s\S]*\})$/)
  if (m) {
    const [, noun, id, jsonStr] = m
    const raw = await call(`get:${noun.trim()}`, id)
    const entity = safeJson(raw, null)
    const expected = parseJson(jsonStr)
    const ok = entity !== null && matchesSubset(entity, expected)
    return { ok, detail: ok ? 'matches' : `got ${JSON.stringify(entity)}` }
  }

  // status NOUN ID is STATUS
  m = p.match(/^status\s+(\S+(?:\s\S+)*?)\s+(\S+)\s+is\s+(\S+)$/)
  if (m) {
    const [, , id, expectedStatus] = m
    const raw = await call(`get:State Machine`, id)
    const sm: any = safeJson(raw, null)
    const actual = sm?.currentlyInStatus ?? null
    const ok = actual === expectedStatus
    return { ok, detail: ok ? `status=${actual}` : `expected ${expectedStatus}, got ${actual ?? '(none)'}` }
  }

  return { ok: false, detail: `unrecognized predicate: ${predicate}` }
}
```

- [ ] **Step 4: Wire the lesson handler to grade via the sandbox**

In the same file, locate the `tutor` tool registration (around line 1597). Inside the handler, change the predicate-evaluation call from `evalExpectPredicate(parsed.expect)` to:

```ts
const check = parsed.expect
  ? await evalExpectPredicate(parsed.expect, tutorSystemCall)
  : { ok: null as any, detail: 'no expect predicate in this lesson' }
```

Add the import at the top of the imports block in `src/mcp/server.ts`:

```ts
import { tutorSystemCall, getSandboxHandle, resetSandbox } from './tutor-sandbox.js'
```

(`getSandboxHandle` and `resetSandbox` are imported in advance; later tasks use them.)

- [ ] **Step 5: Run tests, expect pass**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts`
Expected: PASS — all sandbox tests including the new predicate-grading regression.

Run: `yarn vitest run src/mcp/server.test.ts`
Expected: PASS — pre-existing schema tests remain green.

- [ ] **Step 6: Commit**

```powershell
git commit -m @'
mcp: route tutor lesson predicates through the sandbox

evalExpectPredicate now takes the systemCall as a parameter (default
preserves existing call sites). The tutor lesson handler passes
tutorSystemCall, so `expect` predicates grade against tutor/domains/
regardless of which app is currently active.
'@ -- src/mcp/server.ts src/mcp/tutor-sandbox.test.ts
```

---

## Task 4: `tutor.reset` exposed as a tool

After reset, the next sandbox call recompiles from disk so any new lesson domains take effect.

**Files:**
- Modify: `src/mcp/server.ts` (register `tutor.reset` tool)
- Modify: `src/mcp/tutor-sandbox.test.ts` (add reset test)

- [ ] **Step 1: Write the failing test**

Append to `src/mcp/tutor-sandbox.test.ts`:

```ts
describe('tutor.reset', () => {
  it('drops learner-created entities and re-bootstraps from tutor/domains', async () => {
    await resetSandbox()
    await tutorSystemCall('create:Customer', '<<Name, transient>>')
    const before = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(before.find((c: any) => (c.id ?? c.Name) === 'transient')).toBeDefined()

    await resetSandbox()
    const after = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(after.find((c: any) => (c.id ?? c.Name) === 'transient')).toBeUndefined()

    // Catalog still present (domain readings re-loaded).
    const nouns = JSON.parse(await tutorSystemCall('list:Noun', ''))
    expect(nouns.map((n: any) => n.id ?? n.Name).includes('Order')).toBe(true)
  })
})
```

- [ ] **Step 2: Run test, expect pass**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts -t 'tutor.reset'`
Expected: PASS — `resetSandbox` was implemented in Task 2 and already does this. The test simply pins the contract.

- [ ] **Step 3: Register `tutor.reset` as an MCP tool**

In `src/mcp/server.ts`, immediately after the existing `tutor` tool registration block, add:

```ts
server.registerTool(
  'tutor.reset',
  {
    description: 'Wipe the tutor sandbox engine and SQLite file. The next tutor.* call rebootstraps it from tutor/domains/. Use when you want to redo a track from a clean slate or when you have edited tutor/domains/ readings.',
    inputSchema: {},
  },
  async () => {
    await resetSandbox()
    return textResult({ ok: true, message: 'Tutor sandbox reset.' })
  },
)
```

- [ ] **Step 4: Run all MCP tests, expect pass**

Run: `yarn vitest run src/mcp/`
Expected: PASS for every test file in the directory.

- [ ] **Step 5: Commit**

```powershell
git commit -m @'
mcp: tutor.reset tool — wipe the sandbox

Exposes resetSandbox as an MCP tool so a learner can blow away their
tutor progress without restarting the server. Also the right escape
hatch when tutor/domains/ readings are edited (the next call
recompiles).
'@ -- src/mcp/server.ts src/mcp/tutor-sandbox.test.ts
```

---

## Task 5: Register the seven sandbox-routed mirror tools

`tutor.propose`, `tutor.apply`, `tutor.compile`, `tutor.query`, `tutor.list`, `tutor.get`, `tutor.actions`. Each one mirrors the schema of its bare counterpart but routes through `tutorSystemCall`. Use the simplest possible passthrough so the implementation is uniform and review-friendly.

**Files:**
- Modify: `src/mcp/server.ts` (seven new `server.registerTool` calls)
- Modify: `src/mcp/tutor-sandbox.test.ts` (smoke test that the mirror tools route to the sandbox)

- [ ] **Step 1: Write the failing test**

Append to `src/mcp/tutor-sandbox.test.ts`:

```ts
import { listRegisteredTools } from './server.js'

describe('tutor.* mirror tools', () => {
  it('registers all eight tutor.* tools', () => {
    const names = listRegisteredTools()
    expect(names).toEqual(expect.arrayContaining([
      'tutor', 'tutor.reset',
      'tutor.propose', 'tutor.apply', 'tutor.compile',
      'tutor.query', 'tutor.list', 'tutor.get', 'tutor.actions',
    ]))
  })
})
```

- [ ] **Step 2: Run test, expect fail**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts -t 'tutor.* mirror'`
Expected: FAIL — `listRegisteredTools` is not exported, OR the new tool names are missing.

- [ ] **Step 3: Export a tool inventory helper from `server.ts`**

Near the top of `src/mcp/server.ts` after the `const server = new McpServer({...})` line, add a module-level set populated as tools register, plus an export:

```ts
const _registeredTools = new Set<string>()
const _registerTool = server.registerTool.bind(server)
server.registerTool = ((name: string, ...rest: any[]) => {
  _registeredTools.add(name)
  return _registerTool(name, ...rest)
}) as typeof server.registerTool
export function listRegisteredTools(): string[] {
  return [..._registeredTools].sort()
}
```

This wraps `registerTool` so every subsequent call (including the existing ones below) is recorded. Place this block before any `server.registerTool(...)` call in the file.

- [ ] **Step 4: Register the seven mirror tools**

In `src/mcp/server.ts`, immediately after the `tutor.reset` registration, add:

```ts
// ── tutor.* mirror tools — sandbox-routed ──────────────────────────

server.registerTool(
  'tutor.list',
  {
    description: 'list:NOUN against the tutor sandbox (tutor/domains/). Use this instead of `list` when working through lessons.',
    inputSchema: { noun: z.string().describe('Entity noun, e.g. "Order".') },
  },
  async ({ noun }) => textResult(JSON.parse(await tutorSystemCall(`list:${noun}`, ''))),
)

server.registerTool(
  'tutor.get',
  {
    description: 'get:NOUN/ID against the tutor sandbox.',
    inputSchema: { noun: z.string(), id: z.string() },
  },
  async ({ noun, id }) => textResult(JSON.parse(await tutorSystemCall(`get:${noun}`, id))),
)

server.registerTool(
  'tutor.query',
  {
    description: 'query:FACT_TYPE against the tutor sandbox. Filters are passed as a JSON object.',
    inputSchema: {
      fact_type: z.string(),
      filter: z.record(z.string(), z.string()).optional(),
    },
  },
  async ({ fact_type, filter }) =>
    textResult(JSON.parse(await tutorSystemCall(`query:${fact_type}`, JSON.stringify(filter ?? {})))),
)

server.registerTool(
  'tutor.actions',
  {
    description: 'List the legal SM transitions for a noun in the tutor sandbox.',
    inputSchema: { noun: z.string(), id: z.string().optional() },
  },
  async ({ noun, id }) => {
    const raw = await tutorSystemCall(`transitions:${noun}`, id ?? '')
    return textResult({ raw, parsed: parseTransitionTriples(raw, noun, id ?? '') })
  },
)

server.registerTool(
  'tutor.apply',
  {
    description: 'Apply create/update/transition against the tutor sandbox. Same shape as `apply`.',
    inputSchema: {
      operation: z.enum(['create', 'update', 'transition']),
      noun: z.string(),
      id: z.string().optional(),
      event: z.string().optional(),
      fields: z.record(z.string(), z.string()).optional(),
    },
  },
  async ({ operation, noun, id, event, fields }) => {
    const pairs = Object.entries(fields ?? {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
    const idPair = id ? `<id, ${id}>${pairs ? ', ' : ''}` : ''
    if (operation === 'create') {
      const raw = await tutorSystemCall(`create:${noun}`, `<${idPair}${pairs}>`)
      return textResult(JSON.parse(raw))
    }
    if (operation === 'update') {
      const raw = await tutorSystemCall(`update:${noun}`, `<<id, ${id || ''}>${pairs ? `, ${pairs}` : ''}>`)
      return textResult(JSON.parse(raw))
    }
    const raw = await tutorSystemCall(`transition:${noun}`, `<${id || ''}, ${event || ''}>`)
    return textResult(JSON.parse(raw))
  },
)

server.registerTool(
  'tutor.compile',
  {
    description: 'Compile FORML2 readings into the tutor sandbox (Corollary 5 — self-modification, lesson-scoped).',
    inputSchema: { readings: z.string().describe('FORML2 readings markdown.') },
  },
  async ({ readings }) => textResult({ raw: await tutorSystemCall('compile', readings) }),
)

server.registerTool(
  'tutor.propose',
  {
    description: 'Stage a Domain Change against the tutor sandbox. Same shape as `propose`.',
    inputSchema: {
      rationale: z.string(),
      target_domain: z.string().optional(),
      nouns: z.array(z.string()).optional(),
      readings: z.array(z.string()).optional(),
    },
  },
  async (args) => {
    const raw = await tutorSystemCall(`create:Domain Change`, JSON.stringify(args))
    return textResult(JSON.parse(raw))
  },
)
```

- [ ] **Step 5: Run all MCP tests, expect pass**

Run: `yarn vitest run src/mcp/`
Expected: PASS — sandbox tests now report all eight tools registered.

- [ ] **Step 6: Smoke-test a round trip**

Run a quick end-to-end check inside vitest. Append:

```ts
describe('tutor.apply round trip', () => {
  it('a tutor.apply create is visible to a subsequent tutor.list', async () => {
    await resetSandbox()
    const createRaw = await tutorSystemCall('create:Customer', '<<Name, alice>>')
    const created = JSON.parse(createRaw)
    expect(created).toBeDefined()
    const listed = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(listed.find((c: any) => (c.id ?? c.Name) === 'alice')).toBeDefined()
  })
})
```

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts`
Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git commit -m @'
mcp: tutor.* mirror tools — sandbox-routed apply/query/list/...

Adds tutor.propose, .apply, .compile, .query, .list, .get, .actions.
Each is a thin wrapper that builds the same payload as its bare
counterpart and dispatches through tutorSystemCall, so a learner can
take lessons end-to-end without writing to the active app.

Also wraps server.registerTool to record an inventory; the new
listRegisteredTools export is used by the test that pins the
expected tool surface.
'@ -- src/mcp/server.ts src/mcp/tutor-sandbox.test.ts
```

---

## Task 6: CLI-mode persistence

In CLI-DB mode, `tutorSystemCall` must dispatch to a separate SQLite file at `$AREST_TUTOR_DB` so lesson chains survive MCP server restarts. The active engine's `cliSystemCall` shells out to `arest-cli --db $AREST_DB key input`; the sandbox needs the same shape with a different DB path, plus a one-time bootstrap that compiles `tutor/domains/` into the file.

**Files:**
- Modify: `src/mcp/tutor-sandbox.ts` (CLI dispatch branch + bootstrap)
- Modify: `src/mcp/tutor-sandbox.test.ts` (persistence test gated on `AREST_CLI` being set)

- [ ] **Step 1: Write the failing test**

Append to `src/mcp/tutor-sandbox.test.ts`:

```ts
import { mkdtempSync, existsSync as exists2 } from 'fs'
import { tmpdir } from 'os'
import { join as join2 } from 'path'

const haveCli = Boolean(process.env.AREST_CLI && exists2(process.env.AREST_CLI))

describe.skipIf(!haveCli)('tutor sandbox — CLI persistence', () => {
  it('writes through tutor.apply survive a sandbox-handle reset when AREST_TUTOR_DB is set', async () => {
    const tempDir = mkdtempSync(join2(tmpdir(), 'arest-tutor-'))
    const dbPath = join2(tempDir, 'tutor.db')
    process.env.AREST_TUTOR_DB = dbPath
    // shouldUseCliDb in tutor-sandbox.ts triggers when AREST_CLI is set.

    await resetSandbox()
    await tutorSystemCall('create:Customer', '<<Name, persisted>>')
    expect(exists2(dbPath)).toBe(true)

    // Drop the in-process handle but keep the DB file.
    // (Equivalent to an MCP server restart.)
    const { _testOnly_dropSandboxHandle } = await import('./tutor-sandbox.js') as any
    _testOnly_dropSandboxHandle()

    const listed = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(listed.find((c: any) => (c.id ?? c.Name) === 'persisted')).toBeDefined()

    delete process.env.AREST_TUTOR_DB
  })
})
```

- [ ] **Step 2: Run test, expect fail (or skip if AREST_CLI unset)**

Run: `yarn vitest run src/mcp/tutor-sandbox.test.ts -t 'CLI persistence'`
Expected (with `AREST_CLI` set): FAIL — CLI dispatch is not implemented.
Expected (without): SKIPPED.

- [ ] **Step 3: Add CLI dispatch + bootstrap to `tutor-sandbox.ts`**

In `src/mcp/tutor-sandbox.ts`, replace `tutorSystemCall` and add the supporting helpers:

```ts
import { spawn } from 'child_process'
import { mkdirSync } from 'fs'

function shouldUseCliDb(): boolean {
  return Boolean(process.env.AREST_CLI) && Boolean(tutorSandboxDbPath())
}

function runArestCli(args: string[]): Promise<string> {
  const bin = process.env.AREST_CLI
  if (!bin) throw new Error('AREST_CLI not set')
  return new Promise((resolvePromise, reject) => {
    const child = spawn(bin, args, { env: process.env, windowsHide: true })
    let stdout = '', stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', c => { stdout += c })
    child.stderr.on('data', c => { stderr += c })
    child.on('error', reject)
    child.on('close', code => {
      if (code === 0) resolvePromise(stdout.trim())
      else reject(new Error(stderr.trim() || `arest-cli exited ${code}`))
    })
  })
}

let _cliBootstrapped = false

async function ensureCliBootstrapped(): Promise<void> {
  if (_cliBootstrapped) return
  const dbPath = tutorSandboxDbPath()
  mkdirSync(resolve(dbPath, '..'), { recursive: true })
  if (!existsSync(dbPath)) {
    await runArestCli([tutorDomainsDir(), '--db', dbPath])
  }
  _cliBootstrapped = true
}

export async function tutorSystemCall(key: string, input: string): Promise<string> {
  if (shouldUseCliDb()) {
    await ensureCliBootstrapped()
    return runArestCli(['--db', tutorSandboxDbPath(), key, input])
  }
  const engine = await getEngine()
  const handle = await getSandboxHandle()
  return engine.system(handle, key, input)
}

export function _testOnly_dropSandboxHandle(): void {
  if (_sandboxHandle >= 0 && _engine) {
    try { _engine.release_domain?.(_sandboxHandle) } catch {}
  }
  _sandboxHandle = -1
  _cliBootstrapped = false
}
```

Update `resetSandbox` to also clear the bootstrap flag:

```ts
export async function resetSandbox(): Promise<void> {
  if (_sandboxHandle >= 0 && _engine) {
    try { _engine.release_domain?.(_sandboxHandle) } catch {}
  }
  _sandboxHandle = -1
  _cliBootstrapped = false
  const dbPath = tutorSandboxDbPath()
  try { if (existsSync(dbPath)) rmSync(dbPath) } catch {}
}
```

- [ ] **Step 4: Run tests, expect pass**

Run (with `AREST_CLI` set):

```powershell
$env:AREST_CLI = 'C:\Users\lippe\Repos\arest\crates\arest\target\release\arest-cli.exe'
yarn vitest run src/mcp/tutor-sandbox.test.ts
```

Expected: PASS — CLI persistence test green; WASM-mode tests still green.

- [ ] **Step 5: Commit**

```powershell
git commit -m @'
mcp: tutor sandbox CLI-mode persistence via $AREST_TUTOR_DB

When AREST_CLI is set the sandbox shells out to arest-cli against a
separate SQLite file (default tutor/.sandbox/tutor.db, override via
AREST_TUTOR_DB). The file is bootstrapped on first call by compiling
tutor/domains/ into it. Lesson chains now survive MCP restarts.
'@ -- src/mcp/tutor-sandbox.ts src/mcp/tutor-sandbox.test.ts
```

---

## Task 7: `.mcp.json` cleanup

The `arest-tutor` named entry was a workaround for the active-scope coupling that Tasks 2–5 just removed. Drop it; freshen the `arest` description to advertise the built-in tutor.

**Files:**
- Modify: `.mcp.json`

- [ ] **Step 1: Update the file**

Replace `.mcp.json` with:

```json
{
  "$schema": "https://raw.githubusercontent.com/modelcontextprotocol/sdk/main/schemas/mcp.json",
  "mcpServers": {
    "arest": {
      "description": "AREST engine — FORML2 readings → REST API. Tools: apply / compile / propose / query / audit / verify, plus a built-in sandboxed tutor (tutor + tutor.* tools) that runs orthogonal to your active app.",
      "command": "npx",
      "args": ["-y", "tsx", "src/mcp/server.ts"],
      "env": {
        "AREST_MODE": "local",
        "AREST_READINGS_DIR": "./readings"
      }
    },
    "arest-remote": {
      "description": "AREST engine (remote worker mode). Requires AREST_URL and AREST_API_KEY set in your shell.",
      "command": "npx",
      "args": ["-y", "tsx", "src/mcp/server.ts"],
      "env": {
        "AREST_MODE": "remote",
        "AREST_URL": "${AREST_URL}",
        "AREST_API_KEY": "${AREST_API_KEY}"
      }
    }
  }
}
```

- [ ] **Step 2: Verify the JSON parses**

Run: `node -e "JSON.parse(require('fs').readFileSync('.mcp.json','utf-8'))"`
Expected: no output, exit 0.

- [ ] **Step 3: Commit**

```powershell
git commit -m @'
chore: drop arest-tutor from .mcp.json

The dedicated arest-tutor registration existed only because tutor
predicates graded against the active app. With the in-tree sandbox
(prior commits), the arest entry exposes the tutor for free. Anyone
with arest-tutor pinned in their personal config keeps working — it
still spawns a valid server, just redundant.
'@ -- .mcp.json
```

---

## Task 8: Documentation refresh

**Files:**
- Modify: `tutor/README.md`

- [ ] **Step 1: Update the README**

In `tutor/README.md`, replace the "To use the tutor from Claude Code or Claude Desktop, add this to your MCP config..." section with:

```markdown
## Using the tutor from Claude Code or Claude Desktop

The tutor is built into the main `arest` MCP server — you do **not** need
a separate `arest-tutor` registration. Any current `arest` registration
exposes the lesson tool plus a fully isolated tutor sandbox:

- `tutor` — list lessons or load one (with auto-graded `expect` predicate).
- `tutor.list` / `tutor.get` / `tutor.query` — read against the sandbox.
- `tutor.apply` / `tutor.compile` / `tutor.propose` / `tutor.actions` — mutate the sandbox.
- `tutor.reset` — wipe the sandbox and re-bootstrap from `tutor/domains/`.

The sandbox lives in its own SQLite file at `$AREST_TUTOR_DB`
(default: `tutor/.sandbox/tutor.db`). Your active app is never touched
by `tutor.*` calls.

The previous `arest-tutor` named entry in `.mcp.json` has been removed.
If your personal config still has it, the entry continues to work but is
redundant — point Claude at `arest` instead.
```

- [ ] **Step 2: Confirm the surrounding sections still make sense**

Read the file top-to-bottom; remove any other paragraph that says "you need to add `arest-tutor` to your MCP config" or that describes the tutor as a separate server.

- [ ] **Step 3: Commit**

```powershell
git commit -m @'
docs(tutor): the tutor lives inside the arest MCP now

README rewrite: drop the "arest-tutor MCP registration" section,
document the eight tutor.* tools, and call out $AREST_TUTOR_DB +
tutor/.sandbox/.
'@ -- tutor/README.md
```

---

## Final verification

- [ ] **Run the full MCP test suite**

Run: `yarn vitest run src/mcp/`
Expected: all green.

- [ ] **Run the broader test suite once**

Run: `yarn test`
Expected: all green (the changes are additive).

- [ ] **Manual smoke from Claude Code**

After restarting Claude Code so it picks up the new `arest` registration:

1. `mcp__arest__tutor({ command: 'list' })` — lists 17 lessons across easy/medium/hard.
2. `mcp__arest__tutor({ track: 'easy', num: 1 })` — returns `check.ok === true` even with the active scope set to a non-tutor app.
3. `mcp__arest__tutor.apply({ operation: 'create', noun: 'Customer', fields: { Name: 'alice' } })` — succeeds.
4. `mcp__arest__list({ noun: 'Customer' })` (active scope) — does NOT contain `alice`.
5. `mcp__arest__tutor.reset({})` — succeeds; subsequent `tutor.list { noun: 'Customer' }` returns no learner customers.

If any of these fail, the corresponding test in `src/mcp/tutor-sandbox.test.ts` should also fail; fix that test first.
