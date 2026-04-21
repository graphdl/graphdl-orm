/**
 * AREST State Machine Editor — fetches a State Machine Definition
 * and its Transition facts, renders them as an editable state
 * graph, and computes an xstate 5 config from the facts so
 * consumers can run the machine in the browser.
 */
export {
  arestToXStateConfig,
  describeStatuses,
  listStatuses,
  type ArestStateMachineDefinition,
  type ArestTransition,
  type StatusInfo,
  type XStateConfig,
  type XStateNode,
} from './xstateConfig'

export {
  useStateMachine,
  type UseStateMachineOptions,
  type UseStateMachineResult,
} from './useStateMachine'

export {
  StateMachineEditor,
  type StateMachineEditorProps,
} from './StateMachineEditor'
