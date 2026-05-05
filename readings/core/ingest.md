# Event Ingest

## Entity Types

Webhook Event(.id) is an entity type.
Webhook Event Type(.Name) is an entity type.

## Value Types

JSON Path is a value type.
Payload is a value type.

## Fact Types

### Webhook Event

Webhook Event has Webhook Event Type.
  Each Webhook Event has exactly one Webhook Event Type.

Webhook Event has Payload.
  Each Webhook Event has exactly one Payload.

Webhook Event has Timestamp.
  Each Webhook Event has exactly one Timestamp.

Webhook Event is processed.

### Webhook Event Type

Webhook Event Type belongs to External System.
  Each Webhook Event Type belongs to exactly one External System.
  It is possible that more than one Webhook Event Type belongs to the same External System.

### Yields

Webhook Event Type yields Fact Type with Role from JSON Path.
  For each Webhook Event Type, Fact Type, Role combination, that triple has at most one JSON Path.
  It is possible that some Webhook Event Type yields more than one Fact Type.
  It is possible that more than one Webhook Event Type yields the same Fact Type.

## Constraints

It is forbidden that a Webhook Event is processed more than once.

It is obligatory that for each Webhook Event Type that yields some Fact Type, every Role of that Fact Type appears in some Webhook Event Type yields Fact Type with Role from JSON Path.

## Derivation Rules

* Webhook Event yields Fact iff Webhook Event has Webhook Event Type
  and Webhook Event Type yields Fact Type
  and Fact is of that Fact Type
  and for each Role of that Fact Type some Resource fills that Role
  where that Resource is found by reference scheme over the value at
  JSON Path in the Payload of that Webhook Event.

## Instance Facts

Domain 'ingest' has Access 'public'.
Domain 'ingest' has Description 'Webhook event ingest. External system pushes a Webhook Event carrying a Payload; the Webhook Event Type declares which Fact Types it yields and the JSON Paths that fill each Role.'.
