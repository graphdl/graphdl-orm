# Lesson H7: A DECLARED STATE MACHINE

**Goal:** Write a state machine as a set of facts (with no code and no DSL) and fire a transition.
**Prereqs:** Lesson H6

A state machine in FORML2 is declared exactly like anything else: facts about SM Definitions, Statuses, Transitions, and Event Types. The metamodel spec (`readings/state.md`) says each Transition belongs to exactly one SM, each SM is for exactly one Noun, and the initial status is stated as a fact on the SM.

Always include `Transition 'X' is defined in State Machine Definition 'Y'`. Without that fact, the compiler falls back to a heuristic that can pick the wrong SM when two SMs share a status name.

## Do it

~~~ compile
State Machine Definition 'Case' is for Noun 'Case'.
Status 'Open' is defined in State Machine Definition 'Case'.
Status 'Investigating' is defined in State Machine Definition 'Case'.
Status 'Solved' is defined in State Machine Definition 'Case'.
Status 'Closed' is defined in State Machine Definition 'Case'.
Status 'Open' is initial in State Machine Definition 'Case'.

Transition 'investigate' is defined in State Machine Definition 'Case'.
Transition 'investigate' is from Status 'Open'.
Transition 'investigate' is to Status 'Investigating'.
Transition 'investigate' is triggered by Event Type 'investigate'.

Transition 'solve' is defined in State Machine Definition 'Case'.
Transition 'solve' is from Status 'Investigating'.
Transition 'solve' is to Status 'Solved'.
Transition 'solve' is triggered by Event Type 'solve'.

Transition 'close' is defined in State Machine Definition 'Case'.
Transition 'close' is from Status 'Solved'.
Transition 'close' is to Status 'Closed'.
Transition 'close' is triggered by Event Type 'close'.
~~~

~~~ apply
{ "operation": "create", "noun": "Case", "id": "speckled-band", "fields": {} }
~~~

~~~ apply
{ "operation": "transition", "noun": "Case", "id": "speckled-band", "event": "investigate" }
~~~

## Check

~~~ expect
status Case speckled-band is Investigating
~~~

**NOTE:** The machine fold (`foldl transition s₀ E`) is pure replay. External events from a queue or webhook enter the same stream without needing a local fact. `apply transition` is just the nearest case.

**Next:** [Lesson H8: Self-modification at runtime](./08-self-modification.md)
