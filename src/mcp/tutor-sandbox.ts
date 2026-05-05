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
 *
 * Two modes:
 *   • WASM (no AREST_CLI):   in-process handle, lost on MCP restart.
 *   • CLI  (AREST_CLI set):  shells out to arest-cli against
 *                            $AREST_TUTOR_DB; survives restarts.
 */
/// <reference types="node" />
import { readFileSync, readdirSync, existsSync, rmSync, mkdirSync, mkdtempSync, writeFileSync } from 'fs'
import { resolve, dirname, join } from 'path'
import { tmpdir } from 'os'
import { fileURLToPath } from 'url'
import { spawn } from 'child_process'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

let _sandboxHandle = -1
let _engine: typeof import('../api/engine.js') | null = null
let _bootstrapPromise: Promise<void> | null = null

export function tutorDomainsDir(): string {
  return resolve(__dirname, '..', '..', 'tutor', 'domains')
}

export function tutorSandboxDbPath(): string {
  return process.env.AREST_TUTOR_DB
    ?? resolve(__dirname, '..', '..', 'tutor', '.sandbox', 'tutor.db')
}

function shouldUseCliDb(): boolean {
  return Boolean(process.env.AREST_CLI)
}

function runArestCli(args: string[]): Promise<string> {
  const bin = process.env.AREST_CLI
  if (!bin) throw new Error('AREST_CLI not set')
  return new Promise((resolvePromise, reject) => {
    const child = spawn(bin, args, { env: process.env, windowsHide: true })
    let stdout = ''
    let stderr = ''
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

async function ensureCliBootstrapped(): Promise<void> {
  if (_bootstrapPromise) return _bootstrapPromise
  _bootstrapPromise = (async () => {
    const dbPath = tutorSandboxDbPath()
    mkdirSync(resolve(dbPath, '..'), { recursive: true })
    if (!existsSync(dbPath)) {
      await runArestCli([tutorDomainsDir(), '--db', dbPath])
    }
  })().catch((err) => {
    _bootstrapPromise = null  // allow retry on next call after failure
    throw err
  })
  return _bootstrapPromise
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
  if (shouldUseCliDb()) {
    await ensureCliBootstrapped()
    if (key === 'compile') {
      // arest-cli's `compile` SYSTEM key against persisted state does not
      // append schema. The binary's positional readings-dir form is the
      // one that registers nouns/facts into --db. Drop the input into a
      // tempdir and re-invoke that form.
      const tempDir = mkdtempSync(join(tmpdir(), 'arest-tutor-compile-'))
      try {
        writeFileSync(join(tempDir, '_extra.md'), input)
        return await runArestCli([tempDir, '--db', tutorSandboxDbPath()])
      } finally {
        try { rmSync(tempDir, { recursive: true, force: true }) } catch {}
      }
    }
    return runArestCli(['--db', tutorSandboxDbPath(), key, input])
  }
  const engine = await getEngine()
  const handle = await getSandboxHandle()
  return engine.system(handle, key, input)
}

export async function resetSandbox(): Promise<void> {
  if (_sandboxHandle >= 0 && _engine) {
    try { _engine.release_domain?.(_sandboxHandle) } catch {}
  }
  _sandboxHandle = -1
  _bootstrapPromise = null
  const dbPath = tutorSandboxDbPath()
  try { if (existsSync(dbPath)) rmSync(dbPath) } catch {}
}

/**
 * Test-only helper. Drops the in-process WASM handle and the CLI
 * bootstrap flag without deleting the persisted DB file. Used by the
 * persistence test to simulate an MCP server restart.
 */
export function _testOnly_dropSandboxHandle(): void {
  if (_sandboxHandle >= 0 && _engine) {
    try { _engine.release_domain?.(_sandboxHandle) } catch {}
  }
  _sandboxHandle = -1
  _bootstrapPromise = null
}
