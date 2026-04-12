# Lesson E1: DESCRIBE YOUR APP

**Goal:** Turn a one-line English description into a running app you can query.
**Prereqs:** none

You don't model your app. You describe it. The engine does the modeling, and the description you give becomes the app's source of truth.

In AREST, `propose` takes an English rationale plus whatever nouns and readings come to mind, creates a Domain Change, and hands you the next steps. Once you `apply` the change, the schema is live. No migration, no codegen step — you go from "I want to track orders with customers and totals" to a working CRUD app in one round trip.

## Do it

Describe an app in a single sentence. The tutor will fill in a minimal `propose` call:

~~~ propose
{
  "rationale": "I want to track orders with customers and totals.",
  "target_domain": "orders",
  "nouns": ["Customer", "Order"],
  "readings": [
    "Customer(.Name) is an entity type.",
    "Order(.id) is an entity type.",
    "Order was placed by Customer. Each Order was placed by exactly one Customer.",
    "Order has Amount. Each Order has exactly one Amount."
  ]
}
~~~

Then approve and apply it (one tool call with the returned `change_id`).

## Check

~~~ expect
list Noun contains {"id": "Order"}
~~~

**NOTE:** Propose/apply is the governed path for schema changes. For quick experiments you can call `compile` directly — we'll do that in Lesson H8.

**Next:** [Lesson E2: Ask a question](./02-ask.md)
