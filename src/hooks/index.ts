/**
 * Collection write hooks — deterministic parse-on-write.
 *
 * Hooks run in the Worker context (not inside the DO), receiving the
 * DurableObjectStub. They call db.createInCollection(), findInCollection(),
 * etc. via RPC — same execution context as the POST handler.
 */

export interface HookResult {
  created: Record<string, any[]>
  warnings: string[]
}

export type AfterCreateHook = (
  db: any, // DurableObjectStub at runtime, typed as any for compatibility with GraphDLDBLike
  doc: Record<string, any>,
  context: HookContext,
) => Promise<HookResult>

export interface HookContext {
  domainId: string
  allNouns: Array<{ name: string; id: string }>
  /** When true, constraint rejection is deferred to end of batch */
  batch?: boolean
  /** Accumulator for deferred constraints in batch mode */
  deferred?: Array<{ data: Record<string, any>; error: string }>
}

export const COLLECTION_HOOKS: Record<string, AfterCreateHook> = {}

/** Merge two HookResults, combining created arrays and warnings. */
export function mergeResults(a: HookResult, b: HookResult): HookResult {
  const created = { ...a.created }
  for (const [key, docs] of Object.entries(b.created)) {
    created[key] = [...(created[key] || []), ...docs]
  }
  return { created, warnings: [...a.warnings, ...b.warnings] }
}

/** Empty result constant (frozen to prevent accidental mutation). */
export const EMPTY_RESULT: HookResult = Object.freeze({ created: {}, warnings: [] }) as HookResult

/**
 * Create a record and run its afterCreate hook if one exists.
 * Called by the POST handler and by other hooks for recursive composition.
 */
export async function createWithHook(
  db: any,
  collection: string,
  data: Record<string, any>,
  context: HookContext,
): Promise<{ doc: Record<string, any>; hookResult: HookResult }> {
  // Pre-process: constraints with text need kind before insert (NOT NULL)
  if (collection === 'constraints' && data.text && !data.kind) {
    const { parseConstraintText } = await import('./parse-constraint')
    const parsed = parseConstraintText(data.text)
    if (parsed && parsed.length > 0) {
      data.kind = parsed[0].kind
      data.modality = data.modality || parsed[0].modality
    } else {
      data.kind = 'UC' // default
    }
  }

  // Idempotent: find-or-create for state machine definitions (prevent duplicates)
  if (collection === 'state-machine-definitions' && data.noun) {
    const existing = await db.findInCollection('state-machine-definitions', {
      noun: { equals: data.noun },
      ...(data.domain && { domain: { equals: data.domain } }),
    }, { limit: 1 })
    if (existing.docs.length) {
      return { doc: existing.docs[0], hookResult: EMPTY_RESULT }
    }
  }

  const doc = await db.createInCollection(collection, data)
  const hook = COLLECTION_HOOKS[collection]
  if (hook) {
    // Merge original data into doc so hooks see non-column fields (e.g. transitions array)
    const hookResult = await hook(db, { ...data, ...doc }, context)
    return { doc, hookResult }
  }
  return { doc, hookResult: EMPTY_RESULT }
}

/**
 * Refresh the allNouns list from the database.
 * Called before hook execution to ensure nouns created by prior hooks are visible.
 */
export async function refreshNouns(db: any, domainId: string): Promise<Array<{ name: string; id: string }>> {
  const result = await db.findInCollection('nouns', { domain_id: { equals: domainId } }, { limit: 10000 })
  return result.docs.map((n: any) => ({ name: n.name, id: n.id }))
}

/**
 * Find-or-create pattern. Returns existing doc if found, creates if not.
 */
export async function ensure(
  db: any,
  collection: string,
  where: Record<string, any>,
  data: Record<string, any>,
): Promise<{ doc: Record<string, any>; created: boolean }> {
  const result = await db.findInCollection(collection, where, { limit: 1 })
  if (result.docs.length > 0) {
    return { doc: result.docs[0], created: false }
  }
  const doc = await db.createInCollection(collection, data)
  return { doc, created: true }
}

import { nounAfterCreate } from './nouns'
COLLECTION_HOOKS['nouns'] = nounAfterCreate

import { readingAfterCreate } from './readings'
COLLECTION_HOOKS['readings'] = readingAfterCreate

import { smDefinitionAfterCreate } from './state-machines'
COLLECTION_HOOKS['state-machine-definitions'] = smDefinitionAfterCreate

import { constraintAfterCreate } from './constraints'
COLLECTION_HOOKS['constraints'] = constraintAfterCreate
