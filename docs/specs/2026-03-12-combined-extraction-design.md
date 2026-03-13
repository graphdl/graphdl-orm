# Combined Extraction Endpoints — Design Spec

Wire up the three dead proxy endpoints (`/extract`, `/check`, `/parse`) with real implementations, creating a clean API surface for both domain modeling (FORML2 → claims) and agent verification (prose → constraint warnings).

## Endpoint Contracts

| Route | Repo | Input | Output | Purpose |
|-------|------|-------|--------|---------|
| `POST /parse` | graphdl-orm | `{ text, domain }` | `ExtractedClaims` | Deterministic FORML2 parsing (read-only) |
| `POST /verify` | graphdl-orm | `{ text, domain }` | `{ matches, unmatchedConstraints }` | Deterministic noun matching against prose |
| `POST /graphdl/extract` | apis | `{ text, domain?, seed? }` | `ExtractedClaims` | LLM semantic extraction (rename of `/extract/claims`) |
| `POST /graphdl/extract/all` | apis | `{ text, domain, seed? }` | `{ deterministic, semantic, combined }` | Calls `/parse` + `/extract`, merges |
| `POST /graphdl/verify` | apis | `{ text, domain }` | `{ warnings }` | Full agent verification in 1 call |
| `POST /graphdl/extract/semantic` | apis | `{ text, constraints }` | `{ claims }` | LLM deontic violation detection (unchanged) |

All graphdl-orm routes are root-level (`/parse`, `/verify`) like the existing `/seed` and `/claims` routes. All apis routes are under `/graphdl/`.

## Boundary Principle

graphdl-orm is open source — no LLM calls, no API keys, pure deterministic logic. apis adds authentication, LLM extraction, and orchestration.

## graphdl-orm: `POST /parse`

**Read-only.** Parses FORML2 text into structured claims without writing to the database. To persist, callers pass the result to `/claims`.

### Input

```typescript
{
  text: string    // Multi-line FORML2 text
  domain: string  // Domain ID (used for noun context, not for writes)
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

1. Load existing nouns for the domain (for tokenization context).
2. Split text into blocks on blank lines. Each block is a reading (first line) plus its indented constraints (subsequent indented lines).
3. For each block:
   a. Extract noun names via PascalCase matching (known nouns matched via `tokenizeReading()`, unknown nouns detected by PascalCase).
   b. Determine predicate from the text between the first two nouns.
   c. Apply entity/value heuristic: object of "has" → value type, otherwise entity type.
   d. Parse each indented constraint line via `parseConstraintText()`.
   e. Accumulate nouns, readings, constraints into the result.
4. Detect subtype declarations ("X is a subtype of Y") and add to subtypes.
5. If a constraint references nouns not yet seen, defer it. After all blocks, retry deferred constraints against the full noun set. Report any still-unresolved constraints as warnings.

This is a **pure-function pipeline** — it does NOT use `createWithHook()` or touch the database. It reuses `tokenizeReading()` and `parseConstraintText()` as pure functions.

### Output

Uses the canonical `ExtractedClaims` interface from `src/claims/ingest.ts`:

```typescript
interface ExtractedClaims {
  nouns: Array<{
    name: string
    objectType: 'entity' | 'value'
    plural?: string
    valueType?: string
    format?: string
    enum?: string[]
    minimum?: number
    maximum?: number
    pattern?: string
  }>
  readings: Array<{
    text: string
    nouns: string[]
    predicate: string
    multiplicity?: string
  }>
  constraints: Array<{
    kind: 'UC' | 'MC' | 'RC'
    modality: 'Alethic' | 'Deontic'
    reading: string
    roles: number[]
  }>
  subtypes: Array<{
    child: string
    parent: string
  }>
  transitions: []   // Always empty — FORML2 text does not express transitions
  facts: []          // Always empty — FORML2 text does not express instance facts
}
```

Plus a top-level `warnings: string[]` for unresolvable constraints or unrecognized patterns.

### Error Handling

Malformed input produces partial results with warnings. `/parse` never fails wholesale — it returns whatever it could parse, with warnings for anything it couldn't. Examples:
- Unrecognized constraint pattern → warning, constraint skipped
- Fewer than 2 nouns in a reading → warning, reading skipped
- Deferred constraint still unresolved after retry → warning

### Implementation

New handler file `src/api/parse.ts`. Pure-function chain using:
- `tokenizeReading()` from `src/claims/tokenize.ts`
- `parseConstraintText()` from `src/hooks/parse-constraint.ts`

Does NOT import or call hooks, `createWithHook`, or any DB methods.

## graphdl-orm: `POST /verify`

### Input

```typescript
{
  text: string    // Prose text (e.g., agent draft response)
  domain: string  // Domain ID
}
```

### Behavior

1. Load all readings, constraints, constraint-spans, roles, and nouns for the domain.
2. For each reading, tokenize the prose text to find **noun-type mentions** — occurrences of the reading's noun names in the text.
3. Classify each constraint:
   - **Deterministically checkable**: all nouns referenced by the constraint's reading appear in the prose text. These constraints' nouns are "in scope" and can be checked.
   - **Unmatched**: the constraint's nouns do NOT appear in the prose. These need semantic (LLM) analysis.
4. For deterministically checkable constraints, report which readings matched and what text fragments were found.

Note: The deterministic `/verify` does **not** attempt to extract structured fact tuples or detect UC/MC violations from prose — that requires entity-instance resolution beyond what the tokenizer can do. It identifies which constraints are *relevant* to the text (noun types are mentioned) and which are not. The apis-side `/graphdl/verify` endpoint then uses the LLM to do deeper violation analysis on the relevant constraints.

### Output

```typescript
{
  matches: Array<{
    reading: string       // Reading text (e.g., "Customer has Name")
    nouns: string[]       // Nouns from this reading found in the prose
  }>
  unmatchedConstraints: string[]  // Constraint texts whose nouns are absent from the prose
}
```

### Implementation

New handler file `src/api/verify.ts`. Uses `tokenizeReading()` and constraint/reading data from the DO. Pure deterministic — no LLM.

## apis: `POST /graphdl/extract`

Rename of existing `/graphdl/extract/claims`. Same LLM logic, same Claude system prompt, same `ExtractedClaims` response shape.

- If `seed: true`, ingests the extracted claims via graphdl-orm `/claims`.
- `/graphdl/extract/claims` stays as an alias for backward compatibility.

## apis: `POST /graphdl/extract/all`

The combined extraction endpoint.

### Input

```typescript
{
  text: string      // FORML2, natural language, or mixed
  domain: string    // Domain ID
  seed?: boolean    // If true, ingest combined claims via /claims
}
```

### Behavior

```
1. POST graphdl-orm /parse  { text, domain }  → deterministic: ExtractedClaims
2. POST own /extract        { text, domain }  → semantic: ExtractedClaims
3. mergeClaims(deterministic, semantic)        → combined: ExtractedClaims
4. if (seed) POST graphdl-orm /claims          → ingest combined
```

Steps 1 and 2 can run in parallel (no dependency).

### Deduplication

`mergeClaims()` merges two `ExtractedClaims` objects:
- **Nouns**: deduplicate by `name` (case-sensitive). Deterministic wins on `objectType` conflicts.
- **Readings**: deduplicate by sorted noun-name set. If both sides have the same noun set, keep the deterministic version's text (it's the canonical FORML2 form). If noun sets differ, keep both.
- **Constraints**: deduplicate by `kind` + `reading` + sorted `roles` array.
- **Subtypes**: deduplicate by `child` + `parent` pair.
- **Transitions**: pass through from semantic only (deterministic never produces transitions).
- **Facts**: pass through from semantic only.

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
  domain: string  // Domain ID
}
```

### Behavior

```
1. POST graphdl-orm /verify { text, domain }
   → { matches, unmatchedConstraints }

2. If unmatchedConstraints.length > 0:
   POST own /extract/semantic { text, constraints: unmatchedConstraints }
   → { claims }

3. Combine into warnings:
   - Each match with a constraint → warning with method: 'deterministic'
   - Each semantic claim → warning with method: 'semantic'
```

### Output

```typescript
{
  warnings: Array<{
    reading: string                         // Constraint/reading text
    instance?: string                       // What the agent wrote (if deterministic)
    claim?: string                          // Semantic violation detail (if semantic)
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
| `POST /graphdl/extract/claims` | Alias to `POST /graphdl/extract` (backward compat) |
| `POST /graphdl/check` | Remove — absorbed into `/graphdl/verify` |
| Dead `/graphdl/extract` proxy (`extract-proxy.ts`) | Replaced with real `/graphdl/extract` handler |
| Dead `/graphdl/check` proxy (`check-proxy.ts`) | Removed — replaced by `/graphdl/verify` |
| Dead `/graphdl/parse` proxy (`parse-proxy.ts`) | Replaced with real proxy to graphdl-orm `/parse` |

## Files Changed

### graphdl-orm

| File | Action |
|------|--------|
| `src/api/parse.ts` | Create — `/parse` handler (pure-function FORML2 parser) |
| `src/api/parse.test.ts` | Create — unit tests for `/parse` |
| `src/api/verify.ts` | Create — `/verify` handler (deterministic noun matching) |
| `src/api/verify.test.ts` | Create — unit tests for `/verify` |
| `src/api/router.ts` | Modify — register `/parse` and `/verify` routes |

### apis

| File | Action |
|------|--------|
| `graphdl/extract-proxy.ts` | Rewrite — becomes real `/extract` handler (rename of extract-claims logic), add `/extract/claims` alias |
| `graphdl/check-proxy.ts` | Delete — replaced by verify |
| `graphdl/parse-proxy.ts` | Rewrite — proxy to graphdl-orm `/parse` |
| `graphdl/verify-proxy.ts` | Create — orchestrates deterministic + semantic verification |
| `graphdl/extract-all.ts` | Create — combined extraction endpoint |
| `graphdl/merge-claims.ts` | Create — `mergeClaims()` deduplication logic |
| `graphdl/merge-claims.test.ts` | Create — unit tests for deduplication |
| `graphdl/agent-chat.ts` | Modify — replace 3-call verify with single `/verify` call |
| `index.ts` | Modify — update route registrations |

## Testing

### graphdl-orm

- **`/parse`**: multi-line FORML2 → correct `ExtractedClaims` shape; deferred constraint retry; partial results with warnings for malformed input; empty arrays for transitions/facts; subtype detection
- **`/verify`**: prose mentioning domain nouns → correct matches; prose without domain nouns → all constraints unmatched; mixed → correct split

### apis

- **`mergeClaims()`**: deduplication by noun name, reading noun-set, constraint identity; deterministic wins on conflicts; transitions/facts pass through from semantic
- **`/extract/all`**: mock both sides, verify merge and optional seeding
- **`/verify`**: mock graphdl-orm `/verify` and `/extract/semantic`, verify warnings shape matches agent-chat expectations
- **Agent chat**: verify single-call `/verify` produces same re-draft behavior as old 3-call pipeline
