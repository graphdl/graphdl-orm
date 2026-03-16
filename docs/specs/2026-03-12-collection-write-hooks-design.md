# Collection Write Hooks — Deterministic Parse-on-Write

Restore the parse-on-write behavior lost when graphdl-orm migrated from Payload/Next.js to Cloudflare Workers. Creating a framework object (Reading, Constraint, Noun, StateMachineDefinition) should deterministically produce all associated child objects. This is a prerequisite for simplifying the claims ingestion pipeline and enabling a combined extraction endpoint where graphdl-orm provides deterministic claims and apis provides semantic (LLM) claims.

## Architecture

### Hook Registry

A `COLLECTION_HOOKS` map in `src/hooks/index.ts` maps collection slugs to hook functions:

```typescript
type AfterCreateHook = (
  db: DurableObjectStub,
  doc: Record<string, any>,
  context: { domainId: string; allNouns: Array<{ name: string; id: string }> }
) => Promise<HookResult>

interface HookResult {
  created: Record<string, any[]>  // keyed by collection slug
  warnings: string[]
}
```

Hooks run in the Worker context (not inside the DO), receiving the `DurableObjectStub` — the same execution context as the existing POST handler and `ingestClaims()` via the seed endpoint. Hooks call `db.createInCollection()`, `db.findInCollection()`, etc. via RPC.

### Hook Dispatch

A shared `createWithHook(db, collection, data, context)` function handles both the create and the hook invocation. This function is called by:

- The POST handler in `router.ts` (for HTTP creates)
- Other hooks that need to create child objects (e.g., Reading hook creating Constraints)

This ensures hooks compose recursively: a Reading hook calling `createWithHook('constraints', ...)` triggers the Constraint hook automatically, without going through the HTTP layer.

The generic POST handler in `router.ts` calls `createWithHook()` instead of `db.createInCollection()` directly. The response includes `{ doc, created, warnings }` alongside the standard `{ doc, message }`.

PATCH gets the same treatment via `afterUpdate` hooks for when reading/constraint text changes.

### Hook Files

```
src/hooks/
  index.ts          — COLLECTION_HOOKS map, createWithHook(), HookResult type
  readings.ts       — afterCreate/afterUpdate for readings
  constraints.ts    — afterCreate/afterUpdate for constraints
  nouns.ts          — afterCreate for nouns (subtype parsing)
  state-machines.ts — afterCreate for state-machine-definitions
  parse-constraint.ts — deterministic natural language constraint parser
```

Each hook is a pure function (no Payload dependency, no LLM calls) that receives the DO stub, the just-created doc, and domain context. All hooks use find-or-create ("ensure") patterns for idempotency — creating the same reading twice does not produce duplicate graph schemas, roles, or constraints.

Note on types: the hook's `db` parameter is typed as `DurableObjectStub`, while existing reused functions like `applyConstraints()` are typed as `GraphDLDB`. At runtime these share the same RPC interface (`createInCollection`, `findInCollection`, etc.) via Cloudflare's Durable Object RPC. Implementation should either define a shared interface type or use `as any` casts as the existing codebase does.

## Schema Changes

### Add `text` column to `constraints` table

Constraints must store their source text for round-tripping and debugging. Add via DO migration:

```sql
ALTER TABLE constraints ADD COLUMN text TEXT;
```

The `text` column stores the natural language constraint (e.g., "Each Customer has at most one Name."). It is nullable — constraints created via the shorthand `multiplicity` path (from `ExtractedClaims`) may not have source text.

### Extend `kind` CHECK constraint

The current CHECK allows: `'UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO'`. Add `'RC'` for ring constraints:

```sql
-- SQLite does not support ALTER CHECK; this requires recreating the table or
-- using the DO migration path to drop and recreate the CHECK.
-- The migration adds RC to the valid kinds:
kind TEXT NOT NULL CHECK (kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'RC'))
```

Ring constraint subtypes (irreflexive, acyclic, symmetric, asymmetric, transitive, antisymmetric, intransitive) are stored in the `text` column. The `kind` is always `'RC'`.

## Hook Specifications

### Reading Hook (`readings`)

Input: a Reading doc with `text` field containing one or more lines.

```
Customer has Name.
  Each Customer has at most one Name.
```

Behavior:

1. **Split text on newlines.** First non-indented line is the fact type reading. Indented lines (leading whitespace) are constraint declarations.
2. **Tokenize the reading** via `tokenizeReading()` against domain nouns to extract noun refs and predicate.
3. **Find-or-create nouns** for any noun in the reading that doesn't exist in the domain. Heuristic: if a noun appears only as the object of a "has" predicate, create as value type; otherwise create as entity type.
4. **Find-or-create graph schema** — `name` = concatenated noun names (e.g., "CustomerName"), `title` = reading text (e.g., "Customer has Name"). Note: the existing `ingestClaims` sets `title` to the noun concatenation (same as `name`). This spec intentionally changes `title` to the reading text for better readability. Graph schema lookup uses `name + domain_id` as the match key, so this change is compatible. `ingestClaims` should be updated to match when it is simplified.
5. **Find-or-create roles** — one Role per noun ref, linked to the graph schema, with `roleIndex`.
6. **Delegate constraints** — for each indented constraint line, call `createWithHook('constraints', { text, domain })`. This triggers the Constraint hook, which handles its own parsing.

The Reading hook does NOT parse constraint syntax. It recognizes indented lines as constraints and delegates.

### Constraint Hook (`constraints`)

Input: a Constraint doc. The hook accepts two input formats:

- **Natural language** (has `text` field): e.g., `{ text: "Each Customer has at most one Name.", domain: "..." }`
- **Shorthand notation** (has `multiplicity` field): e.g., `{ multiplicity: "*:1", reading: "Customer has Name.", domain: "..." }` — from `ExtractedClaims`

The hook detects format and dispatches accordingly:

- If `text` is present → `parseConstraintText(text)`
- If `multiplicity` is present → `parseMultiplicity(multiplicity)` (existing function)

#### Natural Language Parsing

`parseConstraintText()` in `src/hooks/parse-constraint.ts` is a pure function:

```typescript
interface ParsedConstraint {
  kind: 'UC' | 'MC' | 'RC'
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  nouns: string[]  // noun names extracted from the text
}

function parseConstraintText(text: string): ParsedConstraint[] | null
```

Returns an **array** (not singular) because some patterns produce multiple constraints:

| Pattern | Result |
|---------|--------|
| "Each X has/belongs to at most one Y" | `[{ kind: 'UC', modality: 'Alethic' }]` |
| "Each X has exactly one Y" | `[{ kind: 'UC' }, { kind: 'MC' }]` (both Alethic) |
| "Each X has at least one Y" | `[{ kind: 'MC', modality: 'Alethic' }]` |
| "For each pair of X and Y, that X ... that Y at most once" | `[{ kind: 'UC' }]` (spanning) |
| "For each combination of X and Y, that X has at most one Z per that Y" | `[{ kind: 'UC' }]` (on X,Y roles) |
| "No X [verb] itself" | `[{ kind: 'RC', modality: 'Alethic' }]` |
| "It is obligatory that ..." | Parse inner, set `modality: 'Deontic'` |
| "It is forbidden that ..." | Parse inner, set `modality: 'Deontic'` |
| "It is permitted that ..." | Parse inner, set `modality: 'Deontic'` |

Unrecognized text returns `null`. The constraint is stored as-is (kind defaults to `'UC'`, no constraint-spans), with a warning.

#### Host Reading Resolution

After parsing, the hook must find the host reading to create constraint-spans:

1. Extract nouns from the parsed constraint.
2. Query readings in the domain for a reading whose noun set is an **exact match** (same nouns, same order) to the constraint's nouns.
3. If exactly one match → use it. If multiple matches → prefer the reading whose predicate best matches the constraint's verb. If still ambiguous → warn and use the first match.
4. Once the host reading is found, resolve which roles the constraint spans based on noun positions in the reading. For binary readings: "Each X ..." constrains role 0 (the X role). For spanning constraints: both roles.
5. Create Constraint record (with `kind`, `modality`, `text`, `domain_id`) and ConstraintSpan records linking to the resolved roles.

#### Rejection

If the host reading is NOT found and this is NOT a batch operation → reject. Do not create the constraint. Return an error.

### Batch Constraint Deferral

Within a batch operation (a single request creating multiple readings and constraints, such as `ingestClaims` or a multi-line reading file):

1. Process all items in order.
2. Constraints whose host reading hasn't been created yet are deferred (not immediately rejected).
3. After all items in the batch are processed, retry deferred constraints. Their host readings may now exist.
4. Any constraints still unresolved after the retry pass are rejected with warnings in the response.

This allows constraints to appear before their host reading within the same file/request.

For single-object creates (POST one Constraint): no deferral. The host reading exists or it doesn't.

### Noun Hook (`nouns`)

Input: a Noun doc. If the noun includes subtype text like `"SupportRequest is a subtype of Request"`:

1. **Parse the text** — match pattern `"X is a subtype of Y"`.
2. **Find-or-create the parent noun** (Y) in the domain.
3. **Set the `superType` FK** on the created noun to point to the parent.

### StateMachineDefinition Hook (`state-machine-definitions`)

Input: a StateMachineDefinition doc with title and associated transition data.

1. **Find-or-create the target Noun** — the entity this state machine governs.
2. **Find-or-create Statuses** — each state mentioned in transitions, linked to this definition.
3. **Find-or-create EventTypes** — each event name.
4. **Create Transitions** — source status → target status, triggered by event type.
5. **Create Guards** — if guard text is provided, create guard records linked to transitions.

## Impact on `ingestClaims()`

Once hooks handle object-level parsing, `ingestClaims()` simplifies to:

1. Create nouns → Noun hooks resolve subtypes.
2. Create readings → Reading hooks create graph schemas, roles, and delegate constraints.
3. Create any remaining explicit constraints → Constraint hooks parse and link (with batch deferral).
4. Create state machine definitions → SM hooks create statuses, transitions, guards.
5. Collect warnings from rejected constraints.

The bulk of the current `ingestClaims()` logic (tokenization, constraint application, role creation) moves into the hooks. `ingestClaims()` becomes a thin batch loop with request-scoped constraint deferral.

## Existing Deterministic Functions Reused

| Function | File | Used By |
|----------|------|---------|
| `tokenizeReading()` | `src/claims/tokenize.ts` | Reading hook, Constraint hook |
| `parseMultiplicity()` | `src/claims/constraints.ts` | Constraint hook (shorthand notation from ExtractedClaims) |
| `applyConstraints()` | `src/claims/constraints.ts` | Constraint hook |
| `nounListToRegex()` | `src/generate/rmap.ts` | Reading hook, Constraint hook |
| `toPredicate()` | `src/generate/rmap.ts` | Reading hook |

## Future: Combined Extraction Endpoint

This spec is the foundation for a combined extraction API in `apis`:

```
POST /graphdl/extract → {
  deterministic: graphdl-orm parses FORML2 text via hooks,
  semantic: apis LLM extracts additional claims from natural language,
  combined: merge and deduplicate → ingest via /claims
}
```

The hooks must work correctly at the object level before this combined endpoint can be simple orchestration. This spec does not cover the combined endpoint — only the prerequisite object-level parsing.

## Testing

Each hook is a pure function testable with a mock DO stub:

- **Reading hook**: verify noun/role/graph-schema creation from reading text; verify idempotency on duplicate creates
- **Constraint hook (natural language)**: verify each pattern produces correct kind/modality/spans
- **Constraint hook (shorthand)**: verify `parseMultiplicity()` path produces same results
- **Constraint hook rejection**: verify rejection when host reading missing (single create)
- **Batch deferral**: verify constraints before readings resolve correctly within same batch
- **Batch deferral**: verify unresolved constraints rejected with warnings after retry pass
- **Noun hook**: verify subtype FK set correctly; verify parent noun auto-created
- **SM hook**: verify status/transition/event-type/guard creation
- **`parseConstraintText()`**: unit tests for every pattern in the table, including "exactly one" producing two constraints (UC + MC)
- **Hook composition**: verify Reading with indented constraints triggers both Reading and Constraint hooks
