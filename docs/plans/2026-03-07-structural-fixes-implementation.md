# Structural Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace rigid markdown parsers with a unified semantic parser, make generated Payload collections deploy into the running instance with domain scoping, and add a readings generator output for source control extraction.

**Architecture:** The unified parser handles all tiers — FORML2 verbal patterns (deterministic), compiled rule matching (deterministic), and LLM extraction (semantic). Generated collections are materialized from DB to `src/collections/generated/` and auto-imported into `payload.config.ts`. The readings generator extracts DB state back to natural language.

**Tech Stack:** Payload CMS 3.79, Next.js 15, MongoDB, Vitest, TypeScript

---

### Task 1: FORML2 Verbal Constraint Parser

Build the core of the unified parser — extract fact types and role constraints from FORML2 verbal patterns.

**Files:**
- Create: `src/parse/forml2.ts`
- Create: `src/parse/forml2.test.ts`

**Step 1: Write the failing tests**

```typescript
// src/parse/forml2.test.ts
import { describe, it, expect } from 'vitest'
import { parseReading } from './forml2'

describe('parseReading', () => {
  it('extracts a simple binary fact type', () => {
    const result = parseReading('Customer has Name', ['Customer', 'Name'])
    expect(result.nouns).toEqual(['Customer', 'Name'])
    expect(result.predicate).toBe('has')
    expect(result.constraints).toEqual([])
  })

  it('extracts UC from "at most one"', () => {
    const result = parseReading('Each Customer has at most one Name', ['Customer', 'Name'])
    expect(result.nouns).toEqual(['Customer', 'Name'])
    expect(result.constraints).toEqual([
      { kind: 'UC', roles: [0], modality: 'Alethic' }
    ])
  })

  it('extracts MC from "some" (mandatory)', () => {
    const result = parseReading('Each Customer has some Name', ['Customer', 'Name'])
    expect(result.constraints).toEqual([
      { kind: 'MC', roles: [0], modality: 'Alethic' }
    ])
  })

  it('extracts UC + MC from "exactly one"', () => {
    const result = parseReading('Each Customer has exactly one Name', ['Customer', 'Name'])
    expect(result.constraints).toContainEqual({ kind: 'UC', roles: [0], modality: 'Alethic' })
    expect(result.constraints).toContainEqual({ kind: 'MC', roles: [0], modality: 'Alethic' })
  })

  it('extracts deontic obligation', () => {
    const result = parseReading(
      'It is obligatory that SupportResponse not contain ProhibitedPunctuation',
      ['SupportResponse', 'ProhibitedPunctuation']
    )
    expect(result.nouns).toEqual(['SupportResponse', 'ProhibitedPunctuation'])
    expect(result.constraints).toContainEqual({ kind: 'MC', roles: [0], modality: 'Deontic' })
  })

  it('extracts ternary fact type', () => {
    const result = parseReading(
      'Listing has Price via ListingChannel',
      ['Listing', 'Price', 'ListingChannel']
    )
    expect(result.nouns).toEqual(['Listing', 'Price', 'ListingChannel'])
  })

  it('extracts subtype declaration', () => {
    const result = parseReading('Admin is a subtype of Customer', ['Admin', 'Customer'])
    expect(result.isSubtype).toBe(true)
    expect(result.nouns).toEqual(['Admin', 'Customer'])
  })

  it('extracts state transition reading', () => {
    const result = parseReading(
      'SupportRequest transitions from Received to Triaging on acknowledge',
      ['SupportRequest', 'Received', 'Triaging']
    )
    expect(result.isTransition).toBe(true)
    expect(result.transition).toEqual({
      subject: 'SupportRequest',
      from: 'Received',
      to: 'Triaging',
      event: 'acknowledge',
    })
  })

  it('handles instance fact with quoted value', () => {
    const result = parseReading(
      "Customer with EmailDomain 'driv.ly' has UserRole 'ADMIN'",
      ['Customer', 'EmailDomain', 'UserRole']
    )
    expect(result.isInstanceFact).toBe(true)
    expect(result.instanceValues).toContainEqual({ noun: 'EmailDomain', value: 'driv.ly' })
    expect(result.instanceValues).toContainEqual({ noun: 'UserRole', value: 'ADMIN' })
  })
})
```

**Step 2: Run tests to verify they fail**

Run: `npx vitest run src/parse/forml2.test.ts`
Expected: FAIL — module `./forml2` not found

**Step 3: Implement the FORML2 parser**

```typescript
// src/parse/forml2.ts

export interface ParsedConstraint {
  kind: 'UC' | 'MC'
  roles: number[]  // indexes into nouns array
  modality: 'Alethic' | 'Deontic'
}

export interface TransitionDef {
  subject: string
  from: string
  to: string
  event: string
}

export interface InstanceValue {
  noun: string
  value: string
}

export interface ParsedReading {
  nouns: string[]
  predicate: string
  constraints: ParsedConstraint[]
  isSubtype: boolean
  isTransition: boolean
  transition?: TransitionDef
  isInstanceFact: boolean
  instanceValues: InstanceValue[]
}

// Sort longest first so "ListingChannel" matches before "Listing"
function buildNounRegex(knownNouns: string[]): RegExp {
  const sorted = [...knownNouns].sort((a, b) => b.length - a.length)
  return new RegExp('\\b(' + sorted.map(n => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|') + ')\\b', 'g')
}

export function parseReading(text: string, knownNouns: string[]): ParsedReading {
  const result: ParsedReading = {
    nouns: [],
    predicate: '',
    constraints: [],
    isSubtype: false,
    isTransition: false,
    isInstanceFact: false,
    instanceValues: [],
  }

  // Check for subtype pattern
  const subtypeMatch = text.match(/^(\w+)\s+is\s+a\s+subtype\s+of\s+(\w+)$/i)
  if (subtypeMatch) {
    result.isSubtype = true
    result.nouns = [subtypeMatch[1], subtypeMatch[2]]
    return result
  }

  // Check for transition pattern
  const transitionMatch = text.match(
    /^(\w+)\s+transitions\s+from\s+(\w+)\s+to\s+(\w+)\s+on\s+(\w+)$/i
  )
  if (transitionMatch) {
    result.isTransition = true
    result.nouns = [transitionMatch[1], transitionMatch[2], transitionMatch[3]]
    result.transition = {
      subject: transitionMatch[1],
      from: transitionMatch[2],
      to: transitionMatch[3],
      event: transitionMatch[4],
    }
    return result
  }

  // Extract quoted instance values
  const quotedPattern = /(?:with\s+)?(\w+)\s+['"'\u2018\u201C]([^'"'\u2019\u201D]+)['"'\u2019\u201D]/g
  let quotedMatch
  while ((quotedMatch = quotedPattern.exec(text)) !== null) {
    if (knownNouns.includes(quotedMatch[1])) {
      result.instanceValues.push({ noun: quotedMatch[1], value: quotedMatch[2] })
    }
  }
  if (result.instanceValues.length > 0) {
    result.isInstanceFact = true
  }

  // Tokenize to find nouns in order
  const nounRegex = buildNounRegex(knownNouns)
  let match
  while ((match = nounRegex.exec(text)) !== null) {
    if (!result.nouns.includes(match[1])) {
      result.nouns.push(match[1])
    }
  }

  // Determine modality
  const isDeontic = /\b(obligatory|forbidden|permitted|prohibited)\b/i.test(text)
  const modality = isDeontic ? 'Deontic' as const : 'Alethic' as const

  // Extract verbal constraints
  // "at most one" = UC on the preceding noun's role
  if (/\bat\s+most\s+one\b/i.test(text)) {
    result.constraints.push({ kind: 'UC', roles: [0], modality })
  }

  // "some" = MC on the preceding noun's role (mandatory)
  if (/\bhas\s+some\b/i.test(text)) {
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  // "exactly one" = UC + MC
  if (/\bexactly\s+one\b/i.test(text)) {
    result.constraints.push({ kind: 'UC', roles: [0], modality })
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  // "one or more" / "at least one" = MC (mandatory, no uniqueness)
  if (/\b(one\s+or\s+more|at\s+least\s+one)\b/i.test(text)) {
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  // Extract predicate (text between first and second noun, minus constraint words)
  if (result.nouns.length >= 2) {
    const firstNounEnd = text.indexOf(result.nouns[0]) + result.nouns[0].length
    const secondNounStart = text.indexOf(result.nouns[1], firstNounEnd)
    if (secondNounStart > firstNounEnd) {
      result.predicate = text
        .slice(firstNounEnd, secondNounStart)
        .replace(/\b(each|at most one|some|exactly one|one or more|at least one)\b/gi, '')
        .trim()
    }
  }

  return result
}
```

**Step 4: Run tests to verify they pass**

Run: `npx vitest run src/parse/forml2.test.ts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/parse/forml2.ts src/parse/forml2.test.ts
git commit -m "feat: FORML2 verbal constraint parser — deterministic tier 1 of unified parser"
```

---

### Task 2: Unified Parse Endpoint

Replace the seed's `parseDomainMarkdown` dependency with a unified parse function that accepts any text and routes through the appropriate tier.

**Files:**
- Create: `src/parse/index.ts`
- Create: `src/parse/index.test.ts`

**Step 1: Write the failing tests**

```typescript
// src/parse/index.test.ts
import { describe, it, expect } from 'vitest'
import { parseText } from './index'

describe('parseText', () => {
  const knownNouns = ['Customer', 'Name', 'SupportRequest', 'Admin', 'Priority']

  it('parses a single FORML2 reading', () => {
    const result = parseText('Each Customer has at most one Name', knownNouns)
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].nouns).toEqual(['Customer', 'Name'])
    expect(result.readings[0].constraints).toContainEqual({
      kind: 'UC', roles: [0], modality: 'Alethic'
    })
  })

  it('parses multiple readings separated by newlines', () => {
    const text = `Customer has Name
Customer submits SupportRequest
SupportRequest has Priority`
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(3)
  })

  it('skips blank lines and comments', () => {
    const text = `Customer has Name

# This is a comment
Customer submits SupportRequest`
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(2)
  })

  it('detects subtypes', () => {
    const text = 'Admin is a subtype of Customer'
    const result = parseText(text, [...knownNouns, 'Admin'])
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].isSubtype).toBe(true)
  })

  it('collects new noun candidates from unrecognized tokens', () => {
    const text = 'Customer has EmailAddress'
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(1)
    // EmailAddress should be detected as a candidate noun
    expect(result.newNounCandidates).toContain('EmailAddress')
  })
})
```

**Step 2: Run tests to verify they fail**

Run: `npx vitest run src/parse/index.test.ts`
Expected: FAIL

**Step 3: Implement the unified parser**

```typescript
// src/parse/index.ts
import { parseReading, type ParsedReading } from './forml2'

export interface ParseResult {
  readings: ParsedReading[]
  newNounCandidates: string[]
}

export function parseText(text: string, knownNouns: string[]): ParseResult {
  const lines = text
    .split('\n')
    .map(l => l.trim())
    .filter(l => l.length > 0 && !l.startsWith('#'))

  const readings: ParsedReading[] = []
  const allNouns = new Set(knownNouns)
  const newNounCandidates = new Set<string>()

  for (const line of lines) {
    const parsed = parseReading(line, [...allNouns])
    readings.push(parsed)

    // Detect capitalized words that aren't known nouns as candidates
    const capitalizedWords = line.match(/\b([A-Z][a-zA-Z]+)\b/g) || []
    for (const word of capitalizedWords) {
      if (!allNouns.has(word) && !isConstraintKeyword(word)) {
        newNounCandidates.add(word)
        allNouns.add(word) // Use for subsequent lines
      }
    }
  }

  return { readings, newNounCandidates: [...newNounCandidates] }
}

const CONSTRAINT_KEYWORDS = new Set([
  'Each', 'It', 'That', 'The',
])

function isConstraintKeyword(word: string): boolean {
  return CONSTRAINT_KEYWORDS.has(word)
}
```

**Step 4: Run tests to verify they pass**

Run: `npx vitest run src/parse/index.test.ts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/parse/index.ts src/parse/index.test.ts
git commit -m "feat: unified parseText — routes lines through FORML2 parser, detects new nouns"
```

---

### Task 3: Wire Unified Parser into Seed Handler

Replace the seed handler's dependency on `parseDomainMarkdown` types with the unified parser. The ensure* helpers stay — they receive parsed output instead of markdown-parsed output.

**Files:**
- Create: `src/seed/unified.ts`
- Create: `src/seed/unified.test.ts`

**Step 1: Write the failing test**

```typescript
// src/seed/unified.test.ts
import { describe, it, expect } from 'vitest'
import { seedReadingsFromText } from './unified'

// This test requires the integration test setup (MongoDB in memory)
describe('seedReadingsFromText', () => {
  it('creates nouns and readings from plain text', async () => {
    const { getPayload } = await import('payload')
    const payload = await getPayload({ config: (await import('@payload-config')).default })

    // Seed a domain first
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-unified', name: 'Test Unified' },
    })

    const result = await seedReadingsFromText(payload, {
      text: 'Customer has Name\nCustomer submits SupportRequest',
      domainId: domain.id,
    })

    expect(result.nounsCreated).toBeGreaterThanOrEqual(3)
    expect(result.readingsCreated).toBe(2)

    // Verify nouns exist
    const nouns = await payload.find({
      collection: 'nouns',
      where: { name: { in: ['Customer', 'Name', 'SupportRequest'] } },
    })
    expect(nouns.docs).toHaveLength(3)
  })
})
```

**Step 2: Run test to verify it fails**

Run: `npx vitest run src/seed/unified.test.ts`
Expected: FAIL — module not found

**Step 3: Implement unified seed function**

```typescript
// src/seed/unified.ts
import type { Payload } from 'payload'
import { parseText } from '../parse'

interface SeedOptions {
  text: string
  domainId: string
  tenant?: string
}

interface SeedResult {
  nounsCreated: number
  readingsCreated: number
  constraintsCreated: number
  errors: string[]
}

async function ensureNoun(payload: Payload, name: string, domainId: string): Promise<any> {
  const existing = await payload.find({
    collection: 'nouns',
    where: { name: { equals: name } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({
    collection: 'nouns',
    data: { name, objectType: 'entity', domain: domainId },
  })
}

export async function seedReadingsFromText(
  payload: Payload,
  options: SeedOptions,
): Promise<SeedResult> {
  const { text, domainId } = options
  const result: SeedResult = { nounsCreated: 0, readingsCreated: 0, constraintsCreated: 0, errors: [] }

  // Get existing nouns for this domain
  const existingNouns = await payload.find({
    collection: 'nouns',
    where: { domain: { equals: domainId } },
    pagination: false,
  })
  const knownNounNames = existingNouns.docs.map((n: any) => n.name as string).filter(Boolean)

  // Parse the text
  const parsed = parseText(text, knownNounNames)

  // Create any new noun candidates
  for (const nounName of parsed.newNounCandidates) {
    await ensureNoun(payload, nounName, domainId)
    result.nounsCreated++
  }

  // Refresh known nouns after creating new ones
  const allNouns = await payload.find({
    collection: 'nouns',
    pagination: false,
  })
  const nounMap = new Map(allNouns.docs.map((n: any) => [n.name, n]))

  for (const reading of parsed.readings) {
    try {
      if (reading.isSubtype) {
        // Set superType relationship
        const subNoun = nounMap.get(reading.nouns[0])
        const superNoun = nounMap.get(reading.nouns[1])
        if (subNoun && superNoun) {
          await payload.update({
            collection: 'nouns',
            id: subNoun.id,
            data: { superType: superNoun.id },
          })
        }
        continue
      }

      // Ensure all nouns in this reading exist
      for (const nounName of reading.nouns) {
        if (!nounMap.has(nounName)) {
          const noun = await ensureNoun(payload, nounName, domainId)
          nounMap.set(nounName, noun)
          result.nounsCreated++
        }
      }

      // Create graph schema
      const schemaName = reading.nouns.join(' ')
      const graphSchema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: schemaName,
          title: schemaName,
          domain: domainId,
        },
      })

      // Create reading — afterChange hook auto-creates roles
      await payload.create({
        collection: 'readings',
        data: {
          text: reading.nouns.length >= 2
            ? `${reading.nouns[0]} ${reading.predicate} ${reading.nouns.slice(1).join(' ')}`
            : reading.nouns[0],
          graphSchema: graphSchema.id,
          domain: domainId,
        },
      })
      result.readingsCreated++

      // Apply constraints to roles after the hook creates them
      if (reading.constraints.length > 0) {
        // Wait for roles to be created by the afterChange hook
        const roles = await payload.find({
          collection: 'roles',
          where: { graphSchema: { equals: graphSchema.id } },
          sort: 'createdAt',
        })

        for (const constraint of reading.constraints) {
          const c = await payload.create({
            collection: 'constraints',
            data: { kind: constraint.kind, modality: constraint.modality },
          })

          const roleIds = constraint.roles.map(
            (idx) => roles.docs[idx]?.id
          ).filter(Boolean)

          if (roleIds.length) {
            await payload.create({
              collection: 'constraint-spans',
              data: { roles: roleIds, constraint: c.id },
            })
            result.constraintsCreated++
          }
        }
      }
    } catch (err: any) {
      result.errors.push(`Failed to seed "${reading.nouns.join(' ')}": ${err.message}`)
    }
  }

  return result
}
```

**Step 4: Run test to verify it passes**

Run: `npx vitest run src/seed/unified.test.ts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/seed/unified.ts src/seed/unified.test.ts
git commit -m "feat: seedReadingsFromText — unified seed from natural language via parser"
```

---

### Task 4: Generated Collections with Domain Scoping

Make the Generator's Payload output produce collection configs with domain field and instance access control, then write a build script to materialize them.

**Files:**
- Modify: `src/collections/Generator.ts` (the `generatePayloadFiles` function, around line 1150)
- Create: `src/collections/generated/.gitkeep`
- Create: `scripts/materialize-collections.ts`
- Modify: `src/payload.config.ts`

**Step 1: Update `generatePayloadFiles` to include domain scoping**

In `src/collections/Generator.ts`, find the `accessToTS` function (around line 2023) and the collection building section in `generatePayloadFiles` (around line 1150 where `collection.access` is set).

Replace the access generation:

```typescript
// Replace the existing access block (around line 1150):
//   const access: Record<string, string> = {}
//   if (permissions.includes('create')) access.create = 'authenticated'
//   ...
//   if (Object.keys(access).length) collection.access = access

// With:
collection.access = {
  read: 'instanceReadAccess',
  create: 'instanceWriteAccess',
  update: 'instanceWriteAccess',
  delete: 'instanceWriteAccess',
}
```

Add `domainField` as the last field in `finalFields`:

```typescript
// After the finalFields construction, before assigning to collection:
finalFields.push({ name: 'domain', type: 'relationship', relationTo: 'domains', index: true })
```

Update `generateCollectionTypeScript` to emit proper imports:

```typescript
function generateCollectionTypeScript(
  slug: string,
  collection: Record<string, unknown>,
): string {
  const pascalName = slug.split('-').map(s => s.charAt(0).toUpperCase() + s.slice(1)).join('')
  const lines: string[] = []
  lines.push("import type { CollectionConfig } from 'payload'")
  lines.push("import { instanceReadAccess, instanceWriteAccess } from '../shared/instanceAccess'")
  lines.push('')
  lines.push(`export const ${pascalName}: CollectionConfig = ${objectToTS(collection, 0)}`)
  lines.push('')
  return lines.join('\n')
}
```

Update `accessToTS` to emit the actual access functions:

```typescript
function accessToTS(access: Record<string, string>, indent: number): string {
  const pad = '  '.repeat(indent)
  const inner = '  '.repeat(indent + 1)
  const entries = Object.entries(access).filter(([, v]) => v !== undefined)
  if (entries.length === 0) return '{}'
  const props = entries.map(([key, value]) => {
    return `${inner}${key}: ${value}`
  })
  return `{\n${props.join(',\n')},\n${pad}}`
}
```

**Step 2: Create the materialize script**

```typescript
// scripts/materialize-collections.ts
import configPromise from '../src/payload.config'
import { getPayload } from 'payload'
import fs from 'fs'
import path from 'path'

const GENERATED_DIR = path.resolve(__dirname, '../src/collections/generated')

async function materialize() {
  const payload = await getPayload({ config: configPromise })

  // Find all generators with outputFormat = 'payload'
  const generators = await payload.find({
    collection: 'generators',
    where: { outputFormat: { equals: 'payload' } },
    pagination: false,
  })

  // Ensure output directory exists
  if (!fs.existsSync(GENERATED_DIR)) {
    fs.mkdirSync(GENERATED_DIR, { recursive: true })
  }

  // Clear existing generated files
  for (const file of fs.readdirSync(GENERATED_DIR)) {
    if (file.endsWith('.ts') && file !== 'index.ts') {
      fs.unlinkSync(path.join(GENERATED_DIR, file))
    }
  }

  // Write each generated collection file
  const slugs: string[] = []
  for (const gen of generators.docs) {
    const files = (gen as any).output?.files || {}
    for (const [filePath, content] of Object.entries(files)) {
      const outPath = path.join(GENERATED_DIR, path.basename(filePath))
      fs.writeFileSync(outPath, content as string)
      const slug = path.basename(filePath, '.ts')
      slugs.push(slug)
    }
  }

  // Write barrel file
  const barrel = slugs.map(slug => {
    const pascalName = slug.split('-').map(s => s.charAt(0).toUpperCase() + s.slice(1)).join('')
    return `export { ${pascalName} } from './${slug}'`
  }).join('\n') + '\n'
  fs.writeFileSync(path.join(GENERATED_DIR, 'index.ts'), barrel)

  console.log(`Materialized ${slugs.length} collections to ${GENERATED_DIR}`)
  process.exit(0)
}

materialize().catch(console.error)
```

**Step 3: Update payload.config.ts to import generated collections**

Add to `src/payload.config.ts` after the existing imports:

```typescript
// At top, after other collection imports:
let generatedCollections: CollectionConfig[] = []
try {
  const generated = require('./collections/generated')
  generatedCollections = Object.values(generated)
} catch {
  // No generated collections yet — that's fine
}
```

In the `collections` array of `buildConfig`:

```typescript
collections: [
  // ... existing 24 meta-collections ...
  ...generatedCollections,
],
```

**Step 4: Create .gitkeep for generated directory**

```bash
mkdir -p src/collections/generated
touch src/collections/generated/.gitkeep
echo "*.ts" > src/collections/generated/.gitignore
echo "!.gitkeep" >> src/collections/generated/.gitignore
echo "!.gitignore" >> src/collections/generated/.gitignore
```

**Step 5: Add script to package.json**

```json
"materialize": "npx tsx scripts/materialize-collections.ts"
```

**Step 6: Commit**

```bash
git add src/collections/Generator.ts src/collections/generated/ scripts/materialize-collections.ts src/payload.config.ts package.json
git commit -m "feat: deployable generated collections with domain scoping and materialize script"
```

---

### Task 5: Readings Generator Output

Add a 6th generator output format that extracts DB state to natural language readings for source control.

**Files:**
- Modify: `src/collections/Generator.ts` (add `readings` to outputFormat options, add `generateReadingsOutput` function)

**Step 1: Add `readings` to the outputFormat select field**

In `src/collections/Generator.ts`, find the `outputFormat` field definition (around line 120) and add the new option:

```typescript
{ label: 'Readings', value: 'readings' },
```

**Step 2: Add the dispatch case in the beforeChange hook**

Find the `else if` chain that dispatches on `outputFormat` (around line 146):

```typescript
} else if (outputFormat === 'readings') {
  data.output = await generateReadingsOutput(payload, domainFilter)
}
```

**Step 3: Implement the generateReadingsOutput function**

Add before the `// #region` section near the end of Generator.ts:

```typescript
async function generateReadingsOutput(payload: any, domainFilter: Where): Promise<any> {
  const nouns = await payload.find({ collection: 'nouns', pagination: false, where: domainFilter }).then((n: any) => n.docs)
  const readings = await payload.find({ collection: 'readings', pagination: false, depth: 3, where: domainFilter }).then((r: any) => r.docs)
  const constraintSpans = await payload.find({
    collection: 'constraint-spans', pagination: false, depth: 6,
  }).then((cs: any) => cs.docs)
  const smDefs = await payload.find({
    collection: 'state-machine-definitions', pagination: false, depth: 4, where: domainFilter,
  }).then((s: any) => s.docs)

  const lines: string[] = []

  // Entity types
  const entities = nouns.filter((n: any) => n.objectType === 'entity')
  const values = nouns.filter((n: any) => n.objectType === 'value')

  if (entities.length) {
    lines.push('# Entity Types')
    lines.push('')
    for (const e of entities) {
      const refScheme = e.referenceScheme?.map((r: any) =>
        typeof r === 'object' ? r.name : r
      ).join(', ')
      const superType = typeof e.superType === 'object' ? e.superType?.name : null
      let line = e.name
      if (refScheme) line += ` (${refScheme})`
      if (superType) line += ` : ${superType}`
      lines.push(line)
    }
    lines.push('')
  }

  if (values.length) {
    lines.push('# Value Types')
    lines.push('')
    for (const v of values) {
      let line = v.name
      const parts: string[] = []
      if (v.valueType) parts.push(v.valueType)
      if (v.format) parts.push(`format: ${v.format}`)
      if (v.pattern) parts.push(`pattern: ${v.pattern}`)
      if (v.enum) parts.push(`enum: ${v.enum}`)
      if (parts.length) line += ` (${parts.join(', ')})`
      lines.push(line)
    }
    lines.push('')
  }

  // Readings as FORML2
  if (readings.length) {
    lines.push('# Readings')
    lines.push('')
    for (const r of readings) {
      if (!r.text) continue
      // Find constraints on this reading's graph schema roles
      const gsId = typeof r.graphSchema === 'object' ? r.graphSchema?.id : r.graphSchema
      const roleConstraints = constraintSpans.filter((cs: any) => {
        const roles = cs.roles || []
        return roles.some((role: any) => {
          const roleGs = typeof role === 'object' ? (typeof role.graphSchema === 'object' ? role.graphSchema?.id : role.graphSchema) : null
          return roleGs === gsId
        })
      })

      let constraintSuffix = ''
      for (const cs of roleConstraints) {
        const constraint = typeof cs.constraint === 'object' ? cs.constraint : null
        if (constraint) {
          const kind = constraint.kind || ''
          const modality = constraint.modality === 'Deontic' ? 'D' : ''
          constraintSuffix += ` [${modality}${kind}]`
        }
      }

      lines.push(r.text + constraintSuffix)
    }
    lines.push('')
  }

  // State machines
  for (const sm of smDefs) {
    lines.push(`# State Machine: ${sm.title || sm.id}`)
    lines.push('')
    const statuses = sm.statuses?.docs || []
    const transitions = await payload.find({
      collection: 'transitions',
      where: {
        or: statuses.map((s: any) => ({ from: { equals: typeof s === 'object' ? s.id : s } })),
      },
      depth: 2,
      pagination: false,
    }).then((t: any) => t.docs)

    for (const t of transitions) {
      const from = typeof t.from === 'object' ? t.from?.name : t.from
      const to = typeof t.to === 'object' ? t.to?.name : t.to
      const event = typeof t.eventType === 'object' ? t.eventType?.name : t.eventType
      if (from && to && event) {
        lines.push(`${sm.title} transitions from ${from} to ${to} on ${event}`)
      }
    }
    lines.push('')
  }

  return { text: lines.join('\n'), format: 'forml2' }
}
```

**Step 4: Commit**

```bash
git add src/collections/Generator.ts
git commit -m "feat: readings generator output — extract DB state to FORML2 for source control"
```

---

### Task 6: Update Seed Endpoint to Use Unified Parser

Wire the `/seed` POST endpoint to accept plain text and route through the unified parser.

**Files:**
- Modify: `src/app/seed/route.ts`

**Step 1: Read the current seed route**

Check `src/app/seed/route.ts` for the current POST handler structure.

**Step 2: Add a plain text seed path**

Add to the POST handler, before the existing markdown-based logic:

```typescript
// If body has 'text' field, use the unified parser
if (body.text && body.domain) {
  const { seedReadingsFromText } = await import('../../seed/unified')
  const domainResult = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: body.domain } },
    limit: 1,
  })
  let domainId: string
  if (domainResult.docs.length) {
    domainId = domainResult.docs[0].id
  } else {
    const newDomain = await payload.create({
      collection: 'domains',
      data: { domainSlug: body.domain, name: body.domain },
    })
    domainId = newDomain.id
  }

  const result = await seedReadingsFromText(payload, {
    text: body.text,
    domainId,
    tenant: body.tenant,
  })
  return Response.json(result)
}
```

**Step 3: Commit**

```bash
git add src/app/seed/route.ts
git commit -m "feat: seed endpoint accepts plain text via unified parser"
```

---

### Task 7: Integration Test — Round-Trip Fidelity

Verify that readings seeded from text can be extracted back and re-seeded to produce the same state.

**Files:**
- Create: `src/parse/roundtrip.test.ts`

**Step 1: Write the round-trip test**

```typescript
// src/parse/roundtrip.test.ts
import { describe, it, expect } from 'vitest'

describe('readings round-trip', () => {
  it('seed text → extract readings → re-seed produces same nouns and readings', async () => {
    const { getPayload } = await import('payload')
    const payload = await getPayload({ config: (await import('@payload-config')).default })
    const { seedReadingsFromText } = await import('../seed/unified')

    // Create domain
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'roundtrip-test', name: 'Round Trip Test' },
    })

    // Seed from text
    const inputText = `Customer has Name
Customer submits SupportRequest
SupportRequest has Priority
Admin is a subtype of Customer`

    const seedResult = await seedReadingsFromText(payload, {
      text: inputText,
      domainId: domain.id,
    })
    expect(seedResult.errors).toHaveLength(0)

    // Count what was created
    const nouns1 = await payload.find({
      collection: 'nouns',
      where: { domain: { equals: domain.id } },
      pagination: false,
    })
    const readings1 = await payload.find({
      collection: 'readings',
      where: { domain: { equals: domain.id } },
      pagination: false,
    })

    // Verify basic counts
    expect(nouns1.docs.length).toBeGreaterThanOrEqual(4)
    expect(readings1.docs.length).toBeGreaterThanOrEqual(3) // subtype doesn't create a reading

    // Verify subtype was set
    const admin = nouns1.docs.find((n: any) => n.name === 'Admin')
    expect(admin?.superType).toBeTruthy()
  })
})
```

**Step 2: Run the test**

Run: `npx vitest run src/parse/roundtrip.test.ts`
Expected: PASS

**Step 3: Commit**

```bash
git add src/parse/roundtrip.test.ts
git commit -m "test: round-trip integration test — seed text, verify DB state"
```

---

## Summary

| Task | What it does | Depends on |
|------|-------------|------------|
| 1 | FORML2 verbal constraint parser | — |
| 2 | Unified parseText entry point | Task 1 |
| 3 | seedReadingsFromText using unified parser | Task 2 |
| 4 | Generated collections with domain scoping + materialize script | — |
| 5 | Readings generator output (DB → text) | — |
| 6 | Seed endpoint accepts plain text | Task 3 |
| 7 | Round-trip integration test | Tasks 3, 5 |

Tasks 1-2-3-6 are the parser chain (serial). Tasks 4, 5 are independent and can be done in parallel with the parser work. Task 7 ties it together.
