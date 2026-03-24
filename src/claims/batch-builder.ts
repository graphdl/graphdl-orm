import type { BatchEntity } from '../batch-wal'

/**
 * BatchBuilder — a mutable accumulator that collects metamodel entities
 * in memory, producing a batch suitable for WAL commit.
 *
 * Replaces direct `db.createInCollection()` calls during claims ingestion.
 * Entities are accumulated synchronously, then flushed as a single batch.
 */
export class BatchBuilder {
  private entities: BatchEntity[] = []
  private index = new Map<string, string>() // "Type:key:value" → entity id

  constructor(readonly domain: string) {}

  /**
   * Add an entity to the batch. Returns the entity's id.
   * If no id is provided, one is generated via crypto.randomUUID().
   */
  addEntity(type: string, data: Record<string, unknown>, id?: string): string {
    const entityId = id || crypto.randomUUID()
    this.entities.push({ id: entityId, type, domain: this.domain, data })
    return entityId
  }

  /**
   * Find-or-add: if an entity of the given type with keyField=keyValue
   * already exists in this batch, return its id. Otherwise add it.
   */
  ensureEntity(type: string, keyField: string, keyValue: string, data: Record<string, unknown>): string {
    const lookupKey = `${type}:${keyField}:${keyValue}`
    const existing = this.index.get(lookupKey)
    if (existing) return existing
    const id = this.addEntity(type, data)
    this.index.set(lookupKey, id)
    return id
  }

  /** Number of entities accumulated so far. */
  get entityCount(): number {
    return this.entities.length
  }

  /** Produce the batch payload (defensive copy of entities array). */
  toBatch(): { domain: string; entities: BatchEntity[] } {
    return { domain: this.domain, entities: [...this.entities] }
  }
}
