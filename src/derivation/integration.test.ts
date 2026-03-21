/**
 * Integration tests: parse REAL spd-1 derivation rules and forward-chain
 * affect region derivation end-to-end.
 */
import { describe, it, expect } from 'vitest'
import { parseRule } from './parse-rule'
import { forwardChain, type Fact, type FactStore } from './forward-chain'
import { parseFORML2 } from '../api/parse'

// ─── Domain constants ────────────────────────────────────────────────

const AFFECT_NOUNS = [
  'Layer State', 'Valence', 'Arousal', 'Affect Region',
  'Graph', 'Layer', 'Resource', 'Role', 'Reading', 'Noun', 'Timestamp',
]

// Real derivation rules from spd-1/domains/affect.md
const STIMULUS_RULE =
  'Graph stimulates Layer := Graph uses Resource for Role and Role belongs to Reading and Reading references Noun and Layer owns Noun.'

const EXCITED_RULE =
  "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3."

const CALM_RULE =
  "Layer State has Affect Region 'Calm' := Layer State has Valence > 0.3 and Layer State has Arousal < -0.3."

const TENSE_RULE =
  "Layer State has Affect Region 'Tense' := Layer State has Valence < -0.3 and Layer State has Arousal > 0.3."

const DEPRESSED_RULE =
  "Layer State has Affect Region 'Depressed' := Layer State has Valence < -0.3 and Layer State has Arousal < -0.3."

// ─── Helpers ─────────────────────────────────────────────────────────

function fact(
  subject: string,
  subjectType: string,
  predicate: string,
  object: string,
  objectType: string,
): Fact {
  return { subject, subjectType, predicate, object, objectType }
}

// ─── Test 1: Parse the stimulus routing rule ─────────────────────────

describe('stimulus routing rule', () => {
  it('parses into a 4-antecedent join with Graph -> Layer consequent', () => {
    const rule = parseRule(STIMULUS_RULE, AFFECT_NOUNS)

    expect(rule.kind).toBe('join')
    expect(rule.antecedents).toHaveLength(4)

    // Consequent: Graph stimulates Layer
    expect(rule.consequent.subject).toBe('Graph')
    expect(rule.consequent.predicate).toBe('stimulates')
    expect(rule.consequent.object).toBe('Layer')

    // Antecedent 1: Graph uses Resource for Role
    expect(rule.antecedents[0].subject).toBe('Graph')
    expect(rule.antecedents[0].predicate).toBe('uses')
    expect(rule.antecedents[0].object).toBe('Resource')
    expect(rule.antecedents[0].qualifier).toEqual({ predicate: 'for', object: 'Role' })

    // Antecedent 2: Role belongs to Reading
    expect(rule.antecedents[1].subject).toBe('Role')
    expect(rule.antecedents[1].predicate).toBe('belongs to')
    expect(rule.antecedents[1].object).toBe('Reading')

    // Antecedent 3: Reading references Noun
    expect(rule.antecedents[2].subject).toBe('Reading')
    expect(rule.antecedents[2].predicate).toBe('references')
    expect(rule.antecedents[2].object).toBe('Noun')

    // Antecedent 4: Layer owns Noun
    expect(rule.antecedents[3].subject).toBe('Layer')
    expect(rule.antecedents[3].predicate).toBe('owns')
    expect(rule.antecedents[3].object).toBe('Noun')
  })
})

// ─── Test 2: Derive Affect Region from Valence and Arousal ───────────

describe('single affect region derivation (Excited)', () => {
  it('derives Excited only for layer states meeting both thresholds', () => {
    const rule = parseRule(EXCITED_RULE, AFFECT_NOUNS)

    // ls1: Valence=0.7, Arousal=0.8 -> should derive Excited
    // ls2: Valence=-0.5, Arousal=0.8 -> should NOT derive Excited
    const store: FactStore = {
      facts: [
        fact('ls1', 'Layer State', 'has', '0.7', 'Valence'),
        fact('ls1', 'Layer State', 'has', '0.8', 'Arousal'),
        fact('ls2', 'Layer State', 'has', '-0.5', 'Valence'),
        fact('ls2', 'Layer State', 'has', '0.8', 'Arousal'),
      ],
    }

    const derived = forwardChain([rule], store)

    // Only ls1 should get Excited
    expect(derived).toHaveLength(1)
    expect(derived[0]).toMatchObject({
      subject: 'ls1',
      subjectType: 'Layer State',
      predicate: 'has',
      object: 'Excited',
      objectType: 'Affect Region',
      derived: true,
    })
  })
})

// ─── Test 3: Derive multiple affect regions ──────────────────────────

describe('multiple affect region derivation', () => {
  it('classifies layer states into Excited and Calm regions', () => {
    const excitedRule = parseRule(EXCITED_RULE, AFFECT_NOUNS)
    const calmRule = parseRule(CALM_RULE, AFFECT_NOUNS)

    // ls1: Valence=0.7, Arousal=0.8 -> Excited
    // ls2: Valence=0.5, Arousal=-0.6 -> Calm
    const store: FactStore = {
      facts: [
        fact('ls1', 'Layer State', 'has', '0.7', 'Valence'),
        fact('ls1', 'Layer State', 'has', '0.8', 'Arousal'),
        fact('ls2', 'Layer State', 'has', '0.5', 'Valence'),
        fact('ls2', 'Layer State', 'has', '-0.6', 'Arousal'),
      ],
    }

    const derived = forwardChain([excitedRule, calmRule], store)

    expect(derived).toHaveLength(2)

    const excited = derived.find(f => f.object === 'Excited')
    expect(excited).toBeDefined()
    expect(excited!.subject).toBe('ls1')
    expect(excited!.subjectType).toBe('Layer State')
    expect(excited!.objectType).toBe('Affect Region')
    expect(excited!.derived).toBe(true)

    const calm = derived.find(f => f.object === 'Calm')
    expect(calm).toBeDefined()
    expect(calm!.subject).toBe('ls2')
    expect(calm!.subjectType).toBe('Layer State')
    expect(calm!.objectType).toBe('Affect Region')
    expect(calm!.derived).toBe(true)
  })

  it('does not derive any region for neutral valence/arousal', () => {
    const rules = [
      parseRule(EXCITED_RULE, AFFECT_NOUNS),
      parseRule(CALM_RULE, AFFECT_NOUNS),
      parseRule(TENSE_RULE, AFFECT_NOUNS),
      parseRule(DEPRESSED_RULE, AFFECT_NOUNS),
    ]

    // ls3: Valence=0.1, Arousal=0.1 -> no region (within dead zone)
    const store: FactStore = {
      facts: [
        fact('ls3', 'Layer State', 'has', '0.1', 'Valence'),
        fact('ls3', 'Layer State', 'has', '0.1', 'Arousal'),
      ],
    }

    const derived = forwardChain(rules, store)
    expect(derived).toHaveLength(0)
  })
})

// ─── Test 4: Parse full FORML2 text with derivation rules ────────────

describe('parseFORML2 captures derivation rules with ruleIR', () => {
  it('parses affect domain FORML2 and attaches ruleIR to derivation readings', () => {
    // Minimal FORML2 document representing the affect domain
    const affectFORML2 = `# Affect

## Entity Types
Layer State(.Layer, .Timestamp) is an entity type.

## Value Types
Valence is a value type.
Arousal is a value type.
Affect Region is a value type.
Timestamp is a value type.
Layer is a value type.

## Fact Types
Layer State has Valence.
Layer State has Arousal.
Layer State has Affect Region.

## Derivation Rules
${EXCITED_RULE}
${CALM_RULE}
${TENSE_RULE}
${DEPRESSED_RULE}
`

    const existingNouns = AFFECT_NOUNS.map(name => ({ name, id: name.toLowerCase().replace(/ /g, '-') }))
    const result = parseFORML2(affectFORML2, existingNouns)

    // Derivation rules are stored as readings with predicate ':='
    const derivationReadings = result.readings.filter(r => r.predicate === ':=')
    expect(derivationReadings.length).toBeGreaterThanOrEqual(4)

    // Each derivation reading should have ruleIR attached
    for (const dr of derivationReadings) {
      expect(dr.ruleIR).toBeDefined()
      expect(dr.ruleIR!.kind).toBe('comparison')
      expect(dr.ruleIR!.antecedents.length).toBe(2)
      expect(dr.ruleIR!.consequent.subject).toBe('Layer State')
      expect(dr.ruleIR!.consequent.object).toBe('Affect Region')
    }

    // Verify the literal values are captured
    const excitedReading = derivationReadings.find(r => r.text.includes('Excited'))
    expect(excitedReading).toBeDefined()
    expect(excitedReading!.ruleIR!.consequent.literalValue).toBe('Excited')

    const calmReading = derivationReadings.find(r => r.text.includes('Calm'))
    expect(calmReading).toBeDefined()
    expect(calmReading!.ruleIR!.consequent.literalValue).toBe('Calm')

    const tenseReading = derivationReadings.find(r => r.text.includes('Tense'))
    expect(tenseReading).toBeDefined()
    expect(tenseReading!.ruleIR!.consequent.literalValue).toBe('Tense')

    const depressedReading = derivationReadings.find(r => r.text.includes('Depressed'))
    expect(depressedReading).toBeDefined()
    expect(depressedReading!.ruleIR!.consequent.literalValue).toBe('Depressed')
  })
})
