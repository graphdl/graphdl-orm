/**
 * Worker-side query orchestration.
 * Fan-out to Entity DOs -> collect into Population -> FOL filter/reduce.
 * FP is the basis of parallelism -- pure functions over immutable data.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface EntityStub {
  get(): Promise<{ id: string; type: string; data: Record<string, unknown> } | null>
}

export interface QueryRequest {
  nounType: string
  factTypeId: string
  filterBindings: Array<[string, string]>
}

export interface Population {
  facts: Record<string, Array<{
    fact_type_id: string
    bindings: Array<[string, string]>
  }>>
}

export interface QueryResult {
  matches: string[]
  count: number
}

// ---------------------------------------------------------------------------
// buildPopulation
// ---------------------------------------------------------------------------

/**
 * Convert entity DO responses into a Population for FOL evaluation.
 *
 * Each entity produces one FactInstance per fact type. The entity's ID becomes
 * a binding for `nounType`, and each `valueField` present in entity.data
 * becomes an additional binding.
 */
export function buildPopulation(
  nounType: string,
  entities: Array<{ id: string; data: Record<string, unknown> }>,
  factTypeId: string,
  valueFields: string[],
): Population {
  const factInstances = entities.map((entity) => {
    const bindings: Array<[string, string]> = [
      [nounType, entity.id],
    ]

    for (const field of valueFields) {
      const value = entity.data[field]
      if (value !== undefined && value !== null) {
        bindings.push([field, String(value)])
      }
    }

    return {
      fact_type_id: factTypeId,
      bindings,
    }
  })

  return {
    facts: {
      [factTypeId]: factInstances,
    },
  }
}

// ---------------------------------------------------------------------------
// fanOutCollect
// ---------------------------------------------------------------------------

const DEFAULT_BATCH_SIZE = 50

/**
 * Fan out to Entity DOs in batches, collect data, build result array.
 * Pure map phase -- each DO returns its immutable data.
 *
 * Entities that return null (deleted / empty DOs) are filtered out.
 */
export async function fanOutCollect(
  entityIds: string[],
  getStub: (id: string) => EntityStub,
  batchSize: number = DEFAULT_BATCH_SIZE,
): Promise<Array<{ id: string; data: Record<string, unknown> }>> {
  if (entityIds.length === 0) return []

  const results: Array<{ id: string; data: Record<string, unknown> }> = []

  for (let i = 0; i < entityIds.length; i += batchSize) {
    const batch = entityIds.slice(i, i + batchSize)

    const batchResults = await Promise.all(
      batch.map(async (id) => {
        const stub = getStub(id)
        const entity = await stub.get()
        return entity
      }),
    )

    for (const entity of batchResults) {
      if (entity !== null) {
        results.push({ id: entity.id, data: entity.data })
      }
    }
  }

  return results
}

// ---------------------------------------------------------------------------
// queryPopulation (TypeScript mirror of Rust query_population)
// ---------------------------------------------------------------------------

/**
 * Filter a Population by a predicate.
 * Returns all entity references of targetNoun where the filterBindings match.
 *
 * This is a TypeScript implementation of the same algorithm as the Rust
 * `query_population` in crates/fol-engine/src/query.rs. The WASM module
 * can be swapped in later for production use.
 */
function queryPopulation(
  population: Population,
  factTypeId: string,
  targetNoun: string,
  filterBindings: Array<[string, string]>,
): QueryResult {
  const facts = population.facts[factTypeId]
  if (!facts) return { matches: [], count: 0 }

  const matches: string[] = []

  for (const fact of facts) {
    const allFiltersMatch = filterBindings.every(([noun, value]) =>
      fact.bindings.some(([n, v]) => n === noun && v === value),
    )

    if (allFiltersMatch) {
      const entityBinding = fact.bindings.find(([n]) => n === targetNoun)
      if (entityBinding) {
        matches.push(entityBinding[1])
      }
    }
  }

  return { matches, count: matches.length }
}

// ---------------------------------------------------------------------------
// executeQuery
// ---------------------------------------------------------------------------

/**
 * Execute a full query: fan out -> collect -> build population -> filter.
 *
 * 1. Fan out to Entity DOs in parallel batches to collect raw data.
 * 2. Build a Population from the collected entities.
 * 3. Apply FOL predicate filtering to find matching entities.
 */
export async function executeQuery(
  entityIds: string[],
  getStub: (id: string) => EntityStub,
  query: QueryRequest,
  valueFields: string[],
  batchSize?: number,
): Promise<QueryResult> {
  // Step 1: Fan out to Entity DOs
  const entities = await fanOutCollect(entityIds, getStub, batchSize)

  // Step 2: Build Population
  const population = buildPopulation(
    query.nounType,
    entities,
    query.factTypeId,
    valueFields,
  )

  // Step 3: Filter with FOL predicate
  return queryPopulation(
    population,
    query.factTypeId,
    query.nounType,
    query.filterBindings,
  )
}
