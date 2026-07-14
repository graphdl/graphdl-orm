# Induction

<!--
## Description
Vocabulary for the induce Func (#846-#852). The engine's search loop
populates Hypothesis Candidate facts; ranking is AUTOMATED and domain-
agnostic (forward-chain COVERAGE of the observed facts (the gate) plus
SIMPLICITY/MDL [fewest hidden facts; Occam/Solomonoff]) NOT hand-
declared per-app. Per Halpin, rulemaking is itself automatable, so the
ranking meta-rule is INDUCED (CSDP applied at the meta level), the
inductive complement of deduction, not a user knob. A Scoring Rule, when
present, is a derived/induced fact, never a hand-tuned per-domain heuristic.
Whitepaper §3 + Theorem 4: induce is a ρ-application over P that returns
candidate populations as facts in P.
-->

## Entity Types
Hypothesis Candidate(.id) is an entity type.
Scoring Rule(.id) is an entity type.

## Value Types
Confidence Score is a value type.

## Fact Types
### Hypothesis Candidate
Hypothesis Candidate has Confidence Score.
  Each Hypothesis Candidate has at most one Confidence Score.

Hypothesis Candidate explains Fact.
Hypothesis Candidate has hidden- Fact.

### Scoring Rule
Scoring Rule applies to Hypothesis Candidate.

## Instance Facts

Domain 'induction' has Access 'public'.
Domain 'induction' has Description 'Operational vocabulary the induce Func populates and consumes. Hypothesis Candidate per candidate population, Confidence Score per candidate, Scoring Rule per ranking heuristic. Whitepaper §3 + Theorem 4.'.
