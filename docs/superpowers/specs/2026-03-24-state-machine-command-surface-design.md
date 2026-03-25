# State Machines as Command Surface

Transitions define what can happen. Tools, buttons, and API actions are projections of the same model data. The readings are the authority.

## Model Extensions

Two new facts added to `readings/state.md`:

```
Event Type has Pattern.
  Each Event Type has at most one Pattern.

Status is initial.
```

One subtype change in `readings/core.md`:

```
Verb is a subtype of Function.
```

Verb IS a Function — it inherits callback URI, HTTP Method, Headers. `Verb executes Function` remains valid as function composition (a Verb can delegate to another Function).

## Command Surface Projection

Every entity with a state machine includes its available transitions in the response:

```json
{
  "entity": { "id": "sr-1", "type": "SupportRequest", "data": {} },
  "state": "Received",
  "isInitial": true,
  "transitions": [
    { "event": "investigate", "target": "Investigating", "verb": "Investigate", "guards": [] },
    { "event": "escalate", "target": "Escalated", "verb": "Escalate", "guards": ["requires-manager"] },
    { "event": "close", "target": "Closed", "verb": "Close", "guards": [] }
  ]
}
```

Three projections of the same data:
- **API**: `POST /api/entities/:type/:id/transition { event: "investigate" }`
- **UI**: render `transitions` as action buttons or inline menu items
- **Agent**: receive `transitions` as dynamically generated LLM tools

CRUDL operations on resources remain as separate endpoints, not state-driven.

## Cascade Pipeline

When a transition fires and its Verb (a Function) has a callback URI:

1. Execute the callback (HTTP request to the URI with configured method/headers)
2. Match the HTTP response status code against Event Type Patterns on outgoing transitions from the new Status
3. If a match is found, fire that transition automatically
4. Repeat until no match or a final state (no outgoing transitions)
5. If any step fails, persist a Failure entity and stop the cascade

Pattern matching follows the state.do convention: `4XX` matches 400-499, `5XX` matches 500-599, `*` matches any status code. Pattern evaluation is an implementation detail, not a domain concept.

## Boundary: graphdl-orm vs apis

**graphdl-orm** owns:
- State machine definitions, transitions, guards (readings-level)
- Command surface projection: given an entity, return valid transitions with guards
- Cascade execution: fire transition, run callback, pattern match, chain
- `GET /api/entities/:type/:id` returns entity data + transitions when a state machine exists
- `GET /api/entities/:type/:id/transitions` returns valid transitions
- `POST /api/entities/:type/:id/transition` fires a transition and runs the cascade
- `Status is initial` flag replaces the "no incoming transitions" heuristic

**apis** owns:
- Authentication, authorization, domain scoping
- LLM orchestration (agent-chat)
- Tool generation: thin mapping from graphdl-orm's transition response to LLM tool definitions
- Proxy to graphdl-orm for all state machine operations

**apis removes:**
- Custom `/state/*` routes — replaced by proxy to entity transition endpoints
- State machine logic in `agent-chat.ts` — replaced by calls to graphdl-orm transition endpoints via proxy
- All state machine knowledge — apis just forwards and maps

## Agent-Chat Flow

```
1. apis: GET /graphdl/entities/SupportRequest/sr-1 (proxied to graphdl-orm)
   -> graphdl-orm returns entity data + transitions array

2. apis: map transitions to LLM tools
   transitions.map(t => ({ name: t.event, description: "Transition to " + t.target }))

3. apis: call LLM with system prompt + entity context + transition tools

4. LLM selects a tool (e.g., "investigate")

5. apis: POST /graphdl/entities/SupportRequest/sr-1/transition { event: "investigate" } (proxied)
   -> graphdl-orm fires transition, runs cascade, returns new state + new transitions

6. apis: update tools from new transitions, continue conversation
```

The agent can only call transitions that are valid from the current state. Guards are checked on execution. If a guard blocks, a Failure entity is persisted and the agent receives the guard failure in the response.

## xstate Output

`generateXState` (already exists in `src/generate/xstate.ts`) remains as a compilation target. The readings-level state machine compiles to xstate JSON for clients that want it. The runtime uses the readings directly.

## What Changes in graphdl-orm

1. **`src/api/entity-routes.ts`**: `handleGetEntity` and `handleListEntities` include transitions when a state machine exists for the entity type
2. **`src/worker/state-machine.ts`**: add cascade execution (Verb callback, pattern match, chain), use `Status is initial` flag
3. **`src/api/state.ts`**: simplify — state RPC handlers delegate to the entity transition endpoints

## What Changes in apis

1. **`graphdl/agent-chat.ts`**: replace hardcoded tools with transition-derived tools from entity response
2. **`index.ts`**: remove custom `/state/*` routes, proxy through entity endpoints
3. **`graphdl/raw-proxy.ts`**: no changes needed — already proxies `/graphdl/entities/*`
