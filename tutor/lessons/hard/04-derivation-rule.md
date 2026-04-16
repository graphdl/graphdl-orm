# Lesson H4: A DERIVATION RULE

**Goal:** Declare a fact that's computed from other facts, mark its derivation mode, and watch the least fixed point do its job.
**Prereqs:** Lesson H3

Some facts are not asserted; they are derived. A "premium customer" is not a thing you type. It is the output of a rule over orders and thresholds. Derivation rules are forward-chained to the LFP on every `apply`. You never call them yourself; the engine does, every time P changes.

Every derived fact type carries a **mode marker**. Halpin ORM 2 (ORM2.pdf p. 8) defines three:

| Marker | Mode | Semantics |
|---|---|---|
| `*` | fully derived | Always computed; asserting it directly is a violation. |
| `**` | derived and stored | Same as `*`, materialized for performance (SQL trigger, etc.). |
| `+` | semi-derived | May be computed OR asserted directly — useful when the rule is one sufficient path, not the only one. |

The marker is a whitespace-separated token: suffixed to the reading in `## Fact Types` and prefixed to the body in `## Derivation Rules`. The body uses `iff` for full (rule IS the definition) or `if` for partial (one sufficient condition of several).

Rules are monotonic, so the fixed point exists and is unique (Theorem 3: Completeness). Reach is bounded by the size of the population. The derivation chain is recorded, so `explain` shows which rules fired in which order.

## Do it

Declare `Customer is premium` as fully derived — a customer IS premium exactly when the rule holds, never otherwise:

~~~ compile
Customer is premium. *

## Derivation Rules
* Customer is premium iff Customer placed at least 3 Orders and each Order has Amount greater than 100.
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

**When to use each mode:**

- `*` (fully derived) — the rule is the complete definition. No manual override possible. Use for computed attributes like `Fact Type has Arity. *` or `User accesses Domain. *` when every access path is captured in the rules.
- `**` (derived and stored) — fully derived AND materialized. Use when the derivation is expensive and a SQL trigger can keep a column in sync.
- `+` (semi-derived) — the rule is a sufficient condition but not necessary. Use when a business override is allowed: `Customer is VIP. +` with `+ Customer is VIP if lifetime orders > 100.` leaves room for sales to also mark small customers as VIPs by hand.

The `:=` form from pre-ORM 1 BNF grammar is retired. The parser still tolerates it during migration, but new rules should use the marker + iff/if form.

**Next:** [Lesson H5: A deontic constraint](./05-deontic-constraint.md)
