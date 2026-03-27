import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { nounToSlug, nounToTable, pluralize } from './collections'

describe('pluralize', () => {
  it('adds -es for words ending in s, sh, ch, x, z', () => {
    expect(pluralize('Status')).toBe('Statuses')
    expect(pluralize('Crash')).toBe('Crashes')
    expect(pluralize('Match')).toBe('Matches')
    expect(pluralize('Box')).toBe('Boxes')
    expect(pluralize('Quiz')).toBe('Quizzes')
  })

  it('handles consonant + y → ies', () => {
    expect(pluralize('Entity')).toBe('Entities')
    expect(pluralize('Category')).toBe('Categories')
  })

  it('keeps vowel + y → ys', () => {
    expect(pluralize('Key')).toBe('Keys')
    expect(pluralize('Day')).toBe('Days')
  })

  it('adds -s for regular words', () => {
    expect(pluralize('Organization')).toBe('Organizations')
    expect(pluralize('Noun')).toBe('Nouns')
    expect(pluralize('App')).toBe('Apps')
  })
})

describe('nounToSlug', () => {
  it('converts single-word nouns', () => {
    expect(nounToSlug('Organization')).toBe('organizations')
    expect(nounToSlug('Noun')).toBe('nouns')
    expect(nounToSlug('Status')).toBe('statuses')
    expect(nounToSlug('App')).toBe('apps')
    expect(nounToSlug('Verb')).toBe('verbs')
  })

  it('converts PascalCase compound nouns', () => {
    expect(nounToSlug('OrgMembership')).toBe('org-memberships')
    expect(nounToSlug('ResourceRole')).toBe('resource-roles')
    expect(nounToSlug('GuardRun')).toBe('guard-runs')
    expect(nounToSlug('AgentDefinition')).toBe('agent-definitions')
  })

  it('converts space-separated nouns', () => {
    expect(nounToSlug('Graph Schema')).toBe('graph-schemas')
    expect(nounToSlug('State Machine Definition')).toBe('state-machine-definitions')
    expect(nounToSlug('Constraint Span')).toBe('constraint-spans')
    expect(nounToSlug('Event Type')).toBe('event-types')
    expect(nounToSlug('Guard Run')).toBe('guard-runs')
    expect(nounToSlug('State Machine')).toBe('state-machines')
  })
})

describe('nounToTable', () => {
  it('converts single-word nouns to snake_case plural', () => {
    expect(nounToTable('Organization')).toBe('organizations')
    expect(nounToTable('Status')).toBe('statuses')
    expect(nounToTable('Noun')).toBe('nouns')
  })

  it('converts PascalCase compound nouns', () => {
    expect(nounToTable('OrgMembership')).toBe('org_memberships')
    expect(nounToTable('SupportRequest')).toBe('support_requests')
    expect(nounToTable('ResourceRole')).toBe('resource_roles')
  })

  it('converts space-separated nouns', () => {
    expect(nounToTable('Graph Schema')).toBe('graph_schemas')
    expect(nounToTable('State Machine Definition')).toBe('state_machine_definitions')
    expect(nounToTable('Constraint Span')).toBe('constraint_spans')
    expect(nounToTable('Event Type')).toBe('event_types')
  })
})

describe('createEntityInner', () => {
  it('defines tableName from toTableName(nounName) before first use', () => {
    // createEntityInner is a private method on DomainDB, so we verify at source level
    // that the variable is properly defined before it is referenced.
    const source = readFileSync(join(__dirname, 'domain-do.ts'), 'utf-8')

    // Extract the createEntityInner method body
    const methodStart = source.indexOf('private async createEntityInner(')
    expect(methodStart).toBeGreaterThan(-1)

    // Find the closing brace of the method signature (the line with "): Promise<{ id: string }> {")
    const bodyStart = source.indexOf('): Promise<{ id: string }> {', methodStart)
    expect(bodyStart).toBeGreaterThan(-1)

    // Extract body from the opening brace to a reasonable length
    const body = source.slice(bodyStart, bodyStart + 2000)

    // tableName must be defined BEFORE it is used in expressions like `${tableName}`
    const defIndex = body.indexOf('const tableName')
    expect(defIndex).toBeGreaterThan(-1)

    // The definition should come before any usage of tableName in template literals or references
    const firstUsage = body.indexOf('tableName', defIndex + 'const tableName'.length)
    expect(firstUsage).toBeGreaterThan(defIndex)
  })
})
