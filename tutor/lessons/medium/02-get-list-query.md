# Lesson M2: READ IT BACK

**Goal:** Use the three read verbs — `get`, `list`, `query` — and know when each fits.
**Prereqs:** Lesson M1

Three tools, three scopes:

- `get noun=Order` — list all Orders as entity summaries.
- `get noun=Order id=m1-demo` — fetch one entity by id.
- `query fact_type=Order_was_placed_by_Customer filter={"Customer":"globex"}` — return raw facts of a given fact type, optionally filtered by role bindings.

`get` is for entity-centric views (what the UI usually wants). `query` is for relationships — follow a fact type to find every entity playing a role.

## Do it

~~~ get
{ "noun": "Order" }
~~~

~~~ get
{ "noun": "Order", "id": "m1-demo" }
~~~

~~~ query
{ "fact_type": "Order_was_placed_by_Customer", "filter": { "Customer": "globex" } }
~~~

## Check

~~~ expect
list Order contains {"id": "m1-demo"}
~~~

**Next:** [Lesson M3: Discover what you can do](./03-actions.md)
