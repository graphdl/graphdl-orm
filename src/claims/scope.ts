export interface NounRecord {
  id: string
  name: string
  domainId: string
  [key: string]: any
}

export interface SchemaRecord {
  id: string
  [key: string]: any
}

export interface Scope {
  /** domainId:nounName -> noun record */
  nouns: Map<string, NounRecord>
  /** reading text -> graph schema record */
  schemas: Map<string, SchemaRecord>
  /** count of items skipped due to idempotency (already exists) */
  skipped: number
  /** accumulated errors */
  errors: string[]
}

export function createScope(): Scope {
  return {
    nouns: new Map(),
    schemas: new Map(),
    skipped: 0,
    errors: [],
  }
}

/** Add a noun to the scope, keyed by domainId:name */
export function addNoun(scope: Scope, noun: NounRecord): void {
  scope.nouns.set(`${noun.domainId}:${noun.name}`, noun)
}

/**
 * Resolve a noun by name within the scope.
 * Search order: local domain first, then all other domains.
 * App/org scoping is applied by the caller (ingestProject pools
 * only domains within the same app).
 */
export function resolveNoun(
  scope: Scope,
  name: string,
  domainId: string,
): NounRecord | null {
  // 1. Local domain
  const local = scope.nouns.get(`${domainId}:${name}`)
  if (local) return local

  // 2. Any domain in scope (scope only contains visible domains)
  for (const [_key, noun] of scope.nouns) {
    if (noun.name === name) return noun
  }

  return null
}

export function addSchema(scope: Scope, readingText: string, schema: SchemaRecord): void {
  scope.schemas.set(readingText, schema)
}

export function resolveSchema(scope: Scope, readingText: string): SchemaRecord | null {
  return scope.schemas.get(readingText) || null
}
