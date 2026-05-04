import { describe, expect, it } from 'vitest'

import {
  DEFAULT_MUTATION_PROMPTS,
  buildMutationContext,
  enforceMutationContext,
  mutationModelingViolations,
} from './mutation-context.js'

function testContext() {
  return buildMutationContext({
    prompts: DEFAULT_MUTATION_PROMPTS.map((prompt) => ({
      ...prompt,
      text: `${prompt.name} prompt body`,
    })),
  })
}

describe('MCP mutation context gate', () => {
  it('builds a receipt from the prompt bundle and rules', () => {
    const context = testContext()

    expect(context.version).toBe('arest-context:v1')
    expect(context.receipt).toMatch(/^arest-context:v1:fnv1a64:[0-9a-f]{16}$/)
    expect(context.receipt_applies_to).toContain('apply')
    expect(context.prompt_bundle.map((p) => p.name)).toEqual([
      'overview',
      'design-principles',
      'entity-modeling',
      'verbalization',
      'advanced-constraints',
      'derivation-deontic',
      'api',
    ])
    expect(context.prompt_bundle.map((p) => p.name)).toContain('entity-modeling')
    expect(context.rules).toContain('The selected AREST app and active fact storage medium are the implicit Universe of Discourse: do not create a UoD meta-fact just to scope ordinary facts.')
    expect(JSON.stringify(context)).not.toContain('app-design')
    expect(JSON.stringify(context)).not.toContain('operation_protocol')
    expect(JSON.stringify(context)).not.toContain('universe_of_discourse')
  })

  it('binds receipts to app scope when scope is supplied', () => {
    const codex = buildMutationContext({
      prompts: DEFAULT_MUTATION_PROMPTS.map((prompt) => ({
        ...prompt,
        text: `${prompt.name} prompt body`,
      })),
      scope: { app: 'codex', db: 'C:/apps/codex/codex-arest.db', readingsDir: 'C:/apps/codex/readings' },
    })
    const support = buildMutationContext({
      prompts: DEFAULT_MUTATION_PROMPTS.map((prompt) => ({
        ...prompt,
        text: `${prompt.name} prompt body`,
      })),
      scope: { app: 'support', db: 'C:/apps/support/support.db', readingsDir: 'C:/apps/support/readings' },
    })

    expect(codex.scope?.app).toBe('codex')
    expect(support.scope?.app).toBe('support')
    expect(codex.receipt).not.toBe(support.receipt)
  })

  it('rejects missing and stale receipts before mutation', () => {
    const context = testContext()

    const missing = enforceMutationContext({
      tool: 'apply',
      context,
      payload: { operation: 'create', noun: 'Order', fields: { Name: 'A-1' } },
    })
    const stale = enforceMutationContext({
      tool: 'compile',
      receivedReceipt: 'arest-context:v1:fnv1a64:deadbeefdeadbeef',
      context,
      payload: { readings: 'Order(.Order Id) is an entity type.' },
    })

    expect(missing.ok).toBe(false)
    expect(missing.ok ? '' : missing.error.error).toBe('context_receipt_required')
    expect(stale.ok).toBe(false)
    expect(stale.ok ? '' : stale.error.error).toBe('context_receipt_stale')
  })

  it('accepts a current receipt for a typed mutation', () => {
    const context = testContext()

    const result = enforceMutationContext({
      tool: 'compile',
      receivedReceipt: context.receipt,
      context,
      payload: {
        readings: [
          'Order(.Order Id) is an entity type.',
          'Customer(.Name) is an entity type.',
          'Order was placed by Customer.',
        ].join('\n'),
      },
    })

    expect(result.ok).toBe(true)
  })

  it('rejects known prose-memory anti-patterns', () => {
    expect(mutationModelingViolations('compile', {
      readings: 'Fact Note(.Note Id) is an entity type.\nFact Note has Note Text.',
    })).toContain('readings contain catch-all prose-memory terms: fact note, note text')

    expect(mutationModelingViolations('apply', {
      operation: 'create',
      noun: 'Codex Memory',
      fields: { 'User Text': 'remember this paragraph' },
    })).toContain('fields contain catch-all prose-memory names: User Text')
  })
})
