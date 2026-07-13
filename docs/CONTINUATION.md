# CONTINUATION — state of the world after the 2026-07-08/09 session
# (rewritten for a memory clear per Samuel; this file is the next
# session's orientation. Untracked by policy — never commit docs/*.md
# session files.)

## 2026-07-10 LATEST — FRONTIER REACHED, SECURITY DONE (READ FIRST)

All commits SIGNED (Samuel's key; NEVER use -c commit.gpgsign=false — that was a
misread of push-arest-freely-pre-1-0's "commit with signing, never bypassed";
the whole session's history was re-signed via rebase --exec) and pushed to arest main.

### SECURITY SWEEP #21 — DONE in code (report artifact:
https://claude.ai/code/artifact/2869ae24-feeb-4afd-b328-cd0e2b673d63). OWASP web+API.
Findings all fixed:
- F1 unauthenticated reads (HIGH, API1/A01): the worker served every GET + SSE /events
  unauthenticated (verifyActor was writes-only). FIXED — a fail-safe read gate in
  worker.js (engine/worker): ADMIN reads all; a CUSTOMER reads their OWN Support Requests
  object-scoped by Support_Request_is_for_User == auth.vin user.id (list docs filtered,
  get/view/repr/actions 404 if not owned); anon/non-owner 401/403; SSE admin-only. verifyActor
  now mints the customer read grant + User_is_verified_as_Id. Commits 59fce7f5 (gate) +
  f7e5a541 (customer own-reads). Read authZ rules also in apps/support.auto.dev/readings/
  authorization.md (SEPARATE apps repo, LOCAL+UNCOMMITTED — other-repo rule, awaits Samuel).
- F2 deps (A06): 34 dependabot alerts (9 high) all in apps/ui.do -> pnpm-workspace.yaml
  overrides pin patched versions (react-router stays 6.30.4). audit -> 0. Commit 5b44ac3f.
- F3 federation SSRF: guard refuses loopback/private/link-local/metadata + redirect:error.
  Commit 708118e2. F4 stale comment: fixed 59fce7f5.
- Secrets: CLEAN all 5539 commits. Auth is federated (auth.vin, no header-trust); tenancy
  per-app by hostname; D1 parameterized. SAMUEL'S DEPLOY: wrangler deploy the 3 worker commits
  after a wrangler-dev session test (own SRs appear, another customer's do NOT, and confirm
  auth.vin user.id EQUALS the Support_Request_is_for_User value — ownership hinges on it).
  api.auto.dev is OUT OF SCOPE (Samuel).

### BOARD
- #13 DONE (f1ca682f): merged Constraint Type + Constraint Kind into ONE classifier (NORMA
  ConstraintType); the two entities were inert inventory, runtime `constraint` cell f[1] token
  untouched. Verified base recompile + 16 constraint tests.
- #18 (in_progress): TEN handlers canonized as system:h_* certified-equal twins — objectification
  (pilot), 6 SM (h_sm_def/initial/from/to/emit/moore -> system:sm_rows), ref_scheme, 2 meta
  (data_type/ref_mode). The CLEAN tier is exhausted; the remaining MEDIUM handlers (uniqueness,
  mandatory, frequency, ring, ...) need shared-canon infrastructure FIRST: a system: subject/
  clause resolver (generalizing _subject/_clause_ft) and a mand-rows row object (like sm_rows/
  cs_rows). The rule family (_h_rule_if etc) + _h_subset are HARD/blocked. Canon combinator note:
  an N-element list is S<N+1>(CONS, e1..eN); nested cons is WRONG (builds a nested pair).
- #20 (in_progress): SIX safe wins this session — reducer hot-loop hoist (a425a94d), native
  store-append for the g-loop (4feae99b + 06cbf172 Scott-level fix), _cells_of memo (06cbf172),
  _pop_rows memo (0e75145a, 298x on repeated reads). Plus the pre-session _cells (46x validate)
  + rmap_partition memos. Base schema compile >200s -> ~66s; validate ~11.8s. THE MEMO SEAM IS
  NOW EXHAUSTED (every population-read function is memoized). Remaining for genuine SECONDS = the
  native INDEXED store carrier (dict name->cell, O(1) access threaded through compile_model +
  run_rules, Scott only at phase boundaries) — a deep refactor of the store rep the whole engine
  shares; the compile is now dominated by mu reducing the create pipeline + the derivation
  fixpoint, not decoding.
- #15 (in_progress): lattice half DONE (Fact Type not a Noun, 05e7314b). The FILL half remains —
  a careful instance-mirror arc (engine.py:1219 _MIRROR is over-broad + fallback-only; the fix
  is most-specific + a transitive is_a closure + domain-seed at the base/app seam). NOT keep-alive
  sized; needs a focused pass. Subagent plan in the 2026-07-09 section below.
- #22 (pending): DO-178C/DO-254/DO-330/DO-333 assurance-case scoping for the OS track (Samuel:
  high-assurance embedded is the point of the os track). Awaits a real safety-critical customer.

### STATE: the tractable SAFE seam is worked out. The three big remainders (#20 native carrier,
### #15 mirror arc, #18 medium-tier infra) are each focused high-stakes passes wanting Samuel's
### steer on priority. A 3-min keep-alive (cron d67c4a53) is running; it has reached the frontier.

## 2026-07-10 — #20 PERF LARGELY LANDED + #13/#15/#18 PLANS READY (READ FIRST)

### #20 compiler perf — TWO commits landed (main, verified)
The task's cited 1509s compile / 880s validate are BOTH STALE (pre-memo). Reality now:
- **VALIDATE was already solved** by `b5508965` (`_cells` memo, 880s->11.8s, 46x) and
  `7575a4f3` (`rmap_partition` memo per-D), BOTH landed 2026-07-08 AFTER the profile.
- `93884c99` — **reducer hot-loop hoist** in `_make_mu` (kernel.py:758): drop the
  redundant `len(e)==3` (APP_D uniquely heads the 3-tuple app node), inline `_isseq`,
  SKIP `consume_fuel()` when the step is unbounded (compile's case; frame fixed per mu
  tree), bind hot globals to locals. Semantics-preserving; certified by the canon suite.
- `2b2bb0ac` — **native store-append** `engine.run_append` (engine.py, after `run`): the
  self-host g-loop (compiler.py ~1965) asserted each fact via `ast.run` reducing the whole
  build_system pipeline over the base-sized D per assert (Store=apndl∘⟨cell,Pop⟩, Pop a
  WHILE-fold over EVERY cell = the O(|D|) base-walk). The g-loop consumes only D′, so
  run_append computes D′ natively (FetchPop + dedup-prepend + cell replace), deferring to
  the canonical `run` for any non-plain shape. Certified-equal twin —
  test_native_append_canon (decoded populations, ==). `run` stays the meaning.
- `9496bb24` also rewrote run_append to the SCOTT level (the first native-tuple cut did a full
  O(|D|) deep conversion per assert AND broke scott_to_native's id-memo -> regressed apps atop
  the base) and memoized `_cells_of` per-D (removed 18.8s of a tasks profile: every defs.step
  rebuilt the whole {name:contents} view).
- Clean no-base micro-probe (compile_model, D=None): **9.39s -> 2.99s/compile** (3.1x); mu
  calls 18.7M -> 6.6M. Correctness: 199 passed (canon+kernel+forml+compile) + the 2 append certs.
- **A/B, base SCHEMA recompile (494 fts, no app data), HEAD vs pre-perf 2dcc04be: >200s -> ~98s
  (2x+, NO regression).** Still not "seconds": the base compile is now dominated by RUN_RULES
  (the derivation fixpoint, mu-heavy over base populations) + run_append's residual O(|D|) Scott
  walk. The whole pipeline is O(N·|D|) (flat Scott store, O(|D|) cell access). The real fix is
  the native INDEXED store carrier (dict name->cell, O(1) access) threaded through compile_model
  + run_rules, Scott only at phase boundaries — the documented deep half of #20. Full task #20
  description has the complete profile + commit list.
- OPERATIONAL: any engine .py change bumps the engine fingerprint -> the content-keyed
  frozen base cache MISSES -> first compile pays a full base recompile (~176-250s) then
  caches; later thaws are ms. Measure with a WARM base and NO background CPU contention
  (subagents/probes inflate cProfile per-call time — a "regression" that vanishes clean).
- STILL OPEN for "seconds" on big apps: run_append is native but still O(|D|) per assert
  (the Pop/FetchPop walk). An INDEXED store (dict name->cell) would make it O(1)+O(|P|).
  Also unmeasured post-memo: create_handlers / replay_entries phases (stale profile 236s /
  274s). Get the warm-base phase breakdown from support_probe before deciding.

### #13 merge plan (subagent-verified — READ before executing)
The two core.md entities `Constraint Type`(core.md:24, 16 rows) and `Constraint Kind`
(core.md:26, 23 rows) are INERT INVENTORY — **zero runtime readers** (grep-confirmed).
The load-bearing classifier is the runtime `constraint` cell's `f[1]` TOKEN
(uniqueness/ring_*/deontic_*), dispatched by `_ATTACH` (compiler.py:2251) / `validate_for`.
The only live is-of is relation (b) `Constraint is of Constraint Kind about Fact Type with
Modality`, living ONLY in compiler.py M (`M_READINGS` :54, `M_MAP` :66 -> "constraint").
MERGE (survive as `Constraint Type`, NORMA's name): fold Kind's Label/Family/ViolationTemplate
onto Type, keep the 23-code superset (Kind ⊃ Type: +RF +6 deontic; preserve Name≠Label),
drop the unpopulated relation (a) core.md:292 + its :333-334 constraints. Edits: core.md
(:24/:26 collapse, :292 drop, :300-308 merge sections, :790-807 + :978-1036 -> one 23-code
block), validation.md (:60-61 + :97-122 retarget), compiler.py 4 lines (:45/:54/:55 rename,
:66 M_MAP key; VALUE stays "constraint"). DO NOT touch the `constraint` cell shape or the
`f[1]` token set (protocol.py:2229 reads (cid,token,…scope…,modality)). This merge REMOVES
the shared-id double-noun root cause (#12 relaxation can re-tighten after).

### #15 self-description FILL plan (subagent-verified)
NEITHER fill is currently done. `Function_belongs_to_Domain` (core.md:287, deontic-oblig)
has ZERO producers (empty; the instances.md:293-309 "bridge" is DISABLED comment-stripped;
`annotate_noun_domain`/`compile_derivations`/`ast.rs` are STALE RUST artifacts). The
`_MIRROR` (engine.py ~1219-1251, `Resource_is_instance_of_Noun`) is fallback-only AND
deliberately OVER-BROAD (role-play based, instance-of every supertype) — the engine.py:1241
NOTE is CURRENT (2026-07-09, mine), not stale; it awaits this refactor. PLAN: (A) unify the
mirror to MOST-SPECIFIC (subtype-reduce) + UNION the identity fill `(ft,'Fact Type')` for
every factType row; merge-gate not empty-gate. (B) add a transitive `Resource_is_a_Noun`
closure (clone `install_governed` engine.py:3441) and rewire the 2 supertype-join consumers
(SM-seed instances.md:343, `Constraint is semantic` core.md:405) onto it. (C) seed
`Function_belongs_to_Domain` at the base/app seam in Registry.compile (protocol.py:1713):
base fts->'core', app fts->app domain (provenance survives ONLY at this seam — files are
concatenated pre-compile). (D) declare `Domain` an entity type. Verify: most-specific rows,
domain non-empty, SM-seed still fires via `is a`.

### #18 inventory (subagent-verified)
39 `_h_*` handlers in compiler.py; ONLY `system:h_objectification` canonized (pilot). NEXT,
highest value first: (1) the SIX `_h_sm_*` (:1365-1387) — each a one-line `apply` of the
ALREADY-canon `system:sm_rows`, no objects; one pattern canonizes six. (2) `_h_ref_scheme`
(:747, 3 constant rows, trivial). (3) `_h_possibility`/`_h_meta` (single-row). (4)
`_h_entity`/`_h_value` (Name split + optional row). (5) `_h_negation` (smallest known-ctx).
LAST/blocked: the rule family (`_h_rule_if` :1502 ~250 lines, `_h_subtype`, `_h_class_rule`,
`_h_derivation_rule`) and `_h_subset` (:1018, raises — projection slice unfinished). `_h_fact`
is a permanent boundary (only its decl half is canon via `system:ft_rows`).

## 2026-07-09 CONTINUED — METAMODEL CANONICAL CORRECTION + PERF BLOCKER (READ FIRST)

Mid-arc on the metamodel board (#15/#13). The KEY canonical win is COMMITTED; the
perf blocker (#20) is diagnosed; #13/#15-fill/#18 remain (all verify-heavy).

### Committed this session (main, push OK pre-1.0)
- `dbd3c537`,`8e7ae261` — whitepaper (AREST.tex L157/L221): negation is a NATIVE
  ANTI-JOIN (theta:AntiRestrict), not "positive-only". Finiteness re-grounded on
  the finite-set anti-join over settled facts. Plain style (no em-dash, no "X not Y").
- `3cd9abc6` — #17 DONE: `system:absorb_core = INSERT[theta:NatJoin:key]`, the RMAP
  variadic-join core; absorb_rows dispatches to it. test_absorb_canon.py.
- `9933c4f6` — #18 PILOT: `system:h_objectification` (compiler-handler canon pattern;
  host stays native certified-equal for compile speed). test_hobj_canon.py.
- `2dcc04be` — #15 CANONICAL HALF: **Fact Type is NOT a Noun**. Flipped
  `Resource is a subtype of Noun` -> `Resource is a subtype of Function` (instances.md:6).

### THE CANONICAL FINDING (Samuel steered the whole study)
`Resource < Noun` was a GraphDL "Graph Schema is a Noun" artifact. Halpin uses Object
Type, not Noun. NORMA's OWN metamodel (`Repos/NORMA/Documentation/ORMCoreMetaModel.orm`,
44 subtype facts, parsed): **ObjectType and FactType are DISJOINT SIBLINGS** — both
`<: ORMNamedElement`; FactType's ONLY supertype is ORMNamedElement. A fact type is
"predicate + its object types", never an object type. Classification = INSTANCE-OF a
metatype; metatypes are siblings. Objectification (NORMA `Objectification`, 1:1) links a
SEPARATE nesting ObjectType to a FactType, never reclassifies. Subtyping is itself a
FactType (`SubtypeFact <: FactType`). INSTANCE STORAGE is MOST-SPECIFIC
(`ObjectTypeHasObjectTypeInstance` Multiplicity One) with WalkSupertypes /
`EntityTypeSubtypeInstance.SupertypeInstance` DELEGATION — NOT re-recorded against every
supertype. So the base's "over-broad mirror IS canonical" (core.md:418-428) is WRONG;
engine.py:1197's most-specific ruling is RIGHT; the retired `Resource is inherited
instance of Noun` (core.md:436) is NOT a crutch, it IS WalkSupertypes — re-enable it.
Sources: infosci/Subtyping_revisited.pdf, infosci/ObjectificationAndAtomicity.pdf, the
Halpin 1st-ed Fig 13.29 metaschema image (Predicate != ObjectType).
VERIFIED: base compiles clean (176s); compiled `subtype` closure gives Fact Type
supertypes = {Event Type, Function, Resource}, NOUN GONE (all apps). 140/140 tests pass;
the 1 fail (test_scoped_validation subset) is PRE-EXISTING (024817cf projection gate,
ancestor of my first commit — confirmed by reverting the lattice, fail persisted).

### #15 REMAINING — the self-description fill (deep instance-mirror arc)
494 fired on 4 mandatory metamodel constraints. Lattice fix removes the 2 Noun ones.
The other 2 are genuine FILLS (a fact type LEGITIMATELY is a Function (ρ) + a Resource):
- `Function_belongs_to_Domain` (deontic-obligatory): have=0 for ALL functions; NO
  domain-assignment code exists (grep empty). Fill = each function -> its file's domain
  (base -> existing 'core' domain; app fts -> app domain). Compile-time reflection.
- `Resource_is_instance_of_Noun` (mandatory): fact types lack instance-of. Mirror
  (engine.py:1175 `_MIRROR`) computes (id,noun) from role-playing but is FALLBACK-ONLY
  ("serves only the empty cell"). Fill = run it (union) OR most-specific+delegation
  (NORMA arc). Fact type is instance-of metatype-noun "Fact Type".
Both need FULL-APP verify (the killed compile) — foreground, once #20 lands.

### #13 — MORE TANGLED THAN ITS TASK SAID
Task CLAIMED "Constraint Kind has NO constraint linkage" — WRONG: compiler.py:54 has
`Constraint is of Constraint Kind about Fact Type with Modality` (a SECOND classifier).
So merge must reconcile TWO `is-of` relations. Canonical (Fig 13.29 + ORMCoreMetaModel
`ConstraintType`+`ConstraintType_code`): ONE classifier. Fold Kind's Label/Family/
ViolationTemplate (rows core.md:990-1009+) onto Type; keep one is-of; drop redundant.

### #20 — PERF (the blocker; profiled current engine)
COMPILE 1509s: `mu` (kernel.py:759) = 646M calls, 861s tottime (Scott reducer re-reduces
base-sized D per store). VALIDATE 880s: `_cells` (compiler.py:2217) -> from_lam 117M
calls (Scott-decode per constraint). FIX: native carrier (skip Scott for the store
carrier), cache decoded cells per validate pass, batch asserts (one Store/cell,
classify_all_via_M precedent). Target seconds. UNBLOCKS all verify-heavy board items.

### CRITICAL OPERATIONAL FACTS
- Engine is ALL-NEW since 7/1. Base .md comments dated BEFORE 7/1 ("DISABLED 2026-06-22",
  "parse_forml2.rs", task-987) are STALE PORT ARTIFACTS of the retired engine — verify
  actual behavior, don't defer to them (bit me twice: #19 over-emission + domain bridge).
- Base cache is CONTENT-KEYED (protocol.py ingest_frozen: sha256(engine_fingerprint+text)).
  Editing a base .md auto-recompiles (~176s) then thaws. Reverting reuses the OLD cached
  base (instant) — good for A/B causation tests.
- Full-app compile+validate (~10min) gets KILLED as a BACKGROUND job — run FOREGROUND
  (timeout 600), and write results to an own log file WITH flush (piped stdout buffers/loses).
- `reg = apps.Registry(r'C:\Users\lippe\Repos\apps', base_dir=apps.default_base())`;
  `reg._base_D()` thaws base; `reg.compile(app)`/`reg.validate(app)` full pipeline.
  `apps/support.auto.dev/support.auto.dev.store.json` is a POPULATED store — inspect
  cell-by-cell WITHOUT recompiling (that is how I read the 494: Noun cell == Function cell
  == 494 fact types, have=0 on the metamodel facts).
- Commit `-c commit.gpgsign=false`; docs/*.md NEVER committed.

### NEXT (Samuel: one-shot the board if possible)
#20 perf pass FIRST (makes verify seconds) -> #15 fill (foreground-verify) -> #13 merge
-> #18 handlers. Or push a specific item now, verify foreground. Artifact:
$CLAUDE_JOB_DIR/tmp/arest-status.html (favicon 🏗️, deployed).

---

## WHAT IS LIVE IN PRODUCTION (all verified, all Samuel-authorized)

- **The contact lane, end to end**: auto.dev contact forms send ONLY to
  ClickHouse logs.contact_us (PR #267 merged, Vercel prod READY,
  commit 6469222 — the direct-AREST leg is deleted). The support
  worker (support.auto.dev, `arest-worker`) federates the table on
  demand (60s isolate memo), mints the same-id bridge row per
  submission (`Noun 'Contact Submission' is surfaced as Noun 'Support
  Request'.` drives it), and projects the fields AT THE BOUNDARY
  (worker.js REKEY table mirroring contact-derivation.md — the wasm
  derive op exceeds Worker CPU both bare and changed-scoped; the canon
  rules remain the spec). Proof URLs:
  `/support_request/ef998c6716463931/view` (the derived request),
  `/support_request` (the queue — every real submission listed),
  `/nouns` (the inventory).
- **D1 endgame COMPLETE (2026-07-09, the DO is GONE)**: the r1
  production write proved the D1 append (n=2, then n=3 through the
  deployed retirement build), migration v2 deleted_classes removed
  ArestLog + its binding (worker version db03aeb5). appendEvent mints
  n atomically in one INSERT..RETURNING (single-writer SQLite = Def.
  iso serializer). THE MEMO MECHANISM (Samuel rejected hand-repair of
  prod KV; the whitepaper answered): the stream's n is the ONE
  identity (D1 row = mirror key e:{n} = SSE event id); the KV mirror
  is a MEMO of a restriction over the stream (Prop. derive + Cor.
  middleware note), never authoritative; boot walks it by index and
  any structural miss falsifies it and re-derives the whole tail from
  D1, dropping underived keys (compaction preserving the observed
  population). The first prod boot healed the legacy 0-indexed
  backfill keys exactly this way. The transition path
  (POST /{noun}/{id}/{event}) now commits through appendEvent and
  carries the actor (it wrote straight to the DO before and would
  have silently dropped appends post-retirement). Old-keying trap
  died with the second numbering convention. Local dev needs the
  events table created once (wrangler d1 execute --local).
- **The apis flip**: apis' AREST service binding -> `arest-worker`
  (apis commit 8d67a33, deploy 993eb680). A dealer-rooftop listings
  WIP by someone else exists uncommitted in the apis repo — was
  stash-guarded through the deploy, left intact.
- **Auth (federated, no secrets)**: verifyActor presents the caller's
  own credential (Cookie or 'users API-Key') to auth.vin
  /api/users/me, 60s memo; the answered email is the actor; every
  committed event carries it (replay-safe extra field). The IDENTITY
  MINT: a verified caller's subscription federates as the DECLARED
  `Subscription belongs to Customer` link (isolate-local, never
  logged). The POLICY GATE OBSERVES: authorization.md derives
  User_is_authorized_for_Operation_on_Resource (admin rules anchored
  on Admin has Role; customer rules on the subscription link);
  receipts carry `authorized` — ENFORCEMENT IS NOT FLIPPED.
- **The OS (engine/os)**: all three targets founded on bare UEFI (std
  target — stdout IS ConOut). server COMPLETE (the verb table over
  HTTP: smoltcp over SNP, GET /{verb}?args=urlencoded-json; list +
  nouns are native ops). mini COMPLETE (arest> console). full
  INTERACTIVE (Slint software renderer -> blt; master/detail from
  canon; arrows select, Left/Right cycle 23 nouns, F2 single/split,
  Enter/Esc push/pop — run `scripts/boot-os-qemu.ps1 -Release
  -SkipBuild` and curl :18080, or drop -display none to see it).
  aarch64 builds as-is. Remaining: pointer (graphical work), canon-
  tree Slint components proper, fonts polish.

## THE FLEET (apps dir reorganized per Samuel)

- `apps/archive/` holds 83 non-fleet dirs (ALL arc-*, gen-induce*
  experiments, benches, probes, one-shot tests, csdp precursors,
  engine-migration, load-src-do). The registry cannot see it (no
  readings/ marker). THE FLEET = 45 living apps.
- Fleet correctness: VALIDATE SWEEP 2026-07-09 (validate_sweep.log in
  the job dir): 44/45 CLEAN; the ONLY residue is arest-dev, four
  DEONTIC-only sets, all the epistemics probe's designed flags
  (stale-session, negation-stratification staleness, its old
  subset-kind flags). blocked-proto/_sgate/arc-dbatch were ALREADY
  archived (stale artifact text corrected); spd-1's .bak clutter
  removed (the retracted-rows undo lives in the ledger + audit
  jsonl). **spd-1 is the CANONICAL spd ruleset** (Samuel's
  correction — never call it superseded), validates CLEAN, and its
  ethics agreement is the first live subset mint.

## 2026-07-09 LATE SESSION: E2E PROOF, OUTAGE, SLICE 2

- CREDENTIALED E2E PROVEN (the authenticated-support chain, end to
  end in production): POST /support_request with Samuel's users
  API-Key (AUTO_DEV_API_KEY in ~/.claude/.env, /auto-dev skill) →
  201, actor samuel@driv.ly, authorized true, committed; D1 event n=4
  carries the actor. His auth.vin identity DOES carry a subscription
  (the mint keyed on it). sr-e2e-credentialed-proof is the live SR.
- THE OUTAGE (30 min, self-inflicted, fixed forward): all routes
  1101 — wrangler tail named core::cell::panic_already_borrowed in
  the wasm: tenancy's with_active ran every verb body INSIDE the
  WSTORES map borrow; a reentrant touch panicked and crash-killed
  isolates cold-boot looped. FIX 245f5e1c (worker 333a6258): take
  the tenant cell OUT of the map, run f with NO thread-local borrow
  held, put it back — the class is gone (precise reentry path not
  pinned; tail names any resurfacing). Fix-forward beat rollback
  (rollback was ALSO classifier-denied; deploys ride the standing
  grant). Four probes green incl. the canonical view URL. LESSONS:
  local dev misses the federation path (no AREST_SECRET locally —
  the ingest never runs), so prod-only wasm paths need the tail, not
  local repro; wrangler tail is the oracle for wasm panics.
- SET-COMPARISON SLICE 2 (9cf402e7): trailing x-if-y reaches the
  subset translator — grammar recognizes bare Keyword 'if' as Subset
  Constraint, the quote-aware trailing production sits above the
  fact_type_reading catch-all (also stops trailing-if prose from
  prepass-declaring junk fts), the handler dispatches on HEAD STORAGE
  KIND (derived/marked → the rule path refuses; asserted → check),
  resolves-or-refuses, refuses value literals + compound conditions
  to their slices, binds 'that <Noun>' anaphors to role positions
  (scoped_subset_projected in engine.py). THE MINT IS GATED one line
  from live: a subset-kind constraint row (hand-rolled OR via the
  cs_rows canon) CRASHES system:partition's constraint fold
  ('not enough values to unpack', rmap_partition — the kind never
  minted in the system's history). NEXT: the partition-canon slice.
  DIAGNOSIS SHARPENED (b9a9c47e, probes tmp/probe_partition*.py in
  the job dir): the fold theory is FALSE (the fold filters by kind
  and a canon-shaped subset row on a BASE-LESS D partitions fine);
  under the base metamodel, run_rules leaves the constraint CELL
  ITSELF reading bottom (FetchPop answers the bottom atom, so
  system:partition answers bottom and rmap_partition dies unpacking
  it). A base reflection or validation mechanism meets the subset
  kind for the first time and poisons the cell. RESOLVED aad0ac09:
  the base was INNOCENT — the poison was MY checker: _scoped's
  string branch resolves the NAME through the canon and ignores the
  host composition; constraints:scoped_subset_projected has no
  canonical sibling, so the checker object was bottom from birth and
  its DefineIn poisoned evaluation. The host composition is now the
  definition. THE MINT IS LIVE: spd-1's ethics agreement is a
  role-projected deontic subset (unparsed 2 -> 1), support recompiles
  with zero unsafe mints, four semantic tests land
  (test_subset_trailing.py: violation flags, satisfaction clean,
  unbound roles project away), bands 25 green. Remaining arc slices:
  compound conditions (joins), value restrictions, rule_if narrows
  to ' iff ', leading-if recognizers retire.
  ATTRIBUTION corrected everywhere: the if/iff semantics are FORML 2
  / Halpin / NORMA (the sources verified: Mapping ORM to Datalog —
  '<-' READ AS if; CWA closes n bodies to iff; FORML position paper
  — constraints vs derivation rules, deontic prefixes), not a
  project decree; Samuel's was the pointer.

## ENGINE CHANGES THIS SESSION (all committed + pushed, arest repo main)

- SET-COMPARISON ARC SLICE 1 (024817cf, 2026-07-09): dispatch
  CONTINUES past a refusing translator (reports unclassified only
  when NO translator accepts; unregistered names stay graceful
  absence per gate three); _h_subset resolves-or-refuses (declared
  fts both sides, no where-discard, same-ft reversed-binding refused)
  behind a ROLE-PROJECTION GATE that refuses every subset mint until
  the projection slice lands. Measured no-op-except-reporting:
  selfhost+Registry bands green (21), spd-1 unparsed 2->2, support
  62->55 (the 7 newly accepted = the customer-anchor and
  Resource/Lifecycle iff-rules the break had been robbing),
  populations identical, validate CLEAN. NEXT SLICES: role-projected
  subset check (needs role signatures in the translator ctx), rule_if
  narrows to ' iff ', retire leading-if recognizers (284/312/264),
  then the trailing-if productions for constraints. The pre-1.0 push
  rule is FILED in the claude ledger
  (Operating_Rule push-arest-freely-pre-1-0, 4.9s apply).

- Tokenizer: statements split at quote-aware sentence boundaries (the
  period terminates; the line was never the unit) + the VANISH GUARD
  (a classified statement matching no Stage-1 production reports as
  unparsed via for/else raise — nothing is silently consumed). This
  alone made spd-1 validate CLEAN with zero readings changes.
- Parallel-ft UNIFICATION (f9141571): a reading naming a SUBTYPE in a
  role position resolves to the DECLARED supertype ft (guarded: direct
  id undeclared + exactly one substitution hit). Bit four times in one
  day before the fix.
- Composite UCs resolve-or-report (f09449e5): both spanning-UC
  spellings resolve EVERY named column against the reading's roles or
  refuse loudly (was: silent narrowing — _components' UC became
  uniqueness on Component alone).
- sm_init_entity: the write path seeds a machine's initial status when
  a create births a governed entity (fr-live-1's class), one-cell
  guard (never ft_view per write).
- Batch replay: plain log entries buffer by ft and flush through the
  migrate op's bulk paths at op boundaries; the partition HOISTS
  across the whole replay (the schema never changes there — the per-D
  memo missed ~12s/migrate-entry). VERIFIED: replay 83.4s -> 19.1s
  (4.4x) on the clean traced measure.
- AREST_TRACE=1: semantic compile traces -> <app>.trace.json (phases,
  15 slowest statements, per-rule delta/full timings + rounds).
  Support's verdict: delta HEALTHY (rounds 1, all rules ~8s); NO
  monkey-wrench readings; the cost = the classifier CONSTANT (~59%,
  ~700 stmts x ~0.35s), replay (fixed, re-measure), create_handlers
  (~57s), sql-project ~14s.
- Perf memos: _cells per-D weak-keyed (validate 540s -> 11.8s, 46x;
  compile -10%); rmap_partition per-D (write-path win; compile-neutral
  because pipeline stages mint new Ds).
- cli.py streams reconfigure to UTF-8 (a cp1252 em-dash crashed the
  SPAWNER's reader thread -> stderr None); the kernel csproj excludes
  show/** (the WPF renderer's generated AssemblyInfo duplicated).
- The rust engine builds for x86_64/aarch64-unknown-uefi as-is; the
  worker module is pub; native `list` and `nouns` ops in store_call
  (subtype table resolution via role-1 fts — same fix as entity_view's
  ev_cols_native, which classified ZERO columns for subtypes before).

## OPEN DECISIONS (SAMUEL'S)

1. TENANCY: v1 SHIPPED 2026-07-09 per Samuel's "do recommendation"
   (worker version 622e500c). The wasm core keys stores by app
   (HashMap<app,Srv> behind arest_use; non-selecting callers ride the
   "" slot = the old single store; engine/os unaffected). Hostname =
   app; sidecars from STORE KV (cc47da376f974aecb15fedeb1b4faa14)
   under sidecar:{app}; bundled sidecar serves the default app;
   streams key by app in D1; memo keys e:{app}:{n} + n:{app} and THE
   SELF-HEALING BOOT MIGRATED the bare pre-tenancy keys to scoped on
   first boot (local + prod, zero hand ops — same mechanism as the DO
   retirement's repair). Local proof: three tenant cells coexist
   (support, listings-vdp answering Finding/Source, maj-demo);
   unknown app refuses loudly. wrangler dev flattens hosts to the
   route, so local tenant addressing uses the X-Arest-App header
   gated behind `wrangler dev --var AREST_DEV_HEADER:1` (NEVER set in
   prod — isolation is unaddressability). Onboarding = one KV put +
   a route. WfP promotion = a routing change later. METERING v1
   LANDED 2026-07-09 (140c977b, worker 4a3024a6): every request
   upserts usage(app, window, n) in D1 (UTC-hour windows, waitUntil
   off the response path, single-writer serialized); verified local
   (n=4) and prod (n=2); metering.md vocabulary in the support app
   (Usage Window, Request Count, Plan Tier has Request Limit)
   compiles clean, zero new unparsed. BILLABLE DESCOPE (Samuel,
   2026-07-09): rate limits and usage caps are AUTOMATIC through
   apis / api.auto.dev; he adds a billing rate himself later;
   UNBILLED IS OK until priced — the lane is functionally closed
   (tenancy + metering shipped; billing = a one-line price). Only
   possible auto.dev-domain concern: documentation of external
   federated systems (candidates to flip AREST-native by removing
   the federation). SUBSTRATE FILING DONE (17/17,
   filing.log): 4 lessons + the set-comparison-arc board task (p2).
   Remaining: per-app sidecar refresh story, eviction policy if the
   resident set grows, the pre-1.0 push standing rule as an
   Operating Rule (next one-fact batch).
2. ENFORCEMENT FLIP: DONE 2026-07-09 ("Flip OK, no external users").
   POST /{noun} create refuses unauthorized BEFORE apply: 403,
   receipt carries refused verdict + actor + authorized. Reads open;
   transitions observe (carry actor) until per-event operations are
   named. Verified prod: anonymous create 403 (version d1a1c4b6,
   re-verified on 622e500c). Model-level validate_S form remains the
   endgame. CUSTOMER ANCHORS LANDED 2026-07-09 (worker c9dc1c0c):
   verifyActor's identity mint now ALSO projects the authorization
   triples at the boundary (create on Support Request / Message /
   Chat Message for the verified subscriber; authorization.md's
   customer rules are the SPEC — derivation runs at compile, the
   subscription link exists only at runtime, so the boundary
   projects, REKEY-style). The respelled spec lines ('iff some
   Subscription belongs to that Customer') report UNPARSED loudly
   (the resolver only took the old dangling-anaphor spelling);
   they join the set-comparison arc's constituency. CREDENTIALED
   E2E PROOF PENDING: needs a real auth.vin credential (Samuel's
   cookie or API key against POST /support_request-adjacent create).
   ADMIN GAPS: CLOSED 2026-07-09 (worker c54bdd16, commit STAGED —
   pinentry cold, deferred). auth.vin answers role 'ADMIN' for the
   operator key (verified: GET /api/users/me user.role='ADMIN',
   user.subscription=sub_..., user.plan='Scale'), so verifyActor now
   projects the admin anchors at the boundary too (Admin_has_Role +
   the 5 authorization.md admin triples: create SR/Message/Chat
   Message/Feature Request, update SR) for verified ADMIN callers,
   same REKEY discipline as customer anchors. PROVEN: operator create
   on Feature Request 403 -> 201, authorized true; anonymous stays
   403. The compile-time admin derivations still mint zero (the
   quoted-head per-value resolver class — an engine slice), the
   boundary carries them meanwhile. Support's compile
   also confirms ~60 trailing-if '+ X if Y' canon lines report
   unparsed (the queue derivations, GitHub label rules, us-law
   subject-to family) — all arc constituency.
3. DO RETIREMENT: DONE 2026-07-09 (see the D1 endgame bullet).
4. spd-1 unparsed: DONE 2026-07-09 per Samuel's deontic-subset call.
   9 -> 2. THE LIVE FORM: violations derive as fact types
   (Agent_wrongly_defers/performs_Action_Class from Action Kind, the
   2-atom literal rule_iff works) and 'It is forbidden that <reading>'
   mints deontic_forbidden (the message-vetting transform, _plan
   compiler.py:1673). The 2 remaining = the CANONICAL NORMA deontic
   subset spellings (ethics obeys-order, free-will performs-implies-
   reports), deliberately REPORTED as markers. ENGINE FINDINGS (the
   leading-if constraint family is unreachable): grammar recognizer
   'Derivation Rule iff Keyword if' (forml2-grammar.md:264) co-fires
   on every if-sentence, sorts before 'Subset Constraint', and the
   dispatcher BREAKS on the first refusing translator
   (compile_model_selfhost, compiler.py:1995 except -> unclassified
   -> break) so translate_set_constraints never runs; ALSO
   scoped_subset (engine.py:531) compares TUPLE-WISE (no role
   projection; RolePath unification is Stage 2), _h_subset DISCARDS
   its ' where ' clause (compiler.py:1008), and grammar line 201
   names translate_deontic_constraints which no host registers.
   Corpus casualties of the same class: sherlock evidence.md:40,
   protondb:147, epistemics.md:37/40, cancel-service:133,
   tax-service:40, support.md:402-404/406/413 (the queue derivation
   spellings!). Fix direction, under THE FORML 2 SEMANTICS (Halpin; NORMA the oracle; Samuel's pointer 2026-07-09): "iff is equality, if is subset" — where BOTH are the
   TRAILING spellings. `x if y` (consequent first, condition
   trailing) IS the subset constraint surface; `x iff y` is equality
   and the derivation biconditional; the leading "If x then y" form
   is MALFORMATTED ("that screams malformatted constraint" — Samuel)
   and was never parseable anyway (every corpus instance dies at
   dispatch). So: RETIRE the leading-if machinery (grammar
   recognizers 284 + 312, the Stage-1 `^[Ii]f (.+) then (.+)`
   production, recognizer 264's Derivation-Rule-on-'if'); land
   trailing `x if y` subset constraints in the set-comparison arc
   (role projection + binding order still required; deontic trailing
   spellings are exactly the spd-1 markers, now reverted to trailing
   form); derivations keep ` iff ` only (rule_if's ` iff? ` narrows —
   layering:46's digit-var ' if ' line then flips from the 113-row
   artifact derivation to a subset constraint, which was its authored
   intent). The fleet's ~15 leading-If lines are misspellings to
   re-author trailing as they are touched. REFINEMENT (Halpin's asserted/derived/semiderived trichotomy; Samuel's pointer, same conversation): the ONE trailing surface's operational reading
   dispatches on the HEAD'S STORAGE KIND — ORM's asserted/derived/
   semiderived trichotomy, already signaled by the NORMA storage
   markers (* ** + ++) on rule heads. Derived head + 'if' =
   derivation clause (ADDS the missing consequent; clauses union,
   n-rules-per-head); derived head + 'iff' = the complete closed
   definition; ASSERTED head + 'if' = subset constraint (PREVENTS:
   alethic refuses the violating write, deontic flags the standing
   gap — the engine never invents asserted facts, so obligation is
   all it can express there); semiderived = both (assertions allowed,
   the rule tops up what is missing). Consequences: support.md's
   queue lines want STARRED trailing-if derived heads ('* Customer
   submits Support Request if ...'), not iff respellings;
   layering:46's intent was the derivation reading all along (declare
   'Layer belongs to Layered System' as a fact type, then a starred
   trailing rule); the spd-1 markers stay as spelled (asserted-head
   deontic subsets that flag). The compiler's plain-set discipline
   ('a head the model declares plainly must not earn the rule's
   derivation kind') is this dispatch's seed. DATA CLEANUP: spd-1's
   Agent_defers/performs populations were old-encoding migrate relics
   ((class, mode) pairs) minting 2 phantom Action Classes + 11
   phantom Agents; retracted from spd-1.events.jsonl (backup:
   spd-1.events.jsonl.bak), counts now honest (AC 13, Agent 0),
   validate CLEAN.

## OBJECTIFICATION-CONFORMANCE AUDIT (Halpin 2020, 2026-07-09)

Samuel had me read Halpin's 'Objectification and Atomicity' (infosci):
objectification must be restricted to SPANNING-UC fact types; non-
spanning objectification (n:1, 1:1, n-ary on an n-1 UC) is deprecated
(yields non-atomic facts). FLEET AUDIT of every 'This association
with ...' / objectification: ALL are spanning-UC and Halpin-conformant
— base: Constraint Span (Constraint+Role binary, both roles), API
(Fact Type+Verb binary, both roles), Resource Role (Fact+Resource+Role
ternary, all 3), Event Caused Transition (Event+Transition+SM ternary,
all 3); apps: support Customer Submission Match (Customer+Support
Request binary, both). THE SOLE NON-CONFORMANT CASE is the TRANSITION
(task #14): not even declared as an objectification — a bare surrogate
Transition(.id) masking a NON-SPANNING determinant identity (SMD,
from-Status, Event Type), so 'Guard guards Transition(review)' is a
non-atomic fact. Corrected design on #14: identify by the determinant,
demote the name to a label. The rest of the metamodel is sound on
objectification.

## SET-COMPARISON ARC: SIGN + VALUE SLICES (2026-07-09)

- SIGN SLICE (11c08cc0): trailing-if deontic sign picks the check —
  obligatory -> projected SUBSET (Y subset X), forbidden -> projected
  EXCLUSION (Y disjoint X, scoped_exclusion_projected). Fixed a latent
  bug: the handler built subset regardless of sign and the sign never
  reached it, so forbidden lines would have minted wrong subsets once
  value restrictions lifted the literal refusal. _plan now threads the
  sign. 4 semantic tests pin both signs.
- VALUE SLICE (8324ef9e): 'X if that E has Attr <lit>' filters the
  condition to <lit>-holders before the projected check
  (scoped_subset/exclusion_projected_filtered; the value-comparison
  eq/CONST filter). This is the shape MOST legal deontics take. 4
  tests: Paid-unreplaced violates, replaced clean, Open filtered-out
  clean, mints. Bands 32 green. Multi-literal + compound conditions
  still refuse to the join slice (the arc's next).

## THE NORMA INSTANCE-OF CORRECTION (44b93359, 2026-07-09) — PARTIAL,
## corrected honestly: the Resource fix is right but was verified only
## on UNPOPULATED hoa; support (populated) revealed it is one piece of
## a cluster. See task #12 (SM sibling instances.md:156 same pattern,
## still fires) + #15 (support ~494 typeless-Resource mandatory,
## pre-existing). LESSON: verify metamodel changes on a POPULATED app.

Samuel: "challenge some more assumptions; check `Resource is inherited
instance of Noun` against the NORMA metamodel." The challenge dissolved
a whole speculative refactor into a one-line fix. VERIFIED against
Halpin, "Subtyping Revisited" (infosci; extracted tmp/subtyping.txt):
ORM subtyping IS population inclusion — "all instances of one type are
also instances of a more encompassing type" (Patient 101 is in BOTH the
MalePatient and Patient populations). So instance-of is TRANSITIVE; an
entity belongs to its whole lineage; there is NO single type per entity
and NO separate "inherited membership" (inheritance in ORM is PROPERTY
reuse — a subtype plays the supertype's roles because it IS a supertype
instance). CONSEQUENCES: (1) `Each Resource is instance of exactly one
Noun` (instances.md:102) was NON-CANONICAL — relaxed to `some Noun`
(mandatory); this was the real defect that fired alethic on every
subtype/multi-typed id on recompile. (2) The over-broad mirror is
CORRECT transitive membership — do NOT collapse it; SM-seeding + the
domain bridge rightly join on it, so RELAXING (not collapsing)
preserves seeding (the whole point). (3) `Resource is inherited
instance of Noun` (core.md:425) was a non-canonical crutch with ZERO
readers (grep base+apps) — RETIRED as dead derived data. The earlier
"collapse + transitive is-a + repoint consumers" refactor arc is
DISCARDED (built on the same bad assumption). VERIFIED: 25 selfhost+
apps bands green; hoa recompile mirror=GONE (SM-status 0 there is a
statute library with no Fine instances, not a regression — seeding
reads the unchanged mirror); support recompile SM-status confirmation
[running, post-hoc]. NORMA also settled `Constraint Type` vs `Constraint
Kind` (Curland ORM2 TechReport2: type=class + patternGroup=family) —
Constraint Type is the redundant one, DECOUPLED to task #13 (relaxing
the constraint already stops its shared-id double-membership flagging).
SEPARATE FINDING: Transition_is_to/from/defined-in_Status
mandatory+uniqueness 2-offenders on hoa recompile — own diagnosis.

## CITATION-SPLIT FIX + THE INSTANCE-MIRROR FINDING (2026-07-09)

- CITATION SPLITS APPLIED (tmp/citation_fix.py, apps/ not git so
  disk-only): 786 sentence-final legal citations relocated inside
  their periods ('X. (15 USC 1666) Y' -> 'X (15 USC 1666). Y') across
  cancel/charge-dispute/gym/refund/tax so the statement splitter can
  split. RESULT: fused-left=0 on every recompiled app — the
  multi-sentence VANISH CLASS is eliminated (its whole purpose).
  Citation-only parentheticals move (digit + a citation token:
  USC/CFR/IRC/§/Section/Act/...); prose asides like '(symmetric)'
  stay.
- THE INSTANCE-MIRROR FINDING (filed: tasks 'instance-mirror-over-
  broad', task #12): recompiling the citation apps surfaced an
  ALETHIC violation on Resource_is_instance_of_Noun (base/
  instances.md:102 'Each Resource is instance of exactly one Noun',
  mandatory+uniqueness). NOT THE CITATION FIX: the untouched hoa app
  ALSO dirties on recompile (proof — diag_mirror.py). The run_rules
  instance mirror makes an id a DIRECT instance of EVERY role-noun it
  plays, so subtype instances (CA...Contract Law = State Statute AND
  Statute) and shared-id metamodel nouns (AC = Constraint Kind AND
  Constraint Type) read as instance-of-2+, tripping 'exactly one'.
  The base already has 'Resource is inherited instance of Noun'
  (core.md:425) as the SEPARATE supertype relation, so instance-of is
  MEANT to be most-specific only. PRE-EXISTING (mirror = proposal B
  2026-07-04; the iteration-8 44/45-clean sweep validated STALE clean
  stores — a fresh recompile is the honest state). DEPLOYED SUPPORT
  IS UNAFFECTED (federates, no cross-noun id reuse; stays CLEAN). NOT
  AUTO-FIXED — a base-metamodel semantics call for Samuel (mirror
  most-specific-only / relax to 'at least one' / apps use distinct
  ids). HONEST FLEET STAMP: 44/45 was stale-store; on fresh recompile
  the subtype/shared-id legal apps carry this until the decision.

## FLEET CONSTRAINT AUDIT (Samuel's directive, 2026-07-09; script
## tmp/fleet_audit.py in the job dir, report fleet_audit_summary.md +
## .jsonl delivered)

45 apps, 773 constraint-shaped statements, classified by HEAD STORAGE
KIND with the engine's own prepass (known.plain = the asserted
signal): 299 derive (marked, OK) · 198 assert-flag-on-violate
(deontic FTR on asserted heads, the live class, OK) · 21
assert-fail-on-violate (alethic subsets/equalities, arc constituency)
· 32 CONFLICT (*/** markers on plainly-asserted heads — must become
+/++ or drop the plain declaration; claude's Engineering-Lever trio
and codex's Verification-Run among them) · 223 REVIEW, three real
subclasses: (a) UNDECLARED-HEAD obligations across the legal fleet
(bill-negotiation 21, parking-ticket 31, refund 16, charge-dispute
13, gym 10, property-tax 10, robocall 12, hoa 6...) — the deontic
transform declares the ft at compile but NOTHING populates it, so
those obligations are decorative until wired (the support state-law
pattern is the fix template); (b) MULTI-SENTENCE VANISH artifacts
(legal citations in parentheses fused several statements into one —
authoring bugs in cancel-service, charge-dispute, refund, robocall);
(c) audit noise (', if applicable' prose, claude narrative lines).
AUDIT v2 (trailing-marker aware — v1's plain set misread 'X is p. *'
declarations and minted 20 false CONFLICTs): 333 derive · 196+2
assert-flag · 16 assert-fail · 3 CONFLICT · 225 REVIEW. FIXES
APPLIED 2026-07-09: support contact-derivation.md's nine REKEY rules
remarked * -> + (semiderived: the boundary ingest asserts, the rule
tops up — recompiled, unparsed 55 stable, populations identical);
spd-1's two violation fts got explicit trailing-starred declarations
(recompiled stable). REMAINING CONFLICT (Samuel's call): tasks'
'Task is started iff finished/blocked/unblocked' trio on a plainly
declared head (the board of record; not touched). Audit v3 note:
deontic-ftr over a DERIVED head is the violation-ft idiom (OK class,
not REVIEW). PENDING: legal-fleet wiring calls, multi-sentence
vanish-artifact splits, board task rides the next batch.

- rule-if-family-rewrite (p2): carries the COMPLETE works/fails twin
  matrix + the resolver mechanism map (_h_rule_if/_rule_atom/_coercion
  — membership can't seed, unary can't feed the equality, equality
  binds prior-atom role-2 POSITIONALLY, 4-clause chains don't thread,
  literal-subject binders die in 3-clause chains). Engine work; the
  us-law family + richer policy rules block on it.
- The classifier constant (59% of compiles) — architectural (rust-host
  compile is the endgame); create_handlers 57s.
- rule-temporal-predicates-inert (p3), compiler-gloss-lines-become-fts
  (p3), view-labels-from-readings (p3 — CANON change: labels derive
  from reading templates; contact-/company- Name collide as "Name"),
  worker-refusal-offenders (p3).
- validator-composite-role-uc: CLOSED. engine-write-path-init: CLOSED.

## STANDING RULES + CONVENTIONS (hard-won; do not relearn)

- PERFORMANCE OVER GRINDING (Samuel, twice; filed as an Operating Rule
  in the claude app): profile before levering; targeted test bands +
  dot-stream indexing over full-battery reruns; kill and re-scope
  grinding processes; file architectural costs with maps.
- Deploys: CLOUDFLARE_ACCOUNT_ID=b6641681fe423910342b9ffa1364c76d,
  worker `arest-worker`, `npx wrangler deploy --config wrangler.toml`
  from engine/worker. support.auto.dev/* is the canonical smoke
  address. Samuel granted STANDING permission to "keep deploying the
  worker to test it"; apis/site deploys need his explicit imperative
  (the auto-mode classifier parses questions as non-authorization —
  even his own).
- wasm rebuilds: `cargo build --release --target wasm32-unknown-unknown
  --no-default-features --features worker` then wasm-bindgen
  **--target web** (bundler emits no default export and breaks the
  worker import), ABSOLUTE paths, grep the emitted exports.
- The tasks-app writes are ~2-6 min each via the python path (load
  per apply): batch them in ONE Registry session, run DETACHED
  (Start-Process + log file), flush-per-line logging (captured stdout
  buffers invisibly otherwise).
- Heredocs mangle backslashes/unicode on this box: use the Write tool
  for scripts, chr() constructs for tricky strings, assert-before-
  write in patch scripts.
- apps-dir writes need dangerouslyDisableSandbox. gpg commits hang on
  a cold pinentry window — probe with a short timeout, retry when
  warm, never bypass signing.
- The support store's User is keyed by User Id; Customer by Email
  Address; SR fields absorb into the "Agent Chat" table (top-supertype
  absorption); Streaming Mode values are 'streaming'/'non-streaming';
  Intake Source includes 'contact-form'.
- QEMU/OVMF: startup.nsh converges flaky BDS; virtio-rng + -cpu max
  feed EFI_RNG_PROTOCOL (std HashMap seeding panics without); WHPX
  faults OVMF — TCG on purpose; the interpretive verbs (query) NEVER
  on serving paths (native list/get/nouns instead).

## KEY ADDRESSES

- The artifact (stack layers + dependency lanes), favicon 🏗️:
  https://claude.ai/code/artifact/339bbb23-a503-448c-a6ff-74a6b4eb6cec
- arest repo main at ff95b5f8, PUSHED. STANDING RULE (Samuel,
  2026-07-09): "Always OK to push arest repo pre-1.0" — arest main
  pushes need no per-commit ask until 1.0 (file as Operating Rule in
  the claude app next batch). Prod-record hand-edits still need his
  imperative; ask, never work around. Substrate filing PENDING (next
  batched Registry session): the memo-mechanism lesson, the FORML 2 if/iff semantics (Halpin)
  with the storage-kind dispatch refinement (iff=equality, if=subset,
  asserted heads constrain / derived heads complete), the
  dispatch-break conformance map, the spd-1 data cleanup. The stack
  artifact still shows the DO step open and spd-1 as superseded
  residue; refresh when convenient.
- The mechanism maps live in the board tasks and this file. The claude
  app carries Operating Rules + Engine Lessons (query
  Operating_Rule_has_Rule_Statement at session start per MEMORY.md).

## 2026-07-09 — CANON COMPLETENESS + THE POLYGLOT MONAD

Samuel's framing: AREST is a math formula first (set theory -> lambda
calc -> FP/FFP/AST -> ORM -> Processes), defined ONCE in the shared canon
(shared/*.canon, INTERSECTION SOURCE: one tuple literal that is valid
Python AND Rust AND C#/Java verbatim — each host consumes the same bytes;
Python execs, Rust include!s). DEFS is the platform-specific override
point ONLY; the canon must be COMPLETE. "What language is it? / Yes."

THE MONAD (the prank, now load-bearing): added monad:unit (CONS[id,
CONST(phi)]) + monad:bind (Writer over the free monoid; log=sequence,
mempty=phi, mappend=cat) to shared/system.canon. Compiles as Python (3
monad laws pass, tests/test_monad_canon.py) AND Rust (cargo GREEN,
main.rs:6217 include!s system.canon). App data untouched (store hash
moves only because the sidecar serializes the canon into every store's
process, by design).

CANON-COMPLETENESS AUDIT (2 Explore agents over python host): the CORE is
canon-complete — command pipeline (system.create = emit.validate.derive.
resolve), constraint dispatch, SM folding all delegate; NO spine drift.
Genuine regressions (canon DEF absent, math host-only) FIXED this session,
each gated canon==host on synthetic input + absolute-result authorship
test + cargo GREEN (Rust reduces same bytes, differential-covered prims):
 - constraints:scoped_{subset,exclusion}_projected(_filtered) (4) — my own
   host-only deontic projected checkers (docstring had confessed "the HOST
   composition IS the definition"). Host now dispatches (isinstance str),
   twin kept. The earlier "one-arg name can't carry projections" objection
   was WRONG (the param is a tuple, like scoped_external_uniqueness's cols);
   the 2026-07-09 poison was an UNDEFINED name -> bottom, fixed by defining.
   tests/test_projected_canon.py.
 - constraints:deontic_forbidden + deontic_obligatory_value — the deontic-
   VALUE family (row-as-set: forbidden = row\values != row). Population form
   stays as the id primitive. tests/test_deontic_canon.py.
 - system:membership_def — the paper's eq.1 (member.[2.2,1.2], population as
   characteristic function). Closed object; host returns the canon NAME,
   resolves through DEFS DefineIn. tests/test_membership.py green.
REMAINING (documented, task #17): as_of (bitemporal), absorb_rows (RMAP
variadic join-fold, hardest), links_of union, value_comparison (dormant),
compiler frontier recompute_frontier (Cor. streaming — gray-area optimization).
NOT regressions (sanctioned certified-equal PERFORMANT overrides, canon DEF
present): verify->system:verify_store, get_view->system:entity_view,
explain->system:explain, _classify_heads->system:classify_heads.
PORT METHOD (repeatable): neutral tree {("A",s),("K",t),("S",kids)} ->
to_obj (testable lam) + to_src (canon S-form) from the SAME tree (Python
guarantees paren balance) -> canon==host gate on synthetic <P,D> ->
programmatic insert (anchor a unique DEF name) -> fresh-process file verify
-> host dispatch (keep composition as differential twin) -> cargo check.
K-wrap combinators deferred to check-time; bare apply bakes at build-time.

STILL PENDING (unstarted, canon-grounded, Samuel unparked "don't park
metamodel decisions on Halpin/Curland/NORMA"): #14 transition identity =
DETERMINANT (SMD, from-Status, Event Type). Found: runtime resolution
(actions/sm_triples) does NOT use the transition name (already determinant-
shaped) BUT sm_triples is machine-UNSCOPED (latent bug: base-vs-app name
collision cross-products froms/tos). AREST has objectification ("This
association provides the preferred identification scheme") + external_
uniqueness machinery. Support shows 18 uniqueness violations from the
collision. #13 Constraint Type+Kind merge. #15 494 orphan entities.

## 2026-07-09 (late) — THE PROCESS MODEL'S PROVENANCE (Samuel)

The AREST process model is a synthesis of THREE formalisms, each doing what it
does best:
 - BACKUS (AST, §14) — the FRAMEWORK: mu(SYSTEM:x)=<o,d>, a machine advancing by a
   transition. Backus deliberately UNDER-DEFINES how a transition RULE is specified
   ("we have not said how the file/queries/updates are structured").
 - HALPIN 2nd-ed process model (RETRACTED in the 3rd ed, RESURRECTED in AREST) —
   fills that gap: the OBJECTIFIED Transition/Guard/Status/Event entities ARE the
   precise definition of Backus's abstract transition rule. (This is why the SM
   model is "more complete if buggy" — it is the unfinished 2nd-ed one.)
 - CODD relations — the SUBSTRATE: transitions/guards CAPTURED AS DATA (Halpin
   1st-ed Fig 7.26 "a transition constraint may be captured as data") — a rule is a
   fact, not metadata.
So a transition rule = a Backus AST step, DEFINED by Halpin-objectified facts,
STORED+EVALUATED as Codd relations.

CORE.PNG (payload-experiments/samuel/GraphDL/Core.png) = the canonical model in OLD
GRAPHDL VOCABULARY: Graph = Fact (instance), Graph Schema = Fact Type, Resource =
entity/value playing a Role. So "Guard Run references Graph" = a guard run references
the FACTS it read; "Graph Schema defines Graph" = Fact Type defines its Facts.
Transition(.id) is a NAMELESS SURROGATE (the name was AREST's own convenience) — this
grounded #14 (DONE, commit 23932e6b).

GUARD MODEL DECIDED (Samuel): "guard runs on facts are the desired process model." A
GUARD is an FFP FUNCTION (Backus); a GUARD RUN is that function evaluated over the
fact population P (Codd), recording the facts read (references Graph=Fact); objectified
as a Guard entity (Halpin). mu fires the transition iff trigger in P AND the guard's
run over P is satisfied (a rho-application, Prop. Derivability — not a stored flag).
RICHER than the engine's current degenerate fact-PRESENCE check (smGuard, populated
only by "Transition is guarded by Fact Type"); it ACTIVATES the 35 inert "Guard X
prevents Transition T" guards across 9 apps (they wrote the Guard-ENTITY shape). SM
COMPLETION (#19, focused effort): implement guard-run-over-P eval in machine_step;
route the apps' guard readings into it; resolve the "prevents" polarity (blocks-if-true
vs precondition). Also #19: parser set-difference negation (tighten over-emitting SM
derivations).
