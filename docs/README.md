# arest Developer Docs

These pages form a self-contained reference for building on arest, and they do not require reading the [whitepaper](https://github.com/graphdl/arest/blob/main/AREST.pdf).

Read them in order if you are new. If you are looking up a particular topic, jump to the relevant chapter.

1. [Introduction](01-introduction.md): what AREST is, when you should reach for it, and when you should not.
2. [Writing Readings](02-writing-readings.md): entity types, fact types, verbs, and instance facts.
3. [Constraints](03-constraints.md): all 17 constraint kinds, the alethic-vs-deontic split, and violation messages.
4. [State Machines](04-state-machines.md): statuses, transitions, events, and facts-as-events.
5. [Derivation Rules](05-derivation-rules.md): forward chaining, join syntax, and the least fixed point.
6. [The Compile Pipeline](06-compile-pipeline.md): what happens between readings and runnable state.
7. [Generators](07-generators.md): SQL, iLayer, XSD, Verilog, and Solidity, plus the opt-in mechanism.
8. [Federation](08-federation.md): external systems, credentials, and populate functions.
9. [MCP Verbs](09-mcp-verbs.md): the v1.0 tool surface.
10. [Self-Modification](10-self-modification.md): `compile`, `propose`, and the Domain Change workflow.

For a quick start, see the [top-level README](https://github.com/graphdl/arest#readme). For the formal foundations, the [whitepaper](https://github.com/graphdl/arest/blob/main/AREST.pdf) presents the five theorems and their proofs.
