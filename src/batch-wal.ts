/**
 * Batch WAL — pure functions for a Write-Ahead Log that tracks entity
 * ingestion batches. This is the core of the slimmed-down DomainDB:
 * a single `batches` table replaces the old 20+ metamodel tables.
 *
 * Each batch holds a JSON array of entities to be committed.
 * The lifecycle is: pending → committed | failed.
 */

// =========================================================================
// Types
// =========================================================================

export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

export interface BatchEntity {
  id: string
  type: string
  domain: string
  data: Record<string, unknown>
}

export interface Batch {
  id: string
  domain: string
  status: 'pending' | 'committed' | 'failed'
  entities: BatchEntity[]
  entityCount: number
  createdAt: string
}

// =========================================================================
// Schema initialisation
// =========================================================================

/**
 * Creates the batches table.
 */
export function initBatchSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS batches (
  id TEXT PRIMARY KEY,
  domain TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'committed', 'failed')),
  entities TEXT NOT NULL,
  entity_count INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  committed_at TEXT,
  error TEXT
)`)
}

// =========================================================================
// Batch operations
// =========================================================================

/**
 * Creates a new batch in pending status. Generates a UUID, stores the
 * entities as a JSON string, and returns the Batch object.
 */
export function createBatch(sql: SqlLike, domain: string, entities: BatchEntity[]): Batch {
  const id = crypto.randomUUID()
  const now = new Date().toISOString()
  const entitiesJson = JSON.stringify(entities)

  sql.exec(
    `INSERT INTO batches (id, domain, status, entities, entity_count, created_at, committed_at, error) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    id,
    domain,
    'pending',
    entitiesJson,
    entities.length,
    now,
    null,
    null,
  )

  return {
    id,
    domain,
    status: 'pending',
    entities,
    entityCount: entities.length,
    createdAt: now,
  }
}

/**
 * Retrieves a batch by ID. Parses the JSON entities string.
 * Returns null if no batch with that ID exists.
 */
export function getBatch(sql: SqlLike, id: string): Batch | null {
  const rows = sql.exec(`SELECT * FROM batches WHERE id = ?`, id).toArray()
  if (rows.length === 0) return null

  const row = rows[0] as Record<string, any>
  return {
    id: row.id,
    domain: row.domain,
    status: row.status,
    entities: typeof row.entities === 'string' ? JSON.parse(row.entities) : row.entities,
    entityCount: row.entity_count,
    createdAt: row.created_at,
  }
}

/**
 * Marks a batch as committed. Sets status to 'committed' and records
 * the committed_at timestamp.
 */
export function markCommitted(sql: SqlLike, id: string): void {
  const now = new Date().toISOString()
  sql.exec(
    `UPDATE batches SET status = ?, committed_at = ? WHERE id = ?`,
    'committed',
    now,
    id,
  )
}

/**
 * Marks a batch as failed. Sets status to 'failed' and stores the
 * error message.
 */
export function markFailed(sql: SqlLike, id: string, error: string): void {
  sql.exec(
    `UPDATE batches SET status = ?, error = ? WHERE id = ?`,
    'failed',
    error,
    id,
  )
}

/**
 * Returns all pending batches ordered by created_at ASC (oldest first).
 */
export function getPendingBatches(sql: SqlLike): Batch[] {
  const rows = sql.exec(
    `SELECT * FROM batches WHERE status = ? ORDER BY created_at ASC`,
    'pending',
  ).toArray()

  return rows.map((row: any) => ({
    id: row.id,
    domain: row.domain,
    status: row.status as 'pending',
    entities: typeof row.entities === 'string' ? JSON.parse(row.entities) : row.entities,
    entityCount: row.entity_count,
    createdAt: row.created_at,
  }))
}
