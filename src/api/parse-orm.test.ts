import { describe, it, expect } from 'vitest'
import { parseOrmXml } from './parse-orm'
import { readFileSync } from 'fs'

describe('parseOrmXml', () => {
  it('parses the GraphDL.orm model', () => {
    const xml = readFileSync('C:/Users/lippe/Repos/payload-experiments/samuel/GraphDL/GraphDL.orm', 'utf-8')
    const result = parseOrmXml(xml)

    // Should have entity types
    const entityNames = result.nouns.filter((n: any) => n.objectType === 'entity').map((n: any) => n.name)
    expect(entityNames).toContain('Noun')
    expect(entityNames).toContain('Reading')
    expect(entityNames).toContain('Role')
    expect(entityNames).toContain('Graph Schema')
    expect(entityNames).toContain('Constraint')
    expect(entityNames).toContain('State Machine Definition')
    expect(entityNames).toContain('Verb')

    // Should have value types
    const valueNames = result.nouns.filter((n: any) => n.objectType === 'value').map((n: any) => n.name)
    expect(valueNames).toContain('Name')
    expect(valueNames).toContain('Text')
    expect(valueNames).toContain('Timestamp')

    // Should have subtypes
    expect(result.subtypes.length).toBeGreaterThan(0)
    expect(result.subtypes).toContainEqual({ child: 'Graph Schema', parent: 'Noun' })
    expect(result.subtypes).toContainEqual({ child: 'Status', parent: 'Noun' })
    expect(result.subtypes).toContainEqual({ child: 'Graph', parent: 'Resource' })

    // Should have readings
    expect(result.readings.length).toBeGreaterThan(20)
    const readingTexts = result.readings.map((r: any) => r.text)
    expect(readingTexts).toContain('Noun plays Role.')
    expect(readingTexts).toContain('Graph Schema has Reading.')
    expect(readingTexts).toContain('Role is used in Reading.')

    // Should have constraints
    expect(result.constraints.length).toBeGreaterThan(10)
    const ucConstraints = result.constraints.filter((c: any) => c.kind === 'UC')
    const mcConstraints = result.constraints.filter((c: any) => c.kind === 'MC')
    expect(ucConstraints.length).toBeGreaterThan(5)
    expect(mcConstraints.length).toBeGreaterThan(5)

    // No errors
    expect(result.warnings).toHaveLength(0)

    console.log(`Parsed: ${entityNames.length} entities, ${valueNames.length} values, ${result.subtypes.length} subtypes, ${result.readings.length} readings, ${result.constraints.length} constraints`)
  })
})
