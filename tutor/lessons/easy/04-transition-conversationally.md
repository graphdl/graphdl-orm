# Lesson E4: MOVE AN ORDER ALONG

**Goal:** Advance an entity through its state machine using plain language.
**Prereqs:** Lesson E3

State machines in AREST are declared via readings — "Transition 'place' is from Status 'Draft'..." — and they're already compiled. You don't have to remember the event name: ask the engine what actions are legal right now and pick one.

In Easy mode the agent picks for you. You say "place it" and the agent translates to the right `apply transition` call.

## Do it

Propose a small SM for Order if one isn't declared yet:

~~~ propose
{
  "rationale": "Orders need a lifecycle: draft → placed → shipped.",
  "target_domain": "orders",
  "readings": [
    "State Machine Definition 'Order' is for Noun 'Order'.",
    "Status 'Draft' is defined in State Machine Definition 'Order'.",
    "Status 'Placed' is defined in State Machine Definition 'Order'.",
    "Status 'Shipped' is defined in State Machine Definition 'Order'.",
    "Status 'Draft' is initial.",
    "Transition 'place' is defined in State Machine Definition 'Order'. Transition 'place' is from Status 'Draft'. Transition 'place' is to Status 'Placed'. Transition 'place' is triggered by Event Type 'place'.",
    "Transition 'ship' is defined in State Machine Definition 'Order'. Transition 'ship' is from Status 'Placed'. Transition 'ship' is to Status 'Shipped'. Transition 'ship' is triggered by Event Type 'ship'."
  ]
}
~~~

Then move `acme-1` from Draft to Placed in English:

~~~ apply
{ "operation": "transition", "noun": "Order", "id": "acme-1", "event": "place" }
~~~

## Check

~~~ expect
status Order acme-1 is Placed
~~~

**Next:** [Lesson M1: Create an entity](../medium/01-apply-create.md)
