# Lesson M4: FIRE A TRANSITION

**Goal:** Move an entity's status by applying a declared transition event.
**Prereqs:** Lesson M3

`apply operation=transition` takes an `id` and an `event` (the name from `actions`'s response), then folds the event over the machine. The SM is `foldl transition s₀ E` — a pure replay. Firing an event that isn't legal from the current status is rejected with a violation; it is not silently dropped.

Transitions are facts too: each fire appends to the audit log (`audit` tool) with the entity id, operation, and outcome.

## Do it

~~~ apply
{
  "operation": "transition",
  "noun": "Order",
  "id": "m1-demo",
  "event": "place"
}
~~~

## Check

~~~ expect
status Order m1-demo is Placed
~~~

**Next:** [Lesson M5: Explain what happened](./05-explain.md)
