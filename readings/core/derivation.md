# Derivation Rule Metamodel (#890)

This file holds the FORML 2 declarative form of the structural
derivation rules that AREST's compiler synthesises during
`compile_to_defs_state`. Each rule is the universal-modus-ponens
schema from whitepaper §5.2 written against the metamodel cells
(`Subtype`, `FactType`, `Role`, `Noun`). (Closed-world negation is
NOT one of these rules — it is an evaluation-time semantics; see the
CWA section below.)

The compiler's `compile_subtype_inheritance_metamodel` (and the
parallel `compile_derivations` paths for SS auto-fill, transitivity,
SM init) lifts each rule into ONE `CompiledDerivation`
whose Func is the union of the per-binding inner Funcs the rule
quantifies over. The forward chainer fires that Func at evaluation
time exactly as it would fire any user-authored derivation, and the
emitted `<ft_id, reading, bindings>` tuples land in the per-FT cells
the user expects.

## Subtype inheritance (#890 — replaces the per-(sub, sup, ft) Rust loop)

Whitepaper §5.2 universal modus-ponens schema for subtype
membership: every Resource that is an instance of a Subtype is also
an instance of its Supertype, in every Fact Type where the
Supertype plays a Role.

* Fact Type has inherited Resource at Role
    iff some Subtype has subtype Sub and that Subtype has supertype Sup
    and that Fact Type has that Role and that Role is played by Sup
    and that Resource is instance of Sub.

The rule's antecedent quantifies over the four metamodel cells
`Subtype × FactType × Role × <Sub-instances>`; its consequent is the
synthesized `<<Sup-role, Resource>>` binding pushed into every FT
cell where Sup plays the Role. `compile_subtype_inheritance_metamodel`
in `crates/arest/src/compile.rs` performs the lift to a Func:

  Concat . [
    per-(Sub, Sup, FT) inner Func,
    ...
  ]

where each inner Func is `Concat . (apply_to_all per_instance .
instances_of_noun_func(Sub))` — the byte-for-byte same shape
`compile_explicit_derivation` produces for a 1-antecedent
`InstancesOfNoun` rule with `Literal(FT_id)` consequent. Behavioural
equivalence with the pre-#890 per-pair fanout is pinned by
`crates/arest/tests/subtype_metamodel_rule_e2e.rs`.

## SS Subset-Constraint auto-fill (#891 — replaces the per-SS-Constraint Rust loop)

Whitepaper §5.2 universal modus-ponens schema for Subset Constraint
auto-fill: every Fact in the antecedent Fact Type is also a Fact in
the consequent Fact Type, whenever the Subset Constraint's
antecedent span carries the `subset_autofill = true` marker.

* Fact Type has auto-filled Fact
    iff some Subset Constraint has antecedent Fact Type Ant and that
    Subset Constraint has consequent Fact Type Cons and that Subset
    Constraint has autofill 'true' and that Fact is instance of Ant
    and that Fact Type is Cons.

The rule's antecedent quantifies over `Subset-Constraint ×
antecedent-FT-fact` cells; its consequent is the same fact pushed
into the consequent FT cell. `compile_ss_autofill_metamodel` in
`crates/arest/src/compile.rs` performs the lift to a Func:

  Concat . [
    per-SS-Constraint inner Func,
    ...
  ]

where each inner Func is the byte-for-byte same shape
`compile_explicit_derivation` produces for a 1-antecedent
`FactType(antecedent_ft)` rule with `Literal(consequent_ft)`
consequent. Behavioural equivalence with the pre-#891 per-SS-
Constraint fanout is pinned by
`crates/arest/tests/ss_autofill_metamodel_rule_e2e.rs`.

## Transitivity of binary Fact Types (#892 — replaces the per-(ft1, ft2) Rust loop)

Whitepaper §5.2 universal modus-ponens schema for transitive
composition: any two binary Fact Types whose join nouns chain
(ft1's second role and ft2's first role share a noun) compose into
a fresh transitive Fact Type whose facts pair ft1's source-role
binding with ft2's destination-role binding.

* Fact Type has inferred transitive Fact
    iff some Fact Type Ft1 has Role at position 1 played by Noun J
    and some Fact Type Ft2 has Role at position 0 played by Noun J
    and that Ft1 has Source Noun S at position 0
    and that Ft2 has Destination Noun D at position 1
    and some Fact F1 in Ft1 binds Resource X at Source S and Resource Y at Join J
    and some Fact F2 in Ft2 binds Resource Y at Join J and Resource Z at Destination D
    and that Fact Type is `_transitive_<Ft1>_<Ft2>`
    and that Fact has Source S at Resource X and Destination D at Resource Z.

The rule's antecedent quantifies over `(FactType × FactType) ×
<Ft1-Fact × Ft2-Fact>` cells gated by the shared-join-noun
condition; its consequent is the synthesized `<<S, X>, <D, Z>>`
binding pushed into every fresh `_transitive_<Ft1>_<Ft2>` cell.
`compile_transitivity_metamodel` in `crates/arest/src/compile.rs`
performs the lift to a Func:

  Concat . [
    per-(Ft1, Ft2) inner Func,
    ...
  ]

where each inner Func is the byte-for-byte same shape
`compile_join_derivation` produces for a 2-antecedent
`[FactType(Ft1), FactType(Ft2)]` rule with
`Literal("_transitive_<Ft1>_<Ft2>")` consequent,
`join_on = [shared_noun]`, and
`consequent_bindings = [src_noun, dst_noun]`. Behavioural
equivalence with the pre-#892 per-pair fanout is pinned by
`crates/arest/tests/transitivity_metamodel_rule_e2e.rs`. The
consequent cell name `_transitive_<Ft1>_<Ft2>` is preserved so
SM-infrastructure gates in `command.rs` (which key off
`_transitive_Status` / `_transitive_Transition`) keep working.

## CWA negation (whitepaper §305 — an evaluation-time semantics, not a materialised rule)

The closed-world assumption is **not** a derivation rule and produces
no synthetic facts. Per whitepaper §305 (citing Halpin ch.7), the
world assumption is a property of each Noun that governs how absence
is interpreted at evaluation time:

> under CWA, a fact not in the population $P$ is false; under OWA, it
> is unknown.

This is realised lazily by `evaluate::prove_from_state` in
`crates/arest/src/compile.rs`'s sibling `evaluate.rs`: when a negation
is queried, the prover searches $P$ for a matching fact; on a miss it
returns `Disproven` if the queried Noun is `WorldAssumption::Closed`
and `Unknown` if `WorldAssumption::Open`. There is no eager complement
population — no `_cwa_negation:<ft>` cells, no `AbsenceOf` antecedent,
no negation-guarded stratum. (An earlier implementation materialised
the complement of every closed-world Noun across every Fact Type; it
was removed 2026-05-19 as redundant with `prove_from_state`,
non-faithful to §305's lazy semantics, and unconsumed — every cell
consumer filtered the `_cwa_negation:` cells out as noise.) Behaviour
is pinned by `evaluate::tests::test_cwa_vs_owa_negation`.

NORMA, the ORM2 reference implementation, models negation the same
way: it is never a standalone "absence" antecedent. Negation is a
flag on a node of an otherwise-positive role path (`IsNegated`), and
closed-world completeness is a property of the *derived* Fact Type
(`DerivationCompleteness = FullyDerived`). AREST follows this: the
per-Noun world assumption is the CWA; should negated readings be
needed in a future version, the faithful representation is a negation
flag on a positive antecedent, not a new antecedent kind.

## Other structural rules (deferred — still synthesised in compile.rs)

Three of #287/#311's structural-rule lifts (subtype inheritance,
SS auto-fill, transitivity) are expressed as declarative metamodel
rules in this file. Future structural-rule lifts that need a similar
treatment should follow the same shape:
the rule body above + a `compile_<name>_metamodel(data)` lift in
`compile.rs` + a `<name>_metamodel_rule_e2e.rs` acceptance pin.

The original "implicit derivation" framing of subtype inheritance
in `readings/core/core.md` §332 (`Resource is inherited instance of
Noun iff Resource is instance of some subtype of that Noun`) is the
older, looser shape that doesn't address per-FT consequent
materialisation. The rule above is the operational form #890
needs — it spells out the consequent FT and Role explicitly so the
compiler can lift it without guessing.

## Authoring derivation rules: supported antecedent shapes (task-814)

What follows is a catalogue of derivation-rule antecedent shapes the
parser currently recognises, plus the comparator vocabulary it does
NOT recognise. Each shape cites the recogniser in
`crates/arest/src/parse_forml2.rs::resolve_derivation_rule` and the
forward-chain lower-bound in
`crates/arest/src/compile.rs::compile_explicit_derivation` (or
`compile_join_derivation` / `compile_aggregate_derivation` when the
classifier routes elsewhere). Author against these shapes; rewrite
unrecognised vocabulary via the priority-cascade pattern at the
bottom of this section.

### Shapes the parser recognises

1. **Positive FT reference** — `X has Y` resolves to a declared
   `X has Y` FT and lands as `AntecedentSource::FactType(ft_id)`.
   This is the default fallthrough at site (1) of the cascade in
   `resolve_derivation_rule` (search `// (1) Comparator-stripped FT
   lookup`). Cited as `shape_literal_in_consequent_pins_role_to_atom`
   in `compile_explicit_derivation_tests.rs`.

2. **Numeric comparator on a role** — `X has Y >= 100` strips to
   `X has Y` plus an `AntecedentFilter { op: ">=", value: 100 }` via
   `split_antecedent_comparator` (Halpin FORML Example 5). Operators
   accepted: `>=`, `<=`, `>`, `<`, `=`, `!=`, `<>` (normalised to
   `!=`). See `peel_trailing_comparator` in `parse_forml2.rs`.

3. **Word-form numeric comparator** — `X exceeds 100`,
   `X is greater than Y`, etc. The phrases live in
   `parse_forml2_stage2::WordComparatorTable` (8 entries: `exceeds`,
   `is greater than`, `is less than`, `is at least`, `is at most`,
   `is more than`, `equals`, `is equal to`). Cross-antecedent
   role-vs-role comparisons (`X1's Y exceeds X2's Y`) lift to
   `AntecedentRoleComparison` via the pre-pass at site (1) of
   `resolve_derivation_rule`; the rule is promoted to `Join` kind
   and routes through `compile_join_derivation`.

4. **Literal pin on a role** — `X has Y 'present'` resolves to the
   `X has Y` FT plus an `AntecedentRoleLiteral { role: "Y", value:
   "present" }` (cited as
   `shape_some_quantifier_with_multi_word_literal_filters_antecedent`).
   Multi-word literals (`'In Code Only'`) survive intact.

5. **Existential quantifier on a role** — `<consequent> iff some X
   has Y 'present'`. The ` some ` token is whole-word-stripped by
   `parse_forml2_stage2::ExistentialQuantifierTable::strip` (see
   `strip_existential_quantifiers` in `parse_forml2.rs`). What
   remains (`X has Y 'present'`) parses as shape 4. The single
   antecedent fans out one derived fact per matching antecedent
   fact — pinned by
   `paper_lift_priority_derivation_fires_through_forward_chain`.

6. **Existential-over-join** — `<consequent> iff X concerns some Y
   that has Z 'present'`. The ` some ` is stripped, then the
   `that`-relative is expanded by `expand_that_relatives` (using
   `parse_forml2_stage2::AnaphoraPronounTable`) into the two-clause
   form `X concerns Y and Y has Z 'present'`. Both clauses must
   resolve to declared FTs OR the expansion is skipped (see
   `head_resolves` in `parse_forml2.rs`). Because `expand_that_relatives`
   CONSUMES the `that`, no anaphora survives, so `join_on` is empty and
   the rule is NOT promoted to `Join` kind — it routes through
   `compile_explicit_derivation`'s multi-antecedent existential-over-join
   path, NOT `compile_join_derivation`. Pinned by
   `shape_existential_over_join_fans_out_per_x`.

7. **Numeric aggregation** — `X has Count iff Count is the count of Y
   where X has Y`. The clause shape `<role> is the <op> of <target>
   where <body>` lifts to `consequent_aggregates`; routes through
   `compile_aggregate_derivation`. Operators accepted (see
   `try_parse_aggregate_clause` in `parse_forml2.rs::AGG_OPS`):
   `count`, `sum`, `avg`, `min`, `max`, `earliest`, `latest`,
   `first`, `last`. `min` and `max` fold over NUMERIC role values
   only — they are NOT a comparator over enum-valued nouns.

8. **Arithmetic-definitional binding** — `Volume is Size * Size *
   Size`. The clause shape `<RoleName> is <expr>` (where RoleName is
   a declared noun and `<expr>` parses through `parse_arithmetic_expr`
   over `+ - * /`) populates `consequent_computed_bindings`. Used by
   `compile_explicit_derivation`'s 1-antecedent path to project a
   computed value into the consequent fact.

9. **Subtype membership check** — `X is a Y` / `X is an Y` (both X
    and Y declared nouns) recognised by `is_subtype_instance_check`
    in `parse_forml2.rs`. Doesn't emit an antecedent source — the
    subtype relationship is structural and handled by the metamodel
    rule earlier in this file.

10. **Temporal predicate** — `now is in the past`, `… in the past`,
    `… in the future` recognised by `TemporalPredicateTable`.
    Runtime clock checks; no FT resolution.

### Comparator vocabulary the parser does NOT recognise (task-814)

The following words are surfaced in user readings (Halpin §6
"sentence-level comparison") but do NOT lift to any
`AntecedentRoleComparison` or aggregate operator today. A rule whose
antecedent contains any of these falls through every classifier in
`resolve_derivation_rule` and lands as an unresolved clause (the
parser emits an `UnresolvedClause` fact; the rule itself stays in
the schema with whatever positive FT antecedents survived):

* `strongest` / `weakest`
* `highest` / `lowest` (only the aggregate forms with
  `is the highest of` / `is the lowest of` work — and only over
  numeric roles)
* `best` / `worst`
* `most` / `least` (without `at`)
* `top` / `bottom`
* Any superlative adjective specific to a value-type's domain
  (`fastest`, `cheapest`, `safest`, `most recent`)

The chainer treats these as opaque tokens — the rule body's FT
references that DO resolve still emit derivations, and the
unrecognised comparator clause is silently dropped via the
`unresolved_clauses` channel. The behaviour the user observes is
"attribute inheritance from the last bound antecedent fact": the
remaining positive antecedents fire, the consequent inherits
bindings from the last antecedent that carries the consequent's
role, and the rule appears to ignore the comparator clause entirely.

### Priority/superlative semantics: an open authoring gap

An earlier draft documented a `has no`-negation 2-level priority
cascade as a workaround for ordering/superlative semantics. That
pattern relied on the parser converting `has no` / `is not` clauses
into `AntecedentSource::AbsenceOf` antecedents and a negation-guarded
stratum in the chainer. Both were removed 2026-05-19 (negation is not
an antecedent kind — see the CWA section and the NORMA model it
cites), so the cascade no longer parses. Ordering/superlative
semantics over enum-valued nouns therefore remain an open authoring
gap; the structural requirements for closing it faithfully are below.

### Why the comparator gap isn't trivial to close

A faithful `strongest Security Posture among Commits the Merge
concerns` lowering would need:

1. **Ordering metadata on the value type.** Today
   `EnumValues` in the grammar cell carries enum members in
   declaration order. The order IS semantic for stratification
   purposes (the priority cascade above relies on the author
   listing strongest-first), but the parser does not promote
   declaration order into an ordering fact. A
   `Security Posture has Strength` ordering FT would have to be
   author-declared (per #890's "explicit metamodel rule" pattern)
   before the engine could pick a winner by ordering.

2. **A new aggregate operator over ordering.** Codd image-set
   aggregates (`compile_aggregate_derivation`) fold pairs via a
   binary `op`, which works for sum/min/max over numbers because
   `Func::Add` / `Func::Lt` / `Func::Gt` evaluate on raw atoms.
   For a `strongest` operator over `Security Posture`, the binary
   op would need to look up each candidate's ordering position
   in the ordering FT — a non-primitive `Func` that doesn't
   compose from the current set.

3. **OR: a per-value priority-cascade synthesiser.** A new lift
   in `compile.rs` could read a `Security Posture has Strength`
   ordering FT and SYNTHESISE the N rules of the cascade above
   in `O(N)` of value types — same recipe as
   `compile_subtype_inheritance_metamodel`. The author's source
   reading stays the natural-language comparator form;
   compilation expands it.

The audit at task-814 settled on option (b): document the
workaround now, defer the synthesiser to a follow-up task once a
second use site (besides Merge's Security Posture) materialises.

### Audit anchor — files cited above

* `crates/arest/src/parse_forml2.rs::resolve_derivation_rule` —
  central antecedent classifier (cascade sites (1)–(12)).
* `crates/arest/src/parse_forml2.rs::try_parse_aggregate_clause` —
  accepted aggregate ops list (`AGG_OPS` const).
* `crates/arest/src/parse_forml2.rs::strip_existential_quantifiers`
  / `expand_that_relatives` — the existential and `that`-anaphora
  pre-processors that drive shapes 5 and 6.
* `crates/arest/src/parse_forml2.rs:1932-1947` — `is_join`
  classifier; the noun-shared-in-2+-antecedent test that promotes
  a rule to `kind = Join`.
* `crates/arest/src/parse_forml2_stage2.rs::WordComparatorTable` —
  the 8 word-form numeric comparators (#783 lift).
* `crates/arest/src/parse_forml2_stage2.rs::ExistentialQuantifierTable`
  — the ` some ` / ` that ` quantifier-strip vocabulary (#883 lift).
* `crates/arest/src/parse_forml2_stage2.rs::AnaphoraPronounTable`
  — the ` that ` anaphora marker (#882 lift); drives the
  relative-clause expansion in `expand_that_relatives`.
* `crates/arest/src/compile.rs::compile_explicit_derivation` —
  1-antecedent fanout and the multi-antecedent equi-join / existence-
  check paths.
* `crates/arest/src/compile.rs::compile_join_derivation` —
  2+-antecedent equi-join (path used by `kind = Join`).
* `crates/arest/src/compile.rs::compile_aggregate_derivation` —
  Codd image-set fold over numeric role values.
* `crates/arest/src/evaluate.rs::prove_from_state` — lazy CWA/OWA
  negation: a goal absent from the population is Disproven (Closed) or
  Unknown (Open), per whitepaper §305.
