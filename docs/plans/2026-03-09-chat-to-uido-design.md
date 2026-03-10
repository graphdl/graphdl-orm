# Redesign: chat.auto.dev → ui.do

**Goal:** chat.auto.dev stops being a separate app and becomes a domain dashboard within ui.do. The support agent stops being a compiled worker and becomes a vanilla GraphDL domain with state machines and LLM verb callbacks.

## What Exists Today

### chat.auto.dev (standalone React app)
- Customer chat interface (POST /chat to support-auto-dev worker)
- Admin dashboard with escalated/open/resolved request views
- Admin actions: re-draft, merge, send-to-customer, state machine events
- Re-draft with LLM claim extraction (constraint learning)
- iLayer rendering system (duplicated from ui.do)
- Deployed on Cloudflare Pages

### support-auto-dev (compiled agent worker)
- 14 domain files, 8 state machines, hand-authored system prompt
- 3-stage constraint verification pipeline
- itty-router REST API for chat, requests, merge, send
- Calls api.auto.dev for state machine events and claim extraction

### ui.do (domain-driven UI renderer)
- Dashboard with sections/widgets (6 widget types including chat-stream)
- Entity list/detail/create/edit views via iLayer
- State machine event triggering from action buttons
- BuildView for new app creation
- Converter registry with 14+ field types

## What Changes

### 1. The support agent becomes a GraphDL domain

No separate worker. The "agent" is:
- **Readings** → system prompt (compiled from domain model)
- **State machines** → behavior (SupportRequest lifecycle)
- **Verb callbacks** → actions (LLM for drafting, HTTP for email)
- **Deontic constraints** → guardrails (learned from redraft reasons)
- **ui.do dashboard** → interface

### 2. Support domain readings

```
SupportRequest(.id) is an entity type.
Customer(.email) is an entity type.
Message(.id) is an entity type.

Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.
Message belongs to SupportRequest.
  Each Message belongs to at most one SupportRequest.
Message has Text.
  Each Message has at most one Text.
Message has Timestamp.
  Each Message has at most one Timestamp.
Message has Role.
  Each Message has at most one Role.
SupportRequest has Subject.
  Each SupportRequest has at most one Subject.
SupportRequest has Priority.
  Each SupportRequest has at most one Priority.
```

Value types:
- Role: enum (customer, agent, admin)
- Priority: enum (low, normal, high, urgent)

### 3. State machine: SupportRequest

```
Statuses: Received, Triaging, Drafting, AwaitingApproval, Responded, WaitingOnCustomer, Resolved, Closed, Merged

Transitions:
  Received → Triaging          (event: acknowledge)    verb: analyzeLLM
  Triaging → Drafting           (event: draft)          verb: draftResponseLLM
  Drafting → AwaitingApproval   (event: submit)
  AwaitingApproval → Drafting   (event: redraft)        verb: redraftLLM
  AwaitingApproval → Responded  (event: approve)        verb: sendToCustomer
  Responded → WaitingOnCustomer (event: waitForReply)
  WaitingOnCustomer → Triaging  (event: customerReply)  verb: analyzeLLM
  Responded → Resolved          (event: resolve)
  WaitingOnCustomer → Resolved  (event: resolve)
  Resolved → Triaging           (event: reopen)
  Resolved → Closed             (event: close)
  * → Merged                    (event: merge)
```

### 4. Verb → callback mappings

| Verb | Callback Type | Callback |
|------|--------------|----------|
| analyzeLLM | AgentDefinition | Support agent (claude-sonnet, domain readings as system prompt) |
| draftResponseLLM | AgentDefinition | Same agent, drafting mode |
| redraftLLM | AgentDefinition | Same agent, with admin reason as additional context |
| sendToCustomer | Function | POST to Resend email API |

The agent's system prompt is derived from the support domain's readings. Its tools are the available event types on the SupportRequest state machine. The constraint readings (learned from redrafts) become part of the prompt.

### 5. ui.do dashboard for support domain

**Admin dashboard** — a Dashboard entity with sections:

```
Section "Escalated" at 1 cols 1
  Widget status-summary SupportRequest [status=Triaging] at 1

Section "Open Requests" at 2 cols 1
  Widget link SupportRequest [status!=Resolved,Closed,Merged] at 1

Section "Chat" at 3 cols 1
  Widget chat-stream SupportRequest at 1
```

Entity views are generated from iLayer:
- **SupportRequest list** → NavigationLayer with status badges, customer info, timestamps
- **SupportRequest detail** → FormLayer with message thread (chat field), action buttons from state machine

Action buttons on the detail view come directly from the state machine's available transitions — no hardcoded buttons needed. The state machine runtime returns `availableEvents`, and ui.do renders them.

**Re-draft** becomes: fire the `redraft` event with a reason in the event body. The verb callback (LLM) receives the reason, regenerates the draft, and the result is stored as a new Message fact.

**Merge** becomes: fire the `merge` event on the source request. The verb callback updates the primary request reference.

**Send to Customer** becomes: fire the `approve` event. The verb callback (Function → Resend HTTP POST) sends the email.

### 6. Customer-facing chat

Two options:
- **A: Separate ui.do route** — `/chat` renders only the chat-stream widget for the support domain, no sidebar/dashboard chrome
- **B: Embedded widget** — chat widget embeddable on auto.dev marketing site via iframe or web component

Option A is simpler and sufficient. The customer sees a chat interface; behind the scenes it creates SupportRequest resources, sends events, and the LLM verb callback generates responses.

## What Gets Retired

- **chat.auto.dev repo** — fully replaced by ui.do support domain dashboard
- **support-auto-dev worker** — its logic becomes domain readings + state machine + verb callbacks
- **Duplicated iLayer code** — chat.auto.dev's `ilayer/` directory is a copy of ui.do's

## What Stays

- **api.auto.dev** — still the API gateway (state machine runtime, claim extraction, rawProxy)
- **graphdl-orm** — domain model storage
- **ui.do** — becomes the universal frontend for all GraphDL domains including support

## Migration Order

1. Seed the support domain readings into graphdl-orm (nouns, readings, state machine, constraints)
2. Configure verb → AgentDefinition and verb → Function callbacks
3. Generate iLayer for the support domain
4. Create Dashboard instance facts for the admin layout
5. Verify the customer chat flow works through ui.do chat-stream widget
6. Redirect chat.auto.dev to ui.do/support (or similar)
7. Decommission support-auto-dev worker and chat.auto.dev repo

## Key Insight

The support agent's "intelligence" was never in the worker code — it was in the readings, constraints, and state machine definitions. The worker was just glue between the domain model and the LLM. By making AI first-class in GraphDL (AgentDefinition, Agent, Completion), that glue becomes declarative. The domain model IS the agent.
