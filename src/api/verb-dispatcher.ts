/**
 * Unified MCP-verb dispatcher (#200).
 *
 * Every surface — CLI, local MCP (stdio), HTTP — routes through
 * `dispatchVerb(verb, body, handle?)` so the input-to-FFP
 * transformation and the Theorem 5 response envelope are identical
 * regardless of transport. The CLI/local-MCP experience and the
 * HTTP experience diverge only where they must (auth, billing).
 *
 * Adding a new verb means adding one switch arm here; the HTTP
 * router and MCP tool registration pick it up automatically.
 */

import * as engine from './engine.js'
import type { Envelope } from './envelope.js'

export type DispatchBody = Record<string, unknown>

export interface Diagnostic {
  level: 'error' | 'warn' | 'hint' | 'unknown'
  source?: 'parse' | 'resolve' | 'deontic'
  reading?: string
  message?: string
  suggestion?: string | null
  raw?: string
}

export async function dispatchVerb(
  verb: string,
  body: DispatchBody,
  handle?: number,
): Promise<Envelope<unknown>> {
  const h = handle ?? engine.currentDomainHandle()

  switch (verb) {
    case 'schema': {
      // Full compiled-state projection: nouns + fact types + constraints
      // + state machines + derivation rules. Gated by the engine's
      // `debug-def` cargo feature — same as the MCP tool.
      const raw = engine.system(h, 'debug', '')
      return envelope(safeJson(raw) ?? { raw })
    }

    case 'get': {
      const { noun, id } = body as { noun?: string; id?: string }
      if (!noun) throw new Error('get requires `noun`')
      const key = id ? `get:${noun}` : `list:${noun}`
      const raw = engine.system(h, key, id ?? '')
      return envelope(safeJson(raw) ?? { raw })
    }

    case 'query': {
      const { fact_type, filter } = body as { fact_type?: string; filter?: Record<string, string> }
      if (!fact_type) throw new Error('query requires `fact_type`')
      const filterStr = filter ? JSON.stringify(filter) : ''
      const raw = engine.system(h, `query:${fact_type}`, filterStr)
      return envelope(safeJson(raw) ?? [])
    }

    case 'compile': {
      const { readings } = body as { readings?: string }
      if (readings === undefined) throw new Error('compile requires `readings`')
      const raw = engine.system(h, 'compile', readings)
      const ok = !raw.startsWith('\u22a5')  // ⊥
      return envelope({ ok, result: safeJson(raw) ?? raw })
    }

    case 'check':
    case 'verify': {
      // #202: dispatch on body shape.
      // - body has `readings`: run check (diagnostics path)
      // - body has `domain`: run structural verification against compiled domain state
      // - default (no fields): return current domain health summary
      const { readings, domain } = body as { readings?: string; domain?: string }

      if (readings !== undefined) {
        // readings-string path (original behaviour)
        const raw = engine.system(h, 'check', readings)
        const diagnostics: Diagnostic[] = raw ? raw.split('\n').map(parseDiagLine) : []
        const hasError = diagnostics.some((d) => d.level === 'error')
        return envelope({ ok: !hasError, diagnostics })
      }

      if (domain !== undefined) {
        // structural verification: validate compiled state for the named domain
        const raw = engine.system(h, 'verify', domain)
        const diagnostics: Diagnostic[] = raw ? raw.split('\n').map(parseDiagLine) : []
        const hasError = diagnostics.some((d) => d.level === 'error')
        return envelope({ ok: !hasError, domain, diagnostics })
      }

      // default: current domain health summary
      const debugRaw = engine.system(h, 'debug', '')
      const schema = safeJson(debugRaw) as Record<string, unknown> | null
      const nounCount = schema ? Object.keys((schema as any).nouns ?? {}).length : 0
      const factTypeCount = schema ? Object.keys((schema as any).factTypes ?? {}).length : 0
      const constraintCount = schema ? ((schema as any).constraints ?? []).length : 0
      return envelope({
        ok: true,
        handle: h,
        nounCount,
        factTypeCount,
        constraintCount,
      })
    }

    case 'explain': {
      const { id, noun, fact } = body as { id?: string; noun?: string; fact?: string }
      if (!id) throw new Error('explain requires `id`')
      const audit = safeJson(engine.system(h, 'audit', '0')) ?? []
      const factData = fact
        ? (safeJson(engine.system(h, `query:${fact}`, JSON.stringify(noun ? { [noun]: id } : {}))) ?? [])
        : []
      const trail = Array.isArray(audit)
        ? (audit as Array<Record<string, unknown>>).filter((a) => a?.entity === id || a?.resource === id)
        : []
      return envelope({ entity: id, fact_query: factData, audit_trail: trail })
    }

    case 'apply': {
      // Same body shape as the MCP `apply` tool: create / update /
      // transition. Returns the engine's raw result under `data`.
      const { operation, noun, id, event, fields } = body as {
        operation?: 'create' | 'update' | 'transition'
        noun?: string
        id?: string
        event?: string
        fields?: Record<string, string>
      }
      if (!operation || !noun) throw new Error('apply requires `operation` and `noun`')
      let raw: string
      if (operation === 'transition') {
        if (!id || !event) throw new Error('transition requires `id` and `event`')
        raw = engine.system(h, `transition:${noun}`, `<${id}, ${event}>`)
      } else {
        const pairs = Object.entries(fields ?? {}).map(([k, v]) => `<${k}, ${v}>`).join(', ')
        const idPair = id ? `<id, ${id}>, ` : ''
        const key = operation === 'create' ? `create:${noun}` : `update:${noun}`
        raw = engine.system(h, key, `<${idPair}${pairs}>`)
      }
      return envelope(safeJson(raw) ?? { raw })
    }

    case 'actions': {
      const { noun, id, status } = body as { noun?: string; id?: string; status?: string }
      if (!noun || !id) throw new Error('actions requires `noun` and `id`')
      let resolvedStatus = status ?? ''
      if (!resolvedStatus) {
        const sm = safeJson(engine.system(h, 'get:State Machine', id))
        if (sm && typeof (sm as { currentlyInStatus?: string }).currentlyInStatus === 'string') {
          resolvedStatus = (sm as { currentlyInStatus: string }).currentlyInStatus
        }
      }
      const transitions = safeJson(engine.system(h, `transitions:${noun}`, resolvedStatus)) ?? []
      const entity = safeJson(engine.system(h, `get:${noun}`, id))
      return envelope({
        entity: id,
        noun,
        status: resolvedStatus || null,
        transitions,
        entity_data: entity,
      })
    }

    case 'snapshot':
    case 'rollback':
    case 'snapshots': {
      const input = typeof body.input === 'string' ? body.input : ''
      const raw = engine.system(h, verb, input)
      return envelope({ result: raw })
    }

    default:
      throw new Error(`unknown verb: ${verb}`)
  }
}

/** Wrap a value in the Theorem 5 four-key shape with empty defaults. */
function envelope<T>(data: T): Envelope<T> {
  return { data, _links: {} }
}

function safeJson(raw: string): unknown {
  try {
    const v = JSON.parse(raw)
    return v ?? null
  } catch {
    return null
  }
}

function parseDiagLine(line: string): Diagnostic {
  const m = /^\[(ERROR|WARN|HINT) (parse|resolve|deontic)\] (.*?): (.*?)(?: \(suggestion: (.*?)\))?$/.exec(line)
  if (!m) return { level: 'unknown', raw: line }
  return {
    level: m[1].toLowerCase() as Diagnostic['level'],
    source: m[2] as Diagnostic['source'],
    reading: m[3],
    message: m[4],
    suggestion: m[5] ?? null,
  }
}

/** MCP verbs that accept a simple readings-string body via this dispatcher. */
export const UNIFIED_VERBS = [
  'schema',
  'get',
  'query',
  'apply',
  'actions',
  'compile',
  'explain',
  'check',
  'verify',
  'snapshot',
  'rollback',
  'snapshots',
] as const

export type UnifiedVerb = typeof UNIFIED_VERBS[number]
