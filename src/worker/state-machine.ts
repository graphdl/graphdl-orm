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
  const def = defs.find(d => d.data.noun === nounId || d.data.nounId === nounId)
  if (!def) return null
  const defId = def.id

  // Load statuses for this definition
  const allStatuses = await loadEntities(registry, getStub, 'Status', domain)
  const statuses = allStatuses.filter(s =>
    s.data.stateMachineDefinition === defId || s.data.stateMachineDefinitionId === defId,
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
  const allTransitions = await loadEntities(registry, getStub, 'Transition', domain)
  const transitions = allTransitions.filter(t =>
    (t.data.from === currentStatusId || t.data.fromId === currentStatusId) &&
    (t.data.stateMachineDefinition === definitionId || t.data.stateMachineDefinitionId === definitionId),
  )

  const options: TransitionOption[] = []
  for (const t of transitions) {
    const toId = (t.data.to || t.data.toId) as string
    const eventTypeId = (t.data.eventType || t.data.eventTypeId) as string

    // Resolve target status and event type by direct entity lookup
    const [targetStatus, eventType] = await Promise.all([
      toId ? loadEntity(getStub, toId) : null,
      eventTypeId ? loadEntity(getStub, eventTypeId) : null,
    ])

    if (targetStatus && eventType) {
      options.push({
        transitionId: t.id,
        event: eventType.data.name as string,
        eventTypeId,
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
