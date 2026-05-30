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

<!--
  Substrate-lift TODO (deletion plan for `compile_subtype_inheritance_metamodel`):

  The rule above is the FORML 2 form of what
  `crates/arest/src/compile.rs::compile_subtype_inheritance_metamodel`
  (~lines 6179-6249) synthesises procedurally. Once the parser learns
  to lift derivation antecedents that quantify over the METAMODEL cells
  (`Subtype`, `FactType`, `Role`, `Noun`) — not just user-declared FTs —
  the Rust synthesiser becomes pure ceremony and can be retired.

  Deletion plan (do not apply until prerequisites land):

  1. Parser prerequisite — `resolve_derivation_rule`
     (crates/arest/src/parse_forml2.rs) must recognise antecedents whose
     FT references are metamodel cells. Today these clauses fall through
     `// (1) Comparator-stripped FT lookup` and land as
     `UnresolvedClause` because Subtype/Role/FactType are not in the
     declared-FT catalog of a user reading set.

  2. Compiler prerequisite — `compile_explicit_derivation` must handle
     a `Literal(ft_id)` consequent whose ft_id is itself a binding from
     a metamodel-cell antecedent (today it expects ft_id to be a
     compile-time-known literal).

  3. Once (1) and (2) ship, this rule body parses into a single
     `CompiledDerivation` whose Func quantifies over
     `Subtype × FactType × Role × <Sub-instances>` directly. At that
     point delete `compile_subtype_inheritance_metamodel` (compile.rs
     ~6179-6249), the `SUBTYPE_INHERITANCE_ID` constant (~6257), the
     synthetic-id branch in `compile_to_defs_state` (~1907) that keys
     this id into every subtype's relevance set, and the call site at
     ~3935. The pin `crates/arest/tests/subtype_metamodel_rule_e2e.rs`
     stays — it verifies emission shape, not the lift mechanism.

  Cascading callers (BLOCKING — `compile_subtype_inheritance_metamodel`
  has non-synthesiser consumers, so the lift is option-6 "document and
  stop" until prerequisites (1) and (2) land):
    * `compile_to_defs_state` `derivation_index` synthetic-id fallback
      (compile.rs ~1737, ~1907) — needs to know "this id covers every
      subtype". Once the rule is parser-lifted to a normal
      CompiledDerivation, the index keys it from its bindings instead
      of the synthetic-id allowlist.
    * `compile_derivations` direct call (compile.rs ~3935).
-->

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

## Transitivity of binary Fact Types (eager materialisation removed — task-969)

Transitive composition of binary Fact Types is **not** eagerly
materialised and produces no synthetic closure facts.

An earlier implementation (#892) lifted the universal modus-ponens
schema for transitive composition — any two binary Fact Types whose
join nouns chain (Ft1's second role and Ft2's first role share a noun
J) compose into a fresh `_transitive_<Ft1>_<Ft2>` Fact Type pairing
Ft1's source binding with Ft2's destination binding — into a single
`_transitivity` metamodel `CompiledDerivation`. Its Func was
`Concat . [per-(Ft1, Ft2) inner Func, ...]`, and every forward-chain
round it materialised the all-pairs transitive composition of every
chaining binary Fact Type into `_transitive_<Ft1>_<Ft2>` cells.

It was removed 2026-05-29 as an unconsumed eager materialisation:
no consumer ever read those cells. State-machine transition validation
joins the explicit `Transition_is_from_Status` /
`Transition_is_to_Status` /
`Transition_is_defined_in_State_Machine_Definition` cells directly,
never a transitive closure; the `command.rs` SM-stratum gate's
`_transitive_Status` / `_transitive_Transition` substring matches were
dead (the only transitivity def id was `_transitivity`, which never
contains the substring `_transitive_`); and the TS/MCP layer has zero
references to `_transitive`. On the dominant repro it was the all-pairs
fanout (~10342 candidate facts per chain round) that drove the
metamodel-create cost. This mirrors the eager CWA-negation complement
below: an eager materialisation every consumer filtered out as noise.

The intentional absence is pinned by
`crates/arest/tests/transitivity_metamodel_rule_e2e.rs`
(`no_eager_transitive_closure_cell_is_materialised`), which asserts
that the Person → City → Country fixture the old rule fanned out over
produces NO `_transitive_*` cell after forward-chain.

(A user-authored transitive *ring constraint* — `TR`: `xRy ∧ yRz → xRz`,
violation when the shortcut is missing — is a separate, still-supported
feature compiled by `compile_ring_transitive_ast`; it enforces, it does
not eagerly materialise a closure.)

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

11. **Enum-declaration-order superlative** (task-953) — `<X> has
    derived <P> iff <X> <verb> some <Y> that has the <super> <P> among
    <Y>s the <X> <verb>` where `<super>` is a superlative word
    (`strongest`/`highest`/`best` or `weakest`/`lowest`/`worst`) and
    `<P>` is an ENUM-valued noun. Recognised by
    `try_parse_superlative_among_clause` in `parse_forml2.rs`; the
    superlative word maps to the existing `min`/`max` aggregate op via
    `SuperlativeComparatorTable` (`readings/forml2-grammar.md` enums
    `Superlative Comparator` / `Superlative Comparator Aggregate Op`),
    and the clause lifts to a `ConsequentAggregate` with `enum_rank =
    true`. The KEY INSIGHT: a superlative is the numeric min/max fold
    applied to a RANK derived from the value type's `enumerates
    'v0','v1',…` declaration order (first-declared = strongest = rank
    0) — there is NO new binary op and NO per-value cascade. The
    `among Ys the X …` group set is the join of the group FT
    (`X concerns Y`) with the value FT (`Y has P`) on the shared
    entity; `compile_aggregate_derivation` synthesises that join
    (`build_superlative_join_source`), promotes each candidate value to
    its declaration-order rank (`enum_rank_lookup`), folds `min`/`max`
    over the ranks, and projects the WINNING enum value (not the rank)
    onto the consequent. Pinned by
    `superlative_strongest_among_selects_enum_earliest_posture`,
    `superlative_highest_priority_among_selects_p0_over_p1`, and
    `superlative_weakest_among_selects_enum_latest_posture` in
    `compile_explicit_derivation_tests.rs`. OUT OF SCOPE (follow-up):
    domain-specific superlatives needing a non-enum ordering
    (`most-recent`/`fastest`/`cheapest` over dates/numbers) — those
    need an ordering source other than the enumerate declaration order.

### Comparator vocabulary the parser does NOT recognise

The `… among …` enum-declaration-order superlatives
(`strongest`/`weakest`, `highest`/`lowest`, `best`/`worst`) ARE now
recognised — see shape 11 above (task-953). The following words still
surface in user readings (Halpin §6 "sentence-level comparison") but
do NOT lift to any `AntecedentRoleComparison` or aggregate operator. A
rule whose antecedent contains any of these falls through every
classifier in `resolve_derivation_rule` and lands as an unresolved
clause (the parser emits an `UnresolvedClause` fact; the rule itself
stays in the schema with whatever positive FT antecedents survived):

* `most` / `least` (without `at`)
* `top` / `bottom`
* Any superlative adjective specific to a value-type's domain whose
  ordering is NOT the enum declaration order — `fastest`/`cheapest`
  over a numeric role, `most recent` over a date. The enum-ordered
  family is handled by shape 11; these need a different ordering
  source (a numeric/date role, not `enumerates …`) and are the
  documented task-953 follow-up.

The chainer treats these as opaque tokens — the rule body's FT
references that DO resolve still emit derivations, and the
unrecognised comparator clause is silently dropped via the
`unresolved_clauses` channel. The behaviour the user observes is
"attribute inheritance from the last bound antecedent fact": the
remaining positive antecedents fire, the consequent inherits
bindings from the last antecedent that carries the consequent's
role, and the rule appears to ignore the comparator clause entirely.

### How the enum-superlative gap was closed (task-953, approach 4)

An earlier draft documented a `has no`-negation 2-level priority
cascade as a workaround for ordering/superlative semantics. That
pattern relied on the parser converting `has no` / `is not` clauses
into `AntecedentSource::AbsenceOf` antecedents and a negation-guarded
stratum in the chainer. Both were removed 2026-05-19 (negation is not
an antecedent kind — see the CWA section and the NORMA model it
cites), so the cascade no longer parses.

task-953 closed the gap for enum-ordered superlatives WITHOUT a
cascade synthesiser (approach 3) or a new `Func` (approach 2), per the
owner directive to prefer FFP forms over procedural Rust. The chosen
approach (4) is an FFP fold: a superlative is the EXISTING numeric
min/max aggregate applied to a RANK promoted from the value type's
`enumerates …` declaration order. The three structural requirements
the task-814 audit flagged (below) were met thus:

1. **Ordering metadata on the value type** — sourced directly from the
   existing `EnumValues` cell (already in `CellIndex::enum_values`),
   read in declaration order. No separate ordering FT is authored; the
   acceptance reading declares none. (The audit's worry that the order
   "is semantic but not promoted" was unfounded — `enum_rank_lookup`
   reads the declaration order at compile time.)

2. **An aggregate operator over ordering** — NOT a new binary op. The
   enum value is rank-PROMOTED to a numeric atom
   (`enum_rank_lookup`, a compile-time-unrolled nested conditional),
   then the EXISTING `min`/`max` Insert-fold runs over the ranks. This
   sidesteps the audit's "doesn't compose" objection (which was about
   plugging a primitive op into the aggregate slot): decoupling via
   rank-promotion-then-numeric-fold means the fold never sees an enum.

3. **NO per-value priority-cascade synthesiser** — rejected. The
   author's reading stays the natural-language superlative form;
   compilation lifts it to one rank-aggregate, not N cascade rules.

The historical "structural requirements" framing is retained below for
provenance; items 1–2 are now satisfied by rank-promotion and item 3
was the rejected alternative.

### Why the comparator gap wasn't trivial to close (historical, task-814 audit)

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
workaround now, defer to a follow-up task once a second use site
(besides Merge's Security Posture) materialised. task-953 picked it up
with the second use site (Task Priority) and chose NEITHER (b)'s
cascade NOR the binary-op of item 2 — it added rank-promotion (item 1
sourced from `EnumValues`) plus a join, feeding the EXISTING numeric
fold. See "How the enum-superlative gap was closed" above.

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
  Codd image-set fold over numeric role values; task-953 added the
  `enum_rank` branch (rank-promote enum values, fold min/max over the
  ranks, project the winning value) and the join source.
* `crates/arest/src/parse_forml2.rs::try_parse_superlative_among_clause`
  — recogniser for shape 11 (`<X> has the <super> <P> among <Y>s …`);
  routes to a `ConsequentAggregate { enum_rank: true }` (task-953).
* `crates/arest/src/parse_forml2_stage2.rs::SuperlativeComparatorTable`
  — superlative-word → `min`/`max` mapping; boot table mirrors the
  `Superlative Comparator` / `Superlative Comparator Aggregate Op`
  grammar enums (task-953).
* `crates/arest/src/compile.rs::enum_rank_lookup` /
  `build_superlative_join_source` — rank-promotion (value → declaration-
  order index) and the group-FT ⋈ value-FT join (task-953).
* `crates/arest/src/evaluate.rs::prove_from_state` — lazy CWA/OWA
  negation: a goal absent from the population is Disproven (Closed) or
  Unknown (Open), per whitepaper §305.
