# Derivation Rule Metamodel (#890)

This file holds the FORML 2 declarative form of the structural
derivation rules that AREST's compiler synthesises during
`compile_to_defs_state`. Each rule is the universal-modus-ponens or
universal-CWA schema from whitepaper §5.2 written against the
metamodel cells (`Subtype`, `FactType`, `Role`, `Noun`).

The compiler's `compile_subtype_inheritance_metamodel` (and the
parallel `compile_derivations` paths for SS auto-fill, transitivity,
CWA negation, SM init) lifts each rule into ONE `CompiledDerivation`
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

## CWA negation (#893 — replaces the per-(CWA-noun, FT, role) Rust loop)

Whitepaper §5.2 universal CWA-negation schema: every instance of a
closed-world Noun that does NOT participate in a Fact Type at a
role played by that Noun is in the complement of that Fact Type at
that role. Open-world Nouns are skipped — absence of evidence is
not evidence of absence under OWA.

* Fact Type has CWA-complement Resource
    iff some Noun has World Assumption 'CWA'
    and some Fact Type has Role at position P played by that Noun
    and that Resource is instance of that Noun
    and no Fact of that Fact Type binds Resource at that Role
    and that Fact is `<<_neg_<Noun>, Resource>>`
    and that Fact lives in the synthetic cell `_cwa_negation:<Fact Type id>`.

The rule's antecedent quantifies over `Noun × FactType × Role ×
<Noun-instances>` cells gated by `Noun.world_assumption = Closed`
and the role-membership condition `Role.noun_name = Noun.name`; its
consequent is the synthesized `<<_neg_<Noun>, Resource>>` binding
pushed into every fresh `_cwa_negation:<FT id>` cell when no Fact
of `FT id` binds Resource at the Noun's role.
`compile_cwa_negation_metamodel` in `crates/arest/src/compile.rs`
performs the lift to a Func:

  Concat . [
    per-(CWA-Noun, FT, Role) inner Func,
    ...
  ]

where each inner Func is the byte-for-byte same shape
`compile_explicit_derivation` produces for an `InstancesOfNoun` +
`AbsenceOf` 2-antecedent rule with
`Literal("_cwa_negation:<FT id>")` consequent and
`consequent_instance_role = "_neg_<Noun>"`. Behavioural equivalence
with the pre-#893 per-triple fanout is pinned by
`crates/arest/tests/cwa_negation_metamodel_rule_e2e.rs`. The
consequent cell name `_cwa_negation:<FT id>` is preserved so
SM-infrastructure gates in `command.rs` and the
`evaluate::derivation_defs_from` consumer (which keys off the
`_cwa_negation:` cell prefix) keep working.

## Other structural rules (deferred — still synthesised in compile.rs)

All four of #287/#311's structural-rule lifts (subtype inheritance,
SS auto-fill, transitivity, CWA negation) are now expressed as
declarative metamodel rules in this file. Future structural-rule
lifts that need a similar treatment should follow the same shape:
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
   join `X concerns Y and Y has Z 'present'`. Both clauses must
   resolve to declared FTs OR the expansion is skipped (see
   `head_resolves` in `parse_forml2.rs`). Routes through
   `compile_join_derivation` because two antecedents share the join
   noun (Y). Pinned by `shape_join_path_via_possessive_expands_and_fires`.

7. **Negation guard** — `X has Y 'value' and X has no Y 'other-value'`
   adds an `AntecedentSource::AbsenceOf { fact_type, role }`
   secondary antecedent (see site (1) of `resolve_derivation_rule`,
   under `// Detect FORML2 negation patterns BEFORE resolving`).
   Surface markers recognised: ` has no `, ` is not `, ` does not `,
   leading `no `, leading `not `. The rule is flagged
   `uses_negation = true` and routes through
   `forward_chain_stratified`'s stratum-2 pass so the consequent of
   any positive rule that emits the cell-under-negation lands FIRST.
   Pinned by `sm_derivation_bridge_lets_readiness_rule_fire_off_projected_status`.

8. **Numeric aggregation** — `X has Count iff Count is the count of Y
   where X has Y`. The clause shape `<role> is the <op> of <target>
   where <body>` lifts to `consequent_aggregates`; routes through
   `compile_aggregate_derivation`. Operators accepted (see
   `try_parse_aggregate_clause` in `parse_forml2.rs::AGG_OPS`):
   `count`, `sum`, `avg`, `min`, `max`, `earliest`, `latest`,
   `first`, `last`. `min` and `max` fold over NUMERIC role values
   only — they are NOT a comparator over enum-valued nouns.

9. **Arithmetic-definitional binding** — `Volume is Size * Size *
   Size`. The clause shape `<RoleName> is <expr>` (where RoleName is
   a declared noun and `<expr>` parses through `parse_arithmetic_expr`
   over `+ - * /`) populates `consequent_computed_bindings`. Used by
   `compile_explicit_derivation`'s 1-antecedent path to project a
   computed value into the consequent fact.

10. **Subtype membership check** — `X is a Y` / `X is an Y` (both X
    and Y declared nouns) recognised by `is_subtype_instance_check`
    in `parse_forml2.rs`. Doesn't emit an antecedent source — the
    subtype relationship is structural and handled by the metamodel
    rule earlier in this file.

11. **Temporal predicate** — `now is in the past`, `… in the past`,
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

Author rules that need an ordering / superlative semantic via the
priority-cascade pattern below.

### Workaround: 2-level priority cascade with `has no` negation

When a target value type has TWO priority levels and the candidate
input is in a SINGLE-FT cell (not an existential-over-join), encode
each priority level as its own derivation rule guarded by an
`AbsenceOf` clause that ensures no higher-priority value was derived
first. Example for a Task whose Readiness should be 'ready' when its
precomputed Candidate Readiness cell carries 'ready', otherwise
'blocked':

```
Task(.id) is an entity type.
Task Readiness is a value type.
Candidate Readiness is a value type.

## Fact Types
Task has Candidate Readiness.
Task has Task Readiness.

## Derivation Rules
* Task has Task Readiness 'ready' iff Task has Candidate Readiness 'ready'.
* Task has Task Readiness 'blocked' iff Task has Candidate Readiness 'blocked' and Task has no Task Readiness 'ready'.
```

Stage-1 (positive) rules emit `ready` whenever its precondition is
met; stage-2 (negation-guarded) rules then emit `blocked` ONLY for
Tasks that don't already have `ready`. The stratified chainer
(`forward_chain_stratified` in `crates/arest/src/evaluate.rs`)
alternates the two strata until both pass with zero novel facts, so
the priority cascade reaches a unique least fixed point.

Pinned by
`priority_cascaded_readiness_picks_highest_via_no_negation` in
`crates/arest/src/compile_explicit_derivation_tests.rs`.

### Engine gaps task-814 surfaced (DO NOT use these shapes)

The audit at task-814 attempted to extend the 2-level cascade to
THREE priority levels via the existential-over-join shape
`* Merge has X 'value' iff Merge concerns some Commit that has X 'value'`
and found three independent engine gaps that make the natural shape
NOT work end-to-end. Each gap is a follow-up task; see citations
below.

1. **Existential-over-join globally collapses to first-fact bindings.**
   The shape `* X has Y 'lit' iff X concerns some Z that has Y 'lit'`
   parses to a 2-antecedent ModusPonens rule (`join_on=[]` because
   the `that`-relative is consumed by `expand_that_relatives` rather
   than appearing as a `that <Noun>` token at antecedent split). It
   routes to `compile_explicit_derivation`'s multi-antecedent
   existence-check fallback (`crates/arest/src/compile.rs:4508-4566`)
   which fires ONCE GLOBALLY whenever every antecedent has any
   surviving fact — and projects bindings from the FIRST fact of
   antecedent 0, not from a per-X fanout. Result: only one
   consequent fact lands, with whatever X value happened to come
   first in `Merge_concerns_Commit`.

2. **`compile_join_derivation` drops `consequent_role_literals`.**
   Authoring `* X has Y 'lit' iff X concerns some Z and that Z has Y 'lit'`
   forces `kind = Join` (the `and that Z has …` form preserves the
   join-key marker), which routes to `compile_join_derivation`. That
   function builds binding_parts by walking consequent role names
   and looking them up in antecedent FTs
   (`crates/arest/src/compile.rs:4818-4824`); roles like `Posture
   Witness` that exist ONLY on the consequent FT are silently
   dropped. The literal pin via `rule.consequent_role_literals` is
   NEVER applied. Result: derived facts have the right subject
   binding but NO value binding — the cell ends up populated with
   `<<Merge, m>>` tuples missing the priority literal.

3. **`compile_join_derivation` drops `AbsenceOf` antecedents.**
   When a Join-classified rule carries an AbsenceOf guard, the join
   pipeline indexes fact_extractors by FT id and AbsenceOf returns
   `""` for `fact_type_id()`. The extract collapses to an empty
   fact list, the iterative join with an empty antecedent yields
   ∅, and the rule produces zero consequent facts. (#918's fix
   targeted only `compile_explicit_derivation`'s implicit-equi-join
   branch.)

4. **`forward_chain_stratified` is 2-stratum only.** (Closed by
   task-814-stratify-3plus.)
   The prior 2-stratum chainer ran ALL negation-guarded rules in
   the same inner round. With 3+ priority levels — dual-gate /
   single-gate / none — both the middle and lowest rules landed
   in stratum 2 and fired together, so the lowest's
   `has no Y 'middle'` guard evaluated against pre-emit state and
   over-fired. The fix:
   - **`evaluate::forward_chain_stratified_n`** is the
     dependency-aware n-stratum entry point. It accepts
     `StratifiedRule` records carrying each rule's
     `consequent_cell`, `consequent_role_literals`, and
     `negation_reads` (each AbsenceOf antecedent's cell + role-
     literal pins). The chainer builds a dep graph where rule B
     depends on rule A iff A's consequent cell matches one of B's
     `negation_reads` cells AND the AbsenceOf's pins are
     compatible with A's consequent pins. Topological depth
     assignment partitions the rule set into sub-strata; each
     sub-stratum runs to fixpoint before advancing.
   - Role-literal pin compatibility (`pins_compatible` in
     `evaluate.rs`) is what keeps the dep graph cycle-free: three
     rules all writing to `Task_has_Target_Posture` with
     different role-literal pins (`'dual-gate'` vs `'single-gate'`
     vs `'none'`) don't form a dependency cycle because each
     AbsenceOf only matches the rule whose consequent emits the
     same literal — not every writer.
   - Cells `derivation_meta:<rule_id>` carry the dep metadata at
     runtime so the CLI compile path, apply path, and MCP query
     path all use n-stratum dispatch automatically without
     re-running the type-level compiler. Empty meta (no
     `consequent_cell`, no `negation_reads`) collapses to depth
     0 and runs in the first sub-stratum — equivalent to the
     2-stratum bucket.
   - The legacy 2-stratum `forward_chain_stratified(s1, s2, d, n)`
     signature is preserved for callers without dep metadata and
     correctly handles the 2-level cascade. Pinned by
     `priority_cascaded_readiness_picks_highest_via_no_negation`
     (unchanged) and the new 3-level pin
     `priority_cascade_three_levels_fires_exactly_one_via_dependency_aware_stratification`.

### Substrate-user workaround for the "strongest of collection" pattern

Until the four gaps above are addressed, encode the
"strongest-X-among-related-Y" semantics in TWO substrate steps:

* **Step 1: precompute the candidate cell as instance facts (or via
  a SQL view that materialises them).** For each X, emit one
  `<X, candidate-value>` fact per related Y whose value contributes.
  The candidate cell carries one fact per X×Y combination, NOT a
  reduction.

* **Step 2: run a 2-level priority cascade on the precomputed
  candidate cell** (per the working pattern above). The cascade
  reads from the candidate cell (which has no internal join), so
  it routes through `compile_explicit_derivation`'s 1-positive-
  antecedent + AbsenceOf path (#918) — the only path that handles
  the workaround shape correctly.

For more than 2 priority levels, split into MULTIPLE 2-level
cascades, each writing to a distinct cell, and add a final stratum-1
projection rule that selects the highest non-empty intermediate
cell. Each pair of intermediate rules can then stratify cleanly
within the existing 2-stratum chainer because each AbsenceOf reads
a distinct cell from what the rule writes.

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
  1-antecedent fanout, `AbsenceOf` guard handling.
* `crates/arest/src/compile.rs:3785-3919` — multi-AbsenceOf branch
  for n-positive + n-negation (#918); the ONLY path that honors
  AbsenceOf guards mixed with positive antecedents.
* `crates/arest/src/compile.rs:4508-4566` — multi-antecedent
  existence-check fallback; emits ONCE globally with first-fact
  bindings (the source of gap #1).
* `crates/arest/src/compile.rs::compile_join_derivation` —
  2+-antecedent equi-join (path used by `kind = Join`). Drops
  `consequent_role_literals` (gap #2) and AbsenceOf antecedents
  (gap #3).
* `crates/arest/src/compile.rs::compile_aggregate_derivation` —
  Codd image-set fold over numeric role values.
* `crates/arest/src/evaluate.rs::forward_chain_stratified` —
  alternating positive / negation-guarded rounds (2-stratum only;
  gap #4 is the 3+-level limit).
* `crates/arest/src/compile_explicit_derivation_tests.rs::priority_cascaded_readiness_picks_highest_via_no_negation`
  — the working 2-level priority cascade pin.
