/**
 * Convention-based naming utilities for noun → slug/table name derivation.
 *
 * Noun names are the authority (from readings, seeded as Noun entities).
 * Slugs and table names are deterministic projections of those names.
 * No hardcoded maps — the Registry is the source of truth for what nouns exist.
 */

// ---------------------------------------------------------------------------
// Pluralization
// ---------------------------------------------------------------------------

/**
 * Simple English pluralization for noun names.
 * Handles common ORM2 patterns: Status→statuses, Entity→entities, App→apps
 */
export function pluralize(word: string): string {
  const lower = word.toLowerCase()
  if (lower.endsWith('z')) {
    return word + 'zes' // Quiz → Quizzes (double z)
  }
  if (lower.endsWith('ss') || lower.endsWith('sh') || lower.endsWith('ch') || lower.endsWith('x')) {
    return word + 'es'
  }
  if (lower.endsWith('s')) {
    return word + 'es' // Status → Statuses
  }
  if (lower.endsWith('y') && !/[aeiou]y$/i.test(word)) {
    return word.slice(0, -1) + 'ies' // Entity → Entities
  }
  return word + 's'
}

// ---------------------------------------------------------------------------
// Noun name → collection slug (kebab-case, pluralized)
// ---------------------------------------------------------------------------

/**
 * Derive a REST collection slug from a noun name.
 *
 * "Organization"             → "organizations"
 * "OrgMembership"            → "org-memberships"
 * "Fact Type"             → "graph-schemas"
 * "State Machine Definition" → "state-machine-definitions"
 * "Status"                   → "statuses"
 */
export function nounToSlug(name: string): string {
  const words = name.includes(' ')
    ? name.split(/\s+/)
    : name.split(/(?=[A-Z])/).filter(Boolean)

  return words
    .map((w, i) => (i === words.length - 1 ? pluralize(w) : w).toLowerCase())
    .join('-')
}

// ---------------------------------------------------------------------------
// Noun name → SQL table name (snake_case, pluralized)
// ---------------------------------------------------------------------------

/**
 * Derive a SQL table name from a noun name.
 *
 * "Organization"    → "organizations"
 * "OrgMembership"   → "org_memberships"
 * "Fact Type"    → "fact_types"
 * "Support Request" → "support_requests"
 * "Status"          → "statuses"
 */
export function nounToTable(name: string): string {
  const words = name.includes(' ')
    ? name.split(/\s+/)
    : name.split(/(?=[A-Z])/).filter(Boolean)

  return words
    .map((w, i) => (i === words.length - 1 ? pluralize(w) : w).toLowerCase())
    .join('_')
}

// ---------------------------------------------------------------------------
// Dynamic slug → noun resolution via Registry
// ---------------------------------------------------------------------------

export interface NounRegistry {
  getRegisteredNouns(): Promise<string[]>
}

/**
 * Resolve a collection slug to its noun type name by querying the Registry
 * for all noun names (seeded from readings) and matching via nounToSlug convention.
 *
 * Returns null if no registered noun produces the given slug.
 */
export async function resolveSlugToNoun(registry: NounRegistry, slug: string): Promise<string | null> {
  const nouns = await registry.getRegisteredNouns()
  for (const noun of nouns) {
    if (nounToSlug(noun) === slug) return noun
  }
  return null
}
