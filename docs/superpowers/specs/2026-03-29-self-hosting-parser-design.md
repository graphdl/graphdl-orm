# Self-Hosting FORML 2 Parser

The FORML 2 parser is expressed as readings evaluated by the engine. The TypeScript claims parser is deleted. One parser. One language. Rust/WASM.

## Theoretical Foundation

Three sources. Three layers. No additions.

**Backus (1978)** — the computation kernel. Objects (atoms, sequences, bottom), primitive functions, functional forms, definitions. Programs are point-free compositions of combining forms. No variables. No state mutation.

**Codd (1970)** — the data access patterns. Projection, natural join, restriction, composition. All compose from Backus's forms applied to populations (sets of tuples = sequences of sequences).

**Halpin (2001/2008)** — the predicates and surface syntax. Constraints (uniqueness, mandatory, frequency, value, subset, equality, exclusion, ring), derivation rules, state machines. Expressed in controlled natural language (FORML 2). Comparison is a constraint predicate, not a primitive function.

## Primitive Set

### Objects (Backus 11.2.1)

An object is an atom, a sequence, or bottom.

- **Atom**: a nonnull string. Numbers are atoms. T (true), F (false). The atom phi is both an atom and the empty sequence.
- **Sequence**: `<x1, ..., xn>` where each xi is an object. Bottom-preserving: if any element is bottom, the sequence is bottom.
- **Bottom** (undefined): all functions preserve bottom. `f:bottom = bottom` for all f.

For self-hosting: a line of text is a sequence of character-atoms. `<'E','a','c','h',' ','P','e','r','s','o','n'>`. Backus's sequence operations apply directly.

### Primitive Functions (Backus 11.2.3)

Structural:
- `s:x` — selector at position s (1-indexed). `1:<A,B,C> = A`. `s:<x1,...,xn> = xs` if n >= s, else bottom.
- `tl:<x1,...,xn>` — tail. `<x2,...,xn>` if n >= 2, phi if n = 1, bottom otherwise.
- `id:x = x` — identity.
- `atom:x` — T if x is an atom, F if x is a sequence, bottom if bottom.
- `eq:<y,z>` — T if y = z (both atoms), F if y != z, bottom otherwise.
- `null:x` — T if x = phi, F otherwise.
- `reverse:<x1,...,xn>` — `<xn,...,x1>`.
- `distl:<y,<z1,...,zn>>` — `<<y,z1>,...,<y,zn>>`. Distribute left.
- `distr:<<y1,...,yn>,z>` — `<<y1,z>,...,<yn,z>>`. Distribute right.
- `length:<x1,...,xn>` — n. `length:phi = 0`. `length:atom = bottom`.
- `apndl:<y,<z1,...,zn>>` — `<y,z1,...,zn>`. Append left.
- `apndr:<<y1,...,yn>,z>` — `<y1,...,yn,z>`. Append right.
- `rotl:<x1,...,xn>` — `<x2,...,xn,x1>` if n >= 2. Rotate left.
- `rotr:<x1,...,xn>` — `<xn,x1,...,xn-1>` if n >= 2. Rotate right.
- `trans` — transpose. `trans:<<x11,...,x1m>,...,<xn1,...,xnm>> = <<x11,...,xn1>,...,<x1m,...,xnm>>`.

Arithmetic (on number-atoms):
- `+:<y,z>` — y + z. Bottom if either is not a number.
- `-:<y,z>` — y - z.
- `*:<y,z>` — y * z.
- `div:<y,z>` — y / z. Bottom if z = 0.

Logic:
- `and:<x,y>` — T if both T, F if either F, bottom otherwise.
- `or:<x,y>` — T if either T, F if both F, bottom otherwise.
- `not:x` — T if F, F if T, bottom otherwise.

### Functional Forms (Backus 11.2.4)

- **Composition**: `(f . g):x = f:(g:x)`. Apply g then f.
- **Construction**: `[f1,...,fn]:x = <f1:x,...,fn:x>`. Apply each fi to x, collect results.
- **Condition**: `(p -> f; g):x = f:x if p:x = T, g:x if p:x = F, bottom otherwise`.
- **Apply to all**: `(alpha f):<x1,...,xn> = <f:x1,...,f:xn>`. Map f over each element.
- **Insert (fold)**: `/f:<x1,...,xn> = f:<x1, /f:<x2,...,xn>>` if n >= 2. `/f:<x> = x`.
- **Binary to unary**: `(bu f x):y = f:<x,y>`. Partial application. Fixes the left argument.
- **While**: `(while p f):x = (while p f):(f:x) if p:x = T, x if p:x = F, bottom otherwise`.
- **Constant**: `x-bar:y = x` for all y != bottom. The constant function.

### Definitions (Backus 11.2.5)

`Def l == r` where l is an unused function symbol and r is a functional form. The set of definitions D is part of the FP system specification.

In FORML 2 surface syntax (Halpin):
```
Line Total is derived as quantity times unit price of that Line Item.
```
Compiles to: `Def lineTotal == (bu * unitPrice) . selector("quantity")`

In AREST, definitions are stored in the `defs: HashMap<String, Func>` parameter of `ast::apply()`.

### Relational Operations (Codd 1970, Section 2)

Composed from Backus's forms — not new primitives:

| Codd Operation | Composition |
|---|---|
| Projection pi_L(R) | `[s_i1, ..., s_ik]` — Construction of Selectors |
| Natural Join R*S | `Filter(eq . [s_shared]) . distl` over R x S |
| Restriction R_L\|_M S | `Filter(predicate)` — Filter with constraint predicate |
| Composition R.S | `pi_1s(R*S)` — Projection of Join |
| Permutation | Construction reordering selectors |
| Tie gamma(R) | `Filter(eq . [s_1, s_n])` — Filter where first = last |

### Constraint Predicates (Halpin 2001/2008)

Constraints are predicates over populations. They compile to Backus's Condition form. Comparison is here, not in the primitive functions.

| Constraint | FORML 2 | Compiles To |
|---|---|---|
| Uniqueness (UC) | "Each A R at most one B" | Condition: count per group <= 1 |
| Mandatory (MC) | "Each A R some B" | Condition: count per entity >= 1 |
| Frequency (FC) | "at least N and at most M" | Condition: N <= count <= M |
| Value (VC) | "possible values are [20..270]" | Condition: value in range |
| Subset (SS) | "A iff B" | Condition: population(A) subset population(B) |
| Equality (EQ) | "A iff B" (biconditional) | Condition: populations equal |
| Exclusion (XC) | "not both A and B" | Condition: populations disjoint |
| Exclusive-or (XO) | "A or B but not both" | Condition: exactly one populated |
| Inclusive-or (OR) | "A or B" | Condition: at least one populated |
| Ring (IR/AS/SY/AT/IT/TR/AC/RF) | "No X relates to itself" etc. | Condition over self-referential facts |
| Comparison | "For each GIrange: minGI < maxGI" | Condition: arithmetic on bound values |
| Forbidden | "It is forbidden that X" | Deontic Condition |
| Obligatory | "It is obligatory that X" | Deontic Condition |

Comparison (gt, lt, gte, lte) is expressed as constraint evaluation using Backus's arithmetic primitives (+, -, *, div) and eq. For example, `x > y` is `not(eq . [-, 0-bar]) and not(negative . -)` where negative can be tested via the sign of subtraction. In practice, the constraint compiler emits a Native that does the arithmetic check — but the operation is sourced in Halpin's constraint taxonomy, not added as an unsourced Backus primitive.

## Architecture

### Bootstrap

The engine needs a minimal kernel to load its own syntax readings. The kernel is the set of Backus primitives + functional forms implemented in Rust. This is fixed and does not change.

Bootstrap sequence:
1. Rust implements Backus's primitive functions and functional forms as `Func` AST nodes
2. `readings/core.md` defines the metamodel (Noun, Reading, Constraint, etc.) — currently parsed by a minimal bootstrap
3. `readings/syntax.md` defines FORML 2 grammar as derivation rules over character-sequences
4. The engine loads `syntax.md` using the bootstrap, then can parse any FORML 2 input by evaluating its syntax rules

The bootstrap parser is the irreducible minimum: it recognizes just enough FORML 2 to load the syntax readings. It is small, fixed, and implemented in Rust. Everything above it is readings.

### Text as Sequences

Input FORML 2 text is decomposed into a population of character-sequence facts:

```
"Each Customer has exactly one Email."
```

Becomes the fact: `Line(1, <'E','a','c','h',' ','C','u','s','t','o','m','e','r',...>)`

Derivation rules in `readings/syntax.md` pattern-match on these sequences using Backus's operations:
- Subsequence matching via `alpha(eq)` with `distl`
- Splitting via `Filter` at delimiter characters
- Token extraction via `tl`, selectors, `apndl`

### Syntax Rules as Readings

`readings/syntax.md` expresses the FORML 2 grammar. Examples:

```
# FORML 2 Syntax

## Derivation Rules

Line declares Entity Type iff that Line's tokens end with
  sequence <'i','s'>, <'a','n'>, <'e','n','t','i','t','y'>, <'t','y','p','e'>.

Line declares Fact Type iff that Line's tokens contain a known Noun
  followed by a verb phrase followed by another known Noun.

Line declares Uniqueness Constraint iff that Line's tokens begin with
  <'E','a','c','h'> and contain <'a','t'>, <'m','o','s','t'>, <'o','n','e'>.

Line declares Derivation Rule iff that Line contains the token <'i','f','f'>
  or that Line contains the token sequence <'i','s'>, <'d','e','r','i','v','e','d'>, <'a','s'>.
```

The exact surface syntax for these meta-readings will be refined during implementation. The point is: the grammar is data (readings), not code (Rust/TypeScript procedures).

### What Gets Deleted

The entire TypeScript claims parser:
- `src/claims/ingest.ts` — replaced by engine evaluation of syntax readings
- `src/claims/tokenize.ts` — replaced by character-sequence operations
- `src/claims/constraints.ts` — replaced by constraint pattern derivation rules
- `src/claims/steps.ts` — replaced by forward chaining
- `src/claims/scope.ts` — replaced by engine scope resolution
- `src/claims/batch-builder.ts` — replaced by engine population construction

Also `crates/fol-engine/src/parse_rule.rs` — its `:=` syntax is replaced by FORML 2 `iff` / `if` / `Define...as` patterns recognized by the syntax readings.

### WASM Interface Change

Current: `load_ir(ir_json: &str)` — TypeScript parser produces JSON IR, Rust compiles it.

New: `load_readings(markdown: &str)` — Rust parses FORML 2 markdown directly, compiles, evaluates.

The TypeScript layer becomes a thin HTTP proxy:
1. Receive readings (markdown text)
2. Pass to WASM: `load_readings(text)`
3. Receive AREST commands
4. Pass to WASM: `apply_command(cmd, population)`
5. Return hypermedia response

No parsing in TypeScript. No IR format. Readings in, hypermedia out.

## Backus Primitives: Implementation Gaps

Current `ast.rs` has:
- Selector, Construction, Composition, Condition, ApplyToAll, Insert, Filter, Constant, Native

Missing from Backus that must be added:
- `tl` (tail)
- `atom` (type test)
- `eq` (equality — currently done via Native, should be a first-class Func)
- `null` (empty test)
- `reverse`
- `distl`, `distr` (distribute)
- `length`
- `+`, `-`, `*`, `div` (arithmetic — currently done via Native, should be first-class)
- `and`, `or`, `not` (logic)
- `apndl`, `apndr` (append)
- `rotl`, `rotr` (rotate)
- `trans` (transpose)
- `bu` (binary to unary — partial application)
- `while` (bounded iteration)

The existing `Func::Native` escape hatch can be removed once all Backus primitives are first-class `Func` variants. Native was a bridge; the primitives are the destination.

## FORML 2 Derivation Rule Syntax

The parser must recognize Halpin's established forms (not the current `:=` syntax):

### Full Derivation (iff-rules)

Relational style:
```
Person1 is an uncle of Person2 iff Person1 is a brother of
  some Person3 who is a parent of Person2.
```

Attribute style:
```
For each Person: uncle = brother of parent.
```

### Partial Derivation (if-rules)

```
Person1 is a Grandparent if Person1 is a parent of some Person2
  who is a parent of some Person3.
```

### Subtype Derivation

```
Each Australian is a Person who was born in Country 'AU'
  and has Population >= 1000000.
```

### Aggregation

```
Quantity = count each Academic who has Rank and works for Dept.
For each PublishedBook, totalCopiesSold = sum(copiesSoldInYear).
```

### Recursive Derivation

```
EgyptianGod1 is an ancestor of EgyptianGod2 iff
  EgyptianGod1 is a parent of EgyptianGod2
  or EgyptianGod1 is a parent of some EgyptianGod3
     who is an ancestor of EgyptianGod2.
```

### Variable Binding (Halpin TechReport ORM2-02)

- Pronouns for unambiguous reference: `who` (personal), `that` (impersonal), `some` (existential)
- Subscripted type names when ambiguous: `Person1`, `Person2`
- Implicit universal quantification on head variables
- Implicit existential quantification on body-only variables

## GraphDL Skill Updates

The graphdl skill must be updated to document:

1. **Combining Forms section** — Backus's FP algebra mapped to AREST domain primitives (the table from the whitepaper)
2. **Derivation Rules section** — FORML 2 syntax for iff/if rules, aggregation, recursion, attribute style, with examples
3. **AREST Execution Model section** — Command : Population -> (Population', Representation), the create pipeline (emit . validate . derive . resolve), HATEOAS as projection
4. **Primitive Functions reference** — Backus's complete set with domain interpretations
5. **Constraint compilation** — how each Halpin constraint type maps to Backus's Condition form
6. **Self-hosting** — the parser is readings, text is character-sequences, the grammar is derivation rules

## Unit Tests

Tests are organized by layer: primitives, forms, relational operations, constraints, parsing, and self-hosting. Each Backus primitive gets its own test. Each functional form gets composition tests. Each Halpin constraint type gets evaluation tests against sample populations.

### Backus Primitive Functions

Each primitive is tested against Backus's own definitions (Section 11.2.3). Bottom-preservation is verified for every primitive.

```
// Selector
1:<A,B,C> = A
2:<A,B,C> = B
3:<A,B> = bottom  (n < s)
s:bottom = bottom

// Tail
tl:<A,B,C> = <B,C>
tl:<A> = phi
tl:phi = bottom
tl:atom = bottom

// Equality
eq:<A,A> = T
eq:<A,B> = F
eq:<A,<B>> = bottom  (not both atoms)

// Null
null:phi = T
null:<A> = F
null:A = F  (atom is not phi unless it IS phi)

// Distribute
distl:<A,<B,C,D>> = <<A,B>,<A,C>,<A,D>>
distl:<A,phi> = phi
distr:<<A,B,C>,D> = <<A,D>,<B,D>,<C,D>>

// Length
length:<A,B,C> = 3
length:phi = 0
length:atom = bottom

// Arithmetic
+:<3,4> = 7
-:<7,4> = 3
*:<3,4> = 12
div:<12,4> = 3
div:<12,0> = bottom

// Logic
and:<T,T> = T
and:<T,F> = F
or:<F,F> = F
not:T = F
not:bottom = bottom

// Append
apndl:<A,<B,C>> = <A,B,C>
apndr:<<A,B>,C> = <A,B,C>

// Reverse
reverse:<A,B,C> = <C,B,A>
reverse:phi = phi

// Rotate
rotl:<A,B,C> = <B,C,A>
rotr:<A,B,C> = <C,A,B>

// Transpose
trans:<<A,B>,<C,D>> = <<A,C>,<B,D>>
```

### Functional Forms

Tested via Backus's own examples (Sections 11.3, 12.2).

```
// Composition
(tl . tl):<A,B,C> = <C>

// Construction
[1, tl]:<A,B,C> = <A, <B,C>>

// Condition
(null -> 0-bar; length):<A,B> = 2
(null -> 0-bar; length):phi = 0

// Apply to all
(alpha 1):<<A,B>,<C,D>> = <A,C>

// Insert (fold)
/+:<1,2,3> = 6
/+:<7> = 7

// Binary to unary (partial application)
(bu + 1):2 = 3
(bu eq A):A = T
(bu eq A):B = F

// While
// Backus: (while p f):x = (while p f):(f:x) if p:x = T, x if p:x = F
// Test: subtract 1 until zero
(while (not . null . tl) tl):<A,B,C> = <C>

// Constant
42-bar:<anything> = 42
42-bar:bottom = bottom

// Inner product (Backus 5.2 / 11.3.2)
// Def IP == (/+) . (alpha *) . trans
IP:<<1,2,3>,<6,5,4>> = 28
```

### Backus Algebra Laws (Section 12.2)

Verify the algebraic laws hold. These are mechanical checks that the implementation respects the algebra.

```
// I.1  [f1,...,fn] . g == [f1.g,...,fn.g]
[1,2] . tl  applied to <A,B,C>  ==  [1.tl, 2.tl] applied to <A,B,C>
both = <B,C>  -- wrong, let me recalculate
// Actually: [1,2].(tl:<A,B,C>) = [1,2]:<B,C> = <B,C>
// [1.tl, 2.tl]:<A,B,C> = <tl:<A,B,C> select 1... no.
// Law I.1: [f1,...,fn].g == [f1.g,...,fn.g]
// [1,2].tl : <A,B,C> = [1,2]:<B,C> = <B,C>
// [1.tl, 2.tl]:<A,B,C> = <1:(tl:<A,B,C>), 2:(tl:<A,B,C>)> = <1:<B,C>, 2:<B,C>> = <B,C>
// Confirmed: both yield <B,C>

// III.2  f . id == id . f == f
(+ . id):<3,4> = +:<3,4> = 7

// III.4  alpha(f.g) == (alpha f) . (alpha g)
// alpha(1 . reverse):<<A,B>,<C,D>> should equal (alpha 1) . (alpha reverse):<<A,B>,<C,D>>
// Left: alpha(1.reverse):<<A,B>,<C,D>> = <(1.reverse):<A,B>, (1.reverse):<C,D>> = <B,D>
// Right: (alpha 1).((alpha reverse):<<A,B>,<C,D>>) = (alpha 1):<<B,A>,<D,C>> = <B,D>
// Confirmed
```

### Codd Relational Operations (composed from primitives)

```
// Projection: select columns 1 and 3 from a 3-column relation
// pi_{1,3}(R) where R = <<a,b,c>,<d,e,f>>
(alpha [1,3]):<<a,b,c>,<d,e,f>> = <<a,c>,<d,f>>

// Natural Join on shared column
// R = <<1,a>,<2,b>>  S = <<a,x>,<b,y>>
// R*S on column 2 of R = column 1 of S
// Result: <<1,a,x>,<2,b,y>>

// Restriction: filter rows where column 1 = "a"
Filter(eq . [1, a-bar]):<<a,x>,<b,y>,<a,z>> = <<a,x>,<a,z>>
```

### Halpin Constraint Predicates

Each constraint type tested against a sample population. Tests verify that violations are detected and that valid populations pass.

```
// Uniqueness (UC): Each Person was born in at most one Country
// Population: {born(alice,US), born(alice,UK)} -> VIOLATION (alice has two countries)
// Population: {born(alice,US), born(bob,UK)} -> OK

// Mandatory (MC): Each Person was born in some Country
// Population: {person(alice), born(alice,US)} -> OK
// Population: {person(alice)} -> VIOLATION (alice has no birth country)

// Frequency (FC): Each Customer submits at least 1 and at most 3 SupportRequest
// Population with 4 requests from same customer -> VIOLATION
// Population with 2 requests from same customer -> OK

// Value (VC): The possible values of Gender are 'M', 'F'
// Population: {gender(alice, M)} -> OK
// Population: {gender(alice, X)} -> VIOLATION

// Subset (SS): Person smokes subset of Person is cancer-prone
// Every smoker must be cancer-prone. Smoker who is not cancer-prone -> VIOLATION

// Equality (EQ): populations must be equal in both directions
// If SS holds in both directions -> EQ satisfied

// Exclusion (XC): Male and Female populations are disjoint
// Person in both -> VIOLATION

// Ring - Irreflexive: No Person is a parent of themselves
// parent(alice, alice) -> VIOLATION

// Ring - Acyclic: No cycles in manages
// manages(a,b), manages(b,c), manages(c,a) -> VIOLATION

// Ring - Symmetric: If married(a,b) then married(b,a)
// married(alice,bob) without married(bob,alice) -> VIOLATION

// Comparison constraint: For each GIrange: minGI < maxGI
// Population: {range(low, 5, 10)} -> OK (5 < 10)
// Population: {range(bad, 10, 5)} -> VIOLATION (10 not < 5)
```

### FORML 2 Parsing (syntax readings)

Each FORML 2 pattern recognized by the syntax readings, tested by feeding text and checking produced facts.

```
// Entity type declaration
Input:  "Customer(.Email) is an entity type."
Output: EntityType { name: "Customer", ref_scheme: ["Email"] }

// Value type declaration
Input:  "Gender is a value type.\n  The possible values of Gender are 'M', 'F'."
Output: ValueType { name: "Gender", enum_values: ["M", "F"] }

// Subtype declaration
Input:  "Male is a subtype of Person."
Output: Subtype { sub: "Male", super: "Person" }

// Fact type (binary)
Input:  "Customer was born in Country."
Output: FactType { reading: "Customer was born in Country", roles: ["Customer", "Country"] }

// Uniqueness constraint
Input:  "Each Customer was born in at most one Country."
Output: Constraint { kind: "UC", spans: [("Customer was born in Country", 0)] }

// Mandatory constraint
Input:  "Each Customer was born in some Country."
Output: Constraint { kind: "MC", spans: [("Customer was born in Country", 0)] }

// Combined (exactly one)
Input:  "Each Customer was born in exactly one Country."
Output: [Constraint { kind: "UC" }, Constraint { kind: "MC" }]

// Frequency constraint
Input:  "Each Customer submits at least 1 and at most 5 SupportRequest."
Output: Constraint { kind: "FC", min: 1, max: 5 }

// Derivation rule (iff)
Input:  "Person1 is an uncle of Person2 iff Person1 is a brother of some Person3 who is a parent of Person2."
Output: DerivationRule { kind: "join", consequent: "uncle", antecedents: ["brother", "parent"] }

// Derivation rule (attribute style)
Input:  "For each Person: uncle = brother of parent."
Output: DerivationRule { kind: "composition", def: Compose(Sel("brother"), Sel("parent")) }

// Aggregation
Input:  "For each PublishedBook, totalCopiesSold = sum(copiesSoldInYear)."
Output: DerivationRule { kind: "aggregate", func: "sum", field: "copiesSoldInYear" }

// State machine
Input:  "State Machine Definition 'Order' is for Noun 'Order'."
Output: StateMachine { name: "Order", noun: "Order" }

Input:  "Status 'Draft' is initial in State Machine Definition 'Order'."
Output: Initial { status: "Draft", machine: "Order" }

Input:  "Transition 'place' is from Status 'Draft'.\n  Transition 'place' is to Status 'Placed'."
Output: Transition { event: "place", from: "Draft", to: "Placed" }

// Deontic
Input:  "It is obligatory that each Customer has some Email."
Output: Constraint { kind: "MC", modality: "deontic", operator: "obligatory" }

Input:  "It is forbidden that the same Customer has more than one Email."
Output: Constraint { kind: "UC", modality: "deontic", operator: "forbidden" }
```

### Self-Hosting (syntax reads itself)

The final test: `readings/syntax.md` fed to the engine produces the same parser that was used to parse it.

```
// Load syntax.md via bootstrap kernel -> CompiledModel A
// Use CompiledModel A to parse syntax.md -> CompiledModel B
// Assert: A and B produce identical results on all test inputs
// This is the fixed-point test: the parser is a fixed point of itself
```

### End-to-End: Readings to HATEOAS

Full pipeline test matching the whitepaper's example (Section 4):

```
Input readings:
  Order(.OrderId) is an entity type.
  Customer(.Name) is an entity type.
  Order was placed by Customer.
    Each Order was placed by exactly one Customer.
  State Machine Definition 'Order' is for Noun 'Order'.
  Status 'Draft' is initial in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
    Transition 'place' is to Status 'Placed'.

Command: POST /orders {"customer":"acme"}

Expected result:
  Population' = Pop union {Order(ord-1), placedBy(ord-1, acme), SM(ord-1, Draft)}
  Response: { "id": "ord-1", "customer": "acme", "status": "Draft",
              "_links": { "place": { "href": "/orders/ord-1/place" } } }

Transition: POST /orders/ord-1/place {"event":"place"}

Expected result:
  Status updated to "Placed"
  Response includes _links: { "ship": ... }
  "place" link no longer present (not a valid transition from Placed)
```

## Success Criteria

1. `load_readings(markdown)` WASM export parses FORML 2 markdown and produces a CompiledModel
2. All existing domain readings (`readings/*.md`, `support.auto.dev/domains/*.md`) parse correctly
3. All existing constraint types compile to Func AST nodes (no Native escape hatch)
4. Derivation rules use FORML 2 syntax (iff/if/attribute style), not `:=`
5. TypeScript claims parser deleted entirely
6. All existing tests pass (or are updated to use new syntax)
7. GraphDL skill documents the full system
8. The syntax readings (`readings/syntax.md`) can parse themselves
