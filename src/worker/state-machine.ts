/**
 * State machine initialization and transitions for Entity DOs.
 *
 * On entity creation: if the noun type has a state machine definition,
 * initialize at the initial state (first status with no incoming transitions).
 *
 * On event: validate the transition from current status, return the new status.
 * The entity's state is stored as _status/_statusId on the Entity DO's data.
 */

export interface StateMachineInit {
  definitionId: string
  definitionTitle: string
  initialStatus: string
  initialStatusId: string
}

export interface TransitionOption {
  transitionId: string
  event: string
  eventTypeId: string
  targetStatus: string
  targetStatusId: string
}

export interface TransitionResult {
  transitionId: string
  event: string
  previousStatus: string
  previousStatusId: string
  newStatus: string
  newStatusId: string
}

export interface DomainDBStub {
  findInCollection(collection: string, where: any, opts?: any): Promise<{ docs: any[]; totalDocs: number }>
}

/**
 * Look up the state machine definition for a noun and determine the initial status.
 * Returns null if the noun has no state machine definition.
 */
export async function getInitialState(
  domainDB: DomainDBStub,
  nounName: string,
  domainId: string,
): Promise<StateMachineInit | null> {
  const nouns = await domainDB.findInCollection('nouns', {
    name: { equals: nounName },
  }, { limit: 1 })
  if (!nouns.docs.length) return null
  const nounId = nouns.docs[0].id

  const defs = await domainDB.findInCollection('state-machine-definitions', {
    noun: { equals: nounId },
  }, { limit: 1 })
  if (!defs.docs.length) return null

  const def = defs.docs[0]
  const defId = def.id as string

  const statuses = await domainDB.findInCollection('statuses', {
    stateMachineDefinition: { equals: defId },
  }, { limit: 100, sort: 'createdAt' })
  if (!statuses.docs.length) return null

  let initialStatus = statuses.docs[0]
  for (const s of statuses.docs) {
    const incoming = await domainDB.findInCollection('transitions', {
      to: { equals: s.id },
    }, { limit: 1 })
    if (!incoming.docs.length) {
      initialStatus = s
      break
    }
  }

  return {
    definitionId: defId,
    definitionTitle: (def.title || def.name || nounName) as string,
    initialStatus: initialStatus.name as string,
    initialStatusId: initialStatus.id as string,
  }
}

/**
 * Get valid transitions from the current status.
 * Returns available events and their target statuses.
 */
export async function getValidTransitions(
  domainDB: DomainDBStub,
  definitionId: string,
  currentStatusId: string,
): Promise<TransitionOption[]> {
  const transitions = await domainDB.findInCollection('transitions', {
    from: { equals: currentStatusId },
    stateMachineDefinition: { equals: definitionId },
  }, { limit: 100 })

  const options: TransitionOption[] = []
  for (const t of transitions.docs) {
    // Get target status name
    const targetStatuses = await domainDB.findInCollection('statuses', {
      id: { equals: t.to },
    }, { limit: 1 })
    // Get event type name
    const eventTypes = await domainDB.findInCollection('event-types', {
      id: { equals: t.eventType },
    }, { limit: 1 })

    if (targetStatuses.docs.length && eventTypes.docs.length) {
      options.push({
        transitionId: t.id as string,
        event: eventTypes.docs[0].name as string,
        eventTypeId: t.eventType as string,
        targetStatus: targetStatuses.docs[0].name as string,
        targetStatusId: targetStatuses.docs[0].id as string,
      })
    }
  }

  return options
}

/**
 * Apply a transition by event name from the current status.
 * Returns the new status, or null if the event is not valid from the current state.
 */
export async function applyTransition(
  domainDB: DomainDBStub,
  definitionId: string,
  currentStatusId: string,
  eventName: string,
): Promise<TransitionResult | null> {
  const options = await getValidTransitions(domainDB, definitionId, currentStatusId)
  const match = options.find(o => o.event === eventName)
  if (!match) return null

  // Get current status name
  const currentStatuses = await domainDB.findInCollection('statuses', {
    id: { equals: currentStatusId },
  }, { limit: 1 })
  const currentName = currentStatuses.docs.length ? currentStatuses.docs[0].name as string : 'unknown'

  return {
    transitionId: match.transitionId,
    event: eventName,
    previousStatus: currentName,
    previousStatusId: currentStatusId,
    newStatus: match.targetStatus,
    newStatusId: match.targetStatusId,
  }
}
