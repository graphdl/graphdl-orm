/**
 * RegistryDB — pure functions for the global registry schema.
 *
 * The registry maps domain slugs to their Durable Object IDs and
 * indexes nouns so cross-domain lookups can resolve which domain
 * owns a given noun type.
 */

import { DurableObject } from 'cloudflare:workers'

// =========================================================================
// Types
// =========================================================================

import type { SqlLike } from './sql-like'
export type { SqlLike } from './sql-like'

// =========================================================================
// Schema initialisation
// =========================================================================

/**
 * Creates the 3 registry tables: domains, noun_index, entity_index.
 */
export function initRegistrySchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS domains (
  domain_slug TEXT PRIMARY KEY,
  domain_do_id TEXT NOT NULL,
  domain_uuid TEXT,
  visibility TEXT NOT NULL DEFAULT 'private'
)`)
  // Migration: add domain_uuid if table was created before this column existed
  try { sql.exec('ALTER TABLE domains ADD COLUMN domain_uuid TEXT') } catch { /* already exists */ }

  sql.exec(`CREATE TABLE IF NOT EXISTS noun_index (
  noun_name TEXT NOT NULL,
  domain_slug TEXT NOT NULL,
  PRIMARY KEY (noun_name, domain_slug)
)`)

  sql.exec(`CREATE TABLE IF NOT EXISTS entity_index (
  noun_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  domain_slug TEXT,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (noun_type, entity_id)
)`)
  // Migration: add domain_slug if table was created before this column existed
  try { sql.exec('ALTER TABLE entity_index ADD COLUMN domain_slug TEXT') } catch { /* already exists */ }
}

// =========================================================================
// Domain registration
// =========================================================================

/**
 * Upsert a domain into the domains table.
 * INSERT OR REPLACE ensures idempotency — re-registering the same slug
 * simply updates the DO ID and visibility.
 */
export function registerDomain(sql: SqlLike, slug: string, doId: string, visibility: string = 'private', uuid?: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO domains (domain_slug, domain_do_id, visibility, domain_uuid) VALUES (?, ?, ?, ?)`,
    slug,
    doId,
    visibility,
    uuid || null,
  )
}

/** Look up a domain slug by its UUID. */
export function resolveSlugByUUID(sql: SqlLike, uuid: string): string | null {
  const rows = sql.exec('SELECT domain_slug FROM domains WHERE domain_uuid = ? LIMIT 1', uuid).toArray()
  return rows.length ? rows[0].domain_slug as string : null
}

// =========================================================================
// Noun indexing
// =========================================================================

/**
 * Upsert a noun-to-domain mapping.
 * INSERT OR REPLACE ensures idempotency — indexing the same noun/domain
 * pair twice is a no-op.
 */
export function indexNoun(sql: SqlLike, nounName: string, domainSlug: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO noun_index (noun_name, domain_slug) VALUES (?, ?)`,
    nounName,
    domainSlug,
  )
}

// =========================================================================
// Entity indexing
// =========================================================================

/**
 * Upsert an entity into the entity_index with deleted=0.
 * INSERT OR REPLACE ensures idempotency and also "re-indexes" a
 * previously soft-deleted entity.
 */
export function indexEntity(sql: SqlLike, nounType: string, entityId: string, domainSlug?: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO entity_index (noun_type, entity_id, domain_slug, deleted) VALUES (?, ?, ?, ?)`,
    nounType,
    entityId,
    domainSlug || null,
    0,
  )
}

/**
 * Soft-delete an entity from the index by setting deleted=1.
 */
export function deindexEntity(sql: SqlLike, nounType: string, entityId: string): void {
  sql.exec(
    `UPDATE entity_index SET deleted=1 WHERE noun_type=? AND entity_id=?`,
    nounType,
    entityId,
  )
}

/**
 * Returns all non-deleted entity IDs for a given noun type.
 */
export function getEntityIds(sql: SqlLike, nounType: string, domainSlug?: string): string[] {
  const rows = domainSlug
    ? sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=? AND domain_slug=? AND deleted=0`, nounType, domainSlug).toArray()
    : sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=? AND deleted=0`, nounType).toArray()

  return rows.map((row: any) => row.entity_id)
}

/**
 * Soft-delete all entity index entries for a given domain.
 * Returns the count of rows affected.
 */
export function deindexEntitiesForDomain(sql: SqlLike, domainSlug: string): number {
  const before = sql.exec(
    `SELECT count(*) as c FROM entity_index WHERE domain_slug=? AND deleted=0`,
    domainSlug,
  ).toArray()[0] as any
  sql.exec(
    `UPDATE entity_index SET deleted=1 WHERE domain_slug=? AND deleted=0`,
    domainSlug,
  )
  return before?.c || 0
}

/**
 * Remove noun_index entries for a given domain.
 * Returns the count of rows removed.
 */
export function deindexNounsForDomain(sql: SqlLike, domainSlug: string): number {
  const before = sql.exec(
    `SELECT count(*) as c FROM noun_index WHERE domain_slug=?`,
    domainSlug,
  ).toArray()[0] as any
  sql.exec(`DELETE FROM noun_index WHERE domain_slug=?`, domainSlug)
  return before?.c || 0
}

/**
 * Get all non-deleted entity IDs across all types for a given domain.
 * Returns array of { nounType, entityId } for fan-out deletion.
 */
export function getAllEntityIdsForDomain(sql: SqlLike, domainSlug: string): Array<{ nounType: string; entityId: string }> {
  const rows = sql.exec(
    `SELECT noun_type, entity_id FROM entity_index WHERE domain_slug=? AND deleted=0`,
    domainSlug,
  ).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, entityId: row.entity_id }))
}

/**
 * Get all non-deleted entity IDs across all types and domains.
 * Returns array of { nounType, entityId } for fan-out deletion.
 */
export function getAllEntityIds(sql: SqlLike): Array<{ nounType: string; entityId: string }> {
  const rows = sql.exec(
    `SELECT noun_type, entity_id FROM entity_index WHERE deleted=0`,
  ).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, entityId: row.entity_id }))
}

/**
 * Wipe all data from all registry tables (for testing/reset).
 */
export function wipeAllRegistryData(sql: SqlLike): void {
  sql.exec(`DELETE FROM entity_index`)
  sql.exec(`DELETE FROM noun_index`)
  sql.exec(`DELETE FROM domains`)
}

/**
 * Get entity counts grouped by noun type (optionally filtered by domain).
 */
export function getEntityCounts(sql: SqlLike, domainSlug?: string): Array<{ nounType: string; count: number }> {
  const rows = domainSlug
    ? sql.exec(
        `SELECT noun_type, count(*) as c FROM entity_index WHERE domain_slug=? AND deleted=0 GROUP BY noun_type ORDER BY noun_type`,
        domainSlug,
      ).toArray()
    : sql.exec(
        `SELECT noun_type, count(*) as c FROM entity_index WHERE deleted=0 GROUP BY noun_type ORDER BY noun_type`,
      ).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, count: row.c }))
}

// =========================================================================
// Noun resolution
// =========================================================================

/**
 * Returns all registered domain slugs.
 */
export function listDomains(sql: SqlLike): string[] {
  const rows = sql.exec('SELECT domain_slug FROM domains').toArray()
  return rows.map((row: any) => row.domain_slug)
}

/**
 * Join noun_index with domains to find which domain owns the given noun.
 * Returns the first match or null if the noun is not indexed.
 */
export function resolveNounInRegistry(sql: SqlLike, nounName: string): { domainSlug: string; domainDoId: string } | null {
  const rows = sql.exec(
    `SELECT n.domain_slug, d.domain_do_id
     FROM noun_index n
     JOIN domains d ON n.domain_slug = d.domain_slug
     WHERE n.noun_name = ?`,
    nounName,
  ).toArray()

  if (rows.length === 0) return null

  const row = rows[0] as Record<string, any>
  return {
    domainSlug: row.domain_slug,
    domainDoId: row.domain_do_id,
  }
}

/**
 * Returns all distinct noun names from the noun_index.
 * These are the nouns seeded from readings — the source of truth for what entity types exist.
 */
export function getRegisteredNouns(sql: SqlLike): string[] {
  const rows = sql.exec('SELECT DISTINCT noun_name FROM noun_index ORDER BY noun_name').toArray()
  return rows.map((row: any) => row.noun_name)
}

// =========================================================================
// Durable Object class
// =========================================================================

export class RegistryDB extends DurableObject {
  private initialized = false

  private ensureInit(): void {
    if (this.initialized) return
    initRegistrySchema(this.ctx.storage.sql)
    this.initialized = true
  }

  async registerDomain(slug: string, doId: string, visibility?: string, uuid?: string): Promise<void> {
    this.ensureInit()
    registerDomain(this.ctx.storage.sql, slug, doId, visibility, uuid)
  }

  async resolveSlugByUUID(uuid: string): Promise<string | null> {
    this.ensureInit()
    return resolveSlugByUUID(this.ctx.storage.sql, uuid)
  }

  async indexNoun(nounName: string, domainSlug: string): Promise<void> {
    this.ensureInit()
    indexNoun(this.ctx.storage.sql, nounName, domainSlug)
  }

  async resolveNoun(nounName: string): Promise<{ domainSlug: string; domainDoId: string } | null> {
    this.ensureInit()
    return resolveNounInRegistry(this.ctx.storage.sql, nounName)
  }

  async listDomains(): Promise<string[]> {
    this.ensureInit()
    return listDomains(this.ctx.storage.sql)
  }

  async indexEntity(nounType: string, entityId: string, domainSlug?: string): Promise<void> {
    this.ensureInit()
    indexEntity(this.ctx.storage.sql, nounType, entityId, domainSlug)
  }

  async deindexEntity(nounType: string, entityId: string): Promise<void> {
    this.ensureInit()
    deindexEntity(this.ctx.storage.sql, nounType, entityId)
  }

  async getEntityIds(nounType: string, domainSlug?: string): Promise<string[]> {
    this.ensureInit()
    return getEntityIds(this.ctx.storage.sql, nounType, domainSlug)
  }

  async deindexEntitiesForDomain(domainSlug: string): Promise<number> {
    this.ensureInit()
    return deindexEntitiesForDomain(this.ctx.storage.sql, domainSlug)
  }

  async deindexNounsForDomain(domainSlug: string): Promise<number> {
    this.ensureInit()
    return deindexNounsForDomain(this.ctx.storage.sql, domainSlug)
  }

  async getAllEntityIdsForDomain(domainSlug: string): Promise<Array<{ nounType: string; entityId: string }>> {
    this.ensureInit()
    return getAllEntityIdsForDomain(this.ctx.storage.sql, domainSlug)
  }

  async getAllEntityIds(): Promise<Array<{ nounType: string; entityId: string }>> {
    this.ensureInit()
    return getAllEntityIds(this.ctx.storage.sql)
  }

  async wipeAll(): Promise<void> {
    this.ensureInit()
    wipeAllRegistryData(this.ctx.storage.sql)
  }

  async getEntityCounts(domainSlug?: string): Promise<Array<{ nounType: string; count: number }>> {
    this.ensureInit()
    return getEntityCounts(this.ctx.storage.sql, domainSlug)
  }

  async getRegisteredNouns(): Promise<string[]> {
    this.ensureInit()
    return getRegisteredNouns(this.ctx.storage.sql)
  }

  /**
   * Materialize a batch of entities: fan out to EntityDB DOs + index in Registry.
   * Runs INSIDE the Registry DO — no subrequest limit on DO-to-DO calls.
   *
   * Idempotent: for each domain in the batch, existing entities of the same
   * types are deindexed first. Re-seeding the same readings replaces metamodel
   * entities without duplicates. Instance entities of other types are preserved.
   */
  async materializeBatch(
    entities: Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }>,
  ): Promise<{ materialized: number; failed: string[] }> {
    this.ensureInit()
    const entityDB = (this.env as any).ENTITY_DB as DurableObjectNamespace

    // Deindex existing entities of the same types+domains (idempotent replace)
    const domainTypes = new Map<string, Set<string>>()
    for (const entity of entities) {
      if (!entity.domain) continue
      if (!domainTypes.has(entity.domain)) domainTypes.set(entity.domain, new Set())
      domainTypes.get(entity.domain)!.add(entity.type)
    }
    for (const [domain, types] of domainTypes) {
      for (const type of types) {
        this.ctx.storage.sql.exec(
          'UPDATE entity_index SET deleted=1 WHERE domain_slug=? AND noun_type=? AND deleted=0',
          domain, type,
        )
      }
      // Re-index nouns for this domain (will be repopulated from batch)
      this.ctx.storage.sql.exec('DELETE FROM noun_index WHERE domain_slug=?', domain)
    }

    const results = await Promise.allSettled(
      entities.map(async (entity) => {
        const stub = entityDB.get(entityDB.idFromName(entity.id)) as any
        await stub.put({ id: entity.id, type: entity.type, data: entity.data })
        indexEntity(this.ctx.storage.sql, entity.type, entity.id, entity.domain)
        if (entity.type === 'Noun') {
          const name = entity.data.name as string
          if (name) indexNoun(this.ctx.storage.sql, name, entity.domain)
        }
      }),
    )

    let materialized = 0
    const failed: string[] = []
    for (let i = 0; i < results.length; i++) {
      if (results[i].status === 'fulfilled') materialized++
      else failed.push(entities[i].id)
    }

    return { materialized, failed }
  }
}
