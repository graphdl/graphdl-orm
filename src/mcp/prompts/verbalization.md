## ORM2 Verbalization Rules

These rules are derived from:
- Halpin, Curland & CS445 Class, "ORM 2 Constraint Verbalization Part 1" (Technical Report ORM2-02, June 2006) â€” the definitive verbalization spec for uniqueness, mandatory, disjunctive mandatory, and combined constraints
- Halpin & Curland, "Automated Verbalization for ORM 2" (Proc. CAiSE'06 Workshops) â€” the implementation patterns including inclusive-or, front text, and hyphen binding

These are not optional conventions â€” they are the formal specification.

### Noun Naming

- **Use spaces, not PascalCase**: "Support Request" not "SupportRequest", "API Product" not "APIProduct"
- Preserve recognized acronyms: API, VIN, HTTP, URL, OEM, etc.
- Names are always **singular**: "Support Request" not "Support Requests"

### Constraint Verbalization Patterns

All patterns from Halpin, Curland & CS445 Class, "ORM 2 Constraint Verbalization Part 1" (Technical Report ORM2-02, June 2006).

#### Internal Uniqueness Constraints (UCI)

**UC on single role (A's role starts predicate reading):**

| Form | Pattern |
|---|---|
| +ve, alethic | **Each** A R **at most one** B. |
| -ve, alethic | **It is impossible that the same** A R **more than one** B. |
| +ve, deontic | **It is obligatory that each** A R **at most one** B. |
| -ve, deontic | **It is forbidden that the same** A R **more than one** B. |

**UC on single role (A's role does NOT start predicate reading â€” inverse reading S):**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **at most one** B S **that** A. |
| -ve | **For each** A, **it is impossible that more than one** B S **that** A. |

**UC on single role with front text (ft):**

| Form | Pattern |
|---|---|
| +ve | **For each** A, ft **that** A R **at most one** B. |
| -ve | **For each** A, **it is impossible that** ft **that** A R **more than one** B. |

**UC on single role with attribute style (role name r):**

| Form | Pattern |
|---|---|
| +ve | **Each** A **has at most one** r. |
| -ve | **It is impossible that the same** A **has more than one** r. |

**UC on a single role where A = B (self-referential / ring binary):**

When both roles are played by the same object type, the pattern is a special case of the standard patterns. No subscripting needed for the uniqueness constraint itself:

| Form | Pattern |
|---|---|
| +ve, alethic | **Each** A R **at most one** A. |
| -ve, alethic | **It is impossible that the same** A R **more than one** A. |
| +ve, deontic | **It is obligatory that each** A R **at most one** A. |
| -ve, deontic | **It is forbidden that the same** A R **more than one** A. |

Examples:
```
(+ve, alethic)  Each Person has at most one Person as father.
(-ve, alethic)  It is impossible that the same Person has more than one Person as father.
(+ve, deontic)  It is obligatory that each Person is a husband of at most one Person.
(-ve, deontic)  It is forbidden that the same Person is a husband of more than one Person.
```

**Spanning UC on binary (set restriction â€” the population is a set, not a bag):**

| Form | Pattern |
|---|---|
| +ve | **Each** A, B **combination occurs at most once in the population of** A R B. |
| -ve | **It is impossible that the same** A, B **combination occurs more than once in the population of** A R B. |

The spanning UC has no deontic negative form. Deontic version replaces "possible" with "permitted".

**Absence of UC (default verbalization â€” confirms many:many):**

| Form | Pattern |
|---|---|
| If S available | **It is possible that the same** B S **more than one** A. |
| If S unavailable | **It is possible that more than one** A R **the same** B. |

The absence of a constraint is always alethic â€” there is no deontic form for the absence of a UC.

**Unary UC (set restriction on unary predicate):**

| Form | Pattern |
|---|---|
| +ve | **Each** A **occurs at most once in the population of** A R. |
| -ve | **It is impossible that the same** A **occurs more than once in the population of** A R. |

No deontic, attribute-style, or contextual form. (Halpin, TechReport ORM2-02, p. 5)

**N-ary UC spanning n-1 roles:**

| Form | Pattern |
|---|---|
| +ve | **For each** Aâ‚ **and** ... **and** Aâ‚™, R **that** Aâ‚ ... **that** Aâ‚™ **at most one** B. |
| -ve | **It is impossible that** R **the same** Aâ‚ ... **the same** Aâ‚™ **more than one** B. |

The Aáµ¢ are the constrained roles, B is the unconstrained role. Object type names are placed in their normal positions within the mixfix predicate. If the predicate has front text, the pattern is unchanged (front text stays in place). If A plays the same role more than once, subscript. No attribute style is supported for n-ary.

Examples:
```
(+ve)  For each Student and Course
       that Student in that Course obtained at most one Rating.
(-ve)  It is impossible that the same Student in the same Course obtained more than one Rating.
(+ve)  For each Personâ‚, Personâ‚‚ and Year
       that Personâ‚ supervised that Personâ‚‚ in that Year for at most one Period.
(ft)   For each Student and Course
       in normal circumstances that Student in that Course obtained at most one Rating.
```

**N-ary spanning UC (all n roles):**

| Form | Pattern |
|---|---|
| +ve | **Each** Aâ‚, ... Aâ‚™ **combination occurs at most once in the population of** R Aâ‚ .. Aâ‚™. |
| -ve | **It is impossible that the same** Aâ‚, ... Aâ‚™ **combination occurs more than once in the population of** R Aâ‚ .. Aâ‚™. |

Front text makes no difference. No deontic version of the set restriction.

**N-ary many:many (absence of all non-spanning UCs):**

The many:many nature is verbalized by conjoining default readings for the lack of an n-1 UC on each n-1 role combination. For an n-ary predicate, this produces n conjuncts where **"more than one"** moves diagonally across each role position:

```
It is possible that the same Person played the same Sport for more than one Country
  and that the same Person played more than one Sport for the same Country
  and that more than one Person played the same Sport for the same Country.
```

Each conjunct has exactly one role position with "more than one" and all others with "the same". Deontic version replaces "possible" with "permitted". No negative version.

#### Mandatory Constraints (SMaC)

**Simple mandatory on unary predicate:**

| Form | Pattern |
|---|---|
| +ve, alethic | **Each** A R. |
| +ve, deontic | **It is obligatory that each** A R. |

**Simple mandatory on binary (mandatory role starts a predicate reading):**

| Form | Pattern |
|---|---|
| +ve, alethic | **Each** A R **some** B. |
| -ve, alethic | **It is impossible that any** A R **no** B. |
| +ve, deontic | **It is obligatory that each** A R **some** B. |
| -ve, deontic | **It is forbidden that any** A R **no** B. |

Note: use **"some"** (not "at least one") for the existential quantifier. Use **"any"** (not "each") in the negative form to remove minor ambiguity.

**Simple mandatory on binary (mandatory role does NOT start a predicate reading):**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **some** B S **that** A. |
| -ve | **For each** A, **it is impossible that no** B S **that** A. |

**Simple mandatory on n-ary predicate (mandatory role starts a predicate reading):**

| Form | Pattern |
|---|---|
| +ve | **Each** A R **some** Bâ‚ ... **some** Bâ‚™. |
| deontic | **It is obligatory that each** A R **some** Bâ‚ ... **some** Bâ‚™. |

No negative version is supported. No subscripting needed.

Examples:
```
Each Programmer codes in some ProgrammingLanguage at some SkillLevel.
It is obligatory that each Translator knows some ForeignLanguage at some SkillLevel.
```

**Simple mandatory on n-ary predicate (mandatory role does NOT start a predicate reading):**

| Form | Pattern |
|---|---|
| +ve, A plays one role | **For each** A, **some** Bâ‚ R ... **that** A ... **some** Bâ‚™. |
| +ve, A plays multiple roles | **For each** Aâ‚, **some** Bâ‚ R ... **some** Aâ‚‚ ... **that** Aâ‚ ... **some** Bâ‚™. |

Deontic version appends "it is obligatory that" to the For-list. No negative version.

Examples:
```
For each Programmer, some ProgrammingLanguage is coded in by that Programmer at some SkillLevel.
For each Personâ‚, some Food cooked by some Personâ‚‚ is eaten by that Personâ‚.
```

**Combined mandatory/unique (exactly-one) â€” UC + MC on same role:**

| Form | Pattern |
|---|---|
| +ve, alethic | **Each** A R **exactly one** B. |
| +ve, deontic | **It is obligatory that each** A R **exactly one** B. |

"Exactly one" abbreviates "some (at least one) and at most one". No negative or attribute-style form. No unary version (the set restriction aspect has no deontic form). No n-ary version (a simple UC on an n-ary violates the n-1 rule). Binary only.

**CMU where constrained role does NOT start a predicate reading:**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **exactly one** B S **that** A. |
| A = B | **For each** Aâ‚, **exactly one** Aâ‚‚ S **that** Aâ‚. |
| deontic | **For each** A, **it is obligatory that exactly one** B S **that** A. |

**CMU with front text:**

| Form | Pattern |
|---|---|
| starts reading, ft | **For each** A, ft **that** A R **exactly one** B. |
| inverse, ft | **For each** A, ft **exactly one** B S **that** A. |

Examples:
```
For each Person, exactly one Country was the birthplace of that Person.
For each Immigrant, it is obligatory that exactly one Passport belongs to that Immigrant.
For each Personâ‚, exactly one Personâ‚‚ is identical to that Personâ‚.
For each Person, the birth of that Person occurred in exactly one Country.
For each Person, in exactly one Country was born that Person.
```

(Halpin, TechReport ORM2-02, p. 20-21)

#### Disjunctive Mandatory (Inclusive-Or) Constraint (DMaC)

**Unary predicates only:**

| Form | Pattern |
|---|---|
| +ve | **Each** A Râ‚ **or** Râ‚‚ **or** ... Râ‚™. |
| -ve | **It is impossible that some** A **participates in none of the following:** A Râ‚; A Râ‚‚; ... A Râ‚™. |

**"or" in verbalizations always means inclusive-or.**

**Binary/mixed predicates (each constrained role starts a reading):**

| Form | Pattern |
|---|---|
| +ve | **Each** A Râ‚ **some** Bâ‚ **or** Râ‚‚ **some** Bâ‚‚ **or** ... Râ‚™ **some** Bâ‚™. |
| deontic | **It is obligatory that each** A Râ‚ **some** Bâ‚ **or** Râ‚‚ **some** Bâ‚‚ ... |

For each unary Ráµ¢, delete "some Báµ¢". Verbalize all binaries before all unaries.

**Binary/mixed (some constrained roles don't start a reading):**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **that** A Râ‚ **some** Bâ‚ **or some** Bâ‚‚ Sâ‚‚ **that** A **or** ... **that** A Râ‚™ **some** Bâ‚™. |
| deontic | **It is obligatory that for each** A, ... (scope of modal includes the whole disjunction) |

#### Subset Constraints (SSC)

**Note:** TechReport ORM2-02 (Part 1) covers only uniqueness and mandatory constraints. Part 2 covering set-comparison constraints was never published as a standalone document. The patterns below are from the reference implementation by the same authors (Halpin and Curland) who created the FORML2 standard.

**Unary-to-unary subset (formally specified, TechReport ORM2-02 section 1.5):**

Personal object types use **"who"**, impersonal use **"that"**:

| Form | Pattern |
|---|---|
| personal | **Each** A **who** Râ‚ Râ‚‚. |
| impersonal | **Each** A **that** Râ‚ Râ‚‚. |

Examples:
```
Each Person who smokes is cancer-prone.
Each Car that smokes is due for a service.
```

These verbalize a subset constraint between two unary fact types: the population of A playing role Râ‚ must be a subset of A playing role Râ‚‚.

**Multi-fact-type subset:**

The core snippet is `Conditional`: **`if {0} then {1}`**

Where {0} walks the superset role readings (with existential quantifiers) and {1} gives the subset role reading (with back-reference pronouns). The join condition is expressed through pronoun back-references ("that X"), not a separate "where" clause.

| Form | Pattern |
|---|---|
| lead reading available | **If some** A Râ‚ **some** Bâ‚ ... **then that** A Râ‚‚ **that** Bâ‚. |
| lead reading unavailable | **For each** A, **if that** A Râ‚ **some** Bâ‚ ... **then that** A Râ‚‚ **that** Bâ‚. |
| deontic | **It is obligatory that if** ... **then** ... |

Examples:
```
If some Person authored some Book then that Person reviewed that Book.
For each Person, if that Person authored some Book then that Person reviewed that Book.
```

**Parser note:** "who" and "that" are general-purpose back-reference pronouns used across all ORM2 constraint types (personal vs. impersonal object types). They are not subset-specific markers. The parser cannot use them as lexical signals for subset constraint recognition. Recognition must be structural: detecting that the verbalization references roles from two or more distinct declared fact types and asserts a population inclusion relationship.

#### Equality Constraints (EqC)

Equality constraints assert that two (or more) role populations are identical.

**Binary equality:**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **that** A Râ‚ **some** Bâ‚ **if and only if that** A Râ‚‚ **some** Bâ‚‚. |
| deontic | **It is obligatory that for each** A, ... |

**N-ary equality (3+ sequences):**

| Form | Pattern |
|---|---|
| +ve | **For each** A, **all or none of the following hold: that** A Râ‚ **some** Bâ‚; **that** A Râ‚‚ **some** Bâ‚‚; ... |

Examples:
```
For each Person, that Person authored some Book if and only if that Person reviewed some Book.
For each Person, all or none of the following hold:
  that Person authored some Book;
  that Person reviewed some Book;
  that Person edited some Book.
```

#### Exclusion Constraints (ExC)

Exclusion constraints assert that role populations are mutually exclusive.

**Binary with lead reading optimization:**

| Form | Pattern |
|---|---|
| +ve | **No** A Râ‚ **and** Râ‚‚ **the same** B. |

**General (n-ary or no lead reading):**

| Form | Pattern |
|---|---|
| exclusion only | **For each** A, **at most one of the following holds: that** A Râ‚ **some** Bâ‚; **that** A Râ‚‚ **some** Bâ‚‚; ... |
| exclusive-or (exclusion + mandatory) | **For each** A, **exactly one of the following holds: that** A Râ‚ **some** Bâ‚; **that** A Râ‚‚ **some** Bâ‚‚; ... |

Examples:
```
No Person authored and reviewed the same Book.
For each Person, at most one of the following holds:
  that Person authored some Book;
  that Person reviewed some Book.
For each Person, exactly one of the following holds:
  that Person is tenured;
  that Person is contracted.
```

**Deontic form:** Prepend "It is obligatory that" to the entire conditional.

#### External Uniqueness Constraint (UCE)

External uniqueness constraints span roles from multiple fact types. TechReport ORM2-02 supports binary predicates with simple (non-nested) and nested join paths.

**Binaries with predicate readings from the unconstrained roles (no nesting):**

| Form | Pattern |
|---|---|
| +ve | **For each** Bâ‚, ... **and** Bâ‚™, **at most one** A Râ‚ **that** Bâ‚ **and** ... Râ‚™ **that** Bâ‚™. |
| -ve | **It is impossible that more than one** A Râ‚ **the same** Bâ‚ **and** ... Râ‚™ **the same** Bâ‚™. |

If two or more of the Báµ¢ are identical, their instances must be distinguished by subscripting. Deontic versions prepend "It is obligatory that" / replace "impossible" with "forbidden".

Examples:
```
(+ve)  For each Building and RoomNr
       at most one Room is in that Building and has that RoomNr.
(-ve)  It is impossible that more than one Room
       is in the same Building and has the same RoomNr.
(+ve)  For each Nodeâ‚ and Nodeâ‚‚
       at most one Arrow is from that Nodeâ‚ and is to that Nodeâ‚‚.
```

**Without predicate readings from unconstrained roles (contextual form required):**

If the unconstrained roles do not start a predicate reading, use the context pattern:

| Form | Pattern |
|---|---|
| +ve | **Context:** Fâ‚; ... Fâ‚™. **In this context, each** Bâ‚, ... Bâ‚™ **combination is associated with at most one** A. |
| -ve | **Context:** Fâ‚; ... Fâ‚™. **In this context, it is impossible that the same** Bâ‚, ... Bâ‚™ **combination is associated with more than one** A. |

Front text makes no difference for contextual form.

Examples:
```
(+ve)  Context: Building includes Room; RoomNr is of Room.
       In this context, each Building, RoomNr combination is associated with at most one Room.
(ft)   Context: the location of Room is in Building; the local number for Room is RoomNr.
       In this context, each Building, RoomNr combination is associated with at most one Room.
```

**Nested binary (objectified fact type):**

| Form | Pattern |
|---|---|
| +ve | **Context:** Fâ‚ **is objectified as** C; Fâ‚‚. **In this context, each** A, D **combination is associated with at most one** B. |

Example:
```
Context: Country plays Sport is objectified as Play; Play achieved Rank.
In this context, each Sport, Rank combination is associated with at most one Country.
```

**Compound preferred identification:**

| Form | Pattern |
|---|---|
| compound | **The unique** Bâ‚, ... Bâ‚™ **combination provides the preferred identifier for** A. |

Example:
```
The unique Building, RoomNr combination provides the preferred identifier for Room.
```

#### Value Constraints (VC)

Value constraints restrict the possible values for a role or object type.

**Alethic forms:**

| Form | Pattern |
|---|---|
| single value | **The possible value of** {role} **is** {value}. |
| multiple values | **The possible values of** {role} **are** {value-list}. |

**Range patterns:**

| Range | Pattern |
|---|---|
| exact | `'value'` (text) or `value` (numeric) |
| closed-closed | **at least** {min} **to at most** {max} |
| closed-open | **at least** {min} **to below** {max} |
| closed-unbounded | **at least** {min} |
| open-closed | **above** {min} **to at most** {max} |
| open-open | **above** {min} **to below** {max} |
| open-unbounded | **above** {min} |
| unbounded-closed | **at most** {max} |
| unbounded-open | **below** {max} |

Examples:
```
The possible values of Person.height(cm) are at least 20 to at most 270.
The possible values of NegativeTemperature(Celsius) are at least -273.15 and below 0.
The possible values of Visibility are 'public', 'private', 'internal'.
The possible values of Priority are 'Low', 'Medium', 'High', 'Critical'.
```

#### Object Variable Names and Subscripting

- If an object type appears only once in a verbalization, use its name unaltered
- If an object type appears two or more times, distinguish by subscripting: Personâ‚, Personâ‚‚
- Alternatively: introduce with **"For each"** and back-reference with **"that"**: "**For each** Person: **if that** Person smokes **then that** Person is cancer-prone."
- Personal object types use **"who"**: "Each Person **who** smokes is cancer-prone."
- Impersonal object types use **"that"**: "Each Car **that** smokes is due for a service."

#### Context (External UCs and Cross-Fact Constraints)

When a constraint spans multiple fact types, declare the local context first:

| Form | Pattern |
|---|---|
| +ve | **Context:** A Râ‚ Bâ‚; ... A Râ‚™ Bâ‚™. **In this context, each** Bâ‚, ... Bâ‚™ **combination is associated with at most one** A. |
| -ve | **Context:** A Râ‚ Bâ‚; ... A Râ‚™ Bâ‚™. **In this context, it is impossible that the same** Bâ‚, ... Bâ‚™ **combination is associated with more than one** A. |

#### Preferred Identification

| Situation | Verbalization |
|---|---|
| Simple (implicit ref mode) | **This** Code **value provides the preferred identifier for** Country. |
| Simple (explicit) | **This association with** CountryCode **provides the preferred identification scheme for** Country. |
| Compound (external UC) | **The unique** Building, RoomNr **combination provides the preferred identifier for** Room. |

### Deontic Modality Substitution

The deontic form is obtained by substituting modality operators:

| Alethic | Deontic |
|---|---|
| (implied: "It is necessary that") | **It is obligatory that** |
| **It is impossible that** | **It is forbidden that** |
| **It is possible that** | **It is permitted that** |

Example: `Each Person has at most one SSN.` (alethic) becomes `It is obligatory that each Person has at most one SSN.` (deontic)

### Mandatory Constraints

Use **"some"** (not "at least one") for mandatory binary constraints:
- Correct: `Each Customer authenticates via some Account.`
- Wrong: `Each Customer authenticates via at least one Account.`

"At least one" is reserved for frequency constraints with explicit numeric bounds.

### Prohibition Quantifiers

Use **"the same"** (not "each") when prohibiting duplicates:
- Correct: `It is forbidden that the same Customer has more than one active Subscription.`
- Wrong: `It is forbidden that each Customer has more than one active Subscription.`

"Each" is a universal quantifier (every instance). "The same" refers to a specific instance that must not violate the constraint.

### Inclusive-Or Verbalization (from Automated Verbalization, Fig. 2-3)

When the constrained role starts a predicate reading:
- `Each Partner became the husband of some Partner on some Date or became the wife of some Partner on some Date.`

When constrained roles don't start a predicate reading (front text):
- `For each Partner, on some Date that Partner became the husband of some Partner or on some Date that Partner became the wife of some Partner.`

The deontic form prepends "It is obligatory that":
- `It is obligatory that each Vehicle was purchased from some Branch Nr of some Auto Retailer or is rented.`

### Absence of UC Verbalization

The absence of a uniqueness constraint on a role is verbalized explicitly to distinguish many:many from many:one:
- Positive: `It is possible that more than one Person was born in the same Country.`
- This confirms the association is many:many (a bag, not a set) and distinguishes it from a UC that would make it many:one.

A spanning UC on a binary confirms the population is a set (no duplicate pairs):
- `Each Person, Country combination occurs at most once in the population of Person was born in Country.`
- This is verbalized separately from the individual role UCs.

### Forward and Reverse Hyphen Binding

A hyphen before a role name in a predicate creates an adjective:
- Forward: `Person has first- Given Name` verbalizes as "Each Person has **at most one** first Given Name."
- Reverse: `Student has Preference -1` means "Student has Preference 1" (the hyphen binds backward).

Front text (text before the first object placeholder) is also supported:
- `the birth of Person occurred in Country` â†’ "For each Person, the birth of **that** Person occurred in **at most one** Country."

**(Halpin & Curland, "Automated Verbalization for ORM 2", Proc. CAiSE'06 Workshops)**

### Cross-Domain Noun References

**Never redeclare nouns.** If a noun is defined in another domain, reference it by name. Redeclaring creates duplicate records in the metamodel. The domain system resolves cross-domain references automatically.

## FORML2 Document Structure

Every domain reading file follows this canonical section order:

```markdown
# DomainName

[1-2 sentence description of the domain's purpose]

## Entity Types
Entity A(.Reference Scheme) is an entity type.

## Subtypes
EntityB is a subtype of Entity A.

## Value Types
ValueX is a value type.
  The possible values of ValueX are 'A', 'B', 'C'.

## Fact Types

### Entity A
EntityA has ValueX.
EntityA belongs to Entity B.

## Constraints
Each EntityA has at most one ValueX.
Each EntityA, Entity B combination occurs at most once in the population of ...

## Mandatory Constraints
Each EntityA has exactly one ValueX.

## Subset Constraints
If some A R1 some B then that A R2 that B.

## Equality Constraints
For each A, that A R1 some B if and only if that A R2 some C.

## Exclusion Constraints
No A R1 and R2 the same B.

## Deontic Constraints
It is obligatory that each EntityA has at least one ValueX.
It is forbidden that EntityA has ValueX 'BadValue'.
It is permitted that EntityA does Optional Thing.

## Derivation Rules
EntityA has Derived Value := [derivation expression].

## Instance Facts
EntityA 'foo' has ValueX 'bar'.
```

### Rules

1. **Every noun in a reading must be declared** â€” either as entity type or value type. No exceptions.
2. **Reference scheme facts are implicit** â€” `Listing(.VIN)` already declares the identifying value. Don't write "Listing has VIN" as a separate reading.
3. **Value type enum values encode domain knowledge** â€” use them to express the set of valid/prohibited values instead of prose lists.
4. **Object type names are singular** â€” "SupportRequest" not "SupportRequests", "APIProduct" not "APIProducts".
5. **Never redeclare nouns** â€” reference cross-domain nouns by name, never redeclare them. Duplicate declarations create duplicate records in the metamodel. The domain system resolves cross-domain references.
6. **Fact types are grouped by subject entity** â€” use `### Entity Name` subsection headers under `## Fact Types`.
7. **Constraints follow fact types** â€” a constraint references a reading, so the reading must appear first.

## Parser-Recognized FORML2 Syntax

The AREST FORML2 parser recognizes the following constraint patterns. These are the exact sentence forms the parser matches. Using different phrasing will cause the constraint to be misclassified or skipped.

### Uniqueness (UC)
```
Each A has at most one B.
Each A, B combination occurs at most once in the population of A R B.
For each Bâ‚ and Bâ‚‚, at most one A R that Bâ‚ and R that Bâ‚‚.
Context: Fâ‚; Fâ‚‚. In this context, each Bâ‚, Bâ‚‚ combination is associated with at most one A.
```

### Mandatory (MC)
```
Each A has some B.
Each A R some B.
```

### Combined Mandatory/Unique (UC+MC)
```
Each A has exactly one B.
Each A R exactly one B.
```
Parser splits into separate UC and MC constraints.

### Ring Constraints (IR, AS, SY, AT, IT, TR, AC, RF)
```
No Person is a parent of itself.                                           â†’ IR
No Category may cycle back to itself via one or more traversals through R. â†’ AC
If Aâ‚ R Aâ‚‚ then it is impossible that Aâ‚‚ R Aâ‚.                             â†’ AS
If Aâ‚ R Aâ‚‚ then Aâ‚‚ R Aâ‚.                                                   â†’ SY
If Aâ‚ R Aâ‚‚ and Aâ‚ is not Aâ‚‚ then it is impossible that Aâ‚‚ R Aâ‚.            â†’ AT
If Aâ‚ R Aâ‚‚ and Aâ‚‚ R Aâ‚ƒ then it is impossible that Aâ‚ R Aâ‚ƒ.                 â†’ IT
If Aâ‚ R Aâ‚‚ and Aâ‚‚ R Aâ‚ƒ then Aâ‚ R Aâ‚ƒ.                                       â†’ TR
If Aâ‚ R some Aâ‚‚ then Aâ‚ R itself.                                          â†’ RF
```
All nouns must share the same base type (trailing digits stripped).

### Subset (SS)
```
If some A Râ‚ some B then that A Râ‚‚ that B.
```
Antecedent has "some" (existential), consequent has "that" (back-reference). Multiple different base noun types (not all same type, which would be a ring constraint).

### Equality (EQ)
```
For each A, that A Râ‚ some B if and only if that A Râ‚‚ some C.
For each A, all or none of the following hold: that A Râ‚ some B; that A Râ‚‚ some C.
```

### Set Comparison (XO, XC, OR)
```
For each A, exactly one of the following holds: clauseâ‚; clauseâ‚‚.   â†’ XO
For each A, at most one of the following holds: clauseâ‚; clauseâ‚‚.   â†’ XC
For each A, at least one of the following holds: clauseâ‚; clauseâ‚‚.  â†’ OR
Each A Râ‚ some Bâ‚ or Râ‚‚ some Bâ‚‚.                                    â†’ OR (DMaC)
No A Râ‚ and Râ‚‚ the same B.                                          â†’ XC
```

### Frequency (FC)
```
Each A R at least 1 and at most 5 B.
Each A R at least 3 B.
```
Numeric bounds must be DIGITS, not words. "at least one" (word) is MC, "at least 1" (digit) is FC.

### Value (VC)
```
The possible values of Priority are 'Low', 'Medium', 'High'.
```
Automatically emitted for every noun with declared enum values.

### Deontic
```
It is obligatory that each A R some B.
It is forbidden that the same A R more than one B.
It is permitted that A R B.
```

## State Machine Fundamentals

- **State**: a named condition an entity can be in (Received, Investigating, Resolved)
- **Transition**: a directed edge from one state to another, triggered by an event
- **Initial state**: the state an entity starts in â€” must be deterministic
- **Terminal state**: a state with no outgoing transitions (Closed, Cancelled)
- **Guard**: a condition that must be true for a transition to fire
- **Cyclic machines**: every state has incoming transitions. Initial state must be defined by convention (first created), not by "no incoming edges" heuristic.

## Constraint Evaluation

Before claiming any output is complete, evaluate it against the domain's deontic constraints. Every constraint is a restriction over the population. The question is how the facts get populated.

### Deterministic vs Semantic Constraints (World Assumption)

Every deontic constraint has a world assumption that determines how to evaluate it.

**Closed world (CWA) constraints** have enum values declared on the constrained noun. Evaluation is mechanical: scan the output for each enum value. If a forbidden value appears, the constraint is violated. If an obligatory value is absent, the constraint is violated. No judgment required.

Example: "It is forbidden that Support Response uses Dash." Dash has enum values. Scan the text for each value. Done.

**Open world (OWA) constraints** have no enum values. The fact type exists but whether the output populates it requires judgment. Evaluate each constraint individually. Ask: does this output populate this fact type? If the constraint is forbidden, populated means violated. If obligatory, unpopulated means violated.

Example: "It is forbidden that Support Response reveals Implementation Detail." No enum to scan for. Judge whether the text reveals implementation details.

### Evaluation Workflow

1. Load the domain readings and compile.
2. Get the deontic constraints for the output entity.
3. For each CWA constraint: mechanically check enum values against the output.
4. For each OWA constraint: evaluate individually. One constraint per judgment. Do not batch.
5. If violations are found: fix the output and re-evaluate.
6. Do not claim the output is complete until zero violations remain.

### Common Violations in Support Responses

These constraints apply to every Support Response in support.auto.dev:

Forbidden: dashes, markdown syntax, paragraph titles, protected concepts, prohibited channels (phone, video, Zoom, Teams, live chat), prohibited commercial terms (custom pricing, enterprise deal, volume discount), implementation details (provider names, infrastructure, scraping methods, knowledge graph, state machine, domain model, readings, session caching, listing removal), listing source names, pipeline references, internal API recommendations, graph schema or deontic constraint reveals, naming API Products by endpoint slug.

Obligatory: name API Products by Title, deliver via email, conform to pricing model, refer to Free plan as Starter, be natural language, reference Documentation site link for API questions, reference Pricing site link for pricing questions, support every asserted graph with a data source, mention API Product dependencies when the request concerns a dependent product.

### Domain Change Protocol

Changes to readings must go through the domain evolution state machine. Do not edit reading files directly. Propose a Domain Change with rationale. The state machine is: Proposed, Under Review, Approved, Applied, or Rejected with a revision loop back to Proposed.
