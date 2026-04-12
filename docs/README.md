# arest Developer Docs

Self-contained reference for building on arest. Does not require reading the [whitepaper](../AREST.tex).

Read in order if you are new. Jump around if you are looking something up.

1. [Introduction](01-introduction.md) — what AREST is, when to use it, when not to.
2. [Writing Readings](02-writing-readings.md) — entity types, fact types, verbs, instance facts.
3. [Constraints](03-constraints.md) — all 17 constraint kinds, alethic vs deontic, violation messages.
4. [State Machines](04-state-machines.md) — statuses, transitions, events, facts-as-events.
5. [Derivation Rules](05-derivation-rules.md) — forward chaining, join syntax, least fixed point.
6. [The Compile Pipeline](06-compile-pipeline.md) — what happens between readings and runnable state.
7. [Generators](07-generators.md) — SQL, iLayer, XSD, Verilog, Solidity; opt-in mechanism.
8. [Federation](08-federation.md) — external systems, credentials, populate functions.
9. [MCP Verbs](09-mcp-verbs.md) — the v1.0 tool surface.
10. [Self-Modification](10-self-modification.md) — `compile`, `propose`, Domain Change workflow.

For a quick start, see the [top-level README](../README.md). For the formal foundations, the [whitepaper](../AREST.pdf) has the five theorems and their proofs.
