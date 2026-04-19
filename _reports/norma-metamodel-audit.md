# NORMA ORM Core MetaModel — Audit vs `readings/core.md`

**Source:** `C:\Users\lippe\Repos\NORMA\Documentation\ORMCoreMetaModel.orm` (NORMA is the reference OSS implementation of ORM 2 / FORML 2 from Neumont University). 9,442 lines of XML, 52 entity types, 22 value types, 44 subtype facts.

**Target:** `crates/arest/readings/core.md` (360 lines of FORML 2).

**Purpose of audit:** scoping reference for #279 / #280 / #281. Distinguishes NORMA concepts we already express, concepts we're missing, and concepts that need a new name because NORMA didn't decompose them the way AREST's rule-body checker would.

## NORMA entity types (52)

```
AssertedSubtype                DerivedFactType            ORMNamedElement
Bound                          Dimension                  PartiallyDefinedSubtype
BusinessRule                   EntityType                 Position
CardinalityConstraint          EqualityConstraint         PreferredUniquenessConstraint
Constraint                     ExclusionConstraint        RingConstraint
ConstraintType                 Facet                      Role
DefinedSubtype                 FactType                   RoleProjection
DerivationRule                 FactTypeDerivationRule     RoleSequence
                               FrequencyConstraint        SetComparisonConstraint
                               FullyDefinedSubtype        SubsetConstraint
                               Join                       Subtype
                               JoinPath                   SubtypeConstraint
                               JoinType                   SubtypeDerivationRule
                               MandatoryOrRingConstraint  SubTypeDerivationRuleType
                               Model                      SubtypeExclusionConstraint
                               NonUnitValueType           SubtypeTotalityConstraint
                               ObjectType                 TextualConstraint
                               ORMModelElement            UniquenessConstraint
                                                          UniquenessOrFrequencyConstraint
                                                          Unit
                                                          Value
                                                          ValueComparisonConstraint
                                                          ValueConstraint
                                                          ValueRange
                                                          ValueType
```

## NORMA value types (22)

```
Alias              DerivationStorageType   Length         Position_nr
BinaryPrecision    DerivationType          LexicalValue   RegexPattern
Clusivity          DigitCount              Modality       RingConstraintType
ConstraintType_code Frequency              Name           SubTypeDerivationRuleType_code
                    JoinType_name          Note
                                          ObjectTypeKind
                                          ORMModelElement_id
                                          ORMNamedElementKind
```

## What we already have, matched

| NORMA concept | `core.md` anchor |
|---|---|
| `ObjectType` | `Noun` |
| `EntityType` (subtype of ObjectType) | Noun w/ `Object Type = 'entity'` |
| `ValueType` | Noun w/ `Object Type = 'value'` |
| `FactType` | `Fact Type` (subtype of `Noun`) |
| `Role` | `Role` |
| `Reading` | `Reading` |
| `Constraint` | `Constraint` |
| `ConstraintType` | `Constraint Type` |
| `Modality` | `Modality Type` (Alethic/Deontic) |
| `UniquenessConstraint`, `MandatoryConstraint`, `FrequencyConstraint`, `SubsetConstraint`, `EqualityConstraint`, `ExclusionConstraint`, `RingConstraint`, `ValueComparisonConstraint` | Constraint Types `UC/MC/FC/SS/EQ/XC/IR/AS/AT/SY/IT/TR/AC/VC` (instance facts) |
| `DerivationRule` | `Derivation Rule` |
| `DerivationType` | `Derivation Mode` (fully-derived / derived-and-stored / semi-derived) |
| `Subtype` | `Noun is subtype of Noun` (ring FT) |
| `Name`, `Description`, `Text`, `Pattern` | value types present |
| `RingConstraintType` | via Constraint Type instance facts |

## Gaps — NORMA concepts not in `core.md`

### Structural decomposition primitives — critical for #280

These are the concepts that NORMA uses to decompose a derivation rule's body into evaluable parts. They are what the AREST classifier cascade is simulating with string pattern matching.

| Concept | What it is | Why we need it |
|---|---|---|
| `RoleSequence` | Ordered sequence of roles that a constraint or derivation path traverses | Constraint spans become first-class; today we flatten span0/span1/... into fields |
| `RoleProjection` | A projection from a role into a derivation-rule result | The consequent's role binding to an antecedent path |
| `JoinPath` | Path through one or more fact types joined on shared roles | Replaces `that <Noun>` anaphora as a proper navigation primitive |
| `Join` | A single join step within a JoinPath | Atomic decomposition of multi-hop rules |
| `JoinType` | `InnerJoin`, `OuterJoin`, etc. | Semantic for `has no X` and `some X` quantifiers |
| `FactTypeDerivationRule` vs `SubtypeDerivationRule` | Two kinds of derivation rules | We conflate them; FORML 2 distinguishes |
| `DerivedFactType` | A fact type that is `fully-derived` | Marker is in core.md, but the derived-fact-type entity isn't |

**What this means for #280:** instead of inventing "Antecedent Clause Shape" (a flat enum of 13 classifier categories), the meta-circular parser should decompose rule bodies into `JoinPath` + `Join` + `RoleSequence` + `RoleProjection` instances. This is NORMA's design, paper §4 Table 1's formal shape, and the AREST `ast::Func` hierarchy (Composition / Selector / ApplyToAll) is already isomorphic to it.

### Value-domain primitives — medium priority

| Concept | What it is | Status in AREST |
|---|---|---|
| `Bound` / `ValueRange` / `Clusivity` | Inclusive/exclusive min/max for value-range constraints | We have `Minimum`, `Maximum`, `Exclusive Minimum`, `Exclusive Maximum` as flat fields on Noun — not first-class entities |
| `Facet` | Value type precision/scale/length constraints | Implicit (`Min Length`, `Max Length`, `Format`, `Pattern`) |
| `Unit` / `NonUnitValueType` / `Dimension` | Units of measure (kilograms, USD) | Not represented |
| `Value` | A literal value in an instance fact | Encoded as `Atom` in FFP — no entity form |
| `LexicalValue` | The string form of a Value | Implicit |

### Constraint surface — small gap

| Concept | Status |
|---|---|
| `CardinalityConstraint` | We have Frequency; NORMA separates cardinality on object types (e.g., "at most 5 Widgets") from role frequencies |
| `SubtypeExclusionConstraint` / `SubtypeTotalityConstraint` | We have `{A, B} are mutually exclusive subtypes` syntax; NORMA models them as first-class constraints |
| `TextualConstraint` | Free-text deontic or informal rules. We don't have — all constraints are structural today |
| `Alias` | Alternate names for object types / fact types. Our `Plural` is one case; NORMA generalizes |

### Model / naming primitives — low priority

| Concept | Status |
|---|---|
| `Model` / `ORMModelElement` / `ORMNamedElement` | We have `Domain`; NORMA has a richer ownership tree |
| `Note` | Annotation on any element. Could subsume `Description` |
| `Position` | Reading position of a role (for `{0} has {1}` templates) | Missing (Reading facts don't expose placeholder order beyond role sequence) |

## Concepts AREST has that NORMA doesn't

- `Derivation Mode = 'fully-derived' / 'derived-and-stored' / 'semi-derived'` as value type (NORMA calls these `DerivationStorageType` — equivalent, but we lack the `DerivedFactType` entity)
- `External System` + URL/Header/Prefix/Kind federation surface — paper §5.3 addition, outside ORM 2's scope
- `HTTP Method` — platform-binding surface, outside ORM 2
- `Permission`, `Role Relationship`, `Scope`, `World Assumption` — runtime concerns layered on top of ORM

## Recommendation for #279

Do **not** invent `Antecedent Clause Shape` as a flat enum. Instead, extend `readings/core.md` with NORMA's structural decomposition — `RoleSequence`, `RoleProjection`, `JoinPath`, `Join`, `JoinType` — and let the derivation rules in #280 work over those. The 13 classifier categories from the engine-resolver-gaps report then become:

| Category | Decomposition in NORMA terms |
|---|---|
| Fact Reference | `JoinPath` with a single `Join` |
| Comparator | `ValueComparisonConstraint` in antecedent scope |
| Aggregate | `RoleProjection` with an aggregate function (count/sum/...) |
| Computed Binding | `RoleProjection` with an arithmetic expression |
| Literal Filter | `JoinPath` + `ValueConstraint` on the last role |
| Ref Scheme Literal | `JoinPath` from entity to its preferred identifier's value |
| Range Filter | `ValueRange` applied to a `JoinPath` tail |
| Subtype Check | `SubtypeConstraint` applied inline |
| Anaphora | `RoleSequence` reuse within a single `JoinPath` |
| Temporal | Platform predicate (runtime `Function`) — same as `httpFetch` |
| Universal Quantifier | `JoinPath` with modifier (paper §4 `α`) |
| Existential FT Reference | `JoinPath` with `some` quantifier |
| Extraction | Platform predicate (runtime `Function`) |

Each of those has a structural shape that readings can describe declaratively. #280's derivations become joins/filters over `JoinPath` + `Role` + `RoleSequence` cells rather than string pattern matches.

## Deliverable summary

- `readings/core.md` gains ~12 entity types (`RoleSequence`, `RoleProjection`, `JoinPath`, `Join`, `JoinType`, `Bound`, `ValueRange`, `Facet`, `Dimension`, `Unit`, `Value`, `TextualConstraint`) and ~8 value types (`Clusivity`, `DerivationStorageType`, `RegexPattern`, `LexicalValue`, `Alias`, `Length`, `BinaryPrecision`, `DigitCount`).
- `ast::Func` needs no new primitives — NORMA's `JoinPath` = `Composition`, `RoleProjection` = `Selector`, `RoleSequence` = `Construction`, `JoinType` = `Condition` per paper Table 1.
- Recommend #282 (text-pattern FFP primitives) stays **not-needed**: the meta-circular parser operates over the decomposed join-path structure, not raw clause text. Text lives only on the `Reading` — once a rule body is decomposed into a JoinPath, no regex is required.

## Cross-references

- Paper: §4 Formal Foundations Table 1 (FFP ↔ domain mapping) and §5.4 (RMAP over fact types).
- NORMA: `Documentation/ORMCoreMetaModel.orm` for the authoritative entity/fact-type declarations; readings embedded inline as `<orm:Reading><orm:Data>{0} has {1}</orm:Data></orm:Reading>` — the templated verbalization form.
- `_reports/engine-resolver-gaps.md` §A–§K for the 13 classifier categories this audit maps onto NORMA decomposition.
