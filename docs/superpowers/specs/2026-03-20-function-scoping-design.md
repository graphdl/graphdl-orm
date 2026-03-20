# Function Scoping: App-Scoped Resolution for Multi-Domain Ingestion

## Problem

When ingesting a multi-domain project like `support.auto.dev`, each domain is processed independently. Cross-domain dependencies — such as the UI domain's `Status 'Received' has Display Color 'blue'` referencing a Status created by the support domain's state machine — rely on insertion order and ad-hoc cross-domain lookups (`resolveNounAcrossDomains`, `resolveReadingAcrossDomains`). This is fragile and produces silent failures when seeding order doesn't match dependency order.

## Insight: Noun is a Function

The metamodel currently models Function as an independent entity type in `state.md` (declaration) with fact types in `core.md` (callback URI, HTTP Method, Function Type, Header). This is inverted. In first-order logic, a Noun IS a function — a unary predicate that classifies entities. A Reading is a binary predicate relating entities. A Constraint is a logical formula over predicates. The entire domain model is a system of functions.

The FOL engine is not a bolt-on for evaluating guards — it is the runtime. Everything in the model is a function. The metamodel should reflect this.

## Model

### Metamodel Change

Noun becomes a subtype of Function. Function becomes the fundamental type from which Noun inherits.

The existing Function entity type has properties that are specific to executable/callable functions: `callback URI`, `HTTP Method`, `Function Type`, `Header`. With Noun as a subtype of Function, these properties would incorrectly apply to every Noun. To resolve this:

- Function becomes the supertype with only the properties that apply to all functions: `Name` and `Scope`
- The existing callable-specific properties (`callback URI`, `HTTP Method`, `Function Type`, `Header`) remain on Function but are optional (each has "at most one" constraints already). Nouns simply don't populate them — they are nullable columns that only executable functions use.
- `Verb executes Function` continues to work — a Verb can execute any Function, including a Noun (which would invoke the FOL engine to evaluate the predicate).

How a function is evaluated (predicate logic, HTTP dispatch, derivation) is a runtime concern, not a domain concept. The model does not distinguish between predicate and imperative functions.

### Changes to core.md

Current state:
- `Noun(.id) is an entity type.` (line 6)
- `Function(.id) is an entity type.` in state.md (line 12)
- Function fact types in core.md (lines 205-215)

New state:
```
Function(.id) is an entity type.
Noun is a subtype of Function.
  Graph Schema is a subtype of Noun.
  Status is a subtype of Noun.
  {Graph Schema, Status} are mutually exclusive subtypes of Noun.
```

The existing subtype hierarchy under Noun is preserved — Graph Schema and Status remain subtypes of Noun, which is now a subtype of Function.

Function's existing fact types (`Function has Name`, `Function has Function Type`, etc.) remain unchanged in core.md. They now apply to all Functions including Nouns, but all are "at most one" (optional), so Nouns simply leave them unpopulated.

Remove from state.md: `Function(.id) is an entity type.` (line 12). The declaration moves to core.md as the supertype.

### Scoping

Add `Scope` as a value type:

```
Scope is a value type.
  The possible values of Scope are 'local', 'app', 'organization', 'public'.
```

Add scoping to Function:

```
Function has Scope.
  Each Function has at most one Scope.
```

Visibility follows the scope chain:

| Scope Level | Visible To |
|---|---|
| local | The defining Domain only |
| app | All Domains within the same App |
| organization | All Domains within the same Organization |
| public | All Domains |

The existing `Visibility` value type in organizations.md controls Domain-level visibility. `Scope` on Function controls function-level visibility. They work together: a Domain's Visibility determines the default Scope for its Functions.

Resolution follows the scope chain: local, then app, then organization, then public. First match wins.

### Readings Update (organizations.md)

Add derivation rules for domain visibility:

```
Domain is visible to Domain.
  Domain is visible to Domain := that Domain is the same Domain.
  Domain is visible to Domain := Domain has Visibility 'public'.
  Domain is visible to Domain := Domain belongs to App and that Domain belongs to the same App.
  Domain is visible to Domain := Domain belongs to Organization and that Domain belongs to the same Organization.
```

## Ingestion Design

### Mental Model

All `domains/*.md` files in a project are chapters of one document. Each chapter is scoped to its domain slug, but resolution happens across the whole document. The ingestion builds one scope from all definitions, then resolves everything against that scope.

### Pipeline: Decompose and Recompose

The current `ingestClaims()` has 6 inline steps. Extract each into a standalone function:

```
Step 1: ingestNouns(db, nouns, domainId, scope)
Step 2: ingestSubtypes(db, subtypes, scope)
Step 3: ingestReadings(db, readings, domainId, scope)
Step 4: ingestConstraints(db, constraints, domainId, scope)
Step 5: ingestTransitions(db, transitions, domainId, scope)
Step 6: ingestFacts(db, facts, domainId, scope)
```

Note: Step 4 is named `ingestConstraints` (not `applyConstraints`) to avoid shadowing the existing `applyConstraints` function in `src/claims/constraints.ts`, which handles the low-level constraint+span record creation. `ingestConstraints` wraps the higher-level logic (finding the host reading, resolving roles, then calling `applyConstraints`).

Each step takes a `scope` — the shared resolution context (accumulated noun map, schema map, errors). Each step reads from and writes to the scope.

**Single-domain** (`ingestClaims`) — thin wrapper, calls steps 1-6 sequentially for one domain. Returns the same `IngestClaimsResult` type (nouns count, readings count, stateMachines count, skipped count, errors array) by reading from the scope. Backward compatible — existing tests must pass unchanged.

**Multi-domain** (`ingestProject`) — for each step, runs it across all domains before advancing:

```
for each step in [1, 2, 3, 4, 5, applySchema, 6]:
  for each domain in project:
    step(db, domain.claims, domain.id, scope)
```

The `applySchema` call between steps 5 and 6 ensures columns like `display_color` exist on metamodel tables before instance facts try to write to them. The current lazy `applySchema` call inside step 6 (ingest.ts lines 583-588) must be removed when using `ingestProject`, since schema is applied as a dedicated phase.

### Scope Object

The scope is the running glossary of the document:

```typescript
interface Scope {
  /** domain:name -> noun record (qualified key to handle same-name nouns across domains) */
  nouns: Map<string, NounRecord>
  /** reading text -> graph schema + roles */
  schemas: Map<string, SchemaRecord>
  /** accumulated errors */
  errors: string[]
}
```

Scope is not a persistent entity. It is ephemeral ingestion state — the resolution context for a single bulk upload.

Noun resolution within the scope follows the visibility rules: when domain B looks up noun "Status", the scope checks domain B first, then sibling domains in the same App, then the same Organization, then public. The scope keys are qualified (`domainId:nounName`) but resolution is done by name with the visibility cascade.

### ingestProject Entry Point

```typescript
async function ingestProject(
  db: GraphDLDB,
  domains: Array<{ domainId: string; claims: ExtractedClaims }>
): Promise<ProjectResult>
```

The caller (API endpoint, seed script) is responsible for:
1. Reading all `domains/*.md` files
2. Parsing each into `ExtractedClaims` via `parseFORML2`
3. Deriving the domain slug from the filename
4. Passing the array to `ingestProject`

The existing batch callers — `src/api/seed.ts` (lines 56-63) and `src/api/claims.ts` (lines 54-61) — currently loop over domains calling `ingestClaims` sequentially. These switch to a single `ingestProject` call.

### Error Handling

Unresolvable references are hard errors — collected in `scope.errors`, the fact is skipped. No deferred retry. An unresolvable reference means the domain files need to be corrected.

### Cross-Domain Resolution Functions

The existing `resolveNounAcrossDomains` and `resolveReadingAcrossDomains` functions in `ingest.ts` (lines 104-179) are superseded by scope-based resolution. They are removed. Their behavior (local → app → org → public lookup) is now handled by the scope's visibility-aware lookup, which resolves from the in-memory scope rather than querying the database per-lookup.

## What Changes

| Component | Change |
|---|---|
| `readings/core.md` | Function becomes supertype, Noun becomes subtype of Function. Add Scope value type and `Function has Scope` reading. |
| `readings/organizations.md` | Add Domain visibility derivation rules |
| `readings/state.md` | Remove `Function(.id) is an entity type.` declaration (moved to core.md) |
| `src/claims/ingest.ts` | Extract 6 steps into standalone functions. Remove `resolveNounAcrossDomains` and `resolveReadingAcrossDomains`. |
| `src/claims/ingest.ts` | `ingestClaims()` becomes thin wrapper (same return type, same behavior) |
| `src/claims/ingest.ts` | New `ingestProject()` composes steps across domains |
| `src/claims/scope.ts` (new) | Scope type definition and visibility-aware resolution helpers |
| `src/api/seed.ts` | Switch from per-domain `ingestClaims` loop to single `ingestProject` call |
| `src/api/claims.ts` | Switch from per-domain `ingestClaims` loop to single `ingestProject` call |
| `src/schema/bootstrap.ts` | Regenerate via `npx tsx scripts/generate-bootstrap.ts` after readings change |

## What Does Not Change

- `generateOpenAPI`, `generateSQLite`, `applySchema` — untouched
- `createEntity` upsert logic — untouched
- `CASCADE_MAP`, deletion order — untouched
- The 6-step ordering within a single domain — same steps, same order
- External API surface (`/seed`, `/api/parse`) — same endpoints, `ingestProject` is called internally
- `applyConstraints` in `src/claims/constraints.ts` — low-level constraint record creation, unchanged

## Data Migration

The metamodel change (Noun as subtype of Function) affects the bootstrap schema:
- The `nouns` table gains Function's columns (or Function becomes a supertable). Since the bootstrap is regenerated from readings, running `npx tsx scripts/generate-bootstrap.ts` produces the new DDL.
- Existing Noun rows do not need migration — Function's properties (`callback_uri`, `http_method`, `function_type`, `header`) are all optional ("at most one"), so existing Nouns have NULL values for these columns.
- No data loss. The schema change is additive (new nullable columns on existing table or new supertable with FK).

## Testing

1. **Step extraction**: Each extracted step function gets its own unit test verifying it produces the same result as the current inline code
2. **Scope accumulation**: Test that nouns created in step 1 for domain A are resolvable by step 6 for domain B via the scope's visibility cascade
3. **Ordering guarantee**: Test that `applySchema` runs for all domains before any instance facts
4. **Error collection**: Test that unresolvable references produce errors, not silent skips
5. **Backward compatibility**: Existing `ingestClaims` tests in `src/claims/ingest.test.ts` must pass unchanged. `ingestClaims()` returns identical `IngestClaimsResult`.
6. **Regression**: Existing tests in `src/pipeline.test.ts` and all other test files must pass unchanged.
7. **Integration**: Parse all `support.auto.dev/domains/*.md`, ingest via `ingestProject`, verify Status display colors, Event Types, and Suggested Prompts all resolve correctly
8. **Name collision**: Test that two domains defining a noun with the same name (e.g., both define "Status") resolve to the correct domain-local version first
