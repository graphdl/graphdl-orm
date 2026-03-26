import { describe, it, expect } from 'vitest'
import { checkDeterministicText, buildTextConstraints, type TextConstraint } from './deterministic-text-check'

describe('checkDeterministicText', () => {
  it('catches forbidden markdown syntax', () => {
    const constraints: TextConstraint[] = [{
      constraintId: 'c1',
      text: 'It is forbidden that Support Response contains Markdown Syntax.',
      operator: 'forbidden',
      values: ['**', '##', '- ', '```'],
    }]

    const text = '**Pricing:** Our Scale plan costs $599/month.'
    const violations = checkDeterministicText(text, constraints)

    expect(violations.length).toBeGreaterThanOrEqual(1)
    expect(violations[0].value).toBe('**')
    expect(violations[0].evidence).toContain('**Pricing:**')
  })

  it('catches forbidden dashes', () => {
    const constraints: TextConstraint[] = [{
      constraintId: 'c2',
      text: 'It is forbidden that Support Response uses Dash.',
      operator: 'forbidden',
      values: ['—', '–', '--'],
    }]

    const text = 'We offer -- among other things -- great support.'
    const violations = checkDeterministicText(text, constraints)

    expect(violations.length).toBe(1)
    expect(violations[0].value).toBe('--')
  })

  it('catches forbidden paragraph titles', () => {
    const constraints: TextConstraint[] = [{
      constraintId: 'c3',
      text: 'It is forbidden that Support Response contains Paragraph Title.',
      operator: 'forbidden',
      values: [':**', ':** '],
    }]

    const text = '**Pricing:** Details below.'
    const violations = checkDeterministicText(text, constraints)

    expect(violations.some(v => v.value === ':** ')).toBe(true)
  })

  it('obligatory constraints report missing values when passed directly', () => {
    // Note: buildTextConstraints filters out obligatory constraints.
    // But if passed directly to checkDeterministicText, they still work.
    const constraints: TextConstraint[] = [{
      constraintId: 'c4',
      text: 'It is obligatory that Support Response is delivered via Channel Name Email.',
      operator: 'obligatory',
      values: ['Email'],
    }]

    const text = 'We sent you a letter.'
    const violations = checkDeterministicText(text, constraints)

    expect(violations.length).toBe(1)
    expect(violations[0].operator).toBe('obligatory')
  })

  it('no violations when text is clean', () => {
    const constraints: TextConstraint[] = [{
      constraintId: 'c1',
      text: 'It is forbidden that Support Response contains Markdown Syntax.',
      operator: 'forbidden',
      values: ['**', '##', '```'],
    }]

    const text = 'Hello, the API returns year, make, and model.'
    const violations = checkDeterministicText(text, constraints)

    expect(violations.length).toBe(0)
  })

  it('finds multiple violations from same constraint', () => {
    const constraints: TextConstraint[] = [{
      constraintId: 'c1',
      text: 'It is forbidden that Support Response contains Markdown Syntax.',
      operator: 'forbidden',
      values: ['**', '- '],
    }]

    const text = '**Pricing:** Plans start at:\n- Free\n- Starter\n- Growth'
    const violations = checkDeterministicText(text, constraints)

    // ** found once, - found three times
    expect(violations.length).toBeGreaterThanOrEqual(2)
  })
})

describe('buildTextConstraints', () => {
  it('builds constraints from deontic constraints referencing nouns with enums', () => {
    const constraints = [{
      id: 'c1',
      data: {
        modality: 'Deontic',
        text: 'It is forbidden that Support Response contains Markdown Syntax.',
      },
    }]
    const nouns = [{
      id: 'n1',
      data: { name: 'Markdown Syntax', objectType: 'value', enumValues: '["**","##","- "]' },
    }]

    const result = buildTextConstraints(constraints, nouns)
    expect(result.length).toBe(1)
    expect(result[0].operator).toBe('forbidden')
    expect(result[0].values).toEqual(['**', '##', '- '])
  })

  it('skips alethic constraints', () => {
    const constraints = [{
      id: 'c1',
      data: { modality: 'Alethic', text: 'Each X has at most one Y.' },
    }]
    const nouns = [{ id: 'n1', data: { name: 'Y', enumValues: '["a"]' } }]

    expect(buildTextConstraints(constraints, nouns).length).toBe(0)
  })

  it('skips nouns without enum values', () => {
    const constraints = [{
      id: 'c1',
      data: { modality: 'Deontic', text: 'It is forbidden that X reveals Implementation Detail.' },
    }]
    const nouns = [{ id: 'n1', data: { name: 'Implementation Detail', objectType: 'value' } }]

    expect(buildTextConstraints(constraints, nouns).length).toBe(0)
  })

  it('handles array enum values', () => {
    const constraints = [{
      id: 'c1',
      data: { modality: 'Deontic', text: 'It is forbidden that Response uses Dash.' },
    }]
    const nouns = [{
      id: 'n1',
      data: { name: 'Dash', objectType: 'value', enum: ['—', '–', '--'] },
    }]

    const result = buildTextConstraints(constraints, nouns)
    expect(result[0].values).toEqual(['—', '–', '--'])
  })
})
