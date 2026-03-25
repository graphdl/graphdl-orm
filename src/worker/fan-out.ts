/**
 * Shared fan-out helpers for loading entities from Registry+EntityDB.
 *
 * Used by state-machine.ts and cascade-transition.ts.
 */

import type { RegistryReadStub, EntityReadStub, EntityRecord } from '../api/entity-routes'

/** Callback to obtain an EntityDB DO stub for a given entity ID. */
export type GetStubFn = (id: string) => EntityReadStub & { patch?(data: any): Promise<any> }

/**
 * Load all entities of a given type from Registry+EntityDB fan-out.
 * Returns non-deleted entities with their full data blobs.
 */
export async function loadEntities(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  entityType: string,
  domain?: string,
): Promise<EntityRecord[]> {
  const ids = await registry.getEntityIds(entityType, domain)
  const settled = await Promise.allSettled(
    ids.map(async (id) => {
      const stub = getStub(id)
      return stub.get()
    }),
  )
  const results: EntityRecord[] = []
  for (const r of settled) {
    if (r.status === 'fulfilled' && r.value && !r.value.deletedAt) {
      results.push(r.value)
    }
  }
  return results
}

/**
 * Load a single entity by ID from its EntityDB DO.
 */
export async function loadEntity(
  getStub: GetStubFn,
  id: string,
): Promise<EntityRecord | null> {
  try {
    const stub = getStub(id)
    const entity = await stub.get()
    return entity && !entity.deletedAt ? entity : null
  } catch {
    return null
  }
}
