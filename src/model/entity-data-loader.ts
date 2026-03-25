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

  // ---- Private helper: fan-out by entity type ----

  private async fetchByType(entityType: string, domainId?: string): Promise<Row[]> {
    const ids = domainId
      ? await this.registry.getEntityIds(entityType, domainId)
      : await this.registry.getEntityIds(entityType)
    const entities = await fanOut(ids, async (id) => {
      const stub = this.getStub(id)
      return stub.get()
    })
    return entities.map((e) => ({ id: e.id, ...e.data }))
  }

  // ---- Noun ----

  async queryNouns(domainId: string): Promise<Row[]> {
    return this.fetchByType('Noun', domainId)
  }

  // ---- Graph Schemas ----

  async queryGraphSchemas(domainId: string): Promise<Row[]> {
    return this.fetchByType('Graph Schema', domainId)
  }

  // ---- Readings ----

  async queryReadings(domainId: string): Promise<Row[]> {
    return this.fetchByType('Reading', domainId)
  }

  // ---- Roles (no domain filter — fetches all) ----

  async queryRoles(): Promise<Row[]> {
    return this.fetchByType('Role')
  }

  // ---- Constraints ----

  async queryConstraints(domainId: string): Promise<Row[]> {
    return this.fetchByType('Constraint', domainId)
  }

  // ---- Constraint Spans (no domain filter — fetches all) ----

  async queryConstraintSpans(): Promise<Row[]> {
    return this.fetchByType('Constraint Span')
  }

  // ---- State Machine Definitions ----

  async queryStateMachineDefs(domainId: string): Promise<Row[]> {
    return this.fetchByType('State Machine Definition', domainId)
  }

  // ---- Statuses ----

  async queryStatuses(domainId: string): Promise<Row[]> {
    return this.fetchByType('Status', domainId)
  }

  // ---- Transitions ----

  async queryTransitions(domainId: string): Promise<Row[]> {
    return this.fetchByType('Transition', domainId)
  }

  // ---- Event Types ----

  async queryEventTypes(domainId: string): Promise<Row[]> {
    return this.fetchByType('Event Type', domainId)
  }

  // ---- Guards ----

  async queryGuards(domainId: string): Promise<Row[]> {
    return this.fetchByType('Guard', domainId)
  }

  // ---- Verbs ----

  async queryVerbs(domainId: string): Promise<Row[]> {
    return this.fetchByType('Verb', domainId)
  }

  // ---- Functions ----

  async queryFunctions(domainId: string): Promise<Row[]> {
    return this.fetchByType('Function', domainId)
  }
}
