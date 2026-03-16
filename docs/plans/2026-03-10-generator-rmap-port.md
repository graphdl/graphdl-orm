# Generator RMap Engine Port — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Port the 2,851-line RMap engine (Generator.ts from Payload CMS) to pure functions running on the Cloudflare DO + SQLite architecture, and add a `sqlite` output format that generates CREATE TABLE DDL from readings — making the hand-written DDL in `src/schema/*.ts` replaceable by generated output.

**Architecture:** The Generator's core is factored into three layers: (1) Pure RMap functions with zero Payload/DO dependency (predicate parsing, property naming, constraint analysis), tested in isolation. (2) Data-fetching adapters that query the DO's SQLite via `findInCollection()` and feed normalized data to the pure functions. (3) Output format renderers (openapi, sqlite, xstate, ilayer, readings) that take the intermediate OpenAPI representation and produce target-specific output. A new `POST /api/generate` route wires it all together.

**Tech Stack:** TypeScript, Cloudflare Workers, Durable Objects, SQLite (via `SqlStorage`), Vitest, itty-router 5

**Source reference:** `readings/Generator.ts.bak` (commit `ddb8880`)

---

## Task 1: Port RMap Pure Functions — Predicate Parsing & Property Naming

These are the foundational string-manipulation functions that have zero external dependencies. They transform reading text into property names for schemas.

**Files:**
- Create: `src/generate/rmap.ts`
- Create: `src/generate/rmap.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/rmap.test.ts
import { describe, it, expect } from 'vitest'
import {
  nameToKey,
  transformPropertyName,
  extractPropertyName,
  toPredicate,
  findPredicateObject,
  nounListToRegex,
} from './rmap'

describe('nameToKey', () => {
  it('removes spaces and hyphens', () => {
    expect(nameToKey('Support Request')).toBe('SupportRequest')
    expect(nameToKey('Make-Model')).toBe('MakeModel')
  })
  it('replaces ampersands', () => {
    expect(nameToKey('Terms & Conditions')).toBe('TermsAndConditions')
  })
})

describe('transformPropertyName', () => {
  it('lowercases first letter of PascalCase', () => {
    expect(transformPropertyName('Priority')).toBe('priority')
  })
  it('handles all-caps', () => {
    expect(transformPropertyName('VIN')).toBe('vin')
  })
  it('handles leading uppercase runs', () => {
    expect(transformPropertyName('APIKey')).toBe('apiKey')
    expect(transformPropertyName('HTTPMethod')).toBe('httpMethod')
  })
  it('returns empty string for undefined', () => {
    expect(transformPropertyName(undefined)).toBe('')
  })
})

describe('extractPropertyName', () => {
  it('extracts from single word', () => {
    expect(extractPropertyName(['Priority'])).toBe('priority')
  })
  it('extracts from multi-word', () => {
    expect(extractPropertyName(['Cost', 'Center'])).toBe('costCenter')
  })
})

describe('nounListToRegex', () => {
  it('creates regex matching noun names', () => {
    const nouns = [{ name: 'Customer', id: '1' }, { name: 'SupportRequest', id: '2' }]
    const regex = nounListToRegex(nouns)
    expect(regex.test('Customer')).toBe(true)
    expect(regex.test('SupportRequest')).toBe(true)
    expect(regex.test('Unknown')).toBe(false)
  })
  it('sorts longer names first', () => {
    const nouns = [{ name: 'Request', id: '1' }, { name: 'SupportRequest', id: '2' }]
    const regex = nounListToRegex(nouns)
    const match = 'SupportRequest'.match(regex)
    expect(match?.[0]).toBe('SupportRequest')
  })
})

describe('toPredicate', () => {
  it('tokenizes reading by noun names', () => {
    const nouns = [{ name: 'Customer', id: '1' }, { name: 'SupportRequest', id: '2' }]
    const result = toPredicate({ reading: 'Customer submits SupportRequest', nouns })
    expect(result).toEqual(['Customer', 'submits', 'SupportRequest'])
  })
  it('handles multi-word predicates', () => {
    const nouns = [{ name: 'Employee', id: '1' }, { name: 'Department', id: '2' }]
    const result = toPredicate({ reading: 'Employee works in Department', nouns })
    expect(result).toEqual(['Employee', 'works', 'in', 'Department'])
  })
})

describe('findPredicateObject', () => {
  it('finds object after subject', () => {
    const predicate = ['Customer', 'has', 'Priority']
    const subject = { name: 'Customer', id: '1' }
    const object = { name: 'Priority', id: '2' }
    const result = findPredicateObject({ predicate, subject, object })
    expect(result.objectBegin).toBe(2)
    expect(result.objectEnd).toBe(3)
  })
  it('skips verbs and prepositions', () => {
    const predicate = ['Service', 'runs', 'on', 'CostCenter']
    const subject = { name: 'Service', id: '1' }
    const object = { name: 'CostCenter', id: '2' }
    const result = findPredicateObject({ predicate, subject, object })
    expect(result.objectBegin).toBe(3)
    expect(result.objectEnd).toBe(4)
  })
})
```

### Step 2: Run tests to verify they fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/rmap.test.ts`
Expected: FAIL — module `./rmap` not found

### Step 3: Write minimal implementation

```typescript
// src/generate/rmap.ts

/**
 * RMap pure functions — predicate parsing, property naming, constraint analysis.
 *
 * Ported from Generator.ts (commit ddb8880), lines 2578-2841.
 * Zero external dependencies. Operates on plain objects, not Payload types.
 */

/** Noun-like object — minimal shape needed by RMap functions. */
export interface NounRef {
  id: string
  name?: string | null
  plural?: string | null
  objectType?: string
  valueType?: string | null
  format?: string | null
  pattern?: string | null
  enumValues?: string | null
  minimum?: number | null
  maximum?: number | null
  superType?: string | NounRef | null
  referenceScheme?: (string | NounRef)[] | null
}

/**
 * Convert a noun name to a schema key (remove spaces, hyphens, replace &).
 * Source: Generator.ts:2578-2580
 */
export function nameToKey(name: string): string {
  return name.replace(/[ \-]/g, '').replace(/&/g, 'And')
}

/**
 * Transform a property name to camelCase following RMap conventions.
 * Handles all-caps (VIN → vin), leading uppercase runs (APIKey → apiKey).
 * Source: Generator.ts:2814-2827
 */
export function transformPropertyName(propertyName?: string): string {
  if (!propertyName) return ''
  propertyName = nameToKey(propertyName)
  if (propertyName === propertyName.toUpperCase()) return propertyName.toLowerCase()
  const leadingUpper = propertyName.match(/^[A-Z]+/)
  if (leadingUpper) {
    const run = leadingUpper[0]
    if (run.length === propertyName.length) return propertyName.toLowerCase()
    if (run.length > 1) return run.slice(0, -1).toLowerCase() + propertyName.slice(run.length - 1)
  }
  return propertyName[0].toLowerCase() + propertyName.slice(1).replace(/ /g, '')
}

/**
 * Extract a property name from a reading's object tokens.
 * Source: Generator.ts:2829-2841
 */
export function extractPropertyName(objectReading: string[]): string {
  const propertyNamePrefix = objectReading[0].split(' ')
  const propertyName = transformPropertyName(
    propertyNamePrefix
      .map((n) => (n === n.toUpperCase() ? n[0].toUpperCase() + n.slice(1).toLowerCase() : n))
      .join('') +
      objectReading
        .slice(1)
        .map((r) => r[0].toUpperCase() + r.slice(1))
        .join(''),
  )
  return propertyName
}

/**
 * Build a regex that matches any noun name in the list, longest first.
 * Source: Generator.ts:2706-2718
 */
export function nounListToRegex(nouns?: NounRef[]): RegExp {
  return nouns
    ? new RegExp(
        '(' +
          nouns
            .filter((n) => n.name)
            .map((n) => '\\b' + n.name + '\\b-?')
            .sort((a, b) => b.length - a.length)
            .join('|') +
          ')',
      )
    : new RegExp('')
}

/**
 * Tokenize a reading string into an array of noun names and predicate words.
 * Source: Generator.ts:2720-2741
 */
export function toPredicate({
  reading,
  nouns,
  nounRegex,
}: {
  reading: string
  nouns: NounRef[]
  nounRegex?: RegExp
}): string[] {
  return reading
    .split(nounRegex || nounListToRegex(nouns))
    .flatMap((token) =>
      nouns.find((n) => n.name === token.replace(/-$/, ''))
        ? token
        : token
            .trim()
            .split(' ')
            .map((word) => word.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase())),
    )
    .filter((word) => word)
}

/**
 * Find the position of the object noun in a tokenized predicate.
 * Returns { objectBegin, objectEnd } indices into the predicate array.
 * Source: Generator.ts:2653-2704
 */
export function findPredicateObject({
  predicate,
  subject,
  object,
  plural,
}: {
  predicate: string[]
  subject: NounRef
  object?: NounRef
  plural?: string | null
}): { objectBegin: number; objectEnd: number } {
  let subjectIndex = predicate.indexOf(subject.name || '')
  if (subjectIndex === -1 && subject.name)
    subjectIndex = predicate.indexOf(subject.name + '-' || '')
  if (subjectIndex === -1) return { objectBegin: 0, objectEnd: 0 }

  let objectIndex = !object ? -1 : predicate.indexOf(object.name || '')
  if (object && objectIndex === -1 && object.name)
    objectIndex = predicate.indexOf(object.name + '-' || '')
  if (object && objectIndex === -1)
    throw new Error(`Object "${object.name}" not found in predicate "${predicate.join(' ')}"`)

  if (plural) predicate[objectIndex] = plural[0].toUpperCase() + plural.slice(1)
  let objectBegin: number
  let objectEnd: number
  if (objectIndex === -1) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate.length
  } else if (subjectIndex < objectIndex) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate[objectIndex].endsWith('-') ? predicate.length : objectIndex + 1
  } else {
    objectBegin = 0
    objectEnd = objectIndex + 1
  }
  while (objectIndex > -1 && !predicate[objectBegin].endsWith('-') && objectBegin < objectIndex - 1)
    objectBegin++
  if (objectBegin < objectIndex) {
    const token = predicate[objectBegin].toLowerCase()
    const verbsAndPrepositions = [
      'has', 'is', 'was', 'are', 'were', 'been',
      'to', 'via', 'from', 'for', 'on', 'of', 'in', 'at', 'by', 'with', 'as',
      'the', 'a', 'an',
      'belongs', 'arrives', 'leads', 'sources', 'includes', 'concerns',
      'submits', 'sends', 'affects', 'involves', 'authenticates',
      'manufactured', 'connects', 'charges', 'covers', 'data',
    ]
    if (verbsAndPrepositions.includes(token)) objectBegin++
  }
  return { objectBegin, objectEnd }
}
```

### Step 4: Run tests to verify they pass

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/rmap.test.ts`
Expected: PASS — all 11 tests green

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/rmap.ts src/generate/rmap.test.ts
git commit -m "feat: port RMap pure functions (predicate parsing, property naming)"
```

---

## Task 2: Port Schema Builder — ensureTableExists, createProperty, setTableProperty

The schema builder functions create OpenAPI-style JSON Schema objects from noun metadata. They depend only on the pure functions from Task 1.

**Files:**
- Create: `src/generate/schema-builder.ts`
- Create: `src/generate/schema-builder.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/schema-builder.test.ts
import { describe, it, expect } from 'vitest'
import {
  ensureTableExists,
  createProperty,
  setTableProperty,
} from './schema-builder'
import type { NounRef } from './rmap'

describe('ensureTableExists', () => {
  it('creates Update, New, and base schemas', () => {
    const tables: Record<string, any> = {}
    const subject: NounRef = { id: '1', name: 'Customer', objectType: 'entity' }
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })
    expect(tables['UpdateCustomer']).toBeDefined()
    expect(tables['NewCustomer']).toBeDefined()
    expect(tables['Customer']).toBeDefined()
    expect(tables['UpdateCustomer'].title).toBe('Customer')
    expect(tables['NewCustomer'].allOf).toBeDefined()
  })

  it('is idempotent', () => {
    const tables: Record<string, any> = {}
    const subject: NounRef = { id: '1', name: 'Customer', objectType: 'entity' }
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })
    expect(Object.keys(tables)).toHaveLength(3)
  })
})

describe('createProperty', () => {
  it('creates string property for string value type', () => {
    const noun: NounRef = { id: '1', name: 'Name', valueType: 'string' }
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('string')
  })

  it('creates number property for number value type', () => {
    const noun: NounRef = { id: '1', name: 'Amount', valueType: 'number' }
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('number')
  })

  it('creates enum property', () => {
    const noun: NounRef = { id: '1', name: 'Status', valueType: 'string', enumValues: 'Active, Inactive' }
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.enum).toEqual(['Active', 'Inactive'])
  })

  it('creates oneOf for entity references', () => {
    const entity: NounRef = { id: '1', name: 'Customer', objectType: 'entity', referenceScheme: [] }
    const tables: Record<string, any> = {}
    const result = createProperty({ object: entity, nouns: [entity], tables, jsonExamples: {} })
    expect(result.oneOf).toBeDefined()
  })
})

describe('setTableProperty', () => {
  it('adds property to UpdateX schema', () => {
    const tables: Record<string, any> = {}
    const subject: NounRef = { id: '1', name: 'Customer' }
    const object: NounRef = { id: '2', name: 'Priority', valueType: 'string' }
    ensureTableExists({ tables, subject, nouns: [subject, object], jsonExamples: {} })
    setTableProperty({
      tables,
      subject,
      object,
      nouns: [subject, object],
      description: 'Customer has Priority',
      required: false,
      property: { type: 'string' },
      jsonExamples: {},
    })
    expect(tables['UpdateCustomer'].properties?.priority).toBeDefined()
    expect(tables['UpdateCustomer'].properties?.priority.type).toBe('string')
  })

  it('strips subject name prefix from property name', () => {
    const tables: Record<string, any> = {}
    const subject: NounRef = { id: '1', name: 'Customer' }
    const object: NounRef = { id: '2', name: 'CustomerName', valueType: 'string' }
    ensureTableExists({ tables, subject, nouns: [subject, object], jsonExamples: {} })
    setTableProperty({
      tables,
      subject,
      object,
      nouns: [subject, object],
      property: { type: 'string' },
      jsonExamples: {},
    })
    expect(tables['UpdateCustomer'].properties?.name).toBeDefined()
  })

  it('adds required to NewX schema', () => {
    const tables: Record<string, any> = {}
    const subject: NounRef = { id: '1', name: 'Order' }
    const object: NounRef = { id: '2', name: 'Total', valueType: 'number' }
    ensureTableExists({ tables, subject, nouns: [subject, object], jsonExamples: {} })
    setTableProperty({
      tables,
      subject,
      object,
      nouns: [subject, object],
      required: true,
      property: { type: 'number' },
      jsonExamples: {},
    })
    expect(tables['NewOrder'].required).toContain('total')
  })
})
```

### Step 2: Run tests to verify they fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/schema-builder.test.ts`
Expected: FAIL — module `./schema-builder` not found

### Step 3: Write minimal implementation

```typescript
// src/generate/schema-builder.ts

/**
 * Schema builder — creates OpenAPI-style JSON Schema objects from noun metadata.
 *
 * Ported from Generator.ts (commit ddb8880), lines 2480-2651.
 * Depends only on rmap.ts pure functions.
 */
import { nameToKey, transformPropertyName, type NounRef } from './rmap'

type JSONSchema = Record<string, any>
type JSONSchemaType = any

/**
 * Create a JSON Schema property definition from a noun.
 * Value types → primitive properties. Entity types → oneOf with $ref.
 * Source: Generator.ts:2480-2576
 */
export function createProperty({
  description,
  object,
  nouns,
  tables,
  jsonExamples,
}: {
  description?: string
  object: NounRef
  nouns: NounRef[]
  tables: Record<string, JSONSchema>
  jsonExamples: Record<string, JSONSchemaType>
}): JSONSchema {
  if (!object) return {}
  if (typeof object === 'string') {
    object = nouns.find((n) => n.id === (object as any)) || ({ id: object, name: object } as any)
  } else if (object.id) {
    object = nouns.find((n) => n.id === object.id) || object
  }
  const property: JSONSchema = {}
  let referenceScheme = object.referenceScheme as (string | NounRef)[] | null | undefined
  let superType = object.superType as string | NounRef | null | undefined
  let valueType = object.valueType

  while (!referenceScheme?.length && !valueType && superType) {
    if (typeof superType === 'string') superType = nouns.find((n) => n.id === superType) as NounRef
    referenceScheme = superType?.referenceScheme as (string | NounRef)[] | null | undefined
    valueType = superType?.valueType
    superType = superType?.superType
  }

  if (valueType) {
    property.type = valueType
    if (object.format) property.format = object.format?.toString()
    if (object.pattern) property.pattern = object.pattern?.toString()
    if (object.enumValues) {
      property.enum = object.enumValues.split(',').map((e: string) => {
        const val = e.trim()
        if (val === 'null') {
          property.nullable = true
          return null
        }
        return val
      })
    }
    if (typeof object.minimum === 'number') property.minimum = object.minimum
    if (typeof object.maximum === 'number') property.maximum = object.maximum
    if (description) property.description = description
  } else {
    if (typeof referenceScheme === 'string')
      referenceScheme = [nouns.find((n) => n.id === referenceScheme?.toString()) as NounRef]
    const required: string[] = []
    const propertyKey = nameToKey(object.name || '')
    property.oneOf = [
      (referenceScheme?.length || 0) > 1
        ? {
            type: 'object',
            properties: Object.fromEntries(
              (referenceScheme || []).map((role) => {
                if (typeof role === 'string') role = nouns.find((n) => n.id === role) as NounRef
                const propName = transformPropertyName(role.name || '')
                required.push(propName)
                return [propName, createProperty({ object: role, tables, nouns, description, jsonExamples })]
              }),
            ),
            required,
          }
        : referenceScheme
          ? createProperty({
              object:
                typeof referenceScheme[0] === 'string'
                  ? (nouns.find((n) => n.id === referenceScheme?.[0]) as NounRef)
                  : referenceScheme[0] as NounRef,
              tables,
              nouns,
              description,
              jsonExamples,
            })
          : {},
      { $ref: '#/components/schemas/' + propertyKey },
    ]
    ensureTableExists({ tables, subject: object, nouns, jsonExamples })
  }
  return property
}

/**
 * Ensure Update/New/Base schema triplet exists for a noun.
 * Source: Generator.ts:2582-2651
 */
export function ensureTableExists({
  tables,
  subject,
  nouns,
  jsonExamples,
}: {
  tables: Record<string, JSONSchema>
  subject: NounRef
  nouns: NounRef[]
  jsonExamples: Record<string, JSONSchemaType>
}): void {
  const title = subject.name || ''
  const key = nameToKey(title)
  if (tables[key]) return
  tables['Update' + key] = { $id: 'Update' + key, title: subject.name || '' }
  tables['New' + key] = { $id: 'New' + key, allOf: [{ $ref: '#/components/schemas/Update' + key }] }
  tables[key] = { $id: key, allOf: [{ $ref: '#/components/schemas/New' + key }] }

  const json = jsonExamples[key]
  if (json) {
    tables['Update' + key].examples = [json]
    tables['New' + key].examples = [json]
    tables[key].examples = [json]
  }

  // Unpack reference scheme
  if (subject.referenceScheme) {
    let refs = subject.referenceScheme as (string | NounRef)[]
    if (!Array.isArray(refs)) refs = [nouns.find((n) => n.id === refs?.toString()) as NounRef]
    for (let idRole of refs) {
      if (typeof idRole === 'string') idRole = nouns.find((n) => n.id === idRole) as NounRef
      if (!idRole) continue
      const prop = createProperty({ object: idRole, nouns, tables, jsonExamples })
      setTableProperty({
        tables,
        subject,
        object: idRole,
        nouns,
        required: true,
        property: prop,
        description: `${title} is uniquely identified by ${idRole.name}`,
        jsonExamples,
      })
    }
  }

  // Wire supertype chain
  let superType: NounRef | string | null | undefined = subject.superType
  if (typeof superType === 'string') superType = nouns?.find((n) => n.id === superType) as NounRef
  if (superType?.name) {
    const superTypeKey = nameToKey(superType.name || '')
    tables['Update' + key].allOf = [{ $ref: '#/components/schemas/Update' + superTypeKey }]
    tables['New' + key].allOf?.push({ $ref: '#/components/schemas/New' + superTypeKey })
    tables[key].allOf?.push({ $ref: '#/components/schemas/' + superTypeKey })
    ensureTableExists({ tables, subject: superType, nouns, jsonExamples })
  } else {
    tables['Update' + key].type = 'object'
  }
}

/**
 * Set a property on a schema table, with subject-name prefix stripping.
 * Source: Generator.ts:2743-2812
 */
export function setTableProperty({
  tables,
  nouns,
  subject,
  object,
  propertyName,
  description,
  required,
  property,
  example,
  jsonExamples,
}: {
  tables: Record<string, JSONSchema>
  nouns: NounRef[]
  subject: NounRef
  object: NounRef
  propertyName?: string
  description?: string
  required?: boolean
  property?: JSONSchema
  example?: any
  jsonExamples: Record<string, JSONSchemaType>
}): void {
  if (!property) property = createProperty({ object, tables, nouns, jsonExamples })
  if (description) property.description = description

  propertyName ||= transformPropertyName(object.name || '')
  // Strip subject name prefix from property name
  const compareName = subject.name?.replace(/ /g, '')?.toUpperCase() || ''
  if (
    subject.name &&
    propertyName.toUpperCase().startsWith(compareName) &&
    propertyName.length > compareName.length &&
    propertyName[compareName.length] === propertyName[compareName.length].toUpperCase()
  ) {
    propertyName = transformPropertyName(propertyName.slice(compareName.length))
  }
  const key = nameToKey('Update' + (subject.name || ''))
  const properties = tables[key].properties ?? {}
  properties[propertyName] = property
  tables[key].properties = properties

  if (required) {
    const reqKey = nameToKey((propertyName === 'id' ? '' : 'New') + (subject.name || ''))
    if (!tables[reqKey].required) tables[reqKey].required = []
    tables[reqKey].required?.push(propertyName)
  }

  if (example) {
    const examples = (tables[key].examples as any[]) || [{}]
    switch (property.type) {
      case 'integer': (examples[0])[propertyName] = parseInt(example); break
      case 'number': (examples[0])[propertyName] = parseFloat(example); break
      case 'boolean': (examples[0])[propertyName] = example === 'true'; break
      default: (examples[0])[propertyName] = example; break
    }
    tables[key].examples = examples
  }
}
```

### Step 4: Run tests to verify they pass

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/schema-builder.test.ts`
Expected: PASS — all tests green

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/schema-builder.ts src/generate/schema-builder.test.ts
git commit -m "feat: port schema builder (ensureTableExists, createProperty, setTableProperty)"
```

---

## Task 3: Port Fact Type Processors — Binary, Array, Unary

These functions process graph schemas with constraints to determine which readings produce object properties, array properties, or boolean flags.

**Files:**
- Create: `src/generate/fact-processors.ts`
- Create: `src/generate/fact-processors.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/fact-processors.test.ts
import { describe, it, expect } from 'vitest'
import {
  processBinarySchemas,
  processArraySchemas,
  processUnarySchemas,
} from './fact-processors'
import type { NounRef } from './rmap'

// Minimal graph schema shape for testing
function makeGraphSchema(overrides: Record<string, any> = {}) {
  return {
    id: overrides.id || 'gs1',
    name: overrides.name || 'TestSchema',
    roles: { docs: overrides.roles || [] },
    readings: { docs: overrides.readings || [] },
    ...overrides,
  }
}

function makeRole(id: string, nounId: string, nounName: string) {
  return {
    id,
    noun: { value: { id: nounId, name: nounName, objectType: 'value', valueType: 'string' } },
    graphSchema: { id: 'gs1' },
  }
}

describe('processBinarySchemas', () => {
  it('adds property to subject schema for single-role UC', () => {
    const customer: NounRef = { id: 'n1', name: 'Customer', objectType: 'entity' }
    const priority: NounRef = { id: 'n2', name: 'Priority', objectType: 'value', valueType: 'string' }
    const nouns = [customer, priority]
    const schemas: Record<string, any> = {}

    const graphSchema = makeGraphSchema({
      roles: [
        { id: 'r1', noun: { value: customer }, graphSchema: { id: 'gs1' } },
        { id: 'r2', noun: { value: priority }, graphSchema: { id: 'gs1' } },
      ],
      readings: [{ text: 'Customer has Priority' }],
    })
    const constraintSpans = [{
      roles: [{ ...graphSchema.roles.docs[0], graphSchema: { id: 'gs1' } }],
    }]

    processBinarySchemas(constraintSpans, schemas, nouns, {}, undefined, [], [graphSchema])
    expect(schemas['UpdateCustomer']?.properties?.priority).toBeDefined()
  })
})

describe('processUnarySchemas', () => {
  it('adds boolean property for single-role schema', () => {
    const customer: NounRef = { id: 'n1', name: 'Customer', objectType: 'entity' }
    const nouns = [customer]
    const schemas: Record<string, any> = {}

    const graphSchema = makeGraphSchema({
      roles: [{ id: 'r1', noun: { value: customer }, graphSchema: { id: 'gs1' } }],
      readings: [{ text: 'Customer is active' }],
    })

    processUnarySchemas([graphSchema], nouns, undefined, schemas, {}, [])
    expect(schemas['UpdateCustomer']?.properties).toBeDefined()
    // Unary facts become booleans
    const props = schemas['UpdateCustomer']?.properties || {}
    const boolProp = Object.values(props).find((p: any) => p.type === 'boolean')
    expect(boolProp).toBeDefined()
  })
})
```

### Step 2: Run tests to verify they fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/fact-processors.test.ts`
Expected: FAIL — module not found

### Step 3: Write minimal implementation

```typescript
// src/generate/fact-processors.ts

/**
 * Fact type processors — binary, array, and unary schema processing.
 *
 * Ported from Generator.ts (commit ddb8880), lines 2148-2295.
 * Depends on rmap.ts and schema-builder.ts.
 */
import {
  toPredicate,
  findPredicateObject,
  extractPropertyName,
  transformPropertyName,
  nameToKey,
  nounListToRegex,
  type NounRef,
} from './rmap'
import { ensureTableExists, createProperty, setTableProperty } from './schema-builder'

type JSONSchema = Record<string, any>

/**
 * Process binary fact types (single-role uniqueness constraints).
 * Each constrained role's reading becomes a property on the subject entity.
 * Source: Generator.ts:2187-2253
 */
export function processBinarySchemas(
  constraintSpans: any[],
  schemas: Record<string, JSONSchema>,
  nouns: NounRef[],
  jsonExamples: Record<string, any>,
  nounRegex: RegExp | undefined,
  examples: any[],
  graphSchemas: any[],
): void {
  const regex = nounRegex || nounListToRegex(nouns)
  for (const { propertySchema, subjectRole } of constraintSpans
    .filter((cs) => cs.roles?.length === 1)
    .map((cs) => {
      const constrainedRole = cs.roles[0]
      const nestedGs = constrainedRole.graphSchema
      const propertySchema = graphSchemas.find((gs: any) => gs.id === (nestedGs?.id || nestedGs))
      return { propertySchema, subjectRole: propertySchema ? constrainedRole : undefined }
    })) {
    if (!subjectRole || !propertySchema) continue

    const subject = subjectRole.noun?.value as NounRef
    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    const objectRole = propertySchema.roles?.docs?.find((r: any) => r.id !== subjectRole.id)
    const objectNounValue = objectRole?.noun?.value
    const object = (typeof objectNounValue === 'string'
      ? nouns.find((n) => n.id === objectNounValue)
      : objectNounValue) as NounRef
    const reading = propertySchema.readings?.docs?.[0]
    if (!reading?.text) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex: regex })
    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object })

    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)

    setTableProperty({
      tables: schemas,
      subject,
      object: object as NounRef,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: subjectRole.required || false,
      property: createProperty({ object: object as NounRef, nouns, tables: schemas, jsonExamples }),
      jsonExamples,
    })
  }
}

/**
 * Process array fact types (compound uniqueness constraints with no parent reference).
 * These become array properties on the subject entity.
 * Source: Generator.ts:2148-2185
 */
export function processArraySchemas(
  arrayTypes: { gs: any; cs: any }[],
  nouns: NounRef[],
  nounRegex: RegExp | undefined,
  schemas: Record<string, JSONSchema>,
  jsonExamples: Record<string, any>,
): void {
  const regex = nounRegex || nounListToRegex(nouns)
  for (const { gs: schema } of arrayTypes) {
    const reading = schema.readings?.docs?.[0]
    if (!reading?.text) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex: regex })
    const subjectRaw = schema.roles?.docs?.[0]?.noun?.value
    const subject = (typeof subjectRaw === 'string' ? nouns.find((n) => n.id === subjectRaw) : subjectRaw) as NounRef
    const objectRaw = schema.roles?.docs?.[1]?.noun?.value
    const object = (typeof objectRaw === 'string' ? nouns.find((n) => n.id === objectRaw) : objectRaw) as NounRef
    if (!subject?.name || !object?.name) continue

    const plural = object.plural
    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object, plural })
    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)
    let propertyName = schema.name || extractPropertyName(objectReading) + (plural ? '' : 's')
    propertyName = transformPropertyName(propertyName)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })
    const key = nameToKey('Update' + (subject.name || ''))
    const properties = schemas[key].properties ?? {}

    const property: JSONSchema = {
      type: 'array',
      items: createProperty({ object, nouns, tables: schemas, jsonExamples }),
    }
    property.description = predicate.join(' ')
    properties[propertyName] = property
    schemas[key].properties = properties
  }
}

/**
 * Process unary fact types (single-role schemas → boolean properties).
 * Source: Generator.ts:2255-2295
 */
export function processUnarySchemas(
  graphSchemas: any[],
  nouns: NounRef[],
  nounRegex: RegExp | undefined,
  schemas: Record<string, JSONSchema>,
  jsonExamples: Record<string, any>,
  examples: any[],
): void {
  const regex = nounRegex || nounListToRegex(nouns)
  for (const unarySchema of graphSchemas.filter((s) => s.roles?.docs?.length === 1)) {
    const unaryRole = unarySchema.roles?.docs?.[0]
    const subject = unaryRole?.noun?.value as NounRef
    const reading = unarySchema.readings?.docs?.[0]
    if (!reading?.text) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex: regex })
    const { objectBegin } = findPredicateObject({ predicate, subject })
    const objectReading = predicate.slice(objectBegin)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    setTableProperty({
      tables: schemas,
      subject,
      object: subject as NounRef,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: unaryRole.required || false,
      property: { type: 'boolean' },
      jsonExamples,
    })
  }
}
```

### Step 4: Run tests to verify they pass

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/fact-processors.test.ts`
Expected: PASS — all tests green

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/fact-processors.ts src/generate/fact-processors.test.ts
git commit -m "feat: port fact type processors (binary, array, unary schemas)"
```

---

## Task 4: Port generateOpenAPI — Data Fetching Adapter

This is the main orchestrator that queries the DO for nouns, readings, constraints, graph schemas, and feeds them to the pure functions. The result is the intermediate OpenAPI representation that all other output formats consume.

**Files:**
- Create: `src/generate/openapi.ts`
- Create: `src/generate/openapi.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/openapi.test.ts
import { describe, it, expect } from 'vitest'
import { generateOpenAPI } from './openapi'

// Mock DB that returns pre-canned data
function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any, opts?: any) => {
      const docs = data[slug] || []
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  } as any
}

describe('generateOpenAPI', () => {
  it('creates schemas for entity nouns with readings', async () => {
    const db = mockDB({
      nouns: [
        { id: 'n1', name: 'Customer', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string', domain: 'd1' },
      ],
      'graph-schemas': [{
        id: 'gs1', name: 'CustomerName', domain: 'd1',
        roles: { docs: [
          { id: 'r1', noun: { value: { id: 'n1', name: 'Customer', objectType: 'entity' } }, graphSchema: { id: 'gs1' } },
          { id: 'r2', noun: { value: { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' } }, graphSchema: { id: 'gs1' } },
        ]},
        readings: { docs: [{ text: 'Customer has Name' }] },
      }],
      'constraint-spans': [{
        roles: [{ id: 'r1', noun: { value: { id: 'n1', name: 'Customer', objectType: 'entity' } }, graphSchema: { id: 'gs1' } }],
        constraint: { kind: 'UC' },
      }],
      readings: [],
    })

    const result = await generateOpenAPI(db, 'd1')
    expect(result.components?.schemas?.Customer).toBeDefined()
    expect(result.components?.schemas?.UpdateCustomer?.properties?.name).toBeDefined()
  })

  it('returns empty schemas when domain has no nouns', async () => {
    const db = mockDB({
      nouns: [],
      'graph-schemas': [],
      'constraint-spans': [],
      readings: [],
    })
    const result = await generateOpenAPI(db, 'd1')
    expect(result.components?.schemas).toBeDefined()
  })
})
```

### Step 2: Run tests to verify they fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/openapi.test.ts`
Expected: FAIL — module not found

### Step 3: Write minimal implementation

The key change from the original: replace `payload.find()` calls with `db.findInCollection()` calls. The DO already supports the same query patterns (where, pagination, sort, depth).

```typescript
// src/generate/openapi.ts

/**
 * generateOpenAPI — queries DO for domain model, feeds to pure RMap functions.
 *
 * Ported from Generator.ts (commit ddb8880), lines 299-1139.
 * Uses GraphDLDB.findInCollection() instead of payload.find().
 */
import { nameToKey, nounListToRegex, type NounRef } from './rmap'
import { ensureTableExists, createProperty, setTableProperty } from './schema-builder'
import { processBinarySchemas, processArraySchemas, processUnarySchemas } from './fact-processors'

type JSONSchema = Record<string, any>

/** Fetch all docs from a collection, handling pagination. */
async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where || {}, { limit: 10000 })
  return result.docs
}

/**
 * Generate OpenAPI intermediate representation from domain readings.
 *
 * @param db — GraphDLDB instance (or mock with findInCollection)
 * @param domainId — domain to generate for
 * @returns OpenAPI-style object with components.schemas
 */
export async function generateOpenAPI(db: any, domainId: string): Promise<any> {
  const schemas: Record<string, JSONSchema> = {}
  const domainFilter = { domain: { equals: domainId } }

  // Fetch domain data
  const [graphSchemas, allNouns, domainNouns, constraintSpansRaw] = await Promise.all([
    fetchAll(db, 'graph-schemas', domainFilter),
    fetchAll(db, 'nouns'), // All nouns for cross-domain reference resolution
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'constraint-spans'),
  ])

  // Populate graph schemas with roles and readings
  for (const gs of graphSchemas) {
    if (!gs.roles?.docs) {
      const roles = await fetchAll(db, 'roles', { graphSchema: { equals: gs.id } })
      // Populate each role's noun
      for (const role of roles) {
        if (role.noun && typeof role.noun === 'string') {
          const noun = allNouns.find((n: any) => n.id === role.noun)
          if (noun) role.noun = { value: noun }
        } else if (!role.noun?.value && role.noun) {
          const nounId = typeof role.noun === 'string' ? role.noun : role.noun?.id
          const noun = allNouns.find((n: any) => n.id === nounId)
          if (noun) role.noun = { value: noun }
        }
        role.graphSchema = { id: gs.id }
      }
      gs.roles = { docs: roles }
    }
    if (!gs.readings?.docs) {
      const readings = await fetchAll(db, 'readings', { graphSchema: { equals: gs.id } })
      gs.readings = { docs: readings }
    }
  }

  // Populate constraint spans with role data
  const constraintSpans = []
  for (const cs of constraintSpansRaw) {
    const roleId = cs.role || cs.roleId
    if (!roleId) continue
    // Find the role with populated noun and graphSchema
    let role: any = null
    for (const gs of graphSchemas) {
      role = gs.roles?.docs?.find((r: any) => r.id === roleId)
      if (role) break
    }
    if (!role) continue

    // Group by constraint
    const constraintId = cs.constraint || cs.constraintId
    const existingCS = constraintSpans.find((c: any) =>
      c.constraintId === constraintId || c.roles?.some((r: any) => {
        // Check if same constraint
        return false
      })
    )

    // Build constraint span with roles array
    let spanEntry = constraintSpans.find((s: any) => s._constraintId === constraintId)
    if (!spanEntry) {
      spanEntry = { _constraintId: constraintId, roles: [] }
      constraintSpans.push(spanEntry)
    }
    spanEntry.roles.push(role)
  }

  // Identify compound uniqueness schemas and array types
  const compoundUniqueSchemas = constraintSpans
    .filter((cs: any) => cs.roles?.length > 1)
    .map((cs: any) => {
      const firstGsId = cs.roles[0]?.graphSchema?.id
      if (!firstGsId || !cs.roles.every((r: any) => r.graphSchema?.id === firstGsId)) return undefined
      const gs = graphSchemas.find((s: any) => s.id === firstGsId)
      return gs ? { gs, cs } : undefined
    })
    .filter(Boolean) as { gs: any; cs: any }[]

  const arrayTypes = compoundUniqueSchemas.filter(
    ({ gs }) => !graphSchemas.find((s: any) =>
      s.roles?.docs?.find((r: any) => r.noun?.value?.id === gs.id),
    ),
  )
  const associationSchemas = compoundUniqueSchemas.filter((cs) => !arrayTypes.includes(cs))

  // Add association schemas as nouns
  const nouns: NounRef[] = [...allNouns, ...associationSchemas.map(({ gs }) => gs)]
  const nounRegex = nounListToRegex(nouns)

  // Process association schemas
  for (const { gs: assocSchema, cs } of associationSchemas) {
    const key = (assocSchema.name || '').replace(/ /g, '')
    schemas['Update' + key] = {
      $id: 'Update' + key,
      title: assocSchema.name || '',
      type: 'object',
      description: assocSchema.readings?.docs?.[0]?.text?.replace(/- /, ' '),
    }
    schemas['New' + key] = { $id: 'New' + key, allOf: [{ $ref: '#/components/schemas/Update' + key }] }
    schemas[key] = { $id: key, allOf: [{ $ref: '#/components/schemas/New' + key }] }

    for (const role of assocSchema.roles?.docs || []) {
      const idNoun = role.noun?.value as NounRef
      if (!idNoun) continue
      setTableProperty({
        tables: schemas,
        subject: assocSchema,
        object: idNoun,
        nouns,
        required: cs.roles?.find((r: any) => r.id === role.id) ? true : false,
        description: `${assocSchema.name} is uniquely identified by ${idNoun.name}`,
        property: createProperty({ object: idNoun, tables: schemas, nouns, jsonExamples: {} }),
        jsonExamples: {},
      })
    }
  }

  // Run fact type processors
  processBinarySchemas(constraintSpans, schemas, nouns, {}, nounRegex, [], graphSchemas)
  processArraySchemas(arrayTypes, nouns, nounRegex, schemas, {})
  processUnarySchemas(graphSchemas, nouns, nounRegex, schemas, {}, [])

  // Flatten allOf chains
  for (const [key, schema] of Object.entries(schemas)) {
    while (schema.allOf) {
      let mergedProperties = schema.properties || {}
      const mergedRequired: string[] = [...(schema.required || [])]
      const mergedAllOf: any[] = []
      for (const ref of schema.allOf) {
        const depKey = ref.$ref?.split('/').pop()
        const dependency = schemas[depKey]
        if (!dependency) continue
        if (dependency.required?.length)
          mergedRequired.push(...dependency.required.filter((f: string) => !mergedRequired.includes(f)))
        if (Object.keys(dependency.properties || {}).length)
          mergedProperties = { ...dependency.properties, ...mergedProperties }
        if (dependency.allOf?.length) mergedAllOf.push(...dependency.allOf)
        if (!schema.title && dependency.title) schema.title = dependency.title
        if (!schema.description && dependency.description) schema.description = dependency.description
        if (!schema.type && dependency.type) schema.type = dependency.type
      }
      delete schema.allOf
      if (Object.keys(mergedProperties).length) schema.properties = mergedProperties
      if (mergedRequired.length) schema.required = mergedRequired
      if (mergedAllOf.length) schema.allOf = mergedAllOf
    }
    schemas[key] = schema
  }

  return {
    openapi: '3.0.0',
    info: { title: 'Generated API', version: '1.0.0' },
    components: { schemas },
  }
}
```

### Step 4: Run tests to verify they pass

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/openapi.test.ts`
Expected: PASS — tests green

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/openapi.ts src/generate/openapi.test.ts
git commit -m "feat: port generateOpenAPI to use DO findInCollection"
```

---

## Task 5: Add generateSQLite Output Format

This is the new output format. It takes the OpenAPI intermediate representation (entity schemas with properties) and generates CREATE TABLE DDL. This replaces the hand-written DDL in `src/schema/*.ts`.

**Files:**
- Create: `src/generate/sqlite.ts`
- Create: `src/generate/sqlite.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/sqlite.test.ts
import { describe, it, expect } from 'vitest'
import { generateSQLite } from './sqlite'

describe('generateSQLite', () => {
  it('generates CREATE TABLE from OpenAPI schemas', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateCustomer: {
            title: 'Customer',
            type: 'object',
            properties: {
              name: { type: 'string', description: 'Customer has Name' },
              email: { type: 'string', format: 'email', description: 'Customer has EmailAddress' },
              age: { type: 'integer', description: 'Customer has Age' },
            },
          },
          NewCustomer: { title: 'Customer', type: 'object', properties: {} },
          Customer: { title: 'Customer', type: 'object', properties: {} },
        },
      },
    }
    const result = generateSQLite(openapi)
    expect(result.ddl).toBeDefined()
    expect(result.ddl.length).toBeGreaterThan(0)
    // Should contain a CREATE TABLE for customers
    const customerDDL = result.ddl.find((d: string) => d.includes('customers'))
    expect(customerDDL).toBeDefined()
    expect(customerDDL).toContain('name TEXT')
    expect(customerDDL).toContain('email TEXT')
    expect(customerDDL).toContain('age INTEGER')
  })

  it('adds FK for entity references', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateOrder: {
            title: 'Order',
            type: 'object',
            properties: {
              customer: {
                oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Customer' }],
                description: 'Order belongs to Customer',
              },
              total: { type: 'number' },
            },
          },
          NewOrder: { title: 'Order', type: 'object' },
          Order: { title: 'Order', type: 'object' },
          UpdateCustomer: { title: 'Customer', type: 'object' },
          NewCustomer: { title: 'Customer', type: 'object' },
          Customer: { title: 'Customer', type: 'object' },
        },
      },
    }
    const result = generateSQLite(openapi)
    const orderDDL = result.ddl.find((d: string) => d.includes('CREATE TABLE') && d.includes('orders'))
    expect(orderDDL).toContain('customer_id TEXT REFERENCES customers(id)')
  })

  it('maps types correctly', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateTest: {
            title: 'Test',
            type: 'object',
            properties: {
              count: { type: 'integer' },
              amount: { type: 'number' },
              active: { type: 'boolean' },
              label: { type: 'string' },
            },
          },
          NewTest: { title: 'Test', type: 'object' },
          Test: { title: 'Test', type: 'object' },
        },
      },
    }
    const result = generateSQLite(openapi)
    const ddl = result.ddl.find((d: string) => d.includes('tests'))
    expect(ddl).toContain('count INTEGER')
    expect(ddl).toContain('amount REAL')
    expect(ddl).toContain('active INTEGER')
    expect(ddl).toContain('label TEXT')
  })

  it('generates domain_id FK on every table', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateWidget: { title: 'Widget', type: 'object', properties: { size: { type: 'number' } } },
          NewWidget: { title: 'Widget', type: 'object' },
          Widget: { title: 'Widget', type: 'object' },
        },
      },
    }
    const result = generateSQLite(openapi)
    const ddl = result.ddl.find((d: string) => d.includes('widgets'))
    expect(ddl).toContain('domain_id TEXT REFERENCES domains(id)')
  })

  it('generates indexes for FK columns', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateItem: {
            title: 'Item',
            type: 'object',
            properties: {
              category: { oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Category' }] },
            },
          },
          NewItem: { title: 'Item', type: 'object' },
          Item: { title: 'Item', type: 'object' },
          UpdateCategory: { title: 'Category', type: 'object' },
          NewCategory: { title: 'Category', type: 'object' },
          Category: { title: 'Category', type: 'object' },
        },
      },
    }
    const result = generateSQLite(openapi)
    const indexDDL = result.ddl.find((d: string) => d.includes('CREATE INDEX') && d.includes('category_id'))
    expect(indexDDL).toBeDefined()
  })
})
```

### Step 2: Run tests to verify they fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/sqlite.test.ts`
Expected: FAIL — module not found

### Step 3: Write minimal implementation

```typescript
// src/generate/sqlite.ts

/**
 * generateSQLite — converts OpenAPI schemas to CREATE TABLE DDL.
 *
 * New output format (not in original Generator.ts).
 * Takes the intermediate OpenAPI representation from generateOpenAPI()
 * and produces SQLite DDL that can replace the hand-written schema files.
 */

/** Convert a PascalCase entity name to a snake_case table name. */
function toTableName(name: string): string {
  return name
    .replace(/([A-Z])/g, '_$1')
    .toLowerCase()
    .replace(/^_/, '')
    .replace(/ /g, '_')
    + 's'
}

/** Convert a camelCase property name to a snake_case column name. */
function toColumnName(name: string): string {
  return name
    .replace(/([A-Z])/g, '_$1')
    .toLowerCase()
    .replace(/^_/, '')
}

/** Map JSON Schema type to SQLite type. */
function toSQLiteType(jsonSchemaType?: string): string {
  switch (jsonSchemaType) {
    case 'integer': return 'INTEGER'
    case 'number': return 'REAL'
    case 'boolean': return 'INTEGER' // SQLite has no boolean
    case 'array': return 'TEXT' // JSON serialized
    case 'object': return 'TEXT' // JSON serialized
    case 'string':
    default: return 'TEXT'
  }
}

/** Check if a property is an entity reference (FK). */
function isEntityRef(prop: any): string | null {
  if (prop.oneOf) {
    const ref = prop.oneOf.find((o: any) => o.$ref)
    if (ref) {
      const refName = ref.$ref.split('/').pop()
      return refName || null
    }
  }
  if (prop.$ref) {
    return prop.$ref.split('/').pop() || null
  }
  return null
}

/**
 * Generate SQLite DDL from OpenAPI intermediate representation.
 *
 * @param openapi — OpenAPI object with components.schemas
 * @returns { ddl: string[], tableMap: Record<string, string>, fieldMap: Record<string, Record<string, string>> }
 */
export function generateSQLite(openapi: any): {
  ddl: string[]
  tableMap: Record<string, string>
  fieldMap: Record<string, Record<string, string>>
} {
  const schemas = openapi.components?.schemas || {}
  const ddl: string[] = []
  const tableMap: Record<string, string> = {}
  const fieldMap: Record<string, Record<string, string>> = {}

  // Find entity schemas (those with an UpdateX entry that has type: 'object')
  const entityNames: string[] = []
  for (const [key, schema] of Object.entries(schemas) as [string, any][]) {
    if (key.startsWith('Update') && schema.type === 'object') {
      const entityName = key.slice(6) // Remove 'Update' prefix
      if (schemas[entityName] || schemas['New' + entityName]) {
        entityNames.push(entityName)
      }
    }
  }

  // Pre-compute table names for FK resolution
  for (const entityName of entityNames) {
    tableMap[entityName] = toTableName(entityName)
  }

  // Generate CREATE TABLE for each entity
  for (const entityName of entityNames) {
    const updateSchema = schemas['Update' + entityName]
    const tableName = tableMap[entityName]
    const columns: string[] = [
      'id TEXT PRIMARY KEY',
    ]
    const fkColumns: string[] = []
    const indexes: string[] = []
    const fieldMapping: Record<string, string> = {}

    const properties = updateSchema.properties || {}
    for (const [propName, prop] of Object.entries(properties) as [string, any][]) {
      const colName = toColumnName(propName)

      // Check if this is an entity reference
      const refEntity = isEntityRef(prop)
      if (refEntity && tableMap[refEntity]) {
        const fkCol = colName + '_id'
        const targetTable = tableMap[refEntity]
        fkColumns.push(`${fkCol} TEXT REFERENCES ${targetTable}(id)`)
        fieldMapping[propName] = fkCol
        indexes.push(`CREATE INDEX IF NOT EXISTS idx_${tableName}_${fkCol} ON ${tableName}(${fkCol})`)
      } else {
        const sqlType = toSQLiteType(prop.type)
        columns.push(`${colName} ${sqlType}`)
        if (colName !== propName) fieldMapping[propName] = colName
      }
    }

    // Add domain FK
    fkColumns.push('domain_id TEXT REFERENCES domains(id)')
    fieldMapping['domain'] = 'domain_id'
    indexes.push(`CREATE INDEX IF NOT EXISTS idx_${tableName}_domain ON ${tableName}(domain_id)`)

    // Add standard columns
    columns.push(...fkColumns)
    columns.push(
      "created_at TEXT NOT NULL DEFAULT (datetime('now'))",
      "updated_at TEXT NOT NULL DEFAULT (datetime('now'))",
      'version INTEGER NOT NULL DEFAULT 1',
    )

    ddl.push(`CREATE TABLE IF NOT EXISTS ${tableName} (\n  ${columns.join(',\n  ')}\n)`)
    ddl.push(...indexes)

    fieldMap[tableName] = fieldMapping
  }

  return { ddl, tableMap, fieldMap }
}
```

### Step 4: Run tests to verify they pass

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/sqlite.test.ts`
Expected: PASS — all tests green

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/sqlite.ts src/generate/sqlite.test.ts
git commit -m "feat: add generateSQLite output format (readings → DDL)"
```

---

## Task 6: Port generateXStateFiles

Generates XState machine configs, agent tool schemas, and agent system prompts from state machine definitions.

**Files:**
- Create: `src/generate/xstate.ts`
- Create: `src/generate/xstate.test.ts`

### Step 1: Write the failing tests

```typescript
// src/generate/xstate.test.ts
import { describe, it, expect } from 'vitest'
import { generateXState } from './xstate'

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any) => {
      const docs = data[slug] || []
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  } as any
}

describe('generateXState', () => {
  it('generates state machine config with transitions', async () => {
    const db = mockDB({
      'state-machine-definitions': [{ id: 'smd1', noun: 'n1', domain: 'd1' }],
      nouns: [{ id: 'n1', name: 'SupportRequest', objectType: 'entity' }],
      statuses: [
        { id: 's1', name: 'Received', stateMachineDefinition: 'smd1' },
        { id: 's2', name: 'Investigating', stateMachineDefinition: 'smd1' },
      ],
      transitions: [
        { id: 't1', from: 's1', to: 's2', eventType: 'et1' },
      ],
      'event-types': [{ id: 'et1', name: 'investigate' }],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'd1')
    expect(result.files).toBeDefined()
    const machineFile = Object.entries(result.files).find(([k]) => k.includes('.json') && k.includes('state-machines'))
    expect(machineFile).toBeDefined()
    const config = JSON.parse(machineFile![1])
    expect(config.initial).toBe('Received')
    expect(config.states.Received.on.investigate).toBeDefined()
  })

  it('generates agent tools from events', async () => {
    const db = mockDB({
      'state-machine-definitions': [{ id: 'smd1', noun: 'n1', domain: 'd1' }],
      nouns: [{ id: 'n1', name: 'Order', objectType: 'entity' }],
      statuses: [
        { id: 's1', name: 'Pending', stateMachineDefinition: 'smd1' },
        { id: 's2', name: 'Shipped', stateMachineDefinition: 'smd1' },
      ],
      transitions: [{ id: 't1', from: 's1', to: 's2', eventType: 'et1' }],
      'event-types': [{ id: 'et1', name: 'ship' }],
      roles: [],
      readings: [],
    })

    const result = await generateXState(db, 'd1')
    const toolsFile = Object.entries(result.files).find(([k]) => k.includes('tools'))
    expect(toolsFile).toBeDefined()
    const tools = JSON.parse(toolsFile![1])
    expect(tools).toHaveLength(1)
    expect(tools[0].name).toBe('ship')
  })
})
```

### Step 2: Run, verify fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/xstate.test.ts`

### Step 3: Write implementation

Port `generateXStateFiles` from Generator.ts:1354-1558, replacing `payload.find()`/`payload.findByID()` with `db.findInCollection()`.

```typescript
// src/generate/xstate.ts

/**
 * generateXState — state machine configs, agent tools, system prompts.
 * Ported from Generator.ts:1354-1558.
 */

export async function generateXState(db: any, domainId: string): Promise<{ files: Record<string, string> }> {
  const domainFilter = { domain: { equals: domainId } }
  const smDefs = (await db.findInCollection('state-machine-definitions', domainFilter, { limit: 1000 })).docs
  const nouns = (await db.findInCollection('nouns', domainFilter, { limit: 1000 })).docs
  const files: Record<string, string> = {}

  for (const smDef of smDefs) {
    const statuses = (await db.findInCollection('statuses', {
      stateMachineDefinition: { equals: smDef.id },
    }, { sort: 'createdAt', limit: 1000 })).docs
    if (!statuses.length) continue

    // Collect all transitions for this machine
    const allTransitions: { from: string; to: string; event: string; callback?: { url: string; method: string } }[] = []
    for (const status of statuses) {
      const transitions = (await db.findInCollection('transitions', {
        from: { equals: status.id },
      }, { limit: 1000 })).docs

      for (const t of transitions) {
        const toStatus = statuses.find((s: any) => s.id === (t.to || t.toStatusId))
        const eventTypeId = t.eventType || t.eventTypeId
        let eventType: any = null
        if (eventTypeId) {
          const et = (await db.findInCollection('event-types', { id: eventTypeId }, { limit: 1 })).docs
          eventType = et[0] || null
          if (!eventType) {
            // Try direct lookup
            const allET = (await db.findInCollection('event-types', {}, { limit: 1000 })).docs
            eventType = allET.find((e: any) => e.id === eventTypeId)
          }
        }

        // Resolve verb → function for callback
        let callback: { url: string; method: string } | undefined
        const verbId = t.verb || t.verbId
        if (verbId) {
          const verbs = (await db.findInCollection('verbs', {}, { limit: 1000 })).docs
          const verb = verbs.find((v: any) => v.id === verbId)
          if (verb) {
            const funcId = verb.function || verb.functionId
            if (funcId) {
              const funcs = (await db.findInCollection('functions', {}, { limit: 1000 })).docs
              const func = funcs.find((f: any) => f.id === funcId)
              if (func?.callbackUrl) {
                callback = { url: func.callbackUrl, method: func.httpMethod || 'POST' }
              }
            }
          }
        }

        if (toStatus?.name && eventType?.name) {
          allTransitions.push({ from: status.name, to: toStatus.name, event: eventType.name, callback })
        }
      }
    }

    // Build XState config
    const states: Record<string, any> = {}
    for (const status of statuses) {
      const outgoing = allTransitions.filter(t => t.from === status.name)
      const on: Record<string, any> = {}
      for (const t of outgoing) {
        const transition: Record<string, any> = { target: t.to }
        if (t.callback) transition.meta = { callback: t.callback }
        on[t.event] = transition
      }
      states[status.name] = Object.keys(on).length ? { on } : {}
    }

    // Initial state: no incoming transitions, or first status
    const statesWithIncoming = new Set(allTransitions.map(t => t.to))
    const initialStatus = statuses.find((s: any) => !statesWithIncoming.has(s.name)) || statuses[0]

    // Resolve noun name for filename
    const nounId = smDef.noun || smDef.nounId
    const noun = nouns.find((n: any) => n.id === nounId)
    const machineName = (noun?.name || 'unknown')
      .replace(/([A-Z])/g, '-$1')
      .toLowerCase()
      .replace(/^-/, '')

    files[`state-machines/${machineName}.json`] = JSON.stringify({
      id: machineName,
      initial: initialStatus.name,
      states,
    }, null, 2)

    // Agent tools from unique events
    const uniqueEvents = new Map<string, { from: string[]; to: string[] }>()
    for (const t of allTransitions) {
      if (!t.event) continue
      if (!uniqueEvents.has(t.event)) uniqueEvents.set(t.event, { from: [], to: [] })
      const entry = uniqueEvents.get(t.event)!
      if (!entry.from.includes(t.from)) entry.from.push(t.from)
      if (!entry.to.includes(t.to)) entry.to.push(t.to)
    }

    const tools = Array.from(uniqueEvents.entries()).map(([event, { from, to }]) => ({
      name: event,
      description: `Transition from ${from.join(' or ')} to ${to.join(' or ')}`,
      parameters: { type: 'object' as const, properties: {} },
    }))
    files[`agents/${machineName}-tools.json`] = JSON.stringify(tools, null, 2)

    // Agent prompt from relevant readings
    const allRoles = (await db.findInCollection('roles', {}, { limit: 10000 })).docs
    const directSchemaIds = new Set<string>()
    const relatedNounIds = new Set<string>()
    relatedNounIds.add(nounId)

    for (const role of allRoles) {
      const roleNounId = role.noun || role.nounId
      const gsId = role.graphSchema || role.graphSchemaId
      if (roleNounId === nounId && gsId) directSchemaIds.add(gsId)
    }
    for (const role of allRoles) {
      const gsId = role.graphSchema || role.graphSchemaId
      if (directSchemaIds.has(gsId)) {
        const roleNounId = role.noun || role.nounId
        if (roleNounId) relatedNounIds.add(roleNounId)
      }
    }
    const expandedSchemaIds = new Set(directSchemaIds)
    for (const role of allRoles) {
      const roleNounId = role.noun || role.nounId
      const gsId = role.graphSchema || role.graphSchemaId
      if (relatedNounIds.has(roleNounId) && gsId) expandedSchemaIds.add(gsId)
    }

    const allReadings = (await db.findInCollection('readings', {}, { limit: 10000 })).docs
    const readings = allReadings.filter((r: any) => {
      const gsId = r.graphSchema || r.graphSchemaId
      return expandedSchemaIds.has(gsId)
    })

    const readingTexts = [...new Set(readings.map((r: any) => r.text).filter(Boolean))] as string[]
    const stateNames = statuses.map((s: any) => s.name)
    const eventNames = Array.from(uniqueEvents.keys())

    const prompt = [
      `# ${noun?.name || 'Agent'} Agent`,
      '',
      '## Domain Model',
      ...readingTexts.map((r: string) => `- ${r}`),
      '',
      '## State Machine',
      `States: ${stateNames.join(', ')}`,
      '',
      '## Available Actions',
      ...eventNames.map(e => {
        const { from, to } = uniqueEvents.get(e)!
        return `- **${e}**: ${from.join('/')} → ${to.join('/')}`
      }),
      '',
      '## Current State: {{currentState}}',
      '',
      '## Instructions',
      'You operate within the domain model above. Use the available actions to transition the state machine. Do not take actions outside the defined transitions for the current state.',
      '',
    ].join('\n')
    files[`agents/${machineName}-prompt.md`] = prompt
  }

  return { files }
}
```

### Step 4: Run tests

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/xstate.test.ts`
Expected: PASS

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/xstate.ts src/generate/xstate.test.ts
git commit -m "feat: port generateXState (state machines, agent tools, prompts)"
```

---

## Task 7: Port generateILayerFiles

Generates iLayer UI definitions from entity nouns, their value-type readings (→ fields), entity-to-entity readings (→ navigation), and state machine events (→ action buttons).

**Files:**
- Create: `src/generate/ilayer.ts`
- Create: `src/generate/ilayer.test.ts`

### Step 1: Write failing tests

```typescript
// src/generate/ilayer.test.ts
import { describe, it, expect } from 'vitest'
import { generateILayer } from './ilayer'

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any) => {
      const docs = data[slug] || []
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  } as any
}

describe('generateILayer', () => {
  it('generates list and detail layers for entity nouns', async () => {
    const db = mockDB({
      nouns: [
        { id: 'n1', name: 'Customer', objectType: 'entity', permissions: ['list', 'read', 'create'] },
        { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [{ id: 'rd1', text: 'Customer has Name', graphSchema: 'gs1', roles: ['r1', 'r2'] }],
      roles: [
        { id: 'r1', noun: { value: { id: 'n1', name: 'Customer' } }, graphSchema: 'gs1' },
        { id: 'r2', noun: { value: { id: 'n2', name: 'Name' } }, graphSchema: 'gs1' },
      ],
      'state-machine-definitions': [],
      statuses: [],
      transitions: [],
      'event-types': [],
    })

    const result = await generateILayer(db, 'd1')
    expect(result.files['layers/customers.json']).toBeDefined()
    expect(result.files['layers/customers-detail.json']).toBeDefined()
    expect(result.files['layers/index.json']).toBeDefined()
  })
})
```

### Step 2: Run, verify fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/ilayer.test.ts`

### Step 3: Write implementation

Port `generateILayerFiles` from Generator.ts:1559-1869, replacing `payload` calls with `db.findInCollection()`. The logic is largely the same — map value readings → fields, entity readings → navigation, state machine events → action buttons.

This file is large (~300 lines). The implementation follows the same structure as the original but with `db.findInCollection()` instead of `payload.find()`. The core helpers (`toCamelCase`, `toLabel`, `toSlug`, `mapFieldType`) port directly.

### Step 4: Run tests

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/ilayer.test.ts`
Expected: PASS

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/ilayer.ts src/generate/ilayer.test.ts
git commit -m "feat: port generateILayer (domain-driven UI layers)"
```

---

## Task 8: Port generateReadingsOutput (Round-Trip)

Generates FORML2 readings text from the database — the reverse of ingestion. Used for export/round-trip verification.

**Files:**
- Create: `src/generate/readings.ts`
- Create: `src/generate/readings.test.ts`

### Step 1: Write failing tests

```typescript
// src/generate/readings.test.ts
import { describe, it, expect } from 'vitest'
import { generateReadings } from './readings'

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string) => {
      const docs = data[slug] || []
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  } as any
}

describe('generateReadings', () => {
  it('outputs entity type declarations', async () => {
    const db = mockDB({
      nouns: [{ id: 'n1', name: 'Customer', objectType: 'entity' }],
      readings: [],
      'constraint-spans': [],
      'state-machine-definitions': [],
      statuses: [],
      transitions: [],
    })
    const result = await generateReadings(db, 'd1')
    expect(result.text).toContain('Customer')
    expect(result.text).toContain('Entity Types')
  })

  it('outputs reading texts', async () => {
    const db = mockDB({
      nouns: [
        { id: 'n1', name: 'Customer', objectType: 'entity' },
        { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [{ id: 'rd1', text: 'Customer has Name', graphSchema: 'gs1' }],
      'constraint-spans': [],
      'state-machine-definitions': [],
      statuses: [],
      transitions: [],
    })
    const result = await generateReadings(db, 'd1')
    expect(result.text).toContain('Customer has Name')
  })
})
```

### Step 2: Run, verify fail

### Step 3: Write implementation (port Generator.ts:189-295)

### Step 4: Run tests, verify pass

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/readings.ts src/generate/readings.test.ts
git commit -m "feat: port generateReadings (FORML2 round-trip export)"
```

---

## Task 9: Wire Generate API Route

Add `POST /api/generate` to the router. Accepts `{ domainId, outputFormat }` and dispatches to the appropriate generator.

**Files:**
- Create: `src/api/generate.ts`
- Modify: `src/api/router.ts` — add route

### Step 1: Write failing test

```typescript
// src/api/generate.test.ts — integration test shape
import { describe, it, expect } from 'vitest'
import { handleGenerate } from './generate'

describe('handleGenerate', () => {
  it('returns 400 for missing domainId', async () => {
    const request = new Request('http://localhost/api/generate', {
      method: 'POST',
      body: JSON.stringify({}),
      headers: { 'Content-Type': 'application/json' },
    })
    const response = await handleGenerate(request, {} as any)
    expect(response.status).toBe(400)
  })

  it('returns 400 for unknown outputFormat', async () => {
    const request = new Request('http://localhost/api/generate', {
      method: 'POST',
      body: JSON.stringify({ domainId: 'd1', outputFormat: 'unknown' }),
      headers: { 'Content-Type': 'application/json' },
    })
    const response = await handleGenerate(request, {} as any)
    expect(response.status).toBe(400)
  })
})
```

### Step 2: Run, verify fail

### Step 3: Write implementation

```typescript
// src/api/generate.ts
import { json, error } from 'itty-router'
import type { Env } from '../types'
import { generateOpenAPI } from '../generate/openapi'
import { generateSQLite } from '../generate/sqlite'
import { generateXState } from '../generate/xstate'
import { generateILayer } from '../generate/ilayer'
import { generateReadings } from '../generate/readings'

const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings']

export async function handleGenerate(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as Record<string, any>
  const { domainId, outputFormat = 'openapi' } = body

  if (!domainId) return error(400, { errors: [{ message: 'domainId is required' }] })
  if (!VALID_FORMATS.includes(outputFormat)) {
    return error(400, { errors: [{ message: `Invalid outputFormat. Valid: ${VALID_FORMATS.join(', ')}` }] })
  }

  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  const db = env.GRAPHDL_DB.get(id) as any

  let output: any
  switch (outputFormat) {
    case 'openapi':
      output = await generateOpenAPI(db, domainId)
      break
    case 'sqlite': {
      const openapi = await generateOpenAPI(db, domainId)
      output = generateSQLite(openapi)
      break
    }
    case 'xstate':
      output = await generateXState(db, domainId)
      break
    case 'ilayer':
      output = await generateILayer(db, domainId)
      break
    case 'readings':
      output = await generateReadings(db, domainId)
      break
  }

  return json({ output, format: outputFormat, domainId })
}
```

Then add to router.ts:

```typescript
// In src/api/router.ts — add before the 404 fallback:
import { handleGenerate } from './generate'
router.post('/api/generate', handleGenerate)
```

### Step 4: Run tests

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/api/generate.test.ts`

### Step 5: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/api/generate.ts src/api/generate.test.ts src/api/router.ts
git commit -m "feat: wire POST /api/generate route with all output formats"
```

---

## Task 10: Seed graphdl-core Readings & Verify Self-Hosting Bootstrap

Seed the existing `readings/*.md` files (core.md, state.md, instances.md, organizations.md, agents.md) into the graphdl-core domain, then run `generateSQLite` and verify the output matches (or improves upon) the hand-written DDL in `src/schema/*.ts`.

**Files:**
- Create: `src/generate/bootstrap.test.ts` (integration test)

### Step 1: Write the failing test

```typescript
// src/generate/bootstrap.test.ts
import { describe, it, expect } from 'vitest'
import { generateSQLite } from './sqlite'
import { generateOpenAPI } from './openapi'

describe('self-hosting bootstrap', () => {
  it('generateSQLite produces DDL for core metamodel entities', async () => {
    // This test uses a mock DB pre-loaded with core.md entities
    // to verify the generator can produce its own schema.
    // For now, verify the sqlite generator handles the expected entity shapes.
    const openapi = {
      components: {
        schemas: {
          UpdateNoun: {
            title: 'Noun',
            type: 'object',
            properties: {
              name: { type: 'string' },
              objectType: { type: 'string', enum: ['entity', 'value'] },
              plural: { type: 'string' },
              valueType: { type: 'string' },
              format: { type: 'string' },
              enumValues: { type: 'string' },
              promptText: { type: 'string' },
            },
          },
          NewNoun: { title: 'Noun', type: 'object' },
          Noun: { title: 'Noun', type: 'object' },
          UpdateReading: {
            title: 'Reading',
            type: 'object',
            properties: {
              text: { type: 'string' },
              graphSchema: { oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/GraphSchema' }] },
            },
          },
          NewReading: { title: 'Reading', type: 'object' },
          Reading: { title: 'Reading', type: 'object' },
          UpdateGraphSchema: { title: 'GraphSchema', type: 'object', properties: { name: { type: 'string' } } },
          NewGraphSchema: { title: 'GraphSchema', type: 'object' },
          GraphSchema: { title: 'GraphSchema', type: 'object' },
        },
      },
    }

    const result = generateSQLite(openapi)

    // Should generate tables for Noun, Reading, GraphSchema
    expect(result.ddl.some(d => d.includes('nouns'))).toBe(true)
    expect(result.ddl.some(d => d.includes('readings'))).toBe(true)
    expect(result.ddl.some(d => d.includes('graph_schemas'))).toBe(true)

    // Noun table should have the expected columns
    const nounDDL = result.ddl.find(d => d.includes('CREATE TABLE') && d.includes('nouns'))!
    expect(nounDDL).toContain('name TEXT')
    expect(nounDDL).toContain('object_type TEXT')
    expect(nounDDL).toContain('value_type TEXT')

    // Reading should FK to graph_schemas
    const readingDDL = result.ddl.find(d => d.includes('CREATE TABLE') && d.includes('readings'))!
    expect(readingDDL).toContain('graph_schema_id TEXT REFERENCES graph_schemas(id)')
  })
})
```

### Step 2: Run, verify fail

Run: `cd C:/Users/lippe/Repos/graphdl-orm && npx vitest run src/generate/bootstrap.test.ts`

### Step 3: Run test, verify pass (uses already-written generateSQLite)

The test uses the existing `generateSQLite` from Task 5 — no new implementation needed here, just the integration verification.

### Step 4: Commit

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/bootstrap.test.ts
git commit -m "test: verify self-hosting bootstrap — generated DDL matches core metamodel"
```

### Step 5: Create barrel export

```typescript
// src/generate/index.ts
export { generateOpenAPI } from './openapi'
export { generateSQLite } from './sqlite'
export { generateXState } from './xstate'
export { generateILayer } from './ilayer'
export { generateReadings } from './readings'

// Pure RMap functions (for direct use or testing)
export {
  nameToKey,
  transformPropertyName,
  extractPropertyName,
  toPredicate,
  findPredicateObject,
  nounListToRegex,
  type NounRef,
} from './rmap'
```

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add src/generate/index.ts
git commit -m "feat: add generate barrel export"
```

---

## Summary of File Structure After All Tasks

```
src/generate/
  index.ts           — barrel export
  rmap.ts            — pure functions (predicate parsing, naming)
  rmap.test.ts
  schema-builder.ts  — ensureTableExists, createProperty, setTableProperty
  schema-builder.test.ts
  fact-processors.ts — processBinarySchemas, processArraySchemas, processUnarySchemas
  fact-processors.test.ts
  openapi.ts         — generateOpenAPI (data fetching + orchestration)
  openapi.test.ts
  sqlite.ts          — generateSQLite (NEW — OpenAPI → CREATE TABLE DDL)
  sqlite.test.ts
  xstate.ts          — generateXState (state machines, agent tools, prompts)
  xstate.test.ts
  ilayer.ts          — generateILayer (UI layers from entity readings)
  ilayer.test.ts
  readings.ts        — generateReadings (FORML2 round-trip)
  readings.test.ts
  bootstrap.test.ts  — self-hosting verification

src/api/
  generate.ts        — POST /api/generate route handler
  generate.test.ts
  router.ts          — (modified) adds /api/generate route
```

## Dependency Graph

```
rmap.ts (pure)
  ↓
schema-builder.ts (pure, uses rmap)
  ↓
fact-processors.ts (pure, uses rmap + schema-builder)
  ↓
openapi.ts (data adapter, uses all above + GraphDLDB)
  ↓
├── sqlite.ts (new target — consumes OpenAPI output)
├── xstate.ts (data adapter, uses GraphDLDB)
├── ilayer.ts (data adapter, uses GraphDLDB)
└── readings.ts (data adapter, uses GraphDLDB)
  ↓
generate.ts (API route, dispatches to above)
```

## What Gets Replaced

Once this is working and verified:
1. `src/schema/metamodel.ts` — hand-written DDL → generated by `sqlite` format
2. `src/schema/state.ts` — hand-written DDL → generated
3. `src/schema/agents.ts` — hand-written DDL → generated
4. `src/schema/instances.ts` — hand-written DDL → generated
5. `src/collections.ts` — hand-written FIELD_MAP → generated alongside DDL
6. The `generators` collection (deleted in rewrite) → replaced by `POST /api/generate`

## What's NOT in Scope

- `generatePayloadFiles` — Payload CMS is no longer used, skip this format
- `generateMermaidDiagrams` — nice-to-have, not blocking anything, skip for now
- Removing the hand-written DDL files (do this AFTER verifying the generated output is correct)
