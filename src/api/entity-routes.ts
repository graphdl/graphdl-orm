/**
 * Entity-type route handlers — pure async functions that operate on
 * injected stubs (RegistryStub, EntityStub) rather than env, so they
 * are fully testable without the Cloudflare runtime.
 *
 * Uses Promise.allSettled for fan-out resilience (same pattern as materializer).
 */

import { checkDeonticConstraints } from '../worker/deontic-check'
import type { ViolationInput } from '../worker/outcomes'

// ---------------------------------------------------------------------------
// Stub interfaces (same shapes as EntityDB / RegistryDB DO RPCs)
// ---------------------------------------------------------------------------

export interface EntityReadStub {
  get(): Promise<EntityRecord | null>
}

export interface EntityWriteStub {
  put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<{ id: string; version: number }>
  delete(): Promise<{ id: string; deleted: boolean } | null>
}

export interface RegistryReadStub {
  getEntityIds(entityType: string, domainSlug?: string): Promise<string[]>
}

export interface RegistryWriteStub {
  indexEntity(entityType: string, entityId: string, domainSlug?: string): Promise<void>
  deindexEntity(entityType: string, entityId: string): Promise<void>
}

export interface EntityRecord {
  id: string
  type: string
  data: Record<string, unknown>
  version: number
  deletedAt?: string
  createdAt?: string
  updatedAt?: string
  transitions?: TransitionInfo[]
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface ListResult {
  docs: EntityRecord[]
  totalDocs: number
  limit: number
  page: number
  totalPages: number
  hasNextPage: boolean
  hasPrevPage: boolean
  warnings?: string[]
}

export interface PaginationOpts {
  limit?: number
  page?: number
  depth?: number
}

export interface TransitionInfo {
  transitionId: string
  event: string
  targetStatus: string
  targetStatusId: string
}

export interface TransitionOpts {
  /** Callback that returns valid transitions given (definitionId, currentStatusId). */
  getValidTransitions: (definitionId: string, currentStatusId: string) => Promise<TransitionInfo[]>
}

export interface DepthOpts {
  depth?: number
  getStub?: (id: string) => EntityReadStub
  transitionOpts?: TransitionOpts
}

// ---------------------------------------------------------------------------
// populateDepthForEntity
// ---------------------------------------------------------------------------

/**
 * Scan entity data for fields ending in `Id` (e.g., `graphSchemaId`).
 * For each, do a secondary fan-out to resolve the referenced entity.
 * The resolved object is added alongside the original ID field.
 */
export async function populateDepthForEntity(
  entity: { id: string; type: string; data: Record<string, unknown>; version: number },
  depth: number,
  getStub: (id: string) => EntityReadStub,
): Promise<Record<string, unknown>> {
  if (depth <= 0) return entity.data

  const populated = { ...entity.data }
  for (const [key, value] of Object.entries(populated)) {
    if (key.endsWith('Id') && typeof value === 'string') {
      try {
        const refStub = getStub(value)
        const refEntity = await refStub.get()
        if (refEntity) {
          populated[key.replace(/Id$/, '')] = {
            id: refEntity.id,
            ...refEntity.data,
          }
        }
      } catch {
        /* leave as ID if unreachable */
      }
    }
  }
  return populated
}

// ---------------------------------------------------------------------------
// handleListEntities
// ---------------------------------------------------------------------------

/**
 * Fan out to EntityDB DOs for a given entity type + domain.
 * Returns a Payload-style paginated result with an optional warnings array
 * tracking entity IDs whose DOs were unreachable.
 */
export async function handleListEntities(
  type: string,
  domain: string,
  registry: RegistryReadStub,
  getStub: (id: string) => EntityReadStub,
  opts?: PaginationOpts,
): Promise<ListResult> {
  const limit = opts?.limit ?? 100
  const page = opts?.page ?? 1

  // Ask the Registry which entity IDs match this type + domain
  const ids = await registry.getEntityIds(type, domain)

  // Fan out reads with allSettled for resilience
  const settled = await Promise.allSettled(
    ids.map(async (id) => {
      const stub = getStub(id)
      const entity = await stub.get()
      return { id, entity }
    }),
  )

  const docs: EntityRecord[] = []
  const warnings: string[] = []

  for (const result of settled) {
    if (result.status === 'rejected') {
      // Extract the entity ID from the original promise — we need to find it
      // by index since allSettled preserves order
      const idx = settled.indexOf(result)
      warnings.push(ids[idx])
      continue
    }
    const { entity } = result.value
    if (entity && !entity.deletedAt) {
      docs.push(entity)
    }
  }

  // Depth population
  const depth = opts?.depth ?? 0
  if (depth >= 1) {
    await Promise.all(
      docs.map(async (doc) => {
        doc.data = await populateDepthForEntity(doc, depth, getStub)
      }),
    )
  }

  // Pagination
  const totalDocs = docs.length
  const offset = (page - 1) * limit
  const paged = limit > 0 ? docs.slice(offset, offset + limit) : docs
  const totalPages = limit > 0 ? Math.ceil(totalDocs / limit) : 1

  return {
    docs: paged,
    totalDocs,
    limit,
    page,
    totalPages,
    hasNextPage: limit > 0 && offset + limit < totalDocs,
    hasPrevPage: page > 1,
    ...(warnings.length > 0 && { warnings }),
  }
}

// ---------------------------------------------------------------------------
// handleGetEntity
// ---------------------------------------------------------------------------

/**
 * Fetch a single entity from its EntityDB DO.
 * Returns null if not found or soft-deleted.
 * When depth >= 1, resolves Id-suffixed fields to full entity objects.
 * When transitionOpts is provided and entity has _statusId, includes valid transitions.
 */
export async function handleGetEntity(
  stub: EntityReadStub,
  opts?: DepthOpts,
): Promise<EntityRecord | null> {
  const entity = await stub.get()
  if (!entity || entity.deletedAt) return null

  const depth = opts?.depth ?? 0
  if (depth >= 1 && opts?.getStub) {
    entity.data = await populateDepthForEntity(entity, depth, opts.getStub)
  }

  // Include valid transitions when entity has a state machine
  if (
    opts?.transitionOpts &&
    typeof entity.data._statusId === 'string' &&
    typeof entity.data._stateMachineDefinition === 'string'
  ) {
    const transitions = await opts.transitionOpts.getValidTransitions(
      entity.data._stateMachineDefinition,
      entity.data._statusId,
    )
    entity.transitions = transitions
  }

  return entity
}

// ---------------------------------------------------------------------------
// handleCreateEntity
// ---------------------------------------------------------------------------

/**
 * Options for deontic constraint checking on entity creation.
 * When provided, constraints with modality='Deontic' are evaluated before
 * the write is committed.
 */
export interface DeonticOpts {
  /** Registry stub that supports getEntityIds (for loading constraints). */
  registryRead: RegistryReadStub
  /** Stub factory for reading Constraint, ConstraintSpan, Role, and Noun entities. */
  getReadStub: (id: string) => EntityReadStub
}

export interface CreateEntityResult {
  id: string
  version: number
  /** Present when deontic constraints produced warnings (write was allowed). */
  warnings?: ViolationInput[]
  /** Present when deontic constraints blocked the write (allowed=false). */
  rejected?: true
  violations?: ViolationInput[]
}

/**
 * Create an entity in its EntityDB DO and index it in the Registry.
 * Generates a UUID for the new entity.
 *
 * When `deonticOpts` is provided, deontic constraints are evaluated first:
 * - If violations with severity='error' are found, the write is rejected
 *   (no entity created) and `rejected: true` is returned with violations.
 * - If only severity='warning' violations exist, the entity is created
 *   and warnings are returned alongside the result.
 */
export async function handleCreateEntity(
  type: string,
  domain: string,
  data: Record<string, unknown>,
  getStub: (id: string) => EntityWriteStub,
  registry: RegistryWriteStub,
  deonticOpts?: DeonticOpts,
): Promise<CreateEntityResult> {
  // Deontic constraint check (when configured)
  if (deonticOpts) {
    const check = await checkDeonticConstraints(
      type, data, domain,
      deonticOpts.registryRead,
      deonticOpts.getReadStub,
    )

    if (!check.allowed) {
      return {
        id: '',
        version: 0,
        rejected: true,
        violations: check.violations,
      }
    }

    // Warnings present but allowed — create entity and attach warnings
    if (check.violations.length > 0) {
      const id = crypto.randomUUID()
      const stub = getStub(id)
      const result = await stub.put({ id, type, data })
      await registry.indexEntity(type, id, domain)
      return { id, version: result.version, warnings: check.violations }
    }
  }

  const id = crypto.randomUUID()
  const stub = getStub(id)
  const result = await stub.put({ id, type, data })
  await registry.indexEntity(type, id, domain)
  return { id, version: result.version }
}

// ---------------------------------------------------------------------------
// handleDeleteEntity
// ---------------------------------------------------------------------------

/**
 * Soft-delete an entity via its EntityDB DO and deindex it from the Registry.
 * Returns null if entity not found.
 */
export async function handleDeleteEntity(
  id: string,
  stub: EntityWriteStub,
  registry: RegistryWriteStub,
  type: string,
): Promise<{ id: string; deleted: boolean } | null> {
  const result = await stub.delete()
  if (!result) return null
  await registry.deindexEntity(type, id)
  return result
}
