# claude-on-kernel MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that the kernel induces the staleness lesson from observation facts — build a `claude-on-kernel` consumer app that maps Engine-Cell behavior facts onto the kernel's I/O and induces `defective ← is-stale-on-mutation` (self-validated `count==count`, held-out predicted), using the existing kernel shapes only.

**Architecture:** `claude-on-kernel` is a consumer of `arest-kernel` (the `file:../kernel` dependency pattern of `apps/maj-demo`). A mapping readings file expresses the staleness class as ONE kernel `Problem` whose single `Problem has Feature 'defective'` is the predicted label; each Engine Cell is an `Observation` that *reads* its property values at input features (`stale`/`rederived`/`authoritative`) and *writes*/`must write` the `defective` label. The kernel's stratified meta-rules + CSDP `count==count` self-validation derive the winning shape (expected: `gather`, `defective ← stale`). No kernel changes.

**Tech Stack:** FORML2 readings (`.md`), `arest-kernel`, the MCP verbs `apps.compile` / `query` / `sql` (local mode).

---

## Background the engineer needs

- The kernel (`apps/kernel/readings/kernel.md`) is a domain-agnostic generator: feed a domain as instance facts mapping onto `Observation reads/writes Value at Feature`; its meta-rules induce which of four shapes (`gather`/`relabel`/`majority`/`lookup`) reproduces ALL training (`count==count`), Sherlock-rank by confidence, select one, predict held-out.
- A consumer app (see `apps/maj-demo/`) is just: a `package.json` with `"arest-kernel": "file:../kernel"`, a `readings/app.md` marker, and a readings file of pure instance facts in the kernel vocabulary.
- KEY shape facts for this plan:
  - `gather`: `Feature1 gathers from Feature2 iff (over every training Observation) the value written at Feature1 equals the value read at Feature2`. **Feature2 (the source) need NOT be in `Problem has Feature`** — only Feature1 (the written/scored feature) must be. This is why classification fits: make `defective` the only `Problem has Feature`; read the inputs at other (non-Problem) features.
  - `relabel` needs a read AND write at the SAME feature, so it cannot apply to the write-only `defective` feature — `gather` is the expected winner.
  - `Problem spans Count` = count of `Problem has Feature`; an Observation `is solved` iff it scores at all spanned features. With exactly one spanned feature (`defective`), scoring reduces to "predicted `defective` correctly."

## File Structure

- Create `apps/claude-on-kernel/package.json` — consumer manifest, depends on `arest-kernel`.
- Create `apps/claude-on-kernel/readings/app.md` — app marker.
- Create `apps/claude-on-kernel/readings/staleness.md` — the staleness Problem mapped onto kernel I/O (the only domain content).

No code; no kernel edits. Verification is `apps.compile` + `query`/`sql` over the kernel's derived facts.

---

## Task 1: Scaffold the consumer app

**Files:**
- Create: `apps/claude-on-kernel/package.json`
- Create: `apps/claude-on-kernel/readings/app.md`

- [ ] **Step 1: Write the package manifest**

`apps/claude-on-kernel/package.json`:
```json
{
  "name": "arest-claude-on-kernel",
  "version": "0.1.0",
  "private": true,
  "description": "Induce ledger Engine Lessons from observation facts via arest-kernel. MVP: the staleness class (a cell is defective iff stale-on-mutation), induced by the gather shape and self-validated count==count.",
  "kind": "app",
  "keywords": ["arest", "forml2", "app", "kernel-test", "ledger", "induction"],
  "license": "MIT",
  "dependencies": {
    "arest-kernel": "file:../kernel"
  }
}
```

- [ ] **Step 2: Write the app marker**

`apps/claude-on-kernel/readings/app.md`:
```markdown
# arest-claude-on-kernel app marker. Loads the generator (arest-kernel) over the
# ledger's engine-behavior observations to induce Engine Lessons. MVP: staleness.

## Instance Facts

App 'claude-on-kernel' has Name 'Claude on Kernel'.
App 'claude-on-kernel' uses Generator 'sqlite'.
```

- [ ] **Step 3: Compile and verify the kernel loads clean**

Run: `apps.compile name=claude-on-kernel` (MCP verb; local mode).
Expected: `compile_result.ok = true`; `active_app`/health `ready`; `apps.status name=claude-on-kernel` shows `dependencies.closure` includes `kernel`. No `rejected:true`.

- [ ] **Step 4: Commit**

```bash
git add apps/claude-on-kernel/package.json apps/claude-on-kernel/readings/app.md
git commit -m "feat(claude-on-kernel): scaffold kernel-consumer app"
```

---

## Task 2: Write the staleness mapping

**Files:**
- Create: `apps/claude-on-kernel/readings/staleness.md`

The dataset uses real ledger Engine Cells. Input read-features: `stale` (= `is-stale-on-mutation`), `rederived` (= re-derived by a disregarding Code Site), `authoritative`. The single Problem feature is `defective`. The negative `Resource_is_currently_in_Status` is `rederived` but NOT `stale` and NOT defective — so the data forces the predictor to be `stale`, not `rederived`.

- [ ] **Step 1: Write the mapping readings**

`apps/claude-on-kernel/readings/staleness.md`:
```markdown
# Staleness class mapped onto the kernel I/O. The ONLY Problem feature is the
# predicted label 'defective'; each Engine Cell reads its property values at the
# input features (stale/rederived/authoritative, which are NOT Problem features)
# and writes the defective label. Expected induction: gather, defective <- stale.

## Instance Facts

Problem 'staleness' has Feature 'defective'.

Problem 'staleness' includes Observation 'Task_has_Task_Status'.
Problem 'staleness' includes Observation 'Task_is_recommended'.
Problem 'staleness' includes Observation 'state_machine_status_id'.
Problem 'staleness' includes Observation 'Resource_is_currently_in_Status'.
Problem 'staleness' includes Observation 'Task_Priority_is_recommended'.

# --- training: known-labelled cells ---
Observation 'Task_has_Task_Status' is training.
Observation 'Task_has_Task_Status' reads Value 'yes' at Feature 'stale'.
Observation 'Task_has_Task_Status' reads Value 'yes' at Feature 'rederived'.
Observation 'Task_has_Task_Status' reads Value 'no' at Feature 'authoritative'.
Observation 'Task_has_Task_Status' writes Value 'yes' at Feature 'defective'.

Observation 'Task_is_recommended' is training.
Observation 'Task_is_recommended' reads Value 'yes' at Feature 'stale'.
Observation 'Task_is_recommended' reads Value 'yes' at Feature 'rederived'.
Observation 'Task_is_recommended' reads Value 'no' at Feature 'authoritative'.
Observation 'Task_is_recommended' writes Value 'yes' at Feature 'defective'.

Observation 'state_machine_status_id' is training.
Observation 'state_machine_status_id' reads Value 'no' at Feature 'stale'.
Observation 'state_machine_status_id' reads Value 'no' at Feature 'rederived'.
Observation 'state_machine_status_id' reads Value 'yes' at Feature 'authoritative'.
Observation 'state_machine_status_id' writes Value 'no' at Feature 'defective'.

Observation 'Resource_is_currently_in_Status' is training.
Observation 'Resource_is_currently_in_Status' reads Value 'no' at Feature 'stale'.
Observation 'Resource_is_currently_in_Status' reads Value 'yes' at Feature 'rederived'.
Observation 'Resource_is_currently_in_Status' reads Value 'yes' at Feature 'authoritative'.
Observation 'Resource_is_currently_in_Status' writes Value 'no' at Feature 'defective'.

# --- held-out: a known stale cell, label withheld as the target ---
Observation 'Task_Priority_is_recommended' reads Value 'yes' at Feature 'stale'.
Observation 'Task_Priority_is_recommended' reads Value 'yes' at Feature 'rederived'.
Observation 'Task_Priority_is_recommended' reads Value 'no' at Feature 'authoritative'.
Observation 'Task_Priority_is_recommended' must write Value 'yes' at Feature 'defective'.
```

- [ ] **Step 2: Compile**

Run: `apps.compile name=claude-on-kernel`
Expected: `compile_result.ok = true`, `rejected:false`. (Resolve any `check Warning Resolve` that names a real unresolved fact type — the kernel FTs like `Observation reads Value at Feature` must match exactly.)

- [ ] **Step 3: Commit**

```bash
git add apps/claude-on-kernel/readings/staleness.md
git commit -m "feat(claude-on-kernel): staleness class mapped onto kernel I/O"
```

---

## Task 3: Verify the induction (count==count + held-out)

**Files:** none (query-only verification).

- [ ] **Step 1: Confirm a shape was selected**

Run: `query app=claude-on-kernel fact_type=Problem_selects_Shape filter={"Problem":"staleness"}`
Expected: one row, `Shape = "gather"` (gather is the consistent cross-feature source; relabel can't read the write-only `defective`; majority/lookup score lower or don't fit). If a different shape is selected but the held-out still predicts correctly (Step 3), that is still a pass — record which shape won.

- [ ] **Step 2: Confirm self-validation (count==count) — the rule reproduces ALL training**

Run: `query app=claude-on-kernel fact_type=Problem_is_gwon filter={"Problem":"staleness"}`
Expected: present (truthy) — `gcount == training total (4)`, i.e. every training cell is reproduced by the induced `defective ← stale` rule, and crucially the negatives (`state_machine_status_id`, `Resource_is_currently_in_Status`) are reproduced as `defective=no` (so the rule did NOT over-fire on `rederived`).

- [ ] **Step 3: Confirm the held-out prediction**

Run: `query app=claude-on-kernel fact_type=Observation_predicts_Value_at_Feature filter={"Observation":"Task_Priority_is_recommended"}`
Expected: `Value="yes"`, `Feature="defective"`.
Then run: `query app=claude-on-kernel fact_type=Observation_is_solved filter={"Observation":"Task_Priority_is_recommended"}`
Expected: present (the held-out cell is solved — predicted `defective=yes` matches `must write`).

- [ ] **Step 4: Confirm the induced source is `stale`, not `rederived` (the discrimination)**

Run: `query app=claude-on-kernel fact_type=Feature_gathers_from_Feature_in_Problem filter={"Problem":"staleness"}`
Expected: `defective gathers from stale`; NOT `defective gathers from rederived` (the `Resource_is_currently_in_Status` negative — `rederived=yes` but `defective=no` — breaks the `rederived` source). This is the substantive result: the kernel induced *"a cell is defective iff it is stale-on-mutation,"* discriminating against the `rederived` confound.

- [ ] **Step 5: Record the verification outcome (no commit — query results)**

If Steps 1-4 pass: the existing kernel shapes induce the staleness lesson; the sufficiency bridge was NOT reached. If `Problem_is_gwon` is absent or no shape is selected: the existing shapes did not fit this dataset — STOP and report; that is the deferred conjunction-stratum bridge (out of this plan; design it next).

---

## Task 4: Extract the induced lesson as the claude-on-kernel output

**Files:**
- Create: `apps/claude-on-kernel/readings/induced-lessons.md` (documenting the extracted result; NOT yet asserted into the live ledger — that admission step is the deferred propose→Domain Change SM follow-on).

- [ ] **Step 1: Record the induced rule in claude-on-kernel terms**

`apps/claude-on-kernel/readings/induced-lessons.md`:
```markdown
# Induced (NOT hand-authored) from the staleness observations via arest-kernel
# (gather; defective gathers from stale; Problem_is_gwon; held-out solved).
# This is the candidate Engine Lesson; admission into the live apps/claude ledger
# is the deferred propose -> Domain Change SM step (out of this MVP).

## Instance Facts

# The induced operator, restated as the ledger's fact-referenced Lesson shape
# (Engine Lesson schema lives in apps/claude/readings/ledger.md):
#   Engine Lesson 'induced-stale-cell-defective' has Lesson Kind 'code-behavior'.
#   Engine Lesson 'induced-stale-cell-defective' concerns Code Site '<the stale cells' re-derivers>'.
# kept as a comment here because asserting it belongs to the admission step, not the MVP.
```

- [ ] **Step 2: Commit**

```bash
git add apps/claude-on-kernel/readings/induced-lessons.md
git commit -m "docs(claude-on-kernel): record the induced staleness lesson (pre-admission)"
```

---

## Self-Review

**1. Spec coverage:**
- Spec "consumer app importing arest-kernel + mapping readings" → Task 1. ✓
- Spec "the mapping (Observation=Cell, Features=behavior ontology, output=is-defective)" → Task 2 (with the resolved framing: `defective` is the sole Problem feature, inputs are read-features). ✓
- Spec "MVP = staleness class, count==count, held-out predicted" → Task 3. ✓
- Spec "dual-use: emits the Engine Lesson" → Task 4 (records the induced lesson; admission deferred per spec). ✓
- Spec deferred bridges (conjunction stratum; induce-verb gate; propose→SM admission) → explicitly out of plan (Task 3 Step 5 + Task 4). ✓
- Spec "no kernel changes" → no task edits `apps/kernel`. ✓

**2. Placeholder scan:** the only `<...>` is inside a COMMENT in Task 4 Step 1 (the admission step is deliberately deferred, not a code placeholder). No TBD/TODO in executable steps; every readings step shows the full content; every verification step shows the exact verb + expected result. ✓

**3. Type consistency:** feature names (`stale`, `rederived`, `authoritative`, `defective`), observation ids, and the kernel FT names (`Problem_selects_Shape`, `Problem_is_gwon`, `Observation_predicts_Value_at_Feature`, `Observation_is_solved`, `Feature_gathers_from_Feature_in_Problem`) are used identically across Tasks 2-3 and match `apps/kernel/readings/kernel.md`. ✓

**Coordination:** under `ledger-csdp-decomposition` (no new task filed). If Task 3 Step 5 hits the bridge, that is the conjunction-stratum design — a separate spec/plan, coordinated with `induce-coverage-gate-returns-empty`.
