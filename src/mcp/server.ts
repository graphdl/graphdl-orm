/**
 * AREST MCP Server — stdio transport.
 *
 * Exposes the AREST engine as MCP tools so an AI agent (Claude Desktop,
 * Claude Code, etc.) can list/create/query entities, compile readings,
 * inspect audit trails, and verify identity signatures.
 *
 * Two modes (selected by env):
 *   AREST_MODE=local     — load the selected app from $AREST_APPS_DIR /
 *                            $AREST_APP, or explicit $AREST_READINGS_DIR /
 *                            $AREST_DB paths. No network. Default when
 *                            AREST_URL is unset or empty.
 *   AREST_MODE=remote    — call a deployed Cloudflare Worker at
 *                            $AREST_URL using $AREST_API_KEY.
 *
 * Usage from a plugin config (Claude Desktop / Claude Code):
 *   {
 *     "mcpServers": {
 *       "arest": {
 *         "command": "npx",
 *         "args": ["-y", "arest", "mcp"],
 *         "env": {
 *           "AREST_MODE": "local",
 *           "AREST_APPS_DIR": "/absolute/path/to/apps",
 *           "AREST_APP": "support"
 *         }
 *       }
 *     }
 *   }
 *
 * Or call directly:
 *   AREST_MODE=local AREST_APPS_DIR=../apps AREST_APP=support npx tsx src/mcp/server.ts
 */

/// <reference types="node" />
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { z } from 'zod'
import { readFileSync, readdirSync, existsSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { fileURLToPath } from 'url'
import { spawn } from 'child_process'
import {
  buildAppCompileArgs,
  checkArestApps,
  createArestApp,
  inferInitialAppName,
  inspectArestApp,
  listArestReadingFiles,
  listArestApps,
  resolveArestApp,
  type ArestApp,
  type ArestAppHealth,
  type ManagedInstanceFactTypeReading,
} from './apps.js'
import {
  buildMutationContext,
  enforceMutationContext,
  DEFAULT_MUTATION_PROMPTS,
  CONTEXT_RECEIPT_FIELD_DESCRIPTION,
  MUTATION_CONTEXT_DESCRIPTION,
  MUTATION_TOOL_DESCRIPTION,
  type MutationContextDetail,
  type MutationContextTool,
} from './mutation-context.js'
import { tutorSystemCall, resetSandbox, parseEngineRaw } from './tutor-sandbox.js'
import { resolveArestCli } from './cli-resolver.js'
import { checkCliStaleness } from './cli-staleness.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(__dirname, '..', '..')

// ── Mode selection ──────────────────────────────────────────────────

const AREST_URL = process.env.AREST_URL || ''
const AREST_API_KEY = process.env.AREST_API_KEY || ''
const AREST_APPS_DIR = process.env.AREST_APPS_DIR || ''
const AREST_READINGS_DIR = process.env.AREST_READINGS_DIR || ''
const AREST_DB = process.env.AREST_DB || ''
// #841: prefer whichever of target/debug or target/release was built
// most recently. Existing AREST_CLI env var still wins when set
// explicitly, so workspace overrides aren't disturbed.
const AREST_CLI = process.env.AREST_CLI || resolveArestCli(REPO_ROOT)
const AREST_MODE = (process.env.AREST_MODE || (AREST_URL ? 'remote' : 'local')).toLowerCase()
const AREST_DEBUG = process.env.AREST_DEBUG === '1'
const INITIAL_APP_NAME = inferInitialAppName(process.env)
const APP_MODE_ENABLED = Boolean(AREST_DB || process.env.AREST_APP || AREST_APPS_DIR)

function appRegistryOptions() {
  return {
    appsDir: AREST_APPS_DIR || undefined,
    cwd: REPO_ROOT,
    explicitAppName: INITIAL_APP_NAME,
    explicitReadingsDir: AREST_READINGS_DIR || undefined,
    explicitDbPath: AREST_DB || undefined,
  }
}

let activeApp = resolveArestApp(INITIAL_APP_NAME, appRegistryOptions())

// ── Local mode: bundled WASM engine via engine.ts ───────────────────
// Lazily imported so remote-mode users don't pay the WASM cost.

let _localHandle: number = -1
let _localEngine: typeof import('../api/engine.js') | null = null
let _localReadingsSignature = ''

function resetLocalHandle() {
  _localHandle = -1
  _localReadingsSignature = ''
}

function activateApp(name: string): ArestApp {
  activeApp = resolveArestApp(name, appRegistryOptions())
  resetLocalHandle()
  return activeApp
}

function currentReadingsDir(): string {
  return activeApp.readingsDir || AREST_READINGS_DIR
}

function currentDbPath(): string {
  return activeApp.dbPath || AREST_DB
}

function shouldUseCliDb(): boolean {
  return AREST_MODE === 'local' && APP_MODE_ENABLED && Boolean(currentDbPath())
}

type AppDetail = 'summary' | 'full'

function compactHealth(health: ArestAppHealth) {
  return {
    status: health.status,
    ok: health.ok,
    issues: health.issues,
    next_actions: health.next_actions,
    readings: {
      count: health.readings.count,
      newestModified: health.readings.newestModified,
    },
    db: {
      exists: health.db.exists,
      stale: health.db.stale,
      modified: health.db.modified,
      bytes: health.db.bytes,
    },
    dependencies: {
      direct: health.dependencies.direct.map((dependency) => dependency.name),
      closure: health.dependencies.closure.map((dependency) => dependency.name),
      newestModified: health.dependencies.newestModified,
      stale: health.dependencies.stale,
    },
  }
}

function appSummary(app: ArestApp = activeApp, detail: AppDetail = 'summary') {
  const inspected = inspectArestApp(app, appRegistryOptions())
  const active = app.name === activeApp.name
  const health = detail === 'full' ? inspected.health : compactHealth(inspected.health)
  const nextActions = [...health.next_actions]
  if (!active && health.status !== 'library' && health.status !== 'not_found') {
    nextActions.push({
      tool: 'apps.use',
      args: { name: app.name },
      reason: 'make this app the active UoD for subsequent local operations',
    })
  }
  return {
    ...app,
    active,
    mode: AREST_MODE,
    app_mode_enabled: APP_MODE_ENABLED,
    health: {
      ...health,
      next_actions: nextActions,
    },
  }
}

async function getLocalEngine() {
  if (_localEngine) return _localEngine
  _localEngine = await import('../api/engine.js')
  return _localEngine
}

async function getLocalHandle(): Promise<number> {
  const readingsDir = currentReadingsDir()
  const signature = readingsSignature(readingsDir)
  if (_localHandle >= 0 && _localReadingsSignature === signature) return _localHandle
  const engine = await getLocalEngine()
  const readings = loadReadingsFromDir(readingsDir)
  _localHandle = engine.compileDomainReadings(...readings)
  _localReadingsSignature = signature
  return _localHandle
}

function loadReadingsFromDir(dir: string): string[] {
  if (!dir || !existsSync(dir)) return []
  return listArestReadingFiles(dir).map(file => readFileSync(file.path, 'utf-8'))
}

function readingsSignature(dir: string): string {
  if (!dir || !existsSync(dir)) return ''
  return listArestReadingFiles(dir)
    .map(file => `${file.path}:${file.modifiedMs}:${file.bytes}`)
    .join('|')
}

// ── Remote mode: HTTP fetch ─────────────────────────────────────────

async function httpRequest(path: string, options?: RequestInit): Promise<any> {
  const url = `${AREST_URL}${path}`
  const headers: Record<string, string> = {
    'Accept': 'application/json',
    'Content-Type': 'application/json',
  }
  if (AREST_API_KEY) {
    headers['Authorization'] = `Bearer ${AREST_API_KEY}`
  }
  const res = await fetch(url, { ...options, headers: { ...headers, ...options?.headers } })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(`${res.status} ${res.statusText}: ${text}`)
  }
  return res.json()
}

function textResult(data: any) {
  return { content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }] }
}

function parseTransitionTriples(raw: string, noun: string, id: string): Array<Record<string, string>> {
  const out: Array<Record<string, string>> = []
  const re = /<([^,<>]+),\s*([^,<>]+),\s*([^<>]+)>/g
  let match: RegExpExecArray | null
  while ((match = re.exec(raw)) !== null) {
    const [, fromStatus, targetStatus, event] = match
    out.push({
      event: event.trim(),
      targetStatus: targetStatus.trim(),
      fromStatus: fromStatus.trim(),
      method: 'POST',
      href: `/api/entities/${encodeURIComponent(noun)}/${encodeURIComponent(id)}/transition`,
    })
  }
  return out
}

function normalizeTransitionRows(raw: string, noun: string, id: string): Array<Record<string, string>> {
  const parsed = parseEngineRaw(raw, [])
  if (Array.isArray(parsed)) {
    return parsed.flatMap((item: any) => {
      if (Array.isArray(item)) {
        const [fromStatus, targetStatus, event] = item.map((v) => String(v))
        return [{
          event,
          targetStatus,
          fromStatus,
          method: 'POST',
          href: `/api/entities/${encodeURIComponent(noun)}/${encodeURIComponent(id)}/transition`,
        }]
      }
      if (item && typeof item === 'object') {
        return [{
          event: String(item.event ?? item.Event ?? ''),
          targetStatus: String(item.targetStatus ?? item.TargetStatus ?? item.to ?? ''),
          fromStatus: String(item.fromStatus ?? item.FromStatus ?? item.from ?? ''),
          method: String(item.method ?? 'POST'),
          href: String(item.href ?? `/api/entities/${encodeURIComponent(noun)}/${encodeURIComponent(id)}/transition`),
        }]
      }
      return []
    }).filter((t) => t.event || t.targetStatus || t.fromStatus)
  }
  return parseTransitionTriples(raw, noun, id)
}

// ── Command dispatch (dual mode) ────────────────────────────────────

async function dispatchCommand(command: any): Promise<any> {
  if (shouldUseCliDb()) {
    return cliApplyCommand(command)
  }
  if (AREST_MODE === 'local') {
    const engine = await getLocalEngine()
    const handle = await getLocalHandle()
    const raw = engine.system(handle, 'apply', JSON.stringify(command))
    try { return JSON.parse(raw) } catch { return { rejected: true, error: raw } }
  }
  // Remote: POST to /arest/:domain/:noun or /arest/:domain/apply
  return httpRequest(`/arest/${command.domain || 'default'}/apply`, {
    method: 'POST',
    body: JSON.stringify(command),
  })
}

async function dispatchRead(path: string): Promise<any> {
  if (AREST_MODE === 'local') {
    const raw = await systemCall('debug', '')
    try { return JSON.parse(raw) } catch { return { raw } }
  }
  return httpRequest(path)
}

// ── Local system call helper ──────────────────────────────────────

async function systemCall(key: string, input: string): Promise<string> {
  if (shouldUseCliDb()) return cliSystemCall(key, input)
  const engine = await getLocalEngine()
  const handle = await getLocalHandle()
  return engine.system(handle, key, input)
}

function runArestCli(args: string[]): Promise<string> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(AREST_CLI, args, {
      cwd: REPO_ROOT,
      env: process.env,
      windowsHide: true,
    })
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('error', reject)
    child.on('close', code => {
      if (AREST_DEBUG && stderr.trim()) console.error(stderr.trim())
      if (code === 0) {
        resolvePromise(stdout.trim())
      } else {
        reject(new Error(stderr.trim() || `arest-cli exited with code ${code}`))
      }
    })
  })
}

function cliSystemCall(key: string, input: string): Promise<string> {
  return runArestCli(['--db', currentDbPath(), key, input])
}

function compileAppReadings(app: ArestApp): Promise<string> {
  return runArestCli(buildAppCompileArgs(app, appRegistryOptions()))
}

function compileResult(raw: string) {
  let parsed: unknown
  try { parsed = JSON.parse(raw) } catch {}
  const rejected = raw.trim().startsWith('⊥')
  return {
    ok: !rejected,
    rejected,
    bytes: raw.length,
    raw,
    ...(parsed !== undefined ? { parsed } : {}),
  }
}

function parseJsonResult(raw: string): any {
  try { return JSON.parse(raw) } catch { return { raw } }
}

// #831(a) — apply no longer round-trips through mcp.md. cor:closure
// (AREST.tex Cor. 6 / commit 9630f882 in cli/entry.rs:491) makes the
// CLI compile preserve population FT cells across recompile, so the
// DB persists apply-written facts on its own. The previous
// `persistManagedApplyFacts` appended every apply to
// readings/instances/mcp.md as a durability hedge against compile
// rebuilding from φ; that's exactly the rebuild that no longer
// happens. The mcp.md file remains parseable as a normal reading
// for any facts that were written there before this change, but
// the server stops adding to it. Migration / cleanup of legacy
// content in mcp.md is a separate concern (the readings author can
// leave it, edit it, or delete it without the server caring).
async function localApplyResult(
  raw: string,
  _input: {
    operation: 'create' | 'update' | 'transition'
    noun: string
    id?: string
    fields?: Record<string, string>
  },
) {
  const result = parseJsonResult(raw)
  return textResult(result)
}

async function cliApplyCommand(command: any): Promise<any> {
  let key = ''
  let input = ''
  switch (command?.type) {
    case 'createEntity': {
      key = `create:${command.noun}`
      const pairs = Object.entries(command.fields || {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
      const idPair = command.id ? `<id, ${command.id}>${pairs ? ', ' : ''}` : ''
      input = `<${idPair}${pairs}>`
      break
    }
    case 'updateEntity': {
      key = `update:${command.noun}`
      const pairs = Object.entries(command.fields || {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
      input = `<<id, ${command.entityId}>${pairs ? `, ${pairs}` : ''}>`
      break
    }
    case 'transition': {
      key = `transition:${command.noun || ''}`
      input = `<${command.entityId || ''}, ${command.event || ''}>`
      break
    }
    default:
      return { rejected: true, error: `unsupported command type: ${command?.type || 'unknown'}` }
  }
  const raw = await cliSystemCall(key, input)
  try { return JSON.parse(raw) } catch { return { raw } }
}

// ── Data Federation: fetch from external systems via populate:{noun} ──
//
// Fetch + Citation-provenance live in ./federation. server.ts only
// resolves the populate:{noun} def from the engine (getFederationConfig)
// and delegates the actual ρ(populate_n) application to that module.

import {
  federatedFetch,
  parseFederationConfig,
  buildIngestPayload,
  enrichResponseWithCitation,
  type FederationConfig,
  type FederatedFetchResult,
} from './federation'

/**
 * Absorb a federated fetch result into P via the engine's
 * federated_ingest:<noun> FFI (#305). Returns the Citation id on
 * success, or null if the result has no citation or the ingest fails.
 * Local mode only — remote mode is already server-side.
 *
 * Error-path semantics: when the fetch returned an HTTP error,
 * `result.facts` is empty but `result.citation` still records the
 * origin (URL / retrieval date / external system). We absorb the
 * Citation alone so downstream derivations over failed-fetch
 * provenance can fire. The engine accepts empty facts arrays.
 */
async function absorbFederatedIntoD(
  noun: string,
  result: FederatedFetchResult,
): Promise<string | null> {
  if (AREST_MODE !== 'local') return null
  if (!result.citation) return null
  try {
    const payload = buildIngestPayload(result)
    const citeId = await systemCall(
      `federated_ingest:${noun}`,
      JSON.stringify(payload),
    )
    return citeId && citeId !== '⊥' ? citeId : null
  } catch {
    return null
  }
}

/** Check if a noun has a populate def and return its config. */
async function getFederationConfig(noun: string): Promise<FederationConfig | null> {
  if (AREST_MODE !== 'local') return null
  try {
    const raw = await systemCall(`populate:${noun}`, '')
    // ⊥ may surface as FFP glyphs or JSON "null" depending on encoding path.
    if (!raw || raw === 'null' || raw === '"null"' || raw.startsWith('⊥') || raw === 'φ') return null
    const config = parseFederationConfig(raw)
    // A populate def must have a non-empty url to be considered federated;
    // otherwise fall back to local population.
    if (!config || !config.url) return null
    return config
  } catch {
    return null
  }
}

const server = new McpServer({
  name: 'arest',
  version: '0.2.0',
})

const _registeredTools = new Set<string>()
const _registerTool = server.registerTool.bind(server) as typeof server.registerTool
server.registerTool = ((name: string, config: any, callback: any) => {
  _registeredTools.add(name)
  return (_registerTool as any)(name, config, callback)
}) as typeof server.registerTool
export function listRegisteredTools(): string[] {
  return [..._registeredTools].sort()
}

function loadPrompt(name: string): string {
  try {
    return readFileSync(resolve(__dirname, 'prompts', `${name}.md`), 'utf-8')
  } catch {
    return `# ${name}\n\nPrompt file not found.`
  }
}

function currentMutationContext(detail: MutationContextDetail = 'summary') {
  return buildMutationContext({
    detail,
    scope: {
      app: activeApp.name,
      db: APP_MODE_ENABLED ? currentDbPath() : undefined,
      readingsDir: currentReadingsDir(),
    },
    prompts: DEFAULT_MUTATION_PROMPTS.map((prompt) => ({
      ...prompt,
      text: loadPrompt(prompt.name),
    })),
  })
}

function mutationGateResult(
  tool: MutationContextTool,
  contextReceipt: string | undefined,
  payload: Record<string, unknown>,
) {
  const gate = enforceMutationContext({
    tool,
    receivedReceipt: contextReceipt,
    context: currentMutationContext(),
    payload,
  })
  return gate.ok ? null : textResult(gate.error)
}

// =====================================================================
// TOOLS — MCP verb set (v1.0)
// =====================================================================
//
// Primitive (algebra-required):
//   assert, retract, project, compile
//
// Entity sugar (convenience over assert/project):
//   get, query, apply, create, read, update, transition, delete
//
// Introspection (read-only):
//   explain, actions, schema, verify
//
// Evolution (governed self-modification):
//   propose   — create Domain Change, enter review workflow
//   compile   — immediate schema change (Corollary 5)
//
// LLM bridge (client sampling):
//   ask       — natural language → project → results
//   synthesize — facts → derive → verbalize → prose
//   validate  — text → extract facts → verify
//
// All framework primitives (Noun, Fact Type / Fact Type, Constraint,
// Derivation Rule, State Machine Definition, Status, Transition, Event
// Type, Instance Fact, Verb, Reading, External System, Agent Definition,
// Generator opt-in) are reachable via these verbs. Runtime functions
// (Platform/Native) are registered server-side and are intentionally not
// LLM-exposed.
// =====================================================================

// ── 0. context: prompt-backed mutation gate ──────────────────────────

server.registerTool(
  'context',
  {
    description: MUTATION_CONTEXT_DESCRIPTION,
    inputSchema: {
      detail: z.enum(['summary', 'full']).optional().describe('summary returns rules and prompt digests. full also includes prompt text.'),
    },
  },
  async ({ detail }) => textResult(currentMutationContext((detail ?? 'summary') as MutationContextDetail)),
)

// ── 0a. apps: select the active app / UoD ────────────────────────────

server.registerTool(
  'apps.current',
  {
    description: 'Show the active AREST app. The active app determines the local readings directory, SQLite DB path, and mutation context scope.',
    inputSchema: {
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details.'),
    },
  },
  async ({ detail }) => textResult({ active_app: appSummary(activeApp, (detail ?? 'summary') as AppDetail) }),
)

server.registerTool(
  'apps.list',
  {
    description: 'List AREST apps under the configured apps directory. Each app is an independent UoD with its own readings and SQLite DB.',
    inputSchema: {
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details.'),
      include_ready: z.boolean().optional().describe('Include ready apps. Default true. Set false to see only apps needing action.'),
    },
  },
  async ({ detail, include_ready }) => {
    const apps = listArestApps(appRegistryOptions())
      .map((app) => appSummary(app, (detail ?? 'summary') as AppDetail))
      .filter((app) => include_ready !== false || app.health.status !== 'ready')
    return textResult({
      active_app: activeApp.name,
      apps_dir: AREST_APPS_DIR || undefined,
      apps,
    })
  },
)

server.registerTool(
  'apps.status',
  {
    description: 'Inspect one AREST app and return health, reading/DB freshness, and next actions. Defaults to the active app.',
    inputSchema: {
      name: z.string().optional().describe('AREST app name. Defaults to the active app.'),
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details. Default full.'),
    },
  },
  async ({ name, detail }) => {
    const app = name ? resolveArestApp(name, appRegistryOptions()) : activeApp
    const isActive = app.name === activeApp.name
    return textResult({
      app: appSummary(app, (detail ?? 'full') as AppDetail),
      context: isActive ? currentMutationContext() : undefined,
    })
  },
)

server.registerTool(
  'apps.check',
  {
    description: 'Check every discovered AREST app and summarize health across the registry. Use this as the first orientation call before choosing an app.',
    inputSchema: {
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details.'),
      include_ready: z.boolean().optional().describe('Include ready apps. Default true. Set false to return only apps needing action.'),
    },
  },
  async ({ detail, include_ready }) => {
    const check = checkArestApps(appRegistryOptions())
    const apps = check.apps
      .filter((app) => include_ready !== false || app.health.status !== 'ready')
      .map((app) => appSummary(app, (detail ?? 'summary') as AppDetail))
    return textResult({
      active_app: activeApp.name,
      apps_dir: AREST_APPS_DIR || undefined,
      summary: check.summary,
      apps,
    })
  },
)

server.registerTool(
  'apps.register',
  {
    description: 'Register AREST apps by scanning the apps directory. Registration is directory-derived: no catalog facts are written by this tool.',
    inputSchema: {
      name: z.string().optional().describe('Optional app name to register/inspect. Defaults to every discovered app.'),
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details.'),
    },
  },
  async ({ name, detail }) => {
    const apps = name
      ? [appSummary(resolveArestApp(name, appRegistryOptions()), (detail ?? 'summary') as AppDetail)]
      : checkArestApps(appRegistryOptions()).apps.map((app) => appSummary(app, (detail ?? 'summary') as AppDetail))
    return textResult({
      registration: 'directory-derived',
      writes_catalog_facts: false,
      active_app: activeApp.name,
      apps_dir: AREST_APPS_DIR || undefined,
      registered_apps: apps,
    })
  },
)

server.registerTool(
  'apps.use',
  {
    description: 'Switch the active local AREST app. Subsequent local reads/writes and context receipts use that app scope.',
    inputSchema: {
      name: z.string().describe('AREST app name under the apps directory.'),
    },
  },
  async ({ name }) => {
    const candidate = resolveArestApp(name, appRegistryOptions())
    const health = inspectArestApp(candidate, appRegistryOptions()).health
    if (health.status === 'library') {
      return textResult({
        error: 'app_is_library',
        message: 'This registry entry is a library, not an app UoD. It cannot be activated.',
        app: appSummary(candidate, 'full'),
      })
    }
    if (health.status === 'not_found') {
      return textResult({
        error: 'app_not_found',
        message: 'No app or library root exists for this name under the apps directory.',
        app: appSummary(candidate, 'full'),
      })
    }
    const app = activateApp(name)
    return textResult({ active_app: appSummary(app, 'full'), context: currentMutationContext() })
  },
)

server.registerTool(
  'apps.create',
  {
    description: 'Create a local AREST app directory with readings storage. Optionally write an initial reading, compile it to the app DB, and activate the app.',
    inputSchema: {
      name: z.string().describe('New AREST app name under the apps directory.'),
      reading: z.string().optional().describe('Optional initial FORML2 reading text to write to readings/app.md.'),
      compile: z.boolean().optional().describe('Compile the app readings into its SQLite DB after creation. Default false.'),
      activate: z.boolean().optional().describe('Make this app active after creation. Default true.'),
    },
  },
  async ({ name, reading, compile, activate }) => {
    if (AREST_MODE !== 'local') return textResult({ error: 'apps.create requires local mode' })

    let app = createArestApp(name, appRegistryOptions(), reading)
    let result: Record<string, unknown> | null = null
    if (compile) {
      const before = inspectArestApp(app, appRegistryOptions())
      if (before.health.readings.count === 0) {
        result = { ok: false, skipped: true, error: 'app has no .md readings to compile' }
      } else {
        const raw = await compileAppReadings(app)
        result = compileResult(raw)
        app = resolveArestApp(app.name, appRegistryOptions())
      }
    }
    if (activate !== false) app = activateApp(app.name)

    return textResult({
      app: appSummary(app, 'full'),
      compile_result: result,
      context: app.name === activeApp.name ? currentMutationContext() : undefined,
    })
  },
)

server.registerTool(
  'apps.compile',
  {
    description: 'Compile an app readings directory into that app SQLite DB. This refreshes the app DB from readings as the source of truth.',
    inputSchema: {
      name: z.string().optional().describe('AREST app name. Defaults to the active app.'),
      activate: z.boolean().optional().describe('Make the compiled app active. Default true when name is provided, otherwise leaves current active app.'),
    },
  },
  async ({ name, activate }) => {
    if (AREST_MODE !== 'local') return textResult({ error: 'apps.compile requires local mode' })

    const target = name ? resolveArestApp(name, appRegistryOptions()) : activeApp
    const before = inspectArestApp(target, appRegistryOptions())
    if (before.health.status === 'library') {
      return textResult({
        error: 'app_is_library',
        message: 'This registry entry is a library, not an app UoD. It is not compiled to its own SQLite DB.',
        app: appSummary(target, 'full'),
      })
    }
    if (before.health.readings.count === 0) {
      return textResult({
        error: 'app_readings_missing',
        message: 'apps.compile requires at least one .md file in the app readings directory.',
        app: appSummary(target, 'full'),
      })
    }
    const raw = await compileAppReadings(target)
    const refreshed = resolveArestApp(target.name, appRegistryOptions())
    const shouldActivate = name ? activate !== false : activate === true
    if (shouldActivate) activateApp(refreshed.name)

    return textResult({
      app: appSummary(refreshed, 'full'),
      compile_result: compileResult(raw),
      active_app: appSummary(activeApp, 'summary'),
      context: refreshed.name === activeApp.name ? currentMutationContext() : undefined,
    })
  },
)

// ── 1. get: retrieve an entity or list entities ──────────────────────

server.registerTool(
  'get',
  {
    description: 'Get an entity by ID, or list all entities of a noun type. Returns the entity with its current state, HATEOAS links, and navigation.',
    inputSchema: {
      id: z.string().optional().describe('Entity ID. If omitted, lists all entities of the noun type.'),
      noun: z.string().optional().describe('Noun type (e.g. "Order"). Required when listing, optional when getting by ID (inferred from population).'),
    },
  },
  async ({ id, noun }) => {
    if (!noun) return textResult({ error: 'Provide noun to get or list.' })

    // Check if this noun is backed by an external system (data federation).
    const fedConfig = await getFederationConfig(noun)
    if (fedConfig) {
      const data = await federatedFetch(fedConfig, id || undefined)
      // Absorb fetched facts + Citation into P so downstream constraints
      // and derivations over the unified population see the federated
      // data. Errors are non-fatal — the fetched result is still
      // returned to the caller either way.
      const citeId = await absorbFederatedIntoD(noun, data)
      if (citeId) {
        return textResult(enrichResponseWithCitation(data, citeId, '/arest/default'))
      }
      return textResult(data)
    }

    // Local population
    if (AREST_MODE === 'local') {
      if (id) {
        const raw = await systemCall(`get:${noun}`, id)
        try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
      }
      const raw = await systemCall(`list:${noun}`, '')
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const path = id
      ? `/arest/default/${encodeURIComponent(noun)}/${encodeURIComponent(id)}`
      : `/arest/default/${encodeURIComponent(noun)}`
    const data = await httpRequest(path)
    return textResult(data)
  },
)

// ── 2. query: query facts across the population ──────────────────────

/**
 * Translate the engine's raw response to the user-facing tuple list.
 *
 * #821: When `query:<ft>` isn't in DEFS (FT name unknown to the
 * schema), `apply` returns `Object::Bottom` which serializes to "⊥".
 * The user-facing answer to "give me facts of type X" is always a
 * list of tuples; an unknown FT yields the same empty list as a
 * known FT with no matching population. Whitepaper §3 ("DEFS holds
 * compiled readings + functions registered by the runtime") casts
 * this as a platform-layer translation: the engine faithfully
 * signals "no def by that name" via Bottom; the MCP runtime maps
 * that to the user-friendly empty tuples list.
 *
 * Other JSON.parse failures still surface as { raw } so genuinely
 * malformed engine responses aren't swallowed silently.
 */
export function parseQueryResponse(raw: string): unknown {
  if (raw === '⊥') return []
  try {
    const parsed = JSON.parse(raw)
    return parsed ?? []
  } catch {
    return { raw }
  }
}

server.registerTool(
  'query',
  {
    description: 'Query facts by fact type. Returns matching facts from the population. Use to explore relationships between entities.',
    inputSchema: {
      fact_type: z.string().describe('Fact type ID (e.g. "Order_was_placed_by_Customer", "Case_has_Observation")'),
      filter: z.record(z.string(), z.string()).optional().describe('Filter by role bindings (e.g. {"Case": "The Speckled Band"})'),
    },
  },
  async ({ fact_type, filter }) => {
    if (AREST_MODE === 'local') {
      const filterStr = filter ? JSON.stringify(filter) : ''
      const raw = await systemCall(`query:${fact_type}`, filterStr)
      return textResult(parseQueryResponse(raw))
    }
    const data = await httpRequest(`/arest/default/query/${encodeURIComponent(fact_type)}`, {
      method: 'POST',
      body: JSON.stringify({ filter }),
    })
    return textResult(data)
  },
)

// ── 2b. sql: read-only SELECT over the relational substrate (#864) ──
//
// Cells ARE relations (RMAP / whitepaper §3). Each FactType cell maps
// to a SQL table named `ft_<sanitize(ft_id)>` whose columns are the
// role names (spaces → underscores). For example, the cell
// `Task_has_Task_Priority` becomes the table `ft_Task_has_Task_Priority`
// with columns `Task` and `Task_Priority`.
//
// `query fact_type=X filter={k:v}` is a degenerate single-table SELECT
// with one WHERE clause; `sql` lifts that to the full SQLite SELECT
// surface (JOINs, subqueries, NOT EXISTS, GROUP BY) — the natural
// language for cross-FT projection. Mutating SQL is refused; INSERT /
// UPDATE / DELETE go through `apply` so derivation, validation, and
// emit run as usual.
//
// Returns `{rows: [{col: val, …}, …]}` on success or `{error: "…"}`
// on parse / exec failure. Errors are always JSON envelopes — no
// thrown exceptions on bad SQL.
export function parseSqlResponse(raw: string): unknown {
  if (raw === '⊥') return { error: 'engine returned ⊥ (handle missing or local feature unavailable)' }
  try {
    const parsed = JSON.parse(raw)
    return parsed ?? { error: 'engine returned null envelope' }
  } catch {
    return { error: 'malformed sql envelope', raw }
  }
}

server.registerTool(
  'sql',
  {
    description: 'Read-only SQL SELECT over the relational substrate (#864). Each FactType cell becomes a SQL table named ft_<FactType_id> with columns matching the role names (spaces → underscores). Example: SELECT "Task" FROM ft_Task_has_Task_Priority WHERE "Task_Priority" = \'p0\'. Returns {rows: [{col: val, ...}, ...]} on success or {error: "..."} on parse / exec failure. Use for cross-fact-type joins, NOT EXISTS, and any projection beyond a single fact type with one WHERE clause — for those, prefer `query`. Mutating SQL (INSERT / UPDATE / DELETE) is refused; use `apply` instead.',
    inputSchema: {
      query: z.string().describe('A SQL SELECT statement. Tables are ft_<FactType_id> (e.g. ft_Task_has_Task_Priority); columns are role names with spaces replaced by underscores. Quote both identifiers and string values per SQL standard.'),
    },
  },
  async ({ query }) => {
    if (AREST_MODE === 'local') {
      const raw = await systemCall('sql', query)
      return textResult(parseSqlResponse(raw))
    }
    const data = await httpRequest('/arest/default/sql', {
      method: 'POST',
      body: JSON.stringify({ query }),
    })
    return textResult(data)
  },
)

// ── 2c. cells: list / get / trace over the cell graph (#870) ──────────
//
// Sister to `sql` (#864): where `sql` materializes per-FT relational
// tables for cross-FT JOINs, `cells` exposes the flat cell-graph
// view — what cells exist, how big they are, what's in them, and
// which derivation rules drive them. Closes the introspection gap
// that previously sent agents to `sqlite3 cells …` for every diagnostic
// question (find malformed cells, check derivation rule outputs,
// verify what compile wrote).
//
// Three modes (chosen via the `mode` parameter):
//
//   list  — `{cells: [{name, size_bytes}, ...]}`
//           Filtered by an optional glob pattern (`*` and `?`
//           wildcards anchored at both ends). `pattern: 'Task_*'`
//           returns only the Task fact-type cells; `pattern: '*'`
//           (the default) returns every cell.
//
//   get   — `{name, contents: <parsed-tuple-list>, size_bytes}`
//           Parses the FFP-encoded cell contents into a JSON array
//           of role-keyed objects (so `Task_has_Task_Priority` rows
//           come back as `[{Task: "1", "Task Priority": "p0"}, ...]`).
//           Returns `{error}` when the cell is absent.
//
//   trace — `{rule_text, consequent_cell, materialized_count}`
//           Looks up a derivation rule by `rule_id` (exact match on
//           the DerivationRule cell's `id` field) or `rule_pattern`
//           (substring match on rule text — first hit wins).
//           `materialized_count` reports the row count of the
//           consequent cell so callers can verify the rule actually
//           fired during the last forward-chain pass.
//
// Returns `{error}` envelopes uniformly on parse / lookup failure;
// no thrown exceptions for malformed input.
export function parseCellsResponse(raw: string): unknown {
  if (raw === '⊥') return { error: 'engine returned ⊥ (handle missing or std-deps feature unavailable)' }
  try {
    const parsed = JSON.parse(raw)
    return parsed ?? { error: 'engine returned null envelope' }
  } catch {
    return { error: 'malformed cells envelope', raw }
  }
}

server.registerTool(
  'cells',
  {
    description: 'Read-only cell-graph introspection (#870). Three modes: `list` returns {cells:[{name,size_bytes},...]} optionally filtered by a glob pattern (e.g. "Task_*"); `get` returns {name,contents,size_bytes} for a single cell with FFP-parsed contents; `trace` returns {rule_text,consequent_cell,materialized_count} for a derivation rule (look up by rule_id exact match or rule_pattern substring match). Use this instead of dropping to `sqlite3 cells …` when you need to know what cells exist, inspect a cell\'s contents, or verify a derivation rule populated its consequent cell. For cross-FT JOINs use `sql` instead.',
    inputSchema: {
      mode: z.enum(['list', 'get', 'trace']).describe('Introspection mode: list, get, or trace.'),
      pattern: z.string().optional().describe('Glob pattern for `list` (e.g. "Task_*", "*derivation*"). Defaults to "*" — all cells.'),
      name: z.string().optional().describe('Exact cell name for `get` (e.g. "FactType", "Task_has_Task_Priority"). Required for get mode.'),
      rule_id: z.string().optional().describe('Exact match on a DerivationRule.id for `trace` mode. Provide either rule_id or rule_pattern.'),
      rule_pattern: z.string().optional().describe('Substring match on a DerivationRule.text for `trace` mode. First match wins.'),
    },
  },
  async ({ mode, pattern, name, rule_id, rule_pattern }) => {
    const envelope: Record<string, string> = { mode }
    if (pattern !== undefined) envelope.pattern = pattern
    if (name !== undefined) envelope.name = name
    if (rule_id !== undefined) envelope.rule_id = rule_id
    if (rule_pattern !== undefined) envelope.rule_pattern = rule_pattern
    if (AREST_MODE === 'local') {
      const raw = await systemCall('cells', JSON.stringify(envelope))
      return textResult(parseCellsResponse(raw))
    }
    const data = await httpRequest('/arest/default/cells', {
      method: 'POST',
      body: JSON.stringify(envelope),
    })
    return textResult(data)
  },
)

// ── 2d. induce: search for Hypothesis Candidates (#854) ──────────────
//
// Wraps the engine's `induce` Func::Platform (registered #846, search
// loop landed in #851 commit 14ebcfdc, ranking landed in #852 commit
// b6235cc6). Until this verb landed, induce was only callable via
// direct `Func::Platform("induce")` in tests; the MCP shim makes it
// routine for agents.
//
// Input envelope (mirrors what `platform_induce` parses off `x`):
//
//   {
//     "ft_id":      "<FT id to search over, required>",
//     "to_explain": [<InstanceFact ...>],   // optional Seq of facts
//     "bound":      {"…": "…"}              // optional binding map
//   }
//
// `to_explain` and `bound` are optional. Empty `to_explain` means
// open-ended search (every constraint-satisfying candidate ranked by
// the user's Scoring Rules); empty `bound` is the default case where
// no role is fixed up front.
//
// Output: a `Seq<Hypothesis Candidate>` (whatever `run_search`
// returns; see `induce::build_hypothesis_candidate`). The MCP shim
// is a pass-through over the JSON envelope — sort order is preserved
// (Confidence-Score-descending, see `induce::run_search`'s stable
// sort) because the parser doesn't re-sort.
//
// On engine error (handle missing, ft_id absent from FactType cell)
// `platform_induce` returns `Object::phi`, which serializes to the
// JSON `[]` — visible to callers as "induce ran but found nothing".
// True engine ⊥ (handle never registered, build missing the verb)
// translates to a structured `{error}` envelope.
export function parseInduceResponse(raw: string): unknown {
  if (raw === '⊥') return { error: 'engine returned ⊥ (handle missing or induce verb not wired)' }
  try {
    const parsed = JSON.parse(raw)
    // `run_search` returns an empty Vec → `Object::Seq` of length zero
    // → JSON `[]`. `null` likewise translates to the empty list so
    // callers see "no candidates" rather than a nullable surprise.
    if (parsed === null || parsed === undefined) return []
    return parsed
  } catch {
    return { error: 'malformed induce envelope', raw }
  }
}

server.registerTool(
  'induce',
  {
    description: 'Search for Hypothesis Candidates over a fact type using the induce engine (#854). Wraps `Func::Platform("induce")` — enumerates candidates from the cartesian product of the FT\'s role types, gates them through the alethic-violation check, runs the user\'s Scoring Rules, and returns a Seq of Hypothesis Candidates ranked Confidence-Score-descending. `to_explain` (optional) is the seq of InstanceFacts you want the candidate to forward-chain-derive; empty means open-ended search. `bound` (optional) pins specific role values up front. See readings/core/induction.md for the Hypothesis Candidate / Confidence Score / Scoring Rule vocabulary.',
    inputSchema: {
      ft_id: z.string().describe('Fact type id to search over (e.g. "Hypothesis_has_Plausibility").'),
      to_explain: z.array(z.unknown()).optional().describe('Optional seq of InstanceFact-shaped facts the candidate should forward-chain-derive. Empty (default) means open-ended search.'),
      bound: z.record(z.string(), z.string()).optional().describe('Optional pre-bound role values keyed by role name. Constrains the cartesian enumeration to candidates that match these bindings.'),
    },
  },
  async ({ ft_id, to_explain, bound }) => {
    if (AREST_MODE === 'local') {
      // Build the FFP-shaped argument the engine's `platform_induce`
      // parser expects: a Seq of pair-bindings keyed by `ft_id`,
      // `to_explain`, and `bound`. atom-shaped values become
      // `<key, value>` pairs; the seq-shaped `to_explain` becomes
      // `<to_explain, <fact1, fact2, …>>` (the parser walks the
      // pair list to find the seq-valued `to_explain` directly per
      // `platform_induce` doc-comment).
      //
      // Mirrors `escape_atom_for_display` semantics (split_top_level
      // treats `<`, `>`, `,` as separators at depth 0; backslash
      // escapes the next char).
      const escapeAtom = (s: string) => s.replace(/[\\<>,]/g, ch => '\\' + ch)
      const renderValue = (v: unknown): string => {
        if (v === null || v === undefined) return 'φ'
        if (typeof v === 'string') return escapeAtom(v)
        if (typeof v === 'number' || typeof v === 'boolean') return String(v)
        if (Array.isArray(v)) {
          if (v.length === 0) return 'φ'
          return `<${v.map(renderValue).join(', ')}>`
        }
        if (typeof v === 'object') {
          const pairs = Object.entries(v as Record<string, unknown>)
            .map(([k, val]) => `<${escapeAtom(k)}, ${renderValue(val)}>`)
          return `<${pairs.join(', ')}>`
        }
        return escapeAtom(String(v))
      }
      const pairs: string[] = [`<ft_id, ${escapeAtom(ft_id)}>`]
      if (to_explain !== undefined) {
        pairs.push(`<to_explain, ${renderValue(to_explain)}>`)
      }
      if (bound !== undefined) {
        pairs.push(`<bound, ${renderValue(bound)}>`)
      }
      const arg = `<${pairs.join(', ')}>`
      const raw = await systemCall('induce', arg)
      return textResult(parseInduceResponse(raw))
    }
    const data = await httpRequest('/arest/default/induce', {
      method: 'POST',
      body: JSON.stringify({ ft_id, to_explain, bound }),
    })
    return textResult(data)
  },
)

// ── 2e. orient: one-screen session re-orientation (#871) ─────────────
//
// Per #869 (MCP UX north-star: agents get value without reading the
// whitepaper), every fresh session today re-discovers the landscape
// via 5-6 separate calls — `apps_list`, `apps_current`, `query` for
// task counts, `cells trace` for the latest derivation activity. One
// envelope returning that entire picture makes re-entry instant.
//
// Returns:
//
//   {
//     "apps":           [{name, root, last_compile, ready_count,
//                         in_progress_count, completed_count}, ...],
//     "active_app":     "tasks" | null,
//     "recent_changes": [{kind, noun, count}, ...],
//     "suggested_next": "Try: ..."
//   }
//
// Counts come from the active app's loaded snapshot (the engine has
// one DB at a time). Sibling apps in `apps_dir` surface as bare
// entries with `last_compile` from the .db file mtime — the engine
// doesn't open every sibling DB to count its rows. Agents that need
// per-app counts call `apps_use` then `orient` again.
//
// Returns `{error}` envelope on malformed input — never throws so the
// verb stays usable as the agent's recovery path when other things
// have already gone wrong in the session.
export function parseOrientResponse(raw: string): unknown {
  if (raw === '⊥') return { error: 'engine returned ⊥ (handle missing or std-deps feature unavailable)' }
  try {
    const parsed = JSON.parse(raw)
    return parsed ?? { error: 'engine returned null envelope' }
  } catch {
    return { error: 'malformed orient envelope', raw }
  }
}

server.registerTool(
  'orient',
  {
    description: 'Read-only session re-orientation envelope (#871). Returns one screen of "where are we": the apps inventory (active app first with ready/in_progress/completed task counts; sibling apps with last_compile timestamps), the currently active app name, a list of recent cell-graph activity (instance fact cells with their row counts), and a one-line suggested-next pointer. Use as the FIRST call in a new session to skip the usual 5-6 probing verbs (apps_list + apps_current + query Task_has_Task_Status + cells trace ...). Optional inputs: `apps_dir` to enumerate sibling apps from disk; `active_app` so the suggestion template names the current app. No state mutation.',
    inputSchema: {
      apps_dir: z.string().optional().describe('Optional absolute path to the apps directory. When set, sibling apps are enumerated from filesystem (each must carry a `readings/` directory and a `*.db` file). When omitted, only the active app is reported.'),
      active_app: z.string().optional().describe('Optional active app name. The verb uses this to name the active entry in the apps list and to render the suggested_next template. When omitted, suggested_next falls back to a "pick an app" pointer.'),
    },
  },
  async ({ apps_dir, active_app }) => {
    const envelope: Record<string, string> = {}
    if (apps_dir !== undefined) envelope.apps_dir = apps_dir
    if (active_app !== undefined) envelope.active_app = active_app
    if (AREST_MODE === 'local') {
      const raw = await systemCall('orient', JSON.stringify(envelope))
      return textResult(parseOrientResponse(raw))
    }
    const data = await httpRequest('/arest/default/orient', {
      method: 'POST',
      body: JSON.stringify(envelope),
    })
    return textResult(data)
  },
)

// ── 3. apply: create, update, or transition an entity ────────────────

server.registerTool(
  'apply',
  {
    description: `Apply an operation to an entity. The operation determines behavior: create (new entity), update (modify fields), transition (fire SM event). Executes the AREST pipeline: resolve -> derive -> validate -> emit. ${MUTATION_TOOL_DESCRIPTION}`,
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      operation: z.enum(['create', 'update', 'transition']).describe('Operation type'),
      noun: z.string().describe('Entity noun type (e.g. "Order", "Case")'),
      id: z.string().optional().describe('Entity ID. Required for update/transition. Optional for create (auto-generated).'),
      fields: z.record(z.string(), z.string()).optional().describe('Fact pairs for create/update (e.g. {"Name": "Acme", "customer": "alice"})'),
      event: z.string().optional().describe('SM event for transition (e.g. "place", "ship")'),
      sender: z.string().optional().describe('Caller identity for authorization'),
      signature: z.string().optional().describe('HMAC-SHA256 signature'),
    },
  },
  async ({ context_receipt, operation, noun, id, fields, event, sender, signature }) => {
    const blocked = mutationGateResult('apply', context_receipt, { operation, noun, id, fields, event })
    if (blocked) return blocked

    if (AREST_MODE === 'local') {
      // Mirrors `escape_atom_for_display` in crates/arest/src/ast.rs.
      // Engine's Object::parse uses split_top_level which treats `,`,
      // `<`, `>` as syntactic separators at depth 0; backslash escapes
      // the next char. Without this, a field value containing any of
      // those (e.g. a Task Description with a comma) gets silently
      // truncated at the first unescaped comma.
      const escapeAtom = (s: string) => s.replace(/[\\<>,]/g, ch => '\\' + ch)
      switch (operation) {
        case 'create': {
          const pairs = Object.entries(fields || {}).map(([k, v]) => `<${escapeAtom(k)}, ${escapeAtom(v)}>`).join(', ')
          const idPair = id ? `<id, ${escapeAtom(id)}>, ` : ''
          const raw = await systemCall(`create:${noun}`, `<${idPair}${pairs}>`)
          return localApplyResult(raw, { operation, noun, id, fields })
        }
        case 'update': {
          const pairs = Object.entries(fields || {}).map(([k, v]) => `<${escapeAtom(k)}, ${escapeAtom(v)}>`).join(', ')
          const raw = await systemCall(`update:${noun}`, `<<id, ${escapeAtom(id || '')}>${pairs ? `, ${pairs}` : ''}>`)
          return localApplyResult(raw, { operation, noun, id, fields })
        }
        case 'transition': {
          const raw = await systemCall(`transition:${noun}`, `<${escapeAtom(id || '')}, ${escapeAtom(event || '')}>`)
          return localApplyResult(raw, { operation, noun, id })
        }
      }
    }
    // Remote mode: dispatch via HTTP
    const command = operation === 'create'
      ? { type: 'createEntity', noun, domain: '', id, fields, sender, signature }
      : operation === 'update'
        ? { type: 'updateEntity', noun, domain: '', entityId: id, fields, sender, signature }
        : { type: 'transition', entityId: id, event, domain: '', sender, signature }
    const data = await httpRequest('/arest/default/apply', { method: 'POST', body: JSON.stringify(command) })
    return textResult(data)
  },
)

// ── 4. actions: get valid actions for an entity (HATEOAS) ────────────

server.registerTool(
  'actions',
  {
    description: 'Get valid actions for an entity. Returns available SM transitions, navigation links (parent/child), and applicable operations. Pure HATEOAS — the agent discovers what is possible without knowing the schema.',
    inputSchema: {
      noun: z.string().describe('Entity noun type'),
      id: z.string().describe('Entity ID'),
      status: z.string().optional().describe('Current SM status (resolved from state if omitted)'),
    },
  },
  async ({ noun, id, status }) => {
    if (AREST_MODE === 'local') {
      const parseOr = <T>(raw: string, fallback: T): T | any => {
        try { const v = JSON.parse(raw); return v ?? fallback } catch { return fallback }
      }
      // Resolve current status from the SM entity keyed by this id when the
      // caller doesn't pass one — transitions:{noun} needs a status to filter
      // outgoing edges, otherwise it returns [].
      let resolvedStatus = status || ''
      if (!resolvedStatus) {
        const smRaw = await systemCall(`get:State Machine`, id)
        const sm = parseOr(smRaw, null)
        if (sm && typeof sm === 'object' && typeof sm.currentlyInStatus === 'string') {
          resolvedStatus = sm.currentlyInStatus
        }
      }
      const rawTransitions = await systemCall(`transitions:${noun}`, resolvedStatus)
      const rawEntity = await systemCall(`get:${noun}`, id)
      const parsedTransitions = parseOr(rawTransitions, null)
      return textResult({
        entity: id,
        noun,
        status: resolvedStatus || null,
        transitions: Array.isArray(parsedTransitions)
          ? parsedTransitions
          : normalizeTransitionRows(rawTransitions, noun, id),
        entity_data: parseOr(rawEntity, null),
      })
    }
    const data = await httpRequest(`/arest/default/${encodeURIComponent(noun)}/${encodeURIComponent(id)}/actions`)
    return textResult(data)
  },
)

// ── 5. explain: derivation trace for a fact or entity ────────────────

server.registerTool(
  'explain',
  {
    description: 'Explain how a fact was derived or why an entity is in its current state. Returns the derivation chain: which rules fired, in what order, producing which facts. Also shows the audit trail for the entity.',
    inputSchema: {
      id: z.string().describe('Entity ID'),
      noun: z.string().optional().describe('Entity noun type'),
      fact: z.string().optional().describe('Specific fact to explain (e.g. "status", "Hypothesis_explains_Observation")'),
    },
  },
  async ({ id, noun, fact }) => {
    if (AREST_MODE === 'local') {
      // Audit trail for this entity
      const auditRaw = await systemCall('audit', '0')
      let audit: any[] = []
      try {
        const parsed = JSON.parse(auditRaw)
        if (Array.isArray(parsed)) audit = parsed
      } catch {}

      // If a specific fact type is requested, query it
      let factData: any = []
      if (fact) {
        const raw = await systemCall(`query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {}))
        try {
          const parsed = JSON.parse(raw)
          factData = parsed ?? []
        } catch { factData = raw }
      }

      return textResult({
        entity: id,
        fact_query: factData,
        audit_trail: audit.filter((a: any) => a?.entity === id || a?.resource === id),
      })
    }
    const data = await httpRequest(`/arest/default/explain/${encodeURIComponent(id)}`)
    return textResult(data)
  },
)

// ── 6. compile: ingest FORML2 readings (self-modification) ───────────

server.registerTool(
  'compile',
  {
    description: `Compile FORML2 readings into the engine (self-modification, Corollary 5). The engine extends its own program. New nouns, fact types, constraints, derivation rules, and state machines become active immediately. Alethic violations reject. ${MUTATION_TOOL_DESCRIPTION}`,
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      readings: z.string().describe('FORML2 readings as markdown text'),
    },
  },
  async ({ context_receipt, readings }) => {
    const blocked = mutationGateResult('compile', context_receipt, { readings })
    if (blocked) return blocked

    if (AREST_MODE === 'local') {
      const raw = await systemCall('compile', readings)
      const ok = !raw.startsWith('⊥')
      let result: any = raw
      try { result = JSON.parse(raw) } catch {}
      return textResult({ ok, result })
    }
    const data = await httpRequest('/parse', {
      method: 'POST',
      body: JSON.stringify({ text: readings }),
    })
    return textResult(data)
  },
)

// ── Utility: schema ──────────────────────────────────────────────────

server.registerTool(
  'schema',
  {
    description: 'Get the full schema: nouns, fact types, constraints, state machines, derivation rules.',
  },
  async () => {
    if (AREST_MODE === 'local') {
      const data = await dispatchRead('/schema')
      return textResult(data)
    }
    const data = await httpRequest('/arest/default/schema')
    return textResult(data)
  },
)

// ── Utility: verify signature ────────────────────────────────────────

server.registerTool(
  'verify',
  {
    description: 'Verify an HMAC-SHA256 signature over sender + payload.',
    inputSchema: {
      sender: z.string().describe('Claimed sender identity'),
      payload: z.string().describe('Signed payload'),
      signature: z.string().describe('Signature to verify'),
    },
  },
  async ({ sender, payload, signature }) => {
    if (AREST_MODE === 'local') {
      const encoded = `<${sender},${payload},${signature}>`
      const raw = await systemCall('verify_signature', encoded)
      return textResult({ valid: raw === 'true' })
    }
    const data = await httpRequest('/crypto/verify', {
      method: 'POST',
      body: JSON.stringify({ sender, payload, signature }),
    })
    return textResult(data)
  },
)

// ── select_component (#493): AI agents query the Component registry ──
//
// Composes UIs by description rather than by toolkit knowledge. Routes
// through to the engine-side handler (command::select_component) via
// the `select_component` system intercept added in lib.rs. Mirrors
// `query`'s request/response shape — JSON in, JSON list out — so an
// LLM tool call can spell:
//
//   select_component({
//     intent: "I need a date picker",
//     constraints: { touch: true, a11y: ["screen_reader"], theme: "dark" }
//   })
//
// and get back a ranked list of {component, role, toolkit, symbol,
// score} records. Selection is metamodel-resident (HHHH's #492 rules
// re-implemented in Rust for sub-millisecond latency); picks are
// reproducible across runs.
server.registerTool(
  'select_component',
  {
    description: 'Select a UI Component implementation by intent and MonoView constraints. Returns a ranked list of (component, toolkit, symbol, score) tuples drawn from the Component registry. Scoring mirrors the metamodel selection rules (touch / density / a11y / theme / surface tier / kernel-resident preferences). Use when an AI agent needs to compose a UI without knowing toolkit names.',
    inputSchema: {
      intent: z.string().describe('Natural-language description of the widget you need (e.g. "I need a date picker"). Matched by case-insensitive substring against the Component Role.'),
      interaction_mode: z.enum(['pointer', 'keyboard', 'touch']).optional().describe('MonoView interaction mode'),
      density: z.enum(['compact', 'regular', 'spacious']).optional().describe('MonoView density scale'),
      a11y: z.array(z.string()).optional().describe('A11y profiles, e.g. ["screen_reader", "high-contrast"]'),
      theme: z.string().optional().describe('Theme mode, e.g. "dark"'),
      surface: z.enum(['backdrop', 'panel', 'overlay', 'drop-shadow']).optional().describe('Surface tier'),
      touch: z.boolean().optional().describe('Convenience: sets interaction_mode="touch" when true'),
      limit: z.number().optional().describe('Max results to return (default 5)'),
    },
  },
  async ({ intent, interaction_mode, density, a11y, theme, surface, touch, limit }) => {
    const constraints: Record<string, any> = {}
    if (interaction_mode !== undefined) constraints.interactionMode = interaction_mode
    if (density !== undefined) constraints.density = density
    if (a11y !== undefined) constraints.a11y = a11y
    if (theme !== undefined) constraints.theme = theme
    if (surface !== undefined) constraints.surface = surface
    if (touch !== undefined) constraints.touch = touch
    if (limit !== undefined) constraints.limit = limit
    const body = JSON.stringify({ intent: intent || '', constraints })
    if (AREST_MODE === 'local') {
      const raw = await systemCall('select_component', body)
      try {
        const parsed = JSON.parse(raw)
        return textResult(parsed ?? [])
      } catch {
        return textResult({ raw })
      }
    }
    const data = await httpRequest('/arest/default/select_component', {
      method: 'POST',
      body,
    })
    return textResult(data)
  },
)

// =====================================================================
// EVOLUTION — governed self-modification via Domain Change
// =====================================================================
//
// propose is sugar over: create Domain Change + attach proposed elements.
// The Domain Change state machine (Proposed → Under Review → Approved →
// Applied) enforces review before schema changes take effect. For
// immediate self-modification (Corollary 5), use compile directly.

server.registerTool(
  'propose',
  {
    description: `Propose a change to the schema or population. Creates a Domain Change entity with the proposed elements (readings, nouns, fact types, constraints, verbs, state machines). Enters the review workflow at status "Proposed". Use transition to advance through Under Review -> Approved -> Applied. For immediate changes bypassing review, use compile directly. ${MUTATION_TOOL_DESCRIPTION}`,
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      rationale: z.string().describe('Why this change is needed'),
      target_domain: z.string().describe('Domain slug to change (e.g. "orders", "core")'),
      readings: z.array(z.string()).optional().describe('FORML2 reading text to add'),
      nouns: z.array(z.string()).optional().describe('Noun names to declare'),
      constraints: z.array(z.string()).optional().describe('Constraint texts'),
      verbs: z.array(z.string()).optional().describe('Verb names to declare'),
    },
  },
  async ({ context_receipt, rationale, target_domain, readings, nouns, constraints, verbs }) => {
    const blocked = mutationGateResult('propose', context_receipt, { rationale, target_domain, readings, nouns, constraints, verbs })
    if (blocked) return blocked

    if (AREST_MODE !== 'local') return textResult({ error: 'propose requires local mode' })

    // Generate a stable change ID from the rationale + time.
    const changeId = `dc-${Date.now().toString(36)}`

    // Create the Domain Change entity.
    const createCmd = {
      op: 'create',
      noun: 'Domain Change',
      domain: 'evolution',
      id: changeId,
      fields: {
        'Change Id': changeId,
        rationale,
        targetDomain: target_domain,
      },
    }
    const createRaw = await systemCall(`create:Domain Change`, JSON.stringify(createCmd))
    let createResult: any
    try { createResult = JSON.parse(createRaw) } catch { createResult = { raw: createRaw } }

    // Attach proposed elements as facts.
    const proposals: Record<string, any> = {}
    if (readings?.length) proposals.readings = readings
    if (nouns?.length) proposals.nouns = nouns
    if (constraints?.length) proposals.constraints = constraints
    if (verbs?.length) proposals.verbs = verbs

    return textResult({
      change_id: changeId,
      status: 'Proposed',
      rationale,
      target_domain,
      proposals,
      create_result: createResult,
      next_actions: [
        { tool: 'transition', args: { noun: 'Domain Change', id: changeId, event: 'review' } },
        { tool: 'transition', args: { noun: 'Domain Change', id: changeId, event: 'approve-change' } },
        { tool: 'transition', args: { noun: 'Domain Change', id: changeId, event: 'apply' } },
      ],
    })
  },
)

// =====================================================================
// LLM BRIDGE — natural-language ↔ formal facts via client sampling
// =====================================================================
//
// These tools use MCP sampling (server.server.createMessage) to request
// LLM completions from the CLIENT'S LLM session. The server composes
// prompts using the schema as context, then runs an engine operation
// with the LLM's response. This inverts the usual agent/tool pattern:
// the engine orchestrates LLM reasoning, not the other way around.

/** Helper to extract text from an LLM sampling response. */
function samplingText(response: any): string {
  const content = response.content
  if (Array.isArray(content)) {
    for (const block of content) {
      if (block.type === 'text') return block.text
    }
    return ''
  }
  return content?.type === 'text' ? content.text : ''
}

/** Strip markdown code fences and parse JSON. */
function parseJsonFromLlm(text: string): any {
  const clean = text.replace(/^```(?:json)?\s*/m, '').replace(/\s*```\s*$/m, '').trim()
  return JSON.parse(clean)
}

/**
 * Try MCP client sampling; on failure return the prompt for manual execution.
 * Callers that already have a sampled response (e.g. the outer agent ran the
 * prompt itself) can pass it in `precomputed` to skip the sampling roundtrip
 * entirely. This keeps the tools composable with agents that do their own
 * sampling, and ensures clients without sampling get a useful payload rather
 * than an error blob.
 */
async function tryLlmSample(
  prompt: string,
  maxTokens: number,
  precomputed?: string,
): Promise<{ ok: boolean; text: string; reason: string; details: string }> {
  if (precomputed && precomputed.trim()) {
    return { ok: true, text: precomputed, reason: '', details: '' }
  }
  try {
    const response = await (server as any).server.createMessage({
      messages: [{ role: 'user', content: { type: 'text', text: prompt } }],
      maxTokens,
    })
    return { ok: true, text: samplingText(response), reason: '', details: '' }
  } catch (e: any) {
    return {
      ok: false,
      text: '',
      reason: 'client does not support MCP sampling (or sampling failed)',
      details: String(e?.message || e),
    }
  }
}

/**
 * Build a uniform prompt-only fallback payload. Surfaces the prompt the tool
 * would have sampled, plus a `next_step` telling the caller how to proceed:
 * run the prompt against any LLM and re-invoke the tool with the result in
 * the `llm_response` arg.
 */
function promptOnlyFallback(
  toolName: string,
  prompt: string,
  reason: string,
  context: Record<string, any> = {},
) {
  return textResult({
    mode: 'prompt-only',
    reason,
    prompt,
    next_step: `Run the prompt against any LLM, then re-invoke \`${toolName}\` with the result passed as \`llm_response\` to complete the operation.`,
    ...context,
  })
}

// ── ask: natural-language query → project → results ──────────────────

server.registerTool(
  'ask',
  {
    description: 'Translate a natural-language question into a projection query (fact_type + filter), execute it against the population, and return matching facts. Use for read-only questions answered directly from facts. For prose answers use synthesize. If the caller has already run the projection prompt elsewhere, pass the JSON result in llm_response to skip sampling.',
    inputSchema: {
      question: z.string().describe('Natural language question, e.g. "How many orders did acme place this month?"'),
      noun: z.string().optional().describe('Optional scope hint: fact type or entity noun name'),
      llm_response: z.string().optional().describe('Pre-sampled JSON projection spec (skip client sampling). Shape: {"fact_type":..., "filter":{...}}'),
    },
  },
  async ({ question, noun, llm_response }) => {
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'ask requires local mode' })
    }
    const schemaRaw = noun
      ? await systemCall(`schema:${noun}`, '')
      : await systemCall('list:Noun', '')

    const prompt = `You are translating a natural-language question into a projection query.

Schema:
${schemaRaw}

Question: ${question}

Respond with JSON ONLY in this format:
{"fact_type": "Fact_Type_Name", "filter": {"role1": "value1"}}

Use the exact fact_type names from the schema. Leave filter empty {} if no specific constraint. Do not include explanations.`

    const sample = await tryLlmSample(prompt, 500, llm_response)
    if (!sample.ok) {
      return promptOnlyFallback('ask', prompt, sample.reason, {
        question,
        schema_excerpt_len: schemaRaw.length,
        details: sample.details,
      })
    }

    let spec
    try {
      spec = parseJsonFromLlm(sample.text)
    } catch {
      return textResult({
        error: 'LLM did not return valid JSON projection spec',
        expected_shape: '{"fact_type":"Fact_Type_Name","filter":{"role":"value"}}',
        llm_response: sample.text,
      })
    }

    if (!spec?.fact_type || typeof spec.fact_type !== 'string') {
      return textResult({
        error: 'Projection spec missing fact_type',
        llm_response: sample.text,
      })
    }

    const filterStr = Object.entries(spec.filter || {})
      .map(([k, v]) => `<${k},${v}>`).join('')
    const raw = await systemCall(`query:${spec.fact_type}`, filterStr)
    let results: any
    try {
      const parsed = JSON.parse(raw)
      results = parsed ?? []
    } catch { results = { raw } }

    return textResult({ question, query: spec, results })
  },
)

// ── synthesize: fact bag → derive + verbalize → prose ────────────────

server.registerTool(
  'synthesize',
  {
    description: 'Turn entity facts into concise natural-language prose. Engine first runs the full pipeline (resolve + derive to LFP + validate) so the prose reflects implicit/derived facts, then the client LLM shapes the prose. Engine guarantees content correctness; LLM only shapes wording. Pass llm_response to supply pre-written prose and skip sampling.',
    inputSchema: {
      noun: z.string().describe('Entity noun, e.g. "Order"'),
      id: z.string().optional().describe('Specific entity ID, or synthesize all entities of the noun if omitted'),
      llm_response: z.string().optional().describe('Pre-sampled prose (skip client sampling). Used verbatim as the `prose` field.'),
    },
  },
  async ({ noun, id, llm_response }) => {
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'synthesize requires local mode' })
    }
    const raw = id
      ? await systemCall(`get:${noun}`, id)
      : await systemCall(`list:${noun}`, '')
    let data: any
    try { data = JSON.parse(raw) } catch { data = { raw } }

    const prompt = `Write a clear, natural-language summary of this information. Use only the facts given. Do not invent details. Prefer direct, declarative prose. Keep it concise.

Entity: ${noun}${id ? ` "${id}"` : ' (all instances)'}

Facts:
${JSON.stringify(data, null, 2)}`

    const sample = await tryLlmSample(prompt, 1000, llm_response)
    if (!sample.ok) {
      return promptOnlyFallback('synthesize', prompt, sample.reason, {
        noun,
        id,
        facts: data,
        details: sample.details,
      })
    }

    return textResult({ noun, id, facts: data, prose: sample.text })
  },
)

// ── validate: raw text → extract facts → constraint check ────────────

server.registerTool(
  'validate',
  {
    description: 'Check whether raw text violates a deontic OWA constraint. The client LLM extracts fact instances from the text that match the constraint\'s fact types; the engine then verifies those facts against the constraint without mutating state. Useful for document review and content moderation. Pass llm_response to supply pre-extracted facts (JSON array) and skip sampling.',
    inputSchema: {
      text: z.string().describe('Raw text to check'),
      constraint: z.string().describe('Constraint ID (from compiled defs) or the constraint reading text'),
      llm_response: z.string().optional().describe('Pre-sampled JSON facts array (skip client sampling). Shape: [{"fact_type":..., "bindings":{...}}, ...]'),
    },
  },
  async ({ text, constraint, llm_response }) => {
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'validate requires local mode' })
    }
    const constraintRaw = await systemCall(`constraint:${constraint}`, '').catch(() => '')

    const prompt = `Extract fact instances from the text that are relevant to the given constraint.

Constraint: ${constraintRaw || constraint}

Text to check:
${text}

Respond with JSON ONLY as an array of facts:
[{"fact_type": "Fact_Type_Name", "bindings": {"role1": "value1"}}, ...]

Only include facts clearly stated or strongly implied by the text. Do not invent. Return [] if no relevant facts are present.`

    const sample = await tryLlmSample(prompt, 1500, llm_response)
    if (!sample.ok) {
      return promptOnlyFallback('validate', prompt, sample.reason, {
        text,
        constraint,
        details: sample.details,
      })
    }

    let facts: any
    try {
      facts = parseJsonFromLlm(sample.text)
    } catch {
      return textResult({
        error: 'LLM did not return valid JSON facts array',
        expected_shape: '[{"fact_type":"Fact_Type_Name","bindings":{"role":"value"}}, ...]',
        llm_response: sample.text,
      })
    }

    if (!Array.isArray(facts)) {
      return textResult({
        error: 'LLM response must be a JSON array of facts',
        llm_response: sample.text,
      })
    }

    const violations: any[] = []
    for (const fact of facts) {
      if (!fact?.fact_type || typeof fact.fact_type !== 'string') continue
      const bindings = fact.bindings || {}
      const factStr = Object.entries(bindings)
        .map(([k, v]) => `<${k},${v}>`).join('')
      try {
        const vraw = await systemCall(`verify:${fact.fact_type}`, factStr)
        const result = (() => { try { return JSON.parse(vraw) } catch { return { raw: vraw } } })()
        if (result.violations && result.violations.length > 0) {
          violations.push({ fact, violations: result.violations })
        }
      } catch (e: any) {
        violations.push({ fact, error: String(e?.message || e) })
      }
    }

    return textResult({
      text,
      constraint,
      extracted_facts: facts,
      violations,
      satisfied: violations.length === 0,
    })
  },
)

// ── tutor: interactive three-track walkthrough ───────────────────────
//
// Loads a lesson from tutor/lessons/<track>/<NN>-*.md, returns its
// narrative, and grades the embedded `~~~ expect` predicate against
// the live D. Stateless: the caller passes `track` and `num`; the
// response carries a `next` hint pointing at lesson num+1. The
// grammar of expect predicates is documented in tutor/lessons/_format.md.

type TutorCall = (key: string, input: string) => Promise<string>

function factValue(row: any, role: string): string | undefined {
  if (!row || typeof row !== 'object') return undefined
  const underscore = role.replace(/\s+/g, '_')
  const compact = role.replace(/\s+/g, '')
  const value = row[role] ?? row[underscore] ?? row[compact]
  return value === undefined || value === null ? undefined : String(value)
}

async function tutorQueryRows(call: TutorCall, factType: string): Promise<any[]> {
  const raw = await call(`query:${factType}`, '')
  const parsed = parseEngineRaw(raw, [])
  return Array.isArray(parsed) ? parsed : []
}

export async function readTutorAuthoringWorkflow(
  call: TutorCall = tutorSystemCall,
  status?: string,
) {
  const [
    orderRows,
    situationRows,
    guidanceRows,
    toolRows,
    statusRows,
  ] = await Promise.all([
    tutorQueryRows(call, 'Authoring_Step_has_Authoring_Step_Order'),
    tutorQueryRows(call, 'Authoring_Step_applies_in_Authoring_Situation'),
    tutorQueryRows(call, 'Authoring_Step_has_Authoring_Guidance'),
    tutorQueryRows(call, 'Authoring_Step_recommends_Authoring_Tool'),
    tutorQueryRows(call, 'Authoring_Step_uses_Status'),
  ])

  const steps = new Map<string, {
    step: string
    order?: number
    status?: string
    situation?: string
    guidance?: string
    tools: string[]
  }>()
  const ensureStep = (step: string) => {
    const existing = steps.get(step)
    if (existing) return existing
    const created: {
      step: string
      order?: number
      status?: string
      situation?: string
      guidance?: string
      tools: string[]
    } = { step, tools: [] }
    steps.set(step, created)
    return created
  }

  for (const row of orderRows) {
    const step = factValue(row, 'Authoring Step')
    if (!step) continue
    const record = ensureStep(step)
    const order = Number(factValue(row, 'Authoring Step Order'))
    if (Number.isFinite(order)) record.order = order
  }
  for (const row of situationRows) {
    const step = factValue(row, 'Authoring Step')
    const situation = factValue(row, 'Authoring Situation')
    if (step && situation) ensureStep(step).situation = situation
  }
  for (const row of guidanceRows) {
    const step = factValue(row, 'Authoring Step')
    const guidance = factValue(row, 'Authoring Guidance')
    if (step && guidance) ensureStep(step).guidance = guidance
  }
  for (const row of toolRows) {
    const step = factValue(row, 'Authoring Step')
    const tool = factValue(row, 'Authoring Tool')
    if (!step || !tool) continue
    const tools = ensureStep(step).tools
    if (!tools.includes(tool)) tools.push(tool)
  }
  for (const row of statusRows) {
    const step = factValue(row, 'Authoring Step')
    const stepStatus = factValue(row, 'Status')
    if (step && stepStatus) ensureStep(step).status = stepStatus
  }

  const sortedSteps = [...steps.values()]
    .sort((a, b) => (a.order ?? Number.MAX_SAFE_INTEGER) - (b.order ?? Number.MAX_SAFE_INTEGER))
    .map((step) => ({ ...step, tools: step.tools.sort() }))
  const currentStatus = status ?? sortedSteps[0]?.status ?? ''
  const rawActions = currentStatus
    ? await call('transitions:Authoring Session', currentStatus)
    : '[]'

  return {
    source: {
      kind: 'readings',
      path: 'tutor/domains/authoring.md',
    },
    noun: 'Authoring Session',
    current_status: currentStatus || null,
    current_step: sortedSteps.find((step) => step.status === currentStatus) ?? null,
    steps: sortedSteps,
    actions: normalizeTransitionRows(rawActions, 'Authoring Session', currentStatus),
  }
}

const TUTOR_TRACKS = ['easy', 'medium', 'hard'] as const
type TutorTrack = typeof TUTOR_TRACKS[number]

function tutorLessonsDir(): string {
  return resolve(__dirname, '..', '..', 'tutor', 'lessons')
}

function listTutorLessons(track: TutorTrack): Array<{ num: number; title: string; path: string }> {
  const dir = resolve(tutorLessonsDir(), track)
  if (!existsSync(dir)) return []
  return readdirSync(dir)
    .filter(f => f.endsWith('.md') && /^\d+/.test(f))
    .sort()
    .map(f => {
      const num = parseInt(f.match(/^(\d+)/)![1], 10)
      const body = readFileSync(join(dir, f), 'utf-8')
      const titleLine = body.match(/^#\s+Lesson\s+\S+\s*:\s*(.+)$/m)?.[1]
        ?? body.match(/^#\s+(.+)$/m)?.[1]
        ?? f
      return { num, title: titleLine.trim(), path: join(dir, f) }
    })
}

function parseTutorLesson(content: string): { title: string; expect: string; nextLink: string } {
  const title = (content.match(/^#\s+(.+)$/m)?.[1] ?? '').trim()
  const expectFence = content.match(/~~~\s*expect\s*\n([\s\S]*?)\n~~~/)?.[1] ?? ''
  const nextLink = (content.match(/\*\*Next:\*\*\s*(.+?)$/m)?.[1] ?? '').trim()
  return { title, expect: expectFence.trim(), nextLink }
}

function matchesSubset(actual: any, expected: any): boolean {
  if (expected === null || typeof expected !== 'object') return actual === expected
  if (Array.isArray(expected)) {
    return Array.isArray(actual)
      && expected.length === actual.length
      && expected.every((e, i) => matchesSubset(actual[i], e))
  }
  if (actual === null || typeof actual !== 'object') return false
  return Object.keys(expected).every(k => matchesSubset(actual[k], expected[k]))
}

function cmpNum(actual: number, op: string, expected: number): boolean {
  switch (op) {
    case '==': return actual === expected
    case '>=': return actual >= expected
    case '<=': return actual <= expected
    case '>':  return actual > expected
    case '<':  return actual < expected
    default:   return false
  }
}

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

server.registerTool(
  'tutor',
  {
    description: 'Interactive three-track AREST walkthrough (easy / medium / hard). Load a lesson by track+num and the response includes its narrative, the check predicate, whether the check currently passes against live D (✓/✗), and a pointer to the next lesson. Use command="list" to enumerate all lessons.',
    inputSchema: {
      command: z.enum(['list', 'lesson']).optional().describe('"list" enumerates every lesson. "lesson" (default) loads one.'),
      track: z.enum(['easy', 'medium', 'hard']).optional().describe('Track. Default: easy.'),
      num: z.number().optional().describe('Lesson number within the track. Default: 1.'),
    },
  },
  async ({ command, track, num }) => {
    if (command === 'list') {
      const out: Record<string, any[]> = {}
      for (const t of TUTOR_TRACKS) {
        out[t] = listTutorLessons(t).map(l => ({ num: l.num, title: l.title }))
      }
      return textResult(out)
    }
    const t: TutorTrack = track ?? 'easy'
    const n = num ?? 1
    const lessons = listTutorLessons(t)
    const lesson = lessons.find(l => l.num === n)
    if (!lesson) {
      return textResult({
        error: `Lesson ${t}/${n} not found`,
        available: lessons.map(l => l.num),
      })
    }
    const content = readFileSync(lesson.path, 'utf-8')
    const parsed = parseTutorLesson(content)
    const check = parsed.expect
      ? await evalExpectPredicate(parsed.expect, tutorSystemCall)
      : { ok: null as any, detail: 'no expect predicate in this lesson' }
    const nextNum = lessons.find(l => l.num > n)?.num
    const nextInTrack = nextNum ? { track: t, num: nextNum } : null
    const nextTrackOrder: TutorTrack[] = ['easy', 'medium', 'hard']
    const nextTrack = !nextInTrack
      ? nextTrackOrder[nextTrackOrder.indexOf(t) + 1] ?? null
      : null
    const next = nextInTrack
      ? nextInTrack
      : nextTrack
        ? { track: nextTrack, num: 1 }
        : null
    return textResult({
      track: t,
      num: n,
      title: parsed.title,
      content,
      expect: parsed.expect,
      check,
      next,
    })
  },
)

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

// ── tutor.* mirror tools — sandbox-routed ──────────────────────────

server.registerTool(
  'tutor.list',
  {
    description: 'list:NOUN against the tutor sandbox (tutor/domains/). Use this instead of `list` when working through lessons.',
    inputSchema: { noun: z.string().describe('Entity noun, e.g. "Order".') },
  },
  async ({ noun }) => {
    const raw = await tutorSystemCall(`list:${noun}`, '')
    return textResult(parseEngineRaw(raw, []))
  },
)

server.registerTool(
  'tutor.get',
  {
    description: 'get:NOUN/ID against the tutor sandbox.',
    inputSchema: { noun: z.string(), id: z.string() },
  },
  async ({ noun, id }) => {
    const raw = await tutorSystemCall(`get:${noun}`, id)
    return textResult(parseEngineRaw(raw, null))
  },
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
  async ({ fact_type, filter }) => {
    const raw = await tutorSystemCall(`query:${fact_type}`, JSON.stringify(filter ?? {}))
    return textResult(parseEngineRaw(raw, []))
  },
)

server.registerTool(
  'tutor.authoring',
  {
    description: 'Project the CSDP schema-authorship workflow from tutor/domains/authoring.md. Returns readings-backed steps and current HATEOAS actions for an Authoring Session status.',
    inputSchema: {
      status: z.string().optional().describe('Current Authoring Session status. Defaults to the initial CSDP authoring status from the readings.'),
    },
  },
  async ({ status }) => textResult(await readTutorAuthoringWorkflow(tutorSystemCall, status)),
)

server.registerTool(
  'tutor.actions',
  {
    description: 'List legal SM transitions for a noun in the tutor sandbox. Pass status for pure workflow projection, or id for legacy entity-oriented calls.',
    inputSchema: {
      noun: z.string(),
      id: z.string().optional(),
      status: z.string().optional(),
    },
  },
  async ({ noun, id, status }) => {
    const current = status ?? id ?? ''
    const raw = await tutorSystemCall(`transitions:${noun}`, current)
    return textResult({ raw, parsed: normalizeTransitionRows(raw, noun, id ?? current) })
  },
)

server.registerTool(
  'tutor.apply',
  {
    description: 'Apply create/update/transition against the tutor sandbox. Same shape as `apply`. Mutations are scoped to the sandbox; the active app is untouched.',
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
    if (operation === 'create') {
      const idPair = id ? `<id, ${id}>${pairs ? ', ' : ''}` : ''
      const raw = await tutorSystemCall(`create:${noun}`, `<${idPair}${pairs}>`)
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    if (operation === 'update') {
      const raw = await tutorSystemCall(`update:${noun}`, `<<id, ${id || ''}>${pairs ? `, ${pairs}` : ''}>`)
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    const raw = await tutorSystemCall(`transition:${noun}`, `<${id || ''}, ${event || ''}>`)
    try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
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
    try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
  },
)

// ── Debug (gated) ────────────────────────────────────────────────────

if (AREST_DEBUG) {
  server.registerTool(
    'debug',
    { description: 'Dump full compiled state. Development only — AREST_DEBUG=1.' },
    async () => {
      if (AREST_MODE === 'local') {
        const raw = await systemCall('debug', '')
        try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
      }
      const data = await httpRequest('/debug')
      return textResult(data)
    },
  )
}

// ── Prompts — domain knowledge served on demand ─────────────────────

server.registerPrompt(
  'arest_overview',
  { description: 'AREST system overview, constraint types, and FORML2 document structure' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('overview') } }] }),
)

server.registerPrompt(
  'arest_entity_modeling',
  { description: 'Entity/value types, reference schemes, normalization, arity, multiplicity, objectification' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('entity-modeling') } }] }),
)

server.registerPrompt(
  'arest_advanced_constraints',
  { description: 'Subtype partitions, subset constraints with autofill, ring constraints' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('advanced-constraints') } }] }),
)

server.registerPrompt(
  'arest_derivation_deontic',
  { description: 'Derivation rules, deontic vs alethic modality, obligatory/forbidden/permitted operators' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('derivation-deontic') } }] }),
)

server.registerPrompt(
  'arest_verbalization',
  { description: 'Full ORM2 verbalization tables: UC, MC, DMaC, SSC, combined patterns from Halpin ORM2-02' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('verbalization') } }] }),
)

server.registerPrompt(
  'arest_principles',
  { description: 'Design principles: facts all the way down, no bridge architecture, the paper is the spec' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('design-principles') } }] }),
)

server.registerPrompt(
  'arest_api',
  { description: 'AREST API reference: CLI keys, MCP tools, HTTP endpoints, identity/signing' },
  () => ({ messages: [{ role: 'user', content: { type: 'text', text: loadPrompt('api') } }] }),
)

// ── Start ───────────────────────────────────────────────────────────

async function main() {
  const transport = new StdioServerTransport()
  await server.connect(transport)
  // eslint-disable-next-line no-console
  console.error(`AREST MCP server started — mode=${AREST_MODE}${AREST_MODE === 'remote' ? ` url=${AREST_URL}` : ` app=${activeApp.name}`}${AREST_DEBUG ? ' [DEBUG]' : ''}`)
  // #842: warn if AREST_CLI is older than crates/arest/src — agent
  // edited engine source but rebuilt the wrong artifact (or didn't
  // rebuild at all). Local mode only; remote/cloudflare uses HTTP/WASM.
  if (AREST_MODE === 'local') {
    const srcDir = resolve(REPO_ROOT, 'crates', 'arest', 'src')
    const stale = checkCliStaleness(AREST_CLI, srcDir)
    if (stale) console.error(`[arest-mcp warning] ${stale}`)
  }
}

main().catch((err) => {
  console.error('AREST MCP server failed:', err)
  process.exit(1)
})
