# GraphDL Agents — AI Behavioral Entities

## Entity Types

Model(.code) is an entity type.
Agent Definition(.id) is an entity type.
Agent(.id) is an entity type.
Completion(.id) is an entity type.

## Readings

### Model
Model has Name.
  Each Model has exactly one Name.

### Agent Definition
Agent Definition belongs to Domain.
  Each Agent Definition belongs to exactly one Domain.

Agent Definition has Name.
  Each Agent Definition has exactly one Name.

Agent Definition uses Model.
  Each Agent Definition uses exactly one Model.

### Agent
Agent is instance of Agent Definition.
  Each Agent is instance of exactly one Agent Definition.

Agent is for Resource.
  Each Agent is for at most one Resource.

### Completion
Completion belongs to Agent.
  Each Completion belongs to exactly one Agent.

Completion has input Text.
  Each Completion has exactly one input Text.

Completion has output Text.
  Each Completion has at most one output Text.

Completion occurred at Timestamp.
  Each Completion occurred at exactly one Timestamp.

### Verb connection
Verb invokes Agent Definition.
  Each Verb invokes at most one Agent Definition.

## Instance Facts

Domain 'agents' has Visibility 'public'.
