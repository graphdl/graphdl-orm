# GraphDL Agents — AI Behavioral Entities

## Entity Types

Model(.code) is an entity type.
AgentDefinition(.id) is an entity type.
Agent(.id) is an entity type.
Completion(.id) is an entity type.

## Readings

### Model
Model has Name.
  Each Model has at most one Name.

### AgentDefinition
AgentDefinition belongs to Domain.
  Each AgentDefinition belongs to at most one Domain.

AgentDefinition has Name.
  Each AgentDefinition has at most one Name.

AgentDefinition uses Model.
  Each AgentDefinition uses at most one Model.

### Agent
Agent is instance of AgentDefinition.
  Each Agent is instance of at most one AgentDefinition.

Agent is for Resource.
  Each Agent is for at most one Resource.

### Completion
Completion belongs to Agent.
  Each Completion belongs to at most one Agent.

Completion has input Text.
  Each Completion has at most one input Text.

Completion has output Text.
  Each Completion has at most one output Text.

Completion occurred at Timestamp.
  Each Completion occurred at at most one Timestamp.

### Verb connection
Verb invokes AgentDefinition.
  Each Verb invokes at most one AgentDefinition.
