/**
 * generateILayer — generates iLayer UI definitions from entity nouns.
 *
 * Each entity gets list, detail, create, and edit layers based on its permissions.
 * Ported from Generator.ts.bak lines 1559-1869. Adapted for DomainModel API.
 *
 * Display field selection is derived from readings (fact types), not from
 * positional ordering. The readings encode semantic roles: "has Subject" is
 * an identifying label, "has IssueType" is a categorization, "Customer submits"
 * is an agency relationship. The generator reads these semantics to pick the
 * most meaningful fields for list display.
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
// Semantic display field selection — derived from readings
// ---------------------------------------------------------------------------

/**
 * Collect the full supertype chain for an entity (inclusive of itself).
 * e.g. SupportRequest → [SupportRequest, Request]
 */
function supertypeChain(entity: NounDef, nounMap: Map<string, NounDef>): string[] {
  const chain: string[] = [entity.name]
  let current = entity
  while (current.superType) {
    const parentName = typeof current.superType === 'string' ? current.superType : current.superType.name
    chain.push(parentName)
    const parent = nounMap.get(parentName)
    if (!parent) break
    current = parent
  }
  return chain
}

/**
 * Derive semantically ranked display fields for a list layer from the
 * readings (fact types). The readings encode the roles each noun plays:
 *
 *   "Customer submits SupportRequest" → Customer is an agent (person who did something)
 *   "Request has Subject"             → Subject is an identifying label
 *   "SupportRequest has IssueType"    → IssueType is a categorization (enum)
 *
 * Returns { primary, secondary } field IDs for the list layer.
 */
function selectDisplayFields(
  entity: NounDef,
  factTypes: Map<string, FactTypeDef>,
  nounMap: Map<string, NounDef>,
  availableFieldIds: Set<string>,
): { primary: string; secondary?: string } {
  const names = new Set(supertypeChain(entity, nounMap))

  // Classify each value-type field by its semantic role in the readings
  const identifiers: string[] = []     // Subject, Title, Name, Label — identifying labels
  const agents: string[] = []          // FK to person-like entities (Customer, User)
  const categorizations: string[] = [] // Enum value types (IssueType, Priority)
  const temporal: string[] = []        // Date, Time, Timestamp fields
  const descriptions: string[] = []   // Description, Body, Content — long-form text
  const others: string[] = []         // Everything else

  for (const ft of factTypes.values()) {
    if (ft.arity !== 2) continue

    // Find the role played by this entity (or its supertype)
    const entityRole = ft.roles.find(r => names.has(r.nounName))
    if (!entityRole) continue

    // Find the other role in the fact type
    const otherRole = ft.roles.find(r => r !== entityRole)
    if (!otherRole) continue

    const otherNoun = otherRole.nounDef || nounMap.get(otherRole.nounName)
    if (!otherNoun) continue

    const fieldId = toCamelCase(otherNoun.name)

    if (otherNoun.objectType === 'entity') {
      // FK to another entity — check if the entity has a person-like reference scheme
      // (identified by EmailAddress, Name, ContactName, etc.)
      const refScheme = otherNoun.referenceScheme
      if (refScheme?.some(rs => /email|name|contact/i.test(rs.name))) {
        agents.push(fieldId)
      }
    } else {
      // Value type — classify by name semantics and value characteristics
      const n = otherNoun.name

      if (/^(Subject|Title|Name|Label|DisplayName|Heading|EmailAddress|Email|ContactEmail)$/i.test(n)) {
        identifiers.push(fieldId)
      } else if (otherNoun.enumValues && otherNoun.enumValues.length > 1) {
        categorizations.push(fieldId)
      } else if (/Date|Time|Timestamp|At$/i.test(n)) {
        temporal.push(fieldId)
      } else if (/^(Description|Body|Content|Summary|Text|Details)$/i.test(n)) {
        descriptions.push(fieldId)
      } else {
        others.push(fieldId)
      }
    }
  }

  // Rank: agents (who) → identifiers (what) → descriptions → categorizations → others
  const ranked = [
    ...agents,
    ...identifiers,
    ...descriptions,
    ...categorizations,
    ...temporal,
    ...others,
  ]

  // Filter to only fields that actually exist in the schema
  const available = ranked.filter(f => availableFieldIds.has(f))

  // If nothing matched from readings, fall back to any available non-id field
  if (available.length === 0) {
    const fallback = [...availableFieldIds].filter(f => f !== 'id')
    return { primary: fallback[0] || 'id', secondary: fallback[1] }
  }

  const primary = available[0]

  // For secondary, prefer a different category than primary
  const primaryCategory = agents.includes(primary) ? 'agent'
    : identifiers.includes(primary) ? 'identifier'
    : descriptions.includes(primary) ? 'description'
    : categorizations.includes(primary) ? 'categorization'
    : 'other'

  // Pick secondary from a complementary category (exclude temporal — those go in the date slot)
  const secondaryPreference =
    primaryCategory === 'agent' || primaryCategory === 'identifier'
      ? [...categorizations, ...descriptions, ...others]
      : primaryCategory === 'description'
        ? [...categorizations, ...others]
        : [...identifiers, ...agents, ...descriptions, ...others]

  const secondary = secondaryPreference
    .filter(f => f !== primary && availableFieldIds.has(f))[0]

  // Date slot: first temporal field, or createdAt as implicit fallback
  const date = temporal.filter(f => availableFieldIds.has(f))[0] || 'createdAt'

  // Build grid cell from semantic classification
  // Row 0: primary (start) + date (end)
  // Row 1: secondary (start, span full width)
  const gridElements: Array<{
    field: string; row: number; column: number;
    columnSpan?: number; horizontalAlignment?: string;
    style?: string; format?: string
  }> = []

  if (primary) {
    gridElements.push({ field: primary, row: 0, column: 0, style: 'primary', format: 'text' })
  }
  if (date) {
    gridElements.push({ field: date, row: 0, column: 1, horizontalAlignment: 'end', style: 'date', format: 'date' })
  }
  if (secondary) {
    gridElements.push({ field: secondary, row: 1, column: 0, columnSpan: 2, style: 'muted', format: 'text' })
  }

  const gridCell = {
    rows: secondary ? 2 : 1,
    columns: 2,
    elements: gridElements,
  }

  return { primary, secondary, date, gridCell }
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
      // Derive display fields from readings semantics, not positional ordering
      const availableFieldIds = new Set(fields.map(f => f.id))
      const chosen = selectDisplayFields(entity, ftMap, nounMap, availableFieldIds)

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
        // Display field mapping + grid cell layout — both derived from readings
        displayFields: chosen,
        gridCell: chosen.gridCell,
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
