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
  <!-- ContentRef is a tagged Object value (#401). The concrete shape
       is a 2- or 3-element FFP sequence whose head atom selects the
       discriminant, mirroring the existing `<CELL, name, contents>`
       convention (see crates/arest/src/ast.rs CELL_TAG):

         Inline path     <INLINE, hex-bytes>
         Region path     <REGION, base-sector, byte-len>

       INLINE / REGION are atom literals; hex-bytes is a lowercase
       ASCII hexadecimal encoding of the raw blob (chosen over base64
       so the value is readable in serial logs and survives a freeze /
       thaw round-trip without an alphabet dependency); base-sector
       and byte-len are decimal-string atoms (u64). Total wire form:

         <INLINE, "deadbeef">
         <REGION, "8192", "131072">

       The 64 KiB inline cap is the encoder's hard switch — at or
       below 64 KiB the encoder emits Inline, above it allocates a
       region via `arest_kernel::block_storage::alloc_region` and
       emits Region. The cap is one DO write payload, comfortable Vec
       resize bound, and small enough that a typical row-shape store
       stays compact even when many cells live inline.

       The region path is backed by a fixed slot table in
       `arest_kernel::block_storage` — 256 slots × 256 KiB each =
       64 MiB max region-backed blob storage, on a virtio-blk disk
       sized to cover sector 8192 + 256 × 512 = sector 139264 (≥ 80
       MiB). Smaller disks fail allocation with `Error::OutOfRange`
       and the consumer must fall back to inline-or-fail. The
       free-list is in-memory only this commit; rebuild-from-File-
       table on mount lands in a follow-up.

       No Object variant was added for blob bytes; the encoding
       sits inside the existing Object::Atom + Object::Seq surface
       so the 55 pattern-match sites across the engine continue to
       compile unchanged. The encoder/decoder lives in the file-ops
       layer (`crates/arest/src/blob.rs`) where it is wired by the
       consumer that lands File create/read; this commit ships the
       on-disk allocator + the encoding spec only. -->

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
