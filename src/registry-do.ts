/**
 * RegistryDB — pure functions for the global registry schema.
 *
 * The registry maps domain slugs to their Durable Object IDs and
 * indexes nouns so cross-domain lookups can resolve which domain
 * owns a given noun type.
 */

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
