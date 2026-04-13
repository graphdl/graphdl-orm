# 01 · Introduction

arest turns plain-English business rules into running applications. You write sentences like

```
Order was placed by Customer.
  Each Order was placed by exactly one Customer.
```

and the compiler produces a database schema, a foreign key, a uniqueness constraint, a state machine, and a REST endpoint. There is no separate ORM definition, no handler boilerplate, and no translation layer.

The approach builds on four decades of published work:

- **[Backus 1978](https://dl.acm.org/doi/10.1145/359576.359579):** functional programming as an algebra, with named definitions resolved by a representation function ρ.
- **[Codd 1970](https://dl.acm.org/doi/10.1145/362384.362685):** the relational model and its algebra θ₁ (projection, join, restriction, tie).
- **[Halpin 2008](https://www.orm.net/pdf/IMRD2EPreface.pdf):** FORML 2 and the RMAP procedure that produces 3NF tables from fact-oriented models.
- **[Fielding 2000](https://www.ics.uci.edu/~fielding/pubs/dissertation/top.htm):** REST and HATEOAS as navigation over a resource graph.

AREST is the composition of these four. A FORML 2 reading is simultaneously a relation schema, a constraint specification, a REST resource, and an FFP object. One sentence occupies all four roles at once. The engine recognizes that identity rather than translating between representations.

## When to use it

arest is a fit under these conditions:

- **Your domain is fact-oriented.** You can describe what you want in sentences like "Each Order was placed by exactly one Customer" rather than pseudo-code.
- **You want the spec to be the implementation.** Business analysts write readings, and developers do not translate those readings into models and controllers.
- **You need the same logic in several runtimes.** The same readings produce SQL, on-chain Solidity, or FPGA gates without rewrites.
- **You need automated agents to be able to modify the system safely.** Every value returned by the API is a ρ-application over facts, so an LLM consumer cannot hallucinate a value that does not exist.
- **You need a compliance or audit story.** Derivations are traced, constraint violations are reported as the original sentence, and every state change is an event in an append-only log.

## When not to use it

It is not a fit under these conditions:

- **Your core logic is arithmetic or optimization.** Statistical scoring, ML inference, and numerical simulation are opaque to ρ. For those, call a Platform function whose implementation the runtime resolves by name.
- **You need to evolve aggressively without review.** `compile` performs immediate self-modification, whereas `propose` is governed. Teams that want neither workflow should not use this system.
- **Your performance budget is measured in microseconds end-to-end.** The compile step runs at roughly 50 ms for 100 fact types. Per-command `create` runs in sub-millisecond time, but adding arest to a hot path where every microsecond counts is the wrong trade.

## What you will learn in these docs

1. This file explains why the project exists and when to pick it up.
2. [Writing readings](02-writing-readings.md): entity types, fact types, verbs, and instance facts.
3. [Constraints](03-constraints.md): all 17 constraint kinds, the alethic-vs-deontic split, and violation messages.
4. [State machines](04-state-machines.md): statuses, transitions, events, and facts-as-events.
5. [Derivation rules](05-derivation-rules.md): forward chaining, join syntax, and the least fixed point.
6. [Compile pipeline](06-compile-pipeline.md): what happens between readings and runnable state.
7. [Generators](07-generators.md): SQL, iLayer, XSD, Verilog, and Solidity, plus the opt-in mechanism.
8. [Federation](08-federation.md): external systems, credentials, and populate functions.
9. [MCP verbs](09-mcp-verbs.md): the full v1.0 tool surface for agents.
10. [Self-modification](10-self-modification.md): `compile`, `propose`, and the Domain Change workflow.

## Conventions

Code blocks labelled `forml2` contain readings you could save to `readings/*.md` and then compile. Code blocks labelled `bash`, `json`, or a language name show exactly what you would run or write in that language.

Each doc ends with a "What's next" section that links to the logical next step.
