# Stage-1 Tokenization Rules (#295)

The classification rules in `readings/forml2-grammar.md` operate on
`Statement` cells that are already populated with fields like
`Head Noun`, `Verb`, and `Quantifier`. The rules *below* specify how
Stage-1 should extract those fields from raw statement text — today
they are implemented in Rust (`crates/arest/src/parse_forml2_stage1.rs`);
moving them into readings closes the bootstrap gap once the FFP
text-pattern primitives land (#282).

Until then, these rules are **normative documentation**: the Rust
tokenizer must produce the same cells the rules below describe, and
any behavioural change in one side requires a matching change in the
other.

Lives in `docs/` (not `readings/`) so the parser doesn't try to parse
it as live grammar — adding prose to `forml2-grammar.md` breaks the
metamodel.

## Input

Stage-1 takes:

- a `Text` — the raw statement line, trailing period stripped; and
- the set of declared `Noun` names — matched longest-first so
  multi-word names (`Support Response`) beat their suffixes (`Response`).

## Tokenization rules

Each rule below populates one `Statement` field by inspecting `Text`
and the declared-noun set. The Rust implementation in
`parse_forml2_stage1::tokenize_statement` is the canonical
reference.

### Head Noun

`Statement has Head Noun` iff `Text` starts with some declared Noun
name and the first token after that name is a Verb (any word not
in the reserved keyword set).

### Verb

`Statement has Verb` iff `Text` contains a phrase between two Role
References that matches no reserved keyword and is not a quantifier.
For unary fact types (`Noun is abstract.`) the Verb is the entire
trailing predicate.

### Trailing Marker

`Statement has Trailing Marker <marker>` iff `Text` ends with one of:

- `is an entity type`
- `is a value type`
- `is abstract`
- `is irreflexive`, `is asymmetric`, `is antisymmetric`, `is symmetric`
- `is intransitive`, `is transitive`, `is acyclic`, `is reflexive`
- `are mutually exclusive`

The marker is stripped from the Text before further field extraction
so the remainder parses as a clean FT reading.

### Quantifier

`Statement has Quantifier <q>` iff `Text` contains one of the
declared quantifier values as a whole-word leading token of any
Role Reference:

`each`, `some`, `no`, `at most one`, `at least one`, `exactly one`,
`at most`, `at least`.

Multi-word quantifiers (`at most one`) match before their single-word
subsets (`at most`).

### Keyword (derivation shape)

`Statement has Keyword <k>` iff `Text` contains one of `iff`, `if`,
`when` as a whole-word token splitting the Text into an antecedent
and consequent. `iff` takes precedence over `if`; `when` only matches
when neither of the other two are present.

### Constraint Keyword (multi-clause shape)

`Statement has Constraint Keyword <k>` iff `Text` contains one of:

- `if and only if` (Equality Constraint)
- `at most one of the following holds` (Exclusion Constraint)
- `exactly one of the following holds` (Exclusive-Or Constraint)
- `at least one of the following holds` (Or Constraint)
- `if some then that` (Subset Constraint)

### Deontic Operator

`Statement has Deontic Operator <op>` iff `Text` starts with
`It is <op> that` where `<op>` is one of `obligatory`, `forbidden`,
`permitted`. The prefix (including trailing `that `) is stripped
before further parsing.

### Derivation Marker

`Statement has Derivation Marker <m>` iff `Text` begins with a marker
prefix that indicates derivation storage:

- `*` prefix → `fully-derived`
- `**` prefix → `semi-derived`
- `+` prefix → `derived-and-stored`

### Enum Values Declaration

`Statement has Enum Value <v>` iff `Text` starts with
`The possible values of <Noun> are` and contains a
comma-separated list of single-quoted literal values. Each literal
becomes one `Enum Value` fact.

## Role Reference extraction

### Role Reference

`Statement has Role Reference` iff some declared Noun name appears in
`Text` outside a quoted literal, excluding the primary Head Noun.
Role References are emitted in textual order.

### Role Position

`Role Reference has Role Position <n>` where `<n>` is the zero-based
textual index of the Role Reference within the Statement. For
`Support Response uses Support Channel`, the `Support Response`
reference has position `0` and `Support Channel` has position `1`.

### Literal Value

`Role Reference has Literal Value <v>` iff the Role Reference is
surrounded by single quotes in `Text`; `<v>` is the content between
the quotes. A Role Reference with a Literal Value maps to an
`Instance Fact` classification at Stage-2.

## Reserved-keyword check (Theorem 1, no-substring)

A declared Noun name must not contain any of the tokenization
keywords as a whole word. The check is case-insensitive and
word-bounded — `Each Way Bet` collides on `each` but `Teacher` does
not, because `each` is not a whole-word token inside `Teacher`.

`Noun has reserved-keyword collision <k>` iff some keyword `<k>`
appears as a whole-word token of that Noun's name.

Keyword set: every Quantifier, every Constraint Keyword, every
Deontic Operator, plus `iff`, `if`, `when`, `is`. Multi-word
keywords match first so `at most one` beats `at most` when both
would apply.

### Escape hatch

A Noun declared with a single-quoted name skips the reserved-keyword
check and the surrounding quotes are stripped from the canonical
name at Stage-2:

```
Noun 'Each Way Bet' is an entity type.
```

This matches `parse_forml2_stage1::reserved_keyword_in`'s documented
contract at `docs/02-writing-readings.md` §"Noun names and reserved
words".

## Migrating to readings (#282 + follow-up)

Once FFP text-pattern primitives land (`Contains`, `StartsWith`,
`EndsWith`, `Regex`, …) each rule above becomes an executable FORML 2
derivation rule in `readings/forml2-grammar.md` under a new
`## Stage-1 Tokenization` section. The Rust tokenizer then retires —
Stage-0 bootstrap produces just enough of the Statement cell for
those derivation rules to fire, and the rules populate every other
field via the metamodel fixpoint.

Until that lands, this document is the spec the Rust implementation
must match.
