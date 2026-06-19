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
 *   AREST_PERSIST_ACTIVE_APP — local mode: when not 0/false/no/off (the
 *                            default), the app selected via apps.use is
 *                            written to <appsDir>/.arest-active-app and
 *                            resumed on the next startup, so an MCP
 *                            reconnect stays on the app you were last
 *                            using instead of snapping back to $AREST_APP.
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
import { readFileSync, writeFileSync, readdirSync, existsSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { fileURLToPath } from 'url'
import { spawn, execFileSync } from 'child_process'
import { connect as netConnect } from 'net'
import {
  buildAppCompileArgs,
  checkArestApps,
  createArestApp,
  defaultAppsDir,
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
  type MutationContextDetail,
  type MutationContextTool,
} from './mutation-context.js'
import { tutorSystemCall, resetSandbox, parseEngineRaw } from './tutor-sandbox.js'
import { resolveArestCli } from './cli-resolver.js'
import { checkCliStaleness } from './cli-staleness.js'
import { compareEngineVersion } from './engine-version.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(__dirname, '..', '..')

// ── Active-app persistence ──────────────────────────────────────────
//
// When AREST_PERSIST_ACTIVE_APP is on (default), apps.use writes the active
// app's name to `<appsDir>/.arest-active-app`, and startup resumes it — so an
// MCP reconnect stays on the app you were last using instead of snapping back
// to the env-inferred default ($AREST_APP / 'default'). Set the env var to
// 0/false/no/off to disable. The marker is a hidden file (never a directory),
// so listArestApps never mistakes it for an app.

const ACTIVE_APP_STATE_FILE = '.arest-active-app'

export function persistActiveAppEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  const v = (env.AREST_PERSIST_ACTIVE_APP ?? '').trim().toLowerCase()
  return !['0', 'false', 'no', 'off'].includes(v)
}

export function activeAppStateFile(appsDir: string): string {
  return join(appsDir, ACTIVE_APP_STATE_FILE)
}

/**
 * Startup app name. The persisted last-active app wins over the env-inferred
 * default WHEN persistence is enabled and the persisted name still resolves to
 * a real app — that is the whole point of "resume where I was", so it must
 * override $AREST_APP. Falls back to `inferInitialAppName` when persistence is
 * disabled, nothing is persisted, or the persisted app no longer exists.
 */
export function chooseInitialAppName(opts: {
  persistEnabled: boolean
  persistedName: string
  persistedExists: boolean
  env: NodeJS.ProcessEnv
}): string {
  if (opts.persistEnabled && opts.persistedName && opts.persistedExists) {
    return opts.persistedName
  }
  return inferInitialAppName(opts.env)
}

/**
 * Common gate for writing the `.arest-active-app` marker. Used by BOTH
 * the explicit `apps.use` path (activateApp) AND the startup-resolution
 * persist (task-959 fix #1) so a reconnect resumes the actually-active
 * app regardless of how it was resolved.
 *
 * Returns true iff persistence is enabled, an apps workspace exists,
 * and the resolved app actually exists on disk -- we never promote a
 * non-existent fallback over a valid earlier marker.
 */
export function shouldPersistResolvedApp(opts: {
  persistEnabled: boolean
  appsDir: string
  appExists: boolean
}): boolean {
  return Boolean(opts.persistEnabled && opts.appsDir && opts.appExists)
}

function readPersistedAppName(appsDir: string): string {
  try {
    return readFileSync(activeAppStateFile(appsDir), 'utf8').trim()
  } catch {
    return ''
  }
}

function writePersistedAppName(appsDir: string, name: string): void {
  try {
    writeFileSync(activeAppStateFile(appsDir), `${name}\n`, 'utf8')
  } catch {
    // Best-effort: a read-only apps dir must not break app switching.
  }
}

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
const PERSIST_ACTIVE_APP = persistActiveAppEnabled(process.env)
// Resolved apps workspace where the `.arest-active-app` marker lives (local
// mode only; defaultAppsDir honors $AREST_APPS_DIR, else <repo>/../apps).
const APPS_DIR = AREST_MODE === 'local' ? defaultAppsDir(REPO_ROOT) : ''
const PERSISTED_APP_NAME = (PERSIST_ACTIVE_APP && APPS_DIR) ? readPersistedAppName(APPS_DIR) : ''
const INITIAL_APP_NAME = chooseInitialAppName({
  persistEnabled: Boolean(PERSIST_ACTIVE_APP && APPS_DIR),
  persistedName: PERSISTED_APP_NAME,
  persistedExists: PERSISTED_APP_NAME
    ? resolveArestApp(PERSISTED_APP_NAME, { appsDir: APPS_DIR, cwd: REPO_ROOT }).exists
    : false,
  env: process.env,
})
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

// task-959 fix #1: persist the resolved initial app so a future
// reconnect deterministically resumes here -- the apps.use path
// (activateApp below) writes the marker on every explicit switch,
// but startup resolutions that came from $AREST_APP via
// inferInitialAppName used to leave the marker stale or empty, so a
// reconnect with a stale marker fell back to $AREST_APP again
// instead of the app the session was actually using. Mirrors the
// activateApp gate via shouldPersistResolvedApp.
if (shouldPersistResolvedApp({
  persistEnabled: PERSIST_ACTIVE_APP,
  appsDir: APPS_DIR,
  appExists: activeApp.exists,
})) {
  writePersistedAppName(APPS_DIR, activeApp.name)
}

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
  // Remember the most-recently-activated app so a restart resumes it.
  if (shouldPersistResolvedApp({
    persistEnabled: PERSIST_ACTIVE_APP,
    appsDir: APPS_DIR,
    appExists: activeApp.exists,
  })) {
    writePersistedAppName(APPS_DIR, activeApp.name)
  }
  return activeApp
}

// ── Per-call app scoping (p0 mcp-active-app-isolation, option b) ──────
//
// Sub-agents share ONE stdio connection with no per-session id, so the
// server can't isolate per session — a global `activeApp` (plus the
// `.arest-active-app` marker) means one agent's `apps.use` silently
// re-scopes another agent's reads/writes. Option (b) lets a single CALL
// carry an optional `app` that resolves its OWN db + readings + engine
// handle for THAT call only.
//
// Invariants (the bug-fix contract):
//   1. resolveCallScope is PURE — it delegates to resolveArestApp (a
//      pure fs read) and never assigns `activeApp`, calls activateApp,
//      or writes the marker via writePersistedAppName. A per-call `app`
//      therefore NEVER mutates the shared global or the on-disk marker.
//   2. When `app` is omitted (scope === undefined) every helper falls
//      back to the global path verbatim, so existing calls are byte-for-
//      byte unchanged (additive + backward-compatible).
//   3. The per-call engine handle lives in a SEPARATE keyed cache
//      (`_perCallHandles`) so a second app never clobbers the global
//      single-slot `_localHandle`.

export interface CallScope {
  name: string
  dbPath: string
  readingsDir: string
  exists: boolean
}

/**
 * Resolve an optional per-call `app` override into a CallScope WITHOUT
 * touching the global `activeApp` or the `.arest-active-app` marker.
 * Pure: the only fs access is via resolveArestApp (read-only path
 * discovery). Defaults `options` to the server's live registry options
 * so a verb callback can call `resolveCallScope(app)` directly; tests
 * pass an explicit `{ appsDir, cwd }` fixture so no live state is read.
 */
export function resolveCallScope(
  app: string,
  options: Parameters<typeof resolveArestApp>[1] = appRegistryOptions(),
): CallScope {
  const resolved = resolveArestApp(app, options)
  return {
    name: resolved.name,
    dbPath: resolved.dbPath,
    readingsDir: resolved.readingsDir,
    exists: resolved.exists,
  }
}

/**
 * Db path for a call: the per-call scope's db when scoped, else the
 * supplied global fallback (today's `currentDbPath()` value). Keeping
 * the fallback as an argument makes the helper pure + testable.
 */
export function scopeDbPath(scope: CallScope | undefined, fallback: string): string {
  return scope ? scope.dbPath : fallback
}

/**
 * Readings dir for a call: the per-call scope's readings when scoped,
 * else the supplied global fallback (today's `currentReadingsDir()`).
 */
export function scopeReadingsDir(scope: CallScope | undefined, fallback: string): string {
  return scope ? scope.readingsDir : fallback
}

// Per-call engine handles, keyed by readings signature. The global app
// keeps using the single-slot `_localHandle` fast path (untouched); a
// per-call `app` gets/keeps its own compiled handle here so two apps
// can coexist in one process without invalidating each other.
const _perCallHandles = new Map<string, number>()

/** Cold-cache miss ⇒ undefined; warm hit ⇒ the stored handle. Pure. */
export function lookupHandleCache(
  cache: Map<string, number>,
  signature: string,
): number | undefined {
  return cache.get(signature)
}

/**
 * Store (or replace) the handle for a readings signature. Replacing on
 * the same signature means a recompile after a readings edit reuses the
 * slot instead of leaking a second entry. Pure (mutates the passed map
 * only).
 */
export function rememberHandleCache(
  cache: Map<string, number>,
  signature: string,
  handle: number,
): void {
  cache.set(signature, handle)
}

function currentReadingsDir(scope?: CallScope): string {
  return scopeReadingsDir(scope, activeApp.readingsDir || AREST_READINGS_DIR)
}

function currentDbPath(scope?: CallScope): string {
  return scopeDbPath(scope, activeApp.dbPath || AREST_DB)
}

function shouldUseCliDb(scope?: CallScope): boolean {
  return AREST_MODE === 'local' && APP_MODE_ENABLED && Boolean(currentDbPath(scope))
}

/**
 * Verb-callback convenience: resolve the optional `app` argument into a
 * CallScope, or `undefined` when `app` is omitted/empty (⇒ use the
 * global active app, unchanged behavior). Resolution is via the pure
 * resolveCallScope, so a per-call `app` never mutates `activeApp` or
 * the marker.
 */
function callScope(app?: string): CallScope | undefined {
  const trimmed = (app ?? '').trim()
  return trimmed ? resolveCallScope(trimmed) : undefined
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
      reason: `set '${app.name}' as the default for calls that omit \`app\` — or pass app="${app.name}" on a single verb to route just that call without changing the default (multi-agent-safe)`,
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

async function getLocalHandle(scope?: CallScope): Promise<number> {
  const readingsDir = currentReadingsDir(scope)
  const signature = readingsSignature(readingsDir)
  // Per-call (`app` supplied): use the keyed cache so a second app gets
  // its own handle instead of clobbering the global single-slot handle.
  if (scope) {
    const cached = lookupHandleCache(_perCallHandles, signature)
    if (cached !== undefined && cached >= 0) return cached
    const engine = await getLocalEngine()
    const readings = loadReadingsFromDir(readingsDir)
    const handle = engine.compileDomainReadings(...readings)
    rememberHandleCache(_perCallHandles, signature, handle)
    return handle
  }
  // Global (no `app`): unchanged fast path.
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

async function dispatchCommand(command: any, scope?: CallScope): Promise<any> {
  if (shouldUseCliDb(scope)) {
    return cliApplyCommand(command, scope)
  }
  if (AREST_MODE === 'local') {
    const engine = await getLocalEngine()
    const handle = await getLocalHandle(scope)
    const raw = engine.system(handle, 'apply', JSON.stringify(command))
    try { return JSON.parse(raw) } catch { return { rejected: true, error: raw } }
  }
  // Remote: POST to /arest/:domain/:noun or /arest/:domain/apply
  return httpRequest(`/arest/${command.domain || 'default'}/apply`, {
    method: 'POST',
    body: JSON.stringify(command),
  })
}

async function dispatchRead(path: string, scope?: CallScope): Promise<any> {
  if (AREST_MODE === 'local') {
    const raw = await systemCall('debug', '', scope)
    try { return JSON.parse(raw) } catch { return { raw } }
  }
  return httpRequest(path)
}

// ── Local system call helper ──────────────────────────────────────

async function systemCall(key: string, input: string, scope?: CallScope): Promise<string> {
  if (shouldUseCliDb(scope)) return cliSystemCall(key, input, scope)
  const engine = await getLocalEngine()
  const handle = await getLocalHandle(scope)
  return engine.system(handle, key, input)
}

function runArestCli(args: string[], stdinPayload?: string): Promise<string> {
  return runArestCliCapture(args, stdinPayload).then(r => r.stdout)
}

/**
 * mcp-surface-compile-diagnostics (task 986 / arc issue 2): the CLI
 * emits compile diagnostics — layer-7 check warnings, model warnings,
 * projection warn-skips — on STDERR, and the success path used to
 * discard them entirely (debug-log only), so `apps.compile` returned
 * `{ok:true, raw:""}` even when the engine warned loudly. This variant
 * returns stderr alongside stdout so callers can surface it.
 */
function runArestCliCapture(
  args: string[],
  stdinPayload?: string,
): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(AREST_CLI, args, {
      cwd: REPO_ROOT,
      env: process.env,
      windowsHide: true,
    })
    // Always close stdin: the CLI only reads it under `--stdin-input`,
    // and an open pipe would hang that read at EOF-never.
    if (stdinPayload !== undefined) {
      child.stdin.write(stdinPayload, 'utf8')
    }
    child.stdin.end()
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
        resolvePromise({ stdout: stdout.trim(), stderr })
      } else {
        reject(new Error(stderr.trim() || `arest-cli exited with code ${code}`))
      }
    })
  })
}

/**
 * mcp-apply-stdin-payload (arc-agi-3 issue 4): Windows caps a spawned
 * command line at ~32 KB TOTAL, so passing the SYSTEM input on argv
 * capped `apply` batches at ~50 ops (task-930 advertises 4096) and
 * spawn ENAMETOOLONG'd beyond it — chunked workarounds forfeited batch
 * atomicity. Above the threshold the payload rides STDIN and argv
 * carries `--stdin-input` instead (arest-cli reads stdin to EOF).
 * Small payloads keep the argv path: zero behavior change for the
 * common case, and compatibility with older binaries everywhere the
 * old path would have worked at all.
 */
export const STDIN_INPUT_THRESHOLD_BYTES = 8192

export function cliCallPlan(
  dbPath: string,
  key: string,
  input: string,
): { args: string[]; stdin?: string } {
  if (Buffer.byteLength(input, 'utf8') <= STDIN_INPUT_THRESHOLD_BYTES) {
    return { args: ['--db', dbPath, key, input] }
  }
  return { args: ['--db', dbPath, key, '--stdin-input'], stdin: input }
}

function cliSystemCall(key: string, input: string, scope?: CallScope): Promise<string> {
  const db = currentDbPath(scope)
  const plan = cliCallPlan(db, key, input)
  return tryWarmCall(db, key, input).then(warm =>
    warm !== null ? warm : runArestCli(plan.args, plan.stdin))
}

/**
 * load-state-cache-or-warm-engine LEVER B (warm engine v1): when an
 * `arest-cli serve` process advertises itself via the `<db>.warm`
 * port file, route the verb over TCP to the RESIDENT engine instead
 * of spawning a per-call process (which pays a full state decode —
 * 13-25s at arc scale — before any work). Falls back to the spawn
 * path on ANY failure: missing/stale port file, refused connection,
 * timeout, empty response — zero-config compat, nothing breaks when
 * the warm process is absent. Binary-staleness is enforced
 * SERVER-side (the serve loop self-exits when the on-disk exe or db
 * changes), so a live socket implies a current engine.
 *
 * The connect timeout is disabled once connected: long verbs (an
 * apply runs its full derive→validate→persist pipeline) send nothing
 * until done, and an idle timeout would kill them mid-op.
 */
function tryWarmCall(dbPath: string, key: string, input: string): Promise<string | null> {
  try {
    const warmFile = `${dbPath}.warm`
    if (!existsSync(warmFile)) return Promise.resolve(null)
    const port = parseInt(readFileSync(warmFile, 'utf8').split('\n')[0] ?? '', 10)
    if (!port || Number.isNaN(port)) return Promise.resolve(null)
    return new Promise<string | null>(resolveWarm => {
      const sock = netConnect({ host: '127.0.0.1', port, timeout: 2000 }, () => {
        sock.setTimeout(0)
        sock.write(JSON.stringify({ key, input }) + '\n')
      })
      let buf = ''
      sock.setEncoding('utf8')
      sock.on('data', chunk => { buf += chunk })
      sock.on('end', () => resolveWarm(buf.trim().length > 0 ? buf.trim() : null))
      sock.on('error', () => resolveWarm(null))
      sock.on('timeout', () => { sock.destroy(); resolveWarm(null) })
    })
  } catch {
    return Promise.resolve(null)
  }
}

// Read the repo's current HEAD SHA for the engine_version staleness
// check. Synchronous + bounded: this is a one-shot diagnostic, not a
// hot path. Returns "unknown" when git is unavailable or the directory
// isn't a repo, so compareEngineVersion can report "indeterminate"
// rather than throw.
function readHeadSha(): string {
  try {
    return execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd: REPO_ROOT,
      encoding: 'utf8',
      windowsHide: true,
    }).trim()
  } catch {
    return 'unknown'
  }
}

function compileAppReadings(app: ArestApp): Promise<{ stdout: string; stderr: string }> {
  return runArestCliCapture(buildAppCompileArgs(app, appRegistryOptions()))
}

/**
 * mcp-surface-compile-diagnostics (task 986 / arc issue 2): pull the
 * diagnostic lines out of the CLI's stderr so the caller sees what the
 * engine warned about — layer-7 check warnings (`[check] ...`), model
 * warnings (`[model warning] ...`), DDL/trigger/projection warn-skips
 * (`Warning: ...`). Capped: projection skips alone can run to
 * thousands of lines on a dirty population; the caller gets the first
 * DIAGNOSTICS_CAP plus the true total.
 */
const DIAGNOSTICS_CAP = 100
function extractDiagnostics(stderr: string): { lines: string[]; total: number } {
  const lines = stderr
    .split(/\r?\n/)
    .filter(l => /warning|violation|\[check\]|\[model/i.test(l))
    .map(l => l.trim())
  return { lines: lines.slice(0, DIAGNOSTICS_CAP), total: lines.length }
}

function compileResult(raw: string, stderr?: string) {
  let parsed: unknown
  try { parsed = JSON.parse(raw) } catch {}
  const rejected = raw.trim().startsWith('⊥')
  const diag = stderr !== undefined ? extractDiagnostics(stderr) : undefined
  return {
    ok: !rejected,
    rejected,
    bytes: raw.length,
    raw,
    ...(parsed !== undefined ? { parsed } : {}),
    ...(diag && diag.total > 0
      ? { diagnostics: diag.lines, diagnostics_total: diag.total }
      : {}),
  }
}

function parseJsonResult(raw: string): any {
  try { return JSON.parse(raw) } catch { return { raw } }
}

// Mirrors `escape_atom_for_display` in crates/arest/src/ast.rs.
function escapeAtom(s: string): string {
  return s.replace(/[\\<>,]/g, ch => '\\' + ch)
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

// task-930: translate one collection member into the engine `Command`
// JSON shape `platform_apply_command` deserializes. createEntity /
// updateEntity / transition mirror the remote-mode encoding below; the
// batch wraps a Vec of these. `sender`/`signature` ride on each member
// so per-op identity/auth still flows (the engine reads them per op).
export function buildApplyCommandForBatch(
  op: {
    operation: 'create' | 'update' | 'transition' | 'assertFact'
    noun?: string
    id?: string
    fields?: Record<string, string>
    event?: string
    fact_type?: string
    pairs?: Array<{ role: string; value: string }>
  },
  ctx: { sender?: string; signature?: string },
): any {
  const { sender, signature } = ctx
  switch (op.operation) {
    case 'create':
      return { type: 'createEntity', noun: op.noun, domain: '', id: op.id, fields: op.fields || {}, sender, signature }
    case 'update':
      return { type: 'updateEntity', noun: op.noun, domain: '', entityId: op.id, fields: op.fields || {}, sender, signature }
    case 'transition':
      return { type: 'transition', noun: op.noun, entityId: op.id, event: op.event, domain: '', sender, signature }
    // apply-pairs-arbitrary-cells: exact-tuple assertion as a batch
    // member — same Command::AssertFact the flat fact_type+pairs path
    // dispatches (serde camelCase factType), so n-ary / same-signature
    // / multi-row facts ride the atomic batch alongside entity ops.
    case 'assertFact':
      return { type: 'assertFact', factType: op.fact_type, pairs: op.pairs || [], sender, signature }
  }
}

async function cliApplyCommand(command: any, scope?: CallScope): Promise<any> {
  let key = ''
  let input = ''
  switch (command?.type) {
    // task-930: a batch is applied atomically by the engine's `apply`
    // verb (→ platform_apply_command → Command::Batch). We must NOT
    // decompose it into per-op CLI calls — that would run N independent
    // applies and lose atomicity. Forward the whole batch JSON as one
    // `apply` system call.
    case 'batch': {
      const raw = await cliSystemCall('apply', JSON.stringify(command), scope)
      try { return JSON.parse(raw) } catch { return { raw } }
    }
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
  const raw = await cliSystemCall(key, input, scope)
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
  scope?: CallScope,
): Promise<string | null> {
  if (AREST_MODE !== 'local') return null
  if (!result.citation) return null
  try {
    const payload = buildIngestPayload(result)
    const citeId = await systemCall(
      `federated_ingest:${noun}`,
      JSON.stringify(payload),
      scope,
    )
    return citeId && citeId !== '⊥' ? citeId : null
  } catch {
    return null
  }
}

/** Check if a noun has a populate def and return its config. */
async function getFederationConfig(noun: string, scope?: CallScope): Promise<FederationConfig | null> {
  if (AREST_MODE !== 'local') return null
  try {
    const raw = await systemCall(`populate:${noun}`, '', scope)
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

// Shared description for the optional per-call `app` override (p0
// mcp-active-app-isolation, option b). Threaded into the READ/WRITE
// verbs that otherwise resolve their DB/readings through the shared
// global active app. Mirrors the ergonomics of `orient`'s active_app
// arg, but actually re-scopes the engine handle for THAT call only.
const APP_OVERRIDE_FIELD_DESCRIPTION =
  'Optional per-call app override (p0 isolation). When set, THIS call resolves ' +
  'its own readings + DB + engine handle for that app only — without changing the ' +
  'session\'s active app (no apps.use side effect, no .arest-active-app marker write). ' +
  'Use it when sub-agents share one MCP connection and must not clobber each other\'s ' +
  'active app. Omit to use the current active app (apps.use remains the ergonomic ' +
  'default for single-app sessions). RECEIPT NOTE (context-receipt-override-scope, ' +
  'BY DESIGN): mutating calls with an app override ride the SESSION receipt — the ' +
  'context_receipt validates against the session\'s active app, and the override ' +
  'does not invalidate or re-scope it (only apps.use does). The receipt attests ' +
  '"this agent read the modeling rules", which are app-agnostic; the override is a ' +
  'routing convenience, not a scope escalation.'

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
    description:
      'Load AREST modeling rules + prompt manifest and mint a context_receipt token. WHEN: call FIRST in any session that will mutate state (apply / retract / compile / propose) — those verbs refuse to run without a fresh receipt. Also useful as a cheap "what does AREST consider good practice?" reference. ALTERNATIVE: orient for a one-screen "where are we" snapshot (apps + recent activity, no rules); schema for the formal model surface. GOTCHA: the receipt is scoped to the currently active app — `apps.use` invalidates the prior receipt, so re-call context after switching apps. Per-call `app:` OVERRIDES are different (by design): they ride the session receipt without re-scoping it — the receipt attests the agent read the (app-agnostic) modeling rules, so an override mutation does not need a receipt minted under the override app. detail=summary returns rule text + prompt digests (cheap); detail=full also inlines prompt bodies (larger). NEXT: read the returned rules / anti_patterns / how_to, then call apply / compile / propose with context_receipt set to the receipt field of this response.',
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
    description:
      'Show the DEFAULT app (readings dir, DB path, health) — the app used when a call omits `app`. WHEN: you need a quick "what is the default scope right now?" answer mid-session. ALTERNATIVE: orient when you also want recent activity + sibling apps in one envelope; apps.status for full health of a specific (possibly non-default) app; apps.list for every app. GOTCHA: this reports the default only — individual calls can still route elsewhere by passing `app=<name>`, which does NOT change this default. NEXT: apps.use name=… to change the default, or pass app=<name> on a single verb to route just that call.',
    inputSchema: {
      detail: z.enum(['summary', 'full']).optional().describe('summary returns compact health. full includes reading file details.'),
    },
  },
  async ({ detail }) => textResult({ active_app: appSummary(activeApp, (detail ?? 'summary') as AppDetail) }),
)

server.registerTool(
  'apps.list',
  {
    description:
      'Enumerate every AREST app under $AREST_APPS_DIR with compact health. WHEN: you want a roster of every UoD the agent can switch into — picking an app to work on, or sweeping for stale DBs. ALTERNATIVE: apps.current for just the active app (cheaper); orient for the same roster PLUS active-app task counts + recent activity (one call instead of two); apps.check when you also want a summary section (counts of ready / stale / library / not-ready) over the same roster. GOTCHA: directory-derived — apps are discovered from disk, not from a catalog fact, so a missing readings/ subdir hides the app. include_ready=false trims ready apps for triage. NEXT: apps.use name=<picked> to switch scope, then context to mint a fresh receipt for that app.',
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
    description:
      'Deep health report for ONE app — reading-file inventory, DB mtime vs readings, dependency closure, next-action suggestions. Defaults to the active app. WHEN: diagnosing a single app — "is the DB stale?", "which readings does this app depend on?", "why is health=not_ready?". ALTERNATIVE: apps.check when you want the same depth across EVERY app in the registry; apps.current for a one-liner without the dependency closure; apps.list when you want a flat roster only. GOTCHA: detail=full is the default here (not summary like apps.list / apps.current) because the verb is meant for deep inspection. NEXT: apps.compile to refresh DB from readings if stale=true; apps.use if you want to make this app active first.',
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
    description:
      'Sweep EVERY app in the registry and roll up a registry-wide summary (counts of ready / stale / library / not-found). WHEN: registry-wide triage — "which apps are stale across the whole repo?", "how many apps need a recompile?". ALTERNATIVE: orient when you only need active-app counts plus one-line "what to do next" pointer (cheaper); apps.list for the same per-app roster WITHOUT the rolled-up summary; apps.status for deep inspection of ONE app. GOTCHA: still directory-derived — apps without a readings/ subdir show as not_found even if the name appears in env. NEXT: apps.use to switch to whichever app needs work, then apps.compile if its DB is stale.',
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
    description:
      'Set the process DEFAULT app — the app used by calls that OMIT `app`. WHEN: you are working single-app for a while and want to stop repeating `app=` on every call. PREFER passing `app=<name>` per call instead: that routes a single call statelessly and is the multi-agent-safe default (two agents sharing this server never clobber each other). ALTERNATIVE: pass `app` on the individual verb (get / query / sql / cells / apply / orient / …) to route ONE call without changing the shared default; apps.create when the app does not yet exist; apps.status to peek at an app without making it the default. GOTCHA: this changes a process-wide default shared by every call that omits `app`, and it INVALIDATES any context_receipt minted under the prior default — mutating verbs reject stale receipts after a switch, so call context again. Library entries (no readings/ + no .db) refuse activation with error="app_is_library". NEXT: context to mint a receipt for the new default, then orient (or apps.current) to confirm.',
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
        const { stdout, stderr } = await compileAppReadings(app)
        result = compileResult(stdout, stderr)
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
    description:
      'Re-compile an app\'s readings/*.md INTO its SQLite .db (full refresh; readings are the source of truth). WHEN: apps.status reports stale=true, or you have just edited readings/ for an app and need the engine to see them. ALTERNATIVE: compile (no `apps.` prefix) when you want to ADD readings to the LIVE active engine without rebuilding the DB on disk (Corollary 5 — in-process self-modification, no file write); use apps.compile when you want the DB on disk to reflect the readings. GOTCHA: this refreshes the SCHEMA from readings while PRESERVING live population — per Closure Under Self-Modification (cor:closure), the load reads the existing DB and merges the freshly-parsed schema over the prior population, so apply-created entity facts survive (FT cells like Task_has_Task_Subject and SM cells like State_Machine_is_currently_in_Status are carried forward; identity-aware merge_states dedupes overlaps). Only readings-derived schema cells (Noun, FactType, Constraint, derivation rules) and internal derived cells (names containing a colon) are regenerated — live apply facts are NOT wiped (see cli/entry.rs run_load: "preserving N user-population cells from existing DB"). The verb refuses on library entries and on apps with zero .md files. NEXT: apps.status to confirm stale=false, then apps.use (if activate=false was set) and orient.',
    inputSchema: {
      name: z.string().optional().describe('AREST app name. Defaults to the active app. Compiling by `name` activates it by default (set activate=false to keep the current active app).'),
      app: z.string().optional().describe('Per-call app override (multi-app scoping): compile THIS app WITHOUT changing the session active app — the same `app=` convention query/apply/retract use. Takes precedence over `name`, and never activates.'),
      activate: z.boolean().optional().describe('Make the compiled app active. Default true when `name` is given; always false when the `app=` override is used.'),
    },
  },
  async ({ name, app, activate }) => {
    if (AREST_MODE !== 'local') return textResult({ error: 'apps.compile requires local mode' })

    // `app=` is the multi-app per-call override (compile a non-active app without
    // switching); `name` is the legacy activating form. `app` wins when both given.
    const targetName = app ?? name
    const target = targetName ? resolveArestApp(targetName, appRegistryOptions()) : activeApp
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
    const { stdout, stderr } = await compileAppReadings(target)
    const refreshed = resolveArestApp(target.name, appRegistryOptions())
    // The `app=` override never activates (multi-app: leave the session active app
    // untouched); `name` activates by default unless activate===false; a bare call
    // activates only if activate===true.
    const shouldActivate = app ? false : (name ? activate !== false : activate === true)
    if (shouldActivate) activateApp(refreshed.name)

    return textResult({
      app: appSummary(refreshed, 'full'),
      compile_result: compileResult(stdout, stderr),
      active_app: appSummary(activeApp, 'summary'),
      context: refreshed.name === activeApp.name ? currentMutationContext() : undefined,
    })
  },
)

// ── 1. get: retrieve an entity or list entities ──────────────────────

server.registerTool(
  'get',
  {
    description:
      'Fetch a 3NF view of ONE entity by id, OR list every entity of a noun (omit id). Returns fields + HATEOAS links; a single-id get additionally carries `transitions` (the legal SM events) and the engine-projected `view` (ui-readings elements + per-target representations) when the engine surfaces them. WHEN: you already know "give me Order ord-1 with all its current single-valued facts" — get assembles the per-entity row across every fact type the noun participates in. Listing (no id) returns one row per entity instance of the noun. ALTERNATIVE: query when you want rows of ONE fact type filtered by role binding (e.g. "every Task with Priority p0"); sql when you need joins / aggregates / NOT EXISTS across multiple FTs; actions when you specifically want the legal SM transitions for one entity. GOTCHA: federation-aware — if the noun is bound to an external system, get fetches from there and absorbs the result into the local population with a Citation. Multi-valued facts come back as arrays; single-valued facts come back as scalar strings. NEXT: actions noun=<N> id=<X> to see what transitions are legal, or apply operation=update to modify.',
    inputSchema: {
      id: z.string().optional().describe('Entity ID. If omitted, lists all entities of the noun type.'),
      noun: z.string().optional().describe('Noun type (e.g. "Order"). Required when listing, optional when getting by ID (inferred from population).'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ id, noun, app }) => {
    if (!noun) return textResult({ error: 'Provide noun to get or list.' })
    const scope = callScope(app)
    const scopeName = scope ? scope.name : activeApp.name

    // Check if this noun is backed by an external system (data federation).
    const fedConfig = await getFederationConfig(noun, scope)
    if (fedConfig) {
      const data = await federatedFetch(fedConfig, id || undefined)
      // Absorb fetched facts + Citation into P so downstream constraints
      // and derivations over the unified population see the federated
      // data. Errors are non-fatal — the fetched result is still
      // returned to the caller either way.
      const citeId = await absorbFederatedIntoD(noun, data, scope)
      if (citeId) {
        return textResult(enrichResponseWithCitation(data, citeId, '/arest/default'))
      }
      return textResult(data)
    }

    // Local population
    if (AREST_MODE === 'local') {
      if (id) {
        // mcp-get-surface-view-representations: route the single-id read
        // through the engine's FULL read path — Command::GetEntity
        // dispatched through the `apply` def, the same calling
        // convention the apply tool's batch path uses — so the
        // Theorem-4 representation rides WITH the row: HATEOAS
        // `transitions` plus the ui-readings `view` layer (elements +
        // per-target representations). The compiled get:{noun} def can
        // never carry these — it returns ONLY the flattened 3NF row.
        // parseGetEntityResponse re-flattens the CommandResult to that
        // exact row shape (additive enrichment only) and returns null
        // whenever the command read misses (older binary, ⊥, rejected,
        // unknown entity) — then we fall through to the legacy path
        // unchanged, so the task-959 wrong-UoD diagnostic still fires.
        try {
          const envelope = JSON.stringify({
            command: { type: 'getEntity', noun, entityId: id },
            population: '',
          })
          const rawCommand = await systemCall('apply', envelope, scope)
          const enriched = parseGetEntityResponse(rawCommand)
          if (enriched !== null) return textResult(enriched)
        } catch {
          // getEntity dispatch failure is non-fatal — the legacy
          // get:{noun} path below answers exactly as before.
        }
        const raw = await systemCall(`get:${noun}`, id, scope)
        return textResult(parseGetResponse(raw, noun, scopeName))
      }
      const raw = await systemCall(`list:${noun}`, '', scope)
      return textResult(parseGetResponse(raw, noun, scopeName))
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
 *
 * query-bottom-origin-envelope: the engine's ⊥-trace decorates Bottom
 * with an origin frame ("⊥ origin: in rule `query:<ft>`"), so an
 * exact `raw === '⊥'` match leaked the raw envelope for unknown FTs
 * (arc-agi-3 issue 6). Bottom is recognized by PREFIX — any ⊥-leading
 * response is "no def by that name", origin trace or not.
 */
export function isBottomRaw(raw: string): boolean {
  return raw.trim().startsWith('⊥')
}

export function parseQueryResponse(raw: string): unknown {
  if (isBottomRaw(raw)) return []
  try {
    const parsed = JSON.parse(raw)
    return parsed ?? []
  } catch {
    return { raw }
  }
}

/**
 * Translate the engine's raw response to the `get` / `list` shape.
 *
 * task-959 fix #3: when the engine returns bare `⊥` for get/list, the
 * single most likely cause is the requested noun isn't declared in the
 * active app's UoD -- a get:<Noun> def the engine doesn't have. Until
 * this fix, the MCP runtime returned `{ raw: '⊥' }`, which looks like
 * data loss when the data is fine and just lives in a different app.
 *
 * The envelope keeps the engine's raw ⊥ string (origin trace and all,
 * per query-bottom-origin-envelope — Bottom is matched by PREFIX) so
 * existing tooling that matched against it still sees it, but adds an
 * `error` + `hint` that names the active app and points at apps_list /
 * apps_use. The warning is intentionally framed as "possible cause" --
 * a ⊥ can also be an engine internal -- so this never falsely asserts.
 */
export function parseGetResponse(raw: string, noun: string, activeAppName: string): unknown {
  if (isBottomRaw(raw)) {
    return {
      error: `Bottom: get/list for '${noun}' returned ⊥ in active app '${activeAppName}'`,
      hint: `'${noun}' is most likely not declared in '${activeAppName}'. Try \`apps_list\` to see other apps and \`apps_use <name>\` to switch UoDs -- the entity may be in a different app.`,
      raw,
    }
  }
  try {
    return JSON.parse(raw)
  } catch {
    return { raw }
  }
}

/**
 * Translate the engine's `getEntity` CommandResult — the FULL Theorem-4
 * read path, dispatched through the `apply` def — into the same
 * flattened 3NF row `get:{noun}` returns today, enriched with the
 * HATEOAS + view layers (mcp-get-surface-view-representations).
 *
 * Both paths bottom out in the engine's `get_noun:{noun}` platform
 * primitive (compile.rs binds `get:{noun}` to exactly that Platform
 * func; command.rs's get_entity_via_defs calls it directly), so
 * `{...data, id}` reconstructs the legacy row field-for-field —
 * existing consumers see an identical base shape. On top of that:
 *
 *   - `transitions` — the legal SM events (serde camelCase
 *     TransitionAction: {event, targetStatus, method, href,
 *     componentRole?}) — attached only when non-empty, so SM-less
 *     nouns keep today's exact shape (serde always emits the array).
 *   - `view` — the ui-readings projection ({view, kind, source,
 *     elements:[{id, factType, componentRole}], representations:
 *     {<target>: "<html>"}}) — passed through VERBATIM when present
 *     (serde skip_serializing_if omits it otherwise).
 *
 * Returns null when the caller must FALL BACK to the legacy get:{noun}
 * path: non-JSON / ⊥ output (e.g. an older binary without the
 * getEntity command), rejected:true, or a missing/empty entities list
 * (unknown id — the read path is not alethic, and the legacy path's
 * task-959 wrong-UoD envelope owns that diagnostic).
 */
export function parseGetEntityResponse(raw: string): Record<string, unknown> | null {
  let parsed: unknown
  try { parsed = JSON.parse(raw) } catch { return null }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
  const result = parsed as {
    entities?: unknown
    rejected?: unknown
    transitions?: unknown
    view?: unknown
  }
  if (result.rejected === true) return null
  if (!Array.isArray(result.entities) || result.entities.length === 0) return null
  const entity = result.entities[0] as { id?: unknown; data?: unknown }
  if (!entity || typeof entity.id !== 'string' || entity.id === '') return null
  const data = entity.data && typeof entity.data === 'object' && !Array.isArray(entity.data)
    ? entity.data as Record<string, unknown>
    : {}
  // Same row shape as the legacy path: data fields + id. The engine
  // already excludes 'id' from data, so the spread cannot clobber it —
  // and putting id last keeps the entity id authoritative if it ever did.
  const flattened: Record<string, unknown> = { ...data, id: entity.id }
  if (Array.isArray(result.transitions) && result.transitions.length > 0) {
    flattened.transitions = result.transitions
  }
  if (result.view !== undefined && result.view !== null) {
    flattened.view = result.view
  }
  return flattened
}

server.registerTool(
  'query',
  {
    description:
      'Read-only single-fact-type filtered projection. WHEN: you want every row of ONE fact type, optionally filtered by exact role bindings (e.g. fact_type="Task_has_Task_Priority", filter={"Task Priority":"p0"}). ALTERNATIVE: sql for joins / aggregates / NOT EXISTS / GROUP BY across multiple FTs; get for the 3NF per-entity view by id; cells mode=get when you want the cell contents directly (same data, different framing); orient as the cheaper "session re-entry" call. GOTCHA: returns [] (not an error) for unknown fact_type — engine returns Object::Bottom (#821) and the MCP layer translates it to "no such facts", indistinguishable from "FT exists but is empty". Verify the FT name with `cells mode=list pattern=<glob>` if you get an unexpected empty list. Filter values are compared as strings (no type coercion). NEXT: cross-reference each row with `get noun=<role-noun> id=<row-value>` to fetch the entity view, or feed the result into a downstream apply.',
    inputSchema: {
      fact_type: z.string().describe('Fact type ID (e.g. "Order_was_placed_by_Customer", "Case_has_Observation")'),
      filter: z.record(z.string(), z.string()).optional().describe('Filter by role bindings (e.g. {"Case": "The Speckled Band"})'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ fact_type, filter, app }) => {
    const scope = callScope(app)
    if (AREST_MODE === 'local') {
      const filterStr = filter ? JSON.stringify(filter) : ''
      const raw = await systemCall(`query:${fact_type}`, filterStr, scope)
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
// Cells ARE relations (RMAP / whitepaper §3) — and the relations the
// verb exposes are the 3NF schema RMAP derives (rmap-3nf-tables
// Stage 2 HARD CUT; the per-FT `ft_<id>` virtual layer is gone):
//   - ENTITY tables (snake-case noun: task, resource, status, …) with
//     synthetic `id` PK and functional absorptions as columns
//     (task.task_subject, resource.status_id, verb.status_id, …);
//     FK columns end `_id` and REFERENCE the parent table.
//   - JUNCTION tables for m:n / UC-less fact types, named from the
//     reading (task_blocks_task with task_id + task_id_2,
//     task_touches_source_file, …).
//   - UNARY occurrence tables for event fact types
//     (task_is_started: task_id + nullable timestamp).
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
    description:
      'Read-only SQL SELECT over the 3NF relational substrate (#864; rmap-3nf-tables Stage 2 HARD CUT — the per-FT ft_<id> layer is GONE). The schema is the SAME one the persisted app db carries: ENTITY tables (snake-case noun — task, resource, status) with synthetic `id` PK and functional absorptions as columns (FK columns end `_id`); JUNCTION tables for m:n fact types named from the reading (task_blocks_task: task_id, task_id_2); UNARY occurrence tables for events (task_is_started: task_id, nullable timestamp). Examples: `SELECT id FROM task WHERE task_priority = \'p0\'`; board status via `SELECT r.id, r.status_id FROM resource r WHERE r.status_id = \'pending\'` or `SELECT id, status_id FROM state_machine`. WHEN: cross-table JOINs, aggregates (COUNT/GROUP BY), NOT EXISTS subqueries, or any projection more expressive than one-FT-plus-one-equality-filter. ALTERNATIVE: query when one FT with simple role-equality filters is enough (it reads the CELL, not the tables); cells mode=get for raw cell contents. GOTCHA: SELECT-only — INSERT / UPDATE / DELETE are refused on purpose so derivation + validation always run through `apply`. Rows that violate alethic constraints (NOT NULL / FK) are absent — same skip set as the persisted db. Returns `{rows:[...]}` on success or `{error:"..."}` envelope. Local-mode only. NEXT: pipe rows into `get noun=<X> id=<row-value>` for per-entity context, or apply for mutations.',
    inputSchema: {
      query: z.string().describe('A SQL SELECT statement over the 3NF schema: entity tables (task, resource, …; columns = snake-case absorptions, FKs end _id), junction tables (task_blocks_task), unary event tables (task_is_started + timestamp). Quote identifiers per SQL standard.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ query, app }) => {
    const scope = callScope(app)
    if (AREST_MODE === 'local') {
      const raw = await systemCall('sql', query, scope)
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
    description:
      'Read-only cell-graph introspection (#870) — list / get / trace. WHEN: you want to know what cells EXIST (mode=list, with optional glob pattern), inspect raw cell contents without writing SQL (mode=get, FFP-parsed into role-keyed objects), or verify a derivation rule actually fired (mode=trace, returns rule_text + consequent_cell + materialized_count). Replaces "drop into sqlite3 to debug" workflows. ALTERNATIVE: query for FT-filtered tuple lists (same data via the projection lens — friendlier role-name keys, but only one FT at a time); sql when you need JOINs / aggregates across multiple cells; schema when you want the formal model (constraints / SMs / DRs) rather than the cell-graph view. GOTCHA: mode=get requires `name` exactly (no globbing); mode=trace needs either rule_id (exact DR.id) or rule_pattern (first substring hit wins) — providing both is fine, rule_id wins. Returns `{error}` envelopes uniformly; ⊥ → {error:"engine returned ⊥"} when std-deps feature absent. NEXT: query / sql for the actual rows once you have confirmed the cell exists and is populated; apply to mutate.',
    inputSchema: {
      mode: z.enum(['list', 'get', 'trace']).describe('Introspection mode: list, get, or trace.'),
      pattern: z.string().optional().describe('Glob pattern for `list` (e.g. "Task_*", "*derivation*"). Defaults to "*" — all cells.'),
      name: z.string().optional().describe('Exact cell name for `get` (e.g. "FactType", "Task_has_Task_Priority"). Required for get mode.'),
      rule_id: z.string().optional().describe('Exact match on a DerivationRule.id for `trace` mode. Provide either rule_id or rule_pattern.'),
      rule_pattern: z.string().optional().describe('Substring match on a DerivationRule.text for `trace` mode. First match wins.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ mode, pattern, name, rule_id, rule_pattern, app }) => {
    const scope = callScope(app)
    // `app` scopes the call; it is NOT part of the engine `cells` envelope.
    const envelope: Record<string, string> = { mode }
    if (pattern !== undefined) envelope.pattern = pattern
    if (name !== undefined) envelope.name = name
    if (rule_id !== undefined) envelope.rule_id = rule_id
    if (rule_pattern !== undefined) envelope.rule_pattern = rule_pattern
    if (AREST_MODE === 'local') {
      const raw = await systemCall('cells', JSON.stringify(envelope), scope)
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
    description:
      'Hypothesis-Candidate search over a fact type (#854) — abduction primitive. WHEN: you want the engine to ENUMERATE plausible bindings for a hidden FT, gate them through alethic constraints, score them by your Scoring Rules, and return ranked candidates (Confidence-Score-descending). Use this when "what value of role R best explains the observed evidence?" is the question. Input: ft_id (FT to search), optional to_explain (seq of InstanceFacts the candidate must forward-chain-derive — empty = open-ended), optional bound (pre-pin certain role values). Output: array of Hypothesis Candidate objects, each with hypothesisCandidateId + confidenceScore + the hidden-fact bindings. ALTERNATIVE: apply operation=create when you already KNOW the fact you want to assert (no search needed); query / sql when the answer can be read out of the existing population directly; propose when the candidate change is at the schema level (new FT / new constraint) rather than at the population level. GOTCHA: ⊥ → {error:"engine returned ⊥"} if handle missing or induce verb not wired; ft_id absent from the FactType cell → `[]` (no candidates), NOT an error. Top candidate appears at index 0 (stable sort preserved by the parser). See readings/core/induction.md for the Hypothesis Candidate / Confidence Score / Scoring Rule vocabulary. NEXT: inspect parsed[0].Hypothesis_Candidate_has_hidden__Fact, and if the binding is convincing, materialize it via apply operation=create.',
    inputSchema: {
      ft_id: z.string().describe('Fact type id to search over (e.g. "Hypothesis_has_Plausibility").'),
      to_explain: z.array(z.unknown()).optional().describe('Optional seq of InstanceFact-shaped facts the candidate should forward-chain-derive. Empty (default) means open-ended search.'),
      bound: z.record(z.string(), z.string()).optional().describe('Optional pre-bound role values keyed by role name. Constrains the cartesian enumeration to candidates that match these bindings.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ ft_id, to_explain, bound, app }) => {
    const scope = callScope(app)
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
      const raw = await systemCall('induce', arg, scope)
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

// ── #872 apply footgun-resistance helpers ─────────────────────────────
//
// Belt-and-suspenders TS-layer guards mirroring the engine fixes for
// #867 (apply create without id silently produced an empty-id orphan)
// and #868 (apply update with partial fields retracted unrelated
// single-valued facts). Engine fixes landed in f321a9dd; the MCP
// guards stay so agents get actionable feedback even if a future
// engine drift reintroduces the silent-failure behavior.
//
// Design (#867, revised task-964): the MCP no longer refuses no-id
// creates. The engine enforces opt-in auto-gen per noun (a marked noun
// auto-generates; an unmarked no-id create is rejected with
// create.id_required), so a blanket MCP refusal would block a marked
// noun's intended auto-gen. The #868 update-merge guard below stays.
//
// Design (#868): pre-fetch the existing entity via `get`, layer the
// payload on top, send the FULL set to the engine. Multi-valued
// touches (arrays in the get response) skip the merge — re-asserting
// them would replay every existing fact. Opt-out via
// `fields_only_replace: true` for the rare case the agent wants the
// old replace-only behavior.

// task-964: the #867/#872 create-id refusal helper was removed. The
// engine now enforces opt-in auto-gen per noun (create.id_required for
// an unmarked no-id create; auto-gen for a noun marked
// `<Noun> has an auto-generated id.`), so the MCP defers rather than
// duplicating an id check that would block marked nouns' auto-gen.

/**
 * #868 guard: merge a partial update payload onto the existing
 * entity snapshot.
 *
 * Only top-level scalar (string) fields are layered. Skipped:
 *   - the synthetic `id` field (engine takes id from the envelope,
 *     not from the fields map),
 *   - array values (multi-valued FT touches — re-asserting these
 *     would replay every existing touch as a fresh fact),
 *   - nested-object values (defensive against future HATEOAS
 *     evolutions of the `get` response shape),
 *   - null / undefined values (the engine reported "no value" — and
 *     pushing them back as "" would CREATE an empty fact, the exact
 *     bug #868 was about).
 *
 * Payload values WIN over existing values for the same field, which
 * gives true update semantics.
 *
 * Tolerates `existing` being null/undefined — degrades to "just send
 * the payload" so the helper is safe to call regardless of whether
 * the get-fetch hit anything.
 */
/**
 * The set of field-keys that map to a DECLARED fact type, derived from a
 * get response's `view.elements` (each carries a `factType` like
 * `Task_has_Task_Subject`, whose value-type name is the displayed field key).
 * The #868 merge uses this to avoid re-asserting NON-canonical phantom fields
 * — e.g. a bare `Timestamp` the get surfaced from an SM event fallback cell —
 * which the engine would only land back in a non-canonical fallback cell (a
 * silent data fork; see command.rs `unresolvable_field_key_violation`).
 * Returns null when no view/elements are present so the merge degrades to its
 * prior "re-assert every scalar" behavior rather than dropping real fields.
 */
export function declaredFieldKeysFromView(
  existing: Record<string, unknown> | null | undefined,
): Set<string> | null {
  const view = existing && typeof existing === 'object'
    ? (existing as Record<string, unknown>).view : undefined
  const elements = view && typeof view === 'object'
    ? (view as Record<string, unknown>).elements : undefined
  if (!Array.isArray(elements) || elements.length === 0) return null
  const keys = new Set<string>()
  for (const el of elements) {
    const ft = el && typeof el === 'object'
      ? (el as Record<string, unknown>).factType : undefined
    if (typeof ft === 'string' && ft.length > 0) {
      // Task_has_Task_Subject -> "Task Subject"; Task_has_Owner -> "Owner".
      keys.add(ft.replace(/^.*_has_/, '').replace(/_/g, ' '))
    }
  }
  return keys.size > 0 ? keys : null
}

export function mergeUpdateFields(
  existing: Record<string, unknown> | null | undefined,
  payload: Record<string, string>,
): Record<string, string> {
  const merged: Record<string, string> = {}
  const declaredKeys = declaredFieldKeysFromView(existing)
  if (existing && typeof existing === 'object') {
    for (const [k, v] of Object.entries(existing)) {
      if (k === 'id') continue
      if (v === null || v === undefined) continue
      if (Array.isArray(v)) continue
      if (typeof v === 'object') continue
      // #868 phantom guard: skip a pre-fetched scalar that maps to no declared
      // fact type (e.g. an SM event `Timestamp` surfaced from a fallback cell).
      // Re-asserting it would fork data into a non-canonical cell. Payload
      // fields (below) are always kept — the agent asked for those explicitly.
      if (declaredKeys && !declaredKeys.has(k)) continue
      merged[k] = String(v)
    }
  }
  for (const [k, v] of Object.entries(payload || {})) {
    merged[k] = v
  }
  return merged
}

/**
 * #872 builder: assemble the full merged update payload for the
 * `apply update` verb. Returns the merged fields map, the merge flag
 * (whether merging actually happened), and the list of preserved
 * field names (those layered from the existing snapshot but NOT
 * overwritten by the payload) — useful as a diff log when debugging.
 *
 * When `fields_only_replace` is true, returns the payload unchanged
 * (no merge), and `preserved` is empty. This is the opt-out for the
 * rare case the agent wants the old replace-only behavior.
 */
export function buildApplyMergedUpdatePayload(args: {
  existing: Record<string, unknown> | null | undefined
  payload: Record<string, string>
  fields_only_replace: boolean
}): { fields: Record<string, string>; merged: boolean; preserved: string[] } {
  if (args.fields_only_replace) {
    return { fields: { ...(args.payload || {}) }, merged: false, preserved: [] }
  }
  const fields = mergeUpdateFields(args.existing, args.payload || {})
  const payloadKeys = new Set(Object.keys(args.payload || {}))
  const preserved = Object.keys(fields).filter(k => !payloadKeys.has(k))
  return { fields, merged: true, preserved }
}

// ── #904 SM-bypass guard ─────────────────────────────────────────────
//
// When an app declares `State Machine Definition X is for Noun Y`,
// an `apply update` that sets Y's Status field directly bypasses the
// SM — the status changes without the transition firing and any
// derivation depending on SM state desynchronizes. apps/tasks/#861
// covered this for Task specifically; this guard generalizes it at
// the MCP layer so EVERY SM-governed noun in EVERY app's schema
// inherits the protection without per-app work.
//
// Design (#904): refuse-with-clear-message (Option 1, mirrors
// #867/#868). The error message names the SM, lists legal transitions
// from the current status (resolved via the actions verb upstream),
// and points at `apply transition event=<X>` as the right verb. The
// agent learns the transition vocabulary by reading the refusal —
// no whitepaper required (per #869 north star).
//
// Escape hatch: `force: true` bypasses the guard entirely. Used by
// migration scripts, admin entity-restore flows, and the rare cases
// where the SM history can't be replayed and direct status mutation
// is the right call.
//
// Why refuse rather than auto-redirect: an auto-redirect would have
// to GUESS which event the agent intended (some statuses have multiple
// outgoing transitions). Surfacing the legal events and letting the
// agent pick keeps the contract explicit. Why not warn-and-proceed:
// silent SM desync is exactly what #904 exists to prevent.

/**
 * #904 helper: detect whether a payload field name refers to the
 * Status value type of an SM-governed noun.
 *
 * Convention: an SM bound to noun `N` is fed by a "Task Status"-style
 * field whose name is `<N> Status` (e.g. `Task Status` for `Task`,
 * `Order Status` for `Order`). This mirrors the value-type naming the
 * apps follow in their readings and the cell layout the engine emits
 * (e.g. `Task_has_Task_Status`).
 *
 * Returns the matching field name if any, or null otherwise.
 */
export function findSmStatusField(
  noun: string,
  fields: Record<string, string> | undefined,
): string | null {
  if (!fields) return null
  const target = `${noun} Status`
  for (const k of Object.keys(fields)) {
    if (k === target) return k
  }
  return null
}

/**
 * #904 guard: refuse `apply update` when the payload sets the Status
 * field of an SM-governed noun in the active app's schema.
 *
 * Returns `null` when the call is safe to pass through:
 *   - `force: true` (explicit opt-out for migration scripts),
 *   - the active app has no SMs (guard is a no-op),
 *   - the noun being updated is not SM-governed in the schema,
 *   - the payload doesn't set the noun's Status field.
 *
 * Returns `{error}` otherwise. The error names the SM (via the
 * SM-governed noun), names the field that triggered the refusal,
 * lists legal transitions when available, and points at
 * `apply transition` as the right verb.
 *
 * `transitions` is optional — the actions verb's transitions list
 * for the entity's current status. When absent or empty, the refusal
 * still fires (the schema alone tells us the noun is SM-governed),
 * just without an enumeration of legal events.
 */
export function smBypassRefusal(args: {
  noun: string
  fields: Record<string, string> | undefined
  schema: { stateMachines?: Array<{ noun?: string }> } | null | undefined
  transitions?: Array<{ event?: string }> | null
  force?: boolean
}): { error: string } | null {
  if (args.force === true) return null
  const stateMachines = args.schema?.stateMachines
  if (!Array.isArray(stateMachines) || stateMachines.length === 0) return null
  // Find the SM bound to this noun, if any. We only care about the
  // *active app's* declared SMs; the schema arg comes from `debug`
  // (or the `/api/debug/schema/:domain` endpoint upstream).
  const sm = stateMachines.find(s => s && s.noun === args.noun)
  if (!sm) return null
  const statusField = findSmStatusField(args.noun, args.fields)
  if (!statusField) return null
  const events = Array.isArray(args.transitions)
    ? args.transitions.map(t => (t && typeof t.event === 'string') ? t.event : '').filter(e => e.length > 0)
    : []
  const eventList = events.length > 0
    ? ` Legal transitions from the current status: ${events.join(', ')}.`
    : ' Call the `actions` verb to enumerate the transitions legal for this entity\'s current status.'
  return {
    error:
      `apply update on noun '${args.noun}' refused (#904): the '${statusField}' field is governed ` +
      `by the '${args.noun} SM' state machine and must change via 'apply transition', not a direct ` +
      `field update. Setting the Status directly bypasses the SM — the status would flip without ` +
      `the transition firing, and any derivation depending on SM state would desync.` +
      eventList +
      ` Use: apply operation=transition noun='${args.noun}' id=<id> event=<one of the above>. ` +
      `Pass force=true to bypass this guard for migration scripts or other legitimate ` +
      `direct-mutation cases (rare).`,
  }
}

server.registerTool(
  'orient',
  {
    description:
      'One-screen session re-orientation (#871) — apps inventory + active app + recent cell-graph activity + suggested-next pointer, in a single envelope. WHEN: FIRST call in a new session, or any time the agent has lost the thread and wants "where am I, what just happened, what should I do next?". ALTERNATIVE: apps.current when you only need the active app name (cheaper); apps.list / apps.check when you want depth on every app and do NOT need the recent-activity summary; context when you specifically need a mutation receipt (orient does not mint one). GOTCHA: pass `app=<name>` to route the counts + recent activity to THAT app (the multi-agent-safe way — it never changes the shared default); omit `app` to report the current default app. Counts come from the routed app\'s loaded snapshot; sibling apps appear with last_compile mtimes but no per-app row counts (the engine holds one DB at a time). Pass apps_dir only when you want sibling enumeration. NEXT: follow the `suggested_next` pointer in the response, or call context if the next move is a mutation.',
    inputSchema: {
      apps_dir: z.string().optional().describe('Optional absolute path to the apps directory. When set, sibling apps are enumerated from filesystem (each must carry a `readings/` directory and a `*.db` file). When omitted, only the active app is reported.'),
      active_app: z.string().optional().describe('DEPRECATED label-only fallback (honored only when `app` is omitted): names the active entry + suggested_next without routing. Prefer `app`, which both routes and labels.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ apps_dir, active_app, app }) => {
    const scope = callScope(app)
    const envelope: Record<string, string> = {}
    if (apps_dir !== undefined) envelope.apps_dir = apps_dir
    // `app` (routing) names the active entry + suggested_next; else the
    // deprecated `active_app` label; else the global default app's name.
    // This is also the bug fix: counts now come from `scope`'s DB, not
    // whatever app happens to be globally active.
    envelope.active_app = scope ? scope.name : (active_app ?? activeApp.name)
    if (AREST_MODE === 'local') {
      const raw = await systemCall('orient', JSON.stringify(envelope), scope)
      return textResult(parseOrientResponse(raw))
    }
    const data = await httpRequest('/arest/default/orient', {
      method: 'POST',
      body: JSON.stringify(envelope),
    })
    return textResult(data)
  },
)

// ── 2f. engine_version: is the LIVE engine current? ──────────────────
//
// The MCP pins AREST_CLI at startup (server.ts:AREST_CLI) and re-spawns
// that exact path on every call. If the startup resolver picked a
// binary from a different build profile than the one being rebuilt, the
// server keeps spawning a STALE engine while the operator believes a
// fresh build is live. cli-staleness.ts only compares the resolved
// exe's MTIME to source, so it is blind to that profile mismatch. The
// unambiguous fix is the RUNNING binary self-reporting its provenance:
// `arest-cli version` prints the git SHA + build time it was compiled
// from, and this verb compares that against the repo's current HEAD.
//
// NOTE: like cli-staleness, this answers about the binary the server is
// ALREADY pinned to — a rebuild does not take effect until the MCP is
// relaunched. That is precisely the footgun it detects.
server.registerTool(
  'engine_version',
  {
    description:
      'Provenance + staleness of the LIVE AREST engine — "which engine is actually running, and is it current with the repo?" in ONE call. WHEN: after a `cargo build` of arest-cli, to confirm the freshly built binary is the one the MCP is actually spawning (the server PINS AREST_CLI at startup and re-spawns that exact path every call, so a rebuild does NOT take effect until relaunch); or any time a "deployed + verified" claim needs to be trusted. HOW: shells the pinned `arest-cli version` (which prints the git SHA + build timestamp it was compiled from, embedded at build time via build.rs), reads the repo HEAD via `git rev-parse HEAD`, and returns {live_sha, live_built, head_sha, pkg, up_to_date, behind_message}. up_to_date is true ONLY when the live binary\'s SHA equals HEAD. GOTCHA: when up_to_date is false the fix is REBUILD **then RELAUNCH the MCP** — the running server cannot pick up a new binary mid-session (this verb is the detector for exactly that situation). A "unknown" live_sha means the running binary predates this subcommand (relaunch after rebuild). ALTERNATIVE: orient for session re-orientation (apps + activity), apps.check for per-app DB health — neither reports the binary\'s git provenance. Local mode only; remote/cloudflare runs WASM/HTTP, not a pinned CLI.',
    inputSchema: {},
  },
  async () => {
    if (AREST_MODE !== 'local') {
      return textResult({
        mode: AREST_MODE,
        up_to_date: null,
        behind_message:
          'engine_version reports the LOCAL pinned arest-cli binary; in ' +
          `${AREST_MODE} mode the engine runs as WASM/HTTP and has no pinned CLI to interrogate.`,
      })
    }
    let versionJson: string
    try {
      versionJson = await runArestCli(['version'])
    } catch (err) {
      // Old binary without the `version` subcommand, or a spawn failure.
      // Surface as an unknown live SHA so the comparison flags it.
      versionJson = JSON.stringify({ sha: 'unknown', built: 'unknown', pkg: 'unknown' })
      if (AREST_DEBUG) console.error('[engine_version] arest-cli version failed:', err)
    }
    const comparison = compareEngineVersion(versionJson, readHeadSha())
    return textResult({ ...comparison, cli_path: AREST_CLI })
  },
)

// ── 3. apply: create, update, or transition an entity ────────────────

server.registerTool(
  'apply',
  {
    description:
      'Population-level mutation — create / update / transition an entity, OR a COLLECTION of such ops applied atomically (task-930). Runs the full pipeline: resolve → derive → validate → emit. WHEN: you have a known fact change to assert. operation=create makes a new entity (REQUIRES explicit id per #867 — MCP refuses silent-id); operation=update modifies fields on an existing id (MERGES with existing single-valued facts by default per #868 / #872, so a partial payload does NOT silently retract untouched fields); operation=transition fires an SM event (engine looks up the legal transition for the current status and target event). BULK / COLLECTION SHAPE (task-930): pass `ops` — an ARRAY of {operation, noun, id, fields?, event?} objects — to apply many ops as ONE atomic request. This is Backus α (apply-to-all) over the collection: the batch resolves all ops over a shared cumulatively-built population, derives to the least fixed point ONCE over the combined state, validates, and emits a single delta. An ALETHIC violation in ANY op rejects the WHOLE batch (D\' = D — nothing lands, not even ops before the violation); deontic findings warn but the batch still commits. A single op is just the 1-element collection — you can always pass `ops:[{...}]` instead of the flat fields, or keep the flat single-op shape. ALTERNATIVE: retract for exact-tuple removal from a FactType cell (skips entity-shape envelope); compile when you are changing the SCHEMA not the population (new FT / new constraint); propose when the schema change needs a governed review workflow before taking effect; induce when you want the engine to SEARCH for the right binding instead of you supplying it. GOTCHA: context_receipt is required — call context first, paste its receipt here. Update merge can be opted out with fields_only_replace=true (rare). transition needs `event` not `fields`. Engine will reject on alethic violation or missing reference scheme. NEXT: get noun=<N> id=<id> to confirm the new state, or actions to see what transitions are now legal.',
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      operation: z.enum(['create', 'update', 'transition']).optional().describe('Operation type for the single-op shape. Omit when passing `ops` (the collection shape).'),
      noun: z.string().optional().describe('Entity noun type (e.g. "Order", "Case"). Single-op shape; omit when passing `ops`.'),
      id: z.string().optional().describe('Entity ID. Required for update/transition. For create: required unless the noun opts into auto-gen via the reading `<Noun> has an auto-generated id.` (task-964) -- an unmarked no-id create is rejected engine-side with create.id_required.'),
      fields: z.record(z.string(), z.string()).optional().describe('Fact pairs for create/update (e.g. {"Name": "Acme", "customer": "alice"})'),
      event: z.string().optional().describe('SM event for transition (e.g. "place", "ship")'),
      ops: z.array(z.object({
        operation: z.enum(['create', 'update', 'transition', 'assertFact']).describe('Operation type for this collection member.'),
        noun: z.string().optional().describe('Entity noun type. Required for create/update; for transition the engine resolves the SM by entity id.'),
        id: z.string().optional().describe('Entity ID for this op.'),
        fields: z.record(z.string(), z.string()).optional().describe('Fact pairs for create/update.'),
        event: z.string().optional().describe('SM event for transition.'),
        fact_type: z.string().optional().describe('assertFact member: fact-type cell name (any cell — n-ary, same-signature, ring).'),
        pairs: z.array(z.object({ role: z.string(), value: z.string() })).optional().describe('assertFact member: ordered role/value pairs (repeated role names allowed).'),
      })).optional().describe('task-930 COLLECTION shape — an array of ops applied atomically as ONE request (Backus α). One derive→validate→emit pass over the combined population; an alethic violation in ANY op rolls back the WHOLE batch (D\' = D). A single op is the natural 1-element collection. When present, the flat operation/noun/id/fields/event are ignored. apply-pairs-arbitrary-cells: members may also be {operation:\'assertFact\', fact_type, pairs} — exact-tuple assertions (n-ary FTs, same-role-signature FTs, m:n multi-row) ride the SAME atomic batch as entity ops, so e.g. one Frame\'s 4 available Action Types land all-or-nothing.'),
      sender: z.string().optional().describe('Caller identity for authorization'),
      signature: z.string().optional().describe('HMAC-SHA256 signature'),
      fields_only_replace: z.boolean().optional().describe('Opt-out (#872) — when true, the MCP skips the merge-with-existing pre-fetch on update and sends ONLY the payload fields to the engine. Use this for the rare case the agent intentionally wants the old replace-only behavior; default (false) is safer (#868 belt-and-suspenders).'),
      force: z.boolean().optional().describe('Opt-out (#904) — when true, the MCP skips the SM-bypass guard on update and lets the call go through even if a payload field is the Status of an SM-governed noun. Use this for migration scripts or other legitimate direct-mutation cases (rare); default (false) refuses the call and points the agent at `apply transition` instead.'),
      // task-971 + apply-pairs-arbitrary-cells: exact-tuple assertion
      // into ANY fact-type cell. The entity-oriented paths use a MAP for
      // fields (unique keys), so they cannot express: same-noun rings
      // (role name appears twice), n-ary FTs whose role set is ambiguous
      // against a sibling FT on the same noun, two FTs sharing a role
      // signature (the cell NAME disambiguates), or m:n multi-row
      // assertions. fact_type + pairs reaches all of them — the engine's
      // assert: path appends the exact tuple and runs the full
      // derive→validate→emit pipeline.
      fact_type: z.string().optional().describe('Exact-tuple assertion: fact-type cell name — ANY cell, not just rings (e.g. "Task_blocks_Task", ternary "Run_took_Action_Count_on_Level", or one of two FTs sharing a role signature, where the cell name disambiguates). Use with `pairs` — not with `operation`/`noun`. For multi-row m:n assertions or mixing with entity ops atomically, use `ops` with {operation:"assertFact", fact_type, pairs} members instead.'),
      pairs: z.array(z.object({
        role: z.string(),
        value: z.string(),
      })).optional().describe('Ordered role/value pairs for the exact-tuple assertion. Repeated role names are allowed (same-noun rings: [{role:"Task",value:"A"},{role:"Task",value:"B"}] asserts Task A blocks Task B); n-ary tuples list every role in declared order (e.g. Run/Action Count/Level). Use with `fact_type`.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ context_receipt, operation, noun, id, fields, event, ops, sender, signature, fields_only_replace, force, fact_type, pairs, app }) => {
    // p0 isolation: a per-call `app` scopes THIS mutation to its own
    // db/readings/handle without touching the shared active app or the
    // marker. Note the mutation gate's context_receipt is still minted
    // against the SESSION's active app; an app-scoped mutation therefore
    // bypasses the per-app receipt binding by design (the caller asked
    // to act on a specific app for one call). Omit `app` for the
    // default receipt-bound active-app behavior.
    const scope = callScope(app)
    // ── task-930: bulk / collection-shaped apply ──────────────────────
    // When `ops` is supplied, build ONE batch command and route it
    // through the engine's `apply` verb (platform_apply_command), which
    // dispatches Command::Batch → apply_command_batch: one atomic
    // request, one fixpoint over the combined population, alethic
    // rollback of the whole collection. We deliberately route through
    // the single `apply` system call (NOT the per-op systemCall path)
    // so atomicity is the engine's, not N independent CLI/engine calls.
    if (Array.isArray(ops) && ops.length > 0) {
      const blockedBatch = mutationGateResult('apply', context_receipt, { operation: 'batch', ops })
      if (blockedBatch) return blockedBatch
      const commands = ops.map(op => buildApplyCommandForBatch(op, { sender, signature }))
      const batch = { type: 'batch', commands }
      const result = await dispatchCommand(batch, scope)
      return textResult(result)
    }

    // task-971: same-noun ring fact assertion via fact_type + pairs.
    // This is the only path that can express <<Task,A>,<Task,B>> because
    // the entity-oriented paths use a MAP (unique keys).
    if (fact_type && Array.isArray(pairs) && pairs.length > 0) {
      const blockedRing = mutationGateResult('apply', context_receipt, { operation: 'assertFact', fact_type, pairs })
      if (blockedRing) return blockedRing
      if (AREST_MODE === 'local') {
        const escapeAtom = (s: string) => s.replace(/[\\<>,]/g, ch => '\\' + ch)
        const input = `<${pairs.map(({ role, value }) => `<${escapeAtom(role)}, ${escapeAtom(value)}>`).join(', ')}>`
        const raw = await systemCall(`assert:${fact_type}`, input, scope)
        const ok = raw === 'ok'
        return textResult(ok ? { status: 'ok', fact_type, pairs } : { error: raw, fact_type })
      }
      // Remote mode: dispatch via HTTP using the assertFact Command shape.
      const command = { type: 'assertFact', factType: fact_type, pairs, sender, signature }
      const data = await httpRequest('/arest/default/apply', { method: 'POST', body: JSON.stringify(command) })
      return textResult(data)
    }

    if (!operation || !noun) {
      return textResult(
        'apply requires either a single op (operation + noun), a collection (ops: [...]), ' +
        'or a ring fact assertion (fact_type + pairs). ' +
        'See the tool description for the bulk / collection shape (task-930) or task-971 ring assertion.',
      )
    }
    const blocked = mutationGateResult('apply', context_receipt, { operation, noun, id, fields, event })
    if (blocked) return blocked

    // task-964: no MCP-layer id refusal on `create`. The engine now
    // enforces opt-in auto-gen per noun -- a noun marked
    // `<Noun> has an auto-generated id.` auto-generates; an unmarked
    // no-id create is a hard alethic reject (create.id_required)
    // surfaced to the agent. The old #867/#872 blanket refusal would
    // block a marked noun's legitimate auto-gen, so we defer to the
    // engine's predicate-driven gate rather than duplicating it here.

    // #904 guard: refuse `update` when a payload field is the Status of
    // an SM-governed noun in the active app's schema. The schema is
    // pulled from the engine's `debug` envelope (same path the schema
    // verb uses) and the transitions list is best-effort via the
    // `transitions:<noun>` system call from the entity's current SM
    // status. Opt-out via `force: true`. Pass-through when the app has
    // no SMs (no schema cost in that case).
    if (operation === 'update' && AREST_MODE === 'local' && force !== true) {
      let schema: { stateMachines?: Array<{ noun?: string }> } | null = null
      try {
        const rawSchema = await systemCall('debug', '', scope)
        const parsed = JSON.parse(rawSchema)
        // `debug` may emit either a JSON-string atom (debug-def feature
        // on) or an FFP-shaped Seq summary (feature off). We only want
        // the former — the latter has no stateMachines list, so the
        // guard is a no-op.
        if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
          schema = parsed as typeof schema
        } else if (typeof parsed === 'string') {
          try { schema = JSON.parse(parsed) as typeof schema } catch {}
        }
      } catch {
        // schema-fetch miss is non-fatal — degrade to "no guard" so
        // we never block on infrastructure failures. The engine
        // still validates downstream.
      }
      if (schema) {
        // Resolve transitions from the entity's current status so the
        // refusal can enumerate legal events. Best-effort: misses fall
        // back to "call the actions verb to enumerate" in the message.
        let transitionsList: Array<{ event: string }> = []
        if (id) {
          try {
            const smRaw = await systemCall('get:State Machine', id, scope)
            const sm = JSON.parse(smRaw)
            const status = sm && typeof sm === 'object' && typeof sm.Status === 'string'
              ? sm.Status
              : ''
            if (status) {
              const rawTransitions = await systemCall(`transitions:${noun}`, status, scope)
              const parsedT = JSON.parse(rawTransitions)
              if (Array.isArray(parsedT)) {
                transitionsList = parsedT
                  .map((t: any) => ({ event: String(t?.event ?? t?.Event ?? '') }))
                  .filter(t => t.event)
              }
            }
          } catch {
            // transitions-fetch miss is non-fatal — refusal still fires
            // with a generic "call the actions verb" hint instead of an
            // enumeration.
          }
        }
        const refusal = smBypassRefusal({
          noun,
          fields,
          schema,
          transitions: transitionsList,
          force: false,
        })
        if (refusal) return textResult(refusal)
      }
    }

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
          const raw = await systemCall(`create:${noun}`, `<${idPair}${pairs}>`, scope)
          return localApplyResult(raw, { operation, noun, id, fields })
        }
        case 'update': {
          // #872 / #868 guard: pre-fetch the existing entity and layer
          // the payload on top so untouched single-valued fields don't
          // get retracted if a future engine drift breaks the per-field
          // retract semantics from f321a9dd. Opt-out via
          // `fields_only_replace: true`.
          let outboundFields = fields || {}
          if (id && Object.keys(outboundFields).length > 0) {
            let existing: Record<string, unknown> | null = null
            try {
              const rawExisting = await systemCall(`get:${noun}`, id, scope)
              const parsed = JSON.parse(rawExisting)
              if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
                existing = parsed as Record<string, unknown>
              }
            } catch {
              // get-fetch miss is non-fatal — the merge degrades to
              // "just send the payload" so we don't block the update.
            }
            const built = buildApplyMergedUpdatePayload({
              existing,
              payload: outboundFields,
              fields_only_replace: fields_only_replace === true,
            })
            outboundFields = built.fields
            // Debug-log the preserved set so a future engine drift is
            // visible without forcing the agent to dig through cells.
            if (AREST_DEBUG && built.merged && built.preserved.length > 0) {
              console.error(
                `[#872 apply update] preserved by merge: ${built.preserved.join(', ')}`,
              )
            }
          }
          const pairs = Object.entries(outboundFields).map(([k, v]) => `<${escapeAtom(k)}, ${escapeAtom(v)}>`).join(', ')
          const raw = await systemCall(`update:${noun}`, `<<id, ${escapeAtom(id || '')}>${pairs ? `, ${pairs}` : ''}>`, scope)
          return localApplyResult(raw, { operation, noun, id, fields: outboundFields })
        }
        case 'transition': {
          const raw = await systemCall(`transition:${noun}`, `<${escapeAtom(id || '')}, ${escapeAtom(event || '')}>`, scope)
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

server.registerTool(
  'retract',
  {
    description:
      'Remove an exact fact tuple from a FactType cell. WHEN: you want to delete one specific row from one specific FT (e.g. "Order ord-1 was_placed_by alice" — but it really was bob). ALTERNATIVE: apply operation=update when you want to REPLACE a single-valued fact (update merges, retract removes); apply operation=transition for SM-driven status removal (transitions are the modeled way to advance / withdraw entity state, not direct fact retraction); compile when you want to remove the FT itself from the schema. GOTCHA: context_receipt is required. Stored semiderivations CAN be retracted, but the engine may re-derive them on the next forward-chain pass from their supporting facts — the only durable removal is to retract those supporting facts too. Use roles={...} for ordinary FTs; use pairs=[{role, value}, ...] when role names repeat. Local-mode only. NEXT: query fact_type=<X> to confirm the tuple is gone; downstream constraints / derivations re-fire on the next read.',
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      fact_type: z.string().describe('Fact type ID / cell name, e.g. "Order_was_placed_by_Customer"'),
      roles: z.record(z.string(), z.string()).optional().describe('Role bindings for the exact fact tuple. Use pairs instead when role names repeat.'),
      pairs: z.array(z.object({
        role: z.string(),
        value: z.string(),
      })).optional().describe('Ordered role/value pairs for exact tuple matching, including repeated role names.'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ context_receipt, fact_type, roles, pairs, app }) => {
    const scope = callScope(app)
    const blocked = mutationGateResult('retract', context_receipt, { fact_type, roles, pairs })
    if (blocked) return blocked

    if (AREST_MODE !== 'local') {
      return textResult({ error: 'retract requires local mode' })
    }

    const entries = Array.isArray(pairs) && pairs.length
      ? pairs.map(({ role, value }) => [role, value] as const)
      : Object.entries(roles || {})

    const input = `<${entries.map(([role, value]) => `<${escapeAtom(role)}, ${escapeAtom(value)}>`).join(', ')}>`
    const raw = await systemCall(`retract:${fact_type}`, input, scope)
    return textResult(parseJsonResult(raw))
  },
)

// ── 4. actions: get valid actions for an entity (HATEOAS) ────────────

server.registerTool(
  'actions',
  {
    description:
      'HATEOAS — return the SM transitions currently legal for ONE entity, plus its current entity_data. WHEN: you have an entity id and you want to know "what can I do next?" without reading the state-machine reading by hand. The verb resolves the current status from the State Machine cell (or accepts an explicit `status`), then asks the engine for outgoing edges keyed by that status. ALTERNATIVE: propose for GOVERNED schema evolution (Domain Change with review workflow — propose is to schema what apply is to population); explain to see how the entity arrived at its current state (audit + derivation chain); get for the entity\'s facts without the transitions list; schema if you want the SM definition itself rather than per-entity legal moves. GOTCHA: returns [] for an entity with no SM binding, or for an unknown id — same shape, not an error. If you omit `status` and the entity has no State Machine row, transitions come back as []. NEXT: pick one transition row and call apply operation=transition noun=<N> id=<X> event=<row.event>.',
    inputSchema: {
      noun: z.string().describe('Entity noun type'),
      id: z.string().describe('Entity ID'),
      status: z.string().optional().describe('Current SM status (resolved from state if omitted)'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ noun, id, status, app }) => {
    const scope = callScope(app)
    if (AREST_MODE === 'local') {
      const parseOr = <T>(raw: string, fallback: T): T | any => {
        try { const v = JSON.parse(raw); return v ?? fallback } catch { return fallback }
      }
      // Resolve current status from the SM entity keyed by this id when the
      // caller doesn't pass one — transitions:{noun} needs a status to filter
      // outgoing edges, otherwise it returns [].
      let resolvedStatus = status || ''
      if (!resolvedStatus) {
        const smRaw = await systemCall(`get:State Machine`, id, scope)
        const sm = parseOr(smRaw, null)
        if (sm && typeof sm === 'object' && typeof sm.Status === 'string') {
          resolvedStatus = sm.Status
        }
      }
      const rawTransitions = await systemCall(`transitions:${noun}`, resolvedStatus, scope)
      const rawEntity = await systemCall(`get:${noun}`, id, scope)
      const parsedTransitions = parseOr(rawTransitions, null)
      // mcp-get-surface-view-representations: the actions verb previously
      // dropped the ui-readings `view` layer that `get` surfaces. `get:{noun}`
      // returns ONLY the flattened 3NF row (never the view), so fetch the view
      // via the same getEntity command path the get tool uses (line ~1256) and
      // ride it alongside the HATEOAS transitions. Additive + non-fatal: a
      // getEntity miss (older binary / ⊥ / rejected → parseGetEntityResponse
      // null) just omits `view`, leaving today's transitions + entity_data shape.
      let view: unknown
      try {
        const envelope = JSON.stringify({
          command: { type: 'getEntity', noun, entityId: id },
          population: '',
        })
        const enriched = parseGetEntityResponse(await systemCall('apply', envelope, scope))
        if (enriched && enriched.view !== undefined && enriched.view !== null) {
          view = enriched.view
        }
      } catch {
        // getEntity dispatch failure is non-fatal — actions still answers
        // with transitions + entity_data exactly as before.
      }
      return textResult({
        entity: id,
        noun,
        status: resolvedStatus || null,
        transitions: Array.isArray(parsedTransitions)
          ? parsedTransitions
          : normalizeTransitionRows(rawTransitions, noun, id),
        entity_data: parseOr(rawEntity, null),
        ...(view !== undefined ? { view } : {}),
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
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ id, noun, fact, app }) => {
    const scope = callScope(app)
    if (AREST_MODE === 'local') {
      // Audit trail for this entity
      const auditRaw = await systemCall('audit', '0', scope)
      let audit: any[] = []
      try {
        const parsed = JSON.parse(auditRaw)
        if (Array.isArray(parsed)) audit = parsed
      } catch {}

      // If a specific fact type is requested, query it
      let factData: any = []
      if (fact) {
        const raw = await systemCall(`query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {}), scope)
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
    description:
      'In-process schema self-modification (Corollary 5): feed FORML2 readings text to the LIVE active engine — new nouns, fact types, constraints, derivation rules, and state machines become callable immediately. WHEN: you want to extend the active app\'s model WITHOUT persisting the change to disk yet — quick iteration, exploration, or "what if I add this constraint?" trials. ALTERNATIVE: apps.compile when you want to REBUILD the on-disk SQLite .db from the readings/ directory (full refresh; readings are the source of truth there); propose when the schema change needs a governed Domain Change review workflow before taking effect; apply when you are changing the population, not the schema. GOTCHA: context_receipt is required — call context first, paste its receipt here. Alethic violations REJECT the compile (the engine returns ⊥ and the model stays as it was). This does NOT write to disk — if you want the readings/ directory to reflect the change, you also need to edit the .md file and call apps.compile. NEXT: schema or cells mode=list to confirm the new definitions are visible; apply to start populating the new fact types.',
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
    description:
      'Dump the FULL formal model: every Noun, FactType, Constraint, State Machine, Derivation Rule, plus reference schemes. WHEN: you need the canonical model surface — agent is composing readings and wants to verify naming conventions, or a downstream tool needs the complete picture. ALTERNATIVE: cells mode=list pattern=<glob> for targeted lookups (much smaller payload, faster); query fact_type=FactType to enumerate just the FT cell; orient when you want activity + apps overview not the model. GOTCHA: this is a LARGE payload — for any apps beyond a small toy domain it can run to many KB. Prefer cells/query whenever you have a specific name in mind. Returns the engine\'s raw schema envelope (no contextual receipt needed — read-only). NEXT: pick a specific FT name from the response and call cells mode=get name=<X> for contents, or query fact_type=<X> for a filtered tuple list.',
    inputSchema: {
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ app }) => {
    const scope = callScope(app)
    if (AREST_MODE === 'local') {
      const data = await dispatchRead('/schema', scope)
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
    description:
      'Governed schema evolution — stage a Domain Change entity that bundles proposed elements (readings / nouns / fact types / constraints / verbs / state machines) and enters the review workflow at status="Proposed". WHEN: the schema change requires human (or another agent) sign-off BEFORE taking effect — audit-tracked, review-tracked, rollback-able. ALTERNATIVE: compile when you want IMMEDIATE in-process schema change with no review (Corollary 5 — fast iteration); apply when the change is at the population level (entity create / update / transition), not the schema; actions when you want to advance an EXISTING Domain Change through its workflow. GOTCHA: context_receipt is required. Creating the Domain Change entity does NOT apply the schema change — you must walk the SM (events: review → approve-change → apply) for the proposed readings to take effect. The verb returns a change_id you will use in the follow-up transitions; the response\'s next_actions array spells out the SM walk. NEXT: apply operation=transition noun="Domain Change" id=<change_id> event=review to advance.',
    inputSchema: {
      context_receipt: z.string().optional().describe(CONTEXT_RECEIPT_FIELD_DESCRIPTION),
      rationale: z.string().describe('Why this change is needed'),
      target_domain: z.string().describe('Domain slug to change (e.g. "orders", "core")'),
      readings: z.array(z.string()).optional().describe('FORML2 reading text to add'),
      nouns: z.array(z.string()).optional().describe('Noun names to declare'),
      constraints: z.array(z.string()).optional().describe('Constraint texts'),
      verbs: z.array(z.string()).optional().describe('Verb names to declare'),
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
    },
  },
  async ({ context_receipt, rationale, target_domain, readings, nouns, constraints, verbs, app }) => {
    const scope = callScope(app)
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
    const createRaw = await systemCall(`create:Domain Change`, JSON.stringify(createCmd), scope)
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
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
      llm_response: z.string().optional().describe('Pre-sampled JSON projection spec (skip client sampling). Shape: {"fact_type":..., "filter":{...}}'),
    },
  },
  async ({ question, noun, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'ask requires local mode' })
    }
    const schemaRaw = noun
      ? await systemCall(`schema:${noun}`, '', scope)
      : await systemCall('list:Noun', '', scope)

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
    const raw = await systemCall(`query:${spec.fact_type}`, filterStr, scope)
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
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
      id: z.string().optional().describe('Specific entity ID, or synthesize all entities of the noun if omitted'),
      llm_response: z.string().optional().describe('Pre-sampled prose (skip client sampling). Used verbatim as the `prose` field.'),
    },
  },
  async ({ noun, id, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'synthesize requires local mode' })
    }
    const raw = id
      ? await systemCall(`get:${noun}`, id, scope)
      : await systemCall(`list:${noun}`, '', scope)
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
      app: z.string().optional().describe(APP_OVERRIDE_FIELD_DESCRIPTION),
      llm_response: z.string().optional().describe('Pre-sampled JSON facts array (skip client sampling). Shape: [{"fact_type":..., "bindings":{...}}, ...]'),
    },
  },
  async ({ text, constraint, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'validate requires local mode' })
    }
    const constraintRaw = await systemCall(`constraint:${constraint}`, '', scope).catch(() => '')

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
        const vraw = await systemCall(`verify:${fact.fact_type}`, factStr, scope)
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
    const actual = sm?.Status ?? null
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
    description:
      'Apply create/update/transition against the tutor sandbox. Same shape as `apply`. Mutations are scoped to the sandbox; the active app is untouched. ' +
      'BULK / COLLECTION SHAPE (task-930): pass `ops` — an ARRAY of {operation, noun, id, fields?, event?} — to apply a COLLECTION of ops in ONE atomic call. This is Backus α (apply-to-all) over the collection: one resolve→derive→validate→emit pass over the combined population, with an ALETHIC violation in ANY op rolling back the WHOLE batch (D\' = D). Practise it here: e.g. seed two Orders and place one of them in a single call, or watch a duplicate-id op roll back the others. A single op is just the 1-element collection.',
    inputSchema: {
      operation: z.enum(['create', 'update', 'transition']).optional(),
      noun: z.string().optional(),
      id: z.string().optional(),
      event: z.string().optional(),
      fields: z.record(z.string(), z.string()).optional(),
      ops: z.array(z.object({
        operation: z.enum(['create', 'update', 'transition']),
        noun: z.string().optional(),
        id: z.string().optional(),
        fields: z.record(z.string(), z.string()).optional(),
        event: z.string().optional(),
      })).optional().describe('task-930 COLLECTION shape — an array of ops applied atomically as ONE request. Alethic violation in any op rolls back the whole batch.'),
    },
  },
  async ({ operation, noun, id, event, fields, ops }) => {
    // task-930: a collection routes through the sandbox `apply` verb
    // (→ platform_apply_command → Command::Batch) so the SAME atomic
    // batch semantics the active app gets are taught in the sandbox.
    if (Array.isArray(ops) && ops.length > 0) {
      const commands = ops.map(op => buildApplyCommandForBatch(op, {}))
      const raw = await tutorSystemCall('apply', JSON.stringify({ type: 'batch', commands }))
      try { return textResult(JSON.parse(raw)) } catch { return textResult({ raw }) }
    }
    if (!operation || !noun) {
      return textResult('tutor.apply needs either a single op (operation + noun) or a collection (ops: [...]).')
    }
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
