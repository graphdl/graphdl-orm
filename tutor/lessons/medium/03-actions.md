# Lesson M3: DISCOVER WHAT YOU CAN DO

**Goal:** Ask the engine which transitions are currently legal for an entity, without knowing the schema.
**Prereqs:** Lesson M2

REST without the "hypermedia as the engine of application state" part is just RPC. `actions` is the HATEOAS projection: given an entity and its current status, it returns the transitions that apply (event names, target statuses, HTTP method, and href), computed directly from fact-type cells.

This is how a UI stays in sync with the schema without any hardcoded state chart: the UI asks the engine, then renders the buttons the engine returns.

## Do it

~~~ actions
{ "noun": "Order", "id": "m1-demo" }
~~~

## Check

~~~ expect
get Order m1-demo equals {"id": "m1-demo"}
~~~

**NOTE:** The response includes both `transitions` (SM moves) and `navigation` (parent/children/peers projected from UCs). Both are views of the same population, and there is no separate navigation table.

**Next:** [Lesson M4: Fire a transition](./04-apply-transition.md)
