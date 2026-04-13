# Lesson M2: READ IT BACK

**Goal:** Use the three read verbs (`get`, `list`, and `query`) and know when each one fits.
**Prereqs:** Lesson M1

There are three tools that cover three scopes:

- `get noun=Order` lists all Orders as entity summaries.
- `get noun=Order id=m1-demo` fetches one entity by id.
- `query fact_type=Order_was_placed_by_Customer filter={"Customer":"globex"}` returns raw facts of a given fact type, optionally filtered by role bindings.

Use `get` for entity-centric views, which is what the UI usually needs. Use `query` for relationships; follow a fact type to find every entity playing a role.

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
