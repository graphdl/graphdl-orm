# AREST UI: Render Subscription — standing ρ-applications (whitepaper §5.2)

The LIVE half of §5.2 Platform Binding (pb-live-binding-reeval). A Render
Subscription is a STANDING render: a client that wants an entity's view
not just now but whenever it changes. Per the whitepaper, a subscriber
is "a ρ-application not yet evaluated" — this reading reifies exactly
that as facts. A subscription names WHAT to watch (a Noun, optionally
one entity instance), HOW to render it (a Render Target from
`readings/ui/render-target.md`), and WHERE to deliver (a callback URI,
reusing the `callback URI` value type the task-919 Function dispatch
declared). The engine's apply path detects when a mutation's delta
touches cells a subscribed view read, re-runs the render function, and
fires the delivery as an EFFECT (the `http_fetch` / `notify` Platform
bodies, `platform/{http_fetch,notify}.rs`). No pub/sub machinery: the
subscription fact plus an effect application on dirtiness IS the live
binding.

Population is runtime-shaped: subscriptions are created and deleted by
`apply`, not authored here — this reading declares only the model.

## Entity Types

Render Subscription(.Name) is an entity type.
  <!-- One standing render. The .Name reference mode is a
       subscriber-chosen slug ('repl-task-pane', 'dashboard-42');
       uniqueness is per the reference scheme. -->

## Value Types

Entity Id is a value type.
  <!-- The subscribed entity instance's id, e.g. a Task id. Absent ⇒
       the subscription watches the NOUN's whole population (the
       collection view) — a later slice; the first slice serves
       instance subscriptions. -->

## Fact Types

### Render Subscription

Render Subscription is for Noun.
  Each Render Subscription is for exactly one Noun.

Render Subscription watches Entity Id.
  Each Render Subscription watches at most one Entity Id.

Render Subscription renders via Render Target.
  Each Render Subscription renders via exactly one Render Target.

Render Subscription delivers to callback URI.
  Each Render Subscription delivers to at most one callback URI.
  <!-- Reuses the task-919 `callback URI` value type (core.md's
       Function dispatch). Absent ⇒ delivery falls back to the
       `notify` effect (stderr line on the CLI host; a kernel surface
       or worker push installs its own notify body) — useful for
       diagnostics and the smoke tests. -->

## Constraints

No two Render Subscriptions share the same Name.

## Deontic Constraints

It is obligatory that each Render Subscription is for some Noun.
It is obligatory that each Render Subscription renders via some Render Target.
