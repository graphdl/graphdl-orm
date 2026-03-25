import { describe, it, expect, vi } from 'vitest'
import { DomainModel, CORE_DOMAIN_ID, DEFAULT_ENTITY_SUPERTYPE, DEFAULT_SUPERTYPE_ROOTS } from './domain-model'
import type { DataLoader } from './domain-model'

const DOMAIN = 'd1'

function mockLoader(data: Partial<Record<string, any[]>>): DataLoader {
  return {
    queryNouns: async () => data.nouns ?? [],
    queryGraphSchemas: async () => data.graphSchemas ?? [],
    queryReadings: async () => data.readings ?? [],
    queryRoles: async () => data.roles ?? [],
    queryConstraints: async () => data.constraints ?? [],
    queryConstraintSpans: async () => data.constraintSpans ?? [],
    queryStateMachineDefs: async () => data.smDefs ?? [],
    queryStatuses: async () => data.statuses ?? [],
    queryTransitions: async () => data.transitions ?? [],
    queryEventTypes: async () => data.eventTypes ?? [],
    queryGuards: async () => data.guards ?? [],
    queryVerbs: async () => data.verbs ?? [],
    queryFunctions: async () => data.functions ?? [],
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

    it('derives permitted as deonticOperator from text', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'MC', modality: 'Deontic', text: 'It is permitted that each Order has multiple Items', domain_id: DOMAIN },
          ],
          constraintSpans: [],
        }),
        DOMAIN,
      )

      const cs = await model.constraints()
      expect(cs[0].deonticOperator).toBe('permitted')
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
    it('resolves guards on transitions', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [{ id: 'n1', name: 'Order', object_type: 'entity', domain_id: DOMAIN }],
          smDefs: [{ id: 'smd1', noun_id: 'n1', domain_id: DOMAIN }],
          statuses: [
            { id: 's1', name: 'Draft', state_machine_definition_id: 'smd1', created_at: '2026-01-01', domain_id: DOMAIN },
            { id: 's2', name: 'Submitted', state_machine_definition_id: 'smd1', created_at: '2026-01-02', domain_id: DOMAIN },
          ],
          transitions: [
            { id: 't1', from_status_id: 's1', to_status_id: 's2', event_type_id: 'et1', verb_id: null, domain_id: DOMAIN },
          ],
          eventTypes: [{ id: 'et1', name: 'Submit', domain_id: DOMAIN }],
          guards: [
            { id: 'g1', transition_id: 't1', graph_schema_id: 'gs99', domain_id: DOMAIN },
            { id: 'g2', transition_id: 't1', graph_schema_id: 'gs99', domain_id: DOMAIN },
          ],
          verbs: [],
          functions: [],
        }),
        DOMAIN,
      )

      const sms = await model.stateMachines()
      const sm = sms.get('smd1')!
      expect(sm.transitions).toHaveLength(1)

      const t = sm.transitions[0]
      expect(t.from).toBe('Draft')
      expect(t.to).toBe('Submitted')
      expect(t.event).toBe('Submit')
      expect(t.guard).toBeDefined()
      expect(t.guard!.graphSchemaId).toBe('gs99')
      expect(t.guard!.constraintIds).toEqual(['g1', 'g2'])
    })

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

  describe('constraintSpans()', () => {
    it('groups spans by constraint ID', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN },
            { id: 'c2', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN },
            { id: 'c3', kind: 'MC', modality: 'Alethic', domain_id: DOMAIN }, // no spans → should not appear
          ],
          constraintSpans: [
            { id: 'cs1', constraint_id: 'c1', role_id: 'r1', domain_id: DOMAIN, graph_schema_id: 'gs1', role_index: 0 },
            { id: 'cs2', constraint_id: 'c1', role_id: 'r2', domain_id: DOMAIN, graph_schema_id: 'gs1', role_index: 1 },
            { id: 'cs3', constraint_id: 'c2', role_id: 'r3', domain_id: DOMAIN, graph_schema_id: 'gs2', role_index: 0 },
          ],
        }),
        DOMAIN,
      )

      const spanMap = await model.constraintSpans()

      // c1 has 2 spans, c2 has 1 span, c3 has 0 spans (not in map)
      expect(spanMap.size).toBe(2)
      expect(spanMap.has('c1')).toBe(true)
      expect(spanMap.has('c2')).toBe(true)
      expect(spanMap.has('c3')).toBe(false)

      const c1Spans = spanMap.get('c1')!
      expect(c1Spans).toHaveLength(2)
      expect(c1Spans[0].factTypeId).toBe('gs1')
      expect(c1Spans[0].roleIndex).toBe(0)
      expect(c1Spans[1].factTypeId).toBe('gs1')
      expect(c1Spans[1].roleIndex).toBe(1)

      const c2Spans = spanMap.get('c2')!
      expect(c2Spans).toHaveLength(1)
      expect(c2Spans[0].factTypeId).toBe('gs2')
      expect(c2Spans[0].roleIndex).toBe(0)
    })

    it('excludes spans from unrelated domains', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: DOMAIN },
          ],
          constraintSpans: [
            { id: 'cs1', constraint_id: 'c1', role_id: 'r1', domain_id: DOMAIN, graph_schema_id: 'gs1', role_index: 0 },
            { id: 'cs2', constraint_id: 'c1', role_id: 'r2', domain_id: 'other-domain', graph_schema_id: 'gs1', role_index: 1 },
          ],
        }),
        DOMAIN,
      )

      const spanMap = await model.constraintSpans()
      // Only 1 span should be included (the one from our domain)
      const spans = spanMap.get('c1')!
      expect(spans).toHaveLength(1)
      expect(spans[0].roleIndex).toBe(0)
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
    it('returns correct graphSchemaId for each reading', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n2', name: 'Name', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
            { id: 'n3', name: 'Order', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n4', name: 'Total', object_type: 'value', domain_id: DOMAIN, value_type: 'number' },
          ],
          readings: [
            { id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN },
            { id: 'rd2', text: 'Order has Total', graph_schema_id: 'gs2', domain_id: DOMAIN },
          ],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN },
            { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 1, domain_id: DOMAIN },
            { id: 'r3', noun_id: 'n3', graph_schema_id: 'gs2', role_index: 0, domain_id: DOMAIN },
            { id: 'r4', noun_id: 'n4', graph_schema_id: 'gs2', role_index: 1, domain_id: DOMAIN },
          ],
        }),
        DOMAIN,
      )

      const readings = await model.readings()
      expect(readings).toHaveLength(2)

      const r1 = readings.find(r => r.text === 'Customer has Name')!
      expect(r1.graphSchemaId).toBe('gs1')

      const r2 = readings.find(r => r.text === 'Order has Total')!
      expect(r2.graphSchemaId).toBe('gs2')
    })

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

  // ---------------------------------------------------------------------------
  // Cross-domain resolution (graphdl-core)
  // ---------------------------------------------------------------------------
  describe('cross-domain resolution', () => {
    it('CORE_DOMAIN_ID is graphdl-core', () => {
      expect(CORE_DOMAIN_ID).toBe('graphdl-core')
    })

    it('includes core nouns alongside domain nouns', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'SupportRequest', object_type: 'entity', domain_id: DOMAIN, super_type_id: 'n1', super_type_name: 'Resource' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      expect(nouns.size).toBe(2)
      expect(nouns.get('Resource')).toBeDefined()
      expect(nouns.get('SupportRequest')?.superType).toBe('Resource')
    })

    it('includes core fact types via roles from graphdl-core domain', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'StateMachine', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
          ],
          graphSchemas: [
            { id: 'gs1', name: 'StateMachineIsForResource', domain_id: CORE_DOMAIN_ID },
          ],
          readings: [
            { id: 'rd1', text: 'StateMachine is for Resource', graph_schema_id: 'gs1', domain_id: CORE_DOMAIN_ID },
          ],
          roles: [
            { id: 'r1', noun_id: 'n2', graph_schema_id: 'gs1', role_index: 0, domain_id: CORE_DOMAIN_ID },
            { id: 'r2', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 1, domain_id: CORE_DOMAIN_ID },
          ],
        }),
        DOMAIN,
      )

      const fts = await model.factTypes()
      expect(fts.size).toBe(1)
      const ft = fts.get('gs1')!
      expect(ft.reading).toBe('StateMachine is for Resource')
      expect(ft.roles).toHaveLength(2)
    })

    it('includes core constraints and spans', async () => {
      const model = new DomainModel(
        mockLoader({
          constraints: [
            { id: 'c1', kind: 'UC', modality: 'Alethic', domain_id: CORE_DOMAIN_ID, text: 'Each Resource has at most one StateMachine' },
          ],
          constraintSpans: [
            { id: 'cs1', constraint_id: 'c1', role_id: 'r1', domain_id: CORE_DOMAIN_ID, graph_schema_id: 'gs1', role_index: 0 },
          ],
        }),
        DOMAIN,
      )

      const cs = await model.constraints()
      expect(cs).toHaveLength(1)
      expect(cs[0].kind).toBe('UC')
      expect(cs[0].spans).toHaveLength(1)
    })

    it('excludes roles from unrelated domains (not domain or core)', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Customer', object_type: 'entity', domain_id: DOMAIN },
          ],
          graphSchemas: [
            { id: 'gs1', name: 'CustomerHasName', domain_id: DOMAIN },
          ],
          readings: [
            { id: 'rd1', text: 'Customer has Name', graph_schema_id: 'gs1', domain_id: DOMAIN },
          ],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: DOMAIN },
            { id: 'r-other', noun_id: 'n99', graph_schema_id: 'gs99', role_index: 0, domain_id: 'other-domain' },
          ],
        }),
        DOMAIN,
      )

      const fts = await model.factTypes()
      // Only 1 fact type from our domain — the other-domain role is filtered out
      expect(fts.size).toBe(1)
    })

    it('merges core and domain readings in readings()', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'SupportRequest', object_type: 'entity', domain_id: DOMAIN },
          ],
          readings: [
            { id: 'rd1', text: 'Resource has Status', graph_schema_id: 'gs1', domain_id: CORE_DOMAIN_ID },
            { id: 'rd2', text: 'SupportRequest has Priority', graph_schema_id: 'gs2', domain_id: DOMAIN },
          ],
          roles: [
            { id: 'r1', noun_id: 'n1', graph_schema_id: 'gs1', role_index: 0, domain_id: CORE_DOMAIN_ID },
            { id: 'r2', noun_id: 'n2', graph_schema_id: 'gs2', role_index: 0, domain_id: DOMAIN },
          ],
        }),
        DOMAIN,
      )

      const readings = await model.readings()
      expect(readings).toHaveLength(2)
      const texts = readings.map(r => r.text)
      expect(texts).toContain('Resource has Status')
      expect(texts).toContain('SupportRequest has Priority')
    })
  })

  // ---------------------------------------------------------------------------
  // Default supertype assignment
  // ---------------------------------------------------------------------------
  describe('default supertype', () => {
    it('assigns Resource as supertype to domain entity nouns without one', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'Request', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n3', name: 'SupportRequest', object_type: 'entity', domain_id: DOMAIN, super_type_name: 'Request' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      // Request has no explicit supertype → defaults to Resource
      expect(nouns.get('Request')?.superType).toBe(DEFAULT_ENTITY_SUPERTYPE)
      // SupportRequest already has an explicit supertype → unchanged
      expect(nouns.get('SupportRequest')?.superType).toBe('Request')
      // Resource is a core root → no supertype
      expect(nouns.get('Resource')?.superType).toBeUndefined()
    })

    it('does not assign supertype to value types', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'Priority', object_type: 'value', domain_id: DOMAIN, value_type: 'string' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      expect(nouns.get('Priority')?.superType).toBeUndefined()
    })

    it('does not assign supertype to core domain nouns', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Graph Schema', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'Reading', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
          ],
        }),
        CORE_DOMAIN_ID,
      )

      const nouns = await model.nouns()
      expect(nouns.get('Graph Schema')?.superType).toBeUndefined()
      expect(nouns.get('Reading')?.superType).toBeUndefined()
    })

    it('does not self-reference root entities', async () => {
      for (const root of DEFAULT_SUPERTYPE_ROOTS) {
        const model = new DomainModel(
          mockLoader({
            nouns: [
              { id: 'n1', name: root, object_type: 'entity', domain_id: DOMAIN },
            ],
          }),
          DOMAIN,
        )

        const nouns = await model.nouns()
        expect(nouns.get(root)?.superType).toBeUndefined()
      }
    })

    it('creates full chain: SupportRequest → Request → Resource', async () => {
      const model = new DomainModel(
        mockLoader({
          nouns: [
            { id: 'n1', name: 'Resource', object_type: 'entity', domain_id: CORE_DOMAIN_ID },
            { id: 'n2', name: 'Request', object_type: 'entity', domain_id: DOMAIN },
            { id: 'n3', name: 'SupportRequest', object_type: 'entity', domain_id: DOMAIN, super_type_name: 'Request' },
          ],
        }),
        DOMAIN,
      )

      const nouns = await model.nouns()
      expect(nouns.get('SupportRequest')?.superType).toBe('Request')
      expect(nouns.get('Request')?.superType).toBe('Resource')
      expect(nouns.get('Resource')?.superType).toBeUndefined()
    })
  })
})
