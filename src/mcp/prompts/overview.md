# AREST Domain Modeling & API

## First Principle: Natural-Language Verbalization

**Every reading must read like natural domain English.** Atomic-fact decomposition is a consequence of reading naturally, not a goal in itself. The canonical FORML 2 example is `Employee earns Salary in Year` â€” a ternary because that is how humans say it, and elementary because that is what the sentence actually asserts. It is not a ternary *in order to be* elementary, nor was it binarized into `Employee has Salary-Earning` + `Salary-Earning has Year` to appear more "normalized."

Two anti-patterns to avoid:

1. **Reifying lookups as synthetic-key entities when a natural ternary exists.** If you find yourself writing `Basic Standard Deduction 'Single-2024' has Amount $14,600`, stop. The natural form is `Filing Status 'Single' has Basic Standard Deduction Amount $14,600 in Tax Year '2024'` â€” a ternary. The `'Single-2024'` ID is a fiction; no statute or form names it.

2. **Binarizing a natural n-ary into a chain of binaries that loses the atomic assertion.** If the domain expert says "the plan charges this price for that product on this interval," the reading is `Plan charges Price for Product per Interval` â€” a quaternary. Splitting into `Plan charges Price`, `Plan charges for Product`, `Plan charges per Interval` loses the single-assertion meaning.

Reify as an entity only when the thing has its own identity beyond the role tuple (e.g., `Tax Bracket` is a slot in a schedule with its own id; `Standard Deduction amount` is a value looked up by `(Filing Status, Tax Year)` and has no separate identity).

### Step 1: Check existing verbalizations and business rules
Before writing any code, ask: "Is this already modeled?" Read the domain's verbalizations. The answer to the user's question may already exist as a fact type, constraint, or derivable from existing facts.

### Step 2: Design or clarify the model
If the answer isn't in the existing model, **propose verbalizations/facts/constraints** â€” not code. Say: "I think the reading is: 'Customer has Domain. Each Domain belongs to at most one Customer.'" â€” not "I'll add a `customerId` field."

Ask modeling questions, not implementation questions:
- YES: "Is this a new entity or a value type on an existing entity?"
- YES: "What's the multiplicity â€” can a Customer have multiple Domains?"
- YES: "Is 'archived' a state in a lifecycle, or a permanent classification?"
- NO: "What should the field name be?"
- NO: "Should this be middleware or a hook?"
- NO: "Should I add this column to the database?"

### Step 3: Run the CSDP loop (Halpin's Conceptual Schema Design Procedure)

CSDP is the 7-step algorithm that converts natural-language conversation with a domain expert into a conceptual schema. It is implicit in the AREST primitives (`propose`, `compile`, `verify`) but you should apply it explicitly when generating an application from scratch or when a reading fails to capture expert intent.

**1. Elicit concrete examples, verbalize as elementary facts.**
Ask the expert for sample sentences about the domain in their own words: "Customer ACME bought 3 licenses of Plan Growth on 2024-08-14." Transcribe each into elementary-fact form, splitting only where it reads naturally split. Prefer the expert's phrasing. Examples, not abstractions, come first.

**2. Draw fact types and population-check.**
For each elementary fact, identify the predicate and its roles. The fact type is the pattern; the examples populate it. Walk the draft schema past the expert with real sample data: "With this model, could you tell me which customers bought Plan Growth in August?" If the expert has to contort the data to answer, the model is wrong.

**3. Identify entity types vs value types; flag arithmetic derivations.**
Things that are referenced have entity types; things that are printed verbatim have value types. `Customer` is an entity (it has facts); `Email Address` is a value (it identifies). When the expert says "the total is the sum of the line items" or "the age follows from the birth date," mark those as derivations (`*`, `**`, or `+`) rather than storing.

**4. Add uniqueness constraints; check arity.**
For each fact type, ask: "Can the same X do this more than once?" UC on 0, 1, or span of roles. On n-ary (ternary+), the UC must span at least nâˆ’1 roles â€” if the expert says fewer roles uniquely determine the rest, the fact should be split.

**5. Add mandatory role constraints; check logical derivations.**
For each role, ask: "Must every Customer have this, or is it optional?" Then look at pairs of fact types: "When we know X, can we compute Y?" â€” candidate for a derivation.

**6. Add value, set-comparison, and subtyping constraints.**
Enum values on value types. Subset, equality, exclusion between fact types. Subtype partitions with "is exclusive" / "is complete" where they apply.

**7. Add remaining constraints and final-check.**
Ring constraints (irreflexive, acyclic, symmetric). Frequency constraints. Deontic vs alethic modality on each constraint: is this a physical impossibility (alethic) or an obligation/policy with tolerated exceptions (deontic)?

The CSDP loop is iterative: each step can surface changes to earlier steps. When the expert reviews the schema and it *reads to them like how they would say it*, the loop has converged.
