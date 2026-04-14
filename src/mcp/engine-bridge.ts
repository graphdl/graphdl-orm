/**
 * Shared WASM engine / handle cache for the MCP servers.
 *
 * Both the remote (streamable-HTTP) and streaming MCP servers run in
 * a single Worker isolate that owns ONE AREST compile state. This
 * module holds the lazy engine import and the shared handle so the
 * two servers can call `systemCall(key, input)` without each
 * initialising its own engine or fighting over separate handles.
 *
 * v1 scope: single shared handle per isolate (same as remote.ts
 * before this extraction). Per-session isolation is a follow-up.
 */

let _engine: typeof import('../api/engine.js') | null = null
let _handle: number | null = null

export async function getEngine(): Promise<typeof import('../api/engine.js')> {
  if (_engine) return _engine
  _engine = await import('../api/engine.js')
  return _engine
}

export async function getHandle(): Promise<number> {
  if (_handle !== null) return _handle
  const engine = await getEngine()
  _handle = engine.compileDomainReadings()
  return _handle
}

/**
 * Invoke the AREST engine's SYSTEM function with a key/input pair.
 * Equivalent to `engine.system(handle, key, input)` with the shared
 * handle plumbed through. Returns the raw JSON string the engine
 * emits; call safeJson() at the call site to decode.
 */
export async function systemCall(key: string, input: string): Promise<string> {
  const engine = await getEngine()
  const handle = await getHandle()
  return engine.system(handle, key, input)
}

export function safeJson<T>(raw: string, fallback: T): T | unknown {
  try { const v = JSON.parse(raw); return v ?? fallback } catch { return fallback }
}
