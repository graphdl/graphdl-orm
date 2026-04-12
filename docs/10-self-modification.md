# 10 · Self-Modification

One of the load-bearing claims of AREST is that the system can change its own schema without leaving the algebra. There are two paths: immediate self-modification via `compile`, and governed self-modification via `propose`. Both preserve all five theorems (Corollary 5: Closure Under Self-Modification).

## The `compile` path

`compile` takes reading text, parses it, compiles it, and merges the resulting defs into `DEFS` via the store operator `↓DEFS`. Subsequent `SYSTEM` applications evaluate the new definitions as if they had been there from the start.

```bash
# CLI
arest-cli readings/ --db app.db        # initial compile
arest-cli readings/v2/ --db app.db     # re-compile with new readings

# MCP
compile({ readings: "New Fact Type: User has Loyalty Tier.\n  Each User has at most one Loyalty Tier." })
```

`compile` is the raw mechanism. Use it when:

- You are bootstrapping.
- You are running a migration.
- You control the system and there is no review process.

Because `compile` reaches `DEFS` directly, it bypasses any approval gate. Anyone with the ability to call `compile` can change your schema. In multi-tenant deployments, gate `compile` behind an admin role.

## The `propose` path

`propose` creates a Domain Change entity with the proposed readings, nouns, or constraints, then enters the state machine declared in the bundled `evolution.md`:

```
Proposed → Under Review → Approved → Applied
         ↘ Rejected
         ↘ (back to Proposed via Revise)
```

The Domain Change entity carries:

- A rationale (why the change is needed)
- A target domain (which slug the change affects)
- A set of proposed elements (readings, nouns, constraints, verbs)
- A state machine position

```typescript
propose({
  rationale: "Add loyalty tier tracking",
  target_domain: "orders",
  readings: ["Customer has Loyalty Tier.\n  Each Customer has at most one Loyalty Tier."],
  nouns: ["Loyalty Tier"]
})
// → { change_id: "dc-lx9f4", status: "Proposed", next_actions: [...] }
```

Then transitions:

```typescript
transition({ noun: "Domain Change", id: "dc-lx9f4", event: "review" })          // → Under Review
transition({ noun: "Domain Change", id: "dc-lx9f4", event: "approve-change" })  // → Approved
transition({ noun: "Domain Change", id: "dc-lx9f4", event: "apply" })           // → Applied
```

On `apply`, the engine runs `compile` on the proposed readings, which means the change actually takes effect only when the state machine reaches `Applied`. Between Proposed and Applied, the proposed elements exist only as data on the Domain Change entity — they are not yet in `DEFS`.

## Human gates

The metamodel declares deontic constraints that require human review for certain domains:

```forml2
It is forbidden that a Domain Change targeting Domain 'evolution' is applied without Signal Source 'Human'.
It is forbidden that a Domain Change targeting Domain 'organizations' is applied without Signal Source 'Human'.
It is forbidden that a Domain Change targeting Domain 'core' is applied without Signal Source 'Human'.
```

Changes to the metamodel itself (`core`, `evolution`, `organizations`) cannot be applied unless a Human signal source is attached. An autonomous agent proposing a metamodel change gets a deontic violation and the change stays in Approved state — waiting for human sign-off.

You can extend this pattern to your own domains:

```forml2
It is forbidden that a Domain Change targeting Domain 'billing' is applied without Signal Source 'Human'.
```

## Signals

A Domain Change is typically triggered by a Signal:

```forml2
Signal(.Signal Id) is an entity type.

Signal has Signal Source.
Signal leads to Domain Change.
```

Signal sources: `Constraint Violation`, `Human`, `Error Pattern`, `Feature Request`, `Support Request`. The LLM validate verb (see [MCP verbs](09-mcp-verbs.md)) naturally produces Constraint Violation signals; a customer support integration might produce Support Request signals.

## Preserving the theorems

Corollary: Closure says all five theorems hold after self-modification:

1. **Grammar Unambiguity** depends on the grammar itself, not on `D`. Unaffected.
2. **Specification Equivalence** depends on `parse` and `compile` being injective stateless functions. Unaffected.
3. **Completeness of State Transfer** operates over `P` and `S`, both of which now include the new content. Still holds.
4. **HATEOAS as Projection** operates over `P` and `S`. Still holds.
5. **Derivability** — every value in the representation is a ρ-application, regardless of when the definitions entered `DEFS`.

The Curry-Howard correspondence applies: proposing a new fact type is proposing a theorem. CSDP validation is the proof check. Successful ingestion is the proof. The system can only evolve by proving something new.

## Auditability

Every Domain Change is itself an entity in `P`. Every status transition is a state-machine event in the audit log. Every applied change leaves a trail:

- Who proposed it (via the `created by User` fact)
- When (via the `occurred at Timestamp` fact)
- What rationale
- What was proposed (as facts attached to the Domain Change)
- Who approved it (via the Human signal)
- When it was applied

The same derivation chain that shows why a User accesses a Domain also shows how the schema that made that access possible came to exist. The system is self-documenting by construction.

## What's next

This is the last doc. You have:

- Read an introduction to the project's principles ([01](01-introduction.md))
- Learned the reading grammar ([02](02-writing-readings.md))
- Covered all 17 constraint kinds ([03](03-constraints.md))
- Understood state machines and facts-as-events ([04](04-state-machines.md))
- Written derivation rules with joins and LFP ([05](05-derivation-rules.md))
- Walked through the compile pipeline ([06](06-compile-pipeline.md))
- Opted into generators for SQL, Solidity, and more ([07](07-generators.md))
- Federated data from external systems ([08](08-federation.md))
- Learned the frozen MCP verb set ([09](09-mcp-verbs.md))
- Evolved the system without leaving the algebra ([10](10-self-modification.md))

The next logical step is to build something real. The [arest-tutor](https://github.com/graphdl/arest-tutor) sample app (in progress) will exercise every feature end to end. If you run into something these docs do not answer, open an issue — the docs are meant to be self-contained.
