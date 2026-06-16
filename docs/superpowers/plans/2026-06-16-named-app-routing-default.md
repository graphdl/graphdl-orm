# Named App Routing as the Default — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make named per-call `app` routing the default invocation path in the MCP server so multiple agents share the substrate without clobbering — close the last routing gaps (`orient` + five aux verbs) and reframe the global active-app as a demoted default.

**Architecture:** The per-call routing primitive (`resolveCallScope` → `callScope(app)` → `systemCall(key, input, scope?)`) already exists and is wired into every data verb (get/query/sql/cells/apply/retract/schema/actions/explain). This plan threads the same `scope` into the verbs that still drop it (`orient`, `induce`, `ask`, `synthesize`, `validate`, `propose`), then reframes `apps.use`/`apps.current`/`appSummary` so per-call `app` reads as primary and the mutable global as the omitted-`app` fallback. No Rust/engine changes.

**Tech Stack:** TypeScript (ESM), `@modelcontextprotocol/sdk`, `zod` schemas, **vitest 4**. Tests are co-located at `src/mcp/server.test.ts` and verify wiring by **source-string assertion** (they deliberately never spawn the engine).

**Spec:** `docs/superpowers/specs/2026-06-16-named-app-routing-default-design.md`

---

## File Structure

- **Modify** `src/mcp/server.ts` — all production changes:
  - `orient` handler + schema (1887–1911)
  - `induce` (1588–1645), `ask` (2621–…), `synthesize` (2693–…), `validate` (2736–…), `propose` (2475–…) handlers + schemas
  - `apps.use` (1068–1097) and `apps.current` (967–977) descriptions
  - `appSummary` next-action nudge (380–402)
- **Modify** `src/mcp/server.test.ts` — the `per-call app scoping` suite:
  - Strengthen the `SCOPED_VERBS` loop (1293–1301) to also assert scope-threading, then extend `SCOPED_VERBS` (1288) with each newly-routed verb.

## Test commands

- Run the server suite: `yarn vitest run src/mcp/server.test.ts`
- Run a single describe: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
- Typecheck: `yarn typecheck`

## Conventions every routing task follows

The uniform per-verb change (mirrors the existing data verbs):
1. Add `app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.')` to the verb's `inputSchema`.
2. Add `app` to the handler's destructured parameter object.
3. As the first handler line, `const scope = callScope(app)`.
4. Pass `scope` as the trailing argument to every `systemCall(...)` (and `dispatchRead`/`dispatchCommand`) the handler makes **on the local path**. The remote `httpRequest('/arest/default/...')` path is intentionally left unscoped (out of scope — see below).

## Out of scope (do NOT do in this plan)

- Changing `apps.use`'s context-receipt invalidation behavior (it still invalidates on switch; reframing is description-only).
- Routing the remote/Cloudflare `httpRequest('/arest/default/...')` paths (local stdio multi-agent is the target).
- `compile`, `select_component`, `verify`, `debug`, `tutor.*` (not active-app-state reads, or separate surfaces).
- Removing the global default entirely.

---

## Task 1: Route `orient` (the bug + primary multi-agent path)

**Files:**
- Modify: `src/mcp/server.test.ts:1286-1301` (strengthen + extend the wiring loop)
- Modify: `src/mcp/server.ts:1887-1911` (orient schema + handler + description)

- [ ] **Step 1: Strengthen the wiring loop to assert scope-threading (stays green)**

In `src/mcp/server.test.ts`, replace the existing `for (const verb of SCOPED_VERBS)` block (1293–1301) with a version that also checks the handler resolves and threads a scope:

```ts
    for (const verb of SCOPED_VERBS) {
      it(`'${verb}' input schema exposes an optional \`app\` override`, () => {
        const config = sliceConfig(verb)
        // The schema must declare `app: z.string().optional()` so a call
        // can scope itself without `apps.use`.
        expect(config, `${verb}: missing app input field`)
          .toMatch(/app:\s*z\.string\(\)\.optional\(\)/)
      })

      it(`'${verb}' resolves a per-call scope and threads it into its engine call`, () => {
        const config = sliceConfig(verb)
        // The handler must resolve the optional `app` to a CallScope and
        // pass it to its systemCall / dispatchRead / dispatchCommand, so
        // the override actually changes which app's DB/readings are read.
        expect(config, `${verb}: handler does not resolve callScope(app)`)
          .toMatch(/const scope = callScope\(app\)/)
        expect(config, `${verb}: scope not threaded into an engine call`)
          .toContain(', scope)')
      })
    }
```

- [ ] **Step 2: Run the suite — verify it is STILL GREEN for the 9 existing verbs**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS (all of get/query/sql/cells/apply/retract/schema/actions/explain already resolve `callScope(app)` and thread `, scope)`). This proves the strengthened guardrail is sound before we add new verbs.

- [ ] **Step 3: Add `'orient'` to `SCOPED_VERBS` — the RED step**

In `src/mcp/server.test.ts`, extend the array (1288–1291):

```ts
    const SCOPED_VERBS = [
      'get', 'query', 'sql', 'cells',
      'apply', 'retract', 'schema', 'actions', 'explain',
      'orient',
    ] as const
```

- [ ] **Step 4: Run the suite — verify orient FAILS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL — `'orient' input schema exposes an optional app override` and `'orient' resolves a per-call scope…` fail (orient has neither `app` nor `callScope(app)` yet).

- [ ] **Step 5: Implement orient routing**

In `src/mcp/server.ts`, add the `app` field to orient's `inputSchema` (after the `active_app` line, 1894):

```ts
      active_app: z.string().optional().describe('DEPRECATED label-only fallback (honored only when `app` is omitted): names the active entry + suggested_next without routing. Prefer `app`, which both routes and labels.'),
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Then replace the handler (1897–1910) with:

```ts
  async ({ apps_dir, active_app, app }) => {
    const scope = callScope(app)
    const envelope: Record<string, string> = {}
    if (apps_dir !== undefined) envelope.apps_dir = apps_dir
    // `app` (routing) names the active entry + suggested_next; else the
    // deprecated `active_app` label; else the global default app's name.
    // This is also the bug fix: counts now come from `scope`'s DB, not
    // whatever app happens to be globally active.
    envelope.active_app = scope ? scope.name : (active_app ?? activeApp.name)
    if (AREST_MODE === 'local') {
      const raw = await systemCall('orient', JSON.stringify(envelope), scope)
      return textResult(parseOrientResponse(raw))
    }
    const data = await httpRequest('/arest/default/orient', {
      method: 'POST',
      body: JSON.stringify(envelope),
    })
    return textResult(data)
  },
```

- [ ] **Step 6: Run the suite — verify orient PASSES**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS (orient now exposes `app` and threads `scope`).

- [ ] **Step 7: Rewrite orient's description (keep WHEN/ALTERNATIVE/GOTCHA/NEXT — orient is a PINNED verb)**

In `src/mcp/server.ts`, replace orient's `description` string (1890–1891). Keep all four markers; replace the old "counts come from the ACTIVE app's loaded snapshot only" gotcha with the per-call routing story:

```ts
    description:
      'One-screen session re-orientation (#871) — apps inventory + active app + recent cell-graph activity + suggested-next pointer, in a single envelope. WHEN: FIRST call in a new session, or any time the agent has lost the thread and wants "where am I, what just happened, what should I do next?". ALTERNATIVE: apps.current when you only need the active app name (cheaper); apps.list / apps.check when you want depth on every app and do NOT need the recent-activity summary; context when you specifically need a mutation receipt (orient does not mint one). GOTCHA: pass `app=<name>` to route the counts + recent activity to THAT app (the multi-agent-safe way — it never changes the shared default); omit `app` to report the current default app. Counts come from the routed app\'s loaded snapshot; sibling apps appear with last_compile mtimes but no per-app row counts (the engine holds one DB at a time). Pass apps_dir only when you want sibling enumeration. NEXT: follow the `suggested_next` pointer in the response, or call context if the next move is a mutation.',
```

- [ ] **Step 8: Run the full server suite — confirm #873 orient description test + orient envelope test stay green**

Run: `yarn vitest run src/mcp/server.test.ts`
Expected: PASS — including `#873 … 'orient' description includes WHEN / ALTERNATIVE / GOTCHA / NEXT` and `#871 orient verb envelope parsing`.

- [ ] **Step 9: Typecheck**

Run: `yarn typecheck`
Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route orient per-call via app (fixes wrong-app counts); harden wiring test"
```

---

## Task 2: Route `induce`

**Files:**
- Modify: `src/mcp/server.test.ts:1288` (add `'induce'` to `SCOPED_VERBS`)
- Modify: `src/mcp/server.ts:1593-1636` (induce schema + handler)

- [ ] **Step 1: Add `'induce'` to `SCOPED_VERBS` — RED**

Append `'induce'` to the `SCOPED_VERBS` array in `src/mcp/server.test.ts`.

- [ ] **Step 2: Run — verify FAIL**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL for `'induce'` (no `app`, no scope threading).

- [ ] **Step 3: Implement induce routing**

In `src/mcp/server.ts`, add to induce's `inputSchema` (after `bound`, 1596):

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Change the handler signature (1599) from `async ({ ft_id, to_explain, bound }) => {` to:

```ts
  async ({ ft_id, to_explain, bound, app }) => {
    const scope = callScope(app)
```

Change the systemCall (1636) from `const raw = await systemCall('induce', arg)` to:

```ts
      const raw = await systemCall('induce', arg, scope)
```

- [ ] **Step 4: Run — verify PASS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `yarn typecheck` (expect no errors), then:

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route induce per-call via app"
```

---

## Task 3: Route `ask`

**Files:**
- Modify: `src/mcp/server.test.ts:1288` (add `'ask'`)
- Modify: `src/mcp/server.ts:2625-2680` (ask schema + handler — three systemCalls)

- [ ] **Step 1: Add `'ask'` to `SCOPED_VERBS` — RED**

Append `'ask'` to `SCOPED_VERBS`.

- [ ] **Step 2: Run — verify FAIL**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL for `'ask'`.

- [ ] **Step 3: Implement ask routing**

Add to ask's `inputSchema` (after `llm_response`, 2628):

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Change the handler signature (2631) from `async ({ question, noun, llm_response }) => {` to:

```ts
  async ({ question, noun, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'ask requires local mode' })
    }
    const schemaRaw = noun
      ? await systemCall(`schema:${noun}`, '', scope)
      : await systemCall('list:Noun', '', scope)
```

(That is: insert `const scope = callScope(app)` as the first line, and add `, scope` to both the `schema:${noun}` call at 2636 and the `list:Noun` call at 2637.)

Then add `, scope` to the projection query later in the handler (2680), changing `const raw = await systemCall(\`query:${spec.fact_type}\`, filterStr)` to:

```ts
    const raw = await systemCall(`query:${spec.fact_type}`, filterStr, scope)
```

- [ ] **Step 4: Run — verify PASS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `yarn typecheck`, then:

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route ask per-call via app"
```

---

## Task 4: Route `synthesize`

**Files:**
- Modify: `src/mcp/server.test.ts:1288` (add `'synthesize'`)
- Modify: `src/mcp/server.ts:2697-2709` (synthesize schema + handler — two systemCalls)

- [ ] **Step 1: Add `'synthesize'` to `SCOPED_VERBS` — RED**

Append `'synthesize'` to `SCOPED_VERBS`.

- [ ] **Step 2: Run — verify FAIL**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL for `'synthesize'`.

- [ ] **Step 3: Implement synthesize routing**

Add to synthesize's `inputSchema` (after `llm_response`, 2700):

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Change the handler (2703–2709) from:

```ts
  async ({ noun, id, llm_response }) => {
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'synthesize requires local mode' })
    }
    const raw = id
      ? await systemCall(`get:${noun}`, id)
      : await systemCall(`list:${noun}`, '')
```

to:

```ts
  async ({ noun, id, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'synthesize requires local mode' })
    }
    const raw = id
      ? await systemCall(`get:${noun}`, id, scope)
      : await systemCall(`list:${noun}`, '', scope)
```

- [ ] **Step 4: Run — verify PASS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `yarn typecheck`, then:

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route synthesize per-call via app"
```

---

## Task 5: Route `validate`

**Files:**
- Modify: `src/mcp/server.test.ts:1288` (add `'validate'`)
- Modify: `src/mcp/server.ts:2740-2798` (validate schema + handler — two systemCalls)

- [ ] **Step 1: Add `'validate'` to `SCOPED_VERBS` — RED**

Append `'validate'` to `SCOPED_VERBS`.

- [ ] **Step 2: Run — verify FAIL**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL for `'validate'`.

- [ ] **Step 3: Implement validate routing**

Add to validate's `inputSchema` (after `llm_response`, 2743):

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Change the handler (2746–2750) from:

```ts
  async ({ text, constraint, llm_response }) => {
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'validate requires local mode' })
    }
    const constraintRaw = await systemCall(`constraint:${constraint}`, '').catch(() => '')
```

to:

```ts
  async ({ text, constraint, llm_response, app }) => {
    const scope = callScope(app)
    if (AREST_MODE !== 'local') {
      return textResult({ error: 'validate requires local mode' })
    }
    const constraintRaw = await systemCall(`constraint:${constraint}`, '', scope).catch(() => '')
```

Then add `, scope` to the per-fact verify call at 2798, changing `const vraw = await systemCall(\`verify:${fact.fact_type}\`, factStr)` to:

```ts
        const vraw = await systemCall(`verify:${fact.fact_type}`, factStr, scope)
```

- [ ] **Step 4: Run — verify PASS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `yarn typecheck`, then:

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route validate per-call via app"
```

---

## Task 6: Route `propose` (a governed write — creates a Domain Change)

**Files:**
- Modify: `src/mcp/server.test.ts:1288` (add `'propose'`)
- Modify: `src/mcp/server.ts:2480-2511` (propose schema + handler — one systemCall)

- [ ] **Step 1: Add `'propose'` to `SCOPED_VERBS` — RED**

Append `'propose'` to `SCOPED_VERBS`.

- [ ] **Step 2: Run — verify FAIL**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: FAIL for `'propose'`.

- [ ] **Step 3: Implement propose routing**

Add to propose's `inputSchema` (after `verbs`, 2487):

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

Change the handler signature (2490) from `async ({ context_receipt, rationale, target_domain, readings, nouns, constraints, verbs }) => {` to:

```ts
  async ({ context_receipt, rationale, target_domain, readings, nouns, constraints, verbs, app }) => {
    const scope = callScope(app)
```

Change the systemCall (2511) from `const createRaw = await systemCall(\`create:Domain Change\`, JSON.stringify(createCmd))` to:

```ts
    const createRaw = await systemCall(`create:Domain Change`, JSON.stringify(createCmd), scope)
```

> NOTE: `propose` makes exactly one engine call (`create:Domain Change` at 2511); the rest of the handler shapes the response. If a future edit adds more `systemCall`/`dispatchCommand` calls to this handler, thread `scope` into those too.

- [ ] **Step 4: Run — verify PASS**

Run: `yarn vitest run src/mcp/server.test.ts -t "per-call app scoping"`
Expected: PASS.

- [ ] **Step 5: Run the full server suite (propose is also pinned by #873)**

Run: `yarn vitest run src/mcp/server.test.ts`
Expected: PASS — including `#873 … 'propose' description includes WHEN / ALTERNATIVE / GOTCHA / NEXT` (unchanged; we did not touch propose's description).

- [ ] **Step 6: Typecheck + commit**

Run: `yarn typecheck`, then:

```bash
git add src/mcp/server.ts src/mcp/server.test.ts
git commit -m "feat(mcp): route propose per-call via app"
```

---

## Task 7: Reframe `apps.use` + `apps.current` as the DEFAULT (descriptions only)

**Files:**
- Modify: `src/mcp/server.ts:1071-1072` (apps.use description)
- Modify: `src/mcp/server.ts:970-971` (apps.current description)

Both verbs are PINNED by #873, so every replacement description MUST keep `WHEN:` / `ALTERNATIVE:` / `GOTCHA:` / `NEXT:`. Mechanism is unchanged (`activateApp` / `appSummary(activeApp)`).

- [ ] **Step 1: Reframe `apps.use` description**

Replace apps.use's `description` (1071–1072) with:

```ts
    description:
      'Set the process DEFAULT app — the app used by calls that OMIT `app`. WHEN: you are working single-app for a while and want to stop repeating `app=` on every call. PREFER passing `app=<name>` per call instead: that routes a single call statelessly and is the multi-agent-safe default (two agents sharing this server never clobber each other). ALTERNATIVE: pass `app` on the individual verb (get / query / sql / cells / apply / orient / …) to route ONE call without changing the shared default; apps.create when the app does not yet exist; apps.status to peek at an app without making it the default. GOTCHA: this changes a process-wide default shared by every call that omits `app`, and it INVALIDATES any context_receipt minted under the prior default — mutating verbs reject stale receipts after a switch, so call context again. Library entries (no readings/ + no .db) refuse activation with error="app_is_library". NEXT: context to mint a receipt for the new default, then orient (or apps.current) to confirm.',
```

- [ ] **Step 2: Reframe `apps.current` description**

Replace apps.current's `description` (970–971) with:

```ts
    description:
      'Show the DEFAULT app (readings dir, DB path, health) — the app used when a call omits `app`. WHEN: you need a quick "what is the default scope right now?" answer mid-session. ALTERNATIVE: orient when you also want recent activity + sibling apps in one envelope; apps.status for full health of a specific (possibly non-default) app; apps.list for every app. GOTCHA: this reports the default only — individual calls can still route elsewhere by passing `app=<name>`, which does NOT change this default. NEXT: apps.use name=… to change the default, or pass app=<name> on a single verb to route just that call.',
```

- [ ] **Step 3: Run the full server suite**

Run: `yarn vitest run src/mcp/server.test.ts`
Expected: PASS — `#873 … 'apps.use'` and `'apps.current'` description tests still find all four markers.

- [ ] **Step 4: Commit**

```bash
git add src/mcp/server.ts
git commit -m "docs(mcp): reframe apps.use/apps.current as the omitted-app default, not the picker"
```

---

## Task 8: Repoint the `appSummary` next-action nudge to per-call `app`

**Files:**
- Modify: `src/mcp/server.ts:384-391` (the non-active-app nudge in `appSummary`)

- [ ] **Step 1: Guard against a pinned test string**

Run: `yarn vitest run src/mcp/server.test.ts -t "make this app the active UoD"` — and also grep the test for the reason string:

Run: `grep -n "make this app the active UoD" src/mcp/server.test.ts`
Expected: no matching test (the reason string is not asserted). If it IS asserted, update that assertion to the new reason text in Step 2.

- [ ] **Step 2: Update the nudge reason**

Replace the `nextActions.push({...})` block (385–391) with:

```ts
  if (!active && health.status !== 'library' && health.status !== 'not_found') {
    nextActions.push({
      tool: 'apps.use',
      args: { name: app.name },
      reason: `set '${app.name}' as the default for calls that omit \`app\` — or pass app="${app.name}" on a single verb to route just that call without changing the default (multi-agent-safe)`,
    })
  }
```

- [ ] **Step 3: Run the full server suite + typecheck**

Run: `yarn vitest run src/mcp/server.test.ts` then `yarn typecheck`
Expected: PASS, no type errors.

- [ ] **Step 4: Commit**

```bash
git add src/mcp/server.ts
git commit -m "docs(mcp): appSummary nudge points at per-call app routing"
```

---

## Task 9: Backfill the consistent `app` describe on the pre-existing data verbs (polish)

The nine original data verbs already expose `app: z.string().optional()` but may lack the consistent `.describe(...)` the newly-routed verbs now carry. This task makes the surface uniform and self-documenting. The wiring regex `/app:\s*z\.string\(\)\.optional\(\)/` still matches when `.describe(...)` is appended, so no test breaks.

**Files:**
- Modify: `src/mcp/server.ts` — the `app` field on `get`, `query`, `sql`, `cells`, `apply`, `retract`, `schema`, `actions`, `explain`.

- [ ] **Step 1: Find every existing `app` field lacking a describe**

Run: `grep -n "app: z.string().optional()" src/mcp/server.ts`
For each match that is NOT immediately followed by `.describe(`, note its line.

- [ ] **Step 2: Append the standard describe to each**

For each such field, change `app: z.string().optional(),` to:

```ts
      app: z.string().optional().describe('Route this call to the named app instead of the default (multi-agent-safe). Omit to use the current default app.'),
```

(Match the surrounding indentation exactly; the field already exists, so this only appends `.describe(...)` before the trailing comma.)

- [ ] **Step 3: Run the full server suite + typecheck**

Run: `yarn vitest run src/mcp/server.test.ts` then `yarn typecheck`
Expected: PASS (the `SCOPED_VERBS` schema regex still matches), no type errors.

- [ ] **Step 4: Commit**

```bash
git add src/mcp/server.ts
git commit -m "docs(mcp): uniform per-call app describe across all routed verbs"
```

---

## Final verification (after all tasks)

- [ ] **Run the entire test suite**

Run: `yarn vitest run`
Expected: PASS (or no NEW failures vs. the pre-change baseline — capture the baseline first with `git stash` if unsure).

- [ ] **Typecheck the whole project**

Run: `yarn typecheck`
Expected: no errors.

- [ ] **Live acceptance check (post rebuild + redeploy + MCP relaunch)**

The MCP server pins its engine/binary at startup, so a relaunch is required for the running session to pick up these changes. After relaunch, with the global default set to a *different* app than `tasks`:
- `orient app="tasks"` returns tasks' task counts (NOT the default app's) — this is the regression check for the original wrong-app bug.
- `orient` with no `app` reports the current default app's counts and names it in `active_app`.
- `query app="tasks" Task_has_Task_Status` and `apps.current` show the per-call route did not change the default.

---

## Self-Review

**1. Spec coverage**
- Spec §1 "thread scope into orient" → Task 1. ✓
- Spec §1 "audit + patch aux verbs" → Tasks 2–6 (induce, ask, synthesize, validate, propose); compile/select_component/verify/debug judged exempt and listed under Out of scope. ✓
- Spec §2 "per-call app is the documented primary" → describe text on every routed `app` field (Tasks 1–6, 9) + reframed apps.use/apps.current (Task 7). ✓
- Spec §3 "omitted app → mutable global default (kept)" → unchanged fallback in `callScope`/`currentDbPath`; no task needed (already true); orient handler preserves it (`scope ? … : activeApp.name`). ✓
- Spec §4 "repurpose apps.use to set the default" → Task 7 + Task 8. ✓
- Spec §5 "context-receipt stays app-agnostic" → no code change required (already true, 877–882); listed under Out of scope (receipt invalidation unchanged). ✓
- Spec Testing bullets → covered by the strengthened `SCOPED_VERBS` loop (schema + threading), the pre-existing purity test (1303) and systemCall-scope test (1316), and the live acceptance check. ✓

**2. Placeholder scan** — no "TBD"/"add error handling"/"similar to Task N"; every code step shows the exact edit. ✓

**3. Type consistency** — every routed handler uses the identical `const scope = callScope(app)` and trailing `, scope)` form already proven by the nine data verbs; `app` field shape is identical everywhere. `callScope`, `systemCall(…, scope)`, `activeApp` are all existing symbols. ✓

**One residual note (carried from the spec, not a gap):** keeping a mutable global default means an omitted-`app` multi-agent call can still collide; mitigated (not eliminated) by per-call `app` being the documented primary. Accepted by the user.
