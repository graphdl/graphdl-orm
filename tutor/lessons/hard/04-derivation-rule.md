# Lesson H4: A DERIVATION RULE

**Goal:** Declare a fact that's computed from other facts, and watch the least fixed point do its job.
**Prereqs:** Lesson H3

Some facts are not asserted; they are derived. A "premium customer" is not a thing you type. It is the output of a rule over orders and thresholds. Derivation rules in FORML2 use `iff` and are forward-chained to the LFP on every `apply`. You never call them yourself; the engine does, every time P changes.

Rules are monotonic, so the fixed point exists and is unique. Reach is bounded by the size of the population. The derivation chain is recorded, so `explain` shows which rules fired in which order.

## Do it

~~~ compile
Customer is premium iff Customer placed at least 3 Orders and each Order has Amount greater than 100.
~~~

Then create a few orders and query:

~~~ apply
{ "operation": "create", "noun": "Order", "id": "p1", "fields": { "Customer": "vip", "Amount": "200" } }
~~~
~~~ apply
{ "operation": "create", "noun": "Order", "id": "p2", "fields": { "Customer": "vip", "Amount": "150" } }
~~~
~~~ apply
{ "operation": "create", "noun": "Order", "id": "p3", "fields": { "Customer": "vip", "Amount": "300" } }
~~~

## Check

~~~ expect
query Customer_is_premium contains {"Customer": "vip"}
~~~

**NOTE:** Rules only ADD facts to P; they never retract. An entity that stops being premium (when its Amount drops below threshold) is reflected on the NEXT `apply`, since the LFP is computed per request rather than cached.

**Next:** [Lesson H5: A deontic constraint](./05-deontic-constraint.md)
