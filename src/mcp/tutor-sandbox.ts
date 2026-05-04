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
