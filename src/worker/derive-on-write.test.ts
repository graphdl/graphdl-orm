import { describe, it, expect, vi } from 'vitest'
import { deriveOnWrite, type DeriveContext } from './derive-on-write'

describe('deriveOnWrite', () => {
  it('derives affect region on Layer State write', async () => {
    const writes: Array<{ entityId: string; field: string; value: string }> = []

    const ctx: DeriveContext = {
      entity: {
        id: 'ls1',
        type: 'Layer State',
        data: { Valence: '0.7', Arousal: '0.8', Timestamp: '2026-03-21' },
      },
      loadDerivationRules: async () => [
        { text: "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3." },
        { text: "Layer State has Affect Region 'Calm' := Layer State has Valence > 0.3 and Layer State has Arousal < -0.3." },
      ],
      loadNouns: async () => ['Layer State', 'Valence', 'Arousal', 'Affect Region', 'Timestamp'],
      loadRelatedFacts: async () => [],
      writeDerivedFact: async (entityId, field, value) => {
        writes.push({ entityId, field, value })
      },
    }

    const result = await deriveOnWrite(ctx)
    expect(result.derivedCount).toBe(1)
    expect(result.derived[0].subject).toBe('ls1')
    expect(result.derived[0].object).toBe('Excited')
    expect(writes).toHaveLength(1)
    expect(writes[0]).toEqual({ entityId: 'ls1', field: 'Affect Region', value: 'Excited' })
  })

  it('produces no derivations when no rules match', async () => {
    const ctx: DeriveContext = {
      entity: {
        id: 'ls2',
        type: 'Layer State',
        data: { Valence: '0.1', Arousal: '0.1' },
      },
      loadDerivationRules: async () => [
        { text: "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3." },
      ],
      loadNouns: async () => ['Layer State', 'Valence', 'Arousal', 'Affect Region'],
      loadRelatedFacts: async () => [],
      writeDerivedFact: vi.fn(),
    }

    const result = await deriveOnWrite(ctx)
    expect(result.derivedCount).toBe(0)
    expect(ctx.writeDerivedFact).not.toHaveBeenCalled()
  })

  it('skips when no derivation rules exist', async () => {
    const ctx: DeriveContext = {
      entity: { id: 'e1', type: 'Customer', data: { name: 'Alice' } },
      loadDerivationRules: async () => [],
      loadNouns: async () => ['Customer', 'Name'],
      loadRelatedFacts: async () => [],
      writeDerivedFact: vi.fn(),
    }

    const result = await deriveOnWrite(ctx)
    expect(result.derivedCount).toBe(0)
  })
})
