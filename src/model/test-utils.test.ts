import { describe, it, expect, beforeEach } from 'vitest'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkConstraint, resetIds } from './test-utils'

describe('test-utils', () => {
  beforeEach(() => resetIds())

  describe('factory functions', () => {
    it('mkNounDef creates entity nouns with auto IDs', () => {
      const n1 = mkNounDef({ name: 'Customer' })
      const n2 = mkNounDef({ name: 'Order' })
      expect(n1.id).not.toBe(n2.id)
      expect(n1.objectType).toBe('entity')
      expect(n1.name).toBe('Customer')
    })

    it('mkValueNounDef creates value nouns', () => {
      const n = mkValueNounDef({ name: 'Email', valueType: 'string' })
      expect(n.objectType).toBe('value')
      expect(n.valueType).toBe('string')
    })

    it('mkFactType auto-fills role IDs and nounNames', () => {
      const customer = mkNounDef({ name: 'Customer' })
      const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
      const ft = mkFactType({
        reading: 'Customer has Name',
        roles: [
          { nounDef: customer, roleIndex: 0 },
          { nounDef: name, roleIndex: 1 },
        ],
      })
      expect(ft.arity).toBe(2)
      expect(ft.roles[0].nounName).toBe('Customer')
      expect(ft.roles[1].nounName).toBe('Name')
      expect(ft.roles[0].id).toBeTruthy()
    })

    it('mkConstraint defaults to Alethic modality', () => {
      const c = mkConstraint({ kind: 'UC', spans: [{ factTypeId: 'gs1', roleIndex: 1 }] })
      expect(c.modality).toBe('Alethic')
      expect(c.kind).toBe('UC')
    })
  })

  describe('createMockModel', () => {
    it('returns a DomainModel-compatible object', async () => {
      const customer = mkNounDef({ name: 'Customer' })
      const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
      const ft = mkFactType({
        reading: 'Customer has Name',
        roles: [
          { nounDef: customer, roleIndex: 0 },
          { nounDef: name, roleIndex: 1 },
        ],
      })
      const uc = mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft.id, roleIndex: 1 }] })

      const model = createMockModel({ nouns: [customer, name], factTypes: [ft], constraints: [uc] })

      const nouns = await model.nouns()
      expect(nouns.size).toBe(2)
      expect(nouns.get('Customer')?.objectType).toBe('entity')

      const fts = await model.factTypesFor(customer)
      expect(fts).toHaveLength(1)

      const cs = await model.constraintsFor([ft])
      expect(cs).toHaveLength(1)
      expect(cs[0].kind).toBe('UC')
    })
  })
})
