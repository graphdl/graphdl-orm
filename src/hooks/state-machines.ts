import { ensure, type HookResult, EMPTY_RESULT, type HookContext } from './index'

/**
 * StateMachineDefinition afterCreate hook.
 *
 * Creates statuses, event types, and transitions from transition data
 * provided in the doc.
 */
export async function smDefinitionAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const transitions = doc.transitions as Array<{
    from: string; to: string; event: string; guard?: string
  }> | undefined

  if (!transitions || transitions.length === 0) return EMPTY_RESULT

  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain
  const definitionId = doc.id

  // Find-or-create the target noun
  const nounName = doc.title || doc.name
  if (nounName) {
    await ensure(
      db, 'nouns',
      { name: { equals: nounName }, domain_id: { equals: domainId } },
      { name: nounName, objectType: 'entity', domain: domainId },
    )
  }

  // Collect unique status names and event names
  const statusNames = new Set<string>()
  const eventNames = new Set<string>()
  for (const t of transitions) {
    statusNames.add(t.from)
    statusNames.add(t.to)
    eventNames.add(t.event)
  }

  // Find-or-create statuses
  const statusMap = new Map<string, string>() // name → id
  for (const name of statusNames) {
    const { doc: status, created } = await ensure(
      db, 'statuses',
      {
        name: { equals: name },
        state_machine_definition_id: { equals: definitionId },
      },
      {
        name,
        stateMachineDefinition: definitionId,
        domain: domainId,
      },
    )
    statusMap.set(name, status.id)
    if (created) {
      result.created['statuses'] = [...(result.created['statuses'] || []), status]
    }
  }

  // Find-or-create event types
  const eventMap = new Map<string, string>() // name → id
  for (const name of eventNames) {
    const { doc: eventType, created } = await ensure(
      db, 'event-types',
      { name: { equals: name }, domain_id: { equals: domainId } },
      { name, domain: domainId },
    )
    eventMap.set(name, eventType.id)
    if (created) {
      result.created['event-types'] = [...(result.created['event-types'] || []), eventType]
    }
  }

  // Create transitions
  for (const t of transitions) {
    const fromId = statusMap.get(t.from)!
    const toId = statusMap.get(t.to)!
    const eventId = eventMap.get(t.event)!

    const transition = await db.createInCollection('transitions', {
      fromStatus: fromId,
      toStatus: toId,
      eventType: eventId,
      domain: domainId,
    })
    result.created['transitions'] = [...(result.created['transitions'] || []), transition]

    // Create guard if provided
    if (t.guard) {
      const guard = await db.createInCollection('guards', {
        name: t.guard,
        transition: transition.id,
        domain: domainId,
      })
      result.created['guards'] = [...(result.created['guards'] || []), guard]
    }
  }

  return result
}
