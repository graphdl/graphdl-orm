/**
 * RegistryDB — the FILE cell's directory.
 *
 * One DO instance per scope (app / org / global). Holds cross-entity
 * state that would otherwise require a global lock: the population
 * index, compiled schema cache, domain → External System secret
 * references, federation configs. Per-entity facts live in EntityDB
 * (one DO per entity id); the two DOs split so entity writes don't
 * contend against registry reads.
 *
 * The population P = ↑FILE : D is the named set of all entity cells.
 * The registry knows which cells exist and where — it IS the population index.
 *
 * No soft-delete. Delete = remove from population. If a cell doesn't
 * exist in the registry, it's not in the population.
 */

import { DurableObject } from 'cloudflare:workers'

import type { SqlLike } from './sql-like'
export type { SqlLike } from './sql-like'

// =========================================================================
// Schema initialisation
// =========================================================================

export function initRegistrySchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS domains (
  domain_slug TEXT PRIMARY KEY,
  domain_do_id TEXT NOT NULL,
  domain_uuid TEXT,
  visibility TEXT NOT NULL DEFAULT 'private'
)`)
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
  PRIMARY KEY (noun_type, entity_id)
)`)

  // Migration: drop deleted column if it exists (we use hard deletes now)
  // SQLite doesn't support DROP COLUMN before 3.35.0, so we just ignore it
  // and never query it. New tables won't have it.
  try { sql.exec('ALTER TABLE entity_index ADD COLUMN domain_slug TEXT') } catch { /* already exists */ }

  initSnapshotSchema(sql)
}

// =========================================================================
// Snapshot storage (freeze/thaw ↔ DO persistence, #203)
// =========================================================================

export function initSnapshotSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS snapshots (
    label TEXT PRIMARY KEY,
    frozen_hex TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    byte_length INTEGER NOT NULL DEFAULT 0
  )`)
}

export function storeSnapshot(sql: SqlLike, label: string, frozenHex: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO snapshots (label, frozen_hex, byte_length) VALUES (?, ?, ?)`,
    label, frozenHex, Math.floor(frozenHex.length / 2),
  )
}

export function fetchSnapshot(sql: SqlLike, label: string): string | null {
  const rows = sql.exec(`SELECT frozen_hex FROM snapshots WHERE label = ?`, label).toArray()
  return rows.length ? (rows[0] as any).frozen_hex : null
}

export function listSnapshots(sql: SqlLike): Array<{ label: string; createdAt: string; byteLength: number }> {
  return sql.exec(`SELECT label, created_at, byte_length FROM snapshots ORDER BY created_at DESC`).toArray()
    .map((r: any) => ({ label: r.label, createdAt: r.created_at, byteLength: r.byte_length }))
}

export function deleteSnapshot(sql: SqlLike, label: string): boolean {
  const before = sql.exec(`SELECT count(*) as c FROM snapshots WHERE label = ?`, label).toArray()[0] as any
  sql.exec(`DELETE FROM snapshots WHERE label = ?`, label)
  return (before?.c ?? 0) > 0
}

// =========================================================================
// Domain registration
// =========================================================================

export function registerDomain(sql: SqlLike, slug: string, doId: string, visibility: string = 'private', uuid?: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO domains (domain_slug, domain_do_id, visibility, domain_uuid) VALUES (?, ?, ?, ?)`,
    slug, doId, visibility, uuid || null,
  )
}

export function resolveSlugByUUID(sql: SqlLike, uuid: string): string | null {
  const rows = sql.exec('SELECT domain_slug FROM domains WHERE domain_uuid = ? LIMIT 1', uuid).toArray()
  return rows.length ? rows[0].domain_slug as string : null
}

// =========================================================================
// Noun indexing
// =========================================================================

export function indexNoun(sql: SqlLike, nounName: string, domainSlug: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO noun_index (noun_name, domain_slug) VALUES (?, ?)`,
    nounName, domainSlug,
  )
}

// =========================================================================
// Entity indexing — hard deletes, no soft-delete flags
// =========================================================================

/** Index an entity in the population. */
export function indexEntity(sql: SqlLike, nounType: string, entityId: string, domainSlug?: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO entity_index (noun_type, entity_id, domain_slug) VALUES (?, ?, ?)`,
    nounType, entityId, domainSlug || null,
  )
}

/** Remove an entity from the population index (hard delete). */
export function deindexEntity(sql: SqlLike, nounType: string, entityId: string): void {
  sql.exec(
    `DELETE FROM entity_index WHERE noun_type=? AND entity_id=?`,
    nounType, entityId,
  )
}

/** Returns all entity IDs for a given noun type. */
export function getEntityIds(sql: SqlLike, nounType: string, domainSlug?: string): string[] {
  const rows = domainSlug
    ? sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=? AND domain_slug=?`, nounType, domainSlug).toArray()
    : sql.exec(`SELECT entity_id FROM entity_index WHERE noun_type=?`, nounType).toArray()
  return rows.map((row: any) => row.entity_id)
}

/** Remove all entity index entries for a given domain (hard delete). */
export function deindexEntitiesForDomain(sql: SqlLike, domainSlug: string): number {
  const before = sql.exec(
    `SELECT count(*) as c FROM entity_index WHERE domain_slug=?`,
    domainSlug,
  ).toArray()[0] as any
  sql.exec(`DELETE FROM entity_index WHERE domain_slug=?`, domainSlug)
  return before?.c || 0
}

/** Remove noun_index entries for a given domain. */
export function deindexNounsForDomain(sql: SqlLike, domainSlug: string): number {
  const before = sql.exec(
    `SELECT count(*) as c FROM noun_index WHERE domain_slug=?`,
    domainSlug,
  ).toArray()[0] as any
  sql.exec(`DELETE FROM noun_index WHERE domain_slug=?`, domainSlug)
  return before?.c || 0
}

/** Get all entity IDs for a given domain. */
export function getAllEntityIdsForDomain(sql: SqlLike, domainSlug: string): Array<{ nounType: string; entityId: string }> {
  const rows = sql.exec(
    `SELECT noun_type, entity_id FROM entity_index WHERE domain_slug=?`,
    domainSlug,
  ).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, entityId: row.entity_id }))
}

/** Get all entity IDs across all types and domains. */
export function getAllEntityIds(sql: SqlLike): Array<{ nounType: string; entityId: string }> {
  const rows = sql.exec(`SELECT noun_type, entity_id FROM entity_index`).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, entityId: row.entity_id }))
}

/** Wipe all data from all registry tables. */
export function wipeAllRegistryData(sql: SqlLike): void {
  sql.exec(`DELETE FROM entity_index`)
  sql.exec(`DELETE FROM noun_index`)
  sql.exec(`DELETE FROM domains`)
}

/** Get entity counts grouped by noun type. */
export function getEntityCounts(sql: SqlLike, domainSlug?: string): Array<{ nounType: string; count: number }> {
  const rows = domainSlug
    ? sql.exec(
        `SELECT noun_type, count(*) as c FROM entity_index WHERE domain_slug=? GROUP BY noun_type ORDER BY noun_type`,
        domainSlug,
      ).toArray()
    : sql.exec(
        `SELECT noun_type, count(*) as c FROM entity_index GROUP BY noun_type ORDER BY noun_type`,
      ).toArray()
  return rows.map((row: any) => ({ nounType: row.noun_type, count: row.c }))
}

// =========================================================================
// Noun resolution
// =========================================================================

export function listDomains(sql: SqlLike): string[] {
  const rows = sql.exec('SELECT domain_slug FROM domains').toArray()
  return rows.map((row: any) => row.domain_slug)
}

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
  return { domainSlug: row.domain_slug, domainDoId: row.domain_do_id }
}

export function getRegisteredNouns(sql: SqlLike): string[] {
  const rows = sql.exec('SELECT DISTINCT noun_name FROM noun_index ORDER BY noun_name').toArray()
  return rows.map((row: any) => row.noun_name)
}

// =========================================================================
// Domain-level sharding helpers (#205)
// =========================================================================

/**
 * Return the DurableObjectId for the registry scoped to a (scope, domain) pair.
 * Falls back to 'global' when domain is empty/undefined so existing callers
 * that already use idFromName('global') are unaffected.
 */
export function registryIdForDomain(
  ns: DurableObjectNamespace,
  scope: string,
  domain?: string,
): DurableObjectId {
  const key = domain ? `${scope}:${domain}` : scope
  return ns.idFromName(key)
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
   *
   * Domains are NORMA tabs, not partitions. Fact types are idempotent.
   * A Noun "Customer" in both "sales" and "support" is ONE cell.
   * INSERT OR REPLACE by (noun_type, entity_id) — no delete-by-domain.
   */
  async materializeBatch(
    entities: Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }>,
  ): Promise<{ materialized: number; failed: string[] }> {
    this.ensureInit()
    const entityDB = (this.env as any).ENTITY_DB as DurableObjectNamespace

    const results = await Promise.allSettled(
      entities.map(async (entity) => {
        const stub = entityDB.get(entityDB.idFromName(entity.id)) as any
        await stub.put({ id: entity.id, type: entity.type, data: entity.data })
        // Index with domain as tag (INSERT OR REPLACE — idempotent)
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

  // ── Freeze/thaw snapshot persistence (#203) ───────────────────────

  async storeSnapshot(label: string, frozenHex: string): Promise<void> {
    this.ensureInit()
    storeSnapshot(this.ctx.storage.sql, label, frozenHex)
  }

  async fetchSnapshot(label: string): Promise<string | null> {
    this.ensureInit()
    return fetchSnapshot(this.ctx.storage.sql, label)
  }

  async listSnapshots(): Promise<Array<{ label: string; createdAt: string; byteLength: number }>> {
    this.ensureInit()
    return listSnapshots(this.ctx.storage.sql)
  }

  async deleteSnapshot(label: string): Promise<boolean> {
    this.ensureInit()
    return deleteSnapshot(this.ctx.storage.sql, label)
  }
}
