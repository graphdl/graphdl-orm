/**
 * generateILayer — generates iLayer UI definitions from entity nouns.
 *
 * Each entity gets list, detail, create, and edit layers based on its permissions.
 * Ported from Generator.ts.bak lines 1559-1869. Adapted for DO findInCollection API.
 */

import { nameToKey } from './rmap'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Fetch all docs from a collection, handling pagination by using a large limit. */
async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

/** PascalCase → camelCase field ID. */
function toCamelCase(name: string): string {
  return nameToKey(name).replace(/^[A-Z]/, (c) => c.toLowerCase())
}

/** PascalCase → space-separated label. */
function toLabel(name: string): string {
  return name.replace(/([A-Z])/g, ' $1').trim()
}

/** Entity noun → URL slug (plural, lowercase, hyphenated). */
function toSlug(noun: any): string {
  if (noun.plural) return noun.plural
  return (
    noun.name
      .replace(/([A-Z])/g, '-$1')
      .toLowerCase()
      .replace(/^-/, '') + 's'
  )
}

/** Map a value noun to an iLayer field type. */
function mapFieldType(valueNoun: any): { type: string; options?: string[] } {
  // Check enum first
  if (valueNoun.enum) {
    const options = valueNoun.enum
      .split(',')
      .map((s: string) => s.trim())
      .filter(Boolean)
    return { type: 'select', options }
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
  subjectNounId: string
  objectNounId: string
  text: string
}

// ---------------------------------------------------------------------------
// generateILayer
// ---------------------------------------------------------------------------

/**
 * Generate iLayer UI definitions from domain model data.
 *
 * @param db       - Durable Object stub with `findInCollection(slug, where?, options?)`
 * @param domainId - The domain ID to scope nouns, readings, and state machines
 * @returns Object with `files` — a map of file paths to JSON strings
 */
export async function generateILayer(db: any, domainId: string): Promise<{ files: Record<string, string> }> {
  const domainFilter = { domain: { equals: domainId } }

  // ------ 1. Fetch domain-scoped data ------
  const [nouns, readings, stateMachineDefinitions] = await Promise.all([
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'readings', domainFilter),
    fetchAll(db, 'state-machine-definitions', domainFilter),
  ])

  // ------ 2. Fetch cross-domain data ------
  const allNouns = await fetchAll(db, 'nouns')
  const allNounById = new Map(allNouns.map((n: any) => [n.id, n]))

  const allRoles = await fetchAll(db, 'roles')
  const roleById = new Map(allRoles.map((r: any) => [r.id, r]))

  // ------ 3. Build reading infos ------
  const entityNouns = nouns.filter((n: any) => n.objectType === 'entity')
  const readingInfos: ReadingInfo[] = []

  for (const reading of readings) {
    const roleIds = (reading.roles || []) as string[]
    if (roleIds.length < 2) continue

    const firstRole = roleById.get(roleIds[0])
    const secondRole = roleById.get(roleIds[1])
    if (!firstRole || !secondRole) continue

    const subjectNounVal = (firstRole as any).noun?.value
    const subjectNounId = typeof subjectNounVal === 'string' ? subjectNounVal : subjectNounVal?.id
    const objectNounVal = (secondRole as any).noun?.value
    const objectNounId = typeof objectNounVal === 'string' ? objectNounVal : objectNounVal?.id

    if (subjectNounId && objectNounId) {
      readingInfos.push({ subjectNounId, objectNounId, text: reading.text })
    }
  }

  const files: Record<string, string> = {}

  // ------ 4. Resolve state machine events for an entity noun ------
  async function getStateMachineEvents(entityNounId: string): Promise<string[]> {
    const events: string[] = []
    for (const smDef of stateMachineDefinitions) {
      const nounRef = smDef.noun as any
      const nounId = typeof nounRef?.value === 'string' ? nounRef.value : nounRef?.value?.id
      if (nounId !== entityNounId) continue

      const statuses = await fetchAll(db, 'statuses', { stateMachineDefinition: { equals: smDef.id } })

      for (const status of statuses) {
        const transitions = (status.transitions?.docs || []) as any[]
        for (const t of transitions) {
          const eventTypeId = typeof t.eventType === 'string' ? t.eventType : t.eventType?.id
          if (!eventTypeId) continue

          // Look up the event type from the event-types collection
          const eventTypes = await fetchAll(db, 'event-types', { id: { equals: eventTypeId } })
          const eventType = eventTypes[0] || (typeof t.eventType === 'object' ? t.eventType : null)

          if (eventType?.name && !events.includes(eventType.name)) {
            events.push(eventType.name)
          }
        }
      }
    }
    return events
  }

  // ------ 5. Generate layers for each entity noun ------
  for (const entity of entityNouns) {
    const slug = toSlug(entity)
    const perms: string[] = entity.permissions || []
    const hasCreate = perms.includes('create')
    const hasRead = perms.includes('read')
    const hasUpdate = perms.includes('update')
    const hasList = perms.includes('list')
    const hasDelete = perms.includes('delete')

    // Find readings where this entity is the subject
    const entityReadings = readingInfos.filter((r) => r.subjectNounId === entity.id)

    // Separate value readings (fields) from entity readings (navigation)
    const fieldReadings: ReadingInfo[] = []
    const navReadings: ReadingInfo[] = []
    for (const r of entityReadings) {
      const objectNoun = allNounById.get(r.objectNounId) as any
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
      const valueNoun = allNounById.get(r.objectNounId) as any
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
    const events = await getStateMachineEvents(entity.id)
    const eventButtons = events.map((event) => ({
      id: event,
      text: toLabel(event),
      address: `/state/${entity.name}/${event}`,
    }))

    // Build navigation from entity-to-entity relationships
    const navigation = navReadings.map((r) => {
      const relatedNoun = allNounById.get(r.objectNounId) as any
      const relSlug = toSlug(relatedNoun || { name: 'unknown', plural: 'unknowns' })
      return {
        text: relatedNoun?.name || 'Unknown',
        address: `/${relSlug}`,
      }
    })

    // ── List layer (NavigationLayer) ──
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

    // ── Detail/Read layer (FormLayer) ──
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

    // ── Create layer (FormLayer with editable fields) ──
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

    // ── Edit layer (FormLayer with editable fields + update action) ──
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

  // ------ 6. Index navigation layer ------
  const indexItems = entityNouns
    .filter((e: any) => {
      const p: string[] = e.permissions || []
      return p.includes('list') || p.includes('read')
    })
    .map((entity: any) => ({
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
