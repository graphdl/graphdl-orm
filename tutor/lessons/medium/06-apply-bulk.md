# Lesson M6: APPLY A COLLECTION OF OPS, ATOMICALLY

**Goal:** Create two Orders and place one of them in a SINGLE `apply` call.
**Prereqs:** Lesson M4

So far every `apply` carried one op. But `apply` is really Backus **α (apply-to-all)** over a sequence: pass `ops` — an array of `{operation, noun, id, fields?, event?}` — and the engine applies the whole COLLECTION as ONE request. It resolves every op over a shared, cumulatively-built population, derives to the least fixed point ONCE over the combined state, validates, and emits a single delta. A lone op is just the 1-element collection, so this is the natural shape, not a special mode.

The batch is **atomic** (AREST.tex, "Completeness of State Transfer"). An **alethic** violation in ANY op rejects the WHOLE batch — `D' = D`, so nothing lands, not even ops that ran before the offending one. A **deontic** finding warns but the batch still commits. That is why a bulk seed-and-transition is safe: either the entire collection takes effect, or none of it does.

## Do it

~~~ apply
{
  "ops": [
    { "operation": "create", "noun": "Order", "id": "m6-a", "fields": { "Customer": "globex", "Amount": "100" } },
    { "operation": "create", "noun": "Order", "id": "m6-b", "fields": { "Customer": "initech", "Amount": "200" } },
    { "operation": "transition", "noun": "Order", "id": "m6-a", "event": "place" }
  ]
}
~~~

## Check

~~~ expect
status Order m6-a is Placed
~~~

## Why

`apply([op1, op2, …])` is α(ρ-dispatch) over the collection: one derive→validate→emit pass to the least fixed point over the combined population, NOT N independent applies. The single atomicity boundary is what gives you all-or-nothing rollback.

**NOTE:** Try it the wrong way to see the rollback: add a fourth op `{ "operation": "create", "noun": "Order", "id": "m6-a", "fields": { "Amount": "9" } }` — a duplicate id. The reference-scheme uniqueness constraint is alethic, so the engine rejects the entire batch and NONE of m6-a, m6-b, or the transition persists.

**Next:** [Lesson H1: Declare a noun](../hard/01-noun.md)
