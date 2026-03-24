/**
 * Cascade delete logic for metamodel entities.
 *
 * When deleting a metamodel entity (e.g. a Noun), find all entities that
 * reference it (Readings, Roles, Constraints) and build a delete batch.
 * The dependency graph comes from the metamodel's fact types — "Graph Schema
 * has Reading" means deleting a Graph Schema cascades to its Readings.
 */

import type { BatchEntity } from '../batch-wal'

// =========================================================================
// Stub interfaces (for testability without real DO bindings)
// =========================================================================

export interface CascadeRegistryStub {
  getEntityIds(nounType: string, domainSlug?: string): Promise<string[]>
}

export interface CascadeEntityStub {
  get(): Promise<{ id: string; type: string; data: Record<string, unknown> } | null>
}

// =========================================================================
// Cascade dependency graph
// =========================================================================

/**
 * Map of entity type → which entity types reference it and via which data field.
 *
 * Each entry says: "if you delete an entity of type X, also delete entities
 * of type Y where Y.data[field] === X.id".
 */
export const CASCADE_GRAPH: Record<string, Array<{ type: string; field: string }>> = {
  'Noun': [
    { type: 'Reading', field: 'graphSchemaId' }, // via Graph Schema
    { type: 'Role', field: 'nounId' },
    { type: 'Constraint', field: 'nounId' },
    { type: 'State Machine Definition', field: 'nounId' },
  ],
  'Graph Schema': [
    { type: 'Reading', field: 'graphSchemaId' },
    { type: 'Role', field: 'graphSchemaId' },
  ],
  'Constraint': [
    { type: 'Constraint Span', field: 'constraintId' },
  ],
  'State Machine Definition': [
    { type: 'Status', field: 'stateMachineDefinitionId' },
    { type: 'Transition', field: 'fromStatusId' },
  ],
}

// =========================================================================
// Cascade delete batch builder
// =========================================================================

/**
 * Builds a flat array of BatchEntity records representing everything that
 * should be deleted when `rootId` (of `rootType`) is deleted.
 *
 * The first element is always the root entity itself. Subsequent elements
 * are dependents discovered by walking CASCADE_GRAPH, including recursive
 * dependents (e.g. Noun → Constraint → Constraint Span).
 */
export async function buildCascadeDeleteBatch(
  rootId: string,
  rootType: string,
  registry: CascadeRegistryStub,
  getStub: (id: string) => CascadeEntityStub,
  domain?: string,
): Promise<BatchEntity[]> {
  const domainStr = domain || ''
  const toDelete: BatchEntity[] = [
    { id: rootId, type: rootType, domain: domainStr, data: {} },
  ]

  const dependents = CASCADE_GRAPH[rootType] || []
  for (const dep of dependents) {
    const ids = await registry.getEntityIds(dep.type, domain)
    const entities = await Promise.all(ids.map(id => getStub(id).get()))

    for (const entity of entities) {
      if (!entity) continue
      if ((entity.data as Record<string, unknown>)[dep.field] === rootId) {
        toDelete.push({
          id: entity.id,
          type: entity.type,
          domain: domainStr,
          data: entity.data,
        })

        // Recursive: if this dependent also has cascades, walk them
        if (CASCADE_GRAPH[entity.type]) {
          const subDeps = await buildCascadeDeleteBatch(
            entity.id,
            entity.type,
            registry,
            getStub,
            domain,
          )
          // Skip element 0 (the sub-root itself — already added above)
          toDelete.push(...subDeps.slice(1))
        }
      }
    }
  }

  return toDelete
}
