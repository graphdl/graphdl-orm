/**
 * State machine initialization and transitions for Entity DOs.
 *
 * On entity creation: if the noun type has a state machine definition,
 * initialize at the initial state (status with isInitial flag, or first
 * status with no incoming transitions as fallback).
 *
 * On event: validate the transition from current status, return the new status.
 * The entity's state is stored as _status/_statusId on the Entity DO's data.
 *
 * Queries use Registry+EntityDB fan-out instead of DomainDB SQL.
 */

import type { RegistryReadStub, EntityReadStub, EntityRecord } from '../api/entity-routes'
import { loadEntities, loadEntity } from './fan-out'
import type { GetStubFn } from './fan-out'
export type { GetStubFn } from './fan-out'

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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Look up the state machine definition for a noun and determine the initial status.
 * Returns null if the noun has no state machine definition.
 */
export async function getInitialState(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  nounName: string,
  domain?: string,
): Promise<StateMachineInit | null> {
  // Load all nouns to resolve the noun ID
  const nouns = await loadEntities(registry, getStub, 'Noun', domain)
  const noun = nouns.find(n => n.data.name === nounName)
  if (!noun) return null
  const nounId = noun.id

  // Load state machine definitions for this noun
  const defs = await loadEntities(registry, getStub, 'State Machine Definition', domain)
  const def = defs.find(d =>
    d.data.noun === nounId || d.data.nounId === nounId ||
    d.data.forNoun === nounName || d.data.noun === nounName ||
    d.data.name === nounName,
  )
  if (!def) return null
  const defId = def.id

  // Load statuses for this definition
  const allStatuses = await loadEntities(registry, getStub, 'Status', domain)
  const defName = def.data.name as string || ''
  const statuses = allStatuses.filter(s =>
    s.data.stateMachineDefinition === defId || s.data.stateMachineDefinitionId === defId ||
    s.data.definedInStateMachineDefinition === defId || s.data.definedInStateMachineDefinition === defName ||
    s.data.stateMachineDefinition === defName,
  )
  if (!statuses.length) return null

  // Prefer the explicit isInitial flag if any status has it
  const flagged = statuses.find(s => s.data.isInitial === true)
  let initialStatus: EntityRecord

  if (flagged) {
    initialStatus = flagged
  } else {
    // Fall back to heuristic: first status with no incoming transitions
    const allTransitions = await loadEntities(registry, getStub, 'Transition', domain)

    initialStatus = statuses[0]
    for (const s of statuses) {
      const hasIncoming = allTransitions.some(t => t.data.to === s.id || t.data.toId === s.id)
      if (!hasIncoming) {
        initialStatus = s
        break
      }
    }
  }

  return {
    definitionId: defId,
    definitionTitle: (def.data.title || def.data.name || nounName) as string,
    initialStatus: initialStatus.data.name as string,
    initialStatusId: initialStatus.id,
  }
}

/**
 * Get valid transitions from the current status.
 * Returns available events and their target statuses.
 */
export async function getValidTransitions(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  definitionId: string,
  currentStatusId: string,
  domain?: string,
): Promise<TransitionOption[]> {
  // Load transitions for this definition from the current status
  // Load all transitions and statuses for this domain
  const allTransitions = await loadEntities(registry, getStub, 'Transition', domain)
  const allStatuses = await loadEntities(registry, getStub, 'Status', domain)

  // Resolve current status name for name-based matching
  const currentStatus = allStatuses.find(s => s.id === currentStatusId)
  const currentStatusName = currentStatus?.data.name as string || ''

  // Filter transitions from the current status
  // Support both ID-based (fromId) and name-based (fromStatus) references
  const transitions = allTransitions.filter(t => {
    const fromMatch = t.data.from === currentStatusId || t.data.fromId === currentStatusId ||
      t.data.fromStatus === currentStatusName || t.data.from === currentStatusName
    return fromMatch
  })

  const options: TransitionOption[] = []
  for (const t of transitions) {
    // Resolve target status — by ID or by name
    const toRef = (t.data.to || t.data.toId || t.data.toStatus) as string
    let targetStatus = toRef ? await loadEntity(getStub, toRef).catch(() => null) : null
    if (!targetStatus) {
      targetStatus = allStatuses.find(s => s.data.name === toRef) || null
    }

    // Resolve event type — by ID or by name
    const eventRef = (t.data.eventType || t.data.eventTypeId || t.data.triggeredByEventType) as string
    let eventType = eventRef ? await loadEntity(getStub, eventRef).catch(() => null) : null
    // If event is referenced by name (not a UUID), use it directly
    const eventName = eventType?.data.name as string || eventRef || ''

    if (targetStatus) {
      options.push({
        transitionId: t.id,
        event: eventName,
        eventTypeId: eventType?.id || eventRef,
        targetStatus: targetStatus.data.name as string,
        targetStatusId: targetStatus.id,
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
  registry: RegistryReadStub,
  getStub: GetStubFn,
  definitionId: string,
  currentStatusId: string,
  eventName: string,
  domain?: string,
): Promise<TransitionResult | null> {
  const options = await getValidTransitions(registry, getStub, definitionId, currentStatusId, domain)
  const match = options.find(o => o.event === eventName)
  if (!match) return null

  // Resolve current status name
  const currentStatus = await loadEntity(getStub, currentStatusId)
  const currentName = currentStatus ? currentStatus.data.name as string : 'unknown'

  return {
    transitionId: match.transitionId,
    event: eventName,
    previousStatus: currentName,
    previousStatusId: currentStatusId,
    newStatus: match.targetStatus,
    newStatusId: match.targetStatusId,
  }
}
