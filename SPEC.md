# AREST Rebuild — the Constitution

Adopted 2026-07-14. Samuel approved the design (sections 1–9) with two amendments,
recorded as Decisions D1 and D5 below. The build is authorized to run one-shot,
without further input; `main` is replaced only on Samuel's explicit word.

Citation forms: (Def n), (Prop n), (Lem 1), (Thm n), (Cor n), (Eq n), (§n) refer to
AREST.tex at this commit. (Backus §n), (Codd §n), (Halpin §n), (NORMA) refer to the
named sources. (D n) refers to the Decisions section. A normative sentence carrying
no citation and no Decision tag is invalid: reject it at review, do not build from it.

Errata protocol: where evidence contradicts a source, file an erratum here
(assertion / evidence / correction) for Samuel's adjudication; do not build against
an unadjudicated erratum. Current list: **empty**. The 2026-07-14 audit
(canon-first 181bc775) found implementation infidelities to Def 5, not theory errors.

---

## 1. The object

1.1 An AREST system is a Backus AST system whose FILE cell contains a population P
of a schema S, and whose DEFS cells hold the FFP objects compiled from FORML 2
readings together with those registered by the runtime (Def 1).

1.2 Membership is application: P x and x ∈ P are one act; ρ exposes a fact as the
function it already is; metacomposition (Eq 1) is the only mechanism — every
constraint, transition, link, and response is an instance of it (§1).

1.3 A schema S comprises the vocabulary (value types; entity types with simple or
compound preferred identifiers, auto-generated or supplied), the fact types F, the
constraint set C_S — uniqueness, mandatory, frequency, ring, value-comparison,
subset, equality, exclusion, value, cardinality, each alethic or deontic — the
derivation rules R_S, the state machines, and the reference schemes (Def 2). S is
the compiled content of DEFS.

1.4 A population P is a finite subset of ⋃_{f∈F} {f} × Val^arity(f). A ground fact
is true if in P, false if its paired negation fact is in P, unknown otherwise;
closed-world on a noun collapses unknown to false (Def 3).

1.5 The accepted language is fragment R (Def 4): object-type and reference-scheme
declarations, elementary fact-type readings, the constraints of 1.3, projective
derivation readings, and state-machine readings whose trigger is an elementary fact
type. Pronoun-correlated clauses and nested objectification are rejected. No
unquoted declared name may contain a formal grammar item as a substring; quoted
identifiers are the escape (NORMA). parse is total on R and
nf = verbalize ∘ compile ∘ parse is idempotent with nf(r) ~ r (Prop 1).

1.6 Retrieval is Codd's θ₁: projection, natural join, tie, restriction
(Codd 1970 §2.2; 1972 §2.3.5). Every constraint of 1.3 compiles to a restriction
whose predicate is one of exactly two families — a cardinality count against
declared bounds, or a membership test against a target population — with polarity
the only parameter separating subset from exclusion (§2). A third family may not be
introduced.

## 2. Commands and the uniform gate

2.1 A command decomposes as resolve_S, derive_S, validate_S, emit_S (Def 5).
resolve_S adds the entity and fact instances named by the input, minting a fresh
identifier exactly when the reference scheme is auto-generating. derive_S is
lfp(F_S, ·). validate_S computes V = ⋃_{c∈C_S} (ρ c):P″ over the ENTIRE result
population (Def 6, Thm 1). emit_S builds the representation from P″, V, and links.

2.2 THE GATE IS ONE. Every mutating step — create, batch, retract (D1), DEFS
ingestion (Cor 5), journal replay (§12) — commits iff V contains no alethic
violation, else D is unchanged (Def 5). There is exactly one implementation of the
gate. A verb may not enumerate its own checker subset. (Evidence for why: the
canon-first create gate consulted uniqueness and mandatory but not the compiled
exclusion, producing a committed-forbidden fact that was unretractable, unfixable,
and immortal under replay — canon-first 181bc775.)

2.3 A deontic violation warns and commits; an alethic violation refuses; the
violation message is the canonical reading of the constraint (Def 6, Prop 1).

2.4 A step input may name finitely many assertions and retractions together; the
gate judges the final population, atomically (Def 5's input names instances,
plural). Consequently every valid-population-to-valid-population move is reachable
in one step, and no sequencing wedge exists.

2.5 Retract (D1): retraction removes one asserted base fact. Fully derived facts
(mode *) are consequences and are not directly retractable; a semi-derived fact (+)
is retractable only in its asserted row. After removal, derive_S recomputes
lfp(F_S, ·) from the asserted base; the gate (2.2) judges the shrunk result; refusal
is an ordinary violation answer. Authorization to retract, when restricted, is a
deontic or derivation reading over P (Cor 3) — never a host check; absent such a
reading, retract is available.

## 3. Derivation and negation

3.1 Derivation rules have projective heads: no rule introduces a fresh entity;
modes are * (fully derived), ** (derived and stored), + (semi-derived) (Def 7).
Entity introduction is confined to resolve_S and to state-transition rules guarded
by positive events (Def 7, NORMA).

3.2 lfp(F_S, P) exists, is finite, and is reached by finite iteration provided no
value-introducing rule lies on a dependency cycle (Lem 1). The hypothesis is a
query over DEFS itself: rule read/derive sets are schema facts, value introduction
is syntactic, and the acyclicity check is finitely many reachability queries,
evaluated on every DEFS change and refused like any alethic violation (Cor 2).

3.3 Negation is open-world and explicit (§2 Negation). An epistemic falsity enters
P as an explicit negation fact and is never inferred from absence. A negated role
path is an inference from absence: a finite-set anti-join over settled facts, fixed
across derivation rounds (Lem 1). State-transition rules: an add or delete may not
occur under negation or disjunction; each disjunctive branch carries a positive
event (NORMA).

3.4 Every noun carries exactly one world-assumption fact. The compiler defaults it
to open-world (Halpin's default; D2) and records the defaulted fact explicitly, so
the metamodel's mandatory constraint is satisfied by construction and negation
semantics are total.

## 4. State machines and time

4.1 A state machine is a set of facts: statuses, transitions, and each transition's
trigger fact type (§1). The live step is the AST transition
μ(SYSTEM:x) = ⟨o, d⟩ with D frozen during evaluation (Prop 2; Backus §14.3.1,
§14.6). Advancing on a trigger fact is part of the committing step, not a second
machine.

4.2 Reconstruction machine(s₀, E) = foldl transition s₀ (order_τ E) is a
ρ-application over event facts, used for migration and audit; it orders by
occurrence timestamp τ (valid time), while the live step takes arrival order
(transaction time) (Prop 2; Halpin §13.6). The journal (§12) preserves both: τ is a
role of the event fact; arrival order is the log's.

## 5. Representation

5.1 links(e) = nav(e) ∪ transitions(status(e)) is a θ₁ expression over P and S,
complete: every valid control and only valid controls (Thm 2). No hand-written
links; no undocumented endpoints.

5.2 Every value in repr(e) — selectors, derived facts, violations, links — is
(ρ f):P for some object f (Prop 3). Authentication, authorization, validation, rate
limiting, and transformation are restrictions, derivations, or facts over P, never
a layer outside ρ (Cor 3). Transports (HTTP, MCP) are registered adapters that form
SYSTEM:x (Eq 2) and carry zero meaning.

5.3 A subscription is a ρ-application not yet evaluated against the current D; an
external event enters P through ↓; externally fired and fact-fired updates are
indistinguishable to the evaluator (Cor 4).

5.4 Deletion is logical: an entity at a status with no outgoing transitions has
links(e) = φ and is excluded from queries by restriction; physical reclamation is a
compaction preserving the population ρ observes (§2). Retraction (2.5) is for
erroneous data, not lifecycle end (D1).

5.5 The liveness discipline is itself a reading: a deontic obligation that each
transition cycle carry some exit transition (§2, after Thm 2).

## 6. Cells, scope, and tenancy

6.1 RMAP assigns each entity its own cell — the 3NF row of facts depending on its
key — so D is a sequence of cells (§3; Halpin). At most one step writes a given
cell at a time; steps writing disjoint cells run concurrently (Def 8). The
recalculation a write forces is bounded to the entity's cell and the role-player
cells its constraints reach, a scope fixed at compile time (§3).

6.2 CAP posture is per noun, chosen by modality and world assumption: alethic +
closed-world + single-writer is CP; deontic + open-world is AP; slack tunes
coordination (§3). Specified now; consensus and multi-peer logs are out of this
week's build (D3).

6.3 A tenant is a cell whose contents is an entire store; isolation is
preservation of addressability — wrong-tenant access is not forbidden but
unaddressable (Prop 4). Sub-tenancy recurses; every node is a full instance
(Cor 5 per store). Specified now; not built this week (D3).

## 7. Self-modification and the boundary

7.1 Self-modification is an ordinary step addressed at DEFS with operation
compile ∘ parse; ingestion is a create, subject to the gate; a schema whose alethic
constraints the current P violates is rejected, so migrations stage as derivations
or deontic rules until P complies (Cor 5).

7.2 A definition is ⟨name, dom, cod, origin, impl⟩ with origin ∈ {compiled,
registered}; compiled means impl = ρ(o), total and decidable over finite P;
registered means host-supplied, in general partial (Def 9). The informal surface is
the restriction Filter(eq ∘ [s_origin, 'registered']):DEFS (Eq 5) and coincides
with the decidability frontier (Cor 6).

7.3 Native legality: a registered definition that shadows a compiled definition and
is observationally equal to it contributes no informality and may be dropped
without changing any value; the licence is Backus's algebra of programs (§4;
Backus §12.2). Same-bytes (G3) is the standing proof obligation for every native
fast path. A host behavior that is not such a shadow is meaning, and meaning
outside the canon is drift.

## 8. The canon artifact

8.1 THE SHARED CODEBASE IS THE CANON (D7): one polyglot λ file, `arest.canon`,
legal source in both host syntaxes — Python executes it, Rust includes it — the
same bytes, so the file is comment-free and restricted to the shared vocabulary
(A/N/K/S1..S9/DEF/PHI). Its basis is fixed and four-legged: ZFC — a set is its
characteristic function, membership is application (1.2); the λ-calculus — a fact
is λf.f(o₁…oₙ), and DEFs lift to closed pure λ-terms (G1); Codd's adequate θ₁
(1.6); and Backus's combining forms (H1). Everything the system MEANS is a term of
this file: grammar tables, metamodel, constraint builders (the two families), the
derivation machinery, the SM step, links/emit, the boundary query. Hosts carry
speed and effects, never meaning (H5).

8.2 Litmus (G1): every canon DEF lifts to a closed pure λ-term (ρ-fidelity), or is
declared a registered tuple of 7.2. A boundary member without a declaration is a
defect.

## 9. The host contract

A host implements exactly this and nothing else:

H1 μ-evaluation of the FFP combining forms: composition, construction, condition,
   constant, selectors, eq, φ, insert, while, apply, and metacomposition (Eq 1)
   (Backus §11.2.4, §13).
H2 Cells: fetch ↑n:D and store ↓n:⟨x, D⟩ (Backus §13.3.4, §14.3).
H3 The single transition: form (SYSTEM:x), evaluate under current DEFS with D
   frozen, yield ⟨o, d⟩ (Eq 2; Backus §14.3.1, §14.6).
H4 The registered set, each entry a 7.2 tuple: server registers httpFetch and
   upsert; a browser registers render (§2 Platform binding; iFactr shapes).
   Effectful functions run during resolve_S or after emit_S; derive_S is pure (§2).
H5 No meaning: any host behavior observable through the gates must be derivable
   from DEFS; hosts may transform compiled objects only into observational
   equivalents (7.3).
H6 Same-bytes: for every conformance app (G3), compile and command outputs are
   byte-identical across hosts.

## 10. The metamodel

10.1 The metamodel is itself readings of fragment R describing DEFS: fact types,
roles, readings, constraints, derivation rules with their read/derive sets (Cor 2),
state machines, world assumptions. It is an app of the system (Cor 5).

10.2 Self-satisfaction (G4): the compiled base population satisfies its own alethic
constraints — validate(base) = ∅. A base that fails its own schema does not ship.
Readings files contain only sentences of R and comments; prose may not enter the
metamodel as a name. (Evidence: the canon-first base carried a prose paragraph as a
fact-type name and hundreds of self-violations, drowning validate — 181bc775.)

## 11. The compile pipeline

11.1 parse (total on R, 1.5) → compile (metamodel facts + the two-family constraint
restrictions + derivation defs + SM rows) → RMAP to cells (6.1) → verbalize
(nf idempotence, Prop 1). The grammar is one artifact, data in DEFS; documentation
of the grammar is generated from it or conformance-tested against it — the docs may
not describe a rule the parser lacks. (Evidence: canon-first docs advertised
multi-join rule bodies the app compiler rejected.)

11.2 Ingesting instance facts inside readings rides the same create gate as runtime
writes (Cor 5). There is no separate "compile-time population" path.

## 12. Journal and replay

12.1 The journal records committed step inputs, in arrival order (4.2). An input
that was refused is not journaled. Derived facts never journal — they are
consequences (2.5).

12.2 Replay folds the journal through the same gate (2.2). Because each entry
committed against its predecessor state and the gate is deterministic, replay
reproduces D exactly. An entry that fails replay signals corruption: halt with a
report; never silently diverge, never silently resurrect. (Evidence: canon-first
replay resurrected a forbidden fact into every rebuilt store — 181bc775.)

## 13. Conformance gates — the definition of green

G1 ρ-fidelity litmus total over the canon (8.2).
G2 NORMA corpus: zero unparsed on fragment R; nf ∘ nf = nf and nf(r) ~ r (Prop 1).
G3 Same-bytes across Python and Rust for the conformance apps:
   base, paper-order (the §1 listing, verbatim), organizations, redo-decision.
G4 validate(base) = ∅ alethic (10.2).
G5 Gate-uniformity regression: the redo-decision forbidden apply is REFUSED;
   create-then-retract of the same fact is identity; the wedge scenario cannot be
   reproduced (2.2, 2.4, 2.5).
G6 Verdict stability: redo-decision recompiles to
   eliminated = {salvage-assembly, strangler-in-place} with the greenfield choice
   committed (the decision ledger re-derives).
G7 The Cor-2 acyclicity query runs on every DEFS ingestion; a value-introducing
   cycle is refused as alethic.

Green means all seven. A step that cannot keep them green is reverted, not patched
around.

## 14. Salvage protocol

14.1 Quarry: canon-first @ 0fa14b7c (read-only). Salvage is artifact-level;
commit history is not replayed. Nothing enters by trust.

14.2 Gates by artifact class: canon DEFs → G1 + a citation for the meaning;
parser/compiler behaviors → G2 (NORMA corpus round-trip); host code → H1–H6 shape +
G3; tests → only if they assert a source-backed behavior (a test asserting drifted
behavior is discarded with a note); docs → regenerated or conformance-tested
(11.1).

14.3 Every salvaged artifact gets one line in SALVAGE.md: source path @ commit,
gate evidence, disposition. An artifact rejected at a gate is listed with the
failure — silence is not a disposition.

## 15. Non-goals (this build)

Tenancy and consensus execution (specced in §6; D3). μ-memoization as a
performance strategy (empirically net-negative; legal only as a 7.3 equivalent;
D4). UI toolkit beyond the render registration shape (H4). propose/induce
re-entry until the core gates are green (then salvaged under 14.2).

## 16. Decisions

D1 2026-07-14 Samuel — retract stays: data cleanup must be possible;
   restrictable by role via deontic readings (2.5, Cor 3), but available.
D2 2026-07-14 — world assumption defaults open-world, recorded as a fact (3.4;
   Halpin's default).
D3 2026-07-14 — week scope: single store; tenancy/consensus specced, not built.
D4 2026-07-14 — μ-memoization is a non-goal (task-26 experiments: 1.7% hit rate,
   net slower).
D5 2026-07-14 Samuel — one-shot: execute without further input; phase gates are
   the conformance gates (§13), not review pauses; `main` only on explicit word.
D6 2026-07-14 — venue: orphan branch `rebuild` in graphdl/arest; commit #1 is this
   file with AREST.tex; canon-first is frozen as the quarry.
D7 2026-07-14 Samuel — the shared codebase is the polyglot λ file `arest.canon`,
   based on ZFC, lambdas, Codd's adequate θ, and Backus's combinators; hosts are
   thin μ-evaluators over the same bytes (8.1, H1–H6).

## 17. Errata

(empty — see protocol in the preamble)
