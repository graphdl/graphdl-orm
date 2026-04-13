# Lesson M5: EXPLAIN WHAT HAPPENED

**Goal:** Read the derivation chain and audit trail for an entity.
**Prereqs:** Lesson M4

Every value in the API is derived from facts. `explain` is how you see the derivation: the facts that fed a rule, the rule that fired, the order in which rules fired, and the result. It also returns the audit log for the entity, covering every `apply` that touched it, with its operation, outcome, sequence, and sender.

Use it when the engine does something you didn't expect. Use it to audit a transition post-mortem. Use it as the transparency layer for anything regulatory.

## Do it

~~~ explain
{ "noun": "Order", "id": "m1-demo" }
~~~

## Check

~~~ expect
get Order m1-demo equals {"id": "m1-demo"}
~~~

**NOTE:** `audit_trail` is filtered to entries whose `entity` matches the id. If the list is empty, the entity was likely created outside the audited pipeline; federation fetches, for example, do not write audit entries.

**Next:** [Lesson H1: Declare a noun](../hard/01-noun.md)
