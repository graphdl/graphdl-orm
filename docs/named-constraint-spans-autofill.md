# Named Constraint Spans with Autofill

## Summary

New FORML2 pattern for declaring that subset constraint spans should auto-populate their target roles when superset conditions are met. Requires three parser additions: subset constraint verbalization (existing), span naming via objectification pattern (new), and autofill unary instance fact (new).

## Motivation

When a support request arrives from an email not associated with any customer account, the system needs fallback matching strategies — name match, OAuth account email match, etc. Each strategy is a subset constraint targeting the same roles in "Customer submits Support Request." When any strategy matches, the target roles should be auto-populated.

Without autofill, the subset constraints are passive checks. With autofill, they actively populate the subset when the superset is satisfied.

## The Pattern

```
## Subset Constraints

If some Support Request has some Email Address and some Customer is identified by that Email Address then that Customer submits that Support Request.
If some Support Request has some contact- Name and some Customer has that Name then that Customer submits that Support Request.
If some Support Request has some Email Address and some Account has that Email Address and that Account is for some Customer then that Customer submits that Support Request.

This span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.
Constraint Span 'Customer Submission Match' autofills from superset.
```

## Three Parts

### 1. Subset constraint verbalizations (existing parser support)

Standard `if...then` pattern per Halpin/Curland SSC verbalization:

```
If some A R1 some B then that A R2 that B.
```

- "some" introduces existential quantifiers in the superset (antecedent)
- "that" provides back-references in the subset (consequent)
- Each line is independently parseable
- Multiple constraints can target the same subset roles

### 2. Span naming (new)

```
This span with Customer, Support Request provides the preferred identification scheme for Customer Submission Match.
```

Follows the objectification naming pattern used for composite identifiers:

```
This association with Graph Schema, Verb provides the preferred identification scheme for API.
```

**Parse rules:**
- Starts with `This span with`
- Followed by comma-separated Noun names (the roles the constraint covers)
- `provides the preferred identification scheme for` is the standard objectification verb
- Final token is the span name
- "This" backreferences the subset constraints above — there is minor leakage here since "This" requires positional context, but it follows the existing "This association with" convention

**Semantics:** A Constraint Span is the set of roles a constraint covers. It exists because a constraint spans multiple roles — if only one role were involved, only constraints would be needed. The named span covers all listed roles as a unit.

### 3. Autofill declaration (new)

```
Constraint Span 'Customer Submission Match' autofills from superset.
```

**Parse rules:**
- Entity type prefix: `Constraint Span`
- Instance identifier in quotes: `'Customer Submission Match'`
- Unary predicate: `autofills from superset`

**Semantics:** When the superset roles of any subset constraint targeting this span are populated, the subset roles (the span) are auto-populated. Direction is always superset → subset.

## Metamodel Backing

Already defined in `readings/core.md`:

```
This association with Constraint, Role provides the preferred identification scheme for Constraint Span.

### Constraint Span (objectification of "Constraint spans Role")
Constraint Span autofills from superset.
```

- `Constraint Span` is an entity type (objectification of "Constraint spans Role")
- `autofills from superset` is a unary fact type on Constraint Span
- The named span is an instance of Constraint Span
- The autofill declaration is an instance fact of the unary

## Example: Customer Matching

The use case that drove this pattern. A support request arrives with an email address and contact name. The system tries to match a customer:

1. **Email identity match** — the support request's email IS the customer's reference scheme email. Highest confidence, most common.

2. **Name match** — the support request's contact name matches a customer's name. Fallback when the customer emails from a personal address different from their account.

3. **OAuth account match** — the support request's email matches an email on an OAuth Account linked to a customer. Catches cases where the customer has multiple emails across providers.

All three target the same span (Customer, Support Request in "Customer submits Support Request"). The autofill means: when any of these superset conditions are met, populate the "Customer submits Support Request" fact.

## What the Parser Needs to Handle

1. **Recognize `This span with X, Y provides the preferred identification scheme for Z`** as a span naming declaration. Extract the role nouns (X, Y) and the span name (Z). Associate with preceding subset constraints via "This" backreference.

2. **Recognize `Constraint Span 'Z' autofills from superset`** as a unary instance fact. Z must be a previously declared span name. Set the autofill flag on the span.

3. **At evaluation time**, when a subset constraint's superset roles are all populated and the constraint's span has autofill enabled, automatically populate the subset roles.
