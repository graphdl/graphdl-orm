/**
 * Unified seed handler: accepts plain natural-language text + domainId,
 * parses via the unified parser (parseText), and creates nouns, graph schemas,
 * readings, and constraints via the Payload local API.
 *
 * This is the bridge between the FORML2 parser and the Payload database.
 * Task 6 will wire this into the seed endpoint.
 */

import type { Payload } from 'payload'
import { parseText } from '../parse'

const SEED_CONSTRAINT_KEYWORDS = new Set(['Each', 'It', 'That', 'The'])

export interface SeedOptions {
  text: string
  domainId: string
  tenant?: string
}

export interface SeedResult {
  nounsCreated: number
  readingsCreated: number
  constraintsCreated: number
  errors: string[]
}


/** Best-effort English pluralization using regular rules only.
 *  Irregular plurals (person→people, species→species) are handled by the LLM
 *  layer which writes the plural form directly in the readings. */
function pluralize(word: string): string {
  const lower = word.toLowerCase()
  if (/(?:s|x|z|ch|sh)$/i.test(lower)) return lower + 'es'
  if (/[^aeiou]y$/i.test(lower)) return lower.replace(/y$/i, 'ies')
  return lower + 's'
}

/** Produce kebab-case plural slug: SupportRequest → support-requests */
function pluralSlug(name: string): string {
  const kebab = name.replace(/([A-Z])/g, '-$1').toLowerCase().replace(/^-/, '')
  const lastSegment = kebab.split('-').pop() || kebab
  const pluralLast = pluralize(lastSegment.charAt(0).toUpperCase() + lastSegment.slice(1))
  return kebab.replace(new RegExp(lastSegment + '$'), pluralLast)
}

async function ensureNoun(payload: Payload, name: string, domainId: string, objectType: 'entity' | 'value' = 'entity'): Promise<any> {
  const existing = await payload.find({
    collection: 'nouns',
    where: { name: { equals: name }, domain: { equals: domainId } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({
    collection: 'nouns',
    data: {
      name,
      objectType,
      plural: pluralSlug(name),
      domain: domainId,
    },
  })
}

export async function seedReadingsFromText(
  payload: Payload,
  options: SeedOptions,
): Promise<SeedResult> {
  const { text, domainId } = options
  const result: SeedResult = { nounsCreated: 0, readingsCreated: 0, constraintsCreated: 0, errors: [] }

  // Get existing nouns for this domain
  const existingNouns = await payload.find({
    collection: 'nouns',
    where: { domain: { equals: domainId } },
    pagination: false,
  })
  const knownNounNames = existingNouns.docs.map((n: any) => n.name as string).filter(Boolean)

  // Analyze multiplicities to determine entity vs value object types
  // In "A has B | *:1", B is a value (many A's share the same B).
  // A noun is a value if it ONLY appears as the object of *:1 facts.
  const subjectOf = new Map<string, Set<string>>() // noun → set of multiplicities where it's the subject
  const objectOf = new Map<string, Set<string>>()  // noun → set of multiplicities where it's the object
  const lines = text.split('\n').map(l => l.trim()).filter(l => l && !l.startsWith('#'))
  for (const line of lines) {
    const [readingPart, multPart] = line.split('|').map(s => s.trim())
    const mult = multPart || '*:1'
    // Extract capitalized words as potential nouns (same heuristic as parser)
    const caps = readingPart.match(/\b([A-Z][a-zA-Z]+)\b/g) || []
    const lineNouns = caps.filter(w => !SEED_CONSTRAINT_KEYWORDS.has(w))
    if (lineNouns.length >= 2) {
      const subject = lineNouns[0]
      const object = lineNouns[lineNouns.length - 1]
      if (!subjectOf.has(subject)) subjectOf.set(subject, new Set())
      subjectOf.get(subject)!.add(mult)
      if (!objectOf.has(object)) objectOf.set(object, new Set())
      objectOf.get(object)!.add(mult)
    }
  }

  // A noun is a value type if:
  // - It appears as the object of *:1 facts
  // - It never appears as the subject of any fact
  const valueNouns = new Set<string>()
  for (const [noun, mults] of objectOf) {
    const onlyStarOne = [...mults].every(m => m === '*:1')
    if (onlyStarOne && !subjectOf.has(noun)) {
      valueNouns.add(noun)
    }
  }

  // Pass 1: discover all noun candidates from the text
  const firstPass = parseText(text, knownNounNames)

  // Create any new noun candidates detected by the parser
  for (const nounName of firstPass.newNounCandidates) {
    try {
      const objectType = valueNouns.has(nounName) ? 'value' : 'entity'
      await ensureNoun(payload, nounName, domainId, objectType)
      result.nounsCreated++
    } catch (err: any) {
      result.errors.push(`Failed to create noun "${nounName}": ${err.message}`)
    }
  }

  // Refresh noun map after creating new ones (domain-scoped)
  const allNouns = await payload.find({
    collection: 'nouns',
    where: { domain: { equals: domainId } },
    pagination: false,
  })
  const nounMap = new Map(allNouns.docs.map((n: any) => [n.name, n]))
  const allNounNames = [...nounMap.keys()]

  // Pass 2: re-parse with all nouns now known so each reading finds its nouns
  const parsed = parseText(text, allNounNames)

  for (const reading of parsed.readings) {
    try {
      // Skip transitions and instance facts — handled separately
      if (reading.isTransition || reading.isInstanceFact) continue

      if (reading.isSubtype) {
        // Set superType relationship on the child noun
        const subNoun = nounMap.get(reading.nouns[0])
        const superNoun = nounMap.get(reading.nouns[1])
        if (subNoun && superNoun) {
          await payload.update({
            collection: 'nouns',
            id: subNoun.id,
            data: { superType: superNoun.id },
          })
        }
        continue
      }

      // Ensure all nouns referenced in this reading exist
      for (const nounName of reading.nouns) {
        if (!nounMap.has(nounName)) {
          const objectType = valueNouns.has(nounName) ? 'value' as const : 'entity' as const
          const noun = await ensureNoun(payload, nounName, domainId, objectType)
          nounMap.set(nounName, noun)
          result.nounsCreated++
        }
      }

      // Build the reading text from the parsed nouns and predicate
      const readingText = reading.nouns.length >= 2
        ? `${reading.nouns[0]} ${reading.predicate} ${reading.nouns.slice(1).join(' ')}`.replace(/\s+/g, ' ').trim()
        : reading.nouns[0]

      if (!readingText) {
        result.errors.push(`Empty reading text from line — skipping`)
        continue
      }

      // Build a PascalCase schema name from the reading nouns
      const schemaName = reading.nouns.join('')

      // Create the graph schema
      const graphSchema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: schemaName,
          title: schemaName,
          domain: domainId,
        },
      })

      // Create reading -- the afterChange hook auto-creates Roles by tokenizing
      // the reading text against known nouns
      await payload.create({
        collection: 'readings',
        data: {
          text: readingText,
          graphSchema: graphSchema.id,
          domain: domainId,
        },
      } as any)
      result.readingsCreated++

      // Apply constraints from the parser to the roles created by the hook
      if (reading.constraints.length > 0) {
        const roles = await payload.find({
          collection: 'roles',
          where: { graphSchema: { equals: graphSchema.id } },
          sort: 'createdAt',
        })

        for (const constraint of reading.constraints) {
          try {
            const c = await payload.create({
              collection: 'constraints',
              data: { kind: constraint.kind, modality: constraint.modality, ...(domainId ? { domain: domainId } : {}) } as any,
            })

            const roleIds = constraint.roles
              .map((idx) => roles.docs[idx]?.id)
              .filter(Boolean)

            if (roleIds.length) {
              await payload.create({
                collection: 'constraint-spans',
                data: { roles: roleIds, constraint: c.id, ...(domainId ? { domain: domainId } : {}) },
              } as any)
              result.constraintsCreated++
            }
          } catch (err: any) {
            result.errors.push(`Failed to create constraint for "${readingText}": ${err.message}`)
          }
        }
      }
    } catch (err: any) {
      result.errors.push(`Failed to seed "${reading.nouns.join(' ')}": ${err.message}`)
    }
  }

  return result
}
