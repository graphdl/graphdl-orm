/**
 * AREST engine — SYSTEM:x = ⟨o, D'⟩
 *
 * Three WASM exports: parse_and_compile, release, system.
 * All operations are applications of the system function.
 * This layer is ↑ (fetch from DO) and ↓ (store to DO), not computation.
 */

import { initSync, parse_and_compile, release, system } from '../../crates/arest/pkg/arest.js'
import wasmModule from '../../crates/arest/pkg/arest_bg.wasm'

let _init = false
function ensureWasm() { if (!_init) { initSync({ module: wasmModule }); _init = true } }

let _h = -1
function h(handle?: number): number { return handle !== undefined && handle >= 0 ? handle : _h }

// ── The three operations, re-exported for callers ───────────────────

export { system }

export function currentDomainHandle(): number { return _h }

export function release_domain(handle: number): void { ensureWasm(); release(handle) }

export function compileDomainReadings(domain: string, readings: string): number {
  ensureWasm()
  return parse_and_compile(JSON.stringify([[domain, readings]]))
}

export async function loadDomainSchema(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<number> {
  ensureWasm()
  const defsCell = await getStub(`defs:${domainSlug}`).get().catch(() => null)
  const readings = defsCell?.data?.readings
  if (!readings) return -1
  _h = parse_and_compile(JSON.stringify([[domainSlug, readings]]))
  return _h
}

// ── Applications of SYSTEM ──────────────────────────────────────────

export function evaluateConstraints(text: string, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'evaluate', JSON.stringify({ text, population })))
}

export function forwardChain(population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'forward_chain', population))
}

export function getTransitions(noun: string, status: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), `transitions:${noun}`, status))
}

export function applyCommand(command: any, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'apply', JSON.stringify({ command, population })))
}

export function querySchema(schemaId: string, targetRole: number, filterBindings: any, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'query', JSON.stringify({ schemaId, targetRole, filterBindings, population })))
}

export function getNounSchemas(noun: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'noun_schemas', noun))
}

export function computeRMAP(handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'rmap', ''))
}

export function parseReadings(markdown: string, domain: string) {
  ensureWasm()
  return JSON.parse(system(0, 'parse', JSON.stringify({ markdown, domain })))
}

export function parseReadingsWithNouns(markdown: string, domain: string, existingNounsJson: string) {
  ensureWasm()
  return JSON.parse(system(0, 'parse_with_nouns', JSON.stringify({ markdown, domain, nouns: JSON.parse(existingNounsJson) })))
}

// ── Population from EntityDB (↑FILE:D) ──────────────────────────────

export async function buildPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  const counts = await registry.getEntityCounts(domainSlug) as Array<{ nounType: string; count: number }>
  const facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}
  const schemaTypes = new Set(['Noun', 'Reading', 'Graph Schema', 'Role', 'Constraint', 'CompiledSchema', 'Derivation Rule', 'State Machine Definition', 'Status', 'Transition', 'External System', 'Instance Fact'])
  const entitySettled = await Promise.allSettled(
    counts.filter(({ nounType }) => !schemaTypes.has(nounType)).flatMap(({ nounType }) =>
      registry.getEntityIds(nounType, domainSlug).then((ids: string[]) =>
        Promise.allSettled(ids.map(async (id: string) => {
          const entity = await getStub(id).get()
          return entity ? { ...entity, nounType } : null
        }))
      )
    )
  )
  entitySettled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled')
    .flatMap(r => r.value)
    .filter((r: any): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
    .map((r: any) => r.value)
    .forEach((entity: any) => {
      Object.entries(entity.data || {}).forEach(([field, value]) => {
        if (field.startsWith('_')) return
        if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') return
        const ftId = `${entity.nounType || entity.type}_has_${field}`
        const list = facts[ftId] || []
        list.push({ factTypeId: ftId, bindings: [[entity.nounType || entity.type, entity.id], [field, String(value)]] })
        facts[ftId] = list
      })
    })
  return JSON.stringify({ facts })
}

export async function loadDomainAndPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  await loadDomainSchema(registry, getStub, domainSlug)
  return buildPopulation(registry, getStub, domainSlug)
}

