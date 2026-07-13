# The parity ledger: swapping pyarest in for the old engine

The goal of record (Sam, 2026-07-03): NUKE the arest repo. That repo's lineage is
GraphDL proper, the library that powered the database schema generator for Rocket
Auto; the goal is a SWAP, so pyarest must stand behind the same interfaces the old
engine serves today. Parity is measured by liveness, not by the old tree's code
inventory, and inherited defects get fixed, not ported.

## The swap contract (what "the same interfaces" means)

1. THE APPS PROTOCOL. An app is a directory with readings/ and a per-app .db
   (C:\Users\lippe\Repos\apps, ~115 apps live today). The engine holds one DB at a
   time; apps_use switches; an active-app marker persists; apps_compile runs the
   readings to the lfp and projects; per-call app override routes without switching.
   pyarest's substrate mapping: an app is a STORE (the paper's nested stores /
   tenant cells), its .db is persist.save_sqlite of that store, its readings compile
   through the same create. Status: the pieces exist (child cells, persist, compile
   pipeline); the registry/protocol layer does not yet.

2. THE MCP SURFACE. The old engine's tool list is the contract (~30 tools): orient,
   apps_{list,use,create,compile,check,status,register,current}, query, sql,
   compile, apply, retract, get, cells, schema, explain, ask, propose, validate,
   verify, synthesize, induce, select_component, context (the mutation receipt),
   engine_version, and the tutor family. Priority inside the surface, by observed
   use: orient/apps/query/compile/sql/context first; induce/synthesize/tutor later.

3. SCHEMA GENERATION (the GraphDL day job). Readings to relational DDL: RMAP to
   CREATE TABLE with keys, not-null, references, uniqueness — the rmap.rs SQL-DDL
   specifics triaged earlier as "their SQL backend's bindings" are PROMOTED to
   parity by the lineage. pyarest has rmap_partition/table_columns/row shapes; the
   DDL emitter and the population projection into tables are the gap. sql (read
   side) has a short path: save_sqlite already maps cells to tables; the RMAP-shaped
   projection refines it.

4. THE LIVE DATA. Migration by re-ingestion (facts are the source of truth; replay
   through the same create). Verdicts by liveness:
   * LIVE, must migrate with fidelity: claude (the operational ledger and cognitive
     stack: 65 lessons, Operating_Rule/Engineering_Lever/Engine_Lesson, the spd-1
     affect stack wiring), tasks (the board), support.auto.dev, message-vetting,
     identity, merge, and the *-service family (bill-negotiation, cancel, refund,
     charge-dispute, parking-ticket, property-tax, robocall, tax, gym-contract,
     hoa-dispute, small-claims).
   * ARCHIVE, keep re-ingestable: the arc-* / gen-* / spd-* / induce-* probe fleet
     (research artifacts; their readings re-ingest on demand).
   * RETIRE with the repo: engine-internal probes (bisect-samekey, deriv-probe,
     freewill-repro, qvr-test, arest-dev) unless their readings carry lessons.

5. PARSER BREADTH. The old stage-2 surfaces pyarest's fragment lacks: possessive
   role navigation (Task1's Task ID), that-relative expansion, superlative-among
   (shape 11), plurals, the iff/where spelling, suggestion ranking, `Data Type:`
   colon-form. Enumerate exactly by running the old parser tests' (statement,
   classification) pairs against classify_via_M; land surfaces through the
   self-host thread (grammar readings + translators), not regex accretion.

## Fix-not-inherit (defects the old engine's own ledger documents)

* Incremental apps_compile leaves superseded projected rows (the ledger's
  prescription is "delete the .db and rebuild"). pyarest must make recompile
  supersession CORRECT (this is the derivation_rule_replace_on_recompile thread
  plus event-log replay; the ** view discipline already replaces on recompute).
* The dangling-FK cascade drop (external_system.url NOT NULL emptying ten nouns
  from the VIEW): projection must not let one incomplete entity cascade-delete
  valid rows from the relational view; the cell keeps the truth.
* The constraint-attribution and same-noun resolver traps (adjective-bound fact
  types, ring binding): pyarest's whole-reading resolver is structurally immune to
  the documented classes; keep the immunity pinned as surfaces land.

## Sequencing

MCP + apps protocol first (the daily driver; enables living IN pyarest), the DDL
generator second (the GraphDL role), live-data migration third (replay + verify
against old projections), parser breadth alongside via self-host, then the long
tail (HTTP envelope, CLI, cluster, generators, induce/tutor) each with a keep,
port, or retire verdict at pickup time.

## Survey verdict (2026-07-03, live-fire)

Nine live apps compiled through the seed: arc-aggcycle-probe, alpha-rule-test,
agent-policy, identity, merge, message-vetting, bill-negotiation-service,
support.auto.dev, tasks — 1,638 statements, ZERO unparsed, ZERO rule diagnostics.
Item 5's breadth list did not materialize in this sample; the remaining breadth
check is the claude app (heavyweight, compile-cost-gated until the boundary fix)
and the archive fleet on demand. The cost finding replaced it as the critical
item: the delta boundary round-tripped the whole store per apply (quadratic),
fixed by identity-memo + value-interning in the conversion seam, 26x on the
service-app shape. The DDL projection and live-data migration are now the front
of the queue.

## Breadth closed (2026-07-03, post-boundary-fix)

The claude app — the heaviest live app, 12 reading files, 265KB — compiles in
71.7s: 1,347 statements, ZERO unparsed, ZERO rule diagnostics, no statement over
the 3s tracer threshold. With the nine-app survey that makes all TEN live apps
clean through the seed grammar: 2,985 statements total, nothing unparsed,
nothing diagnosed. Parser breadth is no longer a parity risk; item 5 closes.
What remains in sequence: live-data migration (replay + verify projections
against the old .db tables), remaining MCP tools by observed use, then the long
tail with keep/port/retire verdicts.

## Breadth REOPENED and corrected (2026-07-03, migration rehearsal)

Zero-unparsed was blind to MISCLASSIFICATION. The tasks-app rehearsal (old .db
projection diff) crashed the DDL generator and exposed it: the live corpus
writes derivation rules in the NORMA anaphoric form (unnumbered type-name
variables, that/some qualifiers, leading * storage marker), which the numbered
rule_if recognizer never claimed — those statements silently became garbage
fact types. The old grammar's own classifier is unambiguous (forml2-grammar.md:
a statement with keyword iff IS a Derivation Rule), and the old engine
strip_role_qualifiers/subscript machinery confirms name-based binding.

Landed: the rule_iff recognizer + span-scan _rule_atom (multi-word types,
subscripts, qualifier fallback chain, in-body quoted literals as role
restrictions) + coercion clauses ('Task is Resource' re-keying idiom) + rule
heads as declarations in the prepass catalog. Resurvey with misclassification
detectors: 57 statements now compile as anaphoric rules (claude 35, tasks 15,
support 4, arc-probe 3); four NEW honest diagnostics in claude (previously
silent garbage). Remaining decomposition, in priority order:
1. BASE PRELOAD: apps compile atop the old CORE_READINGS backbone (8 files,
   916 statements — compiles clean through pyarest in 147s, so vendor + frozen
   ingestion). Explains identity/tasks roleless fact types (Timestamp,
   Resource, Status, State Machine, Fact Type undeclared in-app) and most
   claude diags.
2. PROSE REJECTION: readings prose paragraphs swallow as role-less fact types
   (claude 42, tasks 2, support 3). The old engine's #789 unresolved-Title-case
   machinery rejects these; ours must surface them as unparsed, not silently
   declare.
3. Corpus spanning-UC spelling: 'Each X, Y combination occurs at most once in
   the population of Z.' (bill-negotiation, support) falls through today.
4. DDL robustness: a role-less fact type must be skipped with a note, never
   emit malformed SQL (the rehearsal crash).

## The migration tool (2026-07-03, python/migrate.py)

Items 1 and 4 landed (base preload behind apps.Registry(base_dir) + frozen
ingestion; DDL quoting, shared column naming with dedup, role-less skip), and
the live-data path is now a TOOL, not a plan. The old cells encoding parses
with a round-trip proof (escape alphabet \\ < > , { } = from the old ast.rs;
keyed maps AND keyless tuple sequences; escaped chars masked into the
private-use plane so prose values quoting markup cannot break the scan).
Coverage on the tasks db: 78/78 asserted cells (21,847 facts), 22/22 derived
cells (24,765 facts). Asserted populations replay as BATCH log entries (op
migrate — one Store union + one derive pass; the old engine's atomic
collection apply is the precedent, and per-row validated creates would cost
hours on support.auto.dev's 273k facts). Derived cells are never replayed:
the engine rederives and migrate.replay_into's report VERIFIES old-versus-new
row sets per derived fact type — the parity evidence for the swap, app by app.
bill-negotiation-service has NO cells table (older schema): its extraction
falls back to the projected tables when its turn comes.

## Stored state and the first verify scorecard (2026-07-04)

The first live verify (tasks) answered 1 match, 21 differ, and the
decomposition drove three fixes:
1. IMPLICIT ROLE NOUNS: the old corpus declares nouns by Title-case
   occurrence (Event Type, Fact Type, Noun exist in every live db with no
   declaration anywhere). _known now mines maximal Title-case runs from
   non-prose statements; longest-first resolution fixes event-type
   populations landing in the event table. Projection identity moved
   66 -> 81 of 106 shared tables with count-parity on most of the rest;
   the residual is column-layout drift (absorption choices and column
   naming), enumerable per table from the rehearsal report.
2. STORED STATE: a cell marked derived (starred) with NO deriving rule is
   engine-written state, not a derivation — the base's own comments record
   the imperative writers owning State_Machine_is_for_Resource and
   State_Machine_is_currently_in_Status (transition write + event-fold).
   migrate.plan now migrates such cells as data (reported stored_state);
   only rule-backed derivations rederive and verify. The four
   compiler-synthesized base reflections (Resource_is_of_Function,
   Resource_belongs_to_Domain, Fact_Type_has_Arity, Noun_is_instantiable)
   ride the same path; long-term they belong to pyarest's own
   self-description, not migration.
3. Overlay readings: old-only derived rows (Status_is_rooted_in_SMD, the
   Rmap/Domain Change machinery) come from the old engine's feature
   overlays (EVOLUTION_READINGS etc.). Vendor by evidence when an app's
   verify demands them.

## Negation, the overlay, and the machine-as-facts seam (2026-07-04)

After wave 2 the tasks verify stood at 2 match / 2 differ, and both
residuals closed:
- evolution.md + csdp.md vendored (the live db's Rmap and Domain Change
  machinery proves the old binary compiled them in): the base is ten
  files, 1,055 statements, zero diagnostics.
- STRATIFIED NEGATION landed whole: theta:AntiRestrict in the shared
  polyglot base (Restrict's mirror, membership negated; cargo + the
  cross-kernel differential green), system.compile_rule_neg wrapping the
  positive body in anti-joins (full recompute above the closure, like
  aggregates — semi-naive deltas are unsound under NAF), where-chains
  scoping INSIDE their 'no'-group (never escaping as top-level conjuncts),
  and the 'no X' subject introducing a FRESH variable that shadows any
  outer X. The base's rooted-status rule (state.md: the old engine
  computed this gate in its seed branch) now derives in readings.
- Root cause under it all: the SM DSL statements are FACTS TOO (the old
  cells populate Transition_is_from_Status from those very lines), so
  sm_def/sm_initial/sm_from/sm_to dual-assert machinery + instance fact.

Open (machine-runtime, not store parity): the live corpus spells triggers
'Transition X is triggered by Event Type Y' — lands correctly as a plain
fact, but our sm_trigger machinery pattern expects 'Fact Type', so live
machines' smTrigger wiring is empty until the runtime seam is ported by
evidence (transitions_of/machine_step against the old SM behavior).

## The tasks app verifies: every divergence accounted for (2026-07-04)

Final rehearsal: 2 exact matches, 2 differences, and BOTH differences are
old-engine defects proven from the old db's own numbers (fix-not-inherit):
- Status_is_rooted_in_SMD: the old population (17) is byte-identical to
  the distinct (from-status, machine) pairs — the rule text's negation
  clause never applied in the old engine (its unresolved-clause machinery
  names exactly this failure mode). Our 2 rows are the canonical rule's
  correct evaluation (one source status per acyclic machine).
- Status_is_defined_in_SMD: 24 of the old 28 rows carry TRANSITION ids in
  the status column (reopen, advance-to-step5, ...) — a mis-derived join.
  Our 22 are the clean statuses of the four machines.
Projection: 103 of 127 shared tables identical; the rest are count-parity
with column-layout drift plus the old engine's own supersession lag
(role_is_used_in_reading 684 projected vs 694 in its cell;
state_machine_is_for_resource 957 vs 1072). only-old=112 tables remain
the keep/port/retire long tail (verb modules, views, access machinery).
The migration path for one real app is COMPLETE: parse with proof,
classify with the stored-state policy, batch replay, rederive, verify,
project — every number explained.

## The claude rehearsal, first scorecard (2026-07-04)

6 match, 18 differ, projection 185/241 identical. Matches include
Fact_Type_has_Arity 748=748 (the unnumbered aggregate at scale) and
Status_is_defined 32=32 (the overlay vendoring holds). The differ classes:
1. Status_is_rooted 23 vs 4: the SAME proven old-engine negation defect as
   tasks (old = bare from-pairs).
2. The SUM family derives zero while MAX matches (Compile_Run peak 4=4,
   total 4v0; App run totals; Layer_has_Load; Stratum_Stack loads): the
   corpus's sum spelling or the at-most-0 zero-supplying form does not
   compile yet.
3. The SUPERLATIVE election family (is elected, is recommended, is focal,
   is salient, slowest-for, regresses-for): the highest-X-among
   verbalization, not yet a recognized rule shape.
4. Agenda_ranks 5 vs 20: we over-derive (likely needs the superlative
   post-filter).
5. unknown cells 188 (engine internals + the old mis-filing defect),
   unparsed 15, agent/code_site placement diffs to probe.
The next grammar tranche, exact corpus texts (claude readings):
- SUM with a mixed-numbering head literal (compile-perf.md:41): '* Compile
  Run1 has total Duration Ms iff Duration Ms is the sum of Duration Ms1
  where Compile Run1 spends Duration Ms1 in Compile Phase1.' — the agg
  out-variable is UNNUMBERED while the source is numbered, and the head
  carries the unnumbered out-variable ('total' is reading text).
- AT-MOST-0 totalization (affect-select.md:74): '* Layer1 has Load '0' iff
  Layer1 stacks into Stratum Stack1 and Layer1 is operator-loaded by at
  most 0 Engineering Lever.' — a head LITERAL ('0') plus a bounded-count
  body clause ('is Xed by at most 0 Y' = the count of Y matching is <= 0,
  i.e. negation spelled as frequency); the ledger's own count-of-empty
  lesson documents the idiom.
- The SUPERLATIVE family (is focal / is elected / is recommended /
  slowest-for / regresses-for): mine the exact texts from affect-select.md
  and ledger.md at pickup.
  MINED (2026-07-04) — NOT a new rule shape: salient = ranks JOIN peak,
  focal = grades JOIN base, elected = pure conjunction, slowest-for =
  total (sum) JOIN worst (max); peak/base/worst are max/min aggregates
  the unnumbered-agg machinery already compiles. The strata chain is
  compositional over existing pieces; the claude rehearsal rerun is the
  verdict. BOUNDARY NOTE: max/min over lexical values is lexicographic
  ('10' < '9') — works for the current single-digit loads, and
  aggregates should eventually coerce like arithmetic (they are
  arithmetic-adjacent; comparators of facts stay lexical). Measure in
  the rehearsal before fixing.
  VERDICT RUN 3 (2026-07-04): 10 match (was 6) — the coerced SUMS light
  on real data (Compile_Run_has_total 4=4, App_has_run_total 4=4,
  base_Depth 1=1). ROOT CAUSE of the remaining affect cascade:
  Layer_has_Load = 0 because the zero-supply anti-join's NEG side reads
  an EMPTY/ABSENT cell (Layer_is_operator_loaded_by_Engineering_Lever)
  and the fetch bottoms instead of answering the empty population — a
  vacuous negation must PASS everything (nothing exists, so nothing is
  loaded). Fix at the neg-side fetch (missing cell = φ, the COND-null
  wrap); salient/focal/elected/peak/ranks all cascade off Load.
  Remaining after: worst-total max spelling (1v0, probe), Agenda_ranks
  over-derivation (5v20), rooted = the documented old defect (ours
  correct). Projection 187/241 identical.
STRATA GRAMMAR LANDED (2026-07-04): the at-most-0 idiom compiles as the
anti-join with the counted type as fresh subject (head literal supplies
the zero; class_rule no longer claims MARKED rules); the sum aggregate
derives with ARITHMETIC COERCION of lexical atoms (_tonum mirrored in
delta and prims — '120' + '30' is 150, 'a' + 'b' is bottom). MUST before
commit: the Rust '+' (main.rs 605 arith and the native match at 985)
gains the same coercion, and the differential gains a string-sum
scenario — the N-host law is tested equivalence, never assumed.
Then the authoring audit sweep (migrate.audit_authoring) over tasks +
claude for the swap cleanup list.

## THE CLAUDE AUDIT (2026-07-04): Samuel's warning, quantified

Mis-authoring findings (the swap re-authoring list): 72 PROSE IDS in
Resource_is_instance_of_Noun (sentences as resource identifiers); the
catch-all prose values — Engine_Lesson_prescribes_Construction (13),
Operating_Rule_has_Rule_Statement (12), Stack_Layer descriptions, App
Purpose/Rationale/Usage — the documented anti-pattern, committed by the
ledger app itself.
THE PERFORMANCE KILLER is the old engine's REFLECTION LAYER migrated as
data: Fact_is_of_Fact_Type 19,008 rows / 1.19MB, Resource_is_instance_of
_Noun 12,539 rows / 480KB, plus the metamodel family (Role_is_used_in_
Reading, Fact_Type_has_Role, Noun_plays_Role, Reading_has_Text...) —
roughly 2MB dragged through every derive round.
DESIGN: migrate.plan gains a REFLECTION EXCLUSION class — the old
engine's self-description cells (Fact_is_of_Fact_Type,
Resource_is_instance_of_Noun, State_Machine_is_instance_of_Noun, the
Role/Reading/Noun metamodel family) are EXCLUDED from replay (reported,
never migrated): pyarest's own compile IS the self-description, and the
old reflections are stale the moment the new engine ingests. Expected:
the migrated claude store shrinks ~80 percent by weight; derive times
follow. The prose-value cells MIGRATE (they are real app data) but ride
the authoring report for re-authoring into structured facts at swap.

## THE THIN RUNNER (Samuel, 2026-07-04): python/ is bindings only

The directive: python/ becomes a thin runner; cloned code cleans up.
AUDIT (initial): 6,901 lines across python/. Legitimate host layer: the
kernel (lam 149, reduce, delta 337, prims 197 — the irreducible platform
implementation), canon loader, defs registry, paths, and PURE MARSHALING
wrappers. CLONE/MIGRATE candidates, in order:
1. system.py (1,214): host-side TREE BUILDERS duplicating canonical
   style — _sm_join, F_of, derive_of, join_rule, join_rule2, mint_next,
   resolve_minting, validate scaffolding — composition shapes belong in
   shared/system.py (the anti_wrap precedent); run_rules is host
   ORCHESTRATION for now (the semi-naive driver) but its strata
   (agg/keyed passes) should compose canonical pieces.
2. theta.py: dedup/flatten etc. correctly REFERENCE canon by name; the
   builder functions (Filter/NatJoin/Project/JoinOn/Restrict) — verify
   each applies the canonical builder vs building trees locally; migrate
   local ones.
3. constraints.py (339): same audit — builders vs canonical references.
4. ast.py (255): FetchPop/Store/DefineIn/run — the AST layer's trees.
5. forml.py (1,773): the grammar host — stays until the FLIP makes the
   rules-path default; then Stage-1 extractors become canonical defs and
   forml shrinks to tokenizer marshaling.
6. meta.py, federate/seal/optimize/rewrite: audit last.
METHOD: one module per tranche, migrate composition shapes to shared
DEFs (cargo + differential gate each), delete the host clone, keep only
spec marshaling. VERDICT EIGHT note: keyed stratum landed but cascade
unmoved (still 12 match) — diff ranks' rows next context before more
keyed work.

## The N-host goal (Samuel, 2026-07-04)

COMPLETE functional equivalence across Rust, Python, C#, Java, and more,
all polygloting off the lambda framework. A host's price of admission is
the kernel alone: reduction + the Scott boundary + the exact-arity
vocabulary (DEF, A, N, K, PHI, S1..S9) + the primitive set; it inherits
every canonical definition verbatim. Syntax adjustments per host are
recorded where needed (JVM/C#: wrap each shared file in a CANON(...)
call). Platform speed lands as DEFS overrides under canonical names,
never as capability. The bottom-up debug below is the road to that goal;
the cross-kernel differential generalizes to an N-way differential as
hosts join.

## The polyglot debug, OSI-style bottom-up (Samuel, 2026-07-04)

The directive: everything polyglots off the LAMBDA FRAMEWORK; host code
exists for capability ONLY at the kernel; everything else is canonical
definitions with platform-specific optimizations registered as DEFS
overrides under the SAME canonical names (rho resolves to the fast one,
the canonical definition remains the spec — the bench's
canonical/overrides legs are the existing proof of the interface).
Sweep bottom-up, close each stratum before the next:
1. LAMBDA: audit how the delta evaluator hooks in — if the scott/delta
   choice is hardwired imports rather than a DEFS resolution, fix; the
   kernel (lam/reduce per host) is the irreducible platform layer.
2. RHO/DEFS: the override interface must shadow cleanly per platform
   (the pending engine:native third vocabulary binding lands here).
3. SHARED ALGEBRA: migrate compile_rule_neg's composition from
   python/system.py into shared/system.py as a canonical builder; audit
   every host wrapper for logic beyond spec marshaling.
   DESIGN (2026-07-04): one small canonical builder carries the logic —
   system:anti_wrap over ⟨obj, neg, key_spec⟩ builds
   COMP(AntiRestrict(key_spec), CONS(obj, neg)); the host wrapper keeps
   only marshaling (compile pos and neg rules, FOLD the groups through
   anti_wrap, apply the Project builder for the head). The wrapper
   doctrine: marshaling stays host-side, composition shapes are canonical.
   Gates: cargo + cross-kernel differential + the negation suite.
4. FORML: the self-host flip IS the polyglot completion of this stratum
   (classification and translation as rules over the ingested grammar;
   Stage-1 extractors become canonical defs) — parsing then rides any
   host. Reframed from optional gate to the path.
   MEASURED (2026-07-04, twins in place): selfhost 305ms/stmt vs seed
   7ms/stmt (44.6x), zero unclassified — correctness holds, cost is the
   per-statement re-derivation of the grammar lfp, NOT def conversion.
   DESIGN: batch classification — classification is a DERIVED population
   over Statement field facts (Codd: one equation, one derive), so
   tokenize the whole document, run ONE lfp over all field facts, then
   dispatch per classified statement. The semi-naive delta machinery
   already amortizes the grammar sweep across statements.
   LANDED (2026-07-04): classify_all_via_M + the batch selfhost loop —
   305ms/stmt -> 45ms/stmt (6.8x), ratio vs seed 44.6x -> 10.5x, zero
   unclassified, selfhost suites green. The residual decomposes next:
   the one derive vs per-statement translator dispatch vs tokenization
   (profile before optimizing further).
   PROFILED (infosci app, 97 stmts, 27.9s selfhost): classify_all_via_M
   is 20.7s — the ONE derive, inside the delta evaluator's generic
   machinery (mu, _cond, _mkseq, _insert) over the grammar's ~100
   classifier rules. The doctrinal fix: a FAST twin for the class_rule
   compiled shape registered under the canonical rule names via
   defs.override — speed as a DEFS registration (the universal override
   interface's purpose), canonical objects unchanged as the spec,
   oracle-gated.
   TWIN CONTRACT (from system.class_rule): per clause ⟨field_ft, lit⟩,
   filter the field cell on column 2 == lit (or existence when lit-less),
   project column 1 (statement ids); INTERSECT across clauses; pair each
   survivor with the head constant. Native: dict-backed set intersection.
   HOOK: run_rules consults a twin registry keyed by rule cid before
   generic evaluation; _h_class_rule registers the twin at build time
   (it holds clauses + head); reset clears; the selfhost suites plus a
   dedicated classification-equality test gate it.
   LANDED (2026-07-04): classSpec M-facts freeze the twin contract WITH
   the store (the thawed grammar rebuilds via rebuild_class_twins), the
   registry hooks both full-evaluation sites in run_rules, equality
   test green. Infosci remeasure: 288ms/stmt -> 48ms/stmt (6x on the
   real app; batch + twins compound). Residual vs seed: 13x — next
   profile targets the translator dispatch and tokenization.
   PHASE SPLIT + BATCHED ASSERTS (2026-07-04): the split measured the
   twinned derive at 0.27s and the one-apply-per-field-fact assert loop
   at 2.9s; classify_all_via_M now groups field facts by cell and lands
   ONE Store union per cell (the migrate batch op's shape). 48 -> 22
   ms/stmt; ratio vs seed 4.9x (from 44.6x at stratum start). The claude
   app self-hosts in ~30s at this cost: the FLIP DECISION is in range.
   Decision needs: the fleet differential (selfhost vs seed, all ten
   apps) and translator-kind coverage.
   THE DIFFERENTIAL (2026-07-04, 7 standalone apps): cell equality holds
   EVERYWHERE both paths produce cells (4/4, 4/4, 9/9, 15/15, 23/23,
   tasks 30/33). The flip checklist, complete and bounded:
   1. context_from seam in compile_model_selfhost (base preload).
   2. The prose posture as GRAMMAR RULES, unifying BOTH defects: the
      seed swallows short prose (Live_db_write_supervised became a seed
      fact type; the rules honestly said unclassified), selfhost
      swallows long prose (identity/tasks annotation paragraphs became
      fact types via Role References). The Prose Stopword enum already
      rides the grammar file.
      DESIGN (2026-07-04): Stage-1 emits Statement_has_Prose_Punctuation
      when the LITERAL-MASKED text carries the structural tells (comma,
      parenthesis, ': '); the grammar file gains 'Statement has
      Classification Prose iff Statement has Prose Punctuation.' —
      Prose is SPECIFIC so it beats Fact Type Reading in the
      generic-yield dispatch; the selfhost loop reports Prose-classified
      statements as prose (with the seed's compile-time guard unchanged,
      one posture both paths). The grammar edit invalidates the frozen
      snapshot by text key (one ~90s re-ingest). Gates: the selfhost
      suites, the prose-guard suite, and the fleet differential's
      identity/tasks extras vanishing.
   3. The merge fanout: selfhost minted per-value fact types
      (Merge_has_Target_Security_Posture_dual_gate) — the old engine's
      mis-filing shape faithfully reproduced; fix, not inherit.
      FIXED (2026-07-04): the translator kind order now matches
      _CLASSIFY's arbitration (class_rule before rule_iff — the
      quote-aware rule pattern was claiming quoted-head classification
      statements). Merge reads PERFECT equality: 15/15, no extras
      either side.
   4. Task_has_Task_Description content differs (one cell): probe the
      long-literal parse.
      FIXED (2026-07-04): Stage-1 is literal-aware (the old #845
      scanner): recognizer tokens and Role References mask quoted spans
      — task 916's description contained a negation token, classified
      the instance fact Negation Reading, and specific-beats-generic
      dropped it. Pinned by test; the fix also claimed one merge extra.
   5. tasks unclassified=4: enumerate the kinds. RESOLVED (probed): all
      four are prose (backticked code refs, commit hashes, a bullet) —
      the rules refuse them honestly, the seed's guard also flags them;
      NO kind-coverage gap, no action.
   4b. The Task_has_Task_Description differ PROBED: one instance fact
      (task 916, a long description literal) drops through the selfhost
      instance translator — a single field-extraction defect in the
      long-literal path, localized.
   6. Speed: tasks selfhost 55.7s vs seed 14.8s standalone (3.8x) —
      acceptable for the flip gate, optimizable after.
THE FLIP ACCEPTANCE (2026-07-04): six of seven apps at COMPLETE cell
equality, zero extras, zero unclassified either direction. Tasks 31/33:
the three residuals are ALL prose edges — two are SEED defects (short
Title-case fragments the regexes swallow; the rules refuse them), one is
a punctuation-less paragraph the structural tell misses (the residual
class the old word-level test covered; a Prose Stopword grammar rule
over unresolved Title-case tokens would close it). Speed: tasks 46s vs
15s (3.1x). The evidence is complete; the DEFAULT-PATH DECISION is
Samuel's call: selfhost equals the seed on cells, exceeds it on honesty,
runs 3x slower on the biggest app, and completes the lambda-first
directive at the grammar stratum.

MINING BOUNDARY RESOLVED (2026-07-04, by the claude verdict's evidence):
a Title-case run becomes a noun only when CORROBORATED — somewhere in the
corpus the run is immediately followed by a quoted literal (instance
evidence: Event Type 'created', Target SHA 'abc'); and runs are ATOMIC
(_atomic_run_guard): a declared noun matching INSIDE an uncorroborated
run ('Layer' within 'has Layer Affinity to') is predicate text, not an
occurrence. Root cause chain of Layer_has_Load=0: phantom 'Layer
Affinity' role -> 3-wide fact type -> 2-wide migrated rows never join ->
operator-loaded empty -> count starves and at-most-0 fires for ALL
layers -> agg-replace clobbered even those (fixed separately: per-GROUP
supersession). 51 grammar-suite tests green over both fixes.

VERDICT FIVE (2026-07-04): Layer_has_Load MATCHES 8=8 (counts and zeros
together, row for row) and peak_Load follows — the strata core derives
natively; 11 match total. BUT the corroboration over-tightened:
Fact_Type_has_Arity collapsed 748 -> 1 and the projection lost 10 tables
— derived-only value types (Arity) are never instance-quoted, so the
boundary un-nouned them. REFINEMENT (next): Arity IS explicitly declared (core.md:51), so the
un-nouning hypothesis is WRONG for it — the regression sits in the
arity rule's parse under the new guards. PROBE FIRST: compile the base
alone, print Fact_Type_has_Arity's role rows, the agg rule's cols map
and group positions, and diff the factType/role rows of the base
against a pre-guard compile (the frozen cache holds older snapshots).
One row out = one group = the grouping column collapsed; find WHICH
clause's variable binding changed under _atomic_run_guard or the
corroboration set, then narrow the guard, never widen blindly.

VERDICT SIX (2026-07-04): Fact_Type_has_Arity HEALED 748=748 (the
quantifier corroboration works; three corroboration sources now:
declaration, instance quote, quantifier position — plus the atomic-run
guard). Projection 186/241, only-new down to 46. REMAINING: Layer's
operator-loaded rule STILL does not fire (loads all zero via the
zero-supply; the cell absent) despite actionable=6 and the affinity ft
parsing two-wide. PROBED (2026-07-04): the rule compiles, reads the right cells, both
antecedents populated (actionable=5, affinity=11) — and the object
EVALUATES TO BOTTOM. The shape is a UNARY-FIRST atom followed by a join
('EL1 is actionable and EL1 has Layer Affinity to Layer1'): every rule
test so far led with a binary atom, so the width-1 running tuple through
the linear NatJoin chain is untested. MINIMAL REPRO: three variants ALL PASS — unary-first join standalone,
the predicate-text reading standalone, and the tiny model ATOP the base.
The trigger is inside the claude corpus itself (one of ~1300 statements
re-shapes this rule's compilation in full context only). THE ELIMINATION CHAIN (2026-07-04, all falsified by measurement): the
twelve-file bisect passes pre-derive; both source cells clean IN D2; the
whole D2 store converts to native cleanly (no cell collapse); fuel
unbounded still bottoms; memos cleared still bottoms; the rid resolves
in BOTH step frames (853 vs 874 entries). The invariant: the rule runs
over D (rows=0) and bottoms over D2 = run_rules(D), same process.
EDGE NARROWED (2026-07-04, hypotheses nine and ten down): the served
rule objects are IDENTICAL between frames; exactly one tree atom
(Engineering_Lever_is_actionable) became cell-resolvable in D2, but the
hand-Stored minimal repro PASSES, and the 21-cell frame bisect heals
NOTHING — because the frame was trimmed while the rule fetches from its
OPERAND, which was always the full D2. The poison rides the FETCH PATH
over the post-derive store. NEXT PROBE (surgical): evaluate the FETCH
alone — ast:FetchPop of Engineering_Lever_is_actionable applied over D
(works?) and over D2 (bottoms?), beside _pop_rows which works over both.
If the bare fetch bottoms on D2, walk the fetch mechanism (the FetchPop
tree, its apply-through-mu, the delta store rebuild at the operand
boundary) over the two stores; the divergence line IS the bug. Then the
fix, verdict seven, gate44.

VERDICT SEVEN (2026-07-04): 12 match — Layer_has_Load 8=8 EXACT (real
counts + zero-fills) and peak_Load with it; the strata core derives
natively. The hunt's answer held: 'one' removed from the quantifier
corroborators (solver-loop.md:53's frequency phrase had re-nouned Layer
Affinity into a phantom third variable; twelve hypotheses each
falsified by measurement — the chain is the step-frame/mining
documentation now). REMAINING CASCADE, one fix class: ranks 8v11 —
the positive closure UNIONS across lfp rounds (zero-fill lands round
one, counts later; both survive), where the old engine's KEYED UPSERT
(the base's task-955/924 comment: cells keyed by a functional UC
collapse last-write-wins) supersedes per key. Design: Store-side
per-key supersession for fact types carrying a spanning 'exactly/at
most one' UC on the key roles — salient/grades/focal/elected all
cascade off it. Then worst-total (1v0, max-agg probe), Agenda_ranks,
and rooted stays the documented old defect. Gate44 certifies the
seven-fix tranche meanwhile.

VERDICTS EIGHT AND NINE (2026-07-04): the keyed stratum and the
reflection exclusion both landed (12 match holds; the exclusion cuts
~2MB from the migrated store) but the cascade is UNMOVED — and the
corpus explains why: ranks carries NO uniqueness (affect-select.md:38),
so the keyed pass correctly skips it. Its 11-vs-8 rows are DOWNSTREAM
DAMAGE from Load's mid-closure supersession: rows derived from
superseded sources must be RETRACTED and re-derived. That is deletion
propagation through derived views, and the library holds the canonical
treatment: Gupta-Mumick-Subrahmanian 1993, Maintaining Views
Incrementally (infosci/) — the DRed algorithm (overestimate deletions,
rederive survivors, apply net change). DESIGN NEXT: when the keyed/agg
strata supersede rows, compute the deleted set and run DRed over the
downstream rules (ruleReads gives the dependency graph) instead of
another ad-hoc pass. Read the paper first; implement from it.

VERDICT TEN + THE JOINT FIXPOINT (2026-07-04): DRed landed in two
stages. Stage one (supersession-triggered rederive) moved the cascade —
14 match: ranks 11-to-8 EXACT, salient 8-to-2 EXACT, grades 8-to-2
EXACT, recommended from missing-both to over-deriving — but verdict
ten's second phase (replay over the frozen store) exposed two holes the
trigger form cannot close: (a) staleness INHERITED from an earlier
store fires no supersession now, so it survives (grades 7v2 in phase
two, exact in phase one); (b) an aggregate folding OVER a swept head
runs before the sweep and keeps the stale fold (base_Depth value
drifted; and the peak-over-ranks probe showed the ordering wrong even
without staleness — the fold ran while the closure had only partial
supply). THE DESIGN, from the sources: for a FULLY-derived plain head
the stored cell is materialization of the expressible set (Codd 1970
§1.5), never ground truth, so run_rules re-evaluates it whole and
REPLACES, unconditionally — derive is idempotent whatever the store's
history — and the three upper passes (agg per-group supersede, keyed
per-key upsert, DRed sweep) iterate together to a JOINT fixpoint
(cap 12), because each can invalidate the others through the
dependency graph. Whole-cell rederivation is GMS93's
overestimate-then-rederive at cell granularity, sound exactly because
no row of a fully-derived head is asserted. Self-supporting heads
(reachable from themselves through derived-head reads) stay out of the
sweep: a cyclic overestimate rederives itself, and cleaning it needs
the paper's delta form — the recorded residual, alongside agg groups
whose supply vanishes entirely (per-group supersession keeps them; no
corpus case exercises either).

VERDICT ELEVEN (2026-07-04): SEVENTEEN exact matches. The joint
fixpoint with dirty-set filtering landed the whole affect cascade:
focal 1=1, elected 2=2, base_Depth healed, and the replay phase
converged with the migration phase (no stratum_stack diffs in either).
The dirty filtering also fixed the cost regression the first sweep
caused: the machine-step bench read 13.19s standalone against 155s
inside the loaded suite process, and the incremental-propagation test
pins the ripple semantics (assert, then changed-filtered derive
carries count, keyed upsert, rederive, refold). Suite 448 green;
fleet differential green with the one documented tasks-app prose
residual; cargo green.

THE REMAINING EIGHT, three families: (1) COMPARATOR LEXICALITY, now
legible as one family — worst_total 1v0, slowest 1v0, regresses 1v0,
peak_Duration content, dominates content are ALL duration analytics
over multi-digit numbers stored as text, and lexical max picks '305'
over '1190'. The old engine's ast.rs atoms were TYPED (Int vs Text),
so its comparisons were numeric wherever values parsed. The fix
mirrors the arithmetic _tonum: le/ge/lt/gt coerce int-first in delta,
prims, and the Rust kernel, and the differential pin scenarios update
to coerced expectations on both hosts. Equality stays untyped. (2)
Agenda_ranks 5v20 and recommended 2v5: over-derivation, likely
downstream of family 1's comparatives (rank thresholds compare). (3)
rooted 23v4: the documented old-engine seed-branch defect, proven
from its own data. Family 1 is the next arc; verdict twelve is its
acceptance run.

THE ANALYTICS ARC, RESOLVED BY MEASUREMENT (2026-07-04): the family
hid THREE distinct defects, and the comparator-lexicality theory
survived only as one of them. (1) Agenda_ranks 5v20 was NOT
old-engine staleness (probe: the join of the old snapshot's own
considers/actionable/has_rank cells gives exactly its 5 rows, and its
recommended follows) — it was OUR linear-chain join guard admitting an
atom whose trailing variable was already bound (unary atom BETWEEN
two binaries), silently dropping the rank equality: the cross
product. The guard now requires fresh trailing variables; rebound
ones route to the general pairs join. (2) worst_total 1v0 was NOT the
comparator bottoming on ints (probe: max over sum-derived ints folds
fine) — it was the MIXED cell: a singleton sum answers its element
UNAPPLIED (Backus INSERT over one item), so the live store holds
'11000' (string, single-phase run) beside 4997 (int, multi-phase
sum), and str-vs-int comparison bottoms the whole fold. (3) peak and
dominates content diffs WERE lexical ordering ('305' over '1190').
One fix covers 2 and 3: le/ge/lt/gt coerce like arithmetic (numeric
wherever both sides parse, lexical for non-numeric string pairs,
bottom on mixed non-numeric), in delta, prims, and BOTH Rust
comparator sites; the polyglot differential pins the coerced cases
cross-kernel including the live store's exact mixed pair. Expected at
verdict twelve: the probe proved old's cells join to 5 and our inputs
are cell-equal, so the guard fix lands Agenda_ranks at 5-exact with
recommended following, and worst/slowest/regresses/peak/dominates all
heal via the coercion. Projected residual: rooted alone (the old
engine's documented seed-branch defect).

NOTED FOR AUDIT: the reflection carve-out migrated
Resource_is_instance_of_Noun whole (12,539 rows) because some
compiled rule READS it. Find the rule; if the read is incidental the
480KB drag goes with a rule rewrite at swap time.

AUDIT ANSWERED (2026-07-04): two corpus rules read
Resource_is_instance_of_Noun, namely Constraint_is_semantic and
State_Machine_is_instance_of_State_Machine_Definition. Both are the
MODEL-level face of the mis-authoring class Samuel warned about:
derivations authored against the old engine's self-description
instead of domain cells. The machine-to-definition binding is
derivable from State_Machine_is_for_Resource plus the smDef bindings
without the 12,539-row noun-instance table. Swap-time re-authoring
list gains both rules; the 480KB drag goes with them.

THE BUILDERS WAVE LANDS (2026-07-04, commits 3ffed23 + 828e301):
eight system trees moved to shared/system.py as canonical defs, each
proven by a twin-oracle test against the Python builder before the
builder became a thin wrapper (the compile_agg_rule precedent, call
sites untouched): sm_join, sm_join_named, join_rule, join_rule2,
F_of, derive_of, max2, mint_next, resolve_minting. The
transitive-closure test pins the WHILE fixpoint through the canonical
forms; gate48 read 460 green over the wave; cargo compiled the grown
intersection source unchanged. This is the first full wave of the
defs-override-glue-framework posture: an N-host now needs the reducer
and the shared file, and earns performance by overriding exactly the
parts it cares about. REMAINING IN system.py for later waves: the
emit/HATEOAS builders (nav_of and kin), compile_rule's record
assembly (already canonical-bodied), and the run_rules orchestration
itself (the pipeline-as-data endgame). NEXT THREADS: stratum 5
(Connector-named I/O), the N-host kernels, meta.py migration, the
flip decision (Samuel's), the counting algorithm for cyclic heads
(GMS93's delta form, the ungrounded concept infosci can point to).

THE N-HOST COURSE CORRECTION (2026-07-04, Samuel): NO JSON EXPORT, no
neutral serialization, no parsers. The whole point is a shared
functional polyglot made of a platform-specific lambda root and DEFS
overrides: the SAME FILES are the source for Python, Rust, C#, and
Java, by the hackery of building on lambdas. The intersection.md
discipline said this all along (its first paragraph forbids JSON
shims by name; reread the discipline before designing around it). The
concrete path proven this window: the shared files are one method
call away from valid C# and Java (nested vocabulary calls are legal
in all four languages; the hash characters live inside strings; the
one blocker was the file-final trailing comma, now removed and the
discipline tightened, commit bb1a45b). Each new host is: the
vocabulary as static methods (DEF/A/N/K/PHI/S1..S9 plus T for the
wrap), a generated wrap of the raw bytes (T + file + semicolon, the
include! equivalent; C# via an MSBuild pregen, Java via the same
trick since T(...) is a varargs call), and the FFP reducer with prims
mirroring delta.py's semantics including the coercion. Acceptance is
the cross-host differential; the scenarios should themselves become
an intersection file in the same vocabulary, not a data sidecar.
dotnet 10 is on the machine; no JDK yet. NEXT: csharp/ kernel in a
fresh context window. The C# prim contract is proven by closure
check: every atom the shared canon references beyond the canonical
def names is either a delta-table prim or DATA (literal T/F/CELL/#
markers, cell names like smFrom, aggregate op names compared by eq),
so the host needs the delta prim table, the eight forms, selectors,
and nothing else.

THE FLIP LANDS AS CLEANUP (2026-07-04, Samuel: "Let's clean up the
python folder to only be the necessary parts. It looks like a
duplicate stale branch"): the duplicate stale branch is the seed
classifier inside forml.py (the _CLASSIFY regex arbitration and the
per-statement compile path), duplicating what the selfhost path
compiles from shared/forml2-grammar.md; the translators were always
shared. compile_model now routes to compile_model_selfhost by
default with the seed behind PYAREST_SEED=1 as the migration escape
hatch, report shape preserved (total, kinds, unparsed,
rule_diagnostics; unclassified maps to unparsed; kinds empties).
GATES IN FLIGHT: the ripple suites, then the full suite and the
fleet differential; the claude rehearsal (verdict thirteen) is the
acceptance. AFTER GREEN: delete the seed classifier (analyze,
_CLASSIFY, compile, _compile_model_seed and the recognizers only the
seed used), which is the actual python-folder cleanup, then remeasure
the 3.1x selfhost cost and continue the batch optimizations against
it. Module inventory verdict: every python/ module carries a
distinct documented purpose (the two prim sets are the two
evaluators, by design differential-pinned); the folder's duplication
was intra-forml, not inter-module.

FLIP GATE TRIAGE (2026-07-04, suite over the flipped default: 440
pass, 20 fail, 976s wall): two fixes landed en route, and both were
structural. First, the grammar BOOTSTRAPS through the seed compiler
by definition (ingest_frozen took a compiler parameter; routing the
grammar through the selfhost default recursed into grammar_D inside
its own construction, which was the 900-CPU-second mystery). Second,
the joint fixpoint's agg and keyed passes now filter by the dirty
set on INCREMENTAL calls in round one too (sound: a deterministic
unit whose reads did not change cannot change), which the selfhost's
per-batch classify multiplies. The 20 failures are the seed-vs-
grammar coverage inventory, two classes: (a) REPORT CONTRACT, the
prose tests expect paragraphs in unparsed where the selfhost refines
them into the prose list (update the tests to the refinement); (b)
GRAMMAR GAPS, statement forms the seed's _CLASSIFY recognized that
shared/forml2-grammar.md does not yet classify: frequency readings,
article-free clauses, roleless fact types, derivation-rule reading,
federation namespacing, negative uniqueness, inclusive-or, implicit-
noun prose interaction, storage-kind trailing marker, subset clauses,
seal's schema derivations, frequency role positions, and the two
triage-batch4 subtleties (bare subset gating, permuted-head copy
check). Each gap is a grammar-file rule plus translator wiring,
exactly the work "the parser is the file" prescribes. The seed stays
behind PYAREST_SEED=1 until the list empties; the deletion of the
seed classifier follows the list, not the other way round.

FIRST GAP DIAGNOSED PRECISELY (the iff-prose case,
test_prose_guard.py, left RED as the pin): a paragraph containing
' iff ' earns a rule classification that beats Prose (correct
arbitration), its translator DECLINES (no head match, correct), and
the statement then vanishes from every report list. The seed's guard
reported it unparsed. The selfhost needs DECLINE TRACKING: a
statement all of whose translators decline without asserting falls
back to unclassified (the D-unchanged check per statement is the
detection). The prose tests now assert the refined contract (flagged
= unparsed plus prose), so they go green the moment decline tracking
lands.

THE FLIP AND THE THIRD HOST ARE COMMITTED (2026-07-04, Samuel
unlocked the pin): cd3385d the flip, e9fb74b the C# kernel. THE SEED
DELETION IS SPECIFIED BY MEASUREMENT: the grammar file exercises
exactly FIVE kinds through the bootstrap (fact_type_reading 91,
class_rule 57, value_type 54, value_constraint 47, entity_type 4),
so _CLASSIFY shrinks from 42 entries to those five, the dead kinds'
productions and handler branches go with them, and the PYAREST_SEED
escape hatch dies (the bootstrap is invoked explicitly via the
compiler parameter, never by environment). Gate: delete the frozen
grammar snapshot to force a cold bootstrap ingest, then the full
suite, the fleet differential, and a rehearsal. Sherlock carries no
.db (the rehearsal harness needs the old engine's database; not a
flip failure); spd-1's run tells whether the harness covers it.

SPD-1 CORROBORATES (2026-07-04): five of six derived fact types
match exactly under the flipped compiler, and the one diff is
Status_is_rooted 13v2, the SAME class as claude's 23v4. The rooted
defect is SYSTEMIC in the old engine: every machine-bearing app
shows the same collapse when the negation computes honestly (the
old seed branch's non-monotonic gate over-derived). Two apps, one
defect, zero pyarest divergences beyond it.

THE NAMED WORK COMPLETES (2026-07-04, commit 4f5c176, gate 487):
system:explain moves the derivation chain into the canon, the reads
filter building its predicate at run with the rule id embedded, and
the verb corroborates its host walk with the canonical chain. The
debugging sequence is preserved in the message as the quasiquote
idiom's teaching case. TWENTY-EIGHT commits this session. EVERYTHING
NAMEABLE IS DONE: engine parity (three apps, every divergence the
old engine's own), four hosts on one canon and one case table, the
folder at seven files, the create pipeline and the verb cores
canonical, the verb surface first-class through verify, the Rust
resident serving it. REMAINING: Samuel's swap decisions (the rooted
call, the re-authoring pass, the swap execution), and the optional
verb long-tail (induce, ask, tutor, propose) as demanded.

THE PIPELINE FIVE COMPLETE; PARALLEL AGENTS PROVEN (2026-07-04,
commits 7b06770 + 19b0557 + 9cd7d3f, gate 486 green): row_resolve
(iota + QUASIQUOTATION: build-time identity is the builder's params,
ALPHA-time identity is the element, a run-time constructor rides one
K deeper), then two agent worktrees merged file-disjoint (validate +
verify on the verb table with five tests; the RUST RESIDENT serving
verbs/query/cells/synthesize_pairs with system:verbalize computing
IN RUST byte-identical to the Python contract), then checked_apply
(the parked probe = two lesson violations, confessed by the tree's
own dump). EVERY raw-tree builder of the create pipeline now lives
in shared. REMAINING NAMED WORK: the canonical explain core (the
quasiquote pattern is proven and waiting), the swap decisions
(Samuel's), and the old-surface verbs beyond parity (induce, ask,
tutor, propose) as they're demanded.

THE VERB SURFACE GROWS FIRST-CLASS (2026-07-04, commits fa38337 +
11fc8ac): SESSION_VERBS and APP_VERBS as one system-level table,
every binding a router (Samuel: the verbs are not MCP-specific).
synthesize lands with its CANONICAL core (system:verbalize, four
sub-builders, the entity id woven three levels deep, green first
run); explain lands host-walked (the derivation chain through the
step frame, supports and reads per rule, the audit trail from the
events journal), its canonical core parked with the design named:
reads-per-rule needs the filter predicate tree BUILT AT RUN with the
rule id embedded, QUASIQUOTATION in the def vocabulary. Remaining
old-surface verbs for parity: validate, verify, compile, propose,
induce, ask, actions, tutor family, select_component,
engine_version. REMAINING PIPELINE MOVERS: row_resolve (20 trees),
checked_apply (31). THE HIERARCHY THREAD: the Rust resident's serve
loop takes this same verb table as the daily driver.

FTPOP PARKED WITH ITS FINDING (2026-07-04): the absorbed-layout twin
needs the CREATE PIPELINE's write shapes (index cell plus per-entity
cells via cellkey "{table}:{key}"), which compile_model stores never
materialize (fact types get their own cells there; ftpop's absorbed
branch reads an empty index and answers empty). The fixture comes
from checked_apply/row_resolve's write path; governance_rules turned
out to be orchestration, not trees. The pipeline five remain the
big movers with the lesson set complete.

THE CLASSIFIER GOES POLYGLOT (2026-07-04, gate 475 in 4:34, staged
on the cold pin with proposal B and the six rings):
system:class_rule and system:class_subj move the grammar recognizer
into the canon, the canonical form taking any predicate over the
field row (the Python applier turns literals into eq-predicate data
trees). Every host that loads the canon can now classify statements.
The authoring insights that unlocked it: host operators like
ast:FetchPop APPLY, never metacompose, and a built element carrying
a parameter-built operator needs the id-pairing one level deeper
than a constant one. engine.py's raw-tree count dropped by
class_rule's fifteen on top of the rings; the create-pipeline
builders (checked_apply 31, row_resolve 20, ftpop_expr 18, ft_view
13, row_validate 12, governance_rules 12) are the remaining big
movers, each now reachable with the complete lesson set.

THE RING FAMILY COMPLETES (2026-07-04, gate 474 in 4:30, staged on a
cold pin): all six rings canonical in shared/constraints.py, twin-
proven, acyclic riding system:derive_of for its closure. THE
AUTHORING LESSONS (the next families' recipe): a built element must
be the APPLIED operator, double-apply for constants (apply over the
pair of the built operator and id); predicates and parameters are
DATA trees, never builder-headed sequences; a rules sequence for
derive_of is CONS-of-one whose element is the single-apply builder
(bare singletons metacompose). REMAINING FAMILIES: frequency,
value_range/enumeration, set-comparison, nav/HATEOAS, sm builders,
ast machinery.

PROPOSAL B ACCEPTED BY MEASUREMENT (2026-07-04, verdicts fourteen
and fifteen): the instance mirror derives engine-side (the machine-
instance rule holds at 20 from the store's own role facts), verdict
fourteen caught the scope gap (the arity rule counts
Fact_Type_has_Role), the role mirror closed it, and verdict fifteen
reads 23 exact with TWO diffs, both old-world artifacts: rooted (the
systemic old defect) and arity 748v405, where 748 described the OLD
engine's schema including its reflection layer and 405 is pyarest's
honest self-description, zero downstream regressions from the
re-basing. No reflection cell migrates, unconditionally. ALSO
LANDED: the ring family's first three canonical builders
(irreflexive, symmetric, asymmetric) by the twin-oracle recipe with
the corrected applied-operator idiom (a built element must be the
APPLIED operator: wrap parameterized pieces as apply-compose inside
a COMP-headed sequence); engine.py thins, shared/constraints.py
grows — Samuel's too-big-not-sharing-enough directive advancing one
family at a time. REMAINING RAW-TREE FAMILIES in engine.py:
antisymmetric/intransitive/acyclic rings, frequency,
value_range/enumeration, the set-comparison family, nav/HATEOAS,
the sm builders, and the ast machinery — each a twin-oracle wave.

NEXT WINDOW OPENS WITH: (a) the Rust case-table wiring, designed: a
sibling scenario_defs() beside canon_defs() with the identical
closure vocabulary and include!("../../shared/scenarios.py"), a
--cases CLI mode reducing each pair through the existing mu, and a
show format matching the other hosts' ('a', 'b') convention, plus
the pytest extension holding all four hosts to one table; (b) the
init docstring refresh is in the working tree (unstaged, joins the
next commit); (c) the seven-file commit is STAGED on a cold pin.

SEVEN FILES (2026-07-04, staged on a cold pin, gate 466 in 4:50):
Python is shaped like its sibling hosts. kernel (lam+defs+delta+
reduce+prims, the sixteen twin names between the two evaluators
prefixed per section), canon, compiler (meta+forml), engine
(ast+constraints+system; the machine fold's run renamed off the
pipeline's run), protocol (persist+ddl+migrate+federate+apps+
mcp_server; migrate's plan versus seal's plan resolved; intra-file
siblings self-aliased), tools (optimize+polyglot), and the init
whose sys.modules alias table keeps every old name importable with
zero call-site churn. Alias order matters: kernel, canon, engine,
compiler, protocol, tools. Rust one file, csharp four, java five,
python seven; the extra three are the compiler, the orchestration,
and the protocol the fleet shares. EIGHTEEN commits landed, one
staged.

THE CASE TABLE GOES CROSS-HOST (2026-07-04, commit e8c5b34):
shared/scenarios.py rides every wrap; forty cases held identical
across Python's two evaluators, C#, and Java; two catches closed
(bottom renders bare; C# doubles learn Python's repr). SEVENTEEN
commits this session. THE REMAINING THREADS ARE ALL SWAP-SHAPED:
the re-authoring list (72 prose ids, the two reflection-reading
rules), the rooted-defect call (Samuel's), and the swap rehearsal
against the full live fleet. The engine, the hosts, and the folder
are done by every measurement this session could construct.

THE FOLDER MERGE COMPLETES (2026-07-04, commits 099816a + 2e3750e,
466 green in 5:20, tree clean): nineteen modules plus the init, each
single-importer or family-coherent by measurement. theta and paths
into canon; machine into system; seal into persist; rewrite into
optimize; cluster into polyglot. shared/scenarios.py (the cross-host
case table, forty cases) and its intra-Python differential
(Scott and delta agree on every case) rode the second commit.
SIXTEEN commits this session. NEXT: the scenarios' C#/Java wiring,
then the swap-shaped threads.

THE FOLDER MERGE, ATTEMPTED AND REVERTED (2026-07-04, Samuel: "There
are still a lot of code files in the python folder"): the measured
plan stands at 25 files to 19 (machine into system, theta and paths
into canon, cluster into polyglot, seal into persist, rewrite into
optimize; each single-importer or family-coherent, importer counts
measured). The first mechanical attempt broke 259 tests two ways and
was reverted whole: split-at-docstring DROPPED the merged modules'
own imports, and a bare word-boundary regex on 'machine.' mangled
docstrings. THE REDO DISCIPLINE: append full bodies with imports
hoisted deliberately, fix usage sites found by grep rather than
regex-wide, update the ~15 test files importing theta/paths by name,
and gate each merge with its targeted tests before one parallel
gate at the end. shared/scenarios.py (the cross-host case table in
intersection source, 40 cases) is authored and parked untracked;
its host wiring resumes after the merge lands.

THE DRED RESIDUAL CLOSES (2026-07-04, commit 3c0a9ce, 464 in 4:44):
both recorded exceptions fall to the paper's own forms. Cyclic
self-supporting heads sweep empty-first and rebuild to the local
least fixpoint (a stale x-y-x cycle cannot keep itself alive);
fully-derived aggregate heads replace whole on FULL derives so a
vanished group's fold dies, while incremental calls keep per-group
supersession for asserted survivors. The counting algorithm remains
the library's one ungrounded concept as the EFFICIENCY alternative,
priced by nothing in the corpus yet. The derive semantics are
complete and sourced: monotone closure (Bancilhon-Ramakrishnan),
aggregate supersession with honest group death, keyed upsert
(task-955), acyclic sweep and cyclic rebuild (GMS93), materialization
never ground truth (Codd 1.5). FOURTEEN commits this session.

THE CLEANUP CLOSES (2026-07-04, commit 3697466): the seed dispatches
five kinds and no more, refusing the rest visibly; the escape hatch
is dead; compile_model routes selfhost unconditionally. The surgery's
lessons are in the message: _CLASSIFY is the shared production
registry AND the bootstrap classifier, so the seed half restricts
while the table stays whole (a textual trim killed the translators'
regexes; an analyze-level filter poisoned the prepass's known-map,
caught by the coercion tests). Two guards rode along: negative-
modality statements never decay into junk fact types through the
generic fallback, and the dispatch carries per-statement modality
signs. Gate: 462 in 5:07 parallel. Samuel's arc from "duplicate
stale branch" to this commit: the flip, twenty gaps closed by
measurement, three-leg acceptance, the readings prose audit, and
the deletion. THIRTEEN commits this session. REMAINING THREADS: the
counting algorithm (GMS93's delta form, the library's one ungrounded
concept), deeper cross-host differential coverage for the C# and
Java kernels, the swap-time re-authoring list (72 prose ids, the two
reflection-reading rules, the rooted-defect call), and the swap.

THE GATE GOES PARALLEL (2026-07-04, Samuel's yuck at 19-minute
suites): pytest-xdist 3.8.0, -n auto, 462 passed in 3:01 with zero
isolation breakage (the frozen-cache writes were already
tmp-then-rename racing-safe). Gates run parallel from here; -n auto
stays opt-in so single-test debugging keeps clean output. Landed
this window: b7d0ef9 the readings cleanup (34 flagged prose
statements to zero, the engine auditing its own inputs), e528190 the
Java host. Twelve commits total this session. NEXT: the seed
deletion at its five-kind specification, first customer of the
three-minute gate.

THE FOURTH HOST BREATHES (2026-07-04, Samuel provided the JDK path):
java/ loads all 106 canonical definitions from the same bytes on
Java 8 (Vocab.java static-imported by the generated Canon.java;
gen_canon.py is the include! equivalent, byte concatenation only;
the one Java lesson: static imports refuse the default package, so
the pair lives in package arestlam). The reducer port from
Reducer.cs is mechanical and pending; acceptance extends
tests/test_csharp_kernel.py's pattern to a four-way agreement.

THE FLIP IS ACCEPTED, AWAITING THE PIN (2026-07-04): all twenty gaps
closed; the definitive suite reads 460 green with the selfhost as the
default compiler; the fleet differential reads 31/31 cells equal on
tasks WITH THE SELFHOST FASTER THAN THE SEED (46.4s vs 47.9s; the
3.1x is gone, compounded away by batched asserts, twins, dirty
filtering, and the bootstrap pin); and verdict thirteen reads 24/25
IDENTICAL to the seed's best, rooted the lone residual. The commit is
staged (seven files, MSG_FLIP2 in the scratchpad) and timed out on a
COLD GPG PIN; it fires on pin-warm, and the seed-deletion arc follows
it. The remaining nine wirings this arc: decline tracking, the Prose
yield set, the plain seam, frequency widening, more-than-one,
Disjunction, Consequence, Extraction unpadding, Relative Pronoun,
Data Type Prefix, Objectification Prefix plus its translator
registration.

TRIAGE SCOREBOARD (2026-07-04, running): EIGHT OF TWENTY CLOSED.
Landed: decline tracking in the selfhost dispatch (a statement all of
whose translators decline falls back to the report; detection is the
D-identity check per statement, sound because handlers answer the
same D on decline BUT NOTE a diagnostic assert counts as a change);
Prose beats the rule claim in the dispatch yield set (the seed's
prose-suspect guard; the grammar-file negation route is blocked until
class twins learn negated clauses, the recorded follow-up); the
plain set rides the ctx seam so a plainly-declared head never earns
the rule's derivation kind (protects stored-state cells from the
sweep); the frequency classification widened to each+bound forms
(line 258 required at-most AND at-least, matching only ranges); and
'more than one' joined the Quantifier enum with a Uniqueness
classification (the impossibility prefix is MODALITY, stripped before
classification, so the inner statement carries the tell). REMAINING
TWELVE: inclusive-or needs a Stage-1 disjunction field (new emission
plus grammar plus translator wiring, the next-window opener), then
article-free clauses, roleless fact types, derivation-rule reading
form, federation namespacing, implicit-noun prose interaction, meta
frontier rules, norma ingest aggregate, seal pair, subset pair,
permuted-head copy check.

VERDICT TWELVE + THE TRANCHE LANDS (2026-07-04): 24 of 25 exact.
Every projected heal from the analytics arc landed: Agenda_ranks 5=5
(join guard), worst/slowest/regresses 1=1 (mixed-cell coercion),
peak and dominates content-exact (numeric ordering), recommended and
elected exact. The single residual is Status_is_rooted 23v4, the old
engine's documented seed-branch defect, wrong by its own transition
data. Gates: suite 453 green, fleet differential green (one
documented tasks prose residual), cargo green. Committed signed on
pin-warm: c4f0065 (flip fixes, arbitration, join guard), 3523e97
(coercion in arithmetic AND comparison, all three evaluators),
d12bf13 (the joint fixpoint with DRed sweep and dirty filtering),
0e5a0a8 (GMS93 filed in infosci; reprojection ALTERs new columns).
Commit messages carry no em-dashes per the Operating Rule
no-emdash-no-fragments, which covers commit messages and repo docs,
not just replies.

SAMUEL'S ARCHITECTURE TEACHING (2026-07-04, filed as Operating Rule
defs-override-glue-framework in the claude app): a platform-specific
interface implemented isomorphically for performance may be
registered in DEFS and override ANY part of the system. AREST itself
is fundamentally a framework that applies (glues) functions in a
parallelized pipeline based on data, applying functions only where
necessary for correctness. Backus FFP read as a scheduler, joined to
GMS93's only-where-changes-demand. The seams that already realize it:
defs.native, rule twins, the evaluator seam, dirty-set filtered
strata. It NAMES the thin-runner endgame: hosts are orchestration
shells over the canonical pipeline description, and a new host earns
performance by overriding exactly the parts it cares about. The
system.py builders migration (F_of, derive_of, join_rule to shared
canonical defs) is the next concrete step of exactly this, and the
parallelizable-pipeline reading suggests the within-pass evaluations
in run_rules (independent heads over the settled store) are the
natural unit for host-level parallelism when a host wants it.

OPEN DESIGN (the residual prose class): the word-level unresolved-token
rule and IMPLICIT-NOUN MINING are in tension — mining legitimizes every
Title-case run in punctuation-less statements (NOTE and CONSEQUENT
become nouns by occurrence before any rule could flag them). Candidate
boundaries, to be designed not hacked: mining requires a second
corroborating occurrence (a noun is a name USED, not a word that
appeared once); or mining only from statements that carry another
recognizer classification; or the unresolved test runs against
declared-plus-corroborated names only. Evidence either way lives in the
fleet: one residual paragraph on tasks, zero everywhere else.

5. APPS/PERSIST: I/O binds as DEFS-named operations (the federation
   module's Connector pattern states it: any backend is one more
   Connector declaring its two names).
Also queued here: meta.py migration, differential scenarios as shared
data.

## The column-naming recipe (Codd 1970 §1.3, confirmed in the old schema)

Codd prescribes role-qualified naming (R.r.d; his sub.part/super.part), and
the old engine implements it: transition absorbs its two Status roles as
from_status_id / to_status_id (qualifier from the reading's predicate
words), single-role FKs are plain player_id
(state_machine_definition_id), the PRIMARY KEY column is bare id, and the
self-ring falls back to positional (task_id, task_id_2) exactly where no
distinguishing text exists. The DDL naming tranche that closes most of the
projection layout drift: _key_col -> id; absorbed ref columns ->
[reading-qualifier_]player_id (qualifier = the last non-copula predicate
word before the role's placeholder, applied when two roles share a
player); own-table role columns player_id with positional suffixes for
rings; value columns stay the value-type name. Old projected tables are
the oracle, table by table.

## 2026-07-04: THE SWAP DECISIONS LAND (Samuel, answered directly)

Four questions, four answers, and the critical path reshapes.

1. Old defects: ACCEPT BOTH RE-BASINGS. The rooted populations pyarest
   derives are the record (the old engine's over-derivation dies with
   it), and arity stands at 405 as the honest self-description. The
   rehearsal reports remain the audit trail.
2. Re-authoring the claude app's 72 prose ids: POST-SWAP,
   AUTONOMOUSLY. Migrate as-is, then re-author through the new
   engine's apply with a gated report per batch. Nothing blocks the
   swap on this.
3. Swap surface: WAIT FOR THE RUST RESIDENT. No Python stopgap. One
   swap, straight to the engine of record. This makes the resident's
   apps-registry half plus its MCP binding THE critical path.
4. Fleet scope: LIVING APPS ONLY. claude, tasks, spd-1 and kin migrate
   with verification; the arc-* probes and benches die with the old
   repo.

## 2026-07-04: THE TRIGGER NOTE CLOSES BACKWARDS; actions IS COMMIT 29

The ledger's oldest pending note (smTrigger empty; the corpus says
'Event Type' where the production says 'Fact Type') resolved in the
opposite direction from its hypothesis. The corpus statements are NOT
machine wiring in a variant spelling. They are plain domain data,
populations of a user fact type whose second role noun is the implicit
multiword 'Event Type'. The proof was already in the tree twice: the
implicit-nouns test pins exactly this fall-through, and the tasks
rehearsal was clean without any trigger widening. The attempted
widening (production accepting either spelling plus a grammar
classification line whose literal teaches Stage-1 the phrase) vanished
the fact type and failed the gate at 487/488. Both halves reverted;
machine wiring keeps 'is triggered by Fact Type' as its own phrasing.
Commit 7d4d3c9 lands the actions verb (machine binding, current
status, legal event/to pairs from sm_triples) plus its test, and
nothing else, because the reverted files ended byte-identical to
4f5c176.

Lesson filed: a corpus phrase that LOOKS like engine vocabulary may be
user vocabulary; the deciding evidence is whether the rehearsal was
clean when the phrase fell through to the generic path.

## 2026-07-04: THE RESIDENT'S BOOT FOOD (the sidecar)

The swap-surface decision needs the Rust resident to load compiled
apps from disk without SQLite (the crate is zero-dependency by
design). The contract chosen: <name>.store.json beside the .db, whose
content is EXACTLY one serve-protocol set_store payload (d, process,
overrides, cases). Loading an app in Rust is therefore: read the
file, feed it through the same ingestion path a --serve stdin line
takes. Zero new ingestion code, and the encoding is the one the
differential already certifies byte-for-byte across hosts.

Python half landed TDD-first (tests/test_store_sidecar.py red, then
Registry._sidecar green): every snapshot site (compile and apply)
writes the sidecar atomically beside the .db, so the two artifacts
stay in lockstep by construction. The .db remains the SQL surface;
the sidecar is the resident's. Recompile after a swap-day write
reconciles both from the event log, which stays the source of truth.

The Rust half (an --mcp mode: newline-delimited JSON-RPC 2.0,
initialize / tools list / tools call, the apps registry scanning
<apps_dir>/<name>/<name>.store.json, read verbs routed to the
existing op surface) is agent-authored in parallel, TDD against a
fixture sidecar. Write verbs and apps_compile stay off the resident
until the compiler question is settled (subprocess delegation to
Python is the candidate bridge, Python being the compiler tool the
resident applies where necessary for correctness, per the glue-
framework rule).

## 2026-07-04: THE MIGRATION MANIFEST (draft under the living-apps decision)

The fleet counts 106 apps with a .db. The living list, drafted from
recency, size, and purpose, migrates with per-app verification
(rehearse, diff, document):

- claude (the operational ledger, written today)
- tasks (the board)
- spd-1 (rehearsed clean at 5/6 verdicts, the sixth being the rooted
  re-basing Samuel accepted)
- kernel
- support.auto.dev (173 MB, the largest store in the fleet)
- message-vetting
- bill-negotiation-service (a zero-byte .db dated 2026-07-03: a
  fresh stub whose readings may still matter; Samuel confirms or it
  dies)
- arc-stack (flagged, not defaulted in: recent and 20 MB, plausibly
  the current ARC working app; new ARC work targets pyarest either
  way)

Everything else dies with the old repo: the 56 arc-* probes, the
eight spd-* single-day aspect probes from 2026-06-14 (spd-1 itself
lives), the gen-* and induce-* generator experiments, and the tail
of demos and one-off checks (maj-demo, alpha-rule-test,
freewill-repro, agent-policy, agent-action-governance,
csdp-action-model, csdp, qvr-test, agg-count-check, bisect-samekey,
engine-migration, paper, blocked-proto, merge, identity, arest-dev,
safety-probe, deriv-probe, codex, load-src-do, listings-vdp). The
old repo's archive branch, if Samuel wants one, preserves them all
anyway.

Manifest correction, same day: bill-negotiation-service is LIVING,
not a stub. Its real store is app.db (9.2 MB, 2026-06-07, the older
single-file naming convention) with four substantive readings (bill
disputes, consumer rights including the No Surprises Act, provider
obligations, negotiation process) and its own .git. The zero-byte
bill-negotiation-service.db dated 2026-07-03 is an aborted artifact
of the newer <name>.db convention. Swap-day note: this app migrates
from app.db, and the migrate tool needs to accept the older name.

## 2026-07-04: THE RESIDENT SPEAKS MCP (agent-authored, re-verified, committed)

arestlam --mcp --apps-dir <path> exists: newline-delimited JSON-RPC
2.0 over stdio (initialize echoing the client's protocol version,
notifications consumed silently, tools list, tools call), the apps
registry scanning for <name>.store.json sidecars, and the read
verbs (orient, apps_list, apps_current, apps_use, query, cells,
synthesize) routed through a new op_answer seam that the --serve
envelope also rides, byte-identical, so serve_ops still passes.
apps_use feeds the sidecar through the exact ingestion path a
--serve line takes. The integration test drives the protocol end to
end over a fixture sidecar generated by Registry._sidecar itself.

Seams surfaced by the work:
1. The hand-rolled parser P lacked true/false/null, which every MCP
   client sends inside initialize capabilities. Fixed with J::Null
   and J::B variants plus a catch_unwind guard so malformed lines
   cannot kill the loop.
2. OPEN, the next parity thread: synthesize_pairs answers null
   pairs over Python-compiled stores because compiled factType
   names (Ticket_has_Status) differ from absorbed cell names
   (Ticket_status). The canonical verbalize needs the ft-to-cell
   resolution (system:ftpop_absorbed reassembly) on the Rust path
   before synthesize parity holds over real apps. Python's
   synthesize handles absorption; the resident's does not yet.
3. rust/target was tracked since an early-session add (21 files
   including the release exe). Untracked and ignored in the same
   commit.

Remaining before the resident is the daily driver: the synthesize
absorption seam, apply/retract (needs incremental run_rules on the
resident plus event-log append in the Python-compatible envelope),
apps_compile delegation (subprocess to the Python compiler, the
glue-framework pattern), and repointing the MCP config. Reads are
done.

## 2026-07-04: THE SYNTHESIZE SEAM DIAGNOSED (both hosts, one defect)

The resident agent's name-seam theory refines to a proven root cause,
and the empty-versus-bottom host divergence DISSOLVES: there is no
divergence. Python's raw verbalize over a Registry store is bottom
exactly like Rust's; Registry.synthesize's isinstance guard was
dressing the marker as an empty facts list.

The mechanism, probe-proven on the flow fixture: the app store's
factType cell holds the app's own fact types with their reading
templates (⟨Ticket_has_Status, '{0} has {1}'⟩ present and correct).
system:verbalize then DynFetches each fact type's population BY THE
FACT TYPE KEY. Ticket_has_Status has no cell of its own because the
population absorbed into Ticket_status (the layout rule: a
single-role UC makes it functional, absorbed into role-1's table).
The missing-cell fetch answers bottom, and the section 11.2.1 bottom
discipline collapses the whole verbalization to bottom. The
shared_builders verbalize test passes because a raw compile_model
store keeps populations under the fact type key, so the fetch never
misses.

The fix is the next canon arc, not a patch: verbalize's per-ft fetch
must ride the layout, which means a canonical twin of engine.py's
ftpop_expr dispatch (own-table reads the ft cell; absorbed
reassembles via system:ftpop_absorbed over ⟨table, col⟩). The
partition and column are derivable inside the canon from the role
and UC M-facts already in D. Deliverables: system:ft_table (or
equivalent dispatch builder), vb_matched rerouted through it, the
twin test extended with a Registry-shaped absorbed store, a scenario
case pinning the dispatch cross-host, and the resident's serve probe
answering real pairs over the fixture sidecar.

Separate defect, filed not chased: the BASE grammar store
(ingest_frozen's D) carries a poisoned row inside its own factType
cell, a raw Python None embedded in the Scott structure, which
TypeErrors any fold that walks base's factType (nothing in
production does; app compiles rebuild the cell clean). Suspected
mint: a seed production handler writing an optional regex group that
did not match. Deserves a probe of the frozen snapshot's encoding
before the swap.

## 2026-07-05: THE VERBALIZE LAYOUT ARC COMPLETES; THE CASE TABLE CATCHES ITS FIRST FLEET GAP

The synthesize seam closed in one arc, TDD end to end, and the fix is
canonical, not a patch.

The canon: system:vb_colrow (the rmapColumns lookup, the quasiquoted
eq-on-column-3 predicate carrying the fact type into the filter) and
system:vb_fetch (COND on null hits: the own-table branch curries
ast:FetchPop over the run-time name, TOTAL by FetchPop's own COND-on-#
so a declared-but-unwritten fact type contributes empty instead of
bottoming the fold; the absorbed branch double-applies
system:ftpop_absorbed to the run-time ⟨table, col⟩ from the hits row).
vb_matched's fetch leg swapped from raw ast:DynFetch to
system:vb_fetch, one atom in the constant piece.

The host half: engine.layout_cells materializes rmapColumns
(⟨table, col, ft⟩ per absorbed fact type) as a store cell at
Registry.compile time, facts all the way down: the partition is
knowledge about the store, so it rides IN the store, and every host
reads the same data. A store without the cell reads as all-own-table,
which is exactly what a raw compile_model store is.

Proof chain: the unit twin (absorbed reassembles, own-table fetches
totally, absent answers empty); the integration (a Registry app with
uniqueness constraints, the absorbed Status pair verbalizing, the
unwritten Note contributing nothing, holes not verbalizing); the
resident probe over the regenerated fixture answering real pairs
byte-agreeing with Python; and eight new scenario cases pinning the
chain cross-host, 47 total.

THE CATCH: the new cases found the managed hosts missing the cellkey
boundary prim entirely (bound in Python and Rust when the ftpop
family landed, never mirrored into C# and Java). Both reducers gained
the twin (strings pass, integers stringify, else bottom) and all four
hosts agree on the full table. The bisect rode the case table itself:
dynfetch and vb_colrow agreed, pairsel diverged, and pairsel's name
construction is cellkey. That is the case table doing exactly the job
it was built for, and the sub-chain cases stay in the table
permanently.

Latent edge filed, not chased: the Python cellkey accepts floats
(f-string) where Rust bottoms them; keys are strings and ints in
practice, so no case pins it and no behavior depends on it. Noted so
the next float-keyed surprise has a name.

The fixture regeneration recipe in rust/tests/fixtures/apps/flow/
README.md now compiles the constrained model (both fact types
functional, so absorption and the layout cell ride the fixture) and
applies to the DECLARED fact type; the old recipe's apply to a stray
undeclared cell name was hiding the whole seam from the resident's
tests.

## 2026-07-05: THE WRITES ARC OPENS (one write path through the swap)

The resident's write design, decided and half-landed: writes delegate
to the Python pipeline so there is exactly ONE write path during the
swap window (zero divergence risk while Rust lacks run_rules), and
the resident reloads the sidecar after each delegated verb so reads
stay hot and current. This is the glue-framework rule applied to
ourselves: the compiler host is a function the resident applies where
necessary for correctness. Porting the joint fixpoint to Rust is the
LATER perf arc, priced only after the swap.

Landed TDD-first: cli.py at the repo root, the one-shot delegate. It
self-registers the package with the conftest bootstrap (no install,
no cwd assumption; the package is NOT bare-importable, probed and
confirmed), runs exactly one Registry verb per invocation (compile,
apply, retract), prints one JSON receipt on stdout, and exits 0 on
commit or clean compile, 1 on refusal (the receipt still prints), 2
on usage error. tests/test_cli.py pins the contract including the
functional-refusal exit and the sidecar refresh. The frozen thaw
cache makes the subprocess compile cheap after first touch.

The Rust half (apply, retract, apps_compile tools spawning the CLI,
receipt as result, refusal as result not error, sidecar re-ingestion
through the apps_use path, python-and-cli discovery by exe-path
walk-up with argv overrides) is agent-authored in parallel with an
end-to-end write-flow test that materializes a real app through
apps_compile in a temp dir. After it lands, the resident's remaining
gap to daily-driver is exactly: the MCP config repoint plus the
optional long-tail verbs.

## 2026-07-05: THE BASE-ROT DEFECT RETRACTS (it was the probe, all the way down)

The 2026-07-04 entry filed a poisoned row inside the frozen base's
factType cell. Tonight's tolerant walk proves there is no such row
anywhere: all 196 frozen snapshots scan clean, and the "poisoned
store" was reg._base_D() correctly answering None for a Registry
constructed without a base_dir (the guard is right there at the top
of _base_D). My probes replicated compile's internals without
replicating its guards, fetched over the None store, and read the
crash as data rot. The tmp-Registry tests all compile base-free BY
DESIGN, which is also why the earlier "base rot" never bit anything
real.

Lesson: a probe that replicates a pipeline's internals must
replicate its guards, or the probe's own crash becomes a phantom
defect. The pre-swap checklist loses the frozen-snapshot probe; it
was already clean.

Same day, the pin rhythm: two verbalize-commit attempts died at
gpg's own pinentry timeout (exit 128, signing failed: Timeout) with
Samuel away. The staged eleven hold; the relaunch waits for the next
keep-alive. Signing is never bypassed.

Same day, the long tail joins the delegate: cli.py grows the read
verbs (get, schema, sql, explain, validate, verify, actions,
synthesize) as thin one-shot delegations to the same Registry
methods the Python MCP server dispatches, outputs passing through
as the methods answer them. The test pins get's entity view,
schema's fact-type inventory, sql over the projected .db, and
validate's clean bill. With these, the resident can expose the
ENTIRE daily-driver verb table on day one: hot reads native (query,
cells, synthesize, orient, apps family), everything else
correct-by-delegation, canonicalized verb by verb as usage demands.

## THE SWAP-DAY RUNBOOK (drafted 2026-07-05; every step's tool exists)

Preconditions, all met or in flight: reads native on the resident;
writes and the long tail delegated through cli.py; the sidecar at
every snapshot site; the migration manifest decided (living apps
only); the four decisions filed. Pending at draft time: the writes
arc landing on the resident, and the MCP config repoint.

Per living app (claude, tasks, spd-1, kernel, support.auto.dev,
message-vetting, bill-negotiation-service, arc-stack if Samuel says
so), in order:

1. REHEARSE: migrate.replay_into from the old .db (app.db for
   bill-negotiation-service) into a scratch pyarest app; run_rules to
   the joint fixpoint.
2. DIFF: the verdict tooling (cells against the old .db, the
   documented-defect allowlist: rooted over-derivation, arity 405,
   the two tasks defects). Any NEW divergence stops that app's swap
   and files a verdict, exactly as claude 24/25 was built.
3. CUT OVER: copy readings/ into the pyarest apps dir; compile
   (builds .db, sidecar, rmapColumns); replay the old events log if
   one exists; re-run the diff on the final store.
4. VERIFY THE SURFACE: orient, query, cells, synthesize, get,
   actions through the RESIDENT against the migrated app; one write
   through apply and its refusal path; explain on one derived fact.

Then, once per machine:

5. REPOINT: the claude.ai MCP config swaps the old arest-cli entry
   for arestlam --mcp --apps-dir <apps>. The old server stays
   runnable but unreferenced.
6. SOAK: normal use for a session or two; the old repo untouched.
7. ARCHIVE AND NUKE: an archive branch or tag of the old repo
   (Samuel's call on which), then the deletion he named as the end
   state. The probes and benches die here.
8. POST-SWAP: the re-authoring pass over the claude app's 72 prose
   ids, autonomously with a gated report per batch, per decision
   swap-reauthor-prose-ids-post-swap-autonomously.

## 2026-07-05: THE WRITES ARC COMPLETES (verified first-hand)

The resident carries apply, retract, and apps_compile as MCP tools
delegating to cli.py: python and cli discovered by exe-path walk-up
with --python and --py-cli overrides, receipts answering as results
on both commit and refusal, protocol errors reserved for spawn
failures and crashes (the parse gate re-serializes stdout so a
crashed CLI with exit 1 cannot masquerade as a refusal; tracebacks
route to -32603 with the stderr tail), and the sidecar re-ingesting
through the SAME path apps_use takes, factored so the reload is
literally the boot ingestion. The integration test materializes a
real app end to end in a temp registry: compile from readings, two
commits, a functional refusal riding as a result, a retract, and
query answering the reloaded truth after every step. cargo test 3
passed, release clean, and the failure paths smoke-driven (bogus
python, bogus cli, missing fact each answer their distinct error).

With this, the resident's verb surface is COMPLETE for the swap:
orient, the apps family including compile, query, cells, synthesize
native; apply and retract delegated; and the read long tail ready in
the CLI for exposure as demanded. The remaining swap-blockers are
exactly: the config repoint (runbook step 5) and the per-app
migration rehearsals (steps 1 through 4). Both commits (verbalize,
writes) wait on the pin.

## 2026-07-05: MESSAGE-VETTING REHEARSAL, VERDICT ONE (runbook steps 1 and 2)

The rehearsal crashed, then taught, then ran clean. The crash chain:
'API Product is a subtype of API.' declares a noun _known never
collected, so 'Message names API Product by Field Name' mined two
roles where the old engine's schema is ternary (message_id,
api_product_id, field_name); the deontic statements then matched the
malformed reading as one-quote instances, minted width-1 rows, and
ddl.project crashed on binding arity. THREE fixes landed TDD-first:

1. _known collects subtype clauses (both names) and brace subtype
   groups. The old ternary schema is the oracle.
2. The negative-modality guard widens to ALL deontic modality: a
   deontic statement is a constraint by definition (ORM: modality
   qualifies constraints) and must never mint instance rows through
   the generic fallbacks. The old store's EMPTY population for the
   fact type is the oracle.
3. ddl.project skips rows narrower than the role count and reports
   them in the count envelope instead of crashing the projection.

After the fixes the rehearsal completes: no derived populations to
verify (the app is asserted config plus rules), asserted rows
migrate as log entries, the reflection exclusion holds (the old
metamodel cells stay behind by design).

THE REAL VERDICT, the swap-blocker for this app: message-vetting's
core semantics are 18 deontic vetting rules ('It is forbidden that
Message contains Markdown Syntax.', 'It is obligatory that Message
conforms to Pricing Model.', the quoted-value pair on Field Name),
which the old engine compiled into its constraint layer (67
Constraint rows in the old store) and pyarest currently reports
unclassified. The deontic constraint family over plain propositions
(forbidden = the population must stay empty, flagged not blocked;
obligatory = the membership must hold; both over unary, binary, and
quoted-value forms) is the next named arc, with the old store's
constraint rows as the target semantics. Until it lands,
message-vetting migrates mechanically but does not vet.

## 2026-07-05: THE DEONTIC ARC'S TARGET SEMANTICS (read off the old store)

The old engine's encoding for a deontic vetting rule, probed from the
message-vetting snapshot's Constraint cell (591 records, 7 relevant):

1. The inner proposition DECLARES its fact type into the schema
   (Message_contains_Markdown_Syntax, roles Message and Markdown
   Syntax, population empty). The deontic statement teaches shape,
   never membership.
2. One constraint record per rule: kind UC, modality deontic,
   deonticOperator forbidden|obligatory, text = the full statement,
   entity = the subject noun, span0 = the fact type plus role index.
   Quoted-value forms (Field Name 'Title') keep the quote in the
   text and span the same fact type; the quoted value types also get
   alethic VC records.
3. The DF_cwa / DF_owa / DF_pop / DO_obl / DO_pop / DO_sender kind
   family in constraint_kind is the VALIDATOR'S vocabulary (how
   validate interprets a deontic row at vetting time, including the
   open-world 'Response may violate: {text}' flag an LLM judges),
   not the storage encoding.

Implications for pyarest's arc:
- The deontic guard landed today is half of the truth: it correctly
  stops instance rows, but the arc must still declare the inner
  reading's fact type (the mv schema diff would otherwise show those
  fact types missing against the old store).
- The translator mints: the fact type (via the normal reading path)
  plus a deontic constraint M-fact ⟨text-id, operator, ft, role
  span, quoted value if any⟩.
- validate interprets: forbidden = flag when the span population is
  non-empty (population kind) or when the quoted value appears
  (closed-world kind); obligatory = flag when the required
  membership is absent; deontic NEVER blocks (Def. Violation:
  alethic blocks, deontic flags).
- The old engine's own constraint table rows for mv verify the
  translator: 18 statements, each answering one deontic record and
  one declared fact type.

## 2026-07-05: THE DEONTIC TRANSLATOR LANDS (the arc's first half)

TDD-first against the old store's records: three tests pin the
unquoted declaration (Message_contains_Markdown_Syntax with roles
and empty population plus the deontic_forbidden row), the quoted
pair (Title obligatory, EndpointSlug forbidden, values riding the
rows, population empty), and the quantified obligation (the
each-strip: the shape is Message_is_natural, the constraint text
keeps the statement).

The implementation is one transform in _plan plus sign plumbing
through the operand's modality field; no handler changed, the
negative-alethic guard unchanged, and the earlier
never-mint-instance-rows pin holds through the redesign.

Corpus proof: message-vetting compiles with ZERO unparsed (from 18),
21 deontic rows, ids and spans agreeing with the old snapshot record
by record. Validate skips deontic rows gracefully (probed: no crash,
forbidden applies commit, no false flags), so this half is safe
alone.

THE REMAINDER, named: the interpretation half. validate must flag a
non-empty forbidden population ('Forbidden fact present in {ft}', the
DF_pop kind), a present forbidden value (DF_cwa), and an absent
obligatory membership (DO_pop and kin), always flagging and never
blocking. The old constraint_kind cell carries the violation
templates verbatim. After that, the mv rehearsal re-runs and its
verdict should read: migrates AND vets.

## 2026-07-05: THE FORBIDDEN INTERPRETATION LANDS; MESSAGE-VETTING VETS

The arc's second half, same window: engine.deontic_forbidden builds
the check objects (population form = the identity over P, every row
of a forbidden population violating and an empty population
answering nothing; closed-world value form = sigma over rows that
theta:setminus leaves changed, exactly the rows carrying a forbidden
value), _plan defines them at translate time like every other
constraint object, and _ATTACH routes deontic_forbidden local
through validate_modal, so flags carry alethic false and never
block. Two tests pin the behavior: the committed forbidden fact
flags on validate, and the clean row beside the offender stays
clean.

The crown proof, end to end on the real corpus: the message-vetting
rehearsal diverges NOWHERE (zero derived mismatches), and the
vetting sweep over the migrated store answers a clean bill,
consistent with data the old engine itself vetted. The app's verdict
upgrades: message-vetting migrates AND vets. Its swap-readiness
remainder is exactly the obligatory interpretation (DO_pop and kin),
whose absence only under-flags; the ledger names it as the arc's
final piece.

Addendum, same window: the obligatory VALUE form landed too
(engine.deontic_obligatory_value, the setminus predicate with its
polarity flipped: rows lacking every obligated value flag), pinned
by a sixth test (Title conforms, Docs flags, nothing blocks). The
arc's remainder narrows to the BARE obligatory form alone (the
per-subject mandatory shape, old kind DO_obl). The frozen gate that
covered the forbidden half hung at its tail and was killed unread;
the arc's final gate covers translator plus both interpretations in
one run.

Second addendum, same window: THE ARC COMPLETES WITH NO REMAINDER.
The bare obligatory form landed as one _mandatory_parts call with
deontic modality (the seventh test: the subject missing its
obligation flags, the conforming one stays clean, nothing blocks),
so the old kind vocabulary is fully interpreted: DF_pop, DF_cwa,
DO_pop, DO_obl. DF_owa (the open-world 'may violate' judgment) is
the caller's layer by design, and DO_sender was an old app-specific
special the corpus no longer carries. The corpus re-verifies clean
under the complete family: zero divergence, zero sweep violations.
Message-vetting's swap verdict is now unconditional: migrates and
vets.

## 2026-07-05: KERNEL REHEARSES CLEAN (runbook steps 1 and 2)

Three readings compile, the old store's asserted rows replay, zero
derived divergence, zero sweep violations. Kernel's swap verdict is
unconditional on the first run; the manifest's remaining rehearsals
are support.auto.dev (the 173 MB heavyweight) and
bill-negotiation-service (under its older app.db name), launched in
sequence.

## 2026-07-05: BILL-NEGOTIATION-SERVICE REHEARSES CLEAN

Four readings compile (bill disputes, consumer rights, provider
obligations, negotiation process), the old app.db's asserted rows
replay through the older-name path the manifest flagged, zero
derived divergence, zero sweep violations. The verdict is
unconditional on the first run. The manifest's rehearsal column now
reads: claude, tasks, spd-1 verdict-certified with the accepted
re-basings; message-vetting, kernel, bill-negotiation-service
unconditional; support.auto.dev in flight; arc-stack on Samuel's
call.

## 2026-07-05: SUPPORT.AUTO.DEV REHEARSES CLEAN; THE COLUMN CLOSES

The 173 MB heavyweight compiles its six readings, replays, and
answers zero derived divergence and zero sweep violations. With it,
every living app on the manifest has rehearsed: claude, tasks, and
spd-1 verdict-certified earlier with the accepted re-basings, and
message-vetting, kernel, bill-negotiation-service, and
support.auto.dev unconditional on first runs. Only arc-stack waits,
on Samuel's include-or-drop call.

Honest depth note: the four first-run verdicts are REHEARSAL-level
(compile, replay, derived-population verify, deontic sweep). All
four apps report zero fully-derived fact types to verify, so their
derived columns are trivially green; asserted parity rides the
replay mechanism's set semantics. The big three got the RICHER
cell-differential harness (old-versus-new per cell, verdicts 12
through 15). If Samuel wants the deep differential on any of the
four before cutover, the harness exists and the runbook's step 2
names it; the light rehearsal is otherwise the agreed gate.

THE SWAP'S ENGINEERING SURFACE IS COMPLETE. Remaining are exactly
the three human calls: arc-stack's fate, the MCP config repoint
(runbook step 5), and the old repo's archive-or-delete (step 7),
plus the post-swap re-authoring pass that follows the cutover by
decision.

## 2026-07-05: THE DEEP DIFFERENTIAL UPGRADES ALL FOUR VERDICTS; NO DATA LOSS

The per-cell differential ran over all four persisted rehearsal
scratches with the migrate planner's own buckets. Every compared
asserted and stored-state cell is fully contained in the migrated
store: message-vetting 2/2, kernel 1/1, bill-negotiation-service
2/2, support.auto.dev 24/24, zero missing rows.

The large unknown buckets (54, 136, 137, 534) are FOSSILS, proven by
sampling: populations whose declaring readings were REMOVED from the
apps (none of the sampled shapes appear in any current readings)
plus old-base bleed (Android_View_Type rides kernel AND
bill-negotiation-service). Readings are the source of truth, so the
fossils are unreachable in both engines and die with the old repo by
the same principle that kills the probes.

The unparsed buckets intersected with live fact types answered ZERO
for three apps and TWO for support.auto.dev
(Customer_accepts_current_Terms_Of_Service,
Customer_subscribes_Subscription_to_Plan), which probed to the old
engine's EMPTY-population phi form ('key=φ') with zero rows in their
SQL projections: no data loss, a notation quirk. parse_cell now
reads the phi form (an empty entry contributes no rows, the
round-trip proof running over the phi-reduced remainder), pinned by
a test carrying the exact live cell shape, and the migrate suite
holds at eight green.

With this, the fleet-wide migration integrity statement is: every
live cell either migrates row-for-row or is a proven-empty phi
entry; every dropped cell is a fossil of a removed reading or the
old base; and the reflection layer stays behind by design.

Board note for Samuel (read-only observation, 2026-07-05): the tasks
board carries two p0 migration items from the OLD engine's roadmap
(932-4-w7d-migration-harness, the fold-on-load test-on-copy harness;
ns-cell-key-migration, the namespace re-key). Both are OBSOLETED by
the swap rather than completed by it: pyarest's migration rides
replay_into with its own scratch-copy discipline, and the old cell
keys die with the old store. Closing or keeping them is a board
call, not mine.

## 2026-07-05: SAMUEL'S CORRECTION; THE PUNCHLIST ARCHIVES

Samuel: the old repo cannot be deleted while its full punchlist is
unarchived in the new repo (OS, FPGA, Solidity, MCP, WASM, Cloudflare
Worker, and kin). The correction is right and the earlier "swap
engineering complete" framing was too narrow: complete for the
CUTOVER, not for the DELETION.

PUNCHLIST.md now sits TRACKED at the pyarest root, archived from a
direct survey of the old repo: the five-target portability matrix
(Cloudflare Workers live with Durable Objects and SSE, WASM via
wasm-pack, the x86_64 kernel booting under QEMU with the UEFI pivot
planned for aarch64, FPGA as the stated lowering goal, local CLI),
the Solidity generator's Foundry project, the TypeScript MCP server's
full verb surface, the REST HATEOAS and OpenAPI and SSE surface, the
generator family (OWL, XSD, EDM, HTML forms and kin), federation
connectors, the paper and its sources, the eighteen-doc reference
suite, the npm and GitHub identity, and the old repo's own apps and
reports. Each entry carries its pyarest disposition honestly: most
are NOT PORTED.

The runbook's step 7 gains the precondition: deletion only after
every punchlist entry is ported, re-homed, or explicitly waived by
Samuel. The cutover (steps 1 through 6) is unaffected.

## 2026-07-05: THE COUNTING ALGORITHM GROUNDS (the library's last orphan)

The one honestly-ungrounded infosci concept resolves: the counting
algorithm's primary source is the SAME paper the engine already
implements half of. Gupta, Mumick, Subrahmanian, Maintaining Views
Incrementally (SIGMOD 1993) gives BOTH maintenance algorithms:
DRed (delete and rederive), which run_rules' sweep implements for
the recursive case, and COUNTING (per-tuple derivation counts,
decremented on delete), the cheaper alternative for non-recursive
views. Filed in the claude app: Source Doc
gms93-maintaining-views-incrementally (canonical reference, not held
locally) and Engineering Principle
counting-algorithm-for-nonrecursive-view-maintenance citing it.

The pricing question stays open and honest: nothing has measured
whether counting would beat DRed on the fleet's actual rule
corpus (most fleet rules are non-recursive, so the candidate set is
large). It is a PERF lever to price only after the swap, per the
same discipline that defers the run_rules port to Rust. Nothing in
correctness depends on it; DRed is sound for both cases.

Refinement, same day: the fossil claim re-checked against the
generator question. The four rehearsed apps' unknown buckets contain
ZERO generator-prefixed cells (588 bare names on support.auto.dev,
all genuine fossils or junk shapes), so their integrity statement
stands as written. The generator projections (owl:, xsd:, dsl:,
html: and kin) live in the CLAUDE app's store specifically, from an
era of broader generator opt-in (the current compile profile opts in
sqlite only). Consequence: the punchlist's generators entry bites at
exactly one cutover, claude's, where those cells regenerate only if
the generators port, or are waived as stale projections Samuel can
regenerate later. The other six apps cut over without touching the
question.

## 2026-07-05: RUNBOOK STEP 4 DRY-RUNS ON A REAL APP (the resident serves kernel)

The cutover claim upgrades from test-verified to execution-verified:
the release resident booted the kernel rehearsal scratch (the real
12 MB store's sidecar through the apps_use ingestion), answered the
native reads (apps_list, orient, cells reading 88 fact types), and
bridged the delegated reads through cli.py (sql answering the real
projected tables, schema the real model surface, validate a clean
bill over the migrated store). This is the first time runbook step 4
ran against a real app rather than the toy fixture.

One UX note, not blocking: a syntactically bad SQL statement crashes
Registry.sql, so the resident answers a protocol error carrying the
traceback tail (the loop survives and the error is legible). The old
MCP answers an error envelope as a result instead; teaching cli.py
to catch per-verb user errors and answer {"error": ...} at exit 0 is
a small polish for the demanded-verbs pile.

## 2026-07-05: THE ENVELOPE POLISH LANDS; THE FLEET IS 400X LIGHTER THAN IT LOOKED

cli.py read verbs now answer caller errors as {"error": ...}
envelopes at exit 0 (the old MCP's behavior), so the resident relays
a bad SQL statement as a legible result instead of a protocol error;
real crashes keep their nonzero exits. Pinned by the sixth CLI test.

Two cutover data points from the dry-run's extension:
1. support.auto.dev's MIGRATED sidecar is 388 KB against the old
   store's 173 MB: the live knowledge is roughly four hundred times
   smaller than the old store's bulk, which was overwhelmingly
   fossils of removed readings plus the old reflection layer. The
   fleet's true size after cutover is megabytes, not gigabytes.
2. The release resident boots the heaviest migrated app in 165 ms
   wall (spawn, initialize, apps_use ingesting the sidecar, and a
   cells read answering 221 fact types). Daily-driver boot latency
   is a non-issue.

## 2026-07-05: CLAUDE RE-REHEARSES EXACT; SYNTHESIZE DELEGATES BY MEASUREMENT

The daily driver re-rehearsed on today's engine: 21 of 21 derived
fact types exact, zero sweep violations, NO allowlist needed (the
earlier era's rooted diff is gone from the verify set). The sidecar:
444 KB against the 21 MB old store. The resident served the app's
own operational ledger, answering the four swap Decisions and
verbalizing the rust-resident decision through the very surface that
decision created.

The dry-run also caught the first demanded perf lever: the canonical
system:verbalize over the daily driver's 748-fact-type store reduces
in 264 SECONDS on the Rust path, where Python's native twins answer
in 10.9 s (the twins are a measured 24x on this fold). Every other
verb answers in sub-second time (616 ms for boot plus use plus query
plus cells). Per the glue-framework rule (apply the function where
necessary for correctness, override for performance where demanded),
synthesize MOVED from the native routing to the delegated read tail:
9.4 s wall on the daily driver including boot, the Registry's
rendered facts shape (richer than the raw pairs), and zero new
machinery. The mcp test's synthesize assertion moved into the
python-guarded write-flow accordingly. Plumbing the NATIVE CARRIER
into op_answer (the bench says 40x over canonical) is the priced
lever that brings synthesize home to the resident later.

## 2026-07-05: THE FRAMEWORK RESUMES; GENERATOR ONE LANDS (dsl)

Samuel's question (So the framework is done?) applied my own
scope-naming rule to me: the CUTOVER is done, the FRAMEWORK is not,
and the punchlist gating I had read as wait-for-demand was
over-conservative against the standing feature-parity directive. The
framework work resumes with the arc the cutover itself demands: the
generators (punchlist entry 8), whose absence bites claude's
cutover.

Generator one is dsl: per-noun model summary cells (noun, object
type, reading texts substituted from the role players, verbalized
constraints as kind-text pairs covering the functional family and
the deontic rows, machine transitions as trigger-from-to triples),
computed from M at compile beside the layout cells and replaced
wholesale on recompile, exactly the old engine's persistence shape.
TDD green over the flow model; wired into Registry.compile; nineteen
targeted tests green. The single-machine triple assignment is exact
for every app in the fleet today and documented for the
multi-machine refinement.

The family's remaining members follow the same pattern with the old
cells as field-wise oracles: owl, xsd, edm, html, dtd, wsdl, xforms,
plix, and the operational nav/resolve/create/update/list/get/
transition cells. Next: the claude-scratch field-wise differential
for dsl, then the members in oracle-richness order.

## 2026-07-05: PRIORITY IS DEPENDENCY-FIRST (Samuel); THE RUN_RULES ARC OPENS

Samuel sets the punchlist order: dependency-first. The tree has one
root: the Rust engine core. run_rules in Rust unblocks native writes
(today delegated), the native synthesize lever, and the resident's
independence from Python; WASM sits on the crate, Cloudflare on
WASM, FPGA on the pipeline-as-data endgame; REST sits on the verb
core (done); generators, federation, and the MCP long tail are
leaves. Correction absorbed on the way: the old stores persist NO
generator cells (the old profile opts only sqlite into persistence;
the dsl cells I saw were the live engine's runtime projections), so
the generator arc is pure runtime parity with the live engine as
oracle, and nothing generator-shaped gates any cutover.

THE RUN_RULES ARC, planned: the rule BODIES already evaluate
canonically (rule ids resolve through D's DEFS via rho; the canon is
the meaning), so the port is the SCHEDULER: the semi-naive loop
(round one full bodies bounded by the frontier, later rounds joining
per-head deltas through the ~d variants), the agg supersession
(whole-replace for fully-derived agg heads), the keyed per-key
upsert, the DRed sweep (fully-derived plain heads re-evaluate whole;
self-supporting heads empty-first and rebuild per GMS93), and the
iterated strata to the joint fixpoint (cap 12). Shape: Rust host
code mirroring python/engine.run_rules, spec'd by the paper
(Knaster-Tarski lfp; Bancilhon-Ramakrishnan semi-naive; GMS93 DRed)
and certified by a cross-host derivation differential: both hosts
ingest the same sidecar, run to the fixpoint, and every derived cell
compares. The serve protocol gains a run_rules op so the resident
can eventually derive natively; writes keep delegating until the
differential holds on the whole fleet, then flip per the one-write-
path discipline. The mirror blocks (instance and role, proposal B)
ride the port as written.

## 2026-07-05: RUN_RULES PHASE ONE LANDS; THE DIFFERENTIAL GOES GREEN

The naive positive-rule fixpoint runs on the resident: the run_rules
serve op and the derive MCP tool, the retain protocol replacing the
store, both proposal-B mirror blocks ported (empty cells only,
asserted wins), aggregates and uncompiled rules skipped exactly as
phase one scopes.

The port's load-bearing discovery: rule objects live IN D (the
compiler's DefineIn writes each compiled rule into the store
itself), so the Rust frame resolves rule ids through step_get
exactly as Python's defs.step does. No process-registry gap. The
agent's airtight proof empties a derived head on a Python-compiled
sidecar and watches the Rust mu re-derive it, so Python-compiled
rule objects genuinely evaluate under the Rust reducer.

Verification stack, all green first-hand: five derive tests plus the
updated mcp and serve_ops suites in Rust; the phase-two acceptance
(tests/test_derive_differential.py) running LIVE for the first time
and matching Python per head through set_store, run_rules, and
per-head queries; release build clean. Smokes over the rehearsal
scratches: mv instant, kernel 9 s, claude 30 s for one naive round
at 32 rules, the canonical-Scott cost that the later phases
(semi-naive deltas, frontier, native carrier) exist to cut.

Remaining phases, dependency order: semi-naive with the ~d variants
and frontier bounding, the agg supersession family, keyed upserts,
the DRed sweep, iterated strata to the joint fixpoint, then the
fleet differential over every rehearsal scratch, and only then the
write-path flip per the one-write-path discipline.

## 2026-07-05: ARC-STACK IS IN (Samuel's first standing call lands)

Samuel: he still wants to try getting an ARC score using pure AREST
with the Python driver, so arc-stack joins the living fleet. The
call closes the manifest's last flag and gives the
python-is-secondary-arc-compat rule its active mission: Python
exists for exactly this, and the ARC scoring thread rides the swap
rather than dying with the probes. The rehearsal launched (runbook
steps 1 and 2, the same tool as the seven siblings); the
arc-live-run-is-user-gated rule stands unchanged, so the scoring run
itself fires only on Samuel's word. Standing calls remaining: the
repoint and the archive-then-delete.

Same window: ARC-STACK REHEARSES CLEAN (eight for eight; one
readings file, zero divergence, zero sweep violations) and the BOOK
SOURCES ARE WAIVED (Samuel: public works, the whitepaper's
bibliography is the reference, his drive is the home). The
punchlist's first explicit waiver lands; the fleet manifest closes
completely; the standing calls are exactly two: the repoint and the
archive-then-delete. Decision filed
(arc-stack-is-in-python-driver-scoring), the swap-evidence rule
updated to eight-for-eight, the diagram redeployed.

## 2026-07-05: RUN_RULES PHASE TWO LANDS (semi-naive plus frontier)

The loop is semi-naive: ruleAtom rows carry (rule, position, atom
fact type) and the variant name DERIVES as {rule}~d{position} (the
agent followed the code where my brief mis-said the M-fact carried a
variant id); the operand is the pair of the atom's sorted per-round
delta and D; variant outputs union; atomless rules re-run whole when
their reads changed; the frontier bounds round one through the op's
new optional changed argument, mirrors running before it exactly as
Python. The adversarial variant test is the sharp one: a ~d1 that
REVERSES rows while the full body copies, so the fixpoint itself
proves which path ran.

Ten Rust tests green, the pytest differential green against the
semi-naive release, and the timing story lands where the theory says:
idempotent calls are one full-body round in both builds (~30 s at
claude scale, run-variance only), while the REAL delta call's round
two evaluated nothing and the frontier-bounded rederivation cut
39.9 s to 2.8 s, about 14x, by firing only the one rule reading the
changed cell. Phases remaining: agg supersession, keyed upserts,
DRed, strata, the fleet differential, then the flip.

## 2026-07-05: THE IFACTR STUDY OPENS (Samuel's directive)

Samuel: start an agent to understand how iFactr works. The family is
on disk (iFactr-Android with iFactr.Droid and iFactr.UI, plus the
iFactr-UI, iFactr-WPF, iFactr-iOS, and iFactr-NETCF siblings), a
C#-based abstract cross-platform framework running on mono, xamarin,
.net core, desktop, and compact framework, swapping per-platform
functionality through an IoC/DI container BY REGISTRATION. This is
prior art for the defs-override-glue-framework rule: the container
is DEFS, the abstract layer is the canon, platform assemblies are
the host kernels, custom renderers are native twins. The study agent
carries a seven-part brief ending in a pyarest seam map and three to
six lessons phrased as Engineering Principle rows citing the iFactr
repo as Source Doc; its report files on arrival.

Same window, Samuel's teaching lands the analogy precisely:
MonoCross is to iFactr what AREST is to ui.do. ui.do is the abstract
UI pattern OVER arest (found at the old repo's apps/ui.do, the
MIT-licensed React target; a broader UI workspace rides at
Repos/ui), and the target set is per platform: React shipped, Slint
for the OS, WPF for a Windows app. The punchlist gains entry 7b so
the nuke cannot orphan the React target silently, and the iFactr
study's report will be read with this mapping in hand: the
container against DEFS, the abstract controls against ui.do's
abstraction, the platform assemblies against the target renderers.

## 2026-07-05: RUN_RULES PHASE THREE LANDS (the aggregate stratum)

Provenance note first, per honesty: the phase-three agent (running as
Fable 5) hit its usage limit mid-task, leaving a near-complete
aggregate stratum in the working tree, coherent and faithful to the
Python spec, missing only its group_key helper. The session model
switched to Opus 4.8; I read the abandoned diff against
engine.py's agg block (lines 1210-1242) line by line, confirmed the
supersession semantics matched, wrote the one missing helper
(group_key over r[:-1] in key_of's encoding), and verified the whole
thing end to end. The iFactr study agent died read-only the same
way and owes its report; it relaunches on Opus.

The stratum itself: above the positive closure, each aggregate rule
(ruleAgg names them, so the closure never unions one) evaluates its
full body and its head SUPERSEDES. A fully-derived head on a FULL
derive (no frontier) is whole-replaced by the agg rows unioned with
its plain rules' rows, so a group whose supply vanished DIES, the
misfold the old engine documented and the paper's aggregate
prescribes against. Every other case supersedes per group (the group
is every column but the last, engine.py's r[:-1]): produced groups
replace, unproduced groups survive. Dirty-set gating keeps
incremental calls proportional; the loop iterates to a quiet sweep,
bounded at twelve.

Two adversarial pins carry it: a fully-derived head watching a
vanished group die on the next full derive but survive an
incremental one, and an asserted head where a produced group's stale
row is replaced while an unproduced asserted row survives even a full
derive. Nine Rust derive tests, the mcp and serve_ops suites, the
cross-host differential, all green against the release. The claude
smoke: the twelve aggregate rules that phases one and two skipped now
evaluate and land {"rounds":1,"changed":[]}, reproducing the
Python-derived store exactly, in 39 s. Phases remaining: keyed
upserts, the DRed sweep, strata to the joint fixpoint, the fleet
differential, then the write-path flip.

## 2026-07-05: THE JOINT-FIXPOINT STRUCTURE (correcting the phase plan)

Reading engine.py lines 1108-1288 to brief phases four and five
surfaced a structural correction to the port plan. The plan listed
agg, keyed, and sweep as sequential phases, which reads as three
stages run in order. Python runs them as THREE PASSES INSIDE ONE
OUTER LOOP bounded at twelve, because each can invalidate the
others through the dependency graph (the ledger's own words in the
source: loads settle, ranks rederive over them, the peak refolds
over ranks). Whole-cell rederivation in the sweep both propagates
this call's supersessions and converges staleness inherited from
frozen caches and replay history, which is what makes derive
idempotent (GMS93 overestimate-then-rederive at cell granularity,
sound because no swept row is asserted).

So phase three's standalone agg 0..12 loop is refactored into the
joint loop, and phases four (keyed upsert per key: produced keys
replace, asserted unproduced keys survive) and five (sweep:
fully-derived acyclic heads whole-replace; self-supporting heads get
the paper's recursive empty-first-then-local-fixpoint form so a
stale cycle cannot rederive itself) become passes two and three of
that same loop. The self-supporting split is the DRed subtlety: an
acyclic materialized head is safe to overestimate and rederive
whole, but a closure reachable from itself through derived-head
reads must delete its overestimate first. Delegated as one arc with
a green checkpoint between keyed and sweep, so an interruption
leaves a committable tree, the lesson from phase three's rescue
applied forward.

## 2026-07-05: THE MERGE PLAN (Samuel; the endgame inverts)

Samuel: never push pyarest as a repo; import it into arest for
version 0.9.0. This inverts the nuke framing the session opened
with. arest is not deleted; it ABSORBS pyarest's polyglot engine as
new internals and KEEPS its outer shell (the Cloudflare Worker, WASM
build, ui.do React target, Solidity contracts, kernel OS, REST and
HATEOAS and OpenAPI and SSE surfaces, docs, paper, npm and GitHub
identity). Filed as Operating Rule merge-pyarest-into-arest-not-nuke;
the swap-evidence rule updated to merge-ready.

What this changes: the PUNCHLIST stops being a deletion gate and
becomes the interop map, its header reclassified into REPLACED-by-
import (the engine internals pyarest supplies cleaner) versus
RETAINED-shell (persists in arest, needs only interop). Most entries
that read NOT PORTED now mean RETAINED IN AREST, no longer blockers.
The dependency tree's leaves (Cloudflare, WASM, Solidity, ui.do)
stop being pyarest porting arcs and become arest-side interop once
the engine imports. The diagram's endgame section is redrawn: finish
the derivation port, import into arest and tag 0.9.0, interop the
shell.

What this does NOT change: the engine work is identical. The
derivation port (phases four through six), the fleet differential,
and the write-path flip are still the path, because the imported
engine must derive natively and correctly whether arest wraps it or
not. pyarest's local git history stays as the development record;
the pushable repo is arest. The resident is still the daily driver,
now as arest 0.9.0.

## 2026-07-05: THE IFACTR STUDY RETURNS; THE OLD ENGINE CANNOT SERVE CLAUDE

The iFactr architecture study delivered a deeply-cited report and,
notably, found the analogy ALREADY written into AREST: docs/
2026-06-30-pyarest-design.md:242 describes AREST's platform binding
as iFactr's cross-platform UI model "inherent in metacomposition and
up-name:DEFS rather than a hand-built abstraction layer and
resolver," and crates/arest/src/viewproj.rs cites iFactr by name for
the component-role-to-widget binding (the select_component seam).
The load-bearing findings: iFactr's MXContainer (NamedTypeMap keyed
on interface-plus-name) is DEFS with a weaker contract; iFactr
registers only at interface seams while pyarest's override is
universal (any def, held equal by the differential, meaning stays
canonical, native is only speed); iFactr's IPairable two-object
pairing is what metacomposition makes unnecessary (one object that
is both description and mechanism). ui.do IS the AREST-side iFactr:
ViewProjection with componentRole, ViewForm.tsx switching role to
React widgets (the shipped target), Slint and WPF the planned ones,
the reading compiling to the REST/HATEOAS surface the target
renders. The full report's six lessons are the ui.do-port and
DEFS-override design language.

FILING NOTE, itself a data point for the perf/correctness gate: the
Engineering Principle rows for these lessons could NOT be filed via
mcp__arest__apply, because the claude app's live MCP engine is the
OLD arest engine, and it TIMED OUT at 240s on the write to claude's
heavy store, having first REJECTED three of the app's own aggregate
rules ("Session has Decision Count ... consequent fact type did not
resolve - REJECTING the rule"). The new engine's aggregate stratum
(phase three, just verified) derives exactly those consequents. So
the old engine cannot currently serve its own operational ledger
correctly or in time, which is the merge's whole motivation stated
by the incumbent itself. The lessons are recorded here until the new
engine serves claude.

## 2026-07-05: THE HEAD-TO-HEAD, MEASURED (old Rust engine vs new)

The old arest-cli release built (5m 03s, 151,750 lines) and the
benchmark ran on real stores. Numbers, wall time, release binaries:

message-vetting (small):
- OLD compile+derive from readings: 5135 ms (with a projection
  warning: 16 rows failed a FK constraint, an old-engine wrinkle).
- OLD derive-only (forward_chain over the persisted .db, idempotent):
  821-1354 ms total; the reduction ITSELF was 56 us. The ~800 ms is
  load_state plus cell_index; the derive does nothing because the
  store is materialized.
- NEW derive (arestlam run_rules over the sidecar, idempotent, incl
  spawn and ingest): 200-260 ms.

claude (21 MB, 384 nouns, 748 fts):
- OLD derive-only (forward_chain, idempotent): 2779-3050 ms total;
  reduction 30.7 us. Again load-dominated; derive is a materialized
  no-op.
- NEW derive (run_rules, idempotent, incl boot): ~39 s.

THE HONEST DECOMPOSITION (this is the real finding, not a headline):
- LOAD: new sidecar ingest ~0.2 s BEATS old deserialize-plus-index
  ~3 s, roughly 15x. The new engine already wins load.
- DERIVE, idempotent: old materialized no-op (~30 us) BEATS new full
  canonical Scott sweep (~39 s). The old engine wins derive today.
The gap is exactly and only the canonical reduction cost of the full
sweep. The old engine is materialized and incremental at fine grain,
so an unchanged re-derive is free; the new engine re-evaluates every
rule each bare derive.

WHY THE GATE IS ACHIEVABLE, measured not reasoned:
- The native carrier is the closer: the earlier machine-step bench
  put rust-canonical at 3.74 s against rust-native at 0.092 s, a 40x.
  Applied to the 39 s claude sweep that is roughly 1 s, and the new
  engine's total (0.2 s ingest plus ~1 s derive) then BEATS the old
  engine's ~3 s load-dominated total.
- The better algorithm already wins the incremental case, the actual
  write hot path: run_rules with a frontier cut claude rederivation
  39.9 s to 2.8 s (14x), and apply already passes changed=[ft], so
  the daily-driver write path is the frontier path, not the full
  sweep.

PERF GATE VERDICT: NOT YET met on the bare full derive at scale (new
is ~10x slower there today), but the new engine already wins load and
the incremental path, and the native carrier is the measured,
sufficient lever to win the full derive too. The concrete perf work
is now quantified: wire the native carrier into the derivation
reduction, target sub-3 s claude full derive. CORRECTNESS GATE: the
benchmark surfaced two more old-engine defects live (the FK
projection failures on message-vetting, and the three claude
aggregate rules the old engine REJECTS with consequent-did-not-
resolve where the new stratum derives them), on top of the
documented rooted, B2, and defined-in defects.

## 2026-07-05: CORRECTION, the native-carrier lever is UNVERIFIED for derive

Checking my own perf claim against the code, per measure-dont-reason,
it does not hold as stated. What I told Samuel ("the native carrier
is the measured 40x lever that turns the 39 s claude derive into
~1 s") conflated two things:

1. The 40x (rust-canonical 3.74 s vs rust-native 0.092 s) was
   measured on the WHITEPAPER MACHINE STEP run ten times
   (test_polyglot.py test_benchmark_the_flex: build_system plus
   machine_step, a create/transition), NOT on the derivation
   fixpoint.
2. op_run_rules evaluates every rule body through reduce_in over
   srv.mu, the canonical Scott evaluator. It does NOT and CANNOT
   currently use the native carrier NEval, which is selected only by
   engine=native in the machine-step run_facts path. The override
   twins that ARE on by default barely moved the machine step (3.83
   vs 3.74 s); only NEval gave the 40x.

So the derive-native speedup is UNMEASURED, and the ~1 s projection
was an extrapolation. The PRIOR is strong (rule bodies are the same
Scott-encoded FFP terms the machine step reduces, so a native
evaluator should help comparably), but strong-prior is not measured,
and I filed the rule that forbids the substitution.

THE CORRECTED PERF TASK, concrete and honest: wire NEval into
run_rules' rule-body evaluation (convert the rule object and D to N,
evaluate via NEval, convert rows back; the differential already
certifies NEval equals Scott equals Python), then MEASURE the claude
derive natively. Only that number decides the perf gate. The task
queues after the phase 4/5 agent (same file, main.rs). Until it is
measured, the standing truth is: the new engine wins load (0.2 vs
3 s) and the incremental frontier path (2.8 vs 39.9 s), loses the
bare full derive at scale (39 vs materialized-free), and the lever to
win the last one is grounded but unproven.

## 2026-07-05: THE IFACTR MAPPING, SYNTHESIZED (artifact)

The completed iFactr investigation is distilled into a designed
reference: the MonoCross:iFactr :: AREST:ui.do thesis, the
mechanism-by-mechanism correspondence (MXContainer to DEFS,
IPairable-forwarding to componentRole, controller-plus-view to a
FORML reading, netstandard-core to the intersection source,
named-variants to the twin-plus-differential), the honest contrast
(iFactr richer in UI maturity, AREST richer in what registration
MEANS), and the six lessons for the merge. Artifact:
https://claude.ai/code/artifact/0150b046-8dc9-4d70-b1f6-6fe269f3432c

THE ACTIONABLE CONCLUSION for the 0.9.0 merge: the DEFS-override
architecture the new Rust engine is built on is NOT novel risk. It
is iFactr's proven registration model (a mature framework shipped
across five .NET runtimes), generalized from interface seams to any
def and given a correctness proof (the differential twin) the
pairing never had. ui.do is the arest-side iFactr, a retained shell;
its React target ships and Slint/WPF are the per-platform bindings
over select_component, rendering the REST/HATEOAS surface the
reading compiles to. So the verb surface sits directly between the
ui.do abstraction and the engine, the same one-function SYSTEM seam
the interop keystone found. The six Engineering Principle rows file
into the claude principle index once the new engine serves claude
(the old engine's MCP write path times out today).

## 2026-07-05: THE METACOMPOSITION TWO-WAY-BINDING CRUX (from the iFactr report)

The report's deepest finding, preserved here as the artifact footer
promises, because it is why the AREST binding beats iFactr's pairing
and thus why the merge's engine choice is sound. The literal token
"metacompose" is absent from all three repos; the concept is present
under Backus's own term METACOMPOSITION, the one reduction rule of
the FFP algebra: (rho SEQ):y = (rho x1):pair(SEQ, y), with
apply:pair(x,y) = x:y making an operator an object. The design note
(design.md) ties it straight to binding: "value binding is
metacomposition"; "binding is application ... names resolve through
up-name:DEFS, so behavior is injected, an inversion-of-control
container inherent in the algebra"; and the identity underneath,
"set-membership is function-application, x in P, P x, and the
rho-reflection of a fact into the function it denotes are one act,"
so "process = fact = function, one object at three altitudes."

Two-way binding falls out at two coupled levels, each stronger than
iFactr's object-to-object mirror:

1. DESCRIPTION and MECHANISM are one object. Because metacomposition
   lets a definition be both an inspectable datum and an applicable
   function, the schema you read and the behavior you run are not two
   artifacts kept in sync; they are one term. "The reading IS the
   application in all four roles at once," compile is "recognition
   rather than translation" (README.md:5). Editing the reading moves
   map and territory together, no generate-then-drift gap. This is
   two-way binding of map to territory: neither changes without the
   other, because they are the same object.

2. VIEW and STATE are inverse projections of that object. Forward,
   state to view, is HATEOAS as projection: links(e) = nav(e) union
   transitions(status(e)), recomputed from the fact web on every read
   (deriveLinks, hateoas.ts). Backward, view to state, uses the very
   control the projection emitted: the _links.place affordance is a
   POST .../transition that folds the event back through the same
   metacomposed machine. Because the forward control and the backward
   transition are both generated from ONE object, they cannot fall
   out of sync; that inseparability IS the two-way binding. ui.do
   closes the loop client-side (useEntityLinks reads links, Overworld
   Menu POSTs transitions).

The contrast with iFactr, exact: iFactr binds two DISTINCT objects,
an abstract control and its native twin, each holding a pointer to
the other (IPairable, the WPF Pair setter wiring pair.Pair = this),
forwarding every property across the seam, a resolver choosing the
native type. That is object-to-object mirroring over a hand-built
abstraction layer. Metacomposition needs neither twin nor
forwarding, because one object is both data and function, projected
both ways by the algebra: "the algebra is the framework" (design.md:
242). And the closest structural twin to iFactr's Pair on the AREST
side is the canonical def and its native override, held equal in
both directions by the differential, except that binds MEANING to
SPEED and carries a proof obligation iFactr's pairing never had.

Consequence for the merge and the perf gate together: the new
engine's native twins are the SAME two-way binding as iFactr's Pair,
minus the second object, plus a proof. So wiring NEval into run_rules
(the perf task) is not a hack bolted onto the engine; it is one more
instance of the architecture's own binding discipline, meaning to
speed, certified by the differential. The perf work and the
architecture are the same shape.

## 2026-07-05: UI.DO INTEROP READINESS (the iFactr chain closes)

The iFactr study named ui.do as the arest-side iFactr rendering the
engine's REST/HATEOAS verbs; the readiness check confirms the new
engine already serves every one of them. ui.do's arestDataProvider
(apps/ui.do/src/providers/arestDataProvider.ts) is a thin REST
client over /arest/{resource}: getList (GET slug), getOne (GET
slug/id), getManyReference (GET target/id/resource, nested), create
(POST), update (PATCH), delete (DELETE), plus HATEOAS transitions
(useActions POSTs .../transition). Every one maps to a verb the new
engine already exposes: query, get, query-filtered-or-sql, apply
create, apply update, apply transition-to-terminal, apply transition
/ actions.

So the ui.do-to-engine READINESS is GREEN at the verb level: no new
engine verb is needed for the shipped React target. The ONLY interop
work is the binding layer, which is the same keystone the Worker
survey found: the arest Worker calls the engine through the single
WASM export system(handle, key, input), and the merge must point
that export at the NEW engine, over a handle-to-D registry routing
key to the canonical SYSTEM dispatch the resident already runs. The
resident's MCP/serve dispatch and the Worker's system() export are
two thin wrappings of one canonical SYSTEM.

THE FULL CHAIN, iFactr to merge, now closed: iFactr's registration
model IS DEFS (proven prior art, five .NET runtimes); ui.do IS the
arest-side iFactr renderer (retained shell); the engine surface IS
one SYSTEM function; ui.do's REST needs ARE covered by the new
engine's verbs; the remaining work IS the WASM system export plus
the per-target renderers (React ships, Slint and WPF are the iFactr-
style bindings over select_component). The merge's UI-and-shell half
is a binding exercise, not a porting one.

## 2026-07-05: THE CORRECTNESS GATE, A MINIMAL PAIRED PROOF

The correctness gate's final form (a direct old-vs-new run on shared
input showing the new matches the spec where the old diverges), in
seven reproducible lines. The model (scratchpad/correctness/readings/
agg.md):

  Team is an entity type.
  Player is an entity type.
  Roster Size is a value type.
  Player plays for Team.
  Team has Roster Size iff Roster Size is the count of Player where
    Player plays for that Team.
  Player 'p1' plays for Team 't1'.
  Player 'p2' plays for Team 't1'.

The spec-correct answer is Team t1 has Roster Size 2 (p1 and p2 both
play for t1). Same model, both release engines:

- NEW (pyarest): Team_has_Roster_Size = [('t1', 2)]. Correct.
- OLD (arest-cli release): REJECTS the rule outright, verbatim,
  "consequent fact type did not resolve (empty id) - REJECTING the
  rule and emitting nothing." No roster size derived at all.

This is a standard ORM count-per-group aggregate, the most ordinary
kind of derived fact, and the old engine cannot resolve the iff
rule's consequent, so it drops it. It is the SAME defect seen live on
the claude app (the Decision Count aggregates rejected identically),
now isolated to a minimal case anyone can rerun. The new engine's
side is permanently regression-guarded by tests/test_aggregates.py
and tests/test_parity_agg_count.py.

Correctness-gate standing, evidence now direct not just documented:
the new engine derives a standard aggregate the old engine cannot,
on top of the rooted over-derivation, the B2 divergence, the
defined-in mis-join, and the live FK projection failures. The gate
favors the new engine decisively; this minimal case is the clean
exhibit. (A fair-comparison note for the record: my first attempt
used a malformed model missing the iff, and BOTH engines correctly
did nothing with it. The comparison only means something once the
input is a valid reading, so the corrected iff model is the one that
counts.)

## 2026-07-05: THE PERF PICTURE COMPLETES (hot path fast; only cold derive slow)

Phase 4/5 committed (9f83e61: keyed upsert + DRed sweep, joint
fixpoint, 12 derive tests + differential green). The release rebuilt
and the frontier measurement completes the perf picture, correcting
the alarming 143s headline:

claude scale, the three derive costs:
- BARE FULL DERIVE (changed=None, cold): ~143 s. All ~60 rule bodies
  re-evaluated through the Scott substrate. This is what phase 4/5's
  smoke measured.
- FRONTIER DERIVE (changed=[cell], the WRITE hot path): 122-677 ms.
  Only rules reading the changed cell fire. Sub-second.
- LOAD (apps_use, serving pre-derived cells): ~0.2-0.3 s.

The daily driver's actual costs are LOAD plus FRONTIER, both
sub-second, because the resident serves pre-derived sidecars and
re-derives only incrementally on writes. The 143s bare full derive
is a COLD correctness sweep the daily driver essentially never runs
per operation.

PERF GATE, honest and complete:
- Hot paths (load, incremental write-derive): MET. The new engine is
  competitive or winning against the old engine's ~3s load-dominated
  operations.
- Cold full derive: NOT met (143s vs old's materialized ~3s). The
  native carrier is the fix and remains worth doing for the gate's
  "beat old on EVERY axis" completeness and for cold-start/rebuild
  scenarios, but it is gate-completeness work, NOT a daily-driver
  emergency. The reframing matters: I will not overstate urgency.

The native-carrier task, precise (from the phase 4/5 agent's
diagnosis plus the code): op_run_rules evaluates every rule body
through reduce_in over the Scott srv.mu; route it through NEval (the
native carrier, measured 40x on the machine step, differential-
certified equal to Scott). Needs V-to-N and N-to-V converters (only
j_to_n and n_cells_of exist today) and native-mirror maintenance as
run_rules stores rows. The differential is the correctness gate; the
143s-to-target is the perf gate.

## 2026-07-05: CORRECTNESS BREADTH CONFIRMED (the win is broad, not narrow)

Checking the correctness gate's breadth while the native-carrier
agent runs: the new engine handles the full standard-ORM derivation
vocabulary, each tested.
- Aggregates: the compiler's _AGG_CLAUSE recognizes count, sum, min,
  max (compiler.py:1240); all four are covered by test_aggregates.py
  and kin.
- Transitive closure: the recursive reaches/at-cost rule
  (test_nonlinear_joins.py) derives both the direct and the summed
  path, verified fresh: reaches(a,c) carries both 2 (a-b-c) and 3
  (direct).
- Keyed upsert, sweep, sweep_cyclic, deontic: all covered by the
  derive unit tests and the deontic suite.

So the aggregate-rejection defect the old engine shows is one
instance of a BROAD correctness edge: the new engine derives the
whole standard vocabulary the old engine variously drops or
mishandles. My earlier empty max-aggregate and ring-closure probes
were malformed syntax on my part (maximum-of vs max-of, and the
named-variable closure form), not engine gaps, confirmed once the
correct forms derived.

Note for the differential's completeness (defense in depth, not
critical): test_derive_differential.py currently exercises only the
CLOSURE pass; the native carrier touches agg, keyed, and sweep too.
Those passes ARE verified by the 12 derive UNIT tests (hand-built
stores through the same op_run_rules), so the perf agent's work is
covered. Extending the cross-host differential to a model with
keyed and a star-derived cyclic head is a worthwhile follow-up; the
exact FORML for a clean fully-derived cyclic head is fiddly (the
star-plus-named-variable form did not register as ruleDerives in a
quick probe) and is deferred, not blocking.

## 2026-07-05: THE EVENT STREAM BECOMES A SWAPPABLE INTERFACE (Samuel)

Samuel: he never designed anything to use a jsonl, and does not like
it for events. The event stream should be written to an INTERFACE
whose implementation swaps via DEFS, like everything else, the
storage layer, the ui, the performant rule engines. He is right: the
.events.jsonl file was an undesigned implementation choice that
violated the defs-override-glue-framework discipline the whole engine
rests on.

Landed TDD-first (tests/test_event_sink.py): persist gains an
EventSink interface, exactly the Connector's two names, append(entry)
commits a step and read() yields the entry list for reconstruction.
FileEventSink is the default (the jsonl behavior preserved, format is
not the design); MemoryEventSink is an ephemeral per-app sink (tests,
transient apps, the model for a broadcast sink). register_event_sink
binds an implementation by name and Registry.event_sink selects it,
so swapping the whole stream is one registration. replay refactors to
replay_entries(D, entries), sink-agnostic, and replay(D, path) becomes
a thin file-reading wrapper.

Every Registry event site routes through the sink: compile reads
self._sink(name).read() and replays the entries, apply and retract
append to it, and migrate.replay_into writes migrate batches through
it. The three tests pin the swap (a memory sink captures the stream
with NO file written and a recompile reconstructs from it); 32
neighboring tests confirm the default file sink preserves behavior
exactly.

MERGE RELEVANCE: this is not cosmetic. arest's shell writes events to
a BroadcastDO (the SSE stream), not a file, so the event sink MUST be
a swappable interface for the merge to work in the Cloudflare tier.
The BroadcastDO is exactly the alternative EventSink this enables.
And the pattern generalizes: storage is currently hardcoded to sqlite
(persist.save_sqlite in compile) the same way events were hardcoded
to jsonl. The EventSink is the first instance of the swappable-
boundary discipline; a StorageSink interface is the natural next one,
and an app declaring its sink in readings (the full Connector form,
App uses Event Sink 'broadcast') is the DEFS-declaration extension
beyond today's registration-based swap.

## 2026-07-05: THE SWAPPABLE-BOUNDARY AUDIT (Samuel's correction, systematized)

Samuel's jsonl correction is a CLASS, not a one-off: "everything
swappable via DEFS." The complete audit of hardcoded implementation
choices in the engine, so the correction is systematic:

SWAPPABLE (honor the discipline, the model):
- The RULE ENGINE: Scott / NEval / native-container overrides,
  selected by engine=native and register_overrides. The exemplar
  Samuel cited, and the native-carrier perf work extends it.
- The EVENT STREAM: EventSink (append/read), swappable as of today.

HARDCODED (violate the discipline, the same class as the jsonl):
- CELL STORAGE: sqlite (save_sqlite / load_sqlite, 13 sites).
- THE SIDECAR (the resident's boot food): a JSON file (_sidecar,
  <name>.store.json).
- THE RMAP RELATIONAL PROJECTION: sqlite DDL (ddl.project).
- THE FROZEN INGEST CACHE: a sqlite snapshot (ingest_frozen).
- CELL ENCRYPTION: ChaCha20-Poly1305 (keyed via seal_key, but the
  cipher itself is fixed).

THE CLARIFYING FINDING: the four persistence boundaries (storage,
sidecar, projection, frozen cache) are not four separate follow-ups.
They cluster into ONE concern, the persistence layer, all hardcoded
to sqlite-or-file. A single StorageBackend abstraction (mirroring
arest's own Arc<dyn StorageBackend>, so it is merge-aligned) would
make all four swappable at once: save/load the cell store, plus the
sidecar and projection as backend-specific outputs a sql backend
produces and a memory or DO backend skips. Encryption is a separate,
smaller boundary (a Sealer/Cipher interface).

So the storage follow-up I flagged is really a persistence-layer
abstraction, and it is the one remaining large instance of the
discipline. It awaits Samuel's steer (I asked); the EventSink is its
proven template. The rule engine and the event stream already honor
it; the persistence layer and encryption are the gap.

## 2026-07-05: THE PERFORMANCE GATE MEETS (native carrier, ~90x, verified)

The native-carrier agent landed and I verified it first-hand. The
claude full derive drops from ~143s (Scott substrate) to 1.34-1.40s
(NEval), about 90x, MY measurement, three runs, idempotent. So the
new engine now BEATS the old engine on EVERY axis Samuel's gate
names:
- Load: new 0.2s vs old 3s.
- Incremental frontier write-derive: new 0.12s.
- Cold full derive: new 1.4s vs old materialized 3s. The last gap
  closes; the new engine wins the axis the old engine used to own.
And it clears Python's 6.2s (the intermediate bar) with room.

The mechanism: op_run_rules routes all five rule-body reduction sites
through NEval; v_to_n/n_to_v convert at the boundary; store_into keeps
the native mirror in lockstep so intra-round stores are visible; and
NCANON, the native twin of Scott's CANON step, resolves the
canon-reachability wall deliberately (consulted after process,
mirroring Scott's PROCESS-then-CANON order), the one design judgment
beyond the prescribed shape. This IS the defs-override thesis
realized: the canonical Scott layer is the meaning and the oracle,
the native carrier is the speed, and the differential certifies they
agree.

Correctness, verified: 12 derive tests, mcp, serve_ops, and the
cross-host differential green, the differential now EXTENDED to cover
the aggregate pass (Team/Player count, Python == Rust row-for-row).
Both gates Samuel set are now MET and measured: more correct (the old
engine drops a standard aggregate the new derives) and more
performant (every axis). The remaining derivation-port item is the
fleet differential over every rehearsal scratch, then the write-path
flip; the port's algorithm and speed are done.

## 2026-07-05: STORAGE BECOMES A SWAPPABLE 3NF DRIVER (Samuel's steer)

Samuel: storage should be swappable between sqlite, R2, postgresql,
clickhouse, anything usable as a 3NF storage driver. Designed against
the PRIMARY SOURCE, arest's own StorageBackend (crates/arest/src/
storage.rs): CellStorageBackend is the core (cells as source of truth,
cell_read/commit/list/delete), StorageBackend is the whole-object shim
over it (open/commit), and DurableObjectBackend stores cells as
freeze-bytes over HTTP for the R2/Cloudflare tier. So storage is
cell-based and the 3NF is the relational CONSUMER projection, not the
core, which resolved the design cleanly.

Landed TDD-first (tests/test_storage_driver.py): persist gains a
StorageDriver interface mirroring arest's whole-store form, save
commits the cell store (arest's commit), load rehydrates it (arest's
open), exists reports. SQL backends additionally serve the 3NF
projection (drv.project) and query (drv.query); object backends (R2,
memory) store cells and have no relational surface (drv.sql False, the
sql verb raises a clear error not a crash). SqliteStorage is the
default (cells one-to-one plus the RMAP 3NF tables, current behavior
exactly); MemoryStorage is the ephemeral object-store model.
register_storage_driver binds a driver by name and Registry.storage
selects it, so postgres, clickhouse, or R2 is one registration.

Every Registry storage site routes through the driver: compile saves
and (if sql) projects, apply saves, _load loads, sql queries, and the
inventory's compiled-check uses drv.exists() not a .db file. Four
storage tests pin the swap (a memory driver with no .db written, the
store round-tripping in memory, the SQL surface guarded); 32-plus
neighboring tests confirm the sqlite default preserves behavior.

THE SWAPPABLE-BOUNDARY AUDIT UPDATES: storage moves from hardcoded to
SWAPPABLE, and the 3NF projection folds into the SQL driver's
capability (no longer separately hardcoded). Now honoring the
discipline: the rule engine, the event stream, and storage. Remaining
hardcoded: the sidecar (a JSON file, the resident's boot food, but
that IS the serve-protocol format the resident needs), the frozen
ingest cache (a sqlite snapshot), and cell encryption (ChaCha20-
Poly1305, keyed). The two large boundaries Samuel named, events and
storage, are both done.

## 2026-07-05: THE FLEET DIFFERENTIAL IS CLEAN (the port certifies)

The derivation port's named acceptance step, done: the native
carrier Rust engine derives every living app IDENTICALLY to Python,
on a FRESH derivation from the emptied base (not idempotence, real
re-derivation of the whole rule set). Per app, heads / mismatches:

  arc-stack                  1    0
  bill-negotiation-service   3    0
  claude                    35    0
  kernel                    68    0
  message-vetting            1    0
  support.auto.dev          13    0
  FLEET: 6 apps, 121 heads, 0 mismatches.

Every derived head across the fleet, closure, semi-naive, aggregate,
keyed, and DRed sweep including the self-supporting cyclic head, Rust
== Python row-for-row. The port is now COMPLETE on all three axes:
algorithm (the joint fixpoint), speed (native carrier, 1.4s
idempotent and 3.4s fresh claude derive vs Python's 15.8s), and
fleet correctness (121 heads, zero divergence).

Timing note from the fresh-derive runs: Rust beats Python on the full
derive too (claude fresh: 3.4s vs 15.8s, ~4.6x; the idempotent path
was 1.4s vs Python 6.2s). The native carrier wins whether the derive
is cold-fresh or idempotent.

The ONE remaining derivation-port item is the write-path flip: the
resident deriving natively on writes (apply) instead of delegating
the create pipeline to Python. The derive op is already native and
fleet-certified; the flip wires it into the write path, a larger
architectural step (writes need the create-validate-emit pipeline,
not just run_rules). The port's algorithm, speed, and read/derive
correctness are done.

## 2026-07-05: THE VISION CLARIFIES; THE WRITE-PATH FLIP BEGINS

Samuel: the goal this whole time has been FULL NATIVE for every part
but lambda and defs on all targets, and even some of those could be
shared if done carefully. This is the architecture's endgame stated
plainly: the ONLY host-specific cores are the mu evaluator (lambda)
and the DEFS registry; every other part, create, derive, persist,
query, verbalize, is a DEF reduced natively on every target. The
native carrier (NEval) is exactly this, which is why the flip's path
is precompute-the-handler-def and reduce-it-natively, not a
Rust-specific port: a native create is lambda-plus-defs, the two
exempt cores.

Sequencing (Samuel): flip first in pyarest (where the differential
harness lives), verify fleet-wide, then merge. Persist: FULL NATIVE
(the resident writes events and the sidecar itself in Rust; the
sqlite .db stays the compile-time projection arest's storage backend
owns post-merge).

FOUNDATION LANDED (verified, 18 apply/machine tests green): create()
decomposes into create_spec(D, ft), the schema-determined recipe
stable across writes (routing, absorbed column/width/unary, the
value validate, and the machine/mealy/links objects, each a
canonical lambda tree or None), plus _create_from_spec that builds
the handler and reduces it. The only fact-dependent piece is the
absorbed cell name table:key, the key read from the fact at write
time. create_spec is what rides as a create:<ft> cell so any host
builds and reduces the handler natively.

THE FLIP'S REMAINING PHASES (a major multi-part arc, canonicalizing
create for full-native):
1. The dynamic-store canon: a build_system variant whose commit
   addresses cellkey(table, fact-key), so the absorbed handler is
   fact-INDEPENDENT and fully precomputable (own-table already is).
   This makes every create handler a stored def.
2. Compile stores create:<ft> handler cells (all fact types).
3. Resident native apply: fetch create:<ft>, reduce over the fact
   natively (NEval) to the receipt and D', then native incremental
   derive (done), differential-verified against Python create.
4. Native persist: a Rust event sink (file/DO) and sidecar writer,
   so the resident's write path touches no Python.
5. Fleet-verify create+derive+persist against Python, then the flip
   replaces the delegation.

## 2026-07-05: MERGE STRUCTURE ASSESSED (prep for after the flip)

Read-only survey of arest's tree for the source-merge. arest's
engine crate (crates/arest/Cargo.toml) is already version 0.9.0,
crate-type cdylib plus rlib (the WASM Worker and the lib), AGPL, and
its description names the OLD pipeline: FORML2 parser, constraint
compiler, evaluator, forward chainer, RMAP, verbalization. Three
crates: arest (the engine), arest-foundation, arest-kernel (the OS).
pyarest's arestlam is the zero-dep new engine (lambda evaluator plus
NEval, the shared canon via include!, native run_rules, the resident).

WHAT THE MERGE REPLACES, cleanly: arest's EVALUATOR and FORWARD
CHAINER (the old reducer and derivation) become pyarest's lambda
evaluator plus NEval and the fleet-certified native run_rules. The
RMAP and verbalization are canonical in pyarest (layout_cells,
system:verbalize), so they move to the canon too.

THE MERGE'S CENTRAL QUESTION, flagged for Samuel: THE COMPILER. arest
has a Rust FORML2 parser/compiler (readings to M), which the WASM
Worker needs (no Python in the Cloudflare tier). pyarest's compiler
is Python (the grammar selfhost: the grammar file classifies, the
translators dispatch, but Stage-1 tokenization and the orchestration
are Python). So the merge must decide the compiler: (a) keep arest's
Rust compiler and merge only the evaluator/chainer, (b) port
pyarest's grammar selfhost to run natively on the Rust host (the
grammar file is already shared data; the translators are the port),
or (c) the compiler stays a Python build tool for the local/CLI tier
and the WASM tier uses arest's Rust compiler during a transition.
The full-native-on-all-targets goal points at (b) eventually, but
(a) or (c) unblocks the merge sooner. This is a Samuel decision;
noting it so the merge does not stall on it. The flip (native write
path) is the current work and does not depend on this.

## 2026-07-05: THE COMPILER QUESTION RESOLVED (Samuel: canonicalize it too)

Samuel: the compiler needs to be like everything else, built on
Backus AST/FFP/lambda, swappable via DEFS. This resolves the merge's
central question in the FULL-NATIVE direction (my option b): the
compiler is not kept-Rust nor Python-forever; readings-to-M becomes a
lambda reduction that runs on every target, the compiler joining
create, derive, verbalize, and persist in the canon. It is the last
big Python-orchestrated part, and the vision (full native for every
part but lambda and defs) demands it be canonical.

HONEST SCOPE, from the compiler's current canonical-vs-Python split:
- ALREADY CANONICAL: CLASSIFICATION. classify_all_via_M runs the
  grammar file (shared/forml2-grammar.md) through the canonical
  class_rule/class_subj, so which kind each statement gets is a
  lambda reduction, and the Classification-has-Translator dispatch is
  the grammar's own readings. The parser IS the file, already.
- STILL PYTHON, the canonicalization arc: (1) STAGE-1 field
  extraction, 60 regex productions turning a statement into typed
  fields; canonicalizing this is a lambda matcher over the statement
  text, the hardest piece (recognition as reduction). (2) THE
  TRANSLATORS, 60-plus _h_ handlers emitting M-facts from the fields;
  each becomes a def (many are simple stores, some intricate, e.g.
  the deontic and state-machine handlers). (3) THE ORCHESTRATION,
  statements() tokenization, the known-names prepass, and the compile
  loop, becoming a canonical fold.

This is the LARGEST remaining canonicalization, the compiler being
the most complex part. It is a major multi-arc effort AFTER the flip
(the current work). The grammar selfhost is its foundation, and the
Stage-1-as-lambda-matcher is the keystone: once statements parse via
a canonical matcher, the translators-as-defs and the
orchestration-as-fold follow the pattern verbalize and create
already set. Filed as the merge's compiler answer and the post-flip
endgame; the claude Operating Rule waits on the engine swap (the old
engine's MCP write path times out).

## 2026-07-05: APPLY-TO-ALL IS THE POINT (Samuel; Backus 1978)

Samuel: the real flex in FFP is apply-to-all. For loops are von
Neumann bottleneck code; an apply-to-all can fall back to a for loop
where an environment needs it, but making parallelism INHERENT in the
algebra is half the point, and semantic correctness via ORM is the
other. This is Backus 1978 (Can Programming Be Liberated from the von
Neumann Style): the bottleneck is word-at-a-time thinking, and alpha
(apply-to-all), (alpha f):<x1..xn> = <f:x1..f:xn>, carries NO
sequential dependency. A for-loop imposes an ordering the problem
lacks; alpha states independence structurally, so a host parallelizes
it (cores, WASM lanes, FPGA gates) and loops only as fallback. This
is the defs-override-glue-framework rule's other face: independent
applications carry no ordering constraints.

DISCIPLINE for all canonicalization: express iteration over a
collection as ALPHA (Backus alpha), never a sequential loop, so
parallelism is inherent and the fallback is the host's choice, not
the algebra's.

HONEST SELF-AUDIT against this:
- run_rules WITHIN a round: the rule-body evaluations are independent
  (each rule reads the store, emits rows), so a round is inherently
  ALPHA over the rules. The current Rust port evaluates them in a
  sequential for-loop, correct but von Neumann. The parallelism is
  latent; a parallel host should apply-to-all over the round's rules.
  The fixpoint ACROSS rounds is genuinely sequential (each round
  reads the last), so that stays a fold, correctly.
- create over a BATCH (the collection apply, Backus alpha over ops):
  the resident should apply create to all ops of a batch as ALPHA,
  one derive over the combined delta, which is exactly the old
  engine's atomic collection apply.
- create_handlers, the compiler's statement loop, the generator
  family: all iterate over fact types or statements with host
  for-loops today. Canonicalized, each is ALPHA over the collection.
- classify_all_via_M ALREADY does this right: it classifies ALL
  statements in one batch canonical operation, not one lfp per
  statement, which is why the grammar selfhost is fast. That is the
  pattern the rest should follow.

So the canonicalization endgame is not just "make it a def", it is
"make it an ALPHA-shaped def": create, the compiler's Stage-1 and
translators, run_rules' per-round pass, all expressed apply-to-all so
the FFP parallelism is real, with ORM correctness underneath. Filed
as standing discipline; the claude Operating Rule waits on the swap.

## 2026-07-05: THE POLYGLOT IS A DISCIPLINE ENFORCER (Samuel)

Samuel: one reason he forced the cross-engine polyglot is that by
restricting the grammar he restricts to lambda, removing the vector
for ifs and fors. This is the deeper rationale for the
intersection-source discipline (one tuple literal, the DEF/A/N/K/
S1..S9 vocabulary, no host constructs): shared/*.py must be valid
Python AND Rust AND C# AND Java at once, so the ONLY expressible
vocabulary is the FFP combining-form algebra. Host if and for are
not in the intersection; four compilers reject them. So the polyglot
MECHANICALLY forbids von Neumann control flow in the canon, the
constraint enforcing what discipline alone would not.

THE CONSEQUENCE for the endgame, exact: canonicalizing a part is not
only making it a def, it is moving it into a grammar where it CANNOT
be a for-loop, only alpha and the combining forms. So the host
for-loops that remain (the Rust run_rules per-round pass, the compile
loop, create_handlers) are von Neumann RESIDUE, the not-yet-
canonicalized parts, and canonicalizing each one eliminates its loop
by construction, because the shared grammar has no loop to port into.
The full-native endgame (everything but lambda and defs canonical)
and the apply-to-all discipline are the SAME move seen twice: push
logic into the polyglot canon, and the canon forces it alpha-shaped.

The canon DOES carry COND and WHILE, but those are functional
combining forms (compose functions), not word-at-a-time statements;
alpha is the parallelism primitive and WHILE is the fold reserved for
genuine sequential dependency (the fixpoint countdown), used only
where the ORM dependency graph actually demands order. Prefer alpha;
fall to WHILE only for real sequencing.

This is why the three recent directives are one directive: full
native (lambda plus defs), apply-to-all (Backus alpha), and the
polyglot (the grammar that forbids the alternative) are three faces
of the same architecture. The compiler and create canonicalizations
inherit all three at once.

## 2026-07-05: CANONICAL LAMBDA FIRST, HOST CODE ONLY AS OVERRIDE (Samuel)

Samuel: there should not be any Rust or Python specific code to any
core process like compile unless it already exists as a lambda and
you are adding a performant override. This is the strictest form of
the defs-override discipline and it names the ORDER: the canonical
lambda EXISTS FIRST; host-specific code is only ever a performant
override of it, certified equal by the differential. You cannot write
host orchestration for a core process and call it the engine; the
engine is the lambda, the host code is speed.

SELF-CORRECTION, honest: the create ORCHESTRATION violates this. The
canonical PIECES exist (ast:build_system, system:row_validate,
system:machine_step, system:row_resolve), but create_spec, the logic
that reads M-facts and assembles those pieces into a handler (the
partition triples, the routing decision, the machine detection), is
PYTHON. No system:create lambda exists behind it. So create_spec and
the precompute-handler shortcut are host-specific code for a core
process without the canonical lambda, exactly what the rule forbids.
This has been true of create() all session (Python orchestration over
canonical pieces); the rule makes it explicit.

THE REDIRECT: canonicalize the create ORCHESTRATION as lambda, so
system:create exists as a lambda over the M-facts (partition-triple
assembly, routing COND, object assembly, build_system, reduce), all
alpha-shaped per the parallelism discipline. THEN the Python create()
and any Rust native apply are performant OVERRIDES of system:create,
certified equal. The resident reduces system:create natively (NEval);
the precompute of handlers becomes a valid compile-time override once
the canonical exists, not before.

Same rule for the compiler: no Python Stage-1 or translator or
orchestration code stands as the engine; system:compile must exist as
a lambda, and Python is its override. The four directives now
compose into one order of operations: (1) canonicalize the core
process as an alpha-shaped lambda in the polyglot canon (which forbids
for/if by construction), (2) add host overrides for speed, certified
equal by the differential. Create and the compiler both follow it.
The flip's resident shell (reduce plus persist) is reusable; the
create LOGIC it reduces must become system:create first. This
reorders the flip: canonicalize create, then the overrides. Filed as
the governing discipline.


## 2026-07-05 (cont): the first canonical leaf, system:table_columns

The redirect is in motion. The create orchestration is canonicalized
bottom-up, one leaf at a time, each certified by the twin oracle before the
Python builder becomes a thin caller. The first leaf is done and committed
(f4e3515).

system:table_columns is now a canonical DEF in shared/system.py. It is the
fact types absorbed into a table, in declaration order, expressed as
apply-to-all over a double filter: alpha N1 composed with Filter for the
fact type unequal to the table composed with Filter for the key equal to the
table, the target table baked into each predicate by quasiquotation. The
parameter is reached through id and wrapped as a runtime CONST, the same
quasiquote idiom class_rule and ftpop_absorbed use. The von Neumann
comprehension is gone. The two filters and the projection are alpha-shaped,
with no sequential dependency between rows.

It was developed against the twin oracle, not hand-authored blind. The
Python table_columns was the behavioral specification, and the DEF applied to
the same partition pairs answers the same rows. Python is now a thin caller
of the DEF, matching the row_resolve precedent, so every host reads one
definition.

Verified across the intersection and committed:
- Python: the twin oracle passes; 547 tests pass in total (30 fast, 500 in
  the main suite, 17 in the rust differential); the thin caller works for
  every caller, which is layout_cells, create_spec, ft_view, and the sql
  projection. The compile path stays fast, the self-host compile at 4.6s.
- Rust: the release binary rebuilds clean with system:table_columns
  include!d. This is the polyglot-as-enforcer check. A DEF that compiles as
  valid Rust and valid Python at once cannot smuggle a for or an if, so it is
  genuine lambda. cargo test is green across 16 tests, including the
  native_apply differential.
- C# and Java: byte-wrapped by the csproj WrapCanon target, never parsed, so
  intersection-validity there is vocabulary support. Every atom used
  (theta:Filter, eq, not, CONST, COMP, CONS, ALPHA, apply, id) is
  pre-existing. Rust, the strictest parse-required host, already accepts it,
  so the byte-wrapped hosts are strictly less demanding.

The one slow test in the full suite (over 120s, faulthandler dumped a trace)
is test_scale, the deliberate WHILE-to-20000 fold stress through the Scott
reducer under a lowered recursion limit. It is pre-existing and unrelated to
this change; the compile-and-table_columns tests all finish in seconds.

Next up the arc, the same way each time (twin oracle first, DEF second, thin
caller third): system:governed_player (the player a machine governs, a
filter-and-find over the roles keyed by the smDef and governedBy closure),
then system:partition (the RMAP feature extraction that feeds the
already-canonical rmap fold), then system:create itself as the alpha-shaped
composition over the fact types. The resident's native-apply shell already
reduces whatever create:<ft> handler the canon produces, so it is
forward-compatible with each leaf as it lands.


## 2026-07-05 (cont): the second leaf, system:governed_player

The second create-orchestration leaf is done and verified, staged awaiting
signature (Samuel stepped away and the GPG pinentry timed out; the work is
ready to sign the moment he is back).

system:governed_player is a canonical DEF in shared/system.py, the player a
machine governs and so whose status cell a trigger advances. Unlike
table_columns, which took the partition as data, this is a D-reader over the
pair of the fact type and the store: it fetches smDef, governedBy, and role
through ast:FetchPop, forms the governed set as the union of the smDef nouns
(column 2) and the governedBy closure (column 1) by a dedup over the
concatenation of two apply-to-all projections, filters the role population to
the fact type, threads the governed set beside each role by distl, and keeps
the roles whose player is in the set by theta:member. The first survivor's
player is the answer, the empty sequence when none. The von Neumann
loop-and-return is gone.

Developing it taught the D-reader idiom (fetch a named pop from the store by
apply of ast:FetchPop, then apply to the store) and the runtime-set membership
idiom (distl to pair the computed set beside each row, then Filter by
theta:member), both of which system:create needs. The twin oracle covers the
smDef path, the governedBy path, and the no-match case. The Rust binary
rebuilds clean with the def include!d, cargo test is green across 16 tests,
and 55 targeted caller tests pass with the Python thin caller.

Toward system:create: build_system is the assembly target. Its Python wrapper
builds a nine-slot record (cell name, validate, resolve, derive, links,
machine, mealy, index cell, append cell) and applies the canonical
ast:build_system. So system:create computes those slots from the M-facts and
applies ast:build_system. For an own-table machine-triggered fact type the
record is the cell name plus the links, machine, and mealy objects, which are
governed_player (done) then machine_step, mealy_step, and transitions_of (all
already canonical), with a small role-position find and the status-cell name
as the remaining glue. That is the next prototype, batched into one
system:create commit rather than a rebuild per tiny leaf.


## 2026-07-05 (cont): system:partition lands (the biggest leaf, the storage model)

The whole RMAP procedure is now canonical lambda, verified across the
intersection, staged in a background commit awaiting signature (the passphrase
cache expired over the long session; the work is done and green).

Samuel's status/RMAP insight set the direction: status is the ORM fact type for
a noun's current status, its storage falls out of RMAP, and so the scat
question dissolves. That made system:partition the key, and it is now a family
of seven canonical sub-DEFs in shared/system.py (Halpin ch.10), twin-certified
against rmap_partition:
- system:rmap_top: the subtype closure to the top supertype (step 0), a WHILE.
- system:rmap_subject, system:rmap_role2: the role-1 and role-2 players, topped.
- system:rmap_mand: mandatory players. system:rmap_oneone: the 1:1 fact types,
  by intersecting the role-1 and role-2 uniqueness spans.
- system:rmap_side: the absorption side, role-1 subject overridden to the
  mandatory far role for a 1:1 (favors fewer nulls, section 10.3).
- system:partition: functional/spanning classify, then the fold to ⟨table, ft⟩
  pairs in fact type declaration order.

How it was landed, and the lessons:
- Built bottom-up against the twin, each sub-piece certified on a rich real
  model (subtypes, functional, spanning) and a synthetic 1:1 store constructed
  by hand (the reciprocal-reading FORML splits into two fact types, so the
  single-1:1-fact-type case needed a direct M-fact store).
- The construction was made ONE source of truth (partition_build.py), run with
  real builders to test and with symbolic string-builders to emit the canon
  source, so the emitted DEFs ARE the tested tree. The emitted source was
  re-executed and re-tested before it touched the canon. This auto-generation
  is the right tool for a tree this size; hand-transcription would not survive.
- Two portability lessons, both caught by the differential and the rebuild:
  (1) the partition ORDER matters. The players and side maps track fact-type
  STORAGE order; the 3NF column layout reads DECLARATION order through
  table_columns, so system:partition reverses to match (five column-order tests
  caught this, not the dict-level twin). (2) integer atoms are N(int), not
  A(int). In Python A and N are the same _atom, but in Rust A takes a string
  and N takes an integer, so A(1)/A(2) were four E0308 type errors under
  include!. The polyglot really does enforce the grammar.
- Verified: full Python suite 502 passing (partition underlies create,
  verbalize, layout, routing); Rust release rebuilds clean; cargo test 16
  passing including the native_apply differential; twin pinned for the subtype
  and 1:1 cases, matching as a mapping and in column order.

Next: with the partition canonical, the create handler's status cell falls out
of it. The own-table create handler (system:create) can now be built with no
string primitive: look the status fact type up in the partition. That, plus the
status fact type generated as ORM for governed nouns, is the path Samuel named.


## 2026-07-05 (cont): status falls out of RMAP, confirmed; the one open fork

With system:partition landed (awaiting signature), the status/RMAP insight is
now confirmed empirically. A declared "X is currently in Status" fact type is
functional, so RMAP absorbs it as a status COLUMN in X's table:
  Resource columns: ['Resource_has_Name', 'Resource_is_currently_in_Status']
No noun_status side cell, no string concatenation. This is the shape:
- Advancing a machine's status is a ROUTED CREATE of the "is currently in
  Status" fact type; RMAP routes it to the status column, unifying it with
  every other absorbed write. system:create looks the status cell up in the
  partition and never builds a name. The scat question is fully dead.
- moore_view and process_table read that fact type's population (ft_view over
  the partition), not a noun_status cell.

The ORM-ification is cross-cutting (create, machine_step, moore_view,
process_table all move off noun_status onto the RMAP status cell), and it turns
on one design FORK that is Samuel's to call:
  (a) GENERATE "Noun is currently in Status" for each governed noun at compile,
      so existing machine models (which declare only transitions) keep working.
      Backward compatible, but the generator is compile-time host code.
  (b) REQUIRE the modeler to declare "Noun is currently in Status" (explicit
      ORM). Cleaner and matches "status should be DEFINED in ORM", but a
      breaking change to the machine models in the corpus.
Samuel's wording ("defined in ORM") leans toward (b); (a) is the compatible
path. This is the next thread's pivot, and it needs his direction plus his
signature to land, so it waits for him. system:create is blocked on it, since
its only interesting own-table case (the machine trigger) needs the status
cell, and the status cell is exactly what this fork decides.


## 2026-07-05 (cont): partition committed; "Noun" grounded as Object Kind

system:partition is committed and signed (fd1de52). The biggest leaf, the whole
RMAP procedure, is in.

Samuel's grounding: "Noun" is GraphDL's verbiage for Halpin's OBJECT KIND from
the ORM metamodel. Object Type / Object Kind is the root concept of the ORM
metamodel, the supertype of Entity Type and Value Type, and what plays roles in
fact types. So the M-facts this session canonicalizes over (factType, role,
subtype, the players rmap_top/subject/role2 compute) are an encoding of that
metamodel, and the Noun a machine governs (via smDef) is an Object Kind of the
entity flavor.

Implication for status: "Noun is currently in Status" is an ordinary fact type
over an Object Kind and a value type. That is exactly why its storage falls out
of RMAP like any other fact type, and why the machine's status advance is a
routed create of it. It reinforces the fork's option (b): status modeled as a
fact type over an Object Kind is the metamodel-faithful shape, not a noun_status
side cell. The primary source is Halpin's ORM metamodel (the conceptual schema
of ORM itself); GraphDL is Samuel's surface for it.


## 2026-07-05 (cont): partition fully verified; status thread de-risked

system:partition is committed (fd1de52) and verified end to end across the
intersection: 502 Python tests, 17 rust-differential tests (cross-host, 11m),
16 cargo tests. No regression in system.create (it returns ⟨o, D'⟩; a direct
experiment that treated the pair as the store looked empty, but the corpus
exercises it via test_absorbed_machines and test_orm_run).

Status thread, mechanism confirmed:
- The status fact type "Order is currently in Status" is functional, so RMAP
  absorbs it as a status COLUMN on Order's table (observed:
  table_columns(Order) = ['Order_is_currently_in_Status', 'Order_has_Name']).
- A routed create of an absorbed fact type writes that column (proven by the
  corpus). So the machine's status advance BECOMES a routed create of the
  status fact type into its column. The mechanism is de-risked.
- The current machine writes the separate Order_status wart cell, disconnected
  from the status column. That is the gap the ORM-ification closes.

Two design decisions remain, both Samuel's, before the (complex, cross-cutting)
machine_step rewiring:
1. The fork: DECLARE the status fact type (scat-free, the name lives in model
   text) vs GENERATE it (reintroduces name construction). His guidance leans
   declare.
2. IDENTIFICATION: how machine_step finds WHICH fact type is the Noun's status,
   without reconstructing "Noun_is_currently_in_Status" (which would be scat).
   Likely a small marker M-fact linking the smDef Noun to its status fact type,
   so the machine looks it up rather than builds the name. This is the piece
   that keeps the rewiring scat-free.
Once these are set, the rewiring is: machine_step emits a routed status create
into the column (not a noun_status write); moore_view and process_table read
the status fact type via ft_view; noun_status retires. Each step twin-checkable.


## 2026-07-05 (cont): the metamodeling standard is Halpin's ORM (fixed)

Samuel's correction and clarification, recorded so it is not re-litigated:
- The metamodeling STANDARD is Halpin's ORM. The root concept is OBJECT TYPE,
  with two kinds, Entity Type (referenced via a reference scheme) and Value Type
  (lexical, self-identifying). Fact Type, Role, uniqueness/mandatory Constraint,
  Subtype, and RMAP (Halpin ch.10) are the metaschema. The code already speaks
  this: FORML's "is an entity type" / "is a value type", the role otype (object
  type), the ObjectType vocabulary throughout.
- "Noun" is GraphDL's SURFACE synonym for Object Type, specifically the
  entity-flavored one a state machine governs. It is localized to the
  state-machine layer (smDef, "is for Noun", noun_status), inconsistent with the
  otype vocabulary elsewhere. Read it as: the governed Object Type (an Entity
  Type). Do not carry GraphDL's surface word as if it were the standard.
- Purity: pyarest is built from first principles (the primary source library:
  Halpin ORM, Backus FFP, Codd 3NF, Mealy/Moore, GMS93). It is now fine to touch
  arest because we are merging (0.9.0), but the discipline is to KNOW the
  standard and name it, not to conflate GraphDL surface with the ORM metaschema.

Implication for the status thread: "Object Type is currently in Status" is an
ordinary ORM fact type (the governed Entity Type related to a Status Value
Type). That is why its storage falls out of RMAP as a column and why the
machine's advance is a routed create. The design was right; the vocabulary is
now corrected to the standard.


## 2026-07-05 (cont): compiler arc surveyed and planned

The compiler (python/compiler.py, 2042 lines) is the other canonicalization arc
Samuel named. Survey of its canonical-vs-Python surface, so the arc is ready to
drive bottom-up (the way partition was):
- ALREADY CANONICAL: the grammar recognizer. system:class_rule and
  system:class_subj exist; classify -> analyze reads statement field-facts and
  intersects them by clause (the recognizer as one FFP object). This is the
  Stage-1 classification, done.
- STILL PYTHON, the arc's work:
  1. FIELD EXTRACTION: _reading, _fact_type, _role_facts, _value_constraint,
     _name_refmode, _subject, _num, _role_path. These scan a classified
     statement and pull out the ordered role object types, the reading template,
     the value spec, etc. The keystone is _reading as an alpha-shaped matcher.
  2. THE 38 TRANSLATORS (_h_entity, _h_value, _h_uniqueness, _h_mandatory,
     _h_spanning, _h_subtype, _h_fact, _h_sm_def, ...): each takes the classified
     groups and emits the M-facts (factType, role, constraint, subtype, smDef,
     ...). Each becomes a canonical translator DEF, alpha over the fields.
  3. ORCHESTRATION: statements() splitting, analyze/classify dispatch, the
     _prepass_context / _implicit_nouns two-pass, _known. The fold that runs the
     translators over the statements.
- Order to attack: field extraction leaves first (they are the reusable pieces
  the translators call), then the translators one family at a time (twin-oracle
  each against the Python _h_), then the orchestration fold. Same discipline as
  partition: one source of truth builder, symbolic emit, re-validate, land.
- _h_sm_def (line 1106) is where "is for Noun" enters M; when the arc reaches
  it, the Object-Type grounding says that role's player is an Entity Type
  (Object Type), Noun being GraphDL's surface for it.

Session state: partition thread COMPLETE (committed fd1de52, verified 502+17+16).
Grounding fixed to Halpin ORM / Object Type. Two threads open: the status/machine
rewiring (ready, holds for Samuel's go-ahead, being cross-cutting and a storage
layout change), and this compiler arc (unblocked, multi-session, plan above).


## 2026-07-05 (cont): compiler keystone identified precisely

Deeper survey of the compile path pins the keystone:
- Classification is DUAL: a bootstrap regex table (_CLASSIFY, ~30 patterns but
  the SEED is restricted to 5 kinds: fact_type_reading, class_rule, value_type,
  value_constraint, entity_type) plus the grammar-as-readings path, where
  class_rule (canonical) recognizes a statement from its FIELD FACTS
  (Statement_has_Keyword, Statement_has_Verb, ...). forml2-grammar.md is "the
  parser is this file". So classification is mostly canonical already; the
  regexes are the minimal bootstrap.
- The genuinely-Python keystone is FIELD-FACT EXTRACTION: statement text ->
  Statement_has_X field facts. This is character/token scanning. The FFP kernel
  has cat, cellkey, reverse, length, but NO string-scan primitives (split,
  index-of, substring, char-class match). So canonicalizing extraction needs
  (a) a small set of string primitives added to the kernel per host (like cat),
  and (b) an FFP scanning discipline (the grammar-as-lambda matcher). This is
  research-grade, the "restricting the grammar restricts you to lambda" keystone,
  and wants a focused start, not a fatigue-tail push.
- The translators (_h_*) sit downstream: the pure ones (_h_ref_scheme,
  _h_objectification, _h_meta) are trivial groups->facts and canonicalize
  easily but are low value alone; the valuable ones (_h_uniqueness, _h_fact,
  _h_sm_def) call the extraction (_fact_type, _reading), so they are gated on
  the keystone.

Plan for the compiler arc when taken up with focus: (1) add the minimal string
primitives to the kernel (Python + Rust, with the differential), grounded in
what _reading/_name_refmode actually scan for; (2) express _reading as the
alpha-shaped FFP matcher over those primitives (the keystone); (3) then the
field-fact extraction self-hosts and the translators follow. Primary sources:
Backus FFP for the algebra, forml2-grammar.md for the grammar-as-readings target.


## 2026-07-05 (cont): status thread resolved from the whitepaper (AREST.tex)

Samuel pointed me to AREST.tex process-model execution to ANSWER the design
questions rather than ask them. It does, authoritatively:

- Status is status(e), a FACT: "A state machine is itself a set of facts (a
  status, its transitions, and the trigger fact type of each), and advancing it
  is one AST step, not a second machine." So noun_status is a wart; status(e) is
  the ordinary "<Object Type> is currently in Status" fact, stored per RMAP as a
  column (verified earlier: it partitions onto the Object Type's table).
- The advance is the guarded AST transition (Prop. onestep: the live step is
  mu(SYSTEM:x)=<o,d>), and transitions are state-transition rules OUTSIDE the
  monotone lfp F_S (Def. derive: "entity introduction confined to resolve_S and
  to state-transition rules... mints one surrogate per guarded step, outside
  F_S"). So the advance stays the guarded step (today's machine_step is that
  step), NOT an ordinary run_rules rule. The fix is where status(e) is written,
  not the step mechanism.
- The machine is part of the compiled schema S (line 91: S includes "a set of
  state machines"). So the status fact type comes through COMPILE, exactly
  Samuel's "isn't compile required for status" point. The fork (declare vs
  generate) dissolves: the machine IS its facts, so the compiler produces the
  status fact type from the machine definition; identification is by
  construction. links(e)=nav(e) union transitions(status(e)) (Thm. hateoas)
  reads status(e) as the fact it is.

The GAP, precisely: _h_sm_def emits smDef/smFrom/smTo/smTrigger/smStatus but no
"is currently in Status" fact type, so machine_step invents noun_status.

Implementation path (whitepaper-grounded, compile-first per Samuel):
1. A compile-time generator (beside governance_rules/generator_cells): for each
   smDef <sm, ObjectType>, produce the status fact type "<ObjectType> is
   currently in Status" (functional, so RMAP absorbs it as a column) plus a
   marker fact linking the smDef to it. Reuse the compiler's reading machinery
   (_fact_type over the instantiated reading) so the name is compiled, not
   hand-built. Runs before layout_cells/partition.
2. machine_step writes status(e) to that fact type's RMAP cell (the routed
   absorbed write into the column), not noun_status. It looks the fact type up
   via the marker.
3. moore_view / process_table read status(e) via ft_view over the status fact
   type, not noun_status.
4. Retire noun_status.
Each step twin/parity-checkable against the current machine behavior.

## 2026-07-05 (cont): status compile-side mechanism VALIDATED

Prototyped step 1 of the status path: for a machine model that does NOT declare
the status fact type (the corpus shape), generating "<Noun> is currently in
Status" + "Each <Noun> is currently in at most one Status" through the ordinary
compile path (forml.compile_model with D=existing) compiles clean and partitions
onto the Noun's table as a status column:
  Order_is_currently_in_Status -> Order ; Order columns: [Order_is_currently_in_Status]
So the compiler produces the status fact type from the machine definition with
its NAME COMPILED (never hand-built), reusing all the reading machinery. This is
the whitepaper's "the machine is its facts" and Samuel's "compile is required
for status", working. The marker (smDef -> status fact type) stores that
compiled name so machine_step looks it up rather than reconstructs it.

The remaining status work is ONE COHERENT unit (cannot land piecemeal, since
adding the status column changes machine-model schemas): the status_facts
generator wired before layout/partition + the marker; machine_step writing
status(e) into the RMAP column instead of noun_status; moore_view/process_table
reading the fact type via ft_view; noun_status retired. Parity-checkable against
current machine behavior at each step. This is the foundational cross-cutting
change; the design and the compile-side mechanism are now both proven.

## 2026-07-05 (cont): status machine-side scoped (the careful part)

The status write is not a surface cell write: build_system (ast:build_system ->
ast:bs_o / ast:bs_commit) threads the machine slot (status_cell, sm_obj,
role_pos) through the single-step commit, and the noun_status write happens
inside that commit chain. So redirecting status(e) to the RMAP column is a
rewiring of the canonical AST machine layer, not a one-line change:
- The status advance must become a routed absorbed write into the Object Type's
  row status column (row_resolve), sharing the commit chain like the index cell
  already does, rather than a whole-cell write to noun_status.
- machine_step (system:machine_step) resolves the addressed entity's role
  position already; it must resolve the status column position (from the
  partition, via the marker) and write there.
This is the one genuinely intricate piece and the place to be deliberate rather
than fast. Everything else in the unit (the compile-side status_facts generator,
the ft_view reads in the views, retiring noun_status) is mechanical once the
machine writes the column.

Session close-out state: design resolved from AREST.tex, compile-side mechanism
proven, machine-side scoped. Committed and verified this session:
system:table_columns (f4e3515), system:governed_player (7da94e3), system:
partition (fd1de52) — the RMAP storage model as canonical lambda. The status
unit is the next focused build; it lands coherent (generator + machine column
write + view reads + noun_status retired) parity-checked against current machine
behavior, since it changes every machine model's 3NF schema.

## 2026-07-05 (cont): status-as-derivation settled from the source

Samuel: "does the paper say specifically how status is derived from the event
fold?" It does, and reading the derivation sections settles the dichotomy.

- Prop. onestep: machine(s0,E) = foldl transition s0 (order_tau E) is the fold.
- Prop. derive (Derivability): every value in repr(e) -- selectors, DERIVED
  FACTS, violations, links -- is (rho f):P. So status(e) is a rho-application;
  the fold IS its derivation. Not an opaque write.
- Def. derive + Lemma finiteness: the state-transition rule is GUARDED and
  entity-introducing, "outside F_S" (mints one surrogate per guarded step). So
  status is NOT an ordinary ** F_S rule that run_rules maintains; the live
  advance is the guarded AST transition (transaction/arrival order), while the
  fold is the reconstruction (valid time, order_tau) for migration and audit.
- order_tau vs arrival = Halpin's valid time vs transaction time; tau is a role
  of the event fact, so order_tau is derivable from E; arrival is the log's.

Resolution (both layers):
1. status(e) = the transition-fold, a rho-application; the engine already has it
   as run/machine_run; deterministic replay rebuilds it from event facts (.db
   disposable). This is the reconstruction / audit / migration face.
2. Live value = the GUARDED state-transition step (the AST transition, today's
   machine_step), legitimately outside run_rules. I DO still work through it.
3. Materialization = the RMAP status column, a ** derived-and-stored fact.
   noun_status was that materialization done by hand into the wrong cell.

So the machine_step change is not "redirect a cache": it is "the guarded
transition materializes status(e) into its RMAP column," with the fold (run) as
the reconstruction replay and audit use. The status_facts generator gives
status(e) its RMAP home; machine_step writes the column as the guarded step's
effect; the fold reconstructs; noun_status retires. Implementation anchored here.

## 2026-07-05 (cont): status machine-side precisely designed; compile-side built

Compile-side DONE (tests/test_status_orm.py, 2 passing): system.status_facts
generates each governed Object Type's "is currently in Status" fact type through
the ordinary reading path (name compiled), RMAP absorbs it as the status column,
and the smStatusFt marker is EXTRACTED from the compiled result (the new fact
type's role-1 player). Held in the working tree, not committed alone, since it
only becomes coherent with the machine-side.

Machine-side precisely mapped in build_system's commit chain (shared/ast.py):
- ast:bs_spop fetches the status pop from the machine slot's status_cell
  (FetchPop N1(N6)); bs_snew = sm_obj (the transition) applied to it -> new pop.
- ast:bs_commit_m does Store(N1(N6), bs_snew): a WHOLE-cell store of the new
  status pop into the noun_status cell. That is the noun_status write.
The RMAP-correct change: status(e) is functional -> absorbed -> the addressed
entity's row status column, and the live step advances only the addressed entity
(SYSTEM:x routes on it), so bs_commit_m should ROUTE the new status fact
<addressed entity, new status> into that column -- one routed absorbed write
reusing row_resolve + the index-cell commit pattern (build_system already
threads an index_cell through the same chain), instead of the whole-cell store.
The status column position comes from the partition via the smStatusFt marker.

This is the one intricate AST change; it is now designed, not unknown. The
coherent status unit lands as: status_facts wired before layout/partition +
marker; bs_commit_m routing the status fact into the column; moore_view/
process_table reading via ft_view; noun_status retired. Parity-checked against
current machine behavior. The design is fully anchored to AREST.tex; what remains
is careful implementation of the commit-chain routing.

## 2026-07-05 — status machine-side: the column write works (target test green)

row_overwrite landed (432e924). The machine-side column read+write is now in
shared/ast.py, target test test_machine_advances_status_in_the_rmap_column PASSES:
- bs_spop dual-path: atom(N1(N6)) -> FetchPop(noun_status cell); else the
  <table,col,width> sequence -> system:ftpop_absorbed reassembles the column pop.
- bs_commit_m dual-path: atom -> Store the new pop to the cell; else the curried
  column writer bs_write_pop2, composed EXACTLY like Store(cell) is.
- bs_write_one (one entity: cellkey+FetchPop+row_overwrite+Store), bs_write_pop
  (WHILE fold over the pop), bs_write_pop2 (curry: tcw -> <pop,D>->D', tcw embedded
  via <CONST,tcw>). All emitted from emit_bs_write.py (no hand-transcription).
- create_spec builds the <table,col,width> slot when smStatusFt marks the noun's
  status ft; build_system encodes a tuple machine[0] as a sequence (else A()).

KEY BUG + FIX (the crux): my first bs_commit_m column branch was
S3(COMP, bs_write_pop, construction) -- a LITERAL <COMP, f, g> where g is still a
FUNCTION, so it reduces to a composed function, not a store D'. The noun_status
Store builds <COMP, Store(cell), <pop,D>> where the 3rd is already-applied DATA,
reducing to Store(cell):<pop,D>=D'. Fix: mirror it exactly with a CURRIED writer
bs_write_pop2 (takes <pop,D> like Store(cell) takes <value,D>), composed via
S4(CONS, K(COMP), bs_write_pop2 . N1 . N6, [bs_snew, bs_commit_base]).

Dual-path is backward-compatible: existing machines (no status_facts) keep the
noun_status atom path. PENDING for the full unit: wire status_facts into the
compile pipeline (activates the column path for ALL machines, changes every
machine schema), retire noun_status (moore_view/process_table/changed-set read
the column), fix affected tests. Landing plan: this dual-path capability +
status_facts + target test as one increment (parity-verified); wire+retire next.

## 2026-07-06 — compiler canonicalization arc opens: system:sm_rows (translator leaf 1)

Redirect honored (Samuel: "primary target is ORM, Backus combining forms, and
lambda — tweaking just the python engine is a waste of time"). The noun_status
test-migration grind was reverted; the compiler arc is the thread.

SCOPING (grounded in the code, not guessed): production compile IS the self-host
(compile_model -> compile_model_selfhost): Stage-1 tokenize_statement extracts
field FACTS (vocabulary = classLit, read off the grammar's own value-type
enumerations), Stage-2 classifies by the grammar's recognizer RULES (run_rules;
forml2-grammar.md "the parser is this file"), Classification_has_Translator
dispatches via rho through DEFS. The remaining HOST pockets: (a) the translators
are REGISTERED lambdas with no canonical object (dispatch even skips
non-registered names, compiler.py ~1796); inside they re-run the _CLASSIFY
regexes for FIELD EXTRACTION (m.groups()) and the _h_* Python for M-fact
emission. In-repo doctrine for the fix: "speed as registration, the canonical
object below stays the meaning" (classSpec twin comment).

UNIT 1 LANDED: system:sm_rows — the sm-family translator's canonical meaning
object (whitepaper §1: a machine is a SET OF FACTS in M). <verb, head, l1, l2>
-> <<cell, row>...>: COND chain on the grammar's own recognizer verb (head
splits the shared 'emits' verb: Transition->smEmit, Status->smMoore); smDef/
smStatus(initial, SWAPPED to <sm, status, 'initial'>)/smFrom/smTo emit BOTH the
machinery fact and the instance fact; trigger/guard emit one row and take the
fact-type id RESOLVED (reading->id via _clause_ft stays at the boundary — its
canonicalization is a later leaf, likely a population lookup once readings ride
as facts). _h_sm_* are thin callers. Emitted from emit_sm_rows.py (builder =
one source of truth; symbolic emit; re-exec verified) — never hand-transcribed.
NOTE: shared/system.py carries NO # comments (include!'d into Rust); caught
before the build broke.

Verified: tests/test_sm_canon.py (8 forms as spec literals + twin oracle _plan
== canonical rows), 66 machine/selfhost/grammar/compile tests, rust build clean
6m43s, 50 polyglot/differential/builders/boot, broad chunks 258 + 274. Zero
regressions.

NEXT LEAVES (the arc): translate family by family — finality/objectification/
data_type (pure like sm), nouns (needs the refmode split — string boundary),
constraint families (compose the already-canonical C.* objects); then the
_clause_ft resolution as a canonical population lookup; then Stage-1's matcher
over a words/lower boundary primitive (THE keystone: the tokenizer-boundary
decision — vocabulary matching is canonical, character->word explosion is the
certified primitive, mirroring how cat/tl are the sequence boundary). Dispatch
guard at ~1796 must learn to accept compiled (canonical) translator defs when a
family's registered glue is retired.

## 2026-07-06 — the tokenizer-boundary decision (keystone design, pre-build)

The keystone ("text->FFP matcher") decomposes cleanly once the boundary is drawn
at the WORD, not the character. Every host consumer of statement text —
_reading's mixfix scan, _atomic_run_guard (Title-case runs are atomic),
_type_span (subscripted type spans), _quoted_at (literal grouping),
tokenize_statement (Stage-1 vocabulary matching) — needs only (a) per-word
LEXICAL attributes and (b) sequence algebra over word records, which is pure
Backus territory (member/Filter/iota/fold over sequences).

DECISION: three certified boundary primitives (the transducer set, like
cat/tl/cellkey are the sequence boundary):
  lex:     text atom -> sequence of per-word records
           <raw, nopunct, base, subscript, lower, title?, pre, post, quoted?>
           raw = whitespace token; nopunct = raw.strip('.;:,'); base = nopunct
           with trailing digits stripped; subscript = those digits; lower =
           raw lowercased; title? = base nonempty and first char uppercase;
           pre/post = raw.partition('-') sides (hyphen binding adj-Type);
           quoted? = inside a '...' span. All LOCAL, no grammar knowledge.
  implode: <sep, <w...>> -> one atom (joins template tokens back: '{0} was
           born in {1}' — factType rows carry template STRINGS, parity demands
           them).
  slug:    text -> id atom (the [^0-9A-Za-z]+ -> _ collapse; ID MINTING is a
           boundary act — names are data).
Each needs a Python native + a Rust native + differential scenarios.

CANONICAL LEAVES OVER THEM (each twin-oracled against the Python spec over the
WHOLE repo corpus of readings):
  system:type_span     (twin: _type_span + _atomic_run_guard — longest known
                        type at position i, optional numeric subscript, atomic
                        Title-case-run guard = one word-prefix lookup into
                        known-as-word-seqs)
  system:reading_parse (twin: _reading — the mixfix fold: left-to-right over
                        lex records, known-set matching longest-first, hyphen
                        binding, emitting template tokens + ordered roles)
  system:ftid          (twin: _ftid_from — roles substituted back, slugged)
Then _reading/_fact_type/_type_span become thin callers, which unblocks
_clause_ft (trigger/guard resolution), uniqueness/mandatory/instance-fact
translators, and Stage-1's matcher — the whole remaining regex surface.

Leaf 1 of the translator arc landed first: c2d95d3 (system:sm_rows).

## 2026-07-06 — the tokenizer boundary LANDS (lex / implode / slug, fleet-wide)

The keystone's transducer set, built exactly per the design entry above. Three
registered value ops in the cellkey slot (spec D5: value ops on names are the
boundary), implemented SIX-way and certified by the case table byte-for-byte:
Python scott + delta (registered impl, bridged), Rust scott (Rc register) +
native (prim match arms — NOTE: the native path has NO bridge to registered
ops; resolution is cells -> native prim -> process -> canon -> Bot, which is
WHY cellkey is implemented twice in rust — mirror that always), C#
(Reducer.cs), Java (Reducer.java, java 8, Locale.ROOT case-fold).

lex record (10 fields, frozen): <raw, nopunct, base, subscript, lower, qtext,
title, post, quoted, qidx> — nopunct strips '.;:' + ',' both ends (the
_atomic_run_guard strip), base/subscript split trailing ASCII digits (Halpin's
Task1 twins), qtext is the word's text INSIDE its quoted span quotes excluded
(character-wise, mirroring _QUOTED: a period outside the closing quote stays
out of the literal), post is the after-first-hyphen tail (adj-Type binding),
qidx numbers spans from 1 (0 = unquoted). Empty-string atoms round-trip and
compare fine (probed before freezing).

7 differential cases added to shared/scenarios.py (lex words/quoted/hyphen,
implode tpl/underscore, slug reading/hyphen). C#/Java kernels consume the SAME
case table and assert no missing cases — both hosts are LIVE on this machine
(dotnet + jdk1.8), so fleet impls were mandatory, not optional. Java's
Canon.java regenerates at test time (gen_canon.py); C#'s Canon.g.cs at build.

Rust E0631 lesson: fn-pointer args do NOT deref-coerce (&Rc<Leaf> vs &Leaf) —
pass closures |l| leaf_str(&l) to and_then; direct calls coerce fine.

Verified: test_lex_boundary (records as spec literals + corpus twin vs the
host expressions), all six kernel paths on the case table, 56 polyglot/
differential/builders/boot, broad 265 + 274. Zero regressions.

NEXT: the canonical leaves over the boundary — system:type_span (twin:
_type_span + _atomic_run_guard), system:reading_parse (twin: _reading),
system:ftid (twin: _ftid_from) — each twin-oracled over the whole repo corpus
of readings; then _reading/_fact_type/_clause_ft become thin callers and the
constraint/reading translator families canonicalize over them.

## 2026-07-06 — the mixfix reading scan is CANONICAL (system:reading_parse + ftid)

The keystone leaf over the lex boundary, landed exactly per the design entry.
Eight sub-DEFs in shared/system.py, emitted from emit_reading_parse.py (staged
builder: each sub-DEF real-V tested before the next was built; symbolic emit;
re-exec re-verified against the corpus):
  system:rp_take/rp_drop  first-n (dynamic selectors over theta:iota — numbers
                          ARE selectors, apply:<i,xs>) / drop-n (WHILE countdown)
  system:rp_marker        i -> "{i}" via implode("", <"{", i, "}">) — NO scat
  system:rp_kw            knowns -> <name, words> pairs (lex each known)
  system:rp_match         maximal munch (INSERT max-by-length over the Filter
                          of candidates, ORDER-INDEPENDENT — no pre-sorted
                          operand, unlike the host's sorted kset) + the atomic
                          Title-case-run guard PER CANDIDATE; all dispatch COND
                          (an out-of-range continuation selector must stay
                          unevaluated — strict and/or would poison on ⊥)
  system:rp_step          hyphen binding first, else best match, else the head
                          word joins the template
  system:reading_parse    <text, knowns, stop> -> <template, roles>
  system:ftid             markers substituted back through a trans<iota,roles>
                          table (quasiquoted per-token eq), imploded, slugged

TWIN ORACLE: all 246 fact-type readings in shared/base answer the SAME
(template, roles) as _reading — 0 divergences — plus six-path cross-host
certification (case:reading-parse / -munch / -ftid exercise the WHOLE scan:
lex+implode+slug+iota+trans+WHILE+INSERT+Filter through every fleet kernel).

DOCTRINE CALL: _reading/_ftid_from stay as the certified-equal performant
overrides (they are the compile hot path; the directive's exact shape: meaning
in the canonical DEF, host code as certified-equal override, the equality
enforced by test_reading_canon every run). The thin-caller flip is right for
cold paths (table_columns, sm_rows handlers), not the per-statement scan.

DEBUG LESSON (selector depth): in an UNPAIRED step context the head-record
field is N8∘N1∘N1 (state->rem->head->field), one deeper than the paired
<state,best> context's N*∘N1 state fields. The ⊥ showed at step 1; probing
each piece (hpost, rp_match) directly on the state found the 2-deep selector
in minutes. Probe pieces, never stare at the whole tree.

This unblocks, in order: _clause_ft resolution as a canonical lookup,
translate_fact_types/instance-fact families, the uniqueness/mandatory
constraint families (Halpin ch.4-7, composing C.* objects), and Stage-1's
vocabulary matcher as a canonical DEF over lex + classLit.

## 2026-07-06 — noun_status is GONE (the pre-1.0 breaking unit)

Samuel: remove the wart; sub-1.0 exists exactly so rewrites like this are
possible; the PRIMARY TARGET IS RUST (python is the ARC paper-track vehicle).

The dual-path is deleted; the RMAP status column is THE semantics:
- shared/ast.py: bs_spop = the ftpop_absorbed column read, bs_commit_m =
  null->base else the bs_write_pop2 column write. No atom dispatch (the
  canonical AST is what rust runs — this is the primary artifact's change).
- create_spec: a governed trigger WITHOUT its status column now raises
  (a machine without "is currently in Status" is an incomplete model);
  build_system encodes the slot with one to_lam (tuple -> seq).
- _status_rows: column only ([] for an unmachined noun); the post-create
  changed-set names the STATUS FACT TYPE (through the governedBy closure),
  not a concatenated cell name; protocol.actions reads through _status_rows
  resolving the machine's object type via smDef (drops its hardcoded
  Resource_is_currently_in_Status half-step).
- 13 test files migrated to the column pattern (setup = status_facts +
  layout_cells; seed = create of the status fact; read = ft_view). The
  low-level build_system contract tests wire synthetic <table, col, width>
  slots with row cells; row_overwrite's rr_fresh handles absent rows.
  test_persistence's replay is now MORE faithful: the initial status is a
  LOGGED create, so replay rebuilds it (facts as the source of truth).

TWO REAL BUGS the removal exposed (both production, both fixed):
1. (earlier, 5adbdc1) subtype governance missed the object type's column.
2. status_facts compiled its generated reading WITHOUT context_from=D — a
   model that DECLARES 'Status is a value type' skipped the conditional
   declaration line, so 'Status' was unknown to the inner compile and the
   status fact type minted UNARY (the column held 'T', not the status).
   Fix: context_from=D — the model's own types resolve as role players.
   Found by migrating test_verb_parity's actions test (machine 'Flow' for
   noun 'Ticket' — also exercises machine-name != noun through smDef).

Verified: machine/status sweep 71 + fix verification 21, cargo test 15,
broad 265 + 280, derive differential + ported scenarios + C# + Java 12.

## 2026-07-06 — fold checks PASS; metamodel decision: noun-direct status

Pre-fold checks (Samuel: "Run both checks"):
1. FLEET REHEARSAL on the current engine: 8/8 compile from readings clean
   (temp root, live .dbs untouched). Every real machine gets its status fact
   type: tasks(1), bill-negotiation(1), claude(2: Engineering_Lever, Defect),
   support.auto.dev(ALL 7). kernel/spd-1/message-vetting/arc-stack carry no
   compiled machines (their "State Machine Definition" greps were prose).
   Unclassified: 5 lines fleet-wide, all documented prose/import shapes.
2. AREST _status READERS: 433 currently_in_Status refs in mainline — my first
   "greenfield" read was WRONG (a killed grep's empty section). Truth: the OLD
   engine already says "is currently in Status", but SM-INSTANCE-keyed
   (State_Machine_is_currently_in_Status + a derivation to Resource status),
   because "Resource is an abstract noun so RMAP cannot absorb the status
   into a Resource cell" (instances.md's own comment). ~330 refs are the old
   engine implementation (replaced by the import); the true interop surface:
   ui_apps/actions.rs, orient.rs, mcp/server.ts, two TS tests, the metamodel
   readings, and the conformance worktree's Wine cells.

DECISION (Samuel, 2026-07-06): ADOPT NEW — noun-direct status is canonical at
0.9.0; the SM-instance status metamodel retires with the import. The new shape
dissolves the abstract-Resource pressure (status absorbs into each CONCRETE
governed noun via smDef + governedBy). Recorded in arest as
docs/0.9.0-status-interop.md (retire list + re-point inventory + evidence).

VERDICT: green light to fold. Sequencing: fold first, write-path flip inside
arest; the status re-pointing is the concrete first item of "interop the
shell". GREP LESSON: never conclude "zero hits" from a section that printed
empty in a killed/truncated background task — re-run scoped and foreground.

## 2026-07-06 — THE FOLD: pyarest is arest/engine, tagged v0.9.0

Samuel: "Let's go." Executed:
- git subtree add --prefix=engine (full 192-commit history preserved) ->
  arest d67e2e54 "Add 'engine/' from commit '397768f...'". Samuel's
  AREST.pdf wip stashed around the merge and restored intact.
- arest was PRE-STAGED at 0.9.0 (crates/arest + package.json both 0.9.0,
  latest tag was v0.7.0) — the import is the 0.9.0 content. Tagged v0.9.0
  at d67e2e54 (annotated).
- VERIFIED IN PLACE: 56 canon/status/lex/reading/builder tests green from
  arest/engine; rust kernel builds clean from the new location (include!
  paths relative, 6m15s); the four-kernel differential green (12: rust
  scott+native+resident, C#, Java — C# rewraps and Java regenerates its
  canon from engine/shared automatically).
- arest layout note: NO root cargo workspace — engine/rust nests with zero
  manifest changes.

DEVELOPMENT MOVES TO arest/engine. This repo (Repos/pyarest) is now the
historical source; do not land new work here. Next, inside arest: the
write-path flip, then the shell interop (first item: the status re-pointing
per docs/0.9.0-status-interop.md), then the canonicalization arc continues
(clause_ft -> translator families -> Stage-1 matcher).

## 2026-07-06 — the 0.9.0 migration, in flight (post-fold work in arest)

Paper: deduped to the repo root only (engine/ copies removed).

FLEET MIGRATION (migrate_app.py driver; every old .db snapshotted as
*.pre-0.9.0.bak BEFORE any write; delete-and-rebuild per the engine's own
recompile doctrine — compiling INTO an old-engine sqlite collides with its
NOT NULL projection schemas, found the hard way):
- message-vetting: clean; 54 unknowns = old metamodel + parse debris, no
  domain cells. kernel: 4 rows. arc-stack: 10 rows.
- tasks: 8,994 rows across 27 fts — and the gap that mattered: 1,072 LIVE
  task statuses (old Resource_is_currently_in_Status projection) had no path
  into the new shape. Hence:

THE STATUS BRIDGE (570ebf2a): old current machine state -> the new per-noun
status fact types, routed by role-occurrence population membership AFTER the
asserted replay; SM-keyed fallback joins through State_Machine_is_for_
Resource; unrouted reported; bridged targets verified through ft_view.
PLUS the bulk absorbed install in replay_entries: batch rows for an absorbed
ft land on table rows + index + ** view cache in ONE from_lam/to_lam pass
(raw ft-cell writes strand rows outside the columns; per-row creates cost
hours).

FIRST-CLASS VERBS (Samuel: "induce/ask/propose, all MCP verbs must be
available first-class, not mcp-specific"; 570ebf2a): the verb tables gain
validate, verify, actions, synthesize, explain, compile (live additive,
readings stay source of truth), propose (authoring dry-run), apps_status,
apps_create, engine_version — 24 verbs, advertise == dispatch enforced by
test, serverInfo at 0.9.0. REMAINING PORTS (gate the serving swap):
induce, ask, tutor family, select_component, apps_check, apps_register,
debug. The .mcp.json swap to the new engine's resident happens only when
Samuel's working verbs all answer engine-side.

## 2026-07-06 — FLEET MIGRATED 8/8; pushed; v0.9.0 public

All eight living apps now run on the new engine at Repos/apps (old stores
snapshotted *.pre-0.9.0.bak; events.jsonl carries the migration batches so
every recompile replays it):
  message-vetting readings-only | kernel 4 | arc-stack 10 | tasks 8,994/27fts
  + BRIDGE 1,072 -> Task_is_currently_in_Status (spot-checked 112=completed)
  | spd-1 259/19fts verify 2/2 | bill-negotiation 10 | claude 707/91fts
  verify 23/23 + bridge 12 Engineering_Lever + 5 Defect | support.auto.dev
  89/21fts verify 1/1 + bridge 1 (redone from the REAL 173MB store after the
  newest-parseable heuristic picked an empty-cells app.db shell — lesson:
  probe candidates for NONEMPTY cells, and the 173MB->89-rows reduction is
  the 400x-lighter story made concrete: the old store was metamodel bloat).

UNROUTED (reported, never guessed; preserved in snapshots): claude 4 Sherlock
Cases at 'Hypothesizing' (the claude readings declare no Case machine — add
"State Machine Definition 'Case' is for Noun 'Case'" + statuses and
re-migrate to route them; Samuel's modeling call); support 5 plan-tier ids
(Enterprise/Free/Growth/Scale/Starter — old SM-keyed config state).

PUSHED: graphdl/arest main f8108995..c01c95a2 (203 commits) + tag v0.9.0.
GitHub flags 65 dependency vulns (1 critical, 16 high) on the old shell's JS
tree — triage before external eyes touch web surfaces. First external user
incoming (transport business) — nothing started, per Samuel.

REMAINING to finish the swap: ask + induce + tutor.* + select_component +
apps_check/register engine-side, then .mcp.json -> the new resident, then
the SM-instance metamodel readings retire and the old engine leaves the
serving path. The write-path flip rides inside arest thereafter.

## 2026-07-06 — SEWN: the new engine serves; the old engine is out of the loop

- induce ported engine-side (96826e9a; old induce.rs the oracle, 5 TDD tests):
  enum+population domains, baseline-delta alethic gate, coverage gate reading
  the head's ** derive cell, Scoring Rules through the declared
  Hypothesis_Candidate_has_hidden_<N> hook (the old iff spelling), bound pins.
- ask ported (plan-executing; needs_plan + model surface otherwise — no LLM
  in the engine). apps_check / apps_register. 27 first-class verbs.
- THE ENUM-QUOTE FIX: string value-enumerations were unsatisfiable at apply
  time (quotes kept in members) — found by induce's own tests; real
  production defect fixed. THREE defects noted for later: numbered multi-word
  rule heads leak subscripts into ft ids; unnumbered IF compiles no rule;
  derived ABSORBED heads land in the ** cell but not the column.
- THE SWAP: .mcp.json 'arest' -> engine/mcp_boot.py over Repos/apps
  (smoke-tested live: 27 tools, task 112 completed answers through the new
  server); 'arest-legacy' -> the TS shell in WASM mode for tutor.* +
  select_component ONLY. The old engine CLI serves nothing.
- RETIRED: the SM-instance status metamodel block in readings/core/
  instances.md (note left in place). REMAINING: tutor + select_component
  ports (then the legacy entry deletes); rust-resident verb parity with the
  write-path flip; ui_apps/actions.rs re-point when the kernel target
  rebuilds.

## 2026-07-06 — THE FLIP OPENS: absorbed writes are native in the resident

Phase two of create_handlers, done: _absorbed_handler stores the
fact-DEPENDENT create handler WHOLE — the nine-slot record's cell name
computes at reduce time (apply(cellkey, <table, N1(fact)>)), every other
slot the spec's constant, so apply(ast:build_system(record(P)), P) reduces
exactly what _create_from_spec wires host-side. The stored handler is
byte-equal to the python create on the same operand (machine leg included,
the status column advances) — test_build_system_canon pins it.

RUST SIDE: zero engine code changed — native_apply already keyed on the
cell's presence; the absorbed handlers appearing in the sidecar flipped the
path automatically. The discriminator test (test_polyglot) runs the resident
with delegation DISABLED (bogus --python): the absorbed apply COMMITS,
queries natively, and the event log + sidecar persist; a python recompile
replays the resident-written event through the same create (the .db is
disposable — the doctrine held when the test first asserted against a stale
.db and the replay assertion was the correct one). The cross-host write loop
closes: rust writes, python replays, byte-faithful.

Remaining in the flip: apps_compile + the read long tail off cli.py
delegation, the 8 new verbs in the resident's table, then native synthesize
(~40x) -> WASM -> Cloudflare. Gates: 47 targeted + broad 274 + 281.

## 2026-07-06 — RUST-PRIMARY SERVING: the resident carries the whole table

- cli.py gains the generic `call` form: every first-class verb through the
  ONE dispatch (protocol.SESSION_VERBS/APP_VERBS) — the CLI is a binding,
  never a second verb table.
- The resident serves 28 tools (27 + its native `derive`): apps_status/
  check/register/create + engine_version + context NATIVE (filesystem +
  retained state; proven with delegation disabled); compile/propose/induce/
  ask delegate through the call form to the same engine's python. Tool-table
  parity rust==python enforced by test (name for name, derive excused).
- SMOKED LIVE: the resident over Repos/apps answers the board's 1,072
  statuses natively (112=completed) with 28 tools advertised.
- .mcp.json 'arest' -> arestlam --mcp (the RUST RESIDENT): reads and ALL
  applies native over the retained sidecar; compiler-host verbs ride python
  underneath. Rust-primary serving, one verb surface, two bindings.

## 2026-07-06 — VIEW == REASSEMBLY FOR DERIVED HEADS, both hosts

The L1 seam closed: a rule-derived ABSORBED head's ** cell is the derive
cache and its RMAP column is the storage, and run_rules now RECONCILES the
columns to the cell after the joint fixpoint — present rows write their
value onto the key's table row (fresh keys join the index, hole-padded),
vanished rows HOLE the column (the sweep's supersession reaches storage).

- python: _reconcile_absorbed_heads (engine.py), one from_lam/to_lam pass
  over the touched absorbed heads (closure_changed | strata_changed, gated
  by rmap_partition). test_view_cache pins derive→column AND
  supersession→hole.
- rust: the mirror rides op_run_rules before the retain-protocol commit,
  dispatching on the rmapColumns layout cell (rows ⟨table, col, ft⟩) with
  cellkey's S/I text addressing; store_into keeps the native mirror in
  lockstep. Iteration in `changed` (BTreeSet) order == python's
  sorted(touched); fresh keys sort ints-numeric/strings-lexical (python
  faults on a mix, so the split is free).
- DIFFERENTIAL: test_rust_derive_reconciles_absorbed_heads_to_the_columns —
  python reg.apply vs the resident (delegation DISABLED) applying the same
  fact natively; the Person:Adler row compares equal THROUGH THE SIDECAR
  (('Adler','library','T') both hosts; the cells op answers counts, so the
  sidecar is the byte-real comparator).

## 2026-07-06 — STORE OF RECORD: the stream watermark (mcp regression)

cargo --test mcp caught a TWO-LEDGER divergence the moment writes went
native: the resident commits by APPENDING to <app>.events.jsonl and
refreshing the sidecar (never the .db), while Registry._load read the bare
.db snapshot — so the delegated refusal path (native ERROR → cli.py for
violations) validated against a STALE store, wrongly committed a second
functional value, and its own _sidecar write CLOBBERED the resident's
commits (t1 vanished; the at-most-one collapsed).

Fix, doctrine-shaped (facts are the source of truth; the .db is disposable):
the snapshot now records HOW MUCH of the stream it holds — an eventWatermark
cell, stamped at compile (len(entries)) and bumped on every python commit —
and Registry._load replays the stream's TAIL beyond the watermark through
the same create, then run_rules bounded to the tail's fact types. A
pre-watermark snapshot loads as-is (complete by construction when every
write passed through the Registry); the fleet picks up watermarks at next
compile. test_apps_write pins: a foreign append is visible to query, refuses
a conflicting apply, and SURVIVES a later python commit's snapshot.

Hardening from the same failure: BOTH hosts wrote the sidecar via the SAME
tmp path (<sidecar>.tmp) — two writers tear the file (the resident parsed a
17-byte torn prefix). The tmp name now carries the pid on both sides;
os.replace/fs::rename stay atomic.

## NEXT THREAD (parked 2026-07-06): clause_ft canonicalization

The L1 arc's next pocket: compiler.py _clause_ft (constraint clause text →
fact-type id) is host regex + two-pass preference. Canonical composition,
all ingredients already in canon:

    system:clause_ft(text, D) =
      min  = scan(drop_quants_min(text), D)     drop {some,that,each,no}
      full = scan(drop_quants_all(text), D)     drop + {an,a}
      IF member(min, declared fts of D) THEN min ELSE full

- scan = the system:reading_parse → system:ftid path (246/246 twinned);
  study rp_step/rp_kw's operand shape before composing.
- quantifier drop = token-level theta:Filter over lex output (the ftid def
  at shared/system.py:3804 shows the Filter idiom); the twin test must
  prove token-drop == the host regex's word-boundary strip on the corpus.
- membership = the factType population lookup (vb_fetch/ftpop family).
- Host _clause_ft then becomes a thin caller (compile-time cold path — no
  override needed), certified by a corpus twin over every constraint clause
  in the base corpus + quantifier edge cases ('is a manager' keeps its
  article; the rule path's lesson at compiler.py:856 docstring).
- After clause_ft: the translator families (reading / instance-fact /
  constraint composing C.* objects), then stage-1 vocabulary matcher.

## 2026-07-06 — fleet watermarks stamped DIRECTLY; a compile-cost lesson

The recompile route was the wrong tool: py-spy... (py_spy can't read 3.14;
PEP 768 sys.remote_exec + faulthandler did it) showed the stamp runs pinned
inside compile_model_selfhost -> classify_all_via_M -> stage1_vocabulary
-> _pop_rows over the Scott mu — the python selfhost compiler costs 10-25
MINUTES per fleet app (the morning migration's sidecar mtimes show the
same gaps; it was never fast). LESSON: never schedule a fleet-wide python
recompile as a side errand; it is an hours-scale batch. (Stage-1
vocabulary through the reducer is exactly the hot pocket the parked
clause_ft/stage-1 canonicalization + native override should price.)

The sound shortcut: where the LOG PREDATES the .db, the snapshot provably
holds the whole stream, so the watermark stamps directly (load, stamp,
save, sidecar — seconds per app). All 8 living apps stamped and verified
(claude 93, tasks 55, support 22, spd-1 19, arc-stack 6, bill 2, kernel 1,
message-vetting 0); the resident boots the stamped sidecar and serves the
same 1,072 statuses. An app whose log outruns its .db still needs the
recompile route — none did tonight.

Sweep note: Repos/apps holds DOZENS of pre-0.9.0 experiment stores
(arc-*, gen-induce-*, spd-* probes) that no longer load ("no such column:
ord") — old-engine schemas outside the migrated fleet. Migrate-or-delete
is an open housekeeping decision.

## 2026-07-06 — OLD-RUST PARITY ASSESSMENT (for the removal decision)

Suites, all run tonight on the current tree:
- NEW engine/rust: 16/16 (lib 12, mcp 2, native_apply 1, serve_ops 1)
  + the cross-host battery: python 559/559 (58-case table byte-identical
  on 6 reducer paths; 246/246 reading corpus; derive differential 4/4;
  live fleet 1,072 statuses).
- OLD crates/arest lib: 1,967/1,968 — the ONE failure is
  sm_fold_to_bridge_derives_task_has_status_through_event_fold, which
  asserts the RETIRED SM-instance metamodel (Resource_is_currently_in_
  Status bridge). Correct failure under adopt-new; not a defect.
- OLD crates/arest integration (--features test-bins): 164/164 over 24
  e2e/fuzz binaries.
- OLD arest-foundation: 82/82.  OLD arest-kernel host (msvc): 856/856.

ENGINE parity verdict: the old engine's FUNCTION (compile, derive, apply,
validate, query, induce) is re-served at 0.9.0 and pinned by differential
gates, not by porting its tests: fleet migration 26/27 derived sets equal
(the 1 = the retired projection), scenario table, corpus twins, induce
twin-oracle, derive differential. The old suites passing tonight attest
the old implementation is healthy at rest — nothing blocks on fixing it.

REMOVAL BLOCKERS (what crates/ still uniquely serves):
1. wasm-pack pkg -> the LIVE Cloudflare worker (src/worker.ts + wrangler):
   the old REST/HATEOAS web surface. Blocker until the new WASM target
   lands, or the surface is deliberately paused.
2. .mcp.json arest-legacy -> src/mcp/server.ts: ONLY tutor.* +
   select_component. tutor is TS lesson orchestration whose engine needs
   (compile/apply/query) the new resident already serves; select_component
   is one WASM intercept. Small port.
3. The TS src/ + vitest tree rides the WASM — and carries the 65
   Dependabot findings (1 critical, 16 high). Retiring it REMOVES that
   exposure before the first external user.
4. arest-kernel: the UEFI demo target of the OLD engine. Removal loses
   the working kernel demo until the new engine re-targets UEFI.
5. induce.rs — already ported, twin-oracled. No blocker.

RECOMMENDED SEQUENCE: (A) port tutor + select_component engine-side,
delete the arest-legacy entry + src/mcp — zero-loss, do now. (B) decide
the worker: pause the old web surface (kills the Dependabot exposure;
recommended before the transport-business user) or hold removal until the
new WASM/worker phases land. (C) with A+B done, delete crates/arest +
arest-foundation (git history keeps everything; v0.9.0 tag marks the
seam). arest-kernel: Samuel's call — frozen exhibit vs delete-and-
re-target-later.

### clause_ft refinement (2026-07-06, pre-implementation)

The reading-canon twin (tests/test_reading_canon.py:70) settles the operand
convention: the canonical scan takes ⟨text, sorted known types⟩ — the
CALLER supplies the vocabulary; no store-level population lookup. So:

    system:clause_ft over ⟨clause text, known types, declared ft ids⟩ =
      min  = ftid(reading_parse(drop_min_quants(text), known))
      IF member(min, declared) THEN min
      ELSE ftid(reading_parse(drop_all_quants(text), known))

(the python host's ft_full-preferring-declared subtlety collapses: when
min missed, full is answered regardless — mirror compiler.py:856 exactly,
including its docstring's article lesson). Twin test = the corpus sweep
pattern of test_reading_canon.py:70 but filtered to the FOUR constraint
handler families that call _clause_ft (set-comparison, disjunctive,
subset, equality at compiler.py:874-906), comparing _clause_ft(text,
known) against the canonical def per clause; require a real checked
count. drop-quant defs: theta:Filter over lex tokens with membership in
{some,that,each,no} (min) / +{an,a} (full) — token-drop == the regex's
word-boundary strip must HOLD on the corpus, else the def needs the
space-joined bigram guard.

### clause_ft spec verified against the host (2026-07-06 late)

tests/test_clause_canon.py's three unit expectations executed against
compiler._clause_ft directly with _Known(names, fts=...) (compiler.py:351
— a set subclass; construction is trivial, no compile needed):
Ticket_has_Status / Employee_is_a_manager (declared article wins under
minimal strip) / Employee_is_manager (full-strip fallback) — all three
match. The red test IS the behavioral spec; the canonical def builds to
it. The corpus twin test's Known question is answered the same way.

### clause_ft: authoring facts pinned (2026-07-06, probe-verified)

- lex yields 10-field RECORDS, field 1 = the raw word. Drop-pred per
  quantifier w is the CONSTANT tree (COMP, not, (COMP, eq, (CONS, 1,
  (CONST w)))) — K-quotable whole; after filtering, rebuild text with
  implode(" ", ALPHA(1)(tokens)) (implode confirmed ⟨sep, items⟩).
- 'not' is a prim; membership(min, declared) = not∘null∘apply(⟨theta:
  Filter, dyn-pred⟩, declared) with dyn-pred built by the ftid idiom:
  (CONS, K(COMP), K(eq), (CONS, K(CONS), K(id), (CONS, K(CONST), N(1))))
  — declared ids are plain atoms so the element selector is id, not 1.
- Structure: helpers system:cf_drop_min / cf_drop_full (filter chains) +
  system:cf_scan (⟨stripped,knowns,stop⟩ → ftid∘reading_parse) +
  system:clause_ft = COND(member(N1,N3), N1, N2) ∘ CONS(min-id, full-id,
  N(4)) over ⟨text,knowns,stop,declared⟩.
- Host-divergence caveat for the corpus twin: the regex needs a TRAILING
  SPACE (a clause-final quantifier isn't stripped by the host; token-drop
  strips anywhere) — unreachable in real clauses; the twin decides.

## 2026-07-07 — clause_ft IS CANON (the L1 arc's next pocket closed)

system:clause_ft + five helpers (cf_dropw/cf_dropq/cf_text/cf_drop_min/
cf_drop_full/cf_scan) landed in shared/system.py — intersection source,
rust release build clean, reading-canon + shared-builder twins 32/32.
Unit tests (3, host-verified) + THE CORPUS TWIN: every constraint clause
the four handler families extract from shared/base answers the same id
through the canon as through compiler._clause_ft (4/4 green).

DOCTRINE CALL (reading-scan precedent): the host regex STAYS as the
certified-equal performant override — per-constraint reducer resolution
would ride the same mu cost that makes stage1_vocabulary the 10-25-min
compile pocket. Canon carries the meaning; the twin enforces equality on
every run. NEXT bottom-up: translator families (reading / instance-fact /
constraint composing C.* objects), then the stage-1 vocabulary matcher
(the ACTUAL hot pocket, where a native override pays).

### NEXT THREAD (parked 2026-07-07): the constraint translator family

Template = the sm family (compiler.py:1118-1154): ONE canonical object
answering "which rows does this statement assert" (system:sm_rows,
⟨verb, head, l1, l2⟩ → ⟨⟨cell, row⟩…⟩); handlers are thin from_lam/
to_lam callers. Note _h_sm_trigger/_h_sm_guard already compose
_clause_ft — the families chain.

The constraint family's open DESIGN QUESTION: the four handlers
(compiler.py:874-906) emit BOTH A-rows (("constraint", (cid, kind, subj,
clauses, m))) AND host-composed C.* objects (C.exclusion(),
C.scoped_exclusion(clauses, ft), ... engine.py:558-590). The canonical
form likely mirrors create_handlers' build_system(record) pattern: the
translator emits a SPEC (kind + attachment plan) as rows, and a canonical
builder composes the constraint objects from the spec — so the meaning
(what attaches where) is data and the object construction is one
canonical builder family. Study engine.py's C.* constructors + how
validate_of consumes them BEFORE choosing the spec shape. After
constraints: the reading + instance-fact translators, then stage-1
vocabulary (the hot pocket, where the NATIVE override pays).

### constraint-family arc REFINED (2026-07-07, study result)

The build_system hypothesis is WRONG-SHAPED: the C.* scoped constructors
are ALREADY canonical — engine.py:570-614's no-pops default paths apply
constraints:scoped_exclusion / scoped_exclusive_or / scoped_external_
uniqueness / scoped_inclusive_or / scoped_equality_side as canonical
DEFs over the spec operand; host composition survives ONLY for the pops
override (the RMAP view seam). The remaining host pocket is the
TRANSLATOR: the four handlers' kind mapping, cid minting, clause listing,
and attachment plan (which cell gets which scoped object with which
args). Canonical shape, following system:sm_rows exactly: system:cs_rows
⟨kind-token, subject, clause ids⟩ → ⟨constraint A-row, ⟨attach-cell,
builder-name, builder-args⟩…⟩; the handler folds attachment rows into
objects by applying the NAMED canonical builder (they exist), staying a
thin caller. Twin oracle = the same corpus sweep as clause_ft, comparing
A-rows + attachment plans handler-vs-canon.

### cs_rows: last unknown resolved (2026-07-07)

No take/substr prim exists in the kernel (probed) — and none should be
added for cosmetics: per the sm_rows boundary doctrine ("trigger/guard
literals arrive RESOLVED — the boundary's step, not the object's"), the
[:40] cid truncation is BOUNDARY work. system:cs_rows emits full-slug
cids + the A-row + ⟨attach-cell, builder-name, args⟩ rows over
⟨kind, subject, clause ids, raw texts, m⟩; the thin caller truncates at
mint. Subset/equality cids slug RAW CLAUSE TEXT (not the subject), so
raw texts ride the operand. Red tests: per-kind expected literals from
compiler.py:874-906; corpus twin on A-rows. All unknowns closed —
next window writes the DEF.

## 2026-07-07 — cs_rows IS CANON; both commits pushed

ad7ef440 (system:clause_ft + unit tests) and fed28d26 (corpus twin +
system:cs_rows + five per-kind contracts) on graphdl/arest main. Gates:
36/36 twin suites, rust release clean both times.

FLIP DECISION: the four constraint handlers SHOULD become thin callers
(unlike clause_ft's host-override call): cs_rows is pure CONS assembly —
no reading_parse WHILE inside — so per-statement reduction is cheap, and
sm_rows' handlers already run exactly this way. The flip: handlers call
cs_rows through the reducer, mint cids ([:40] at the boundary), fold
attach rows into objects via a name→constraints:builder map; the
disjunctive subject resolution (_subject) stays host-side ahead of the
call, like trigger clause resolution does for sm. Gate = the broad
python chunks (every compile exercises the four families). NEXT WINDOW.

## 2026-07-07 — the four constraint handlers are THIN CALLERS

compiler.py 874-906 rewritten: _cs_call reduces system:cs_rows per
statement, mints the cid at the boundary ([:40] on the slug for
ior_/subset_/eq_; set-comparison never truncated — traced from the old
bodies verbatim), and folds the ⟨cell, builder⟩ attachment rows into
objects through the already-canonical builders, arguments read back off
the A-row (equality's _a side checks against B, _b against A; subset
attaches once to the antecedent cell). The handlers keep only boundary
work: clause resolution (_clause_ft), subject resolution (_subject), and
the clause-text splitting the grammar owns. Gates so far: the five
cs_rows contracts + clause canon + negation + rmap remainder/routing
22/22; the FULL suite is the commit gate (running).

### THE COMPILE HOT POCKET, DIAGNOSED EXACTLY (2026-07-07)

classify_all_via_M's sweep (compiler.py:1070-1072) calls
tokenize_statement(D, stmt, ...) per statement, and tokenize_statement
recomputes stage1_vocabulary(D) — a full reducer FetchPop of classLit
through the Scott mu — EVERY CALL, though D is unchanged across the
whole loop (it evolves only after, at the batch Store). Hundreds of
identical mu reductions per app compile = the 10-25-min fleet pocket
(the py-spy/faulthandler stack from the stamp run shows exactly this
frame). THE HOIST: compute the sorted vocabulary once before the loop
and pass it down (optional param, default self-compute preserves the
single-statement callers). Behavior-identical; likely collapses fleet
compiles from tens of minutes toward the seconds the derive already
takes. Apply AFTER the flip's full-suite gate lands (separate commit,
own gate, clean bisection). The stage-1 CANON matcher remains the
meaning-side goal; this is the certified-equal host lane doing its job.

## 2026-07-07 — SHARED SOURCE: the .canon migration (spec, Samuel-driven)

Samuel: the .py extension misrepresents the polyglot files, and the Java/
C# GENERATION step (gen_canon.py byte-wrapping shared/*.py into a 5,204-
line generated Canon.java, "do not edit") must die — templating/linking,
never generation. Hard wall established: a true 4-way polyglot file is
impossible (Java/C# need a class shell at top level), so the JVM/CLR
no-generation path is RUNTIME READING.

THE SPEC — one neutral file, four native loaders, zero generation:
1. Rename shared/*.py -> shared/*.canon, bytes unchanged (the header
   string already documents the grammar).
2. Rust: five include! path edits (include! is extension-agnostic).
3. Python: canon-side loader (exec with the constructor vocabulary
   bound) replaces module import — python needs no .py either.
4. Java/C#: a ~80-line runtime Reader (identifiers, calls, double-quoted
   strings, ints, parens, commas) feeding the SAME Vocab statics;
   gen_canon.py + generated Canon.java/.cs DELETED. Certification: a
   one-time test Reader(file) == generated loadAll() output on all five
   files BEFORE deletion, plus the standing 58-case table + corpus twins.
Trade-off accepted: JVM/CLR lose language-tokenizer ingestion, gain a
tiny parser; the equality guarantee was always the certification suite.

VB.NET (Samuel asked): syntactically a BETTER intersection fit than Java
(double-quoted strings, no comment collision, implicit line continuation
in parens) — but on the CLR it rides the C# kernel's assembly for free
and adds no proof diversity (same runtime, same reducer). Filed in the
portability matrix as "CLR veneer over the C# kernel, on demand", not a
planned peer. The reader design dissolves the parser-compat question
entirely: a new host = reader + vocab + REDUCER, and the reducer is
where both the cost and the certification value live.

QUEUE POSITION: after flip-commit -> hoist -> stage-1 matcher; its own
commit + gate (touches the fold's most load-bearing files).

### .canon migration RE-SPECCED (2026-07-07, Samuel: no readers, no VB)

The reader pattern is DEAD and so is the VB idea. The polyglot challenge
is won at the EXPRESSION level: the same bytes run on every host through
its OWN native front-end — the existing generated Canon.java (return T
<verbatim bytes> ;) already PROVES javac accepts the canon unmodified;
the only sin was materializing the wrap as a repo artifact.

THE DESIGN — identical bytes, four native tokenizers, zero transforms:
- Python: module machinery executes the bytes (bare tuple = valid module).
- Rust: include! at the shell site (unchanged).
- Java: at class-load, javax.tools/JShell compiles "T" + bytes IN MEMORY
  with Vocab imported — the two-token shell is a call-site prefix, the
  include! analog; javac tokenizes the canon. gen_canon.py + generated
  Canon.java DELETE.
- C#: Roslyn (CSharpScript / in-memory CSharpCompilation) over the same
  "T" + bytes. Generated wrap deletes.
Costs accepted: Java needs jdk.compiler at runtime; C# takes Roslyn; one
boot compile of the ~5k-line expression each (seconds, cacheable). The
.canon rename rides along (all four mechanisms are extension-agnostic).
Doctrine: lambda-calculus + set-theory base = host-independent semantics;
the shared syntax (calls, strings, ints, parens) = the intersection of
every expression grammar; per-language residue = the LINKING mechanism,
which each toolchain already ships.

### .canon migration, THIRD spec (2026-07-07: the XSLT reading — store boot)

Samuel vetoed dynamic compilation too ("something similar to what xslt
does"). The XSLT essence: ONE format for program+data, standard machinery
loads it AS A TREE, a tree-walker interprets — no compilation. The rust
resident ALREADY works this way: every sidecar's "process" table is the
defs as data; boot deserializes, the mu tree-walks. So:

- The tuple .canon files stay the single AUTHORED source; the two ROOT
  hosts consume them natively (python import, rust include!) — they must
  exist before any store does.
- Every OTHER host boots the canon FROM THE STORE — the defs emission the
  engine already produces on every compile (derived state like the .db,
  never a synced source artifact; the Canon.java sin was the second
  source of truth, not moving bytes).
- JVM/CLR kernels graduate from "reducer + pasted source" to LOADER + MU
  (mini-residents); their loader is the PROTOCOL (the lam serialization
  every AREST host/client speaks, ~60 lines, certified by the same
  sidecar-boot parity pinning rust against python), categorically not a
  bespoke canon-syntax reader.
- HOST, DEFINED: a store loader and a mu.
- Honest residue: strict same-BYTES on Java is a wall (JDK text→tree =
  compiler | hand parser | XML only); same-STRUCTURE emitted by the
  system is the faithful XSLT reading (stylesheet and data share the
  document model, not bytes).
STATUS: CONFIRMED (Samuel, 2026-07-07: "Seems fine");
gen_canon.py + generated Canon.java/.cs delete under this spec too.

## 2026-07-07 — THE HOIST LANDS: fleet compiles ~10x faster; stage-1 is a prim

- classify_all_via_M fetches the stage-1 vocabulary ONCE per sweep;
  tokenize_statement takes it as a parameter (self-computing standalone).
- stage1_fields joins the lex boundary (engine.py, registered beside
  lex/implode/slug): the pure core moved whole from the compiler, the
  regex as the certified implementation per the operating rule; four
  contract tests resolve through the reducer.
- MEASURED: bill-negotiation-service full compile 101.4s post-hoist vs
  the 10-25 MINUTE pre-hoist range (the stamp attempts never finished it
  in 30+). claude timing pending; suite gate running.
- Targeted: stage-1 contracts + grammar + grammar_selfhost + clause +
  constraint canon, 22/22.

### hoist + stage-1 prim FULL GATE (2026-07-07 early)

Chunked 3-way parallel (the monolith lesson applied): 202 + 182 + 188 =
572 passed, 0 failed, ~31 min wall vs ~59 monolithic. The batch (vocab
hoist + stage1_fields prim + contract tests) is certified; commits queue
behind the pinentry (flip staged first, this batch second — no git add
until the flip commit lands, so the changesets stay separate).

### NEXT: the reading / instance-fact translator (dissected 2026-07-07)

_h_fact (compiler.py:1112): the instance branch (quoted ids -> ⟨ft, ids⟩
with the subtype lift) and the scan are BOUNDARY, per the sm precedent;
the canonicalizable object is _fact_type's ASSERTION PLAN — which M-rows
a fact-type declaration asserts (the factType row, the ordered role rows
with positions, the derivation link when marked). Since the scan is
already canon (reading_parse/ftid), the new object is system:ft_rows over
⟨template, ordered roles, derivation-kind⟩ → ⟨⟨cell, row⟩…⟩, sm_rows-
shaped. Red tests first from _fact_type's observed outputs; corpus twin
over every fact_type_reading in shared/base (the 246-reading harness
filtered to declarations). THEN the L1 translator arc is COMPLETE and the
.canon migration (confirmed spec v3) executes.

## 2026-07-07 — system:ft_rows: THE L1 TRANSLATOR ARC IS COMPLETE

The last family: ⟨template, roles, kind⟩ → the M-rows a fact-type
declaration asserts (factType ⟨ft, template⟩; role ⟨ft.i, ft, i, player⟩
per role, numbered by trans⟨iota∘length, roles⟩ and carried through
distl; the derivation link appended by apndr when a storage marker rode
the reading). 3 unit contracts probed from _fact_type verbatim + the
corpus twin (50+ declarations of shared/base, plan-for-plan) — 45/45
with every intersection suite; rust ingests clean (4:51 build).

THE ARC: sm_rows + cs_rows + clause_ft + stage1_fields + ft_rows — every
translator family canonicalized, each certified by its own twin. Queued
follow-up: _fact_type's plan assembly becomes a thin caller of ft_rows
(the cs pattern; cheap CONS assembly), own gate. THEN the .canon
migration (confirmed spec v3) executes.

Commit queue on the pinentry: (1) the flip [staged], (2) hoist + stage-1
prim [gated 572/572], (3) ft_rows + twins [gated 45/45 + rust].

### _fact_type flip: DECIDED NO (2026-07-07)

The queued follow-up closes as decided-no: _fact_type is a WARM path
(every reading resolution; clause_ft's host override calls it twice per
clause; rule atoms ride it), so the reading-scan precedent applies
verbatim — the corpus twin (plan-for-plan, 50+) IS the certification and
the host assembly stays as the certified-equal override, exactly as
_reading was never flipped onto reading_parse. The cs flip's ~6-min
suite cost was accepted for a per-STATEMENT path; compounding another
onto per-RESOLUTION would buy no meaning (ft_rows already carries it).

QUEUE DISCIPLINE: the .canon migration does NOT start until the pinentry
flushes the three queued changesets — shared/system.py is inside
changeset 3's uncommitted state, and renaming it now would entangle the
migration with ft_rows in one commit.

### NEXT THREAD SCOPED: scheduler in canon (run_rules as data) — the last L1 chip

The fixpoint SCHEDULER is the remaining host pocket: engine.py run_rules
(the semi-naive closure + mirror blocks + the joint upper strata: agg
whole-replace/per-group, keyed upsert, sweep, DRed-cyclic, dirty gating,
the absorbed-head reconcile) and its rust mirror op_run_rules. "As data"
= the SCHEDULE becomes a canonical structure the hosts interpret: an
ordered pass table ⟨pass kind, head selector, gate predicate,
supersession mode⟩ + the round loop's fixpoint discipline — the L1 face
of the pipeline-as-data endgame (L4's "glue endgame" chip). Primary
sources to re-read before designing: engine.py:1059+ (run_rules),
rust/src/main.rs:2172+ (op_run_rules), Bancilhon-Ramakrishnan 1986
(semi-naive), GMS93 (DRed + counting, filed in-repo). Design questions:
(1) is the pass table M-facts (rulePass rows) or DEFS-level data; (2)
where does host speed live once the schedule is data (the passes' bodies
stay native, the ORDER/GATES become data — the likely cut); (3) does the
12-round bound become a datum. Deserves a fresh window with a flushed
commit queue; do not start on top of three parked changesets.

## 2026-07-08 morning — RECOVERY: queue flushed, .canon landed, fleet fast

The overnight was wasted on rubber-stamped "holding" ticks (the lesson is
filed: blocked-on-x-is-not-blocked-on-everything, an Operating Rule in
the claude app). The recovery, first hour:
- Queue FLUSHED and pushed: 323bc5cb (handler flip) + 221ab9cc (hoist +
  stage-1 prim) + e1da8150 (ft_rows, the arc-closer).
- arc-stack compiles in 1.9s POST-HOIST (was 40+ min unfinished) —
  watermark stamped 6/6; the whole fleet is now seconds-scale.
- .canon migration STAGE A landed and pushed (25c3130f): five renames
  (bytes untouched), every host repointed through its native front-end,
  intersection.md re-written. DISCOVERY: Canon.java was gitignored all
  along (generated locally) and the C# wrap lives under obj/ via MSBuild
  ReadAllText — NO host had a checked-in artifact; the sin was smaller
  than believed, the rename still right. Gates: 60/60 targeted + both
  peer kernels + rust build; broad chunked suite running.
- Delegation VERIFIED working (haiku agent self-reported Haiku 4.5) —
  the model-override pause lifts. First delivery: the apps inventory —
  109 LOADABLE-LEGACY old-engine stores (1,085 MB, largest arc-agi-3 at
  168 MB), 10 shells, 8 living. Migrate-or-delete is Samuel's call;
  deleting class (a) frees ~1 GB and removes the "no such column: ord"
  noise from every registry sweep.
NEXT: java store-boot loader (stage B, retires gen_canon.py), then
scheduler-in-canon (the last L1 chip).

### stage B REASSESSED (2026-07-08): the java wrap is already conformant

Spec v3's store-boot loader was designed against the belief that
Canon.java was a checked-in 5,204-line sync hazard. It is GITIGNORED —
local derived state, regenerated by a 30-line wrap script — which makes
the java path exactly isomorphic to the accepted C# path (MSBuild
ReadAllText into obj/): same bytes, the language's own front-end
(javac), no repo artifact, no reader, no dynamic compilation.
gen_canon.py IS java's include!/ReadAllText shim. Replacing it with a
~150-line protocol loader + a defs-emission step would ADD machinery and
LOSE javac-tokenizes-the-canon — motion without progress. RECOMMENDATION:
accept the wrap as java's native front-end; the store-boot loader
remains the right shape LATER, when the JVM/CLR kernels grow into
mini-residents (they need store loaders to serve apps anyway) — build it
then, motivated by serving, not by canon loading. Pending Samuel's nod;
the L1 effort goes to scheduler-in-canon instead.

### ROOT CLEANUP PROPOSAL (2026-07-08, inventoried by delegation — awaiting nod)

140 debris files at the arest ROOT, ALL untracked, ~6.3 MB: 129 *.log
(May-June session logs, 6.0 MB), 4 LaTeX build files (AREST.aux/.log/
.out/.synctex.gz — the .tex and .pdf stay), 5 root .db snapshots
(arest.db, paper.db, tasks.db + 2 pre-migration backups — the LIVING
tasks app is in Repos/apps; these are old-engine relics), testout.txt +
trace.txt. One command set, Samuel's to run or nod at:
  rm -f *.log AREST.aux AREST.out AREST.synctex.gz testout.txt trace.txt
  rm -f arest.db paper.db tasks.db tasks.db.backup-* tasks.db.bak-*
Nothing is git-tracked; no history is touched.

## 2026-07-08 — passHeads LANDS and the ** gate heals a live regression

The staged scheduler-in-canon slice 1, resolved and green the same day:

- THE OPEN QUESTION RESOLVED EMPIRICALLY (the survey settled it, not
  doctrine alone): run_rules' destructive passes gated on fully-derived
  ONLY, but NORMA's ** ("derive materializes into the cell, KEPT IN
  SYNC" — compiler.py:608, engine.py _MATERIALIZE) carries the same
  no-user-assertions license * does. Gating on * only had left every
  non-keyed ** head in NO pass — the tasks board's recommendation
  columns (Task_is_recommended, Task_unblocks_work_in_progress,
  fallback/last-resort) and the claude app's deontic trigger
  (Investigation_should_apply_Reasoning_Practice) were silently DEAD
  since the 0.9.0 swap on 07-06. The ENGINE bent: _OWNED =
  {fully-derived, derived-and-stored} in both hosts, same commit
  (engine.py sweep/dred/agg-whole-replace; main.rs kind_owned at the
  sweep membership and agg gates). +/++ and unmarked ruled heads stay
  out — and that conservatism is LOAD-BEARING: the tasks app's
  Task_Priority_is_* heads are unmarked (kind=None) with
  "highest-among" rule bodies of UNVERIFIED compilation; sweeping them
  could erase the stored ('p0',) row and cascade recommendations to
  empty. Do not mark them ** without first proving their rules compile
  (rule_diagnostics) — filed under decisions below.

- THE SLICE: _classify_heads(D) extracted (engine.py, beside
  layout_cells) — ONE classification: agg / keyed (kind-blind, like the
  pass) / sweep / dred (self-support split, GMS93). run_rules builds
  sweep, sweep_cyclic, and the keyed loop's membership FROM it;
  scheduler_cells(D) materializes it as the passHeads cell ⟨pass,
  head⟩, wired into protocol compile() after layout_cells. Two
  faithfulness bugs in the parked draft fixed during apply: its reach
  included agg-rule reads (run_rules' reach is plain-rules-only) and it
  subtracted agg heads from keyed (run_rules does not — a dual
  agg+keyed head runs in BOTH passes). The old test model was rewritten
  to the documented marker pattern (declaration **; the trailing-**
  -on-rule spelling is NOT parsed — see defects) after the survey
  showed it tripped the subscripted-head slug defect (Task1_has_Cost).

- GATES: test_scheduler_canon 4/4 (classification + asserted-exclusion
  + unmarked-exclusion + the revive behavior pin); focused derivation
  suites 59/59; full chunked python suite 580 collected — chunks 1+3
  green, chunk 2 pending at this writing; cargo 16+1 green including
  the new a_derived_and_stored_head_sweeps_exactly_like_fully_derived;
  READ-ONLY live-store verification: tasks app, 1,351 cells diffed
  before/after full derive under the new gate — exactly ONE cell
  changed, a stale Task_unblocks_work_in_progress row whose support no
  longer exists (correct GMS93 cleaning); cross-host serve-line
  differential on the same live store pending at this writing.

- ALSO CLOSED: the post-rename broad gate's outstanding chunk 2 (bench
  families) completed from the previous session's task queue: 215
  passed, 50:26. Post-rename total 191+170+215 = 576/576 at 25c3130f.

- DEFECT QUEUE ADDITIONS (all pre-existing, surfaced by the survey):
  (a) trailing derivation markers on rule sentences are silently
  ignored ('... iff ... . **' — only DECLARATION markers and LEADING
  rule stars parse); (b) tasks app Task_Priority_is_* + dependency
  blocked/clear heads are unmarked ruled heads (kind None) — dead in
  every pass, stale rows serving as data; needs a Samuel call (mark **
  after verifying their rules compile, or accept staleness); (c) the
  claude app investigation.md swallowed a PARAGRAPH as a unary FT
  (The_surface_trigger_is_EXISTENTIAL_...) — it is fully-derived and
  SWEEPS; add to the 72-prose-id re-authoring pass; (d) the session
  MCP 'arest' server answered orient with the OLD crates' "--features
  local" error — the serving config resolving to the legacy stack from
  the Repos/ working directory needs a look (repo .mcp.json launches
  release/arestlam.exe, rebuilt post-commit with the new gates).

- FOLLOW-ON (unchanged): rust op_run_rules READS passHeads instead of
  recomputing kindmap/keyspans/self_supporting (~80 lines die); the
  verify verb's self-audit (protocol.py:1939, fully-derived-only) joins
  the one-classification discipline in that same slice; then pass
  ORDER/gates as data.

### slice 2 DESIGN NOTE (2026-07-08, while slice 1 gated) — rust reads passHeads

Scoped against main.rs op_run_rules as it stands post-_OWNED:

- WHAT DELETES: the kindmap block, the kind_owned closure, the
  self_supporting walk, and the sweep/sweep_cyclic membership branching
  (~80 lines) — replaced by pop_rows("passHeads") partitioned by pass
  label. The keyed pass iterates the cell's 'keyed' rows.
- WHAT STAYS (pass-BODY inputs, not schedule): plain_of/head_leaf_of
  (rule lists), spans_of/keyspans (key POSITIONS for the keyed upsert),
  reach (dirty-set filtering), agg_rules.
- DESIGN GAP FOUND: the agg whole-replace-vs-per-group decision needs
  the KIND (owned on a full derive -> whole-replace), and the
  membership-only cell does not carry it. Slice 2 therefore extends
  _classify_heads/scheduler_cells with a FIFTH projection — rows
  ⟨aggwhole, head⟩ for derivation-owned agg heads — python and rust in
  the same commit (the cell replaces wholesale; readers of the four
  original labels are unaffected).
- FALLBACK POLICY: none. Deployment step after slice 1 pushes:
  recompile the fleet (seconds-scale post-hoist) so every sidecar/
  loadcache carries passHeads; the resident then reads the cell
  unconditionally (absent cell == no derived heads, true for a store
  without rules). This recompile ALSO heals the tasks board's stored
  recommendation staleness on disk — the deployment step and the
  regression's remedy are the same action.
- GATE: cargo differential + the live-store cross-host diff rerun, and
  the python suite untouched (python keeps computing _classify_heads
  directly; the cell is its projection).

## 2026-07-08 — Samuel's directive batch (debris, legacy dbs, priorities, rename)

Direct message mid-session: "Do debris cleanup, delete legacy dbs if
data is forwarded to new system, fix task priorities, remove arest-cli
and rename arestlam to arest. Use new Rust binary for mcp." Executed:

- DEBRIS: 139 untracked root files deleted (6.4 MB — logs, LaTeX build
  files, 5 old root .db snapshots); AREST.tex/.pdf intact.
- LEGACY DBS: schema-probe classifier (0.9.0 stores carry
  cells(ord,name,contents); the old engine's cells lack ord — the
  'no such column: ord' failure). 136 files / 1,947 MB DELETED off an
  audited manifest (13 .pre-0.9.0.bak of the verified-migrated fleet;
  claude's contaminated-era pair; 121 old-format dbs in dirs with
  readings — rebuildable under 0.9.0 by compile+replay). KEPT and
  reported: arc-agi-3/_corpus/run.db + _offline/spdnav.db (10.7 MB,
  old-format, no readings beside them — no forward path) and 5 tiny
  unreadables. Manifest: job tmp legacy_db_manifest.tsv.
- RENAME (7f513249, pushed): engine/rust arestlam -> arest (package,
  bin, serverInfo, engine_version, CARGO_BIN_EXE in 4 test files,
  python rust_bin, repo .mcp.json). arest-cli REMOVED from crates/
  (bin block + shim main.rs + built exes; the WASM lib stays for the
  legacy tutor/select_component entry). GLOBAL ~/.claude.json 'arest'
  entry now launches the resident (was the retired TS stack — the
  substrate outage's cause; backup .bak-20260707-arest-mcp). Cargo
  17/17 under the new name; release rebuilt + smoked. FOLLOW-UP filed:
  java `package arestlam` + csharp arestlam.csproj identifiers are
  host-internal and keep the old name for now.
- TASK PRIORITIES (the careful one): the dry run CONFIRMED the danger —
  naively marking the Priority heads ** wiped recommendations 28/26/7
  -> 0/0/0 (their 'highest ... among' enum-superlative is NOT in the
  0.9.0 aggregate grammar; rules evaluated empty and the sweep erased
  the stored rows; Task_is_dependency_clear's 'every ... has' universal
  likewise derives empty — 1,060 rows would die; it STAYS unmarked,
  agent-side per the model's own CSDP comments). THE FIX, rehearsed on
  a scratch app copy before touching the live board: each tier reads
  through a fully-derived carrier FT (Task carries <status> Task
  Priority) and takes MIN — highest priority IS the lexical minimum
  over the closed enum p0..p3 (min|max|count|sum exist in the 0.9.0
  grammar; min classifies the Priority heads into the AGG pass).
  Rehearsal: 0 diagnostics; ('p0',) per tier; the chain reproduces the
  live board EXACTLY (28/26/7). Landed on the live model + recompiled.
  Task_is_dependency_blocked marked ** (dry-run-proven 9/9 rederive).
- GPG LESSON: commit signing hung twice (agent cache ~10-min TTL;
  pinentry can't prompt here). The fix: probe-sign-then-commit in one
  command — a successful probe both proves and RESETS the warm cache.
- NEW prose-slug sighting for the re-authoring queue: the tasks app
  carries a NOTE_asserting_a_clean_started_... unary FT (from
  readings/instances/) that is fully-derived and SWEEPS.

### system:classify_heads DESIGN (2026-07-08, per Samuel's doctrine directive)

"All functionality available in a performant override must be defined in
the shared lambda base." The scheduler classification therefore lands in
shared/system.canon as the def of record; python _classify_heads becomes
the certified-equal override (twin test evaluates the def through the
reducer against the python function over the corpus models — same
discipline as reading_parse/clause_ft); the passHeads cell stays the
compile-time materialization; rust reads the cell (unchanged plan).

Decomposition (mirroring the partition def family's shape; defs are PURE
— the host binding fetches the pops and applies, like sm_join over its
three pops). Input: ⟨ruleAgg, ruleDerives, derivation, spans,
constraint, ruleReads⟩ (six fetched pops). Sub-defs:
  classify:aggids     ruleAgg column 1 (the agg rule ids)
  classify:agg        heads of ruleDerives whose rid ∈ aggids
  classify:plain      ⟨head, rid⟩ pairs, rid ∉ aggids
  classify:owned      derivation rows with kind ∈ {fully-derived,
                      derived-and-stored} (theta:Filter over K set)
  classify:keyspanned constraint (uniqueness|spanning_uniqueness) ⋈
                      spans (nonempty) → fact types
  classify:reach      plain ⋈ ruleReads → ⟨head, read-ft⟩ edges
  classify:selfsup    the WHILE walk (rmap_top's shape): from each
                      owned candidate head, close reach over derived
                      heads; self-supporting iff the head reappears
  classify:heads      fold to ⟨pass, head⟩ rows: agg / keyed (plain ∩
                      keyspanned, kind-blind) / sweep (owned acyclic) /
                      dred (owned self-supporting) / aggwhole (agg ∩
                      owned-kind)
Ordering: rows sorted per pass (the cell is canonical bytes).
The twin test corpus: the scheduler test model + the fleet models
(tasks/claude/kernel compile → def(pops) == _classify_heads(D) row-set
equality per pass).

## 2026-07-08 — SCHEDULER-IN-CANON COMPLETE (both slices + the canon def)

Three commits close the arc the morning opened:

- 943a2b47 system:classify_heads — THE CLASSIFICATION IS CANON. Samuel's
  doctrine directive ("all functionality available in a performant
  override must be defined in the shared lambda base") executed: the
  cls_* def family in shared/system.canon — projections over the six
  M-pops, the generic key-match pairjoin (rmap_top's constructed-filter
  idiom), cls_closure (a WHILE fixpoint over reach edges), cls_selfsup,
  and the five-label assembly. python _classify_heads is the
  certified-equal override, twinned by tests/test_classify_canon.py.
  The aggwhole FIFTH LABEL rode along (the agg whole-replace license as
  data); kindmap left run_rules entirely. Authoring pattern proven:
  candidate-canon scratch + per-sub-def probe runner (seconds-scale
  feedback); the full assembly greened on its FIRST run.
- 8bce65fe the resident READS the schedule — op_run_rules consumes
  passHeads (sweep/dred lists, keyed membership, aggwhole gate) exactly
  as it reads rmapColumns; kindmap/kind_owned/self_supporting/
  derived_heads/agg_heads and the membership branching DELETED from
  main.rs. Absent cell = positive closure only (rmapColumns' posture).
  Hand stores in cargo tests carry schedule rows now; the python
  differential fixtures mint the cell via scheduler_cells. New pin: a
  store with NO derivation rows sweeps because the cell says so.
- 54085473 the peer kernels take the name too (java package + csproj).

Also this stretch: dependency_clear LIVE on the board (12 honest rows;
the carrier ring + at-most-0 rewrite, probe-first; the 1,060 stale rows
were beyond even the old rule's semantics); gpg lesson RE-CONFIRMED
(probe-sign-then-commit in one command; each successful sign resets the
agent TTL); the tasks store briefly ran AHEAD of the pushed engine
(a live recompile picked up uncommitted aggwhole python — benign, now
converged).

GATES at close: cargo 18/18; python differential 3/3 cross-host
per-head equality; twin 2/2; scheduler 5/5; polyglot + both peer
kernels over the grown canon; full 3-chunk python suite RERUNNING as
the day's final gate after the fleet recompile (which lifts every
sidecar to the 5-label cell — kernel's 25 agg heads need aggwhole for
whole-replace parity under the read-the-cell resident).

NEXT (bottom-up): pass ORDER and the gates as data (the 12-round bound
as a datum; the scheduler-as-data endgame), then pipeline-as-data (L4);
the verify verb's self-audit joins the one-classification discipline;
tutor + select_component port off the legacy WASM entry.

### pass ORDER as data — DESIGN (2026-07-08, next slice, pre-staged)

The ledger's three questions answered against the landed passHeads:
(1) M-facts vs DEFS-level: CELL, like passHeads/rmapColumns — layout
knowledge about the store rides IN the store. (2) host speed: pass
BODIES stay native executors; the hosts' joint loop DISPATCHES by the
cell's order (name -> body table; unknown names skip, forward-compat).
(3) the 12-round bound: yes, a datum.

THIN SLICE: scheduler_cells grows two cells —
  passOrder  rows ⟨ord, pass⟩ = 1 agg, 2 keyed, 3 sweep, 4 dred
             (from a canonical CONSTANT def system:pass_order; the
             order is doctrine today, dependency-derived someday)
  passBound  one row ⟨12⟩ — the joint loop's round bound
Both hosts read them (python run_rules + rust op_run_rules), defaults
(the current literals) when absent. The dirty-set GATES stay native and
uniform — no value in data-fying the touched-policy yet (YAGNI, noted).
The positive closure stays OUTSIDE the ordered passes (it is the floor
the strata stand on, not a stratum).

Sequencing: AFTER the deployment tail closes (fleet 5-label + release
binary + full gate) — do not stack on an un-deployed slice.

### DEPLOYMENT CLOSED (2026-07-08, foreground workaround) + 1e521a47

Background jobs in this session died repeatedly (long AND short) — the
tail finished FOREGROUND per-call instead: fleet recompiled app-by-app
(all stores 5-label), release arest.exe rebuilt (12:05, 6m06s), and the
serving smoke passed end-to-end over the LIVE board (mcp initialize ->
serverInfo 'arest'; apps_use tasks; Task_is_recommended 28;
Task_is_dependency_clear 12). Final gate chunks 1+3 foreground 388/388;
chunk 2's heavy members covered by the day's focused green runs. ALSO:
1e521a47 verify joins the one-classification discipline (self-audit
head set = the schedule's sweep/dred/aggwhole + owned-keyed corner).
OPERATING NOTE for the substrate: background-jobs-die-here —
foreground, under 10 minutes per call, chunked.

### pass ORDER + bound as data — GREEN, STAGED, AWAITING SIGNATURE (2026-07-08)

The slice is complete in both hosts and fully gated (scheduler 6/6 incl.
the order/bound pin; twin 2/2; differential 3/3; kernels + canon-native
over the grown canon; cargo 18/18): system:pass_order / system:pass_bound
constant defs; scheduler_cells materializes passOrder ⟨ord, pass⟩ +
passBound ⟨12⟩; python run_rules dispatches its pass bodies by the
evaluated constants; rust op_run_rules reads the cells with the doctrine
literals as fallback. COMMIT PARKED: the gpg agent is cold and pinentry
times out unanswered (Samuel away). The files sit STAGED; a later tick
probes and commits the moment the agent warms. Fleet/binary refresh
follows the push (the current serving state stays coherent meanwhile —
the live binary ignores the new cells).

### tutor port RECON (2026-07-08, read-only — the next big arc, sized)

The legacy tutor is: a SANDBOX engine handle bootstrapped from
tutor/domains/ readings + lessons as md files (tutor/lessons/{easy,
medium,hard}, expect-predicate grammar in _format.md) + tutor.* verbs
routing to the sandbox so learners never disturb the active app.

UNDER THE NEW ENGINE the sandbox is JUST AN APP: a registry-managed
_tutor app whose readings are tutor/domains/ (reset == recompile; the
stream/db machinery comes free). The port decomposes:
  1. sandbox app registration (+ gitignored db) — small.
  2. tutor_apply/query/propose/compile/actions == the EXISTING
     first-class verbs scoped app='_tutor' — verb-table entries only.
  3. tutor_get/list + expect-predicate evaluation — the lesson parser
     ports TS -> python (~200 lines vs src/mcp: lesson load, step
     grammar, predicate checks against the sandbox store).
  4. tutor_reset == registry compile of _tutor.
Estimated one focused session. select_component is SEPARATE (it reads
the Component registry via an old-lib system intercept; needs its own
recon into where the component readings live under 0.9.0).
After both: the arest-legacy MCP entry deletes; then the old-rust
removal plan (the 65-Dependabot worker decision) unblocks.

### select_component RECON addendum (2026-07-08, read-only)

The Component registry is NOT in the 0.9.0 base: it lives in the OLD
repo's readings/ui/ family (components.md + render-target-instances +
view-* etc.), consumed by old-lib #492 selection rules re-implemented
in old-rust for latency. The 0.9.0 port therefore has two halves:
  1. ingest readings/ui/ as an app (or into the base) under the new
     compiler — the registry becomes ordinary fact populations;
  2. the selection scoring (#492: intent-substring on Component Role +
     constraint filters + preference weights) as a Registry verb over
     those populations (python first; the doctrine's canon def when the
     scoring stabilizes).
Prerequisite check before porting: does readings/ui/ COMPILE under the
0.9.0 grammar (same drill as the tasks-app rules — expect old-grammar
spellings to surface). Sequenced after the tutor port; both retire the
arest-legacy entry together.

### legacy-corpora compile probe (2026-07-08, read-only) — the port sizing hardens

- tutor/domains: 8 files compile ESSENTIALLY CLEAN under 0.9.0 — zero
  unparsed, ONE diagnostic (Order_has_Amount aggregate-clause spelling;
  the same respell drill as the tasks Priority rules). THE TUTOR PORT
  IS UNBLOCKED: sandbox-as-app needs one rule respelled, the rest is
  the lesson parser + verb table.
- readings/ui: 13 unparsed (doc prose swallowed — the familiar
  prose-slug family) + 12 diagnostics dominated by ONE missing grammar
  form: the E-parenthesised EXISTENTIAL/SKOLEM HEAD (skolem-head-
  design.md's `ViewElement(E) has ...` — "head variable(s)
  ['ViewElement'] unbound"). select_component is therefore BLOCKED on
  a design call: port the skolem-head syntax into the 0.9.0 compiler
  (a grammar feature, primary source = skolem-head-design.md) or
  respell the ui rules without it. Samuel's call — grammar additions
  are doctrine.

### skolem-head ASSESSMENT (2026-07-08, from the primary source, task-970)

readings/ui/skolem-head-design.md is FINISHED DOCTRINE, not an open
question: existential (TGD) heads with semi-oblivious/Skolem-chase
value invention — E.id = fnv1a64(frontier values), deterministic, so
same frontier -> same id -> set-dedup makes re-derivation idempotent.
Two conclusions for 0.9.0:
  1. RESPELLING IS NOT AN OPTION: fresh-entity invention (one
     ViewElement per (View, Transition) binding) is a genuine
     capability, not a spelling — the ui family cannot exist without
     it. The design call is WHEN, not whether.
  2. THE DESIGN MESHES WITH THE LANDED SCHEDULER: Skolem idempotence
     ("lazy re-derivation safe and stable") is EXACTLY the sweep's
     delete-and-rederive requirement — a skolem head is a
     derivation-owned sweep head whose ids are frontier-stable. The
     0.9.0 slice: (a) parser surface Noun(Var) per §5, (b) a skolem
     prim (fnv1a64 over frontier values — polyglot: canon def + host
     twins), (c) rule-compile emission binding head-only vars,
     (d) idempotence pins. Sized 1-2 sessions. Unblocks the ui family,
     select_component, and the 934 view-projection arc above it.

### tutor port COMPLETE python-side (2026-07-08, awaiting signature to land)

All ten legacy tutor tools ride the one verb table: sandbox-as-app
(_tutor: reset == wipe learner state + copy tutor/domains + recompile —
668 facts from the real corpus), the lesson reader (fences + the
_format.md expect grammar; the corpus's 'E1' heading form accepted over
the spec's 'easy.1' — corpus wins), the four-form expect checker
(value-based contains/equals), and tutor_authoring (the 5-FT positional
join, ordered). Gates: tutor 6/6 hermetic + real-corpus integration
(18 lessons, checks evaluate); advertise==dispatch green at 10 new
descriptors; mcp + verb-parity suites green. HELD: the rust resident's
tool-table advertisement (main.rs is in the STAGED pass-order changeset
— no mixing); it follows the first commit. orders.md's sum respelled
through a carrier (domains now compile 0.9.0-clean).

### skolem-head 0.9.0 MAPPING (2026-07-08, from the full task-970 design)

The old-engine anatomy translates: (1) Platform("skolem") -> a BASE op
at the lex-boundary precedent (fnv1a64 over the frontier values is a
tiny pure byte transducer; four host twins + case-table pins — the
hash-impl-in-base sibling of regex-impl-in-defs-ok), NOT combinator
arithmetic in canon. (2) The §5 surface: a parenthesised head variable
otherwise unbound is existential; frontier = antecedent-bound roles
co-occurring in the same consequent FT, declaration order; the
multi-consequent shared-E case splits into per-FT rules carrying the
SAME frontier (deterministic hash => the shared id matches for free).
(3) LAZINESS RE-EVALUATED: the old engine had to be lazy (the 593-FT
metamodel join hang); under 0.9.0 a skolem head is an ordinary OWNED
sweep head — eager delete-and-rederive IS the semi-oblivious chase
step, idempotent by frontier-determinism + set semantics, over stores
the vocab-hoist made seconds-scale. Start eager; a lazy view mode only
if ui-scale derives measure poorly (its own slice if ever).
Slice plan (TDD, compiler.py + kernels — DISJOINT from both parked
changesets): a) skolem prim x4 hosts + case rows; b) compiler head
resolution appends ⟨role, skolem(frontier)⟩ into the rule projection;
c) the landed scheduler does the rest (owned sweep); d) pins:
determinism, 2-bindings->2-fresh-entities, re-derive byte-identical,
cross-host rows.

## 2026-07-08 — THE TRAIN LANDS: the agent warmed and three commits pushed

The gpg agent finally answered (Samuel touched it); the probe caught the
warm window and the parked queue FLUSHED in one sequence (each sign
resets the TTL — the probe-then-commit discipline, now proven at train
scale): 5e5f7207 pass ORDER + bound as data (both hosts dispatch by the
schedule); f24addf9 the tutor rides the new engine (ten verbs, sandbox-
as-app, the legacy entry's first retirement half); d949c1fd the skolem
boundary op in ALL FOUR kernels + case rows (ve_4c85ed03a10dd979
byte-identical everywhere). Deployment refresh running foreground:
fleet recompiled through claude (tasks re-running); release rebuild +
smoke next; then the artifact's parked flags lift.

REMAINING skolem half: the compiler surface — the unbound-subscripted-
head-variable site is compiler.py:1496-1497 (the exact diagnostic the
ui probe tripped); the slice turns that diagnostic into the skolem
binding emission (frontier = body-bound vars, first-appearance order,
minimal increment) + the 2-bindings idempotence pin.

### STAGE B CLOSED — Samuel accepts the wrap (2026-07-08) + the doctrine stated

"Accept wrap. The intent is no duplicated code, not even as a generated
artifact. Only text frames around the source file, which must run as
source in that language and not as IL per se."

The operating rule polyglot-same-bytes-native-frontends gains its
precise criterion: hosts may add TEXT FRAMES around the shared source;
the shared bytes must be consumed AS SOURCE by each language's own
front-end (CPython exec / rustc include! / MSBuild ReadAllText / javac
over the regenerated wrap) — never precompiled to an intermediate
representation and shipped. A generated frame is acceptable when it is
gitignored transient derived state and the framed bytes are verbatim;
the SOURCE of truth stays singular. gen_canon.py stays as java's
include! shim; store-boot loaders return with JVM/CLR mini-residents.
Every decision on the board is now CLOSED.

### the dispatch bug postmortem line (2b20b62f, pushed)

The serving binary between 14:29 and the fix dispatched ZERO passes on
any derive over a compiled store (quoted-vs-bare pass-name mismatch) —
window ~2h; python-side compiles unaffected (python evaluates the
constants directly); resident-native applies in the window would have
committed WITHOUT derived-head maintenance — healed by any later derive
(idempotent). The catch credits the fixture-mints-the-cell discipline:
hand stores ride fallbacks and can never see cell-parse bugs.

## 2026-07-08 — UI REDIRECT (Samuel): binding, not entity materialization

Samuel, on the skolem-ui dependency: "The whitepaper clearly says to
use binding for UI, and we can use the open-source, abstract screen
designs from MonoView/iFactr for our rendering. The control
implementations can be registered in DEFS the same way that iFactr
pulls its cross-platform stunts." VERIFIED against the primary source —
AREST.tex §Platform binding (line 209): "Binding a user interface is
then registering a render function, so a fact renders itself." The old
readings/ui view-*.md design (ViewElements as DERIVED FACT POPULATIONS
minted by existential rules, task-970/934) contradicted the paper's own
doctrine and is SUPERSEDED: those files re-author to binding, they do
not port.

CONSEQUENCES: (1) select_component decouples from the skolem surface
entirely — its real shape is the Component REGISTRY as ordinary facts
(components.md compiles standalone: 0 diagnostics, 18 FTs, the full
vocabulary incl. Component_has_ImplementationBinding_for_some_Toolkit —
the iFactr registration point) + a scoring VERB on the one table.
(2) The skolem capability STAYS (four-host certified, one compiler
branch, general TGD machinery; the re-probe showed it clears the old
corpus 12 diags -> 0) but leaves the critical path; its parser
sub-task and the ui-rule port are CANCELLED. (3) The rendering layer's
design of record: MonoView/iFactr abstract screens + per-target render
functions in DEFS, per the paper.

One components.md nit for the port: "No two Components share the same
Name." is an old-grammar uniqueness spelling (1 unparsed) — respell as
the at-most-one form at ingest.

## 2026-07-08 — ARest-LEGACY RETIRES (208535b6)

The old engine serves NOTHING. select_component landed as
registry-facts + a scoring verb (9b1a53c2, the UI redirect executed:
binding doctrine per AREST.tex §Platform binding; the real registry
ranks gtk4 GtkNotebook over qt6 QTabBar for 'tab' + dark_mode_native —
per-toolkit trait discrimination, the iFactr pattern as facts). The
existential-head capability committed separately (18b062ca) and stays
banked off the critical path. The resident's tool table grew the
eleven delegated descriptors and the arest-legacy entry DELETED from
.mcp.json. The release-binary refresh is DEFERRED ON LOCK: a live
resident holds arest.exe (someone is serving — good); until its next
restart the new verbs ride the python binding only. UNBLOCKED NEXT:
the old-rust/old-shell removal (crates/, src/mcp, node dependency
surface incl. the 65 Dependabot findings) — its own arc, Samuel-visible.

Day tally: FOURTEEN commits pushed (73d5d58f..208535b6).

### the synthesize plumb PRICED FOR REAL (2026-07-08 night)

The one-arm carrier swap ((system:verbalize : id) : D through NEval,
NCANON resolving the def) was TRIED and the serve-op pins caught it
answering EMPTY — the pins doing exactly their job. Diagnosis: NEval's
native op coverage is complete for compiled RULE objects (the
run_rules family) but NOT for the verbalize def-family; some op in its
chain bottoms and the answer collapses. The Scott arm is restored
(pins green again) with the finding in the arm's comment. THE PLUMB'S
REAL SCOPE: instrument NEval's Bot fallthrough (a debug counter naming
the op that missed), enumerate the gap over a verbalize evaluation,
port the missing ops with case-table rows, THEN swap the arm — a
focused session, the L2 carrier chip's true price. The 40x stays
priced, not free.

## 2026-07-08 — THE OLD RUST IS GONE (d3104058 + 9e71c1f0)

Samuel's go executed: 5,937 files, 771,079 LINES deleted (crates/ —
the old engine, foundation, kernel; src/mcp — the retired TS server;
the kernel-boot scripts, test-all.ps1, the old wasm plumbing), CI
rewritten to the 0.9.0 engine (python fast leg + engine/rust cargo
leg), and ~19 GB of build state cleared from disk (on top of the
morning's 1.95 GB legacy-db sweep). The WASM path continuity is IN the
commit message per Samuel's check ('that's the path to JS engine
compatibility'): the successor build rides engine/rust (zero-dep by
design — wasm32 + a thin shim over the serve protocol), the root arc's
next-but-one rung. The worker surface stays FROZEN in-tree (deployed
instance unaffected; source can't rebuild wasm by design); its deletion
is the named follow-on. The carrier-gap instrumentation rode along
(9e71c1f0): tracers + the A/B toggle, with the silent-semantic-
divergence finding filed — the bisection is the next focused session.

Session tally: SIXTEEN commits, 73d5d58f..9e71c1f0.

### the carrier gap CORNERED (2026-07-08 night, neval probes)

The debug op landed ({"op":"neval"} — gated on AREST_NEVAL_TRACE; the
cases mechanism rides Scott, this is its native counterpart) and the
bottom-up bisection cornered the silent-semantic divergence in TWO
probe rounds: distl/eq/filter_eq/member/vb_pred ALL natively correct;
the BUILT filter over rows correct ([["p1","Ada"]]); vb_matched over
⟨ftrow, D⟩ answers [] — THE FETCH SIDE. system:vb_fetch (RMAP-aware:
rmapColumns dispatch, own-table -> ast:FetchPop) or FetchPop itself
answers EMPTY over the carrier's D. Fix candidates are narrow and of
the passOrder bug's class: the native CELL-tag match or the cell-name
comparison in the walk. Next probe: vb_fetch and raw FetchPop over
⟨"Person_has_Name", DEFS⟩ — one answer names the line.

### the 40x lever's TRUE story (2026-07-08 night, the bisection closes)

The final probe named it: length(D) natively answered ZERO — the
carrier's ops were COMPLETE ALL ALONG; the empty-pairs mystery was my
new arm trusting srv.nd, which op_run_rules' own comment warns "may be
stale: a prior run_rules replaced srv.d without refreshing the mirror."
The fix is the established idiom (build the native view FRESH from d:
v_to_n + n_cells_of), applied to both the neval debug op and the
synthesize arm. Post-fix the native path answers BYTE-EQUAL pairs on
the pinned store, and the full cargo suites pass with the toggle both
off AND on (5+5). The tool-side flip landed under the toggle: MCP
synthesize routes op_answer(synthesize_pairs) natively instead of
delegate_read. The release A/B timing on the real tasks store decides
the default flip. LESSON (an operating-rule candidate): when a native
mirror exists beside a source of truth, READ THE CONSUMER'S IDIOM
FIRST — op_run_rules knew; the new arm didn't ask.

### ui re-authoring DESIGN (2026-07-08 late, from the primary sources)

view-projection-design.md's THESIS is already the binding doctrine:
"the whole UI is rho-applied over P: view(entity) = project(P, entity)"
— Theorem 4 (HATEOAS as projection) extended from actions to whole
views. Only its §4 MECHANISM (the projection AS derivations,
materialized ViewElements — the skolem path) strayed from AREST.tex
§Platform binding; the thesis never required materialization.

THE RE-AUTHORING (design of record, per Samuel's redirect):
- KEEP §0-3 + §4.2-4.4 verbatim in spirit: the MT.D conventions, the
  target vocabularies, the list/detail/menu mapping table, value-type
  -> Component Role modeling, constraint -> validation, captions —
  the CONTENT of the projection.
- REPLACE §4.1 (stored view facts): the projections become CANONICAL
  DEFS — system:view_list / view_detail / view_menu, pure functions
  over ⟨noun-or-id, D⟩ answering the ABSTRACT ELEMENT TREE as a VALUE
  (the response), never stored facts. The menu def IS the existing
  actions/transitions fold (Theorem 4 verbatim, already serving).
  Component selection inside the tree = select_component's scoring
  over the registry. RENDER = per-target functions registered in DEFS
  (the iFactr registration; browser/slint/html each register render;
  "a fact renders itself" — AREST.tex line 209).
- No fresh entities anywhere: the skolem need evaporates exactly as
  Samuel said; the capability stays banked for genuine TGD arcs.
SLICE SHAPE when it starts: system:view_menu first (it wraps the
proven actions fold), then view_detail (the §3.2 table over get's
view), then view_list; each a canon def + twin test + one registered
reference render (the html Render_Target already modeled in the
registry app).

### DOCTRINE RESTATED (Samuel, 2026-07-08 night) — and a correction taken

"Remember, the shared canon is the primary target, and each DEFS
override is only done for performance." Applied immediately to
07e1c915 (render:html): the render's MEANING (tree -> element
structure, the joins) was registered host-side under a D5-boundary
justification — too much. The conforming shape, in flight:
system:render_html in the shared canon (structure + implode joins);
ONLY the escape transducer stays a boundary op (byte-level, lex
family, four-host twins + case rows); the python function demotes to
a certified-equal override twinned by test (the _classify_heads
discipline). The rule's sharpest form for the operating ledger:
MEANING IN CANON, BOUNDARY FOR TRANSDUCTION ONLY, OVERRIDES FOR SPEED
ONLY — and every override twinned.

### system:entity_view DESIGN (2026-07-08 night — get's doctrine gate)

get is the store-only family's last verb and it WAITS, correctly: its
crux (ddl._analyze + _entity_columns — the RMAP column classification:
unary booleans vs absorbed functional values vs the played-type field
naming) is HOST MEANING not yet in canon, and porting it to rust would
duplicate meaning host-side — the exact violation of the night's
restated doctrine. THE SLICE: system:entity_view over ⟨noun, id, D⟩
answering the 3NF view ⟨exists, fields, facts⟩ — composed from
system:partition (already canon) + a cls_*-style def family for the
column classification (role walks over the role/refScheme/instanceOf
pops; the same authoring drill as classify_heads, probe-runner and
all) + vb_fetch for the reads. Python's Registry.get demotes to the
certified-equal override; the rust arm then evaluates the canon def on
the carrier like actions does. Sized: one focused session (the
classify_heads authoring took one evening with the harness; this
family is smaller).

## THE DEPLOYMENT NORTH STAR (Samuel, 2026-07-08 night)

"The plan eventually is to deploy support on Cloudflare as a react
site." Decoded into the ladder: support.auto.dev (migrated, lean — the
400x story) serves from Cloudflare with the ENGINE AS A WASM32 WORKER
(bindgen; a fetch handler speaking the one verb table as REST) and a
REACT frontend. CONSEQUENCES: (1) the L5 REST+HATEOAS chip joins the
critical path — a react site consumes HTTP; Theorem 4's _links
projection is the client's walk; (2) the render target of record is
REACT: the Component registry gains toolkit='react' (symbols =
component names) and the view trees' Component-Role vocabulary maps
onto a react component library consuming the trees — the iFactr
pattern, react the first real citizen; the html renderer stays the
spec's executable example; (3) get/system:entity_view are ON this
path (detail pages ARE the 3NF view); (4) the frozen worker retires
when this successor replaces the deployment. LADDER (bottom-up,
unchanged discipline, new destination): entity_view canon -> get
native -> REST surface on the resident -> wasm32 worker -> the react
target + the support site.
(addendum: bottom-up preferred, nothing hacked up for the shiny — Samuel)

### the tasks-scale smoke names TWO defects in native actions (2026-07-08)

The support.auto.dev smoke is emphatic (ingest 0.15s; native schema
13ms over 105 types / 228 FTs) — the small-store path is right. The
TASKS-scale smoke is the teacher: (1) actions took 247s — the
build-fresh-from-d idiom is per-CALL there, converting the whole 5MB
store to the carrier on every read; op_run_rules pays it once per
derive, reads must not pay it per call. THE FIX is mirror COHERENCE,
not per-call rebuilds: refresh srv.nd at the store-replacement sites
(wherever srv.d is written: retain, run_rules' store_into, apps_use)
and TRUST it in reads — the stale-mirror comment then retires with the
staleness. (2) status answered None on a real task (machine found,
transitions empty) — a correctness gap in the vb_fetch leg at tasks
shape (absorbed column? row shape? the smStatusFt mapping?) — the
neval probe drill localizes it next. The fixture pin passed because
its shape is simple; scale is the second gate, again.

### the status gap DIAGNOSED — one root for both defects (2026-07-08)

The probe drill over the tasks serve-line store: smStatusFt maps Task
correctly, rmapColumns shows the absorbed layout, and native vb_fetch
answers 1,072 STATUS ROWS — data and defs perfect in --serve. So the
--mcp leg's status=None isolates to srv.d: the LOADCACHE ingest
retains its truth in srv.cells while srv.d rides thin, and the native
reads (actions' vb_fetch leg, native_verbalize) build the carrier
FROM srv.d — empty in, empty out. schema worked because it reads
srv.cells. THE ONE FIX for both defects (the 247s per-call rebuild AND
the empty reads): maintain srv.nd + srv.ncells AT the ingestion and
write sites from each path's actual truth (set_store's d; the
loadcache's cells; store_into already keeps lockstep during derives),
and READS TRUST THE MIRROR — the stale-mirror comment retires with the
staleness instead of being worked around per call. The write-site
audit is the implementation: apps_use/load_sidecar, set_store/retain,
and the delegate-reload path.

### mirror coherence LANDED; the thin-d hypothesis DISPROVEN (2026-07-08)

The write-site audit implemented: retain (~1925) and the native-apply
reload (~4040) now refresh srv.nd/srv.ncells beside d/cells (set_store
and run_rules' exit already did), and ALL native reads trust the
mirror — native_verbalize, the actions arm, the neval probe, and
op_run_rules' seed each dropped their per-call v_to_n(&srv.d). Suite
green (18/18), release rebuilt.

CORRECTION to yesterday's diagnosis: srv.d was NEVER thin. apps_use
rides load_sidecar → handle({"d":...}) → the full-refresh site, and
the tasks sidecar on disk is RICH (cell Task_is_currently_in_Status
carries the full population, ~30KB of rows; d spine 1405 cells).
There is no loadcache-to-cells-only ingest path in the resident —
every d/cells write site was inventoried and all four move together.
So the mirror fix retires the 247s rebuild for CERTAIN, but the
status=None cause is NOT thin-d; the re-smoke over the real apps dir
tells whether it reproduces at all (the arm's vb_fetch pair is
selector-shaped per the canon: ⟨ft, D⟩ as a 2-seq, N(1)/N(2) — same
convention the probe proved).

### the binding doctrine is BIDIRECTIONAL (Samuel, 2026-07-08)

"Fact binding over react events should also be possible once it gets
to that level." The react slice binds BOTH directions: facts render to
elements (view_menu's buttons carry ⟨event, to⟩), and ELEMENT EVENTS
TRANSDUCE BACK TO FACTS — a button's onClick is apply of the
transition fact the button already names, not an ad-hoc handler. The
round trip is the existing verb table: render answers the tree, the
event fires apply(app, ft, row), the derive re-renders. No JS-side
state, no controller layer — the store is the state and the event is
a fact. This shapes the react component layer's contract when
bottom-up reaches it: components receive ⟨element tree, apply
endpoint⟩ and nothing else.

### vb_fetch NATIVE (30,000x) and the status gap's TRUE root (2026-07-08)

Two more layers under the mirror fix. (1) The interpretive absorbed
reassembly was the real cost: canonical system:vb_fetch evaluates one
ast:DynFetch per entity id — measured 301 s PER FACT TYPE over the
tasks store, so one synthesize ≈ 16 hours. The resident now carries
its first canon-NAMED native prim: "system:vb_fetch" in NEval's prim
table (cells still shadow it; prim wins over process/canon — the
same order defs resolve). One spine pass, keyed map, every sentinel
mirrored: FetchPop's missing-or-"#" → empty population, DynFetch's
missing/atom wide row → "#" → filtered, short wide row → ⊥ (α
strictness). Twinned by the_native_vb_fetch_twins_the_canonical_def
(tests/derive.rs): Scott (cases) evaluates the canon def as the
meaning, neval hits the prim, byte-equal on absorbed + pad + missing
+ own-table + unknown. vb_fetch 301s→0.01s; verbalize >300s→1.4s;
actions 247s→0.00s end-to-end via MCP.

(2) status=None was never the engine: the tasks app's LIVE DATA
carries a phantom task with id φ (an empty id leaked through some
past write; its only fact Task_is_started, its wide row Task:φ is 22
cols — written before Task_is_currently_in_Status grew the table to
col 23 and missed by the re-pad). Selector 23 over a 22-seq is ⊥ and
α strictness correctly bottoms EVERY absorbed fetch of the Task
table — canon and the native twin agree; the reader is RIGHT to
refuse. The serve export lacked φ (1,072 vs 1,073 spine rows), which
is why every probe drill worked while every MCP read answered empty.
Repair = retract the phantom through the engine's own write path.
CRUMBS: (a) verify should audit ragged wide rows against
rmapColumns width — a named defect instead of a silent ⊥; (b) the
write side should refuse φ/empty ids at apply; (c) the re-pad sweep
that grows entity tables missed a row whose only fact is a unary
marker — find and fix that enumeration.

### entity_view slice: naming + classification in SHARED CANON (2026-07-08)

Samuel's correction mid-flight: "Not 4 separate implementations. The
shared lambda source." The sqlcol boundary op (already written 4x)
was REVERTED; the naming now lives as canon defs — system:sqlname
(slug → single-token lex row's lower field → "t" fallback) and
system:sqlcol/_base (unary strips the noun via the ONE new generic
base op strip_prefix; ref implode-joins player_mode; value names the
played type; the dedup ordinal suffixes _n from 2 — canon counts, the
boundary concatenates). strip_prefix is the only 4-host addition:
⟨prefix, s⟩ → tail-or-s, policy-free. Case rows certify system:sqlcol
across kernels (C#/java gates green; C# needed a pattern-variable
rename). protocol._sql_name is hereby the twin, not the definition —
test_entity_view pins them byte-equal incl. φ and empty.

The classification family landed the same way, ALL CANON: ev_colrows
(rmapColumns for the noun, layout order), ev_roles, ev_entities
(instanceOf ObjectType — the python conjunction 'other in entities AND
other in entity_tables' PROVABLY reduces to the entity test, entities
⊆ entity_tables by construction), ev_others, ev_kind (unary/value/
ref + played type), ev_refrows/ev_refmode (refScheme over refMode
over "id"), then the WHILE-fold: ev_step_kd/ev_classified/ev_name/
ev_mode/ev_base/ev_count/ev_item/ev_step/ev_cols — classified columns
⟨ft, kind, other, col⟩ with seen-count dedup, 12/12 python pins.

NEXT (batch 3, designed): ev_fields ⟨noun,id,D⟩ = α over ev_cols —
key = unary→col else other-or-col; value from FetchPop(ft) OWN CELL
(get reads _pop_rows, NOT the wide column): unary → member T/F with
the len-guard COND (python skips short rows silently — predicate
COND(ge(length,k), eq, F) mirrors that laziness), binary → last-wins
(1r) or "#". ev_facts = own fts (factType minus rmapColumns' α3) ×
noun-positions from role rows × row[p]==id guarded. entity_view →
⟨exists, fields, facts⟩; exists = field-seen ∨ facts nonempty ∨ spine
member. Python Registry.get then demotes to certified-equal override
(twin must align fact ORDER: canon = factType cell order, python =
partition dict order — verify or sort); rust get goes native by
evaluating system:entity_view over the carrier and rendering get's
JSON shape (the actions-arm pattern). ALSO: tasks app modeling crumb —
smTrigger cell MISSING (9 smFrom/smTo rows, no triggers), so actions
answers zero transitions for every status, python and native agreeing;
the readings never declare events.

### entity_view COMPLETE in canon (2026-07-08, batch 3)

system:ev_fields (get's key/value semantics: unary key = sql column,
binary key = played type or col; value from the ft's OWN CELL —
FetchPop, never the wide column; unary → T/F membership, binary →
last-wins (1r) or "#"; the len-guard COND mirrors python's silent
short-row skip), system:ev_facts (own fts = factType minus
rmapColumns' α3; per ft the noun's role POSITIONS, each row kept when
ANY position matches the id — dynamic selectors apply(p,row) under
ge-guards; flattened by INSERT(cat) ∘ append_phi), and
system:entity_view = ⟨exists, fields, facts⟩ with exists = any column
SEEN (kind-aware: unary needs T, binary needs key-present — value
encoding alone cannot tell a unary False from a binary "F") ∨ facts
nonempty ∨ spine membership. 16/16 python pins.

Two authoring bugs caught by the bisect discipline (evaluate every
subterm before the assembly): a selector over-hop (2∘2∘2 where the
context is two deep — ev_posmatch's three-deep spelling copied into
ev_ftfacts' meta), and apply(⟨ALPHA, fn⟩) where the ftpop_absorbed
pattern BUILDS ⟨ALPHA, fn⟩ as a value for the OUTER apply — applying
ALPHA to fn is ⊥, building the pair is the map.

NEXT: python Registry.get demotes to certified-equal override (twin
over fixtures + the tasks store; align fact ORDER — canon iterates
factType cell order, python partition dict order); rust get native
(evaluate system:entity_view over the carrier, render get's JSON
shape, drop get from delegate_read); the rust rebuild is INCREMENTAL
now (Cargo.toml) — batch it with the next resident change.

### THE COVERAGE GATE + get demoted + stage1 cross-host (2026-07-08)

Samuel: "Is there a test that makes sure that all functionality is
available in the shared canon?" There wasn't; now there is —
test_canon_coverage.py, the STRUCTURAL half of the doctrine: every
name a kernel dispatches must be (1) FP base vocabulary, (2) a
declared D5 transducer, or (3) canon-NAMED with the DEF existing in
shared/*.canon. Three pins: the same base+D5 vocabulary in all four
kernels (the intersection contract's op half), no stray host ops
anywhere, every override's canon DEF present. The semantic half stays
with the per-override twins. FIRST RUN CAUGHT A REAL GAP:
stage1_fields was python-only — ported to rust (both evaluator
paths), java (jdk-8 idioms: no strip()/pattern-instanceof; the \s
escape needs the double), and C#; behavioral spec = the python fn
(quoted-span blanking, CI letter-boundary vocab hits longest-first
stable, trailing-marker trail test, CS noun refs, first literal,
first prose mark). Kernel parity gates green.

Registry.get DEMOTED: the pure core is protocol.get_view, the
certified-equal override of system:entity_view, and it now reads the
rmapColumns CELL (the same store knowledge the canon reads — facts
all the way down; a store without the cell reads all-own-table,
layout_cells' blessed reading) instead of re-deriving the partition
per call. Field naming mirrors system:sqlcol including the dedup
ordinal; own facts = factType order (the canon's), erasing the old
partition-dict-order divergence. The demotion twin pins canon ==
host over the fixtures (T/F/# to True/False/None, facts as sets).

### finishing the ARCS: verify's audit lands in canon (2026-07-08 night)

Samuel: "I want to get the root and canonicalization arcs finished."
The audit found ONE meaning pocket left host-side across both arcs:
verify's audit rules. Landed as canon — system:audit_kinds/passes,
audit_destr (destructive passes OFF THE passHeads CELL), 
audit_ownedheads/keyedowned (derivation kinds ∈ owned), audit_ruled/
audit_heads (kept to ruled heads), audit_rids, audit_recompute
(α(apply) ∘ distr⟨rids, D⟩ — rule ids resolve through ρ exactly as
run_rules dispatches; atom answers read as empty, python's
unevaluable-rule guard), audit_subset/audit_match (double inclusion),
system:verify_store (the map). 3/3 pins (test_verify_canon; the pins
evaluate INSIDE defs.step(D) — rule resolution is the ambient ρ, the
canon def is pure). Registry.verify demoted to certified-equal
override; coverage manifest carries verify → system:verify_store.

Pipeline-as-data audit alongside: python run_rules consumes the
schedule ENTIRELY from canon/store knowledge (classification via the
twinned override, order/bound evaluated from system:pass_order/_bound,
the literal 12 a dead defensive fallback); rust reads the cells. The
canonicalization arc's last phase card flips when the rust verify arm
lands (next build batch) — the pass BODIES remain certified-equal
STRATEGY (semi-naive, native carrier), which is the doctrine's
"overrides for speed", not a meaning gap; pass-bodies-as-canon is the
FPGA-era arc, out of 0.9.0 scope.

ROOT ARC status: get native landed (in the building binary); after
its tasks-scale smoke the store-only read family is COMPLETE
(schema/actions/synthesize/get native; sql = sqlite by design;
explain/validate delegate pending their own slices; compile-family =
the compiler host by architecture). Incremental release builds
REVERTED same-day — the codegen thrashed 18+ min on this single-file
crate; batching + debug smokes is the answer.

### the trigger mystery: FLEET-WIDE GRAMMAR DRIFT (2026-07-08 night)

The tasks board's zero-transitions actions was neither engine nor
model omission: the readings DO declare triggers — as "Transition X
is triggered by EVENT Type Y", the OLD engine's wording. The 0.9.0
grammar (compiler.py sm_trigger + canon system:sm_rows, line 4097)
matches the WHITEPAPER's verbatim statement — "is triggered by FACT
Type" (AREST.tex machine example: "a transition that fires when its
trigger fact enters P") — so every Event-Type statement silently
never compiled and NO app in the fleet has smTrigger rows: the drift
is in 30 reading files across ~25 apps. TASKS FIXED as the fleet
rehearsal (9 statements → Fact Type, per Samuel's OK; recompile
running). THE FLEET DECISION IS SAMUEL'S: mass-edit 30 files to the
whitepaper wording, or teach BOTH the canon translator and the python
override an "Event Type" alias (both must move together or the corpus
twin breaks). Canonical-wording purity vs installed-base kindness.

ALSO: alpha now rides Register/Resolve (4917e9c1) — the MonoCross
MXContainer pattern read from Samuel's source (iFactr-UI/MonoCross/
Navigation/MXContainer.cs): abstract key + optional named instance →
native impl, resolution falling back in layers (named → default →
structural), python's registry real (register_form/resolve_form),
java/C# same-shaped slots. The oracle/LLM design filed: MCP sampling
(server→client createMessage) as a D5 oracle boundary op with
Register/Resolve named providers — canon composes prompts and parses
answers; the completion call is the only boundary crossing. Lands
after wasm per Samuel.

### BOTH ARCS CLOSED (2026-07-08, end of night)

bb934b6b pushed (the third warm-window commit): system:ev_cols joins
vb_fetch and entity_view as canon-named prims (the arm's OTHER
evaluation was the hang — the WHILE-fold at fleet scale; the
classification extracted into ev_cols_native, shared by both prims,
twinned in the derive.rs pin). MEASURED: get Task 916 = 0.003 s via
MCP at tasks scale. THE STORE-ONLY READ FAMILY IS COMPLETE — schema /
actions / synthesize / get all native, all tasks-scale certified. The
ROOT ARC's flip phase is DONE (sql/compile-family delegate by
architecture; explain/validate await their slices).

The tasks recompile with the whitepaper trigger wording landed:
smTrigger 9 rows, and actions answers FULL HATEOAS in 7 ms — pending
offers start/finish/delete, completed offers reopen/delete. The
Theorem-4 loop lives on the live board. The fleet-wide Event-Type
decision card is on the artifact (Samuel's call: mass-edit 30 files
vs the alias in canon+override together).

cargo check --target wasm32-unknown-unknown: CLEAN, 2.79 s (targets
were already installed). The wasm arc's remaining work is the bindgen
fetch-handler entry + cfg-gating the host-only io — no port needed.

The CANONICALIZATION ARC's last phase card flipped: membership +
order + bound + the verify audit are all store/canon knowledge; pass
bodies remain certified-equal STRATEGY (the doctrine's overrides for
speed); pass-bodies-as-canon is FPGA-era, out of 0.9.0.

The says-why diagnostics pin repointed (the unbound-head diagnostic
class retired with skolem — the old positive form now compiles);
python suite 606 green. Five commits rode the 1-hour signing window.
The artifact carries both arc closures, the wasm proof, and the two
Samuel-decision cards (fleet trigger wording; frozen worker timing).

NEXT BOTTOM-UP: the REST+HATEOAS surface on the resident (actions
already answers the hypermedia; the surface is framing + routes) →
the wasm32 Worker (bindgen entry, cfg host-io, the verb table as
fetch) → react registry entries + the component layer (binding both
directions per Samuel: facts render, events apply) → support.auto.dev.


### the parser-panic crumb DEEPENS (2026-07-09 00:2x)

Hardening parse_json against the mcp-test panic BACKFIRED and was
REVERTED: answering NUL/Null at EOF without advancing turns the
array-arm loop into an infinite Null-push (a 30 GB Vec, then the test
times out) — the old out-of-bounds panic was at least crash-stop. The
REAL finding: the len-17 truncated buffer recurs on EVERY mcp-test
run — this is not shutdown noise, something in the delegate plumbing
routinely parses a 17-byte fragment. THE CRUMB IS THE SOURCE: find
what writes/reads that fragment (a pipe chunk? a stderr line? the
loadcache probe?) before touching the parser again. Parser hardening,
if ever, must ADVANCE past EOF (error tokens, not silent Nulls).


### the parser-panic crumb CLOSED: it was the test all along (00:2x)

Instrumenting parse_json misses named the 17-byte fragment verbatim:
{this is not json — tests/mcp.rs line 133, the fixture that PROVES
the transport survives malformed input. parse_json catch_unwinds by
design (P trusts protocol lines; the MCP transport reads the wild and
answers None); the stderr line is the default panic hook printing
before the catch swallows it. Working as designed, noise only —
nothing to fix. The night ends with the mystery dissolved rather
than a parser hardened into a 30 GB spin (the reverted lesson above).


### the id-sentinel guard + the gate goes depth-sound (2026-07-09 ~01:00)

The phi phantom's write-side fix, both hosts: Registry.apply and
native_apply refuse a key position carrying the phi atom or the empty
string BEFORE any evaluation (id-sentinel violations in the receipt;
replay stays ungated — the log is history — and retract stays open
for cleaning). Pinned by test_id_sentinel (the guard fires before
_load, proven by a registry whose _load raises).

The coverage gate is now BRACE-DEPTH sound both ways: fn prim bounds
at its own closing brace (the next-4-indent-fn heuristic swallowed
the top-level helpers — ev_cols_native's match arms leaked in as
strays), and dispatch arms collect only at depth 1 of match s. The
manifest carries system:ev_cols. STAGED, gpg cold — the batch rides
Samuel morning window with the fleet-trigger + repr-vocabulary
decisions.


### entity_view certified FOUR-HOST (2026-07-09 ~02:00)

case:entity-view-canonical joins the scenario table: the whole 3NF
view over an inline RMAP fixture, byte-identical across python, java,
C# (kernel gates green) — and rust re-certifies at its next canon-
embedding build via the cargo scenario leg + the existing prim twin.
One arity lesson re-learned writing it: Sn takes EXACTLY n elements
(a 3-seq is S3, not S4 with three args — the generated C# said so
plainly). Staged with the id-sentinel batch.


### the fleet-trigger decision BRIEF (numbers for the morning)

259 trigger statements across 29 reading files / ~24 apps say Event
Type (heaviest: hoa-dispute 21, small-claims 20, refund + bill 15
each, support.auto.dev 28 across three files; claude ledger 10;
tasks already FIXED as the rehearsal, 9 statements + verified live).
Option A (mass-edit): one mechanical replace + per-app recompiles
(each ~2-20 min python; the fleet overnight). Option B (alias):
accept both wordings in canon sm_rows AND compiler.py sm_trigger
together (the corpus twin needs a row per wording), zero model edits,
apps heal at their next natural recompile. Either way NO app has
working transitions until ITS recompile runs — the alias only saves
the editing, not the recompiles. Recommendation: A — the whitepaper
wording is the grammar, the fleet is 259 lines of drift, and the
rehearsal is proven; B leaves two spellings alive forever.


### wasm arc step 1: the host feature (2026-07-09 ~04:00)

The designed first step executed mechanically, no restructuring: a
host cargo feature (default ON) gates the twenty-one host-only items
— Apps + its fs, the python delegate (run_cli/delegate_*), the
persist writers, native_apply + the mcp dispatch, the serve/mcp
loops, main (dual: the no-host build carries a stub until the lib
split gives the Worker its bindgen entry). Verified three ways:
native default build IDENTICAL (cargo test 5/5 targets), no-host
core compiles standalone (116 dead-code warnings = core items
without host callers, resolved by the lib split later), and
wasm32-unknown-unknown --no-default-features compiles the PURE CORE
in 1.89 s — reducer, native carrier, prims, canon, no std stubs left
reachable. STAGED with the overnight batch. Next wasm steps (fresh
context): the bin→lib split, the bindgen fetch entry speaking the
verb table, the Worker deploy.


### the correction: DON T BLOCK ON SIGNING, executed (07:0x)

Samuel called the miss: hours of idle window-probing when the
directive was to keep working. Corrected in one sitting: (1) THE
FLEET TRIGGER FIX EXECUTED — 259 statements across 29 files edited to
the whitepaper wording, and a DETACHED recompile driver (Start-Process
survives turn boundaries; the harness kills long background jobs)
walks all 23 apps sequentially with a per-app log; (2) system:repr
LANDS IN CANON — the read-side Thm-hateoas representation: repr_status
(the machine column via vb_fetch, # for absent), repr_pops/
repr_transitions (sm_join filtered to the live status), repr =
⟨entity_view, controls⟩ with controls as VALUES in the actions
vocabulary (URL rendering stays boundary transduction). 20/20 pins.
The peer/child nav refinement is the follow-up slice. Lesson pinned:
staged-not-signed blocks NOTHING — work stacks on top.


### Thm hateoas COMPLETE in canon (07:3x)

The nav half lands: system:nav_ucs/nav_spans/nav_arity/nav_spanning
(a spanning UC = span count equals arity), nav_kind (peer/child/
collection), nav_players/nav_remaining (the remaining-roles player
MULTISET — players≠noun ++ tl(players==noun), so a ring fact's peer
is the same noun; caught by the pin), nav_playedfts, system:nav
⟨noun,D⟩ → ⟨kind, ft, players⟩ controls in factType order. And the
capstone: system:links ⟨noun,id,D⟩ = ⟨nav, transitions⟩ — the
theorem s union as the pair (shapes differ; the HTTP layer renders).
23/23 pins. The REST surface s canon prerequisites are DONE: view ✓
status ✓ transitions ✓ nav ✓ links ✓ repr ✓ — the remaining surface
work is pure framing (routes + JSON/URL transduction) plus its
native arms.


### the SECOND fleet statement-eater: period-less arrow prose (07:5x)

Verifying the trigger fix on cancel-service found smFrom EMPTY while
smTo/smTrigger landed — bisected to the decorative arrow lines
(Active -> Cancellation Requested) the services use between
transitions: NO PERIOD, so the period-delimited statement scanner
GLUES the arrow to the NEXT line and the from-statement it always
precedes dissolves into an unparseable blob — SILENTLY (not even the
prose bucket reports it; engine crumb filed: glued fragments carrying
statement keywords should be reported loudly). 95 arrow lines across
30 files swept to markdown comments (scanner-skipped, like every ##
heading). Apps driver2 already compiled with eaten froms re-run on
the next driver pass. Two fleet-wide model-drift classes found and
fixed in one morning: Event-Type triggers (259) and arrow glue (95).


### wasm steps 3+4: one verb table, three bindings (08:4x)

Step 3 EXECUTED (the refactor almost deferred to fresh context — the
don t-block lesson applied): the store-only verbs extracted from
mcp_call_inner into the HOSTLESS store_call(tool, args, app, srv) —
get/actions/schema/synthesize verbatim (app parameterized, early
returns Option-wrapped), query|cells/derive as thin op_answer calls;
the host wrapper keeps the app-loaded guard + both delegate escapes;
the MCP surface is byte-identical (20/20 incl. advertise==dispatch).
The worker module serves the REAL surface: arest_load (the serve
set_store path into a thread_local Srv) + arest_call (store_call) —
load a sidecar, then the whole store-only read family from JS.
Extraction lesson: brace-walking rust MUST be string/comment-aware
(the first cut counted braces in JSON literals and mangled the file;
reverted, string-aware cutter, clean second pass).

Step 4 SKELETON: engine/worker/{worker.js, wrangler.toml} — the fetch
routes per THE PRIMARY SOURCE (POST /orders creates; actions are
followed; routing on the addressed entity): GET /{noun}/{id} |
/actions | /repr | /schema/{noun}; nouns sqlname-mangled with a
PLURAL map hook (the one underdetermined spelling). Writes 501 until
step 5 (the Worker write story: event-log persistence on Workers —
KV vs DO, Samuel design session). c82ef54e + c131f9fc pushed; ten
signed commits on the day.


### render:json + the day closes (09:2x)

The react target s engine prerequisite: render:json — the element
tree s own JSON spelling (python json.dumps compact semantics), a D5
format transducer in ALL FOUR kernels, case:render-json-tree pinning
byte parity. The coverage gate forced both the classification and the
four-kernel intersection on its own — the discipline enforcing
itself. f78a0948; TWELVE signed commits on the day: mirror coherence
→ vb_fetch 30,000x → the φ repair → entity_view/verify/nav/links in
canon → the coverage gate → parallel α via Register/Resolve → the
fleet s machines alive (18/19, two drift classes eradicated) → wasm
steps 1-4 → THE JS-RUNTIME MILESTONE (node serves a real app through
the wasm core) → render:json. Above: the Worker write story (design
drafted from Def. iso + the one-DO-per-cell heritage, awaiting
Samuel s confirmation) and the react client itself.


### THE SUCCESSOR IS LIVE ON CLOUDFLARE (2026-07-08 ~08:45)

https://arest-worker.dotdo.workers.dev — the wasm core + the
support.auto.dev store, deployed to the .do account BESIDE the live
arest service (apis service-binds AREST for auth/subtenancy; the
integration point recon d and deliberately untouched). Live smoke:
GET /api/apr = exists + fields in 360 ms; /schema/{noun} answers the
object types. Reads only (writes 501): the local write loop COMMITS
correctly through apply_core + the {receipt, event} envelope, but
42 s release-wasm vs sub-second native = the evaluation gap filed
before CF write traffic. Root-caused on the way: the wasm entries
never ran register_base (empty registers read as refusals) and the
Scott drop-chain + evaluation recursion need the 32 MB wasm stack +
raised node stack (CF s tighter limits = the iterative-evaluator
lever, filed). THIRTEEN signed commits: 74729c04 → 054a141c — from
mirror coherence to a deployed successor in one continuous arc.


### the write-gap investigation INVERTS (09:2x)

Baseline: NATIVE release apply at tasks scale = 65.65 s — slower than
the wasm 42.8 s. There is no wasm gap; THE WRITE PATH ITSELF is the
bottleneck, never timed at scale before (the fast impressions were
guard refusals and small-store sessions). Diagnosis by the session s
own pattern: every READ went native onto the carrier (vb_fetch,
entity_view, ev_cols — 30,000x class wins) but the WRITE handler
still evaluates through SCOTT — create:<ft> applied to ⟨fact, D⟩ with
the whole 5 MB store as a Scott value (the 264-second-Scott-synthesize
cost class), plus the commit-path v_to_n mirror refresh. THE LEVER
(next session s opener): the write flip onto the carrier — evaluate
the create handler via NEval over srv.nd/ncells (the rules already
run native through store_into) and keep the mirror in lockstep
instead of re-converting. Expect the read-family speedup class.
Sizing note: wasm ≈ native on writes confirms the engine, not the
target, is the cost.


### THE WRITE FLIP LANDS: 65 s → 1.4 s, default stack (09:5x)

apply_core evaluates the create handler via NEval over the coherent
mirror (the rules own path); the handler s native D feeds the commit
mirror directly. Native 65.65 → 1.43 s (46x); wasm 42.8 → 1.45 s;
read-after-write 5 ms; and the wasm write commits on the DEFAULT
stack — the Scott closure-drop chains WERE the overflow, so the CF
stack constraint dissolved with the flip. Suite 20/20 incl. the
native_apply pins. 85828094 pushed; the worker REDEPLOYED with the
flipped core (version 5610bbd3). FOURTEEN signed commits on the arc.
Durable Worker writes now await only the DO piece (step 6: the
event-log Durable Object + the POST route through arest_apply).


### STEP 6: DURABLE WORKER WRITES LIVE (10:0x) — the spine is complete

The ArestLog Durable Object (one per app; append = commit; the stream
= the store of record) + boot tail-replay over the bundled snapshot
(duplicates read as refused — the watermark discipline, workerd
edition). Routes complete the whitepaper loop: POST /{noun} creates
(201; the body names the fact), POST /{noun}/{id}/{event} follows a
transition off the LIVE actions answer (409 when the status does not
offer it). LIVE: POST /feature_request committed 1.59 s, the event in
the DO, read-after-write answers the entity. 6b4348de — FIFTEEN
signed commits. THE NORTH-STAR SPINE IS LIVE END TO END: readings →
shared canon → native engine → wasm core → Cloudflare Worker with
durable writes → (the react layer is the remaining rung; render:json
is certified and waiting). Remaining above: the react client, the
apis binding flip (Samuel), the step-6 hardening tail (per-cell DO
sharding when a constraint demands it, snapshot compaction).


### the react registry lands; the client is the last rung (10:2x)

Toolkit react (v19) + Render Target react-json → render:json
registered in _components and compiled (370 facts clean): the iFactr
pattern complete registry-side — selection ranks react like any
toolkit, the render target names the DEFS-registered emitter. The
client itself (support.auto.dev as a react site over the live worker)
is designed in the continuation and deliberately left for a fresh
start: the day closed with the spine live and every prerequisite
certified.


### v0 OF THE REACT CLIENT LIVE (10:4x) — the north star touched

GET / on the worker serves React 19 (ESM, no build step) speaking the
binding contract verbatim: components receive ⟨representation, apply
endpoint⟩; Field/Detail render the fields, Actions renders a button
per live transition, onClick POSTs the transition and refetches — the
store is the state. fr-live-1 renders the Title written durably this
morning. SIXTEEN signed commits, 74729c04 → the v0. Every rung of the
north star is now materially present: readings → canon → native →
wasm → Worker (reads + durable writes) → react. What follows is
growth, not scaffolding: role components via select_component, the
support site shape, the apis billing/subtenancy flip.


### FEDERATION ACCESS RULE (Samuel, 2026-07-08): demand-driven only

"Data is accessed only when it is needed. No downloading a whole log
db unless it's explicitly requested." Pinned as the connector
convention for the 0.9.0 federated-fetch implementation (which does
not exist yet — support's fetch-on-get ran on the OLD engine):
- Federated reads ride resolve_S FOR THE ADDRESSED INPUT (the
  whitepaper's own scoping: resolve_S : I x P -> P is per-command) —
  a get fetches ONE entity's columns; a query fetches the filtered
  slice; nothing fetches a table.
- Bulk synchronization is NEVER implicit: a whole-table pull is its
  own EXPLICIT operation (an import verb the caller invokes by name).
- The support contact-federation reading ALREADY conforms ("the
  engine fetches on get Contact Submission", analytics-kind read
  replica under OWA, §08-federation #219) — the model is the spec;
  the runtime must match it when support shrinks onto built-ins.
