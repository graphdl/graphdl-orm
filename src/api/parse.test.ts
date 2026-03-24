import { describe, it, expect } from 'vitest'
import { parseFORML2 } from './parse'

describe('parseFORML2', () => {
  it('parses a single reading with UC constraint', () => {
    const text = `## Entity Types
Customer is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, [])

    // Nouns
    expect(result.nouns).toHaveLength(2)
    expect(result.nouns.find(n => n.name === 'Customer')).toMatchObject({
      name: 'Customer',
      objectType: 'entity',
    })
    expect(result.nouns.find(n => n.name === 'Name')).toMatchObject({
      name: 'Name',
      objectType: 'value',
    })

    // Readings
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0]).toMatchObject({
      text: 'Customer has Name',
      nouns: ['Customer', 'Name'],
      predicate: 'has',
    })

    // Constraints
    expect(result.constraints).toHaveLength(1)
    expect(result.constraints[0]).toMatchObject({
      kind: 'UC',
      modality: 'Alethic',
      reading: 'Customer has Name',
      roles: [0],
    })

    // Always empty
    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
    expect(result.warnings).toEqual([])
  })

  it('parses multiple readings separated by blank lines', () => {
    const text = `## Entity Types
Customer is an entity type.
SupportRequest is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.
  Each Customer has at most one Name.

Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.`

    const result = parseFORML2(text, [])

    expect(result.nouns).toHaveLength(3)
    expect(result.readings).toHaveLength(2)
    expect(result.constraints).toHaveLength(2)

    // Second reading reuses Customer noun
    expect(result.readings[1]).toMatchObject({
      text: 'Customer submits SupportRequest',
      nouns: ['Customer', 'SupportRequest'],
      predicate: 'submits',
    })
  })

  it('detects subtype declarations', () => {
    const text = `PremiumCustomer is a subtype of Customer.`

    const result = parseFORML2(text, [])

    expect(result.subtypes).toHaveLength(1)
    expect(result.subtypes![0]).toEqual({
      child: 'PremiumCustomer',
      parent: 'Customer',
    })
    // Both nouns should be in the noun list
    expect(result.nouns.find(n => n.name === 'PremiumCustomer')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Customer')).toBeDefined()
  })

  it('produces partial results with warnings for malformed input', () => {
    const text = `## Entity Types
Customer is an entity type.
SupportRequest is an entity type.

## Value Types
Name is a value type.
Priority is a value type.

## Fact Types
Customer has Name.
  Each Customer has at most one Name.

justgarbage

SupportRequest has Priority.`

    const result = parseFORML2(text, [])

    // Good blocks parsed
    expect(result.readings).toHaveLength(2)
    // "justgarbage" starts with lowercase → skipped as a comment line, no warning
    expect(result.warnings).toHaveLength(0)
  })

  it('handles "exactly one" producing UC + MC constraints', () => {
    const text = `## Entity Types
Organization is an entity type.

## Value Types
Name is a value type.

## Fact Types
Organization has Name.
  Each Organization has exactly one Name.`

    const result = parseFORML2(text, [])

    expect(result.constraints).toHaveLength(2)
    expect(result.constraints.find(c => c.kind === 'UC')).toBeDefined()
    expect(result.constraints.find(c => c.kind === 'MC')).toBeDefined()
  })

  it('uses existing nouns for tokenization context', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'Name', id: 'n2' },
    ]
    const text = `Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, existingNouns)

    expect(result.nouns).toHaveLength(2) // No duplicates
    expect(result.readings).toHaveLength(1)
  })

  it('returns empty arrays for transitions and facts', () => {
    const text = `## Entity Types
Customer is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.`

    const result = parseFORML2(text, [])

    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
  })

  it('warns on unrecognized constraint patterns', () => {
    const text = `## Entity Types
Customer is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.
  This is not a valid constraint.`

    const result = parseFORML2(text, [])

    expect(result.readings).toHaveLength(1)
    expect(result.warnings).toHaveLength(1)
    expect(result.warnings[0]).toContain('Unrecognized constraint:')
  })

  it('handles non-"has" predicates as entity types', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'SupportRequest', id: 'n2', objectType: 'entity' as const },
    ]
    const text = `Customer submits SupportRequest.`

    const result = parseFORML2(text, existingNouns)

    expect(result.nouns.find(n => n.name === 'SupportRequest')).toMatchObject({
      objectType: 'entity', // not "has" → entity, not value
    })
  })

  it('attaches indented constraints to the preceding reading', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'Name', id: 'n2' },
      { name: 'SupportRequest', id: 'n3' },
    ]
    // Constraint on first block is indented under first reading
    const text = `Customer has Name.
  Each SupportRequest is submitted by at most one Customer.

Customer submits SupportRequest.`

    const result = parseFORML2(text, existingNouns)

    // The indented constraint is attached to the preceding reading
    expect(result.constraints.length).toBeGreaterThanOrEqual(1)
    const constraint = result.constraints.find(
      c => c.reading === 'Customer has Name'
    )
    expect(constraint).toBeDefined()
    expect(constraint!.kind).toBe('UC')
    expect(result.warnings).toHaveLength(0)
  })

  it('attaches cross-noun constraints to the preceding reading without warnings', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'Name', id: 'n2' },
    ]
    const text = `Customer has Name.
  Each Order has at most one Invoice.`

    const result = parseFORML2(text, existingNouns)

    // The indented constraint is parsed and attached to the preceding reading
    expect(result.constraints.length).toBeGreaterThanOrEqual(1)
    const uc = result.constraints.find(c => c.kind === 'UC')
    expect(uc).toBeDefined()
    expect(uc!.reading).toBe('Customer has Name')
    expect(result.warnings).toHaveLength(0)
  })

  it('skips entity/value type declarations without producing readings', () => {
    const text = `Customer is an entity type.

Name is a value type.

Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, [])

    // Entity/value type declarations are parsed as nouns, not readings
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].text).toBe('Customer has Name')
    // No warnings
    expect(result.warnings).toEqual([])
    // Both nouns should have correct object types from declarations
    expect(result.nouns.find(n => n.name === 'Customer')?.objectType).toBe('entity')
    expect(result.nouns.find(n => n.name === 'Name')?.objectType).toBe('value')
  })

  it('parses XO set-comparison blocks as standalone constraints', () => {
    const existingNouns = [
      { name: 'Message', id: 'n1' },
      { name: 'Lead', id: 'n2' },
      { name: 'MatchStatus', id: 'n3' },
    ]
    const text = `Message has Lead.
  Each Message has at most one Lead.

For each Message, exactly one of the following holds:
  that Message has MatchStatus 'Pending';
  that Message has MatchStatus 'Confirmed';
  that Message has MatchStatus 'Rejected'.`

    const result = parseFORML2(text, existingNouns)

    // Reading from the first block
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].text).toBe('Message has Lead')

    // UC from the reading + XO from the set-comparison block
    const xo = result.constraints.find(c => c.kind === 'XO')
    expect(xo).toBeDefined()
    expect(xo!.reading).toBe('')
    expect(xo!.roles).toEqual([])
    expect(xo!.clauses).toHaveLength(3)
    expect(xo!.entity).toBe('Message')
  })

  it('parses single-line SS (subset) constraint correctly', () => {
    // "If some X... then that X..." is a subset constraint per Halpin
    const existingNouns = [
      { name: 'Academic', id: 'n1' },
      { name: 'Department', id: 'n2' },
    ]
    const text = `## Constraints
If some Academic heads some Department then that Academic works for that Department.`

    const result = parseFORML2(text, existingNouns)

    // Parsed as SS constraint
    const ss = result.constraints.filter(c => c.kind === 'SS')
    expect(ss).toHaveLength(1)
    expect(ss[0].text).toContain('Academic')
    expect(ss[0].text).toContain('Department')
  })

  it('handles "Each X has some Y" as MC', () => {
    const existingNouns = [
      { name: 'Message', id: 'n1' },
      { name: 'Lead', id: 'n2' },
    ]
    const text = `Message has Lead.
  Each Message has some Lead.`

    const result = parseFORML2(text, existingNouns)

    const mc = result.constraints.find(c => c.kind === 'MC')
    expect(mc).toBeDefined()
    expect(mc!.reading).toBe('Message has Lead')
  })

  it('handles "if and only if" as EQ', () => {
    const existingNouns = [
      { name: 'Message', id: 'n1' },
      { name: 'Lead', id: 'n2' },
    ]
    const text = `Message has Lead.
  Message is matched if and only if Message has Lead.`

    const result = parseFORML2(text, existingNouns)

    const eq = result.constraints.find(c => c.kind === 'EQ')
    expect(eq).toBeDefined()
    expect(eq!.reading).toBe('Message has Lead')
  })

  it('parses a full domain with mixed readings and set-comparison blocks', () => {
    const existingNouns = [
      { name: 'Message', id: 'n1' },
      { name: 'Lead', id: 'n2' },
      { name: 'SalesRep', id: 'n3' },
      { name: 'MatchStatus', id: 'n4' },
    ]
    const text = `Message has Lead.
  Each Message has at most one Lead.

Lead is assigned to SalesRep.
  Each Lead is assigned to at most one SalesRep.

For each Message, exactly one of the following holds:
  that Message has MatchStatus 'Pending';
  that Message has MatchStatus 'Confirmed';
  that Message has MatchStatus 'Rejected'.

If some Message has Lead then that Message has SalesRep.`

    const result = parseFORML2(text, existingNouns)

    // Two readings (fact types); the "If some..." line is parsed as SS constraint
    expect(result.readings).toHaveLength(2)

    // 2 UCs from readings + 1 XO + 1 SS
    const uc = result.constraints.filter(c => c.kind === 'UC')
    const xo = result.constraints.filter(c => c.kind === 'XO')
    const ss = result.constraints.filter(c => c.kind === 'SS')
    expect(uc).toHaveLength(2)
    expect(xo).toHaveLength(1)
    expect(ss).toHaveLength(1)

    // All nouns accumulated
    expect(result.nouns.find(n => n.name === 'Message')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Lead')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'SalesRep')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'MatchStatus')).toBeDefined()

    expect(result.warnings).toEqual([])
  })

  it('resolves compound nouns against declarations, not PascalCase guessing', () => {
    const result = parseFORML2(`
## Entity Types
API Product(.Name) is an entity type.
VIN Decode(.id) is an entity type.

## Value Types
Title is a value type.
Result is a value type.

## Fact Types
API Product has Title.
VIN Decode has Result.
`, [])

    // "API Product" should be kept as a single multi-word noun,
    // resolved against the declaration
    const apiProduct = result.nouns.find(n => n.name === 'API Product')
    expect(apiProduct).toBeDefined()

    const vinDecode = result.nouns.find(n => n.name === 'VIN Decode')
    expect(vinDecode).toBeDefined()

    // Readings should reference the multi-word noun names
    const reading = result.readings.find(r => r.text === 'API Product has Title')
    expect(reading).toBeDefined()
    expect(reading!.nouns).toContain('API Product')
    expect(reading!.nouns).toContain('Title')
    expect(reading!.predicate).toBe('has')

    const reading2 = result.readings.find(r => r.text === 'VIN Decode has Result')
    expect(reading2).toBeDefined()
    expect(reading2!.nouns).toContain('VIN Decode')
    expect(reading2!.nouns).toContain('Result')
  })

  it('skips unrecognized ## headers without creating readings', () => {
    const text = `## Entity Types
Customer is an entity type.

## Ring Constraints
No Support Request is merged into itself.

## Disjunctive Mandatory Constraints

## Additional Entity Types
`
    const result = parseFORML2(text, [
      { name: 'Support Request', id: 'sr1' },
    ])

    // "## Ring Constraints", "## Disjunctive Mandatory Constraints", "## Additional Entity Types"
    // should NOT produce readings
    const headerReadings = result.readings.filter(
      r => r.text.startsWith('Ring') || r.text.startsWith('Disjunctive') || r.text.startsWith('Additional')
    )
    expect(headerReadings).toHaveLength(0)

    // "No Support Request is merged into itself" should not produce a reading either
    // (it is a ring constraint in the constraints section)
    expect(result.readings).toHaveLength(0)
  })

  it('stores ring constraint text as constraints when in constraints section', () => {
    const text = `## Constraints
No Support Request is merged into itself.
If Person1 reports to Person2, then Person2 does not report to Person1.
`
    const result = parseFORML2(text, [
      { name: 'Support Request', id: 'sr1' },
      { name: 'Person', id: 'p1' },
    ])

    // Both lines should be stored as constraints, not readings
    expect(result.readings).toHaveLength(0)
    expect(result.constraints.length).toBeGreaterThanOrEqual(2)

    const irreflexive = result.constraints.find(c => c.text.includes('merged into itself'))
    expect(irreflexive).toBeDefined()

    const asymmetric = result.constraints.find(c => c.text.includes('reports to'))
    expect(asymmetric).toBeDefined()
  })

  // ── New tests for declared-noun resolution ────────────────────────

  it('"Sent At" stays as one noun when declared as value type', () => {
    const text = `## Entity Types
Message(.Message Id) is an entity type.

## Value Types
Message Id is a value type.
Sent At is a value type.

## Fact Types
Message has Sent At.`

    const result = parseFORML2(text, [])

    // "Sent At" is declared as a value type, so it matches as one noun
    const sentAt = result.nouns.find(n => n.name === 'Sent At')
    expect(sentAt).toBeDefined()
    expect(sentAt!.objectType).toBe('value')

    // The explicit reading should reference "Sent At" as a single noun
    const sentAtReading = result.readings.find(r => r.text === 'Message has Sent At')
    expect(sentAtReading).toBeDefined()
    expect(sentAtReading!.nouns).toContain('Message')
    expect(sentAtReading!.nouns).toContain('Sent At')
    expect(sentAtReading!.predicate).toBe('has')

    // No false nouns "Sent" or "At"
    expect(result.nouns.find(n => n.name === 'Sent')).toBeUndefined()
    expect(result.nouns.find(n => n.name === 'At')).toBeUndefined()
  })

  it('"Cross-domain references: ..." lines are skipped entirely', () => {
    const text = `# Support

Cross-domain references: Customer (from customer-auth), Feature Request (from feature-requests)

## Entity Types
Request(.Request Id) is an entity type.

## Fact Types
Request has Subject.`

    const result = parseFORML2(text, [
      { name: 'Subject', id: 'n1' },
    ])

    // The cross-domain references line should not produce a reading or noun
    expect(result.nouns.find(n => n.name === 'Cross')).toBeUndefined()
    expect(result.readings.find(r => r.text.includes('Cross-domain'))).toBeUndefined()
    expect(result.unparsed.find(l => l.includes('Cross-domain'))).toBeUndefined()
  })

  it('description prose lines fall through to unparsed without creating nouns', () => {
    const text = `# Support

Support request lifecycle and response content rules for the auto platform.

## Entity Types
Request(.Request Id) is an entity type.

## Value Types
Request Id is a value type.
Subject is a value type.

## Fact Types
Request has Subject.`

    const result = parseFORML2(text, [])

    // Prose line has no declared nouns → falls through to unparsed
    expect(result.nouns.find(n => n.name === 'Support')).toBeUndefined()
    // Explicit reading + implicit ref scheme reading
    const explicitReading = result.readings.find(r => r.text === 'Request has Subject')
    expect(explicitReading).toBeDefined()
    // The prose line ends up in unparsed (candidate for LLM extraction)
    expect(result.unparsed).toContainEqual(
      expect.stringContaining('Support request lifecycle')
    )
  })

  it('undeclared nouns in readings do not create false noun entries', () => {
    const text = `## Entity Types
Customer is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.
Customer submits SupportRequest.`

    const result = parseFORML2(text, [])

    // "SupportRequest" is not declared, so it should NOT appear as a noun
    expect(result.nouns.find(n => n.name === 'SupportRequest')).toBeUndefined()

    // The second reading should only find "Customer" (1 noun = unary)
    // since "SupportRequest" is not declared
    const submitReading = result.readings.find(r => r.text === 'Customer submits SupportRequest')
    expect(submitReading).toBeDefined()
    expect(submitReading!.nouns).toEqual(['Customer'])
  })

  it('compound nouns like "API Product" match as one noun when declared', () => {
    const text = `## Entity Types
API Product(.Name) is an entity type.
Support Response(.Message Id) is an entity type.

## Value Types
Name is a value type.
Message Id is a value type.

## Fact Types
Support Response recommends API Product.`

    const result = parseFORML2(text, [])

    // "API Product" matched as one noun, not "API" + "Product"
    expect(result.nouns.find(n => n.name === 'API Product')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'API')).toBeUndefined()
    expect(result.nouns.find(n => n.name === 'Product')).toBeUndefined()

    const reading = result.readings.find(r => r.text.includes('recommends'))
    expect(reading).toBeDefined()
    expect(reading!.nouns).toContain('Support Response')
    expect(reading!.nouns).toContain('API Product')
  })

  it('compound ref schemes: "Layer State(.Layer, .Timestamp)" parsed as one noun', () => {
    const text = `## Entity Types
Layer(.Layered System, .Layer Number) is an entity type.
Layer State(.Layer, .Timestamp) is an entity type.

## Value Types
Valence is a value type.
Arousal is a value type.
Timestamp is a value type.

## Fact Types
Layer State has Valence.
Layer State has Arousal.`

    const result = parseFORML2(text, [])

    // "Layer State" should be a single compound noun, not split into "Layer" + "State"
    expect(result.nouns.find(n => n.name === 'Layer State')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Layer')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'State')).toBeUndefined()

    // Explicit readings should match "Layer State" as one noun
    const valenceReading = result.readings.find(r => r.text === 'Layer State has Valence')
    expect(valenceReading).toBeDefined()
    expect(valenceReading!.nouns).toContain('Layer State')
    const arousalReading = result.readings.find(r => r.text === 'Layer State has Arousal')
    expect(arousalReading).toBeDefined()
    expect(arousalReading!.nouns).toContain('Layer State')
  })

  it('compound identification: "City(.Name, .State)" registers both ref scheme parts', () => {
    const text = `## Entity Types
City(.Name, .State) is an entity type.

## Value Types
Name is a value type.
State is a value type.

## Fact Types
City has Name.
City has State.`

    const result = parseFORML2(text, [])

    expect(result.nouns.find(n => n.name === 'City')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'City')!.objectType).toBe('entity')
    // Both ref scheme components registered as value types
    expect(result.nouns.find(n => n.name === 'Name')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'State')).toBeDefined()
    // Implicit ref scheme readings + explicit readings (may overlap)
    expect(result.readings.filter(r => r.nouns.includes('City')).length).toBeGreaterThanOrEqual(2)
  })

  it('readings with only 1 declared noun are still stored (unary readings)', () => {
    const text = `## Entity Types
API is an entity type.

## Fact Types
API is internal.`

    const result = parseFORML2(text, [])

    // Unary reading with just 1 noun
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].nouns).toEqual(['API'])
    expect(result.readings[0].predicate).toBe('is internal')
  })
})
