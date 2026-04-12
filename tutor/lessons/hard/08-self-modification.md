# Lesson H8: SELF-MODIFICATION AT RUNTIME

**Goal:** Extend the running engine's own program by compiling new readings, and watch the theorems survive.
**Prereqs:** Lesson H7

`compile` is an application of `SYSTEM` whose addressed entity is `DEFS` and whose operation is `compile ∘ parse`. Parsed readings become FFP objects, stored into DEFS via `↓DEFS`. Every subsequent `SYSTEM` call evaluates the new definitions. Migration, versioning, and schema evolution are the same mechanism.

All the guarantees that hold at startup still hold after self-modification — the new D is still unambiguous, still specification-equivalent, still complete, still HATEOAS-projective, still fully derivable. Migration is ingestion.

## Do it

Add a new fact type and constraint to a running Orders app:

~~~ compile
Order has Priority.
  Each Order has at most one Priority.

It is obligatory that each high-Priority Order is shipped within 2 days.
~~~

Now assert a Priority on an existing entity:

~~~ apply
{
  "operation": "update",
  "noun": "Order",
  "id": "m1-demo",
  "fields": { "Priority": "high" }
}
~~~

And watch `actions` reflect the new deontic constraint:

~~~ actions
{ "noun": "Order", "id": "m1-demo" }
~~~

## Check

~~~ expect
query Order_has_Priority contains {"Order": "m1-demo", "Priority": "high"}
~~~

**NOTE:** For anything you'd want reviewed before landing, use `propose` (the governed workflow) instead of `compile` directly. Compile-in-place is for fast iteration; propose is for production schema changes where an audit trail on the Domain Change is load-bearing.

**Next:** You're done. Open a PR, report a bug, or start a domain of your own.
