# 03 · Constraints

Constraints are the facts your data must satisfy. arest supports seventeen constraint kinds from ORM 2, every one of which compiles to the same shape: a restriction `Filter(p) : P` over the population. When the restriction produces a non-empty set, you have a violation, and the compiler returns the original reading as the error message.

## Alethic vs deontic

Every constraint carries a modality.

An **alethic** constraint is a rule the data must satisfy. Violations reject the command outright. These are your business invariants — the ones where the bad state should not exist, not for a moment.

```forml2
Each Order was placed by exactly one Customer.
```

A **deontic** constraint is a rule the data *ought* to satisfy. Violations warn but do not reject. The command proceeds and the violation is recorded in the response and in the audit trail.

```forml2
It is obligatory that each Order ships within 48 hours of being Placed.
```

Prefer alethic when you can. Reach for deontic only when you know exceptions will happen and you need to log rather than block.

## Cardinality family

Six constraint kinds check whether a count falls within bounds `[k, m]`.

### Uniqueness (UC)

At most one value on the constrained side.

```forml2
Each Order was placed by exactly one Customer.
Each Order has at most one Amount.
```

`exactly one` means UC + MC (one and only one). `at most one` means UC only (zero or one).

Compound uniqueness — the combination of two roles is unique, but neither alone:

```forml2
Each Employee earns at most one Salary in each Year.
```

### Mandatory (MC)

At least one value required.

```forml2
Each Order has some Amount.
Each Order was placed by some Customer.
```

### Frequency (FC)

Bounded counts.

```forml2
Each Order has at least 2 and at most 5 Line Items.
```

### Exclusive-or (XO)

Exactly one of several unary fact types holds.

```forml2
User is active or User is banned, but not both.
```

### Inclusive-or / Disjunctive-mandatory (OR)

At least one of several fact types holds.

```forml2
For each Person, some Person has Email or some Person has Phone.
```

### Exclusion (XC)

No overlap between two or more sets.

```forml2
No Employee is a Contractor.
```

## Membership family

Nine constraint kinds check whether a tuple exists (or is absent) in a target set.

### Subset (SS)

One fact type's extension is included in another's.

```forml2
If some Fact uses some Resource for some Role then that Resource is instance of some Noun that plays that Role.
```

The compiler detects the conditional `if ... then ...` and generates a subset check that scans every antecedent tuple against the consequent set.

### Equality (EQ)

Two fact types have identical extensions.

```forml2
Person is registered iff Person has verified Email and Person accepted Terms.
```

`iff` compiles to a two-way subset: every LHS tuple must be in RHS and vice versa.

### Irreflexive (IR)

No self-reference. Ring constraint — both roles played by the same noun.

```forml2
No Person is a parent of themselves.
```

### Asymmetric (AS)

If `xRy` then not `yRx`.

```forml2
If Person1 is a parent of Person2 then Person2 is not a parent of Person1.
```

### Antisymmetric (AT)

`xRy ∧ yRx → x = y`.

```forml2
If Person1 is older than Person2 and Person2 is older than Person1 then Person1 and Person2 are the same Person.
```

Implies irreflexivity is acceptable only for trivially-equal pairs.

### Symmetric (SY)

`xRy → yRx`. Declare and let the derivation chain fill in reverse edges if needed.

```forml2
Person1 is a sibling of Person2 implies Person2 is a sibling of Person1.
```

### Intransitive (IT)

No transitive closure. Used rarely, but exists.

```forml2
If Person1 is a parent of Person2 and Person2 is a parent of Person3 then Person1 is not a parent of Person3.
```

### Transitive (TR)

`xRy ∧ yRz → xRz`.

```forml2
If Region1 contains Region2 and Region2 contains Region3 then Region1 contains Region3.
```

When declared with `iff`, the derivation engine fills in the transitive edges.

### Acyclic (AC)

No cycles in the relation, of any depth. Compiles to transitive-closure cycle detection via `Func::Platform("tc_cycles")`.

```forml2
No App may cycle back to itself via one or more traversals through extends.
```

### Value Comparison (VC)

Constrain a value to a fixed set.

```forml2
The possible values of Severity are 'error', 'warning', 'info'.
```

Declared implicitly when you give a value type its possible values.

## Textual / deontic keyword constraints

For deontic text constraints — rules over free-form text — the OWA path looks for keyword co-occurrence and fires when more than half the keywords are present. Use this sparingly:

```forml2
It is forbidden that a response contains hate, violence, or slur.
```

This produces a `Filter(Gt(Length(matched), threshold))` restriction over any response-shaped fact. See [derivation rules](05-derivation-rules.md) for more on how the compiler builds these predicates.

## Violation messages

When a constraint fires, the response includes the original reading as the error. This is Corollary: Verbalization — `compile⁻¹(c) = reading` by injectivity of parse and compile.

```json
{
  "rejected": true,
  "violations": [
    {
      "constraint_id": "c4",
      "reading": "Each Order was placed by exactly one Customer.",
      "modality": "Alethic",
      "detail": { ... }
    }
  ]
}
```

Your agent or UI can feed this text straight back to the user. No error-code lookup, no separate message catalog.

## What's next

Constraints catch bad data. [State machines](04-state-machines.md) tell your entities how to change over time.
