## Data Modeling & Relational Theory

### Entity vs Value Types

- **Entity**: a real-world object identified by a reference scheme (Customer identified by EmailAddress, Order identified by OrderNumber)
- **Value**: a primitive identified by its literal constant (a string, a number, a date)
- Entities become tables/collections. Values become columns/fields on entities.
- If you're generating a standalone table for a value type (e.g., a `descriptions` collection), you've confused an entity with a value.

### Type vs Instance (Meta-Levels)

A **type** (definition, class, schema) describes the structure. An **instance** (object, record, document) is a concrete occurrence of that type.

- **Definition** = the type (e.g., a workflow template with states, transitions, guards)
- **Instance** = a concrete occurrence (e.g., a specific order currently in status "Shipped")
- A foreign key from an instance should point to the definition table, not back to the instance table

**Common bug:** A `relationTo` or foreign key that points to the instance collection instead of the definition collection (or vice versa). The field description says "is instance of Definition" but the code references the instance table â€” a self-referential relationship that makes no semantic sense.

**How to catch it:** When a relationship field's description mentions a different entity than its `relationTo` target, the wiring is wrong. Read the description as a fact: "Order follows Workflow Definition" â€” the object of that fact (`WorkflowDefinition`) must be the target collection, not the `orders` table.

### Reference Schemes

- Every entity must have one: the value(s) that uniquely identify it
- Simple: `Customer(EmailAddress)` â€” one value
- Compound: `Account(Customer + OAuthProvider)` â€” multiple values
- Reference scheme facts are **implicit** â€” don't write "Customer has EmailAddress" as a separate reading if EmailAddress IS the reference scheme. That's redundant.

### Normalization (Quick Rules)

- **1NF**: No repeating groups. Each cell holds one value.
- **2NF**: Every non-key attribute depends on the WHOLE key, not part of it.
- **3NF**: No transitive dependencies. If A â†’ B â†’ C, don't store C on A's table.
- **BCNF**: Every determinant is a candidate key. (Catches the edge cases 3NF misses.)

### Elementary Facts & Arity

A fact is **elementary** if it can't be decomposed into simpler facts.

- **Unary**: 1 role â€” "Customer is active"
- **Binary**: 2 roles â€” "Customer submits SupportRequest" (most common)
- **Ternary**: 3 roles â€” "Plan charges PricePerCall for APIProduct"
- **And-test**: if a reading uses "and" to conjoin two independent assertions (e.g., "Customer has Name and submits Request"), it encodes two facts and must be split. "And" joining roles in a single predicate (e.g., "Plan charges Price for Product") is fine.
- **Arity check**: a uniqueness constraint on an n-ary fact type must span at least nâˆ’1 roles. If it spans fewer, the fact MUST be split into smaller facts. No elementary ternary can have a simple (single-role) UC.

**UCs ARE multiplicity** â€” they are the same thing expressed differently. A fact type has a SET of UCs, not one "relationship type".

**Inline vs section placement**: When writing FORML2 documents, constraints go in the `## Constraints` section (or `## Mandatory Constraints`, `## Deontic Constraints`). The indented-beneath-reading style shown below is for quick illustration and discussion â€” it shows which reading a constraint belongs to. In a formal document, the reading goes under `## Fact Types` and the constraint goes under `## Constraints`.

Express multiplicity as FORML2 natural language constraints:

```
Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.
```
This means UC on SupportRequest (many requests per customer, each request has at most one customer).

```
Customer has APIKey.
  Each Customer has at most one APIKey.
  Each APIKey belongs to at most one Customer.
```
Two separate UCs, one per role â€” each uniquely identifies the other.

```
SupportRequest concerns APIProduct.
  Each SupportRequest, APIProduct combination occurs at most once in the population of SupportRequest concerns APIProduct.
```
Spanning UC on a binary â€” this is the m:n default. Every elementary binary has an implied spanning UC, so stating it explicitly is optional but legitimate (generators emit these for round-trip fidelity). The arity decomposition rule applies to **ternary and higher**: if a UC on an n-ary fact spans fewer than nâˆ’1 roles, the fact must be split.

Add **mandatory constraints** with "exactly one" (= at most one + at least one) or "at least one":
```
Organization has Name.
  Each Organization has exactly one Name.

Domain belongs to Organization.
  Each Domain belongs to at most one Organization.
```

**Ternary+ constraints**: Do not use compact n:n notation for ternaries. Express which role combination has the UC using "For each ... and ...":
```
Plan has Price per Interval.
  For each Plan and Interval that Plan has that Interval at most one Price.
```
This means UC(Plan, Interval) â€” each plan-interval pair determines one price. (Halpin, TechReport ORM2-02, p. 10)

### Multiplicity (Uniqueness Constraints on Binary Facts)

Express multiplicity as natural language FORML2 constraints beneath each reading. Never use compact notation like `*:1` or `1:*` â€” always use full sentences.

| Pattern | FORML2 Constraint | Meaning |
|---------|------------------|---------|
| Many-to-one | "Each SupportRequest has at most one Priority." | UC on the "many" side â€” many requests share one priority |
| One-to-many | Same mechanism, just read from the other direction. "Each Priority applies to at most one SupportRequest" would make it 1:n. The UC always goes on the side that IS unique. | UC on the "one" side |
| One-to-one | "Each Customer has at most one APIKey. Each APIKey belongs to at most one Customer." | Two separate UCs â€” each uniquely identifies the other |
| Many-to-many | "Each X, Y combination occurs at most once in the population of X does Y." The spanning UC â€” this is the default for binaries. (Halpin, TechReport ORM2-02, p. 8) | Stating it explicitly is normal, not a smell |
| Mandatory | "Each Domain has exactly one Visibility." | "exactly one" = at most one + at least one |
| Optional | "Each Domain belongs to at most one Organization." | "at most one" = unique but not required |

**Common mistake**: writing "Each X has at most one Y" AND "Each Y belongs to at most one X" when you mean only one side is unique. Only constrain the side that IS unique. If many SupportRequests share a Priority, only write "Each SupportRequest has at most one Priority" â€” do NOT add a UC on Priority.

### Objectification

**Objectification requires a spanning uniqueness constraint.** A fact type may only be objectified (promoted into an entity type) if it has a UC that spans all its roles. This is not a guideline â€” it is a formal requirement from Halpin's "Objectification and Atomicity" (2020).

**Why:** Objectification promotes a fact type into an entity type. An entity needs a unique identifier â€” which is exactly what a spanning UC provides. Without a spanning UC, the fact type has no natural identity, so the resulting entity cannot be consistently populated or referenced. Non-spanning objectifications also violate atomicity: they force users to populate non-atomic fact entries (e.g., identifying a Birth by both Person and Country when the birth year fact type only has a UC on Person).

**The rule:**
1. A fact type with a spanning UC may be objectified. The spanning UC becomes the preferred reference scheme of the objectified entity.
2. If the fact type has multiple spanning UCs (e.g., a ternary with two overlapping n-1 UCs), one must be chosen as the preferred reference scheme at objectification time.
3. A fact type WITHOUT a spanning UC should NOT be objectified. Model it differently instead â€” typically by flattening into separate binary facts or by introducing an independent identifying value (e.g., a certificate number).
4. Unary fact types are fine to objectify (they have an implied spanning UC).

**Examples:**

Good â€” spanning UC exists:
- "Person is husband of Person. Each Person, Person combination occurs at most once in the population of Person is husband of Person." â†’ spanning UC â†’ objectify as CurrentMarriage
- Better: give CurrentMarriage an independent reference scheme (e.g., marriage certificate number) to avoid the sub-conceptual choice of which role identifies it

Bad â€” no spanning UC:
- "Person was born in Country. Person occurred in Year." â†’ Birth has a UC only on Person (n:1 from Person to Country) â€” this is NOT spanning. Do NOT objectify Birth. Instead, model as separate binaries: "Person was born in Country" and "Person has Birth Year."

**(Halpin, "Objectification and Atomicity", 2020, p. 1, 5)**

### Mandatory vs Optional Roles

- **Mandatory**: every instance of the object type MUST play that role (e.g., every Listing MUST have a VIN)
- **Optional**: instances MAY play the role (e.g., a Listing MAY have AccidentCount)
- **Exclusive-or**: exactly one of several roles must be played (e.g., tenured XOR contracted)
