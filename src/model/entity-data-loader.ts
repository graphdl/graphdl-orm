/**
 * EntityDataLoader — a DataLoader implementation that fans out to EntityDB
 * Durable Objects via a Registry, fetching entities by type and domain.
 *
 * This replaces SqlDataLoader's direct SQLite queries with DO-per-entity
 * fan-out reads. Both loaders coexist during migration.
 */
import type { DataLoader } from './domain-model'

// ---------------------------------------------------------------------------
// Stub interfaces
// ---------------------------------------------------------------------------

type Row = Record<string, any>

export interface RegistryStub {
  getEntityIds(entityType: string, domainSlug?: string): Promise<string[]>
}

export interface EntityStub {
  get(): Promise<EntityData | null>
}

export interface EntityData {
  id: string
  type: string
  data: Row
}

// ---------------------------------------------------------------------------
// Batched fan-out helper
// ---------------------------------------------------------------------------

const BATCH_SIZE = 50

async function fanOut<T>(
  ids: string[],
  fn: (id: string) => Promise<T | null>,
): Promise<T[]> {
  const results: T[] = []

  for (let i = 0; i < ids.length; i += BATCH_SIZE) {
    const batch = ids.slice(i, i + BATCH_SIZE)
    const batchResults = await Promise.all(batch.map(fn))
    for (const result of batchResults) {
      if (result != null) {
        results.push(result)
      }
    }
  }

  return results
}

// ---------------------------------------------------------------------------
// EntityDataLoader
// ---------------------------------------------------------------------------

export class EntityDataLoader implements DataLoader {
  constructor(
    private registry: RegistryStub,
    private getStub: (id: string) => EntityStub,
  ) {}

  // ---- Implemented: Noun fan-out ----

  async queryNouns(domainId: string): Promise<Row[]> {
    const ids = await this.registry.getEntityIds('Noun', domainId)
    const entities = await fanOut(ids, async (id) => {
      const stub = this.getStub(id)
      return stub.get()
    })
    return entities.map((e) => ({ id: e.id, ...e.data }))
  }

  // ---- Stubs: will be implemented in Task 6 ----

  queryGraphSchemas(_domainId: string): Row[] {
    return []
  }

  queryReadings(_domainId: string): Row[] {
    return []
  }

  queryRoles(): Row[] {
    return []
  }

  queryConstraints(_domainId: string): Row[] {
    return []
  }

  queryConstraintSpans(): Row[] {
    return []
  }

  queryStateMachineDefs(_domainId: string): Row[] {
    return []
  }

  queryStatuses(_domainId: string): Row[] {
    return []
  }

  queryTransitions(_domainId: string): Row[] {
    return []
  }

  queryEventTypes(_domainId: string): Row[] {
    return []
  }

  queryGuards(_domainId: string): Row[] {
    return []
  }

  queryVerbs(_domainId: string): Row[] {
    return []
  }

  queryFunctions(_domainId: string): Row[] {
    return []
  }
}
