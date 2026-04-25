# AREST Filesystem: File, Directory, Tag

Foundation nouns for the file-browser domain (epic #397). The nouns
and binary facts were declared in #398; the ring-acyclic parentage
plus mandatory File containment invariants land in #399 (this file).

## Entity Types

File(.id) is an entity type.
Directory(.id) is an entity type.
Tag(.id) is an entity type.

## Value Types

MimeType is a value type.
Size is a value type.
ContentRef is a value type.
  <!-- TODO(#397d): resolve ContentRef encoding. Intended shape is a
       tagged union of Inline(bytes) and RegionRef(base_sector, len);
       the concrete value-type decomposition (subtype partition? facet
       pattern? paired lexical encoding?) belongs in #397d once the
       block-storage region API lands. Until then ContentRef is an
       opaque scalar. -->

## Readings

### File

File has Name.
  Each File has exactly one Name.
  It is possible that more than one File has the same Name.

File has MimeType.
  Each File has exactly one MimeType.
  It is possible that more than one File has the same MimeType.

File has Size. *
  Each File has exactly one Size.
  It is possible that more than one File has the same Size.

File has ContentRef.
  Each File has exactly one ContentRef.
  It is possible that more than one File has the same ContentRef.

File has created-at Timestamp.
  Each File has exactly one created-at Timestamp.
  It is possible that more than one File has the same created-at Timestamp.

File has owner User.
  Each File has exactly one owner User.
  It is possible that more than one File has the same owner User.
  <!-- User is the entity type declared in readings/organizations.md
       (User(.Email)). No dedicated Person noun exists in core today;
       #397 is content with User as the ownership subject. If a richer
       subject (service account, agent) is later needed, promote this
       role to a supertype noun in a follow-up. -->

File is in Directory.
  Each File is in exactly one Directory.
  It is possible that more than one File is in the same Directory.

### Directory

Directory has Name.
  Each Directory has exactly one Name.
  It is possible that more than one Directory has the same Name.

Directory has parent Directory.
  Each Directory has at most one parent Directory.
  It is possible that more than one Directory has the same parent Directory.

<!-- Directory-in-Directory containment: the canonical formulation is
     "Directory has parent Directory" above. The alternative reading
     "Directory is in Directory" would cover the same fact population
     and is intentionally omitted to keep a single source of truth for
     the containment edge. The ring-acyclic constraint is attached
     under "## Ring Constraints" below. -->

### Tag

Tag has Name.
  Each Tag has exactly one Name.
  Each Name belongs to at most one Tag.

Tag has Description.
  Each Tag has at most one Description.
  It is possible that more than one Tag has the same Description.

### File-Tag association

File has-tag Tag.
  It is possible that some File has-tag more than one Tag.
  It is possible that some Tag is applied to more than one File.
  For each pair of File and Tag, that File has-tag that Tag at most once.

## Constraints

<!-- Mandatory File-in-Directory containment is already declared by the
     `Each File is in exactly one Directory.` line under "File has in
     Directory" above (forml2-grammar: 'exactly one' → Uniqueness +
     Mandatory Role on the File side). No extra sentence needed here. -->

## Ring Constraints

No Directory has parent itself.
No Directory may cycle back to itself via one or more traversals through has parent.

## Derivation Rules

* File has Size iff File has ContentRef and Size is the byte-length of ContentRef.

## Instance Facts

Domain 'filesystem' has Access 'public'.
Domain 'filesystem' has Description 'File-browser foundation (epic #397). Declares the File, Directory, and Tag nouns and their binary facts plus the containment invariants (ring-acyclic Directory parentage, mandatory File-in-Directory containment) added in #399.'.
