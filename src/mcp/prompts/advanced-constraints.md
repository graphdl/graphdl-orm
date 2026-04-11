
### Subtype Constraints

Subtypes partition or overlap a supertype's population. Three constraint types:

- **Totality**: every instance of the supertype must belong to at least one subtype. "Each Person is a Male or a Female."
- **Exclusion**: subtypes are mutually exclusive â€” no instance belongs to more than one. "No Person is both a Male and a Female."
- **Partition (exclusive-or)**: totality + exclusion combined â€” every supertype instance belongs to exactly one subtype. Express in FORML2:

```
Person is partitioned into Male, Female.
```
Or equivalently as separate constraints:
```
Male is a subtype of Person.
Female is a subtype of Person.
  Each Person is a Male or a Female.
  No Person is both a Male and a Female.
```

**Common mistake**: declaring subtypes without specifying whether they are total, exclusive, or both. Always state the constraint explicitly.

### Subset Constraints

Subset constraints are different from subtype constraints. Subtype constraints partition or overlap an **entity type's population**. Subset constraints constrain **role populations across different fact types**: every combination of objects playing certain roles in one fact type (the subset) must also appear playing certain roles in other fact types (the superset).

**Verbalization pattern:**

The general form uses "If some... who/that... then... where...":

```
If some Message matches some Sales Rep who is involved in some Sales Message Match
that is by some Phone Number then that Message is with that internal Phone Number
where that Sales Rep has that Phone Number.
```

The structure breaks down as:
- **If some** [objects exist playing roles in the superset fact types]
- **then** [the subset fact type must be populated with those objects]
- **where** [join condition linking roles across the superset fact types]

**Autofill semantics:** Superset populates subset, never the reverse. When the superset roles are all populated (a Sales Rep has a Phone Number AND a Message matches that Sales Rep), the subset fact (Sales Message Match is by that Phone Number) is derived automatically. The direction is always general to specific.

**Example: typed API parameters**

Superset fact types:
- `API Endpoint accepts Noun as parameter.` (general acceptance)
- `VIN is a subtype of Noun.` (type hierarchy)

Subset fact type:
- `API Endpoint accepts VIN as parameter.` (specific acceptance)

The subset constraint says: if an API Endpoint accepts a Noun as parameter, and that Noun is a VIN, then the API Endpoint accepts VIN as parameter. The general parameter acceptance (superset) drives the specific typed parameter (subset). This is how endpoint path templates like `/{vin}` and `/{zip}` get resolved from the general "accepts Noun" facts.

**FORML2 section placement:** Subset constraints go in the `## Constraints` section, after uniqueness and mandatory constraints.

**Common mistakes:**

| Mistake | Fix |
|---------|-----|
| Confusing subset with subtype constraints | Subtypes partition entity populations. Subsets constrain role populations across fact types. They are orthogonal concepts. |
| Reversing the autofill direction (reading from subset, deriving into superset) | Superset is the source. Subset is derived. Read from the general facts, populate the specific. |
| Modeling the specific fact type independently instead of deriving it | If "API accepts VIN as parameter" can be derived from "API accepts Noun" + "VIN is a subtype of Noun", don't assert it as an independent base fact. |
| Omitting the join condition ("where" clause) | The join condition links the roles across superset fact types. Without it, the constraint is ambiguous about which objects correspond. |

**Named Constraint Spans with Autofill:**

When multiple subset constraints target the same roles and those roles should be auto-populated from the superset, name the constraint span using the objectification naming pattern, then declare autofill as a unary instance fact on the named span.

```
## Subset Constraints

If some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.
If some Support Request has some contact- Name and some Customer has that Name then that Customer submits that Support Request.
If some Support Request has some Email Address and some Account has that Email Address and that Account is for some Customer then that Customer submits that Support Request.

This span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.
Constraint Span 'Customer Submission Match' autofills from superset.
```

The pattern has three parts:
1. **Subset constraint verbalizations** â€” standard `if...then` pattern with "some" (existential) in the superset and "that" (back-reference) in the subset. Each line is independently parseable.
2. **Span naming** â€” `This span with X, Y provides the preferred identification scheme for Z.` Names the span (the set of roles the constraint covers) using the objectification naming pattern. "This" backreferences the subset constraints above. A span is the entire multi-role coverage of a constraint, not individual role pairs.
3. **Autofill declaration** â€” `Constraint Span 'Z' autofills from superset.` A unary instance fact on the named span. When superset roles are populated, the subset roles are auto-populated.

The metamodel defines `Constraint Span autofills from superset` as a unary fact type on the `Constraint Span` entity (objectification of "Constraint spans Role"). A span covers all the roles a constraint spans â€” it exists because a constraint covers multiple roles.

### Ring Constraints

Ring constraints apply to binary facts where both roles are played by the same object type (or a type and its subtype). They constrain the "shape" of the relationship.

Ring constraints verbalize using conditional "if {0} then {1}" structure with type-specific patterns.

| Constraint | Meaning | Verbalization |
|-----------|---------|---------|
| **Irreflexive** | No instance relates to itself | **No** Aâ‚ R **itself.** |
| **Asymmetric** | If Aâ†’B then not Bâ†’A | **If** Aâ‚ R Aâ‚‚ **then it is impossible that** Aâ‚‚ R Aâ‚. |
| **Intransitive** | If Aâ†’B and Bâ†’C then not Aâ†’C | **If** Aâ‚ R Aâ‚‚ **and** Aâ‚‚ R Aâ‚ƒ **then it is impossible that** Aâ‚ R Aâ‚ƒ. |
| **Acyclic** | No cycles of any length | **No** Aâ‚ **may cycle back to itself via one or more traversals through** R. |
| **Symmetric** | If Aâ†’B then Bâ†’A | **If** Aâ‚ R Aâ‚‚ **then** Aâ‚‚ R Aâ‚. |
| **Antisymmetric** | If Aâ†’B and Aâ‰ B then not Bâ†’A | **If** Aâ‚ R Aâ‚‚ **and** Aâ‚ **is not** Aâ‚‚ **then it is impossible that** Aâ‚‚ R Aâ‚. |
| **Transitive** | If Aâ†’B and Bâ†’C then Aâ†’C | **If** Aâ‚ R Aâ‚‚ **and** Aâ‚‚ R Aâ‚ƒ **then** Aâ‚ R Aâ‚ƒ. |
| **Reflexive** | If A relates to any B then A relates to itself | **If** Aâ‚ R **some** Aâ‚‚ **then** Aâ‚ R **itself.** |
| **Purely Reflexive** | A only relates to itself | Aâ‚ R Aâ‚‚ **where** Aâ‚ **=** Aâ‚‚. |

Compound ring constraints are expressed by conjoining the individual verbalizations.

(Halpin, ORM2, Fig. 9, p. 8)

