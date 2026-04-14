**When to check**: any binary fact where subject and object have the same type (e.g., "Employee manages Employee", "Category contains Category", "Task depends on Task").

### Derivation Rules

A derived fact is one whose population is computed from other facts rather than directly asserted. In FORML2:

Every derived fact type carries a **Derivation Mode** marker on the reading plus a matching marker as a prefix on the rule body. Halpin ORM 2 (ORM2.pdf p. 8):

| Marker | Mode | Semantics |
|---|---|---|
| `*` | fully derived | Never asserted; always computed from the rule. |
| `**` | derived and stored | Same as `*` but materialized (SQL trigger, etc.). |
| `+` | semi-derived | May be computed from the rule OR asserted directly. |

The rule body uses `iff` for full derivation (rule IS the definition) or `if` for partial (rule is one sufficient condition; others may also populate).

```
## Fact Types
Person has Full Name *.

## Derivation Rules
* Person has Full Name iff Person has First Name and Person has Last Name and Full Name is First Name concatenated with Last Name.
```

The `:=` form from pre-ORM 1 BNF grammar is retired.

Derivation rules belong in the `## Derivation Rules` section of a FORML2 document. A derived fact should never be stored as an independent field â€” it is a query over the base facts.

**Common mistake**: adding a stored field for something that is derivable from existing facts or state machine history (e.g., adding `isApproved: boolean` when approval state is already in the lifecycle).

### Occurrence Frequency Constraints

Constrain how many times an object can play a role:

```
Each Customer submits at least 1 and at most 5 SupportRequest.
```

This is different from a UC â€” a UC says each instance in the role is unique. A frequency constraint says each instance of the object type appears a specific number of times in the fact population. Frequency 1 on a role is equivalent to a UC + mandatory constraint on that role.

## Deontic vs Alethic Modality (ORM2)

ORM2 distinguishes two modalities for constraints:

- **Alethic** (default) â€” impossible to violate; enforced by the schema itself. "Each Customer has at most one Name." â€” the database physically prevents a second name.
- **Deontic** â€” possible to violate but shouldn't be; enforced by policy, not schema. "It is obligatory that each SupportResponse conforms to PricingModel." â€” violations can occur and must be detected.

### Deontic Operators

| Operator | Meaning | Pattern |
|----------|---------|---------|
| **It is obligatory that** | Must happen (positive duty) | `It is obligatory that each X has some Y.` |
| **It is forbidden that** | Must not happen (prohibition) | `It is forbidden that SupportResponse contains ProhibitedPunctuation.` |
| **It is permitted that** | Allowed but not required (permission) | `It is permitted that SupportResponse offers data retrieval assistance to Customer.` |

### Examples

```
# Alethic (schema-enforced, no prefix needed)
Each Customer has at most one Name.
Each Listing has at most one Mileage.

# Deontic obligation (policy-enforced)
It is obligatory that each SupportResponse is delivered via ChannelName 'Email'.
It is obligatory that RevenueStream total exceeds CostCenter total per Frequency 'Monthly'.

# Deontic prohibition (policy-enforced)
It is forbidden that SupportResponse contains ProhibitedPunctuation.
It is forbidden that CustomerAcquisitionCost exceeds CustomerLifetimeValue.

# Deontic permission (explicitly allowed)
It is permitted that SupportResponse offers data retrieval assistance to Customer.
```

### Common Deontic Mistakes

| Mistake | Fix |
|---------|-----|
| `Support Response must not contain X` | `It is forbidden that Support Response contains X.` â€” use standard operator |
| `Revenue should exceed costs` | `It is obligatory that Revenue Stream total exceeds Cost Center total per Frequency.` â€” "should" is not a FORML2 operator |
| `Runway must be at least 6 months` | `It is forbidden that Runway is less than Runway Months 6.` â€” express as prohibition with instance value |
| Deontic constraint on a fact that could be alethic | If the schema CAN enforce it (e.g., uniqueness), make it alethic. Deontic is for constraints the schema cannot express. |
| `It is obligatory that...` without declaring all nouns | Every noun in a deontic constraint must be declared as an entity type or value type. Undeclared nouns are the #1 audit finding. |

