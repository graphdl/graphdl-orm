# JS Library Imports

## Entity Types

JS Package(.Name) is an entity type.

## Value Types

Module Path is a value type.
Symbol Name is a value type.
Version is a value type.
Package Manager is a value type.
  The possible values of Package Manager are 'npm', 'yarn', 'pnpm', 'bun', 'deno', 'jsr'.

## Fact Types

### JS Package
JS Package has Version.
  Each JS Package has at most one Version per Domain.
  It is possible that the same JS Package has more than one Version across Domains.

JS Package has Description.
  Each JS Package has at most one Description.

JS Package has Package Manager.
  Each JS Package has at most one Package Manager.

### Verb
Verb is exported from JS Package.
  Each Verb is exported from at most one JS Package.
  It is possible that some JS Package exports more than one Verb.

Verb has Module Path.
  Each Verb has at most one Module Path.

Verb has Symbol Name.
  Each Verb has at most one Symbol Name.

Verb has Description.
  Each Verb has at most one Description.

## Constraints

It is obligatory that each Verb exported from some JS Package has some Module Path.
It is obligatory that each Verb exported from some JS Package has some Symbol Name.

It is forbidden that a Verb is exported from a JS Package and also is backed by an External System.

## Instance Facts

Domain 'imports' has Access 'public'.
Domain 'imports' has Description 'JS library imports as a federation primitive. Verbs exported from a JS Package are bound at runtime via DEFS the same way HTTP-backed Verbs are.'.
