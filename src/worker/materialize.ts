/**
 * Batch materializer — fans out BatchEntity[] to individual EntityDB
 * Durable Objects and indexes them in RegistryDB.
 *
 * Uses Promise.allSettled so one failure doesn't abort the whole batch.
 * Failed entity IDs are tracked for retry.
 */

import type { BatchEntity } from '../batch-wal'

// ---------------------------------------------------------------------------
// Stub interfaces (injected at call site)
// ---------------------------------------------------------------------------

export interface EntityStub {
  put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<{ id: string; version: number }>
}

export interface RegistryStub {
  indexEntity(entityType: string, entityId: string, domainSlug?: string): Promise<void>
  indexNoun(nounName: string, domainSlug: string): Promise<void>
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

export interface MaterializeResult {
  materialized: number
  failed: string[] // entity IDs that failed
}

// ---------------------------------------------------------------------------
// materializeBatch
// ---------------------------------------------------------------------------

const BATCH_SIZE = 50

/**
 * Materialize a batch of entities by fanning out to EntityDB DOs
 * and indexing each one in RegistryDB.
 *
 * - Processes entities in chunks of BATCH_SIZE (50) for bounded concurrency.
 * - Uses Promise.allSettled so individual failures don't abort the batch.
 * - Only Noun entities trigger indexNoun; all entities trigger indexEntity.
 */
export async function materializeBatch(
  entities: BatchEntity[],
  getEntityStub: (id: string) => EntityStub,
  registry: RegistryStub,
): Promise<MaterializeResult> {
  let materialized = 0
  const failed: string[] = []

  for (let i = 0; i < entities.length; i += BATCH_SIZE) {
    const chunk = entities.slice(i, i + BATCH_SIZE)
    const results = await Promise.allSettled(
      chunk.map(async (entity) => {
        const stub = getEntityStub(entity.id)
        await stub.put({ id: entity.id, type: entity.type, data: entity.data })
        await registry.indexEntity(entity.type, entity.id, entity.domain)
        if (entity.type === 'Noun') {
          const name = entity.data.name as string
          if (name) await registry.indexNoun(name, entity.domain)
        }
      }),
    )
    for (let j = 0; j < results.length; j++) {
      if (results[j].status === 'fulfilled') materialized++
      else failed.push(chunk[j].id)
    }
  }

  return { materialized, failed }
}
