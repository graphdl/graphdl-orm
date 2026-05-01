# JS Library Imports

JS library imports as a federation primitive — analogous to External System
for HTTP APIs (`docs/08-federation.md`). The reading declares which library
a Verb comes from and which symbol to bind; the runtime imports the package
and registers each declared symbol into DEFS via `register_runtime_fn(name,
func, state)`. The compile pass emits a `populate:{verb}` config carrying
the Module Path + Symbol Name so the host-side loader can issue the
equivalent of `import { symbol } from 'module'` at boot.

This is the JS-library analog of `Verb is backed by External System`. Both
ride the same DEFS dispatch surface; the difference is whether the runtime
issues an HTTP call or invokes a locally-resident JS function. Domains that
declare their Verbs against either binding compose without distinction at
the call site.

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

### Verb (extends core.md Verb subtype of Function)
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

## Derivation Rules

### A Verb resolves to a DEFS entry once compiled
### Compile pass emits `populate:{verb}` for each Verb whose binding is
### declared (HTTP-backed or JS-imported). The runtime walks the populate
### config to install the function into DEFS at boot, after which the
### Verb is callable through the same `Func::Platform(name)` dispatch
### path as any compile-derived binding.

## Instance Facts

Domain 'imports' has Access 'public'.
Domain 'imports' has Description 'JS library imports as a federation primitive. Verbs exported from a JS Package are bound at runtime via DEFS just like HTTP-backed Verbs are. The reading declares what the library exports and where to find it (Module Path + Symbol Name); the runtime maps that to the equivalent of an ES module import and registers each function into DEFS for uniform dispatch alongside the HTTP federation surface. Library readings (templates/vercel-ai.md, templates/vercel-chat.md, etc.) provide the instance facts.'.
