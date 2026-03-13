import type { Generator, NounRenderer, FactTypeRenderer } from '../model/renderer'
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, RoleDef } from '../model/types'

export interface MdxuiOutput {
  files: Record<string, string>
}

// Each walker visit produces a MdxuiPart that the combiner merges
type MdxuiPart =
  | { type: 'entity'; slug: string; name: string; fields: string[]; navLinks: string[]; constraints: string[] }
  | { type: 'skip' }

export const mdxuiGenerator: Generator<MdxuiPart, MdxuiOutput> = {
  noun: {
    entity(noun: NounDef, factTypes: FactTypeDef[], constraints: ConstraintDef[]): MdxuiPart {
      if (!noun.permissions?.length) return { type: 'skip' }

      const fields: string[] = []
      const navLinks: string[] = []

      for (const ft of factTypes) {
        if (ft.arity === 1) {
          fields.push(`- **${ft.reading}**: Checkbox\n`)
          continue
        }
        if (ft.arity !== 2) continue
        const objectNoun = ft.roles[1]?.nounDef
        if (objectNoun?.objectType === 'value') {
          fields.push(renderField(objectNoun, ft))
        } else if (objectNoun?.objectType === 'entity') {
          const slug = toSlug(objectNoun)
          navLinks.push(`  <Card title="${objectNoun.name}" href="/pages/${slug}" />\n`)
        }
      }

      const constraintLines = constraints.map(c =>
        `<Callout type="info" title="${c.kind} Constraint">\n${c.text}\n</Callout>\n`)

      const slug = toSlug(noun)
      return { type: 'entity', slug, name: noun.name, fields, navLinks, constraints: constraintLines }
    },
    value(_noun: NounDef): MdxuiPart {
      return { type: 'skip' }
    },
  },
  combine(parts: MdxuiPart[]): MdxuiOutput {
    const files: Record<string, string> = {}
    const entities = parts.filter((p): p is Extract<MdxuiPart, { type: 'entity' }> => p.type === 'entity')

    for (const e of entities) {
      const lines: string[] = []
      lines.push(`import { Callout, Card, Cards } from 'mdxui/components'`)
      lines.push('')
      lines.push(`# ${e.name}`)
      lines.push('')
      if (e.fields.length) { lines.push('## Fields', '', ...e.fields) }
      if (e.navLinks.length) { lines.push('## Related', '', '<Cards>', ...e.navLinks, '</Cards>', '') }
      if (e.constraints.length) { lines.push('## Constraints', '', ...e.constraints) }
      files[`pages/${e.slug}.mdx`] = lines.join('\n')
    }

    // Index page
    const indexLines = [
      `import { Card, Cards } from 'mdxui/components'`,
      '', '# Domain Entities', '', '<Cards>',
      ...entities.map(e => `  <Card title="${e.name}" href="/pages/${e.slug}" />`),
      '</Cards>',
    ]
    files['pages/index.mdx'] = indexLines.join('\n')

    return { files }
  },
}

// Convenience wrapper for direct invocation
export async function generateMdxui(model: { render: (gen: Generator<MdxuiPart, MdxuiOutput>) => Promise<MdxuiOutput> }): Promise<MdxuiOutput> {
  return model.render(mdxuiGenerator)
}

// -- Helper functions used by the walker --

function toSlug(noun: NounDef): string {
  return (noun.plural || noun.name.replace(/([a-z])([A-Z])/g, '$1-$2').toLowerCase() + 's')
    .toLowerCase().replace(/\s+/g, '-')
}

function renderField(noun: NounDef, ft: FactTypeDef): string {
  const label = noun.name
  if (noun.enumValues?.length) return `- **${label}**: Select (${noun.enumValues.join(', ')})\n`
  if (noun.valueType === 'boolean') return `- **${label}**: Checkbox\n`
  if (noun.format === 'date') return `- **${label}**: DatePicker\n`
  if (noun.format === 'email') return `- **${label}**: TextBox (email)\n`
  if (noun.valueType === 'number') return `- **${label}**: TextBox (number)\n`
  return `- **${label}**: TextBox\n`
}
