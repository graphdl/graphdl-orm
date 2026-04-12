# Lesson M1: CREATE AN ENTITY

**Goal:** Add a new entity to the population with an explicit `apply` call.
**Prereqs:** Lesson E4 (or an Orders domain already compiled)

In Easy mode you asked the agent to place an order. Now you make the call yourself. `apply` takes an `operation`, a `noun`, an optional `id`, and a `fields` map. The engine runs the full pipeline (resolve → derive → validate → emit) and returns the new entity plus its SM state, violations, transitions, and navigation links.

Read the response carefully — it is everything a REST client would need to continue without any out-of-band knowledge of the schema.

## Do it

~~~ apply
{
  "operation": "create",
  "noun": "Order",
  "id": "m1-demo",
  "fields": { "Customer": "globex", "Amount": "400" }
}
~~~

## Check

~~~ expect
list Order contains {"id": "m1-demo", "Customer": "globex", "Amount": "400"}
~~~

**NOTE:** If `fields` omits a mandatory role the response carries a violation and `rejected: true`. The entity is NOT persisted. Read `violations[].constraint_text` — it's the same FORML2 reading that was compiled, verbatim.

**Next:** [Lesson M2: Read it back](./02-get-list-query.md)
