# NORMA hyphen-binding + relational column-naming spec (#24)

Source study of the NORMA implementation (C:\Users\lippe\Repos\NORMA), 2026-07-10, for
aligning AREST's hyphen binding and RMAP column naming. Samuel's directive: both must
FOLLOW NORMA — this is the citation-grounded spec.

## A. Hyphen binding in readings

The whole mechanism is `VerbalizationHyphenBinder` (ORMModel/ObjectModel/Verbalization.cs:
1355-1990). Hyphen binding is a READING-TEXT PARSING CONVENTION, not a stored field — the
reading's raw Text is authoritative, re-parsed on demand by one compiled regex (:1428;
human-readable form :1388-1420).

Syntax (placeholder = {N} for role N):
- LEADING adjective: `adjective-␣{N}` — the hyphen TOUCHES the adjective, then whitespace,
  then the role. Guard requires the literal `\S-\s`. Binds the word to the FOLLOWING role.
  Corpus example: `{0} has parent- {1}` (Tests.Test1.Compare.orm:3196).
- TRAILING adjective: `{N}␣-adjective` — whitespace, then the hyphen touching the word.
  Binds to the PRECEDING role.
- `parent-{1}` (no space before the field) does NOT bind.
- ONE word per hyphen (`\S+` maximal run). Multi-word adjectives chain internal hyphens
  (`very-fast- {0}` binds "very-fast"; `very fast- {0}` binds only "fast").
- Double-hyphen escape keeps a literal hyphen: `FORE-- WORD` -> `FORE-WORD`
  (:1907-1911; NormalizeLeftHyphen :1915-1926 / NormalizeRightHyphen :1934-1946).
- Serialization (Reading.cs:1043-1130): the .orm stores raw text in <orm:Data> AND a
  derived cache <orm:ExpandedData><orm:RoleText PreBoundText=... RoleIndex=N/>; the cache
  regenerates from Data at write time — Data is the source of truth.
- Adjective vs Role.Name are DISTINCT concepts: Role.Name is model-level (order- and
  reading-independent); the adjective is reading-local. Role.Name WINS in naming.

## B. Relational column naming

`DefaultDatabaseNameGenerator.GenerateColumnName(Column, phase)` —
RelationalModel/OialDcilBridge/NameGeneration.cs:523-845. Per ordinary column-path step,
name parts emit with this precedence (if / else-if / else at :670 / :733 / :738):

1. Predicate text from a matching reading (:670-732) — COLLISION-ONLY fallback
   (decorateWithPredicateText = phase==1, :529) or unaries.
2. Explicit far Role.Name (:733-736) — emitted ALONE (no object-type name).
3. Hyphen-bound adjective(s) wrapped around the far object-type name + reference-mode
   identifier (:738-824; format via GetFormatStringForHyphenBoundRole :748; player name
   via ReferenceModeNaming.ResolveObjectTypeName :807-818).
4. Object-type name + ref-mode identifier alone (branch 3's base case).

Collisions: Utility.GenerateUniqueNames (NameGeneration.cs:126) re-requests colliding
elements at phase+1 (Utility.cs:918); phase 1 turns on predicate-text decoration; last
resort literal "COLUMN" (:842).

Composition — NamePart.GetFinalName (ORMModel/ObjectModel/NamePart.cs:331-397):
- split every part on space, '-', and camel/Pascal boundaries (NameDelimiterArray :143);
- COLLAPSE adjacent duplicate words (:228-308 — why self-references don't yield FolderFolder);
- casing DoFirstWordCasing (:704-720): Camel = first word lower, later words Pascal;
  acronym-like words left as-is;
- spacing GetSpacingReplacement (:398-415).

DEFAULTS (RelationalNameGenerator, NameGeneration.cs:1663-1674): columns = Camel + Remove
(camelCaseNoSpaces); tables = Pascal + Remove (PascalCaseNoSpaces).

## The Transition example (the exact from/to-Status pattern)

- (a) explicit role names From Status / To Status -> `fromStatus` / `toStatus`;
- (b) readings `... is from- Status` / `... is to- Status` -> adjective + player (+ ref-mode
  id) -> `fromStatus` / `toStatus` (or fromStatusId/toStatusId when the ref mode contributes);
- (c) neither -> both columns want `status` -> collision -> phase 1 prepends predicate text
  -> `isFromStatus` / `isToStatus`.

Proof sample: TestSuites/RelationalTests/FullRegeneration/Tests.Test1.Compare.orm — the
self-referencing FolderHasParentFolder: reading `{0} has parent- {1}` (:3196-3200) feeds
OIAL oppositeName "parent Folder" (:15292) and produces column Name="parentFolderId" on
the Folder table (:16387) beside the PK folderId — the hyphen adjective IS the FK
disambiguator. Same pattern: taskParentTaskId, taskAssociatedFileId (:3824/16205/16210).

## What AREST must do to match (the #24 work-list)

1. PARSER: REPLACE the touching-hyphen bind with NORMA syntax (Samuel, 2026-07-10: "I
   don't like the touching hyphen. It should be NORMA syntax."). Today AREST forward-binds
   `from-Status` (hyphen touching BOTH sides, compiler.py:608-611) — that form is RETIRED:
   a touching hyphen is just a word (which also fixes the latent mis-bind of hyphenated
   tokens whose suffix happens to be a known type). The bind forms become exactly NORMA's:
   leading `adj- {N}` (hyphen touches the adjective, whitespace before the role), trailing
   `{N} -adj`, the `--` literal escape, one word per hyphen (chain internal hyphens for
   multi-word). Migrate any corpus readings using the touching form (scenarios.canon
   case:lex-hyphen carries `valence-Coord` — update the case with the syntax). Keep the
   reading text as the source of truth; the ftid slug already collapses `from- Status` to
   Transition_is_from_Status (verified — cells/rules/stores unchanged by respelling).
2. RMAP NAMING: implement the four-step precedence + phase-1 collision decoration when
   projecting columns (role-qualifier concept: AREST has no Role.Name property yet — the
   hyphen adjective is the available naming source; add Role.Name later if wanted).
3. COMPOSITION: split on space/hyphen/camel boundaries, collapse adjacent duplicates,
   camelCase columns / PascalCase tables, spaces removed — yielding fromStatus/toStatus
   and parentFolderId identically to NORMA.
