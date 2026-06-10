# CSDP

## Description

Halpin's Conceptual Schema Design Procedure (7 steps) as an
EXECUTABLE state machine of the framework itself, not prose and not
an app: every AREST universe of discourse carries the procedure that
designs it (procedural-code-to-substrate — the engine drives and
ENFORCES its own design procedure; you cannot skip a CSDP step the
SM does not afford). Each Schema Design is an entity whose status is
the CSDP step it has reached; the legal transitions out of the
current status are the ONLY HATEOAS affordances. Building an app is
navigation of this machine — the links ARE the next valid CSDP
steps.

Steps per Halpin 2001: (1) elementary facts from examples (sec 3.3),
(2) draw fact types and populate (3.4), (3) trim schema; note basic
derivations (3.5), (4) uniqueness constraints + arity check (4.1),
(5) mandatory roles + logical derivations (5.1), (6) value,
set-comparison and subtype constraints (6.1), (7) other constraints
+ final checks (7.1).

Registered in the evolution slice (`EVOLUTION_READINGS`): CSDP is
how a schema comes to BE, the Domain Change SM is how it CHANGES —
the two halves of the self-modification machinery. First proven as
the apps/csdp dogfood walk (the tasks app's in-progress-
recommendation derivation traversed design → rmap end-to-end);
promoted to the framework proper 2026-06-10 (user directive).

## Instance Facts

Noun 'Design Note' has Format 'text'.
  <!-- Widget opt-in (pb-zero-glue-acceptance): the §4.2 view rules
       key widgets off the value type's Format; until the CDT→Format
       bridge lands (audit-entity-datatype-norma-vs-view Phase 2) a
       value type declares its Format explicitly. With this fact a
       getEntity on a Schema Design synthesizes an instance view
       whose Design Note renders as a text-input — through the
       generic render seam, zero procedure-specific code. -->.

## Entity Types

Schema Design(.name) is an entity type.

## Value Types

Design Note is a value type.
The data type of Design Note is text.

## Fact Types

### Schema Design

Schema Design has Design Note.
  Each Schema Design has at most one Design Note.

### CSDP step-completion event facts

Schema Design notes elementary facts.
Schema Design populates fact types.
Schema Design trims schema and notes derivations.
Schema Design adds uniqueness constraints.
Schema Design adds mandatory roles.
Schema Design adds value and subtype constraints.
Schema Design passes final checks.

## State Machine

State Machine Definition 'CSDP' is for Noun 'Schema Design'.
Status 'step1-elementary-facts' is initial in State Machine Definition 'CSDP'.

Transition 'advance-to-step2' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step2' is from Status 'step1-elementary-facts'.
Transition 'advance-to-step2' is to Status 'step2-populate'.
Transition 'advance-to-step2' is triggered by Event Type 'Schema Design notes elementary facts'.

Transition 'advance-to-step3' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step3' is from Status 'step2-populate'.
Transition 'advance-to-step3' is to Status 'step3-trim-derivations'.
Transition 'advance-to-step3' is triggered by Event Type 'Schema Design populates fact types'.

Transition 'advance-to-step4' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step4' is from Status 'step3-trim-derivations'.
Transition 'advance-to-step4' is to Status 'step4-uniqueness'.
Transition 'advance-to-step4' is triggered by Event Type 'Schema Design trims schema and notes derivations'.

Transition 'advance-to-step5' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step5' is from Status 'step4-uniqueness'.
Transition 'advance-to-step5' is to Status 'step5-mandatory'.
Transition 'advance-to-step5' is triggered by Event Type 'Schema Design adds uniqueness constraints'.

Transition 'advance-to-step6' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step6' is from Status 'step5-mandatory'.
Transition 'advance-to-step6' is to Status 'step6-value-subtype'.
Transition 'advance-to-step6' is triggered by Event Type 'Schema Design adds mandatory roles'.

Transition 'advance-to-step7' is defined in State Machine Definition 'CSDP'.
Transition 'advance-to-step7' is from Status 'step6-value-subtype'.
Transition 'advance-to-step7' is to Status 'step7-final-checks'.
Transition 'advance-to-step7' is triggered by Event Type 'Schema Design adds value and subtype constraints'.

Transition 'complete-design' is defined in State Machine Definition 'CSDP'.
Transition 'complete-design' is from Status 'step7-final-checks'.
Transition 'complete-design' is to Status 'designed'.
Transition 'complete-design' is triggered by Event Type 'Schema Design passes final checks'.

# Rmap

## Description

Halpin's basic Rmap procedure (relational mapping, 2001 sec 10.3,
summary box p. 428) as an executable state machine, the sibling of
the CSDP machine above: a designed conceptual schema is mapped to a
relational schema by walking steps 0-2, and the legal transitions
are the only affordances. Step 0: absorb subtypes into their top
supertype, mentally erase explicit primary identification schemes,
treat compositely identified object types as black boxes. Step 1:
map each fact type with a compound UC to a separate table. Step 2:
group fact types with functional roles attached to the same object
type into one table keyed on that object type's identifier; map 1:1
cases to a single table favoring fewer nulls (subtype-specific
columns carry their qualifications).

## Entity Types

Relational Mapping(.name) is an entity type.

## Fact Types

### Relational Mapping

Relational Mapping maps Schema Design.
  Each Relational Mapping maps exactly one Schema Design.

### Rmap step-completion event facts

Relational Mapping absorbs subtypes.
Relational Mapping maps compound fact types.
Relational Mapping groups functional fact types.

## State Machine

State Machine Definition 'Rmap' is for Noun 'Relational Mapping'.
Status 'step0-absorb-subtypes' is initial in State Machine Definition 'Rmap'.

Transition 'advance-to-rmap1' is defined in State Machine Definition 'Rmap'.
Transition 'advance-to-rmap1' is from Status 'step0-absorb-subtypes'.
Transition 'advance-to-rmap1' is to Status 'step1-compound-uc-tables'.
Transition 'advance-to-rmap1' is triggered by Event Type 'Relational Mapping absorbs subtypes'.

Transition 'advance-to-rmap2' is defined in State Machine Definition 'Rmap'.
Transition 'advance-to-rmap2' is from Status 'step1-compound-uc-tables'.
Transition 'advance-to-rmap2' is to Status 'step2-functional-grouping'.
Transition 'advance-to-rmap2' is triggered by Event Type 'Relational Mapping maps compound fact types'.

Transition 'complete-mapping' is defined in State Machine Definition 'Rmap'.
Transition 'complete-mapping' is from Status 'step2-functional-grouping'.
Transition 'complete-mapping' is to Status 'mapped'.
Transition 'complete-mapping' is triggered by Event Type 'Relational Mapping groups functional fact types'.
