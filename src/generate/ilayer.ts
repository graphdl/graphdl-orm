/**
 * generateILayer — generates iLayer UI definitions from entity nouns.
 *
 * Each entity gets list, detail, create, and edit layers based on its permissions.
 * Ported from Generator.ts.bak lines 1559-1869. Adapted for DomainModel API.
 */

import { nameToKey } from './rmap'
import type { NounDef, FactTypeDef, StateMachineDef } from '../model/types'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** PascalCase → camelCase field ID. */
function toCamelCase(name: string): string {
  return nameToKey(name).replace(/^[A-Z]/, (c) => c.toLowerCase())
}

/** PascalCase → space-separated label. */
function toLabel(name: string): string {
  return name.replace(/([A-Z])/g, ' $1').trim()
}

/** Entity noun → URL slug (plural, lowercase, hyphenated). */
function toSlug(noun: { name: string; plural?: string }): string {
  if (noun.plural) return noun.plural
  return (
    noun.name
      .replace(/([A-Z])/g, '-$1')
      .toLowerCase()
      .replace(/^-/, '') + 's'
  )
}

/** Map a value noun to an iLayer field type. */
function mapFieldType(valueNoun: NounDef): { type: string; options?: string[] } {
  // Check enum first
  if (valueNoun.enumValues && valueNoun.enumValues.length > 0) {
    return { type: 'select', options: valueNoun.enumValues }
  }

  // Check format
  if (valueNoun.format) {
    if (valueNoun.format === 'email' || valueNoun.format === 'idn-email') return { type: 'email' }
    if (valueNoun.format === 'date' || valueNoun.format === 'date-time' || valueNoun.format === 'time') return { type: 'date' }
    if (valueNoun.format === 'uri' || valueNoun.format === 'uri-reference') return { type: 'text' }
  }

  // Check name for Email
  if (valueNoun.name && /email/i.test(valueNoun.name)) return { type: 'email' }

  // Check valueType
  switch (valueNoun.valueType) {
    case 'boolean':
      return { type: 'bool' }
    case 'number':
    case 'integer':
      return { type: 'numeric' }
    case 'string':
    default:
      return { type: 'text' }
  }
}

// ---------------------------------------------------------------------------
// Reading info type
// ---------------------------------------------------------------------------

interface ReadingInfo {
  subjectNounName: string
  objectNounName: string
  text: string
}

// ---------------------------------------------------------------------------
// State machine events helper
// ---------------------------------------------------------------------------

function getStateMachineEvents(entityNounName: string, smMap: Map<string, StateMachineDef>): string[] {
  const events: string[] = []
  for (const [, sm] of smMap) {
    if (sm.nounName !== entityNounName) continue
    for (const t of sm.transitions) {
      if (t.event && !events.includes(t.event)) events.push(t.event)
    }
  }
  return events
}

// ---------------------------------------------------------------------------
// generateILayer
// ---------------------------------------------------------------------------

/**
 * Generate iLayer UI definitions from domain model data.
 *
 * @param model - DomainModel providing nouns, factTypes, and stateMachines
 * @returns Object with `files` — a map of file paths to JSON strings
 */
export async function generateILayer(model: {
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  stateMachines(): Promise<Map<string, StateMachineDef>>
}): Promise<{ files: Record<string, string> }> {
  // ------ 1. Fetch domain-scoped data ------
  const [nounMap, ftMap, smMap] = await Promise.all([
    model.nouns(),
    model.factTypes(),
    model.stateMachines(),
  ])

  // ------ 2. Build reading infos from fact type roles ------
  const readingInfos: ReadingInfo[] = []

  for (const [, ft] of ftMap) {
    if (ft.arity < 2) continue // skip unary
    const subject = ft.roles[0]?.nounDef
    const object = ft.roles[1]?.nounDef
    if (!subject || !object || subject.name === object.name) continue
    readingInfos.push({ subjectNounName: subject.name, objectNounName: object.name, text: ft.reading })
  }

  const files: Record<string, string> = {}

  // ------ 3. Generate layers for each entity noun ------
  const entityNouns = [...nounMap.values()].filter((n) => n.objectType === 'entity')

  for (const entity of entityNouns) {
    const slug = toSlug(entity)
    const perms: string[] = entity.permissions || ['list', 'read', 'create', 'update', 'delete']
    const hasCreate = perms.includes('create')
    const hasRead = perms.includes('read')
    const hasUpdate = perms.includes('update')
    const hasList = perms.includes('list')
    const hasDelete = perms.includes('delete')

    // Find readings where this entity is the subject
    const entityReadings = readingInfos.filter((r) => r.subjectNounName === entity.name)

    // Separate value readings (fields) from entity readings (navigation)
    const fieldReadings: ReadingInfo[] = []
    const navReadings: ReadingInfo[] = []
    for (const r of entityReadings) {
      const objectNoun = nounMap.get(r.objectNounName)
      if (!objectNoun) continue
      if (objectNoun.objectType === 'value') {
        fieldReadings.push(r)
      } else if (objectNoun.objectType === 'entity') {
        navReadings.push(r)
      }
    }

    // Build fields from value readings (deduplicate by field ID)
    const fields: any[] = []
    const seenFieldIds = new Set<string>()
    for (const r of fieldReadings) {
      const valueNoun = nounMap.get(r.objectNounName)
      if (!valueNoun) continue
      const { type, options } = mapFieldType(valueNoun)
      const field: any = {
        id: toCamelCase(valueNoun.name),
        type,
        label: toLabel(valueNoun.name),
      }
      if (seenFieldIds.has(field.id)) continue
      seenFieldIds.add(field.id)
      if (options) field.options = options
      fields.push(field)
    }

    // Build state machine event buttons
    const events = getStateMachineEvents(entity.name, smMap)
    const eventButtons = events.map((event) => ({
      id: event,
      text: toLabel(event),
      address: `/state/${entity.name}/${event}`,
    }))

    // Build navigation from entity-to-entity relationships
    const navigation = navReadings.map((r) => {
      const relatedNoun = nounMap.get(r.objectNounName)
      const relSlug = toSlug(relatedNoun || { name: 'unknown', plural: 'unknowns' })
      return {
        text: relatedNoun?.name || 'Unknown',
        address: `/${relSlug}`,
      }
    })

    // -- List layer (NavigationLayer) --
    if (hasList || hasRead) {
      const listLayer: any = {
        name: slug,
        title: entity.name,
        type: 'layer',
        items: [
          {
            type: 'list',
            items: fields.slice(0, 3).map((f: any) => ({
              text: f.label,
              subtext: `{${f.id}}`,
              address: `/${slug}/{id}`,
            })),
          },
        ],
      }
      if (hasCreate) {
        listLayer.actionButtons = [{ id: 'create', text: `New ${entity.name}`, address: `/${slug}/new` }]
      }
      files[`layers/${slug}.json`] = JSON.stringify(listLayer, null, 2)
    }

    // -- Detail/Read layer (FormLayer) --
    if (hasRead) {
      const actionButtons: any[] = []
      if (hasUpdate) actionButtons.push({ id: 'edit', text: 'Edit', action: 'edit' })
      if (hasDelete) actionButtons.push({ id: 'delete', text: 'Delete', action: 'delete' })
      actionButtons.push(...eventButtons)

      const detailLayer: any = {
        name: `${slug}-detail`,
        title: entity.name,
        type: 'formLayer',
        layout: 'Rounded',
        fieldsets: [{ header: entity.name, fields: fields.map((f: any) => ({ ...f, type: 'label' })) }],
      }
      if (actionButtons.length) detailLayer.actionButtons = actionButtons
      if (navigation.length) detailLayer.navigation = navigation

      files[`layers/${slug}-detail.json`] = JSON.stringify(detailLayer, null, 2)
    }

    // -- Create layer (FormLayer with editable fields) --
    if (hasCreate) {
      const createLayer: any = {
        name: `${slug}-new`,
        title: `New ${entity.name}`,
        type: 'formLayer',
        layout: 'Rounded',
        fieldsets: [{ header: entity.name, fields }],
        actionButtons: [
          { id: 'save', text: 'Save', action: 'create' },
          { id: 'cancel', text: 'Cancel', address: `/${slug}` },
        ],
      }
      files[`layers/${slug}-new.json`] = JSON.stringify(createLayer, null, 2)
    }

    // -- Edit layer (FormLayer with editable fields + update action) --
    if (hasUpdate) {
      const editLayer: any = {
        name: `${slug}-edit`,
        title: `Edit ${entity.name}`,
        type: 'formLayer',
        layout: 'Rounded',
        fieldsets: [{ header: entity.name, fields }],
        actionButtons: [
          { id: 'save', text: 'Save', action: 'update' },
          { id: 'cancel', text: 'Cancel', address: `/${slug}/{id}` },
          ...eventButtons,
        ],
      }
      if (navigation.length) editLayer.navigation = navigation
      files[`layers/${slug}-edit.json`] = JSON.stringify(editLayer, null, 2)
    }
  }

  // ------ 4. Index navigation layer ------
  const indexItems = entityNouns
    .filter((e) => {
      const p: string[] = e.permissions || ['list', 'read']
      return p.includes('list') || p.includes('read')
    })
    .map((entity) => ({
      text: entity.name,
      subtext: entity.plural || toSlug(entity),
      address: `/${toSlug(entity)}`,
    }))

  const indexLayer = {
    name: 'index',
    title: 'Home',
    type: 'layer',
    items: [{ type: 'list', items: indexItems }],
  }

  files['layers/index.json'] = JSON.stringify(indexLayer, null, 2)

  return { files }
}
