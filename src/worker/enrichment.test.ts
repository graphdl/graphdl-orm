import { describe, it, expect, vi } from 'vitest'
import { enrichEntity, type AutofillConstraint, type EnrichmentContext } from './enrichment'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeCtx(
  constraints: AutofillConstraint[],
  lookupMap: Record<string, Record<string, Record<string, string[]>>> = {},
): EnrichmentContext {
  return {
    constraints,
    resolveEntities: vi.fn(async (nounType: string, field: string, value: string) => {
      return lookupMap[nounType]?.[field]?.[value] ?? []
    }),
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('enrichEntity', () => {
  it('returns data unchanged when there are no constraints', async () => {
    const data = { phoneNumber: '+1234', body: 'hello' }
    const ctx = makeCtx([])

    const result = await enrichEntity(data, ctx)

    expect(result).toEqual(data)
    expect(ctx.resolveEntities).not.toHaveBeenCalled()
  })

  it('resolves a single match (phone -> customer)', async () => {
    const constraint: AutofillConstraint = {
      sourceFactTypeId: 'message-has-phone',
      sourceField: 'phoneNumber',
      targetNounType: 'Customer',
      targetField: 'phoneNumber',
      derivedField: 'customerId',
    }
    const ctx = makeCtx([constraint], {
      Customer: { phoneNumber: { '+1234': ['cust-42'] } },
    })

    const result = await enrichEntity({ phoneNumber: '+1234', body: 'hi' }, ctx)

    expect(result).toEqual({ phoneNumber: '+1234', body: 'hi', customerId: 'cust-42' })
    expect(ctx.resolveEntities).toHaveBeenCalledWith('Customer', 'phoneNumber', '+1234')
  })

  it('handles no match (field value not found)', async () => {
    const constraint: AutofillConstraint = {
      sourceFactTypeId: 'message-has-phone',
      sourceField: 'phoneNumber',
      targetNounType: 'Customer',
      targetField: 'phoneNumber',
      derivedField: 'customerId',
    }
    const ctx = makeCtx([constraint], {
      Customer: { phoneNumber: {} },
    })

    const result = await enrichEntity({ phoneNumber: '+9999', body: 'hi' }, ctx)

    expect(result).toEqual({ phoneNumber: '+9999', body: 'hi' })
    expect(result).not.toHaveProperty('customerId')
  })

  it('handles multiple matches (returns array)', async () => {
    const constraint: AutofillConstraint = {
      sourceFactTypeId: 'order-has-email',
      sourceField: 'email',
      targetNounType: 'Account',
      targetField: 'email',
      derivedField: 'accountId',
    }
    const ctx = makeCtx([constraint], {
      Account: { email: { 'a@b.com': ['acct-1', 'acct-2'] } },
    })

    const result = await enrichEntity({ email: 'a@b.com' }, ctx)

    expect(result).toEqual({ email: 'a@b.com', accountId: ['acct-1', 'acct-2'] })
  })

  it('skips non-string source fields', async () => {
    const constraint: AutofillConstraint = {
      sourceFactTypeId: 'event-has-count',
      sourceField: 'count',
      targetNounType: 'Metric',
      targetField: 'count',
      derivedField: 'metricId',
    }
    const ctx = makeCtx([constraint])

    const result = await enrichEntity({ count: 42 }, ctx)

    expect(result).toEqual({ count: 42 })
    expect(ctx.resolveEntities).not.toHaveBeenCalled()
  })

  it('is pure — original data not mutated', async () => {
    const constraint: AutofillConstraint = {
      sourceFactTypeId: 'message-has-phone',
      sourceField: 'phoneNumber',
      targetNounType: 'Customer',
      targetField: 'phoneNumber',
      derivedField: 'customerId',
    }
    const ctx = makeCtx([constraint], {
      Customer: { phoneNumber: { '+1234': ['cust-7'] } },
    })

    const original = { phoneNumber: '+1234' }
    const result = await enrichEntity(original, ctx)

    expect(result).toHaveProperty('customerId', 'cust-7')
    expect(original).not.toHaveProperty('customerId')
    expect(original).toEqual({ phoneNumber: '+1234' })
  })

  it('resolves all constraints when multiple are provided', async () => {
    const constraints: AutofillConstraint[] = [
      {
        sourceFactTypeId: 'message-has-phone',
        sourceField: 'phoneNumber',
        targetNounType: 'Customer',
        targetField: 'phoneNumber',
        derivedField: 'customerId',
      },
      {
        sourceFactTypeId: 'message-has-region',
        sourceField: 'regionCode',
        targetNounType: 'Region',
        targetField: 'code',
        derivedField: 'regionId',
      },
    ]
    const ctx = makeCtx(constraints, {
      Customer: { phoneNumber: { '+1234': ['cust-42'] } },
      Region: { code: { 'US-CA': ['reg-5'] } },
    })

    const result = await enrichEntity(
      { phoneNumber: '+1234', regionCode: 'US-CA', body: 'hello' },
      ctx,
    )

    expect(result).toEqual({
      phoneNumber: '+1234',
      regionCode: 'US-CA',
      body: 'hello',
      customerId: 'cust-42',
      regionId: 'reg-5',
    })
    expect(ctx.resolveEntities).toHaveBeenCalledTimes(2)
  })
})
