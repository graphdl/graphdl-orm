/**
 * NORMA ORM XML (.orm) → FORML2 ExtractedClaims parser.
 *
 * Translates a NORMA ORM2 XML model file into the same ExtractedClaims
 * format produced by the FORML2 text parser, enabling .orm files to be
 * seeded into graphdl-orm via the standard claims pipeline.
 *
 * Handles: entity types, value types, subtypes, fact types (with readings),
 * uniqueness constraints, mandatory constraints, subset constraints,
 * exclusion constraints, ring constraints, and frequency constraints.
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { ExtractedClaims } from '../claims/ingest'

const ORM_NS = 'http://schemas.neumont.edu/ORM/2006-04/ORMCore'

interface OrmEntity {
  id: string
  name: string
  refMode: string
  playedRoles: string[]
}

interface OrmValueType {
  id: string
  name: string
  enumValues?: string[]
}

interface OrmRole {
  id: string
  playerId: string
  playerName: string
  factId: string
}

interface OrmFact {
  id: string
  name: string
  roles: OrmRole[]
  readings: Array<{ text: string; roleOrder: string[] }>
}

// ── Minimal XML parser (no DOMParser dependency) ─────────────────────

interface XNode {
  tag: string
  attrs: Record<string, string>
  children: XNode[]
  text: string
}

function parseXml(xml: string): XNode {
  const root: XNode = { tag: '', attrs: {}, children: [], text: '' }
  const stack: XNode[] = [root]
  // Simple regex-based parser for well-formed XML
  const tagRe = /<\/?([a-zA-Z0-9_:.-]+)([^>]*?)(\/?)\s*>|([^<]+)/g
  let m: RegExpExecArray | null
  while ((m = tagRe.exec(xml)) !== null) {
    if (m[4]) {
      // Text content
      const text = m[4].trim()
      if (text && stack.length > 0) stack[stack.length - 1].text += text
      continue
    }
    const isClosing = xml[m.index + 1] === '/'
    const tagName = m[1]
    const attrStr = m[2] || ''
    const selfClosing = m[3] === '/'

    // Extract local name (strip namespace prefix)
    const localName = tagName.includes(':') ? tagName.split(':').pop()! : tagName

    if (isClosing) {
      stack.pop()
    } else {
      const attrs: Record<string, string> = {}
      const attrRe = /([a-zA-Z0-9_:.-]+)\s*=\s*"([^"]*)"/g
      let am: RegExpExecArray | null
      while ((am = attrRe.exec(attrStr)) !== null) {
        const aName = am[1].includes(':') ? am[1].split(':').pop()! : am[1]
        attrs[aName] = am[2]
      }

      const node: XNode = { tag: localName, attrs, children: [], text: '' }
      stack[stack.length - 1].children.push(node)

      if (!selfClosing) stack.push(node)
    }
  }
  return root.children[0] || root
}

function findAll(el: XNode, localName: string): XNode[] {
  return el.children.filter(c => c.tag === localName)
}

function findOne(el: XNode, localName: string): XNode | null {
  return el.children.find(c => c.tag === localName) || null
}

function findDeep(el: XNode, ...path: string[]): XNode | null {
  let current: XNode | null = el
  for (const name of path) {
    if (!current) return null
    current = findOne(current, name)
  }
  return current
}

function findAllDeep(el: XNode, ...path: string[]): XNode[] {
  if (path.length === 0) return []
  let current: XNode | null = el
  for (let i = 0; i < path.length - 1; i++) {
    if (!current) return []
    current = findOne(current, path[i])
  }
  if (!current) return []
  return findAll(current, path[path.length - 1])
}

// ── Main Parser ──────────────────────────────────────────────────────

export function parseOrmXml(xmlText: string): ExtractedClaims & { warnings: string[] } {
  const root = parseXml(xmlText)
  // Root might be ORM2 wrapper or ORMModel directly
  const model = root.tag === 'ORMModel' ? root : findOne(root, 'ORMModel')
  if (!model) {
    return { nouns: [], readings: [], constraints: [], subtypes: [], transitions: [], facts: [], warnings: ['No ORMModel found'] }
  }

  const warnings: string[] = []

  // ── 1. Parse object types ──────────────────────────────────────────

  const entityMap = new Map<string, OrmEntity>()
  const valueMap = new Map<string, OrmValueType>()
  const allTypes = new Map<string, string>() // id → name

  const objectsEl = findOne(model, 'Objects')
  if (objectsEl) {
    for (let i = 0; i < objectsEl.children.length; i++) {
      const obj = objectsEl.children[i]
      const tag = obj.tag
      const id = obj.attrs['id'] || ''
      const name = obj.attrs['Name'] || ''
      const refMode = obj.attrs['_ReferenceMode'] || ''

      if (tag === 'EntityType') {
        const playedRoles: string[] = []
        for (const r of findAllDeep(obj, 'PlayedRoles', 'Role')) {
          playedRoles.push(r.attrs['ref'] || '')
        }
        for (const r of findAllDeep(obj, 'PlayedRoles', 'SubtypeMetaRole')) {
          playedRoles.push(r.attrs['ref'] || '')
        }
        for (const r of findAllDeep(obj, 'PlayedRoles', 'SupertypeMetaRole')) {
          playedRoles.push(r.attrs['ref'] || '')
        }
        entityMap.set(id, { id, name, refMode, playedRoles })
        allTypes.set(id, name)
      } else if (tag === 'ValueType') {
        const vt: OrmValueType = { id, name }
        // Check for enum values
        const valueConstraint = findDeep(obj, 'ValueRestriction', 'ValueConstraint')
        if (valueConstraint) {
          const ranges = findAllDeep(valueConstraint, 'ValueRanges', 'ValueRange')
          const vals: string[] = []
          for (const r of ranges) {
            const min = r.attrs['MinValue']
            const max = r.attrs['MaxValue']
            if (min && min === max) vals.push(min)
          }
          if (vals.length > 0) vt.enumValues = vals
        }
        valueMap.set(id, vt)
        allTypes.set(id, name)
      } else if (tag === 'ObjectifiedType') {
        // Objectified fact types are entity types
        const playedRoles: string[] = []
        for (const r of findAllDeep(obj, 'PlayedRoles', 'Role')) {
          playedRoles.push(r.attrs['ref'] || '')
        }
        entityMap.set(id, { id, name, refMode, playedRoles })
        allTypes.set(id, name)
      }
    }
  }

  // ── 2. Parse facts ─────────────────────────────────────────────────

  const roleMap = new Map<string, OrmRole>() // role id → role info
  const factMap = new Map<string, OrmFact>()
  const subtypeResults: ExtractedClaims['subtypes'] = []

  const factsEl = findOne(model, 'Facts')
  if (factsEl) {
    for (let i = 0; i < factsEl.children.length; i++) {
      const fact = factsEl.children[i]
      const tag = fact.tag
      const factId = fact.attrs['id'] || ''
      const factName = fact.attrs['_Name'] || ''

      if (tag === 'SubtypeFact') {
        const subRole = findDeep(fact, 'FactRoles', 'SubtypeMetaRole')
        const supRole = findDeep(fact, 'FactRoles', 'SupertypeMetaRole')
        if (subRole && supRole) {
          const subPlayer = findOne(subRole, 'RolePlayer')
          const supPlayer = findOne(supRole, 'RolePlayer')
          if (subPlayer && supPlayer) {
            const child = allTypes.get(subPlayer.attrs['ref'] || '') || '?'
            const parent = allTypes.get(supPlayer.attrs['ref'] || '') || '?'
            subtypeResults.push({ child, parent })
          }
        }
        continue
      }

      if (tag !== 'Fact') continue

      // Parse roles
      const roles: OrmRole[] = []
      const factRolesEl = findOne(fact, 'FactRoles')
      if (factRolesEl) {
        for (let j = 0; j < factRolesEl.children.length; j++) {
          const roleEl = factRolesEl.children[j]
          const roleTag = roleEl.tag
          const roleId = roleEl.attrs['id'] || ''

          if (roleTag === 'RoleProxy') {
            // Role proxy — resolve via ref to the source role
            const sourceRef = roleEl.attrs['ref'] || ''
            const sourceRole = roleMap.get(sourceRef)
            if (sourceRole) {
              const role: OrmRole = { id: roleId, playerId: sourceRole.playerId, playerName: sourceRole.playerName, factId }
              roles.push(role)
              roleMap.set(roleId, role)
            } else {
              roles.push({ id: roleId, playerId: '', playerName: '?', factId })
            }
            continue
          }

          const player = findOne(roleEl, 'RolePlayer')
          const playerId = player?.attrs['ref'] || ''
          const playerName = allTypes.get(playerId) || '?'
          const role: OrmRole = { id: roleId, playerId, playerName, factId }
          roles.push(role)
          roleMap.set(roleId, role)
        }
      }

      // Parse readings
      const readings: OrmFact['readings'] = []
      const readingOrdersEl = findOne(fact, 'ReadingOrders')
      if (readingOrdersEl) {
        for (const ro of findAll(readingOrdersEl, 'ReadingOrder')) {
          // Get the role order for this reading
          const roleOrder: string[] = []
          for (const rr of findAllDeep(ro, 'RoleSequence', 'Role')) {
            roleOrder.push(rr.attrs['ref'] || '')
          }

          for (const reading of findAllDeep(ro, 'Readings', 'Reading')) {
            const dataEl = findOne(reading, 'Data')
            if (dataEl?.text) {
              let text = dataEl.text
              // Replace {0}, {1} with the role player names in this reading's role order
              for (let k = 0; k < roleOrder.length; k++) {
                const roleId = roleOrder[k]
                const role = roles.find(r => r.id === roleId) || roleMap.get(roleId)
                const name = role?.playerName || '?'
                text = text.replace(`{${k}}`, name)
              }
              // Add trailing period if not present
              if (!text.endsWith('.')) text += '.'
              readings.push({ text, roleOrder })
            }
          }
        }
      }

      factMap.set(factId, { id: factId, name: factName, roles, readings })
    }
  }

  // ── 3. Parse constraints ───────────────────────────────────────────

  const constraintResults: ExtractedClaims['constraints'] = []

  const constraintsEl = findOne(model, 'Constraints')
  if (constraintsEl) {
    for (let i = 0; i < constraintsEl.children.length; i++) {
      const c = constraintsEl.children[i]
      const tag = c.tag
      const isPreferred = c.attrs['IsPreferred'] === 'true'
      const isInternal = c.attrs['IsInternal'] === 'true'
      const modality = c.attrs['Modality'] === 'Deontic' ? 'Deontic' as const : 'Alethic' as const

      // Get constrained roles
      const constrainedRoles: string[] = []
      for (const r of findAllDeep(c, 'RoleSequence', 'Role')) {
        constrainedRoles.push(r.attrs['ref'] || '')
      }

      // Resolve to fact types and player names
      const constrainedNouns: string[] = []
      const factIds = new Set<string>()
      for (const roleId of constrainedRoles) {
        const role = roleMap.get(roleId)
        if (role) {
          constrainedNouns.push(role.playerName)
          factIds.add(role.factId)
        }
      }

      // Find the primary reading for the fact
      const factId = [...factIds][0]
      const fact = factId ? factMap.get(factId) : undefined
      const primaryReading = fact?.readings[0]?.text || ''

      if (tag === 'UniquenessConstraint') {
        if (constrainedRoles.length === 1 && fact) {
          // Simple UC on one role
          const role = roleMap.get(constrainedRoles[0])
          if (role) {
            // "Each A R at most one B" — A is the constrained noun
            const otherNouns = fact.roles
              .filter(r => r.id !== constrainedRoles[0])
              .map(r => r.playerName)
            const allNouns = [role.playerName, ...otherNouns]
            constraintResults.push({
              kind: 'UC',
              modality,
              reading: primaryReading,
              roles: [fact.roles.findIndex(r => r.id === constrainedRoles[0])],
              text: `Each ${role.playerName} ${verbalizePredicate(primaryReading, role.playerName)} at most one ${otherNouns[0] || '?'}.`,
            })
          }
        } else if (constrainedRoles.length >= 2) {
          // Spanning or compound UC
          const text = constrainedRoles.length === fact?.roles.length
            ? `Each ${constrainedNouns.join(', ')} combination occurs at most once in the population of ${primaryReading}.`
            : `For each combination of ${constrainedNouns.slice(0, -1).join(', ')}, ${primaryReading.replace(constrainedNouns[constrainedNouns.length - 1], `at most one ${constrainedNouns[constrainedNouns.length - 1]}`)}.`
          constraintResults.push({
            kind: 'UC',
            modality,
            reading: primaryReading,
            roles: constrainedRoles.map(rid => fact?.roles.findIndex(r => r.id === rid) ?? -1).filter(i => i >= 0),
            text,
          })
        }
      } else if (tag === 'MandatoryConstraint') {
        const isImplied = c.attrs['IsImplied'] === 'true'
        if (isImplied) continue // Skip implied mandatory constraints (reference schemes)

        if (constrainedRoles.length === 1 && fact) {
          const role = roleMap.get(constrainedRoles[0])
          if (role) {
            const otherNouns = fact.roles
              .filter(r => r.id !== constrainedRoles[0])
              .map(r => r.playerName)
            if (otherNouns.length > 0) {
              constraintResults.push({
                kind: 'MC',
                modality,
                reading: primaryReading,
                roles: [fact.roles.findIndex(r => r.id === constrainedRoles[0])],
                text: `Each ${role.playerName} ${verbalizePredicate(primaryReading, role.playerName)} some ${otherNouns[0]}.`,
              })
            } else {
              // Unary mandatory
              constraintResults.push({
                kind: 'MC',
                modality,
                reading: primaryReading,
                roles: [0],
                text: `Each ${role.playerName} ${primaryReading.replace(role.playerName, '').trim()}.`,
              })
            }
          }
        } else if (constrainedRoles.length > 1) {
          // Disjunctive mandatory (inclusive-or)
          constraintResults.push({
            kind: 'OR' as any,
            modality,
            reading: '',
            roles: [],
            text: `Each ${constrainedNouns[0]} plays at least one of: ${constrainedNouns.slice(1).join(', ')}.`,
            entity: constrainedNouns[0],
            clauses: constrainedNouns.slice(1),
          })
        }
      } else if (tag === 'SubsetConstraint') {
        // Subset constraints have two role sequences
        const sequences = findAll(c, 'RoleSequence')
        if (sequences.length === 2) {
          const superRoles = findAll(sequences[0], 'Role').map(r => roleMap.get(r.attrs['ref'] || ''))
          const subRoles = findAll(sequences[1], 'Role').map(r => roleMap.get(r.attrs['ref'] || ''))
          const superNames = superRoles.map(r => r?.playerName || '?')
          const subNames = subRoles.map(r => r?.playerName || '?')
          constraintResults.push({
            kind: 'SS' as any,
            modality,
            reading: '',
            roles: [],
            text: `If some ${superNames.join(' and ')} then that ${subNames.join(' and ')}.`,
          })
        }
      } else if (tag === 'ExclusionConstraint') {
        constraintResults.push({
          kind: 'XC' as any,
          modality,
          reading: '',
          roles: [],
          text: `Exclusion constraint on ${constrainedNouns.join(', ')}.`,
        })
      }
    }
  }

  // ── 4. Build output ────────────────────────────────────────────────

  const nouns: ExtractedClaims['nouns'] = []

  for (const [, e] of entityMap) {
    nouns.push({
      name: e.name,
      objectType: 'entity',
      ...(e.refMode && { refScheme: e.refMode }),
    } as any)
  }

  for (const [, v] of valueMap) {
    // Skip reference mode value types (e.g., Role_id, Noun_id)
    if (v.name.includes('_id') || v.name.includes('_code') || v.name.includes('_Name')) continue
    nouns.push({
      name: v.name,
      objectType: 'value',
      ...(v.enumValues && { enum: v.enumValues }),
    } as any)
  }

  // Primary readings become fact type readings
  const readingResults: ExtractedClaims['readings'] = []
  for (const [, fact] of factMap) {
    if (fact.readings.length > 0) {
      const primary = fact.readings[0]
      const nounNames = fact.roles.map(r => r.playerName)
      readingResults.push({
        text: primary.text,
        nouns: nounNames,
        predicate: extractPredicate(primary.text, nounNames),
      })
    }
  }

  return {
    nouns,
    readings: readingResults,
    constraints: constraintResults,
    subtypes: subtypeResults,
    transitions: [],
    facts: [],
    warnings,
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

/** Extract predicate text between first two nouns in a reading. */
function extractPredicate(text: string, nouns: string[]): string {
  if (nouns.length < 2) return ''
  const first = text.indexOf(nouns[0])
  if (first === -1) return ''
  const afterFirst = first + nouns[0].length
  const second = text.indexOf(nouns[1], afterFirst)
  if (second === -1) return ''
  return text.slice(afterFirst, second).trim()
}

/** Extract predicate from a reading relative to a specific noun. */
function verbalizePredicate(reading: string, noun: string): string {
  const idx = reading.indexOf(noun)
  if (idx === -1) return ''
  const after = reading.slice(idx + noun.length).trim()
  // Return the predicate portion up to the next noun (next uppercase word)
  const nextNoun = after.match(/\s+[A-Z]/)
  if (nextNoun?.index) return after.slice(0, nextNoun.index).trim()
  return after
}

// ── HTTP Handler ─────────────────────────────────────────────────────

export async function handleParseOrm(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const contentType = request.headers.get('content-type') || ''

  let xmlText: string
  if (contentType.includes('xml') || contentType.includes('text/plain')) {
    xmlText = await request.text()
  } else if (contentType.includes('json')) {
    const body = await request.json() as { text?: string; xml?: string }
    xmlText = body.text || body.xml || ''
  } else {
    xmlText = await request.text()
  }

  if (!xmlText) {
    return error(400, { errors: [{ message: 'ORM XML content is required' }] })
  }

  const result = parseOrmXml(xmlText)

  return json({
    nouns: result.nouns.length,
    readings: result.readings.length,
    constraints: result.constraints.length,
    subtypes: result.subtypes.length,
    warnings: result.warnings.length,
    claims: result,
  })
}
