/**
 * AREST-wired widgets — domain-specific renderings that compose the
 * generic views + hooks into richer surfaces.
 *
 * OutcomesFeed (the outcomes domain — readings/outcomes.md) surfaces
 * Violations and Failures as first-class facts. The whitepaper §8
 * calls this out as the load-bearing "no silent paths" property: an
 * operator SHOULD always be able to see what the system refused and
 * why. These widgets give them that view.
 */
export {
  ViolationsFeed,
  FailuresFeed,
  OutcomesBoard,
  type ViolationsFeedProps,
  type FailuresFeedProps,
  type OutcomesBoardProps,
  type OutcomesFeedOptions,
  type Violation,
  type Failure,
  type Severity,
  type FailureType,
} from './OutcomesFeed'
