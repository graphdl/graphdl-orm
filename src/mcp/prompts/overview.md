# GraphDL Domain Modeling & API

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
