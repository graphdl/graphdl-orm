# Combined Extraction Endpoints — Design Spec

Wire up the three dead proxy endpoints (`/extract`, `/check`, `/parse`) with real implementations, creating a clean API surface for both domain modeling (FORML2 → claims) and agent verification (prose → constraint warnings).

## Endpoint Contracts

| Route | Repo | Input | Output | Purpose |
|-------|------|-------|--------|---------|
| `POST /parse` | graphdl-orm | `{ text, domain }` | `ExtractedClaims` | Deterministic FORML2 parsing |
| `POST /verify` | graphdl-orm | `{ text, domain }` | `{ matches, unmatchedConstraints }` | Deterministic fact matching against prose |
| `POST /graphdl/extract` | apis | `{ text, domain?, seed? }` | `ExtractedClaims` | LLM semantic extraction (rename of `/extract/claims`) |
| `POST /graphdl/extract/all` | apis | `{ text, domain, seed? }` | `{ deterministic, semantic, combined }` | Calls `/parse` + `/extract`, merges |
| `POST /graphdl/verify` | apis | `{ text, domain }` | `{ warnings }` | Full agent verification in 1 call |
| `POST /graphdl/extract/semantic` | apis | `{ text, constraints }` | `{ claims }` | LLM deontic violation detection (unchanged) |

## Boundary Principle

graphdl-orm is open source — no LLM calls, no API keys, pure deterministic logic. apis adds authentication, LLM extraction, and orchestration.

## graphdl-orm: `POST /parse`

### Input

```typescript
{
  text: string    // Multi-line FORML2 text
  domain: string  // Domain ID
}
```

The `text` field contains one or more readings with optional indented constraints, separated by blank lines:

```
Customer has Name.
  Each Customer has at most one Name.

Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.
```

### Behavior

1. Split text into blocks on blank lines. Each block is a reading (first line) plus its indented constraints.
2. Create a batch `HookContext` with `batch: true` and an empty `deferred` array.
3. For each block, call `createWithHook('readings', { text: block, domain }, context)`.
4. After all blocks, retry deferred constraints (constraints that referenced readings appearing later in the text).
5. Collect all created objects into an `ExtractedClaims`-shaped response.

### Output

```typescript
interface ExtractedClaims {
  nouns: Array<{ name: string; objectType: 'entity' | 'value' }>
  readings: Array<{ text: string; nouns: string[]; predicate: string; multiplicity?: string }>
  constraints: Array<{ kind: string; modality: string; reading: string; roles: number[] }>
  subtypes: Array<{ child: string; parent: string }>
  warnings: string[]
}
```

Same shape as the LLM extractor returns (minus `transitions` and `facts` which are not expressed in FORML2 constraint syntax). This allows `/extract/all` to merge them trivially.

### Implementation

Reuses 100% of the existing hook chain — no new parsing logic. The route is a thin wrapper that feeds multi-line text through `createWithHook` in batch mode, then collects the results.

Register in `router.ts` alongside `/seed` and `/claims`.

## graphdl-orm: `POST /verify`

### Input

```typescript
{
  text: string    // Prose text (e.g., agent draft response)
  domain: string  // Domain ID
}
```

### Behavior

1. Load all readings, constraints, constraint-spans, and roles for the domain.
2. For each reading, tokenize the prose text to find fact instances — occurrences of the reading's nouns in proximity with a matching predicate.
3. For each constraint, check if the deterministic matches violate it:
   - UC violations: two instances share the same constrained role value.
   - MC violations: a required role has no instance.
   - RC violations: a self-referential instance matches.
4. Separate constraints into deterministically checked (violation found or definitively clear) and unmatched (need semantic analysis).

### Output

```typescript
{
  matches: Array<{
    reading: string       // Reading text
    instances: string[]   // Extracted fact instances from the prose
  }>
  unmatchedConstraints: string[]  // Constraint texts that need semantic checking
}
```

### Implementation

New handler file `src/api/verify.ts`. Uses `tokenizeReading()` from `src/claims/tokenize.ts` and constraint data from the DO. Pure deterministic — no LLM.

## apis: `POST /graphdl/extract`

Rename of existing `/graphdl/extract/claims`. Same LLM logic, same Claude system prompt, same `ExtractedClaims` response shape.

- If `seed: true`, ingests the extracted claims via graphdl-orm `/claims`.
- `/graphdl/extract/claims` becomes an alias for backward compatibility.

## apis: `POST /graphdl/extract/all`

The combined extraction endpoint.

### Input

```typescript
{
  text: string      // FORML2, natural language, or mixed
  domain: string    // Domain ID
  seed?: boolean    // If true, ingest combined claims
}
```

### Behavior

```
1. POST graphdl-orm /parse  { text, domain }  → deterministic: ExtractedClaims
2. POST own /extract        { text, domain }  → semantic: ExtractedClaims
3. mergeClaims(deterministic, semantic)        → combined: ExtractedClaims
4. if (seed) POST graphdl-orm /claims          → ingest combined
```

### Deduplication

`mergeClaims()` merges two `ExtractedClaims` objects:
- **Nouns**: deduplicate by name. Deterministic wins on `objectType` conflicts.
- **Readings**: deduplicate by noun set + predicate. Keep both if predicates differ.
- **Constraints**: deduplicate by kind + reading + role indices.
- **Subtypes**: deduplicate by child + parent pair.

### Output

```typescript
{
  deterministic: ExtractedClaims   // From /parse only
  semantic: ExtractedClaims        // From LLM only
  combined: ExtractedClaims        // Merged and deduplicated
  ingested?: any                   // Result from /claims if seed: true
}
```

## apis: `POST /graphdl/verify`

Unified agent verification — replaces the 3-call pipeline in agent-chat.ts.

### Input

```typescript
{
  text: string    // Agent's draft response
  domain: string  // Domain name or ID
}
```

### Behavior

```
1. POST graphdl-orm /verify { text, domain }
   → { matches, unmatchedConstraints }

2. If unmatchedConstraints.length > 0:
   POST own /extract/semantic { text, constraints: unmatchedConstraints }
   → { claims }

3. Combine:
   - Deterministic violations from matches → warnings with method: 'deterministic'
   - Semantic violations from claims → warnings with method: 'semantic'
```

### Output

```typescript
{
  warnings: Array<{
    reading: string                         // Constraint text
    instance?: string                       // What the agent wrote
    claim?: string                          // Semantic violation detail
    method: 'deterministic' | 'semantic'
    confidence?: number                     // Only for semantic (0.0–1.0)
  }>
}
```

### Agent Chat Update

Replace the 3-stage `verify()` function in `agent-chat.ts` with a single call:

```typescript
// Before (3 calls):
const extractResult = await extractProxy(...)
const semanticResult = await extractSemantic(...)
const checkResult = await checkProxy(...)

// After (1 call):
const { warnings } = await verifyProxy({ text, domain }, env)
```

## Deprecation

| Old Route | Action |
|-----------|--------|
| `POST /graphdl/extract/claims` | Alias to `POST /graphdl/extract` |
| `POST /graphdl/check` | Remove — absorbed into `/graphdl/verify` |
| Dead `/graphdl/extract` proxy | Replaced with real `/graphdl/extract` |
| Dead `/graphdl/check` proxy | Replaced with real `/graphdl/verify` |
| Dead `/graphdl/parse` proxy | Replaced with real `/graphdl/parse` proxy |

## Files Changed

### graphdl-orm
| File | Action |
|------|--------|
| `src/api/parse.ts` | Create — `/parse` handler |
| `src/api/verify.ts` | Create — `/verify` handler |
| `src/api/router.ts` | Modify — register `/parse` and `/verify` routes |

### apis
| File | Action |
|------|--------|
| `graphdl/extract-proxy.ts` | Rewrite — rename to `/extract`, alias `/extract/claims` |
| `graphdl/check-proxy.ts` | Delete — replaced by verify |
| `graphdl/parse-proxy.ts` | Rewrite — proxy to graphdl-orm `/parse` |
| `graphdl/verify-proxy.ts` | Create — orchestrates deterministic + semantic |
| `graphdl/extract-all.ts` | Create — combined extraction endpoint |
| `graphdl/merge-claims.ts` | Create — `mergeClaims()` deduplication logic |
| `graphdl/agent-chat.ts` | Modify — replace 3-call verify with single `/verify` |
| `index.ts` | Modify — update route registrations |

## Testing

### graphdl-orm
- **`/parse` unit tests**: multi-line FORML2 → correct `ExtractedClaims` shape; batch deferral works; idempotent re-parse
- **`/verify` unit tests**: prose with fact instances → correct matches; UC violation detection; unmatched constraints listed

### apis
- **`/extract/all` integration**: mock both graphdl-orm `/parse` and own `/extract`, verify merge logic
- **`mergeClaims()` unit tests**: deduplication by noun name, reading noun-set, constraint identity
- **`/verify` integration**: mock graphdl-orm `/verify` and `/extract/semantic`, verify warnings shape
- **Agent chat**: verify single-call `/verify` produces same re-draft behavior
