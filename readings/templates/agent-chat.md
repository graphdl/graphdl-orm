# Agent Chat

Multi-turn conversations between a User and an Agent, with tool calls
and streaming. Builds on `templates/agents.md` (Agent, Agent Definition,
Model, Completion). Used as the supertype primitive when an app needs
both human-mediated and direct interaction modes — e.g. an admin-reviewed
email-drafting flow vs a real-time end-user chat — without duplicating
the Agent / message / tool-call structure across both subtypes.

## Entity Types

Agent Chat(.id) is an entity type.
Chat Message(.id) is an entity type.
Tool Call(.id) is an entity type.

## Value Types

Message Role is a value type.
  The possible values of Message Role are 'user', 'assistant', 'system', 'tool'.

Tool Call Id is a value type.

Streaming Mode is a value type.
  The possible values of Streaming Mode are 'streaming', 'non-streaming'.

## Fact Types

### Agent Chat
Agent Chat is for User.
  Each Agent Chat is for exactly one User.
Agent Chat is with Agent.
  Each Agent Chat is with exactly one Agent.
Agent Chat uses Streaming Mode.
  Each Agent Chat uses exactly one Streaming Mode.
Agent Chat occurred at Timestamp.
  Each Agent Chat occurred at exactly one Timestamp.

### Chat Message
Chat Message belongs to Agent Chat.
  Each Chat Message belongs to exactly one Agent Chat.
  It is possible that more than one Chat Message belongs to the same Agent Chat.

Chat Message has Body.
  Each Chat Message has exactly one Body.

Chat Message has Message Role.
  Each Chat Message has exactly one Message Role.

Chat Message occurred at Timestamp.
  Each Chat Message occurred at exactly one Timestamp.

### Tool Call (objectification of "Chat Message invokes Verb")
Chat Message invokes Verb.
  It is possible that some Chat Message invokes more than one Verb.

Tool Call is for Chat Message.
  Each Tool Call is for exactly one Chat Message.
Tool Call invokes Verb.
  Each Tool Call invokes exactly one Verb.
Tool Call has Tool Call Id.
  Each Tool Call has exactly one Tool Call Id.
Tool Call has Result.
  Each Tool Call has at most one Result.

This association with Chat Message, Verb, Tool Call Id provides the preferred identification scheme for Tool Call.

### User actions
User closes Agent Chat.
  Each User, Agent Chat combination occurs at most once in the population of User closes Agent Chat.

## Constraints

If some Tool Call is for some Chat Message then that Chat Message has Message Role 'assistant'.

It is obligatory that each Agent Chat has at least one Chat Message after it occurred.

## Instance Facts

State Machine Definition 'Agent Chat' is for Noun 'Agent Chat'.
Status 'Open' is initial in State Machine Definition 'Agent Chat'.
Status 'Closed' is defined in State Machine Definition 'Agent Chat'.
Status 'Closed' is terminal in State Machine Definition 'Agent Chat'.

Transition 'append-message' is defined in State Machine Definition 'Agent Chat'.
Transition 'append-message' is from Status 'Open'.
Transition 'append-message' is to Status 'Open'.
Transition 'append-message' is triggered by Fact Type 'Chat Message belongs to Agent Chat'.

Transition 'close' is defined in State Machine Definition 'Agent Chat'.
Transition 'close' is from Status 'Open'.
Transition 'close' is to Status 'Closed'.
Transition 'close' is triggered by Fact Type 'User closes Agent Chat'.

Domain 'agent-chat' has Access 'public'.
Domain 'agent-chat' has Description 'Multi-turn conversations between a User and an Agent. Models messages, tool calls, and streaming mode. Used as the supertype primitive when an app needs both human-mediated and direct interaction modes (e.g. drafted emails admin-reviewed before sending versus real-time end-user chat). Builds on templates/agents.md (Agent, Agent Definition, Model). The Verb a Tool Call invokes is whichever Verb the Agent has access to via its Domain — typically HTTP-backed Verbs (federation) and JS-imported Verbs (templates/vercel-ai.md core/imports.md).'.
