/**
 * Bootstrap data for the GraphDL metamodel.
 *
 * Seeds the system's own entity types as noun records so the framework
 * is self-aware from first boot. Uses INSERT OR IGNORE for idempotency.
 *
 * This is the minimum viable metamodel — just the entity types that
 * correspond to physical tables + key subtypes. Full readings, constraints,
 * and graph schemas come from the seed script (scripts/seed-metamodel.ts).
 */

const DOMAIN_ID = 'graphdl-core'
const NOW = "datetime('now')"

/** Deterministic IDs based on entity name. */
function nounId(name: string): string {
  return `meta-${name.toLowerCase().replace(/\s+/g, '-')}`
}

/**
 * Bootstrap SQL statements. Run after DDL, idempotent via INSERT OR IGNORE.
 *
 * Creates:
 * 1. The graphdl-core domain
 * 2. Core entity type nouns (one per physical table + key subtypes)
 */
export const BOOTSTRAP_DML: string[] = [
  // ── Domain ────────────────────────────────────────────────────────────
  `INSERT OR IGNORE INTO domains (id, domain_slug, name, visibility, created_at, updated_at, version)
   VALUES ('${DOMAIN_ID}', 'graphdl-core', 'GraphDL Core Metamodel', 'public', ${NOW}, ${NOW}, 1)`,

  // ── Organizations & Access Control ────────────────────────────────────
  ...entityNoun('Organization'),
  ...entityNoun('Domain'),

  // ── Core Metamodel Entities ───────────────────────────────────────────
  ...entityNoun('Noun'),
  ...entityNoun('Graph Schema', 'Noun'),
  ...entityNoun('Status', 'Noun'),
  ...entityNoun('Reading'),
  ...entityNoun('Role'),
  ...entityNoun('Constraint'),
  ...entityNoun('Constraint Span'),

  // ── State Machine Definitions ─────────────────────────────────────────
  ...entityNoun('State Machine Definition'),
  ...entityNoun('Event Type'),
  ...entityNoun('Transition'),
  ...entityNoun('Guard'),
  ...entityNoun('Verb'),
  ...entityNoun('Function'),
  ...entityNoun('Stream'),

  // ── Runtime Instances ─────────────────────────────────────────────────
  ...entityNoun('Graph', 'Resource'),
  ...entityNoun('Resource'),
  ...entityNoun('Resource Role'),
  ...entityNoun('State Machine'),
  ...entityNoun('Event'),
  ...entityNoun('Guard Run'),
]

/** Generate INSERT OR IGNORE for an entity noun, optionally with a supertype. */
function entityNoun(name: string, superType?: string): string[] {
  const id = nounId(name)
  const superTypeId = superType ? `'${nounId(superType)}'` : 'NULL'
  return [
    `INSERT OR IGNORE INTO nouns (id, name, object_type, domain_id, super_type_id, created_at, updated_at, version)
     VALUES ('${id}', '${name}', 'entity', '${DOMAIN_ID}', ${superTypeId}, ${NOW}, ${NOW}, 1)`,
  ]
}
