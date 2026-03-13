import { describe, it, expect, vi } from 'vitest'
import { DomainModel } from './domain-model'
import type { DataLoader } from './domain-model'

const DOMAIN = 'd1'

function mockLoader(data: Partial<Record<string, any[]>>): DataLoader {
  return {
    queryNouns: () => data.nouns ?? [],
    queryGraphSchemas: () => data.graphSchemas ?? [],
    queryReadings: () => data.readings ?? [],
    queryRoles: () => data.roles ?? [],
    queryConstraints: () => data.constraints ?? [],
    queryConstraintSpans: () => data.constraintSpans ?? [],
    queryStateMachineDefs: () => data.smDefs ?? [],
    queryStatuses: () => data.statuses ?? [],
    queryTransitions: () => data.transitions ?? [],
    queryEventTypes: () => data.eventTypes ?? [],
    queryGuards: () => data.guards ?? [],
    queryVerbs: () => data.verbs ?? [],
    queryFunctions: () => data.functions ?? [],
  } as DataLoader
}

describe('DomainModel', () => {
  describe('nouns()', () => {
    it('returns entity and value nouns with resolved superTypes', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Person', object_type: 'entity', domain_id: DOMAIN, super_type_id: null },
            { id: 'n2', name: 'Customer', object_type: 'entity', domain_id: DOMAIN, super_type_id: 'n1', super_type_name: 'Person' },
            { id: 'n3', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      expect(nouns.size).toBe(3)
      expect(nouns.get('Customer')?.superType).toBe('Person')
      expect(nouns.get('Name')?.valueType).toBe('string')
    })

    it('parses enum_values from JSON string', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Priority', object_type: 'value', domain_id: DOMAIN, enum_values: '["low","medium","high"]' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      expect(nouns.get('Priority')?.enumValues).toEqual(['low', 'medium', 'high'])
    })

    it('caches results across calls', async () => {
      const loader = mockLoader({
        nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }],
      })
      const spy = vi.spyOn(loader, 'queryNouns')
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns()
      await model.nouns()
      expect(spy).toHaveBeenCalledTimes(1)
    })
  })

  describe('factTypes()', () => {
    it('groups roles by graph schema and resolves noun references', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n2', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
          ],
          graphSchemas: [{ id: 'gs1', name: 'CustomerHasName', domain_id: DOMAIN }],
          readings: [{ id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN }],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN },
            { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 1, domain_id: DOMAIN },
          ],
        }),
        DOMAIN,
      )

      const fts = await model.factTypes()
      expect(fts.size).toBe(1)
      const ft = fts.get('gs1')!
      expect(ft.reading).toBe('Customer has Name')
      expect(ft.arity).toBe(2)
      expect(ft.roles[0].nounName).toBe('Customer')
      expect(ft.roles[1].nounDef.valueType).toBe('string')
    })
  })

  describe('constraints()', () => {
    it('groups spans by constraint and resolves role indices', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [{ id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN }],
          roles: [{ id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN }],
          constraints: [{ id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN }],
          constraintSpans: [
            { id: 'cs1', constraint_id: 'c1', role_id: 'r1', domain_id: DOMAIN, graph_schema_id: 'gs1', role_index: 0 },
          ],
        }),
        DOMAIN,
      )

      const cs = await model.constraints()
      expect(cs).toHaveLength(1)
      expect(cs[0].kind).toBe('UC')
      expect(cs[0].spans).toHaveLength(1)
      expect(cs[0].spans[0].factTypeId).toBe('gs1')
    })

    it('derives deonticOperator from text', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'MC', modality: 'Deontic', text: 'It is obligatory that each Order has at least one Item', domain_id: DOMAIN },
            { id: 'c2', kind: 'MC', modality: 'Deontic', text: 'It is forbidden that X does Y', domain_id: DOMAIN },
          ],
          constraintSpans: [],
        }),
        DOMAIN,
      )

      const cs = await model.constraints()
      expect(cs[0].deonticOperator).toBe('obligatory')
      expect(cs[1].deonticOperator).toBe('forbidden')
    })
  })

  describe('stateMachines()', () => {
    it('resolves full transition chain: status -> event -> verb -> function', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [{ id: 'n1', name: 'Order', object_type: 'entity', domain_id: DOMAIN }],
          smDefs: [{ id: 'smd1', noun_id: 'n1', domain_id: DOMAIN }],
          statuses: [
            { id: 's1', name: 'Draft', state_machine_definition_id: 'smd1', created_at: '2026-01-01', domain_id: DOMAIN },
            { id: 's2', name: 'Pending', state_machine_definition_id: 'smd1', created_at: '2026-01-02', domain_id: DOMAIN },
          ],
          transitions: [
            { id: 't1', from_status_id: 's1', to_status_id: 's2', event_type_id: 'et1', verb_id: 'v1', domain_id: DOMAIN },
          ],
          eventTypes: [{ id: 'et1', name: 'Submit', domain_id: DOMAIN }],
          verbs: [{ id: 'v1', name: 'submitOrder', domain_id: DOMAIN }],
          functions: [
            { id: 'f1', verb_id: 'v1', callback_url: '/api/submit', http_method: 'POST', headers: null, domain_id: DOMAIN },
          ],
          guards: [],
        }),
        DOMAIN,
      )

      const sms = await model.stateMachines()
      expect(sms.size).toBe(1)
      const sm = sms.get('smd1')!
      expect(sm.nounName).toBe('Order')
      expect(sm.statuses).toHaveLength(2)
      expect(sm.transitions[0].from).toBe('Draft')
      expect(sm.transitions[0].to).toBe('Pending')
      expect(sm.transitions[0].event).toBe('Submit')
      expect(sm.transitions[0].verb?.func?.callbackUrl).toBe('/api/submit')
    })
  })

  describe('invalidate()', () => {
    it('clears specific cache keys by collection', async () => {
      const loader = mockLoader({
        nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }],
      })
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns()
      model.invalidate('nouns')

      const spy = vi.spyOn(loader, 'queryNouns')
      await model.nouns()
      expect(spy).toHaveBeenCalledTimes(1)
    })

    it('clears all caches when no collection specified', async () => {
      const loader = mockLoader({
        nouns: [{ id: 'n1', name: 'X', object_type: 'entity', domain_id: DOMAIN }],
        constraints: [{ id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN }],
      })
      const model = new DomainModel(loader, DOMAIN)

      await model.nouns()
      await model.constraints()
      model.invalidate()

      const nounSpy = vi.spyOn(loader, 'queryNouns')
      const cSpy = vi.spyOn(loader, 'queryConstraints')
      await model.nouns()
      await model.constraints()
      expect(nounSpy).toHaveBeenCalledTimes(1)
      expect(cSpy).toHaveBeenCalledTimes(1)
    })
  })

  describe('factTypesFor()', () => {
    it('filters fact types where noun plays role at index 0', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n2', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
            { id: 'n3', name: 'Order', object_type: 'entity', domain_id: DOMAIN },
          ],
          graphSchemas: [
            { id: 'gs1', name: 'CustomerHasName', domain_id: DOMAIN },
            { id: 'gs2', name: 'OrderHasName', domain_id: DOMAIN },
          ],
          readings: [
            { id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN },
            { id: 'rd2', text: 'Order has Name', graph_schema_id: 'gs2', domain_id: DOMAIN },
          ],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN },
            { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 1, domain_id: DOMAIN },
            { id: 'r3', noun_id: 'n3', graph_schema_id: 'gs2', role_index: 0, domain_id: DOMAIN },
            { id: 'r4', noun_id: 'n2', graph_schema_id: 'gs2', role_index: 1, domain_id: DOMAIN },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      const customerFts = await model.factTypesFor(nouns.get('Customer')!)
      expect(customerFts).toHaveLength(1)
      expect(customerFts[0].id).toBe('gs1')
    })
  })

  describe('constraintsFor()', () => {
    it('returns constraints whose spans reference given fact types', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN },
            { id: 'c2', kind: 'MC', modality: 'Alethic', domain_id: DOMAIN },
          ],
          constraintSpans: [
            { id: 'cs1', constraint_id: 'c1', role_id: 'r1', domain_id: DOMAIN, graph_schema_id: 'gs1', role_index: 0 },
            { id: 'cs2', constraint_id: 'c2', role_id: 'r2', domain_id: DOMAIN, graph_schema_id: 'gs2', role_index: 0 },
          ],
        }),
        DOMAIN,
      )

      const matched = await model.constraintsFor([{ id: 'gs1', reading: '', roles: [], arity: 0 }])
      expect(matched).toHaveLength(1)
      expect(matched[0].id).toBe('c1')
    })
  })

  describe('noun()', () => {
    it('returns a single noun by name', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [{ id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN }],
        }),
        DOMAIN,
      )

      const noun = await model.noun('Customer')
      expect(noun?.name).toBe('Customer')
      expect(await model.noun('Unknown')).toBeUndefined()
    })
  })

  describe('readings()', () => {
    it('returns reading definitions with resolved roles', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n2', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
          ],
          readings: [{ id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN }],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN },
            { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 1, domain_id: DOMAIN },
          ],
        }),
        DOMAIN,
      )

      const readings = await model.readings()
      expect(readings).toHaveLength(1)
      expect(readings[0].text).toBe('Customer has Name')
      expect(readings[0].roles).toHaveLength(2)
      expect(readings[0].roles[0].nounName).toBe('Customer')
    })
  })
})
