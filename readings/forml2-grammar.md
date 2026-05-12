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

Translator(.name) is an entity type.

## Value Types

Text is a value type.
Head Noun is a value type.
Verb is a value type.
Trailing Marker is a value type.
  The possible values of Trailing Marker are 'is an entity type', 'is a value type', 'is abstract', 'is acyclic', 'is asymmetric', 'is antisymmetric', 'is intransitive', 'is irreflexive', 'is reflexive', 'is symmetric', 'is transitive', 'are mutually exclusive', 'is partitioned into', 'is a subtype of'.
Quantifier is a value type.
  The possible values of Quantifier are 'each', 'at most one', 'at least one', 'exactly one', 'some', 'no', 'at most', 'at least'.
Prose Stopword is a value type.
  The possible values of Prose Stopword are 'If', 'When', 'Then', 'That', 'This', 'An', 'A', 'The', 'Each', 'Some', 'No', 'Every'.
Constraint Span Prefix is a value type.
  The possible values of Constraint Span Prefix are 'It is obligatory that ', 'It is forbidden that ', 'It is permitted that ', 'Each ', 'each ', 'at most one ', 'exactly one ', 'at least one ', 'some ', 'No ', 'no '.
Deontic Predicate Operator is a value type.
  The possible values of Deontic Predicate Operator are ' ends with', ' does not end with', ' starts with', ' does not start with'.
Deontic Predicate Operator Kind is a value type.
  The possible values of Deontic Predicate Operator Kind are 'ends_with', 'ends_with', 'starts_with', 'starts_with'.
Deontic Predicate Operator Negated is a value type.
  The possible values of Deontic Predicate Operator Negated are 'false', 'true', 'false', 'true'.
Non Canonical Negation Hint is a value type.
  The possible values of Non Canonical Negation Hint are ' does not ', ' do not ', ' did not ', ' cannot ', ' can not ', ' must not ', ' will not ', ' would not ', ' never ', ' no longer '.
Derivation Marker is a value type.
  The possible values of Derivation Marker are 'fully-derived', 'derived-and-stored', 'semi-derived'.
Derivation Marker Symbol is a value type.
  The possible values of Derivation Marker Symbol are '**', '*', '+'.
Role Position is a value type.
Literal Value is a value type.
Keyword is a value type.
  The possible values of Keyword are 'iff', 'if', 'when'.
Deontic Operator is a value type.
  The possible values of Deontic Operator are 'obligatory', 'forbidden', 'permitted'.
Literal Role is a value type.
Enum Value is a value type.
Constraint Keyword is a value type.
  The possible values of Constraint Keyword are 'if and only if', 'at most one of the following holds', 'exactly one of the following holds', 'at least one of the following holds', 'if some then that'.
Ring Adjective is a value type.
  The possible values of Ring Adjective are 'irreflexive', 'asymmetric', 'antisymmetric', 'symmetric', 'intransitive', 'transitive', 'acyclic', 'reflexive'.
Word Comparator is a value type.
  The possible values of Word Comparator are 'exceeds', 'is greater than', 'is less than', 'is at least', 'is at most', 'is more than', 'equals', 'is equal to'.
Range Operator is a value type.
  The possible values of Range Operator are 'within', 'before', 'after'.
Quote Escape is a value type.
  The possible values of Quote Escape are 'doubled-quote'.
Universal Quantifier Keyword is a value type.
  The possible values of Universal Quantifier Keyword are 'for each '.
Extraction Clause Keyword is a value type.
  The possible values of Extraction Clause Keyword are ' is extracted from ', ' is derived from '.
Noun Has Noun Literal Keyword is a value type.
  The possible values of Noun Has Noun Literal Keyword are ' has '.
Ring Constraint Trailing Marker is a value type.
  The possible values of Ring Constraint Trailing Marker are 'is irreflexive', 'is asymmetric', 'is antisymmetric', 'is symmetric', 'is intransitive', 'is transitive', 'is acyclic', 'is reflexive'.
Ring Constraint Kind Code is a value type.
  The possible values of Ring Constraint Kind Code are 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC', 'RF'.
Conditional Ring Pattern is a value type.
  The possible values of Conditional Ring Pattern are 'and+impossible+isnot-ante', 'and+impossible', 'and', 'impossible', 'isnot-conse', 'itself-conse', 'plain'.
Conditional Ring Kind Code is a value type.
  The possible values of Conditional Ring Kind Code are 'AT', 'IT', 'TR', 'AS', 'AS', 'RF', 'SY'.
Deontic Constraint Kind Code is a value type.
  The possible values of Deontic Constraint Kind Code are 'UC', 'UC', 'UC'.
Deontic Constraint Modality is a value type.
  The possible values of Deontic Constraint Modality are 'deontic', 'deontic', 'deontic'.
Cardinality Constraint Kind is a value type.
  The possible values of Cardinality Constraint Kind are 'Frequency Constraint', 'Uniqueness Constraint', 'Mandatory Role Constraint'.
Cardinality Constraint Kind Code is a value type.
  The possible values of Cardinality Constraint Kind Code are 'FC', 'UC', 'MC'.
Set Constraint Kind is a value type.
  The possible values of Set Constraint Kind are 'Equality Constraint', 'Subset Constraint', 'Exclusive-Or Constraint', 'Or Constraint', 'Exclusion Constraint'.
Set Constraint Kind Code is a value type.
  The possible values of Set Constraint Kind Code are 'EQ', 'SS', 'XO', 'OR', 'XC'.
Set Constraint Arbitration Rule is a value type.
  The possible values of Set Constraint Arbitration Rule are 'derivation_rule_wins', 'antecedent_diversity_min_2', 'derivation_rule_wins', 'derivation_rule_wins', 'derivation_rule_wins'.
Object Type Source Kind is a value type.
  The possible values of Object Type Source Kind are 'Abstract Declaration', 'Partition Declaration', 'Entity Type Declaration', 'Value Type Declaration', 'Subtype Declaration'.
Object Type is a value type.
  The possible values of Object Type are 'abstract', 'abstract', 'entity', 'value', 'entity'.

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
Statement has Enum Value.
Statement has Constraint Keyword.
Statement has Classification.

Classification has Translator.

Statement has Role Reference.
Role Reference has Head Noun.
Role Reference has Literal Value.
Role Reference has Role Position.

## Statement Translator Dispatch (#833)

Per AREST.tex §3 (eq:sys) — *the entity handles the dispatch, not the
system function.* New translators are registered into DEFS without
modifying any entity. The Rust pipeline consults this table to
discover which translators apply to a given Statement Classification.
The relation is many-to-many: e.g., Subtype Declaration is handled by
both `translate_nouns` and `translate_subtypes`, and
`translate_set_constraints` handles five constraint kinds.

The Entity Type and Fact Type for this dispatch are declared in the
top-level `## Entity Types` and `## Fact Types` sections so the
bootstrap grammar parser picks them up. The Instance Facts populating
the table follow.

### Instance Facts

Classification 'Entity Type Declaration' has Translator 'translate_nouns'.
Classification 'Value Type Declaration' has Translator 'translate_nouns'.
Classification 'Subtype Declaration' has Translator 'translate_nouns'.
Classification 'Subtype Declaration' has Translator 'translate_subtypes'.
Classification 'Abstract Declaration' has Translator 'translate_nouns'.
Classification 'Partition Declaration' has Translator 'translate_nouns'.
Classification 'Partition Declaration' has Translator 'translate_partitions'.
Classification 'Enum Values Declaration' has Translator 'translate_enum_values'.
Classification 'Instance Fact' has Translator 'translate_instance_facts'.
Classification 'Fact Type Reading' has Translator 'translate_fact_types'.
Classification 'Fact Type Reading' has Translator 'translate_derivation_mode_facts'.
Classification 'Derivation Rule' has Translator 'translate_derivation_rules'.
Classification 'Uniqueness Constraint' has Translator 'translate_cardinality_constraints'.
Classification 'Mandatory Role Constraint' has Translator 'translate_cardinality_constraints'.
Classification 'Frequency Constraint' has Translator 'translate_cardinality_constraints'.
Classification 'Ring Constraint' has Translator 'translate_ring_constraints'.
Classification 'Subset Constraint' has Translator 'translate_set_constraints'.
Classification 'Equality Constraint' has Translator 'translate_set_constraints'.
Classification 'Exclusion Constraint' has Translator 'translate_set_constraints'.
Classification 'Exclusive-Or Constraint' has Translator 'translate_set_constraints'.
Classification 'Or Constraint' has Translator 'translate_set_constraints'.
Classification 'Value Constraint' has Translator 'translate_value_constraints'.
Classification 'Deontic Constraint' has Translator 'translate_deontic_constraints'.

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
Classification 'Exclusive-Or Constraint' is a Classification.
Classification 'Or Constraint' is a Classification.
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

Statement has Classification 'Mandatory Role Constraint' iff Statement has Quantifier 'some'.

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
Statement has Classification 'Exclusion Constraint' iff Statement has Constraint Keyword 'at most one of the following holds'.

Statement has Classification 'Exclusive-Or Constraint' iff Statement has Constraint Keyword 'exactly one of the following holds'.

Statement has Classification 'Or Constraint' iff Statement has Constraint Keyword 'at least one of the following holds'.

Statement has Classification 'Equality Constraint' iff Statement has Constraint Keyword 'if and only if'.

Statement has Classification 'Subset Constraint' iff Statement has Constraint Keyword 'if some then that'.

Statement has Classification 'Value Constraint' iff Statement has Classification 'Enum Values Declaration'.

Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'obligatory'.
Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'forbidden'.
Statement has Classification 'Deontic Constraint' iff Statement has Deontic Operator 'permitted'.
