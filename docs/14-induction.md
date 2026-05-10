# 14 · Induction

Induction is a first-class API in arest. The platform Func `induce` enumerates candidate populations for a given fact-type shape, gates them by alethic constraints, optionally requires them to forward-chain to a target set of observed facts, and emits a ranked list of Hypothesis Candidate facts. It is the inductive complement to deduction (which the forward chainer already does on every `create`).

## Why induction is a first-class API

Inductive logic is a whole branch of logic apart from deductive logic. Induction proposes candidate populations for a given outcome and calculates the most likely population. Deduction concludes from known premises; induction asks "which premises would explain the observations?" Both belong inside the algebra rather than outside it as an LLM call.

Per the whitepaper Domain Change SM section, the CSDP loop closes when the system can self-modify in response to constraint violations, error patterns, or feature requests. `induce` is the missing primitive in that loop. The system that asserts a constraint violation can also propose the candidate populations whose addition would resolve it, and route that proposal through the standard Domain Change state machine (see [10-self-modification](10-self-modification.md)).

This is also how the [Sherlock](https://github.com/graphdl/apps-sherlock) app solves mysteries, and how AREST will go up against ARC-AGI-3: enumerate candidates over the finite domain, gate by constraints, rank by user-declared scoring rules. No black-box LLM in the inner loop.

## Distinct from abduction

The same engine search supports both shapes via different `to_explain` inputs:

- **Induction (rule extraction)**: `to_explain` carries observed positive examples; candidates are general rules whose closure covers the examples.
- **Abduction (conclusion → candidate premises)**: `to_explain` carries the conclusion; candidates are missing premises whose addition derives it.

Both are routed through `induce::run_search` in `crates/arest/src/induce.rs`. The shape distinction lives in the data the caller passes, not in separate engine paths.

## The input/output shape

Invoke as a Func application:

```text
apply Func::Platform("induce") to <<ft_id, "FactType_id">,
                                   <to_explain, <fact₁, fact₂, …>>>
```

- `ft_id` names the relation to enumerate candidates over (e.g. `Coin_has_Side`, or in Sherlock, `Suspect_committed_Crime`). The engine reads the FactType + Role cells to determine the role positions and noun types.
- `to_explain` is a list of observed facts the candidate must derive after forward-chain LFP. May be empty for unconstrained enumeration (every constraint-satisfying candidate is emitted).

Output: `Seq<Object>` of Hypothesis Candidate facts per the schema in `readings/core/induction.md`. Each Hypothesis Candidate carries:

```json
{
  "id": "hyp-Coin_has_Side-0",
  "Hypothesis_Candidate_has_hidden__Fact": [
    { "Coin": "c1", "Side": "heads" }
  ],
  "Hypothesis_Candidate_explains_Fact": [
    { "Coin": "c1", "Side": "heads" }
  ]
}
```

The `id` is deterministic on `(ft_id, idx)` so re-running the same search produces stable ids — useful for de-dup across successive scoring passes. `hidden-Fact` carries the candidate's projected per-FT atom; `explains-Fact` carries one pointer per `to_explain` entry (empty when `to_explain` is empty).

The Confidence Score binding is attached separately by Scoring Rule firings (see below) so ranking is data, not engine code.

## Search semantics

`run_search` (in `crates/arest/src/induce.rs`) does three things in order, per candidate:

1. **Enumerate** every fact of shape `ft_id` over the finite domain of each role (`enumerate_candidates_for_fact_type`). For value-typed roles the domain comes from the EnumValues cell; for entity-typed roles, from the existing population. The cartesian product across roles produces the candidate list. An empty domain at any role collapses the product to empty.
2. **Gate by alethic constraints** (`candidate_passes_constraints`). Push the candidate into the per-FT cell, run the existing `validate` def, drop candidates that surface any violation. Same path the compile-rejection harness uses for alethic constraints.
3. **Require forward-chain coverage** (`candidate_derives`). Push the candidate into state, run `forward_chain_defs_state` to LFP, confirm every fact in `to_explain` appears in post-state cells. Skipped when `to_explain` is empty.

This is a ρ-application over `P` per Theorem 4 (HATEOAS as Projection): the result is derivable from the existing population plus the candidate, never invented from outside the algebra.

## How scoring rules are user-extensible

Per "readings as source code" — scoring is data, not code. Each Scoring Rule is itself a derivation rule expressed in user readings. Drop a new rule into your app's readings; it ranks candidates without engine changes.

```forml2
## Fact Types
Hypothesis Candidate has Confidence Score. +
Scoring Rule applies to Hypothesis Candidate.

## Derivation Rules
+ Hypothesis Candidate has Confidence Score 'High' if
    Hypothesis Candidate explains some Fact and
    that Fact has Evidence Weight 'Strong'.

+ Hypothesis Candidate has Confidence Score 'Low' if
    Hypothesis Candidate explains some Fact and
    that Fact has Evidence Weight 'Weak'.
```

The engine's `forward_chain_defs_state` evaluates these on the post-search state; rules emit `Hypothesis Candidate has Confidence Score` facts. Callers sort the search result by Confidence Score before presentation. Adding a new heuristic is an `apply propose` against the schema, not a Rust patch.

## Sherlock walk-through (sketch)

The Sherlock app at `C:\Users\lippe\Repos\apps\sherlock` solves mysteries by abduction over a Crime + Evidence schema. The fixture is still pending (#853), so this is sketched against the substrate's intent.

Schema (from `apps/sherlock/readings/{crime,evidence,reasoning}.md`):

```forml2
Suspect(.Name) is an entity type.
Victim(.Name) is an entity type.
Crime(.id) is an entity type.
Suspect committed Crime.
Suspect has Motive against Victim.
Suspect has Opportunity at Location.
```

Observations seed the population: `<v1, found at l1>`, `<s1, has Motive against v1>`, `<s2, has Opportunity at l1>`. The detective wants to know who committed the crime.

Call:

```json
{
  "ft_id": "Suspect_committed_Crime",
  "to_explain": [{ "Crime": "c1", "Victim": "v1" }]
}
```

`run_search` enumerates `{s1, s2} × {c1}` = 2 candidates. Each candidate is gated by alethic constraints (one Crime per Suspect). Each is then required to forward-chain to a `Crime explains Victim` fact via the existing derivation rules in `evidence.md` (Motive + Opportunity → committed). Only `s1` survives both gates. A user-written Scoring Rule (`Suspicion Score derives from prior Association count`) then ranks the surviving candidate.

The same engine call works whether you are testing one suspect or fifty — the cartesian product enumeration is what makes induction first-class instead of bespoke per-domain code.

## Whitepaper alignment

- §3 (Algebra of Definitions): `induce` is a Func like any other. The platform name lives in `apply_platform`'s dispatch (`crates/arest/src/ast.rs`).
- §5.2 (Domain Change SM): induction closes the CSDP loop. A constraint violation can propose a Domain Change whose body is the highest-Confidence-Score candidate.
- Theorem 4 (HATEOAS as Projection): the result is a function of `P` plus the candidate, never invented.
- Theorem 5 (Closure Under Self-Modification): a Hypothesis Candidate that becomes an applied Domain Change is itself a fact in `P`, audit-trail intact.

## What's next

State machines describe change over time. Derivation rules describe what else follows from the current facts. Induction proposes which facts could have been there to begin with. With these three plus self-modification, the system is closed under its own evolution — no external code path is required to keep up with the schema.
