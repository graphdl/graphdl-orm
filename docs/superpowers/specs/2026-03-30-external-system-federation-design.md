# External System Federation Design

## Context

The auto.dev API is a shipped product with paying customers. The AREST system is an operational support layer on top of it — monitoring, diagnosing, responding to support requests, and patching the existing codebase. Over time, AREST replaces more product code with readings-driven behavior, but today it operates on top of what exists.

The core metamodel (`readings/core.md`) already declares External System bindings:

```
External System(.Name) is an entity type.
External System has Base URL.
Noun is backed by External System.
Function is backed by External System.
Domain connects to External System with Secret Reference.
Function Type: 'httpFetch', 'query', 'agentInvocation', 'transform'.
```

TypeScript federation infrastructure exists (`engine.ts`: ServiceEndpoint, FederatedSource, resolveFromService) and secret storage exists (`entity-do.ts`: storeSecret, resolveSecret). But nothing connects them — the readings don't drive the federation, and the engine doesn't evaluate backing relationships.

## Architecture

Two layers with a hard boundary:

**Logic layer (readings -> WASM engine):** All decisions. What to fetch, when to retry, which provider to use, when to escalate, what to tell the customer. Expressed as derivation rules, constraints, and state machines in FORML 2. Compiled to FFP objects. Evaluated by the Rust/WASM engine to least fixed point.

**Effect layer (TypeScript runtime):** Executes directives. No decisions. The runtime registers functions into DEFS via store. Facts bind to registered functions. The engine applies them via rho. The runtime is a dumb pipe.

The boundary is Backus's output pair. The engine produces output containing bound effects. The runtime executes them. Results come back as input facts in the next application of the system function.

### Platform Binding

The runtime registers functions into DEFS at startup:

| Function | What it does | Registered by |
|---|---|---|
| httpFetch | GET a URL, return response body as facts | Server runtime |
| httpPost | POST payload to URL, return response | Server runtime |
| queryLog | Query ClickHouse, return rows as facts | Server runtime |
| notify | Send message to Slack/email | Server runtime |
| agentInvoke | Call LLM via ai/chat, return draft | Server runtime |
| render | Bind fact to UI widget | Browser runtime |
| upsert | Persist dirty facts to cell storage | Storage runtime |

The engine doesn't have a hardcoded list. The runtime stores whatever functions it supports. New runtimes (mobile, CLI, IoT) register their own functions. The engine just applies rho.

Bindings are live: when a cell's contents change, the rho-application re-evaluates and the bound function fires. Event streaming is re-evaluation of rho over updated state, not a separate pub/sub mechanism.

### No Procedural Cruft

There is no TypeScript code that reads facts from P and makes decisions. The federation config is not a manual object somebody constructs. The engine evaluates the backing relationships from the readings, produces effect directives, and the runtime executes them. If a decision needs to be made, it's a derivation rule in a reading.

## Routing

AREST endpoints use `/arest/` prefix. Legacy auto.dev product endpoints stay on `/api/`. Both are valid External System targets:

- `https://api.auto.dev/arest/entities/...` — AREST system function
- `https://api.auto.dev/data/...`, `/specs/...`, `/listings/...` — auto.dev product API

The ui.do proxy and population module must be updated from `/api/` to `/arest/`.

## Readings

### External System Resolution (extends core.md)

The existing declarations in core.md are sufficient for the metamodel. The engine needs to honor them:

- `Noun is backed by External System` — when a query targets this noun, produce a fetch effect using the External System's Base URL
- `Domain connects to External System with Secret Reference` — resolve auth credentials for the fetch
- `Function is backed by External System` — when this function fires, call the External System

### Service Health Domain (new: service-health.md)

```
Log Entry(.id) is an entity type.
Log Entry has Timestamp.
  Each Log Entry has exactly one Timestamp.
Log Entry has HTTP Status.
Log Entry has Endpoint.
Log Entry has Response Time.
Log Entry has External System.
Log Entry has User.
Log Entry has Request Body.
Log Entry has Response Body.
Log Entry has Error Message.

Service Health(.id) is an entity type.
External System has Service Health.
  Each External System has exactly one Service Health.

Service Health Status is a value type.
  The possible values of Service Health Status are
    'healthy', 'degraded', 'down'.

Error Rate is derived from Log Entry count
  where Log Entry has HTTP Status >= 400
  per External System per Interval.

Average Response Time is derived from Log Entry Response Time
  per External System per Interval.
```

### Derivation Rules (service-health.md)

```
External System has Service Health Status 'degraded'
  iff Error Rate for External System exceeds Error Threshold per Interval.

External System has Service Health Status 'degraded'
  iff Average Response Time for External System
  exceeds Latency Threshold per Interval.

External System has Service Health Status 'down'
  iff Error Rate for External System exceeds Down Threshold per Interval.

Usage Anomaly is detected for User
  iff Request Count for User exceeds Normal Range per Interval.

Revenue Signal is derived from Log Entry count
  per API Product per Billing Period.
```

### Incident State Machine (service-health.md)

Most errors are transient — retry and fallback happen automatically via derivation rules, no incident needed. An Incident is only created when something is persistently wrong.

```
Incident(.id) is an entity type.
Incident references External System.
Incident has Incident Status.

Incident Status 'open' is initial
  in State Machine Definition 'Incident'.
Transition 'investigate' is from Incident Status 'open'.
  Transition 'investigate' is to Incident Status 'investigating'.
Transition 'escalate' is from Incident Status 'investigating'.
  Transition 'escalate' is to Incident Status 'escalated'.
Transition 'resolve' is from Incident Status 'investigating'.
  Transition 'resolve' is to Incident Status 'resolved'.
Transition 'resolve' is from Incident Status 'escalated'.
  Transition 'resolve' is to Incident Status 'resolved'.
```

Four states: open, investigating, escalated, resolved. That's it.

Incident transitions operate under open world assumption. The derivation rules capture known reasons to transition. Unknown reasons are possible — the system doesn't prevent manual transitions just because it can't derive a reason for them. If we knew all the reasons, there would be no need to ever fire anything manually.

### Automatic Behaviors (derivation rules, not states)

Retry and fallback are continuous derivations — they fire whenever conditions are met, not as formal incident transitions:

```
External System response is retried
  iff HTTP Status >= 500 and Retry Count < Retry Limit.

Noun population is resolved from alternate External System
  iff primary External System has Service Health Status 'degraded'
  and alternate External System serves same Noun.

Incident is created
  iff External System has Service Health Status 'degraded'
  and no existing open Incident references that External System.
```

### Investigation Actions (happen during 'investigating')

When an Incident is being investigated, the engine reads logs, analyzes the error, and attempts a fix. These are actions taken in the investigating state, not states themselves:

```
Incident investigation reads logs
  iff Incident has Incident Status 'investigating'.

Incident fix is attempted via reading update
  iff investigation finds external model change.

Incident fix is attempted via code patch
  iff investigation finds code bug in existing product.

Incident transitions to 'escalated'
  if fix attempt fails.

Incident transitions to 'escalated'
  if root cause is undetermined.
```

### Deontic Constraints (Escalation Policy)

```
It is permitted that retry is executed autonomously.
It is permitted that fallback is executed autonomously.
It is permitted that reading update is executed autonomously.
It is obligatory that code change requires Approval.
It is obligatory that Escalation Notification is sent
  when Incident has Incident Status 'escalated'.

It is forbidden that autonomous action modifies Billing configuration.
It is forbidden that autonomous action deploys to production
  without passing Tests.
```

### Support Response (extends support.md)

```
Support Response draft uses Function Type 'agentInvocation'
  iff Support Request has Status 'Awaiting Response'.

It is obligatory that Support Response conforms to Communication Policy.
It is forbidden that Support Response references
  internal External System outage details.
It is permitted that Support Response offers
  data retrieval assistance to Customer.
```

The LLM drafts via agentInvocation. The engine checks deontic constraints. Violations feed back as the original reading text. The LLM redrafts. Bounded by max iterations — converges or fails.

## The Full Loop

A support request comes in about bad vehicle data:

1. **Intake**: Customer submits Support Request about VIN — is-cmd, create, facts in P.

2. **Data fetch**: Derivation rule fires — VIN is backed by auto.dev. httpFetch bound to fact. Runtime GETs `api.auto.dev/data/{vin}`. Response becomes facts in P.

3. **Comparison**: Derivation rules compare customer-reported data vs fetched data. Discrepancy derived. Error classification from api-errors.md readings.

4. **Log analysis**: queryLog bound to log facts. Runtime queries ClickHouse for recent requests to the failing provider. All log entries become facts in P.

5. **Health derivation**: Derivation rules fire over log facts. Error rate derived. Provider health status updated. Incident created with state machine.

6. **Remediation**: Retry and fallback happen automatically (derivation rules). If the problem persists, an Incident is created and the engine investigates — reads more logs, analyzes the pattern, tries a fix (reading update or code patch). If it can't fix it, it escalates with the full diagnosis.

7. **Customer response**: agentInvoke drafts response using fetched data + diagnosis. Deontic constraints checked. Violations redraft. Send or fail.

8. **Policy**: Retry/fallback/reading updates are silent. Code changes need approval. Escalation sends a Slack notification with what the engine knows and what it tried.

## What Needs to Change

### Rust/WASM Engine (crates/fol-engine/)

1. **NounDef needs backed_by field** — `backed_by: Option<String>` in types.rs
2. **Parser recognizes backing declarations** — "Noun is backed by External System" produces a ParseAction that sets backed_by on the NounDef
3. **IR includes External System entities** — Base URL, Secret Reference stored in the compiled schema
4. **Engine produces effect directives** — when a query targets a backed noun, the evaluation produces an effect in the output (not a direct HTTP call)

### TypeScript Runtime (src/)

1. **Runtime registers functions into engine** — httpFetch, queryLog, notify, agentInvoke, upsert passed to the WASM engine at init
2. **Engine calls registered functions** — the WASM engine invokes the registered function when a backed fact needs resolution
3. **Results fed back as input facts** — function return values become facts in the next evaluation cycle
4. **Delete manual FederatedSource wiring** — the existing resolveFromService/buildFederatedPopulation code is replaced by engine-driven evaluation

### Router (src/api/router.ts)

1. **All AREST endpoints move to /arest/ prefix** — `/arest/entities`, `/arest/access`, `/arest/seed`, etc.
2. **Legacy /api/ endpoints untouched** — these serve the existing product

### UI (ui.do/)

1. **vite.config.ts proxy paths** — `/api` -> `/arest` for AREST calls
2. **population.ts API constant** — update fetch paths from `/api/` to `/arest/`
3. **Account endpoint stays at /account** — this is auto.dev auth, not AREST

### Readings (support.auto.dev/domains/)

1. **service-health.md** — new domain with Log Entry, Service Health, Incident state machine, remediation derivation rules, escalation policy
2. **Verify existing readings** — api-products.md, api-errors.md, database-routing.md, apis-surface.md contain the domain knowledge the engine needs. Ensure they're parseable by the FORML 2 parser.

## What NOT to Build

- No separate monitoring dashboard — the UI renders (rho f):P, monitoring facts show up like any other entity
- No webhook/event system — event streaming is rho re-evaluation over updated D
- No middleware layer — everything is a derivation rule or deontic constraint
- No manual federation config — readings drive everything
- No separate incident management tool — incidents are entities with state machines like any other entity
