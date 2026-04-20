# FORML 2 Grammar

Classification grammar + recognizer derivation rules for FORML 2.
The parser is not a program. It is this file.

Stage-1 (#285) tokenizes input into `Statement` cells with structured
fields; Stage-2 (#280) applies the derivation rules below to populate
downstream metamodel cells (`Noun`, `Fact Type`, `Role`,
`Instance Fact`, `Derivation Rule`, `Constraint`).

This file uses only Stage-1 bootstrap productions: entity types, value
types, enum values, binary / unary fact types, derivation rules.

## Entity Types

Statement(.id) is an entity type.

Role Reference(.id) is an entity type.

Classification(.name) is an entity type.

## Value Types

Text is a value type.
Head Noun is a value type.
Verb is a value type.
Trailing Marker is a value type.
Quantifier is a value type.
  The possible values of Quantifier are 'each', 'at most one', 'at least one', 'exactly one', 'some', 'no', 'at most', 'at least'.
Derivation Marker is a value type.
  The possible values of Derivation Marker are 'fully-derived', 'derived-and-stored', 'semi-derived'.
Role Position is a value type.
Literal Value is a value type.

## Fact Types

Statement has Text.
Statement has Head Noun.
Statement has Verb.
Statement has Trailing Marker.
Statement has Quantifier.
Statement has Derivation Marker.
Statement has Literal Role.
Statement has Keyword.
Statement has Deontic Operator.
Statement has Classification.

Statement has Role Reference.
Role Reference has Head Noun.
Role Reference has Literal Value.
Role Reference has Role Position.

## Instance Facts — the classification vocabulary

Classification 'Entity Type Declaration' is a Classification.
Classification 'Value Type Declaration' is a Classification.
Classification 'Subtype Declaration' is a Classification.
Classification 'Partition Declaration' is a Classification.
Classification 'Abstract Declaration' is a Classification.
Classification 'Enum Values Declaration' is a Classification.
Classification 'Fact Type Reading' is a Classification.
Classification 'Unary Fact Type Reading' is a Classification.
Classification 'Derivation Rule' is a Classification.
Classification 'Instance Fact' is a Classification.
Classification 'Uniqueness Constraint' is a Classification.
Classification 'Mandatory Role Constraint' is a Classification.
Classification 'Frequency Constraint' is a Classification.
Classification 'Value Constraint' is a Classification.
Classification 'Subset Constraint' is a Classification.
Classification 'Equality Constraint' is a Classification.
Classification 'Exclusion Constraint' is a Classification.
Classification 'Ring Constraint' is a Classification.
Classification 'Deontic Constraint' is a Classification.

## Derivation Rules — the recognizers

Statement has Classification 'Entity Type Declaration' iff Statement has Trailing Marker 'is an entity type'.

Statement has Classification 'Value Type Declaration' iff Statement has Trailing Marker 'is a value type'.

Statement has Classification 'Subtype Declaration' iff Statement has Verb 'is a subtype of'.

Statement has Classification 'Partition Declaration' iff Statement has Verb 'is partitioned into'.

Statement has Classification 'Abstract Declaration' iff Statement has Trailing Marker 'is abstract'.

Statement has Classification 'Enum Values Declaration' iff Statement has Verb 'the possible values of'.

Statement has Classification 'Derivation Rule' iff Statement has Keyword 'iff'.
Statement has Classification 'Derivation Rule' iff Statement has Keyword 'if'.
Statement has Classification 'Derivation Rule' iff Statement has Keyword 'when'.

Statement has Classification 'Fact Type Reading' iff Statement has Role Reference.

Statement has Classification 'Instance Fact' iff Statement has Literal Role.

Statement has Classification 'Uniqueness Constraint' iff Statement has Quantifier 'at most one'.

Statement has Classification 'Uniqueness Constraint' iff Statement has Quantifier 'exactly one'.

Statement has Classification 'Mandatory Role Constraint' iff Statement has Quantifier 'at least one'.

Statement has Classification 'Frequency Constraint' iff Statement has Quantifier 'at most' and Statement has Quantifier 'at least'.

Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is irreflexive'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is asymmetric'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is antisymmetric'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is symmetric'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is intransitive'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is transitive'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is acyclic'.
Statement has Classification 'Ring Constraint' iff Statement has Trailing Marker 'is reflexive'.

Statement has Classification 'Exclusion Constraint' iff Statement has Trailing Marker 'are mutually exclusive'.

Statement has Classification 'Value Constraint' iff Statement has Classification 'Enum Values Declaration'.

Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'obligatory'.
Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'forbidden'.
Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'permitted'.
