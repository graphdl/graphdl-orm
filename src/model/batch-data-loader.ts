/**
 * BatchDataLoader — reads metamodel entities from DomainDB batch WAL.
 *
 * Alternative to EntityDataLoader for domains where entities are stored
 * in the DomainDB's batch WAL (from seed) rather than individual EntityDB DOs.
 * No subrequest fan-out — all data is in the DomainDB's SQLite.
 */

import type { DataLoader } from './domain-model'

type Row = Record<string, any>

export interface DomainDBStub {
  queryEntitiesByType(entityType: string): Promise<Array<{ id: string; type: string; data: Record<string, unknown> }>>
}

export class BatchDataLoader implements DataLoader {
  constructor(
    private domainDO: DomainDBStub,
  ) {}

  private async fetchByType(entityType: string): Promise<Row[]> {
    const entities = await this.domainDO.queryEntitiesByType(entityType)
    return entities.map(e => ({ id: e.id, ...e.data }))
  }

  async queryNouns(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Noun')
  }

  async queryGraphSchemas(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Graph Schema')
  }

  async queryReadings(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Reading')
  }

  async queryRoles(): Promise<Row[]> {
    return this.fetchByType('Role')
  }

  async queryConstraints(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Constraint')
  }

  async queryConstraintSpans(): Promise<Row[]> {
    return this.fetchByType('Constraint Span')
  }

  async queryStateMachineDefs(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('State Machine Definition')
  }

  async queryStatuses(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Status')
  }

  async queryTransitions(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Transition')
  }

  async queryEventTypes(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Event Type')
  }

  async queryGuards(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Guard')
  }

  async queryVerbs(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Verb')
  }

  async queryFunctions(_domainId?: string): Promise<Row[]> {
    return this.fetchByType('Function')
  }
}
