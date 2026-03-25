import { describe, it, expect, vi } from 'vitest'
import { deriveSemanticFlags } from './derive-semantic'

describe('deriveSemanticFlags', () => {
  function makeCtx(entities: Record<string, any>, entityIdsByType: Record<string, string[]>) {
    return {
      getEntityIds: vi.fn().mockImplementation((type: string, _domain?: string) =>
        Promise.resolve(entityIdsByType[type] || [])
      ),
      getEntity: vi.fn().mockImplementation((id: string) =>
        Promise.resolve(entities[id] || null)
      ),
      patchEntity: vi.fn().mockResolvedValue(undefined),
    }
  }

  it('marks deontic constraint as semantic when noun has no instances', async () => {
    const entities: Record<string, any> = {
      'c1': { id: 'c1', type: 'Constraint', data: { modality: 'Deontic', kind: 'UC' } },
      'cs1': { id: 'cs1', type: 'ConstraintSpan', data: { constraintId: 'c1', roleId: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { nounId: 'n1' } },
      'n1': { id: 'n1', type: 'Noun', data: { name: 'ProhibitedPattern', objectType: 'value' } },
    }
    const ids = {
      'Constraint': ['c1'],
      'ConstraintSpan': ['cs1'],
      'Role': ['r1'],
      'Noun': ['n1'],
      'Resource': [], // no instances
    }

    const ctx = makeCtx(entities, ids)
    const result = await deriveSemanticFlags('test', ctx)

    expect(result.semantic).toBe(1)
    expect(result.deterministic).toBe(0)
    expect(ctx.patchEntity).toHaveBeenCalledWith('c1', { isSemantic: true })
  })

  it('marks deontic constraint as deterministic when noun has instances', async () => {
    const entities: Record<string, any> = {
      'c1': { id: 'c1', type: 'Constraint', data: { modality: 'Deontic', kind: 'MC' } },
      'cs1': { id: 'cs1', type: 'ConstraintSpan', data: { constraintId: 'c1', roleId: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { nounId: 'n1' } },
      'n1': { id: 'n1', type: 'Noun', data: { name: 'Customer', objectType: 'entity' } },
      'res1': { id: 'res1', type: 'Resource', data: { nounId: 'n1' } },
    }
    const ids = {
      'Constraint': ['c1'],
      'ConstraintSpan': ['cs1'],
      'Role': ['r1'],
      'Noun': ['n1'],
      'Resource': ['res1'],
    }

    const ctx = makeCtx(entities, ids)
    const result = await deriveSemanticFlags('test', ctx)

    expect(result.semantic).toBe(0)
    expect(result.deterministic).toBe(1)
    expect(ctx.patchEntity).toHaveBeenCalledWith('c1', { isSemantic: false })
  })

  it('skips alethic constraints', async () => {
    const entities: Record<string, any> = {
      'c1': { id: 'c1', type: 'Constraint', data: { modality: 'Alethic', kind: 'UC' } },
    }
    const ids = { 'Constraint': ['c1'], 'ConstraintSpan': [], 'Role': [], 'Noun': [], 'Resource': [] }

    const ctx = makeCtx(entities, ids)
    const result = await deriveSemanticFlags('test', ctx)

    expect(result.semantic).toBe(0)
    expect(result.deterministic).toBe(0)
    expect(ctx.patchEntity).not.toHaveBeenCalled()
  })

  it('treats constraint with no spans as semantic', async () => {
    const entities: Record<string, any> = {
      'c1': { id: 'c1', type: 'Constraint', data: { modality: 'Deontic', kind: 'UC' } },
    }
    const ids = { 'Constraint': ['c1'], 'ConstraintSpan': [], 'Role': [], 'Noun': [], 'Resource': [] }

    const ctx = makeCtx(entities, ids)
    const result = await deriveSemanticFlags('test', ctx)

    expect(result.semantic).toBe(1)
    expect(ctx.patchEntity).toHaveBeenCalledWith('c1', { isSemantic: true })
  })

  it('does not patch if value unchanged', async () => {
    const entities: Record<string, any> = {
      'c1': { id: 'c1', type: 'Constraint', data: { modality: 'Deontic', kind: 'UC', isSemantic: true } },
    }
    const ids = { 'Constraint': ['c1'], 'ConstraintSpan': [], 'Role': [], 'Noun': [], 'Resource': [] }

    const ctx = makeCtx(entities, ids)
    await deriveSemanticFlags('test', ctx)

    expect(ctx.patchEntity).not.toHaveBeenCalled()
  })
})
