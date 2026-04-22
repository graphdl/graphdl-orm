# Parser Anaphora + Subscript + Metamodel-FT Push (#317)

Follow-up to #316 (implicit derivations as readings). #316 landed the
four implicit derivation kinds as FORML 2 rules in `readings/core.md`.
Their bodies reference metamodel fact types (`Noun is subtype of
Noun`, `Noun plays Role`, `Fact uses Resource for Role`) and use
anaphora + subscripts throughout. This document captures what the
parser still needs to do so those rule bodies resolve to concrete
antecedent sources the compiler can chain at runtime — at which
point `compile_derivations`' Rust synthesis for subtype inheritance,
CWA negation, SS auto-fill, and binary transitivity can retire.

## What's already working

- **`that <Noun>` anaphora expansion**
  (`crates/arest/src/parse_forml2.rs::expand_that_relatives`): rewrites
  `X has Y and that Y has Z` into `X has Y and Y has Z` so
  downstream classifiers see two clean binary FT references. Skips
  when the head doesn't resolve to a declared FT — prevents noisy
  diagnostics on compound nouns like `Billable Request is for
  Customer and Meter Endpoint`.

- **Subscripted noun references in conditional ring shapes**
  (`parse_forml2_stage2.rs::conditional_ring_kind`): `Noun1`, `Noun2`,
  `Noun3` all resolve to bare `Noun` via the trailing-digit strip.
  `is_that_anaphora_ref` similarly treats `Person3` as a reference
  to `Person`.

- **Digit-subscript strip in constraint span resolution** (#326 via
  `resolve_constraint_span_ft`): "If Noun1 is subtype of Noun2 and
  Noun2 is subtype of Noun3, then Noun1 is subtype of Noun3." parses
  and lands its ring span on `Noun is subtype of Noun` (the
  self-referential binary) rather than the first Noun-bearing FT.

- **"itself" pronoun expansion** (#326): "No X R itself." treats
  `itself` as a repeat of the preceding noun so the span resolves to
  the `X R X` self-referential binary.

## What's still missing

### 1. Metamodel-FT cell push

The implicit derivation rules from #316 have bodies like:

```
Resource is inherited instance of Noun iff Resource is instance of
some subtype of that Noun.
```

`Resource is instance of Noun` and `Noun is subtype of Noun` are
declared metamodel FTs, but the parser currently emits an
`UnresolvedClause` for each antecedent clause referencing them
because the classifier looks for direct Noun-Verb-Noun matches,
not cross-cell joins where one antecedent reads a metamodel cell
(`Subtype`) to bind a variable that another antecedent uses
(`Resource is instance of …`).

**Fix shape:** extend `classify_antecedent_clause` (or the
equivalent stage-2 pass) so that a clause whose noun sequence
matches a declared metamodel FT produces an `AntecedentSource::
FactType(ft_id)` with no role-to-role join keys — the runtime
reads the FT's cell as a full population, same way user-FT
antecedents work today. Metamodel FTs get no special handling;
they're just FTs whose cells happen to be populated at compile
time rather than during user-data ingestion.

### 2. Cross-clause role binding (anaphora over metamodel joins)

For the subtype rule above, the consequent's `Noun` refers to the
outer noun in the antecedent's `subtype of that Noun`. The parser
needs to:

1. Identify `that Noun` as an anaphora binding to the outer
   `Noun` in `is instance of some subtype of that Noun`.
2. Emit an antecedent source + role binding that ties the outer
   `Noun` role of the consequent to the outer `Noun` role of the
   antecedent's `is subtype of` join.
3. Produce a `ConsequentBinding { role, source_antecedent_role }`
   so `forward_chain_defs_state` knows where to read the value
   at emission time.

The existing `DerivationRuleDef::consequent_bindings` field
already models this; the missing work is the parser-side
recognition of the cross-clause bind when the antecedent carries
more than one Noun occurrence and the consequent borrows one
back via `that Noun`.

### 3. Subscripted noun-role binding for conditional shapes

Conditional ring shapes like
`If Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3, then
Noun1 is subtype of Noun3.` currently resolve their kind (TR) and
span-FT correctly (#326) but still leave the per-role bindings
implicit. To derive new facts (not just classify the shape), the
parser needs to bind:

- `Noun1` → `role 0` of the first antecedent FT *and* `role 0`
  of the consequent FT
- `Noun3` → `role 1` of the second antecedent FT *and* `role 1`
  of the consequent FT
- `Noun2` as a join key between antecedents (role 1 of first,
  role 0 of second)

`DerivationRuleDef::join_on` models the middle-noun join today.
The parser needs to populate it from the subscript-resolved
graph.

### 4. Pronoun chains

`some subtype of that Noun` compounds two anaphora references in a
single clause. The parser's current "single `that <Noun>` rewrite"
pass (`expand_that_relatives`) only handles the simple form
`X and that X`. Nested chains need another pass — or a unified
anaphora-resolution stage that walks the clause graph and binds
every `that <Noun>` / `that <Noun_n>` reference to its nearest
preceding occurrence.

## Acceptance

When #317 lands, the four implicit derivation rules in `readings/
core.md::## Implicit Derivation Rules (#316 / #287c)` parse into
`DerivationRuleDef`s with:

- non-empty `antecedent_sources` (no `UnresolvedClause` entries)
- populated `join_on` where the antecedent has >1 clause
- populated `consequent_bindings` tying each consequent role back
  to its source antecedent role

and `compile_derivations`' `compile_subtype_inheritance` /
`compile_cwa_negation` / `compile_ss_autofill` /
`compile_transitivity` synthesis passes can be deleted — the
readings drive the runtime directly via `compile_explicit_derivation`.

## Staging

Land in this order to minimise churn:

1. Metamodel-FT push — smallest, unblocks the rest. Touches
   `classify_antecedent_clause` and adds a test that the four #316
   rules produce `antecedent_sources.len() == <expected>` for each.
2. Pronoun chains — extend `expand_that_relatives` to iterate over
   nested `that X` references in one clause.
3. Subscripted role binding — populate `join_on` from
   `conditional_ring_kind`'s subscript-resolved graph.
4. Delete the four `compile_*_synthesis` passes once tests confirm
   the readings-driven path produces identical `DerivationRuleDef`s
   for the bundled metamodel.

Each step is one commit with a focused regression test. The
`bundled_metamodel_passes_validate_model` test in `tests/
properties.rs` stays green throughout.
