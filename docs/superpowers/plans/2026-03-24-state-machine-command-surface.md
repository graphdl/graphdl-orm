# State Machines as Command Surface — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make transitions the primary command surface — tools, buttons, and API actions are projections of valid transitions from the current state.

**Architecture:** graphdl-orm projects the command surface (entity data + transitions in every response). Cascade execution chains Verb callbacks with Event Type Pattern matching. apis proxies all state operations and maps transitions to LLM tools. Custom state routes in apis are removed.

**Tech Stack:** TypeScript (Cloudflare Workers), vitest, itty-router

**Spec:** `docs/superpowers/specs/2026-03-24-state-machine-command-surface-design.md`

---

## Phase 1: Initial Status Flag (graphdl-orm)

### Task 1: Use `Status is initial` flag instead of heuristic

**Files:**
- Modify: `src/worker/state-machine.ts`
- Modify: `src/worker/state-machine.test.ts`

- [ ] **Step 1: Write failing test for initial status by flag**

```typescript
describe('getInitialState with is_initial flag', () => {
  it('selects status with isInitial=true over heuristic', async () => {
    const entities = {
      'smd-1': { id: 'smd-1', type: 'State Machine Definition', data: { nounId: 'n1', title: 'Order' } },
      's-received': { id: 's-received', type: 'Status', data: { name: 'Received', stateMachineDefinitionId: 'smd-1', isInitial: true } },
      's-processing': { id: 's-processing', type: 'Status', data: { name: 'Processing', stateMachineDefinitionId: 'smd-1' } },
    }
    // ... mock registry + stubs
    const result = await getInitialState('Order', registry, getStub)
    expect(result.initialStatus.name).toBe('Received')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/worker/state-machine.test.ts -t "is_initial"`
Expected: FAIL — current code uses "no incoming transitions" heuristic, ignores `isInitial`

- [ ] **Step 3: Update `getInitialState` to check `isInitial` flag first**

In `src/worker/state-machine.ts`, modify `getInitialState`:
- First, look for a Status with `data.isInitial === true`
- If found, use it
- If not found, fall back to existing heuristic (backward compatibility)

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/worker/state-machine.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/worker/state-machine.ts src/worker/state-machine.test.ts
git commit -m "feat(state): use Status isInitial flag, fall back to heuristic"
```

---

## Phase 2: Transitions in Entity Responses (graphdl-orm)

### Task 2: Include transitions in entity GET responses

**Files:**
- Modify: `src/api/entity-routes.ts`
- Modify: `src/api/entity-routes.test.ts`
- Modify: `src/api/router.ts`

- [ ] **Step 1: Write failing test**

```typescript
describe('entity response with transitions', () => {
  it('includes transitions array when entity has a state machine', async () => {
    // Mock: entity has _statusId in data
    // Mock: state machine definition exists for entity type
    // Mock: valid transitions from current status
    const result = await handleGetEntity(stub, { depth: 0, getStub, transitions: { registry, getStub: smGetStub } })
    expect(result.transitions).toBeDefined()
    expect(result.transitions.length).toBeGreaterThan(0)
    expect(result.transitions[0]).toHaveProperty('event')
    expect(result.transitions[0]).toHaveProperty('target')
  })

  it('omits transitions when entity has no state machine', async () => {
    // Mock: entity without _statusId
    const result = await handleGetEntity(stub)
    expect(result.transitions).toBeUndefined()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/api/entity-routes.test.ts -t "transitions"`

- [ ] **Step 3: Implement transition inclusion**

In `handleGetEntity`, after fetching the entity:
- If entity data has `_statusId`, call `getValidTransitions` to get available transitions
- Include them in the response as `transitions: [{ event, target, verb, guards }]`
- If no `_statusId`, omit the field

In `handleListEntities`, same logic per entity (optional, could be expensive for large lists — consider only including when `?transitions=true` query param is set).

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/api/entity-routes.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/api/entity-routes.ts src/api/entity-routes.test.ts src/api/router.ts
git commit -m "feat(api): include valid transitions in entity GET responses"
```

---

## Phase 3: Cascade Execution (graphdl-orm)

### Task 3: Implement cascade pipeline in state machine

**Files:**
- Create: `src/worker/cascade-transition.ts`
- Create: `src/worker/cascade-transition.test.ts`

- [ ] **Step 1: Write failing tests**

```typescript
describe('cascade transition', () => {
  it('executes Verb callback on transition', async () => {
    // Transition has a Verb, Verb has callback URI
    // Verify callback is fetched
  })

  it('matches response status against Event Type Pattern', async () => {
    // Callback returns 200
    // Outgoing transition from new status has Event Type with Pattern '2XX'
    // Verify the next transition fires
  })

  it('stops cascade at final state (no outgoing transitions)', async () => {
    // After callback, new state has no outgoing transitions
    // Verify cascade stops
  })

  it('persists Failure entity on callback error', async () => {
    // Callback throws/returns 500
    // No matching pattern
    // Verify Failure is created
  })

  it('chains multiple cascades', async () => {
    // State A -> callback -> 200 matches '2XX' -> State B -> callback -> 201 matches '2XX' -> State C
    // Verify all three states are visited
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/worker/cascade-transition.test.ts`

- [ ] **Step 3: Implement cascade execution**

```typescript
export interface CascadeResult {
  finalState: string
  statesVisited: string[]
  callbackResults: Array<{ status: number; url: string }>
  failures: string[] // Failure entity IDs
}

export async function executeCascade(
  entityId: string,
  initialTransition: { fromStatus: string; toStatus: string; verb?: VerbData },
  context: {
    registry: RegistryStub
    getStub: (id: string) => EntityStub
    env: Env
    domain: string
  },
): Promise<CascadeResult> {
  // 1. Fire the initial transition (update entity status)
  // 2. If the transition's Verb has a callback URI, execute it
  // 3. Get the HTTP response status code
  // 4. Load outgoing transitions from the new status
  // 5. Match response status against Event Type Patterns
  // 6. If match found, fire that transition and repeat from step 2
  // 7. If no match or no callback, stop
  // 8. On any failure, persist Failure entity and stop
}

function matchPattern(statusCode: number, pattern: string): boolean {
  // '4XX' -> /4\d\d/, '5XX' -> /5\d\d/, '*' -> /.*/, '200' -> /^200$/
  const regex = new RegExp('^' + pattern.replace(/X/gi, '\\d') + '$')
  return regex.test(statusCode.toString())
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run src/worker/cascade-transition.test.ts`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/worker/cascade-transition.ts src/worker/cascade-transition.test.ts
git commit -m "feat(state): implement cascade pipeline — Verb callbacks with Event Type Pattern matching"
```

---

### Task 4: Wire cascade into transition endpoint

**Files:**
- Modify: `src/api/state.ts`
- Modify: `src/api/router.ts`

- [ ] **Step 1: Write failing test**

Test that `POST /api/entities/:type/:id/transition` runs the cascade and returns cascade result (states visited, final state, new transitions).

- [ ] **Step 2: Wire `executeCascade` into the transition handler**

In the `handleSendEvent` function (or the router's transition endpoint), after determining the valid transition:
- Call `executeCascade` instead of directly updating status
- Return the cascade result + new valid transitions from the final state

- [ ] **Step 3: Run tests**

Run: `npx vitest run src/api/state.test.ts`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/api/state.ts src/api/router.ts
git commit -m "feat(api): wire cascade execution into transition endpoint"
```

---

## Phase 4: Remove Custom State Routes from apis

### Task 5: Remove `/state/*` routes from apis (apis repo)

> **Note:** This task targets `C:/Users/lippe/Repos/apis/`

**Files:**
- Modify: `C:/Users/lippe/Repos/apis/index.ts`

- [ ] **Step 1: Read `index.ts` and find the `/state/*` routes**

- [ ] **Step 2: Remove the custom state routes**

The state machine operations should go through the existing entity proxy (`/graphdl/entities/*`) which already proxies to graphdl-orm's entity endpoints. Remove:
- `ALL /state/*` route
- Any state-specific route handlers

- [ ] **Step 3: Verify the proxy covers state operations**

The existing proxy at `/graphdl/entities/*` already forwards to `/api/entities/*` in graphdl-orm. State transitions are at `POST /api/entities/:type/:id/transition` — already covered.

- [ ] **Step 4: Commit**

```bash
cd C:/Users/lippe/Repos/apis
git add index.ts
git commit -m "refactor: remove custom /state/* routes — proxied via entity endpoints"
```

---

### Task 6: Replace hardcoded agent tools with transition-derived tools (apis repo)

> **Note:** This task targets `C:/Users/lippe/Repos/apis/`

**Files:**
- Modify: `C:/Users/lippe/Repos/apis/graphdl/agent-chat.ts`

- [ ] **Step 1: Read `agent-chat.ts` thoroughly**

Understand:
- Where tools are defined (hardcoded `query_graph`, `escalate_to_human`)
- Where state machine operations happen (creating instances, sending events)
- How the tool-use loop works

- [ ] **Step 2: Replace hardcoded tools with transition-derived tools**

After fetching the entity (SupportRequest), extract transitions from the response:

```typescript
// OLD:
const tools = [
  { name: 'query_graph', description: '...', input_schema: {...} },
  { name: 'escalate_to_human', description: '...', input_schema: {...} },
]

// NEW:
const entityRes = await env.GRAPHDL.fetch(`https://graphdl-orm/api/entities/SupportRequest/${entityId}`)
const entity = await entityRes.json()
const transitionTools = (entity.transitions || []).map(t => ({
  name: t.event,
  description: `Transition to ${t.target}`,
  input_schema: { type: 'object', properties: {} },
}))
// Keep query_graph as a CRUDL tool (not state-driven)
const tools = [
  { name: 'query_graph', description: '...', input_schema: {...} },
  ...transitionTools,
]
```

- [ ] **Step 3: Replace tool execution to fire transitions**

When the LLM calls a transition tool:

```typescript
// OLD:
if (toolName === 'escalate_to_human') { /* custom logic */ }

// NEW:
if (entity.transitions?.some(t => t.event === toolName)) {
  const res = await env.GRAPHDL.fetch(
    `https://graphdl-orm/api/entities/SupportRequest/${entityId}/transition`,
    { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ event: toolName }) }
  )
  const result = await res.json()
  // Update entity state and available tools from result
  toolResult = JSON.stringify(result)
}
```

- [ ] **Step 4: After each transition, refresh available tools**

After a transition fires, the entity is in a new state with new available transitions. Refresh the tools array for the next LLM turn.

- [ ] **Step 5: Commit**

```bash
cd C:/Users/lippe/Repos/apis
git add graphdl/agent-chat.ts
git commit -m "feat(agent): replace hardcoded tools with transition-derived tools from entity response"
```

---

## Phase 5: Simplify State RPC (graphdl-orm)

### Task 7: Consolidate state.ts into entity transition endpoints

**Files:**
- Modify: `src/api/state.ts`
- Modify: `src/api/router.ts`

- [ ] **Step 1: Audit which state.ts handlers are still needed**

Read `src/api/state.ts` and `src/api/router.ts`. Check which `/api/state/*` routes exist and whether they're covered by the entity transition endpoints:
- `GET /api/entities/:type/:id/transitions` — already exists
- `POST /api/entities/:type/:id/transition` — already exists
- `GET /api/state/...` — legacy, should be removed or redirected

- [ ] **Step 2: Redirect any remaining state routes to entity endpoints**

- [ ] **Step 3: Remove dead code from state.ts**

Any handlers that are now fully covered by entity-routes + cascade can be removed.

- [ ] **Step 4: Run all tests**

Run: `npx vitest run`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/api/state.ts src/api/router.ts
git commit -m "refactor(state): consolidate state RPC into entity transition endpoints"
```
