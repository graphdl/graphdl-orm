# GraphDL State — Behavioral Entities

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| State Machine Definition | Title (within Domain) |
| Status | Name (within State Machine Definition) |
| Transition | (within State Machine Definition) |
| Guard | Name (within Transition) |

## Readings

### State Machine Definition
State Machine Definition belongs to Domain.
  Each State Machine Definition belongs to exactly one Domain.
State Machine Definition has Title.
  Each State Machine Definition has exactly one Title.
State Machine Definition is for Noun.
  Each State Machine Definition is for exactly one Noun.

### Status
Status belongs to State Machine Definition.
  Each Status belongs to exactly one State Machine Definition.
Status has Name.
  Each Status has exactly one Name.
Verb is performed in Status.
  Each Verb is performed in at most one Status.

### Transition
Transition has Status as source.
  Each Transition has exactly one Status as source.
Transition has Status as target.
  Each Transition has exactly one Status as target.
Transition is triggered by Event Type.
  Each Transition is triggered by exactly one Event Type.
Verb is performed during Transition.
  Each Verb is performed during at most one Transition.

### Guard
Guard has Name.
  Each Guard has exactly one Name.
Guard references Graph Schema.
  Each Guard references at most one Graph Schema.
Guard prevents Transition.
  Each Guard prevents exactly one Transition.
