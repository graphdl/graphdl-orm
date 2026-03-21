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

export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

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
  visibility TEXT NOT NULL DEFAULT 'private'
)`)

  sql.exec(`CREATE TABLE IF NOT EXISTS noun_index (
  noun_name TEXT NOT NULL,
  domain_slug TEXT NOT NULL,
  PRIMARY KEY (noun_name, domain_slug)
)`)

  sql.exec(`CREATE TABLE IF NOT EXISTS entity_index (
  noun_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (noun_type, entity_id)
)`)
}

// =========================================================================
// Domain registration
// =========================================================================

/**
 * Upsert a domain into the domains table.
 * INSERT OR REPLACE ensures idempotency — re-registering the same slug
 * simply updates the DO ID and visibility.
 */
export function registerDomain(sql: SqlLike, slug: string, doId: string, visibility: string = 'private'): void {
  sql.exec(
    `INSERT OR REPLACE INTO domains (domain_slug, domain_do_id, visibility) VALUES (?, ?, ?)`,
    slug,
    doId,
    visibility,
  )
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
export function indexEntity(sql: SqlLike, nounType: string, entityId: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO entity_index (noun_type, entity_id, deleted) VALUES (?, ?, ?)`,
    nounType,
    entityId,
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
export function getEntityIds(sql: SqlLike, nounType: string): string[] {
  const rows = sql.exec(
    `SELECT entity_id FROM entity_index WHERE noun_type=? AND deleted=0`,
    nounType,
  ).toArray()

  return rows.map((row: any) => row.entity_id)
}

// =========================================================================
// Noun resolution
// =========================================================================

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

  async registerDomain(slug: string, doId: string, visibility?: string): Promise<void> {
    this.ensureInit()
    registerDomain(this.ctx.storage.sql, slug, doId, visibility)
  }

  async indexNoun(nounName: string, domainSlug: string): Promise<void> {
    this.ensureInit()
    indexNoun(this.ctx.storage.sql, nounName, domainSlug)
  }

  async resolveNoun(nounName: string): Promise<{ domainSlug: string; domainDoId: string } | null> {
    this.ensureInit()
    return resolveNounInRegistry(this.ctx.storage.sql, nounName)
  }

  async indexEntity(nounType: string, entityId: string): Promise<void> {
    this.ensureInit()
    indexEntity(this.ctx.storage.sql, nounType, entityId)
  }

  async deindexEntity(nounType: string, entityId: string): Promise<void> {
    this.ensureInit()
    deindexEntity(this.ctx.storage.sql, nounType, entityId)
  }

  async getEntityIds(nounType: string): Promise<string[]> {
    this.ensureInit()
    return getEntityIds(this.ctx.storage.sql, nounType)
  }
}
