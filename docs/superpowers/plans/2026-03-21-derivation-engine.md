# Live Derivation Engine & Conceptual Query Language

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse derivation rules into structured IR, build a TypeScript forward-chainer that fires on entity writes, and add a conceptual query function that resolves natural language queries through reading paths.

**Architecture:** Derivation rules (`X := Y and Z`) are parsed into conjuncts of `(subject, predicate, object)` triples. On entity write, matching rules fire and produce derived facts. The forward-chainer loops until fixpoint (no new facts). Conceptual queries tokenize against known nouns, follow reading paths between them, and produce structured API filters.

**Tech Stack:** TypeScript, Vitest, existing `tokenizeReading` from `src/claims/tokenize.ts`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/derivation/parse-rule.ts` | Create | Parse `:=` text into structured `DerivationRule` IR (conjuncts, comparisons, aggregates) |
| `src/derivation/parse-rule.test.ts` | Create | Tests for rule parsing |
| `src/derivation/forward-chain.ts` | Create | TypeScript forward-chainer: evaluate rules against a population, produce derived facts, loop to fixpoint |
| `src/derivation/forward-chain.test.ts` | Create | Tests for forward chaining |
| `src/derivation/conceptual-query.ts` | Create | Resolve natural-language queries via reading paths to structured filters |
| `src/derivation/conceptual-query.test.ts` | Create | Tests for conceptual queries |
| `src/generate/schema.ts` | Modify | Wire parsed `:=` rules into `derivationRules` array |
| `src/api/parse.ts` | Modify | Store structured rule IR alongside text when parsing `:=` lines |

---

### Task 1: Parse Derivation Rules into Structured IR

**Files:**
- Create: `src/derivation/parse-rule.ts`
- Create: `src/derivation/parse-rule.test.ts`

- [ ] **Step 1: Write failing tests for join-chain rules**

```typescript
// src/derivation/parse-rule.test.ts
import { describe, it, expect } from 'vitest'
import { parseDerivationRule } from './parse-rule'

describe('parseDerivationRule', () => {
  it('parses a join-chain rule into conjuncts', () => {
    const nouns = [
      { name: 'Graph', id: 'n1' },
      { name: 'Layer', id: 'n2' },
      { name: 'Resource', id: 'n3' },
      { name: 'Role', id: 'n4' },
      { name: 'Reading', id: 'n5' },
      { name: 'Noun', id: 'n6' },
    ]
    const rule = parseDerivationRule(
      'Graph stimulates Layer := Graph uses Resource for Role and Role belongs to Reading and Reading references Noun and Layer owns Noun.',
      nouns,
    )
    expect(rule.consequent).toEqual({
      subject: 'Graph',
      predicate: 'stimulates',
      object: 'Layer',
    })
    expect(rule.antecedents).toHaveLength(4)
    expect(rule.antecedents[0]).toEqual({
      subject: 'Graph',
      predicate: 'uses',
      object: 'Resource',
      qualifier: { predicate: 'for', object: 'Role' },
    })
    expect(rule.kind).toBe('join')
  })

  it('parses a value-comparison rule', () => {
    const nouns = [
      { name: 'Layer State', id: 'n1' },
      { name: 'Valence', id: 'n2' },
      { name: 'Arousal', id: 'n3' },
      { name: 'Affect Region', id: 'n4' },
    ]
    const rule = parseDerivationRule(
      "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3.",
      nouns,
    )
    expect(rule.consequent.subject).toBe('Layer State')
    expect(rule.consequent.predicate).toBe('has')
    expect(rule.consequent.object).toBe('Affect Region')
    expect(rule.consequent.literalValue).toBe('Excited')
    expect(rule.kind).toBe('comparison')
    expect(rule.antecedents).toHaveLength(2)
    expect(rule.antecedents[0].comparison).toEqual({ op: '>', value: 0.3 })
  })

  it('parses domain visibility self-reference rule', () => {
    const nouns = [{ name: 'Domain', id: 'n1' }]
    const rule = parseDerivationRule(
      'Domain is visible to Domain := that Domain is the same Domain.',
      nouns,
    )
    expect(rule.consequent.subject).toBe('Domain')
    expect(rule.consequent.predicate).toBe('is visible to')
    expect(rule.consequent.object).toBe('Domain')
    expect(rule.kind).toBe('identity')
  })

  it('parses aggregate rules as kind "aggregate"', () => {
    const nouns = [
      { name: 'Graph Schema', id: 'n1' },
      { name: 'Arity', id: 'n2' },
      { name: 'Role', id: 'n3' },
    ]
    const rule = parseDerivationRule(
      'Graph Schema has Arity := count of Role where Graph Schema has Role.',
      nouns,
    )
    expect(rule.kind).toBe('aggregate')
    expect(rule.aggregate).toEqual({ fn: 'count', noun: 'Role' })
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/derivation/parse-rule.test.ts`
Expected: FAIL — module not found

- [ ] **Step 3: Implement `parseDerivationRule`**

```typescript
// src/derivation/parse-rule.ts
import { tokenizeReading, type NounRef } from '../claims/tokenize'

export interface RuleTriple {
  subject: string
  predicate: string
  object: string
  /** Ternary qualifier: "uses Resource *for* Role" */
  qualifier?: { predicate: string; object: string }
  /** Value comparison on the object: "> 0.3" */
  comparison?: { op: '>' | '<' | '>=' | '<=' | '=' | '!='; value: number }
  /** Literal value in consequent: "has Affect Region 'Excited'" */
  literalValue?: string
}

export interface DerivationRule {
  /** Full original text */
  text: string
  /** The derived fact (left side of :=) */
  consequent: RuleTriple
  /** Conditions (right side of :=, split on "and") */
  antecedents: RuleTriple[]
  /** Rule classification */
  kind: 'join' | 'comparison' | 'aggregate' | 'identity'
  /** For aggregate rules */
  aggregate?: { fn: string; noun: string }
}

/**
 * Parse a derivation rule text into structured IR.
 *
 * Format: "Consequent := Antecedent1 and Antecedent2 and ..."
 */
export function parseDerivationRule(
  text: string,
  nouns: Array<{ name: string; id: string }>,
): DerivationRule {
  const cleaned = text.replace(/\.$/, '').trim()
  const parts = cleaned.split(/\s*:=\s*/)
  if (parts.length !== 2) {
    return { text, consequent: { subject: '', predicate: '', object: '' }, antecedents: [], kind: 'join' }
  }

  const [lhs, rhs] = parts
  const consequent = parseTriple(lhs, nouns)

  // Identity rules: "that Domain is the same Domain"
  if (rhs.includes('the same') || rhs.includes('is the same')) {
    return { text, consequent, antecedents: [], kind: 'identity' }
  }

  // Aggregate rules: "count of X where ..."
  const aggMatch = rhs.match(/^(count|sum|avg|min|max)\s+of\s+(.+?)(?:\s+where\s+(.+))?$/i)
  if (aggMatch) {
    const aggNoun = findNounInText(aggMatch[2], nouns)
    const whereClause = aggMatch[3]
    const antecedents = whereClause ? splitConjuncts(whereClause).map(c => parseTriple(c, nouns)) : []
    return {
      text, consequent, antecedents, kind: 'aggregate',
      aggregate: { fn: aggMatch[1].toLowerCase(), noun: aggNoun || aggMatch[2].trim() },
    }
  }

  // Join/comparison rules: split on " and "
  const conjuncts = splitConjuncts(rhs)
  const antecedents = conjuncts.map(c => parseTriple(c, nouns))

  const hasComparisons = antecedents.some(a => a.comparison)
  const kind = hasComparisons ? 'comparison' : 'join'

  return { text, consequent, antecedents, kind }
}

/** Split RHS on " and " but not inside quoted strings */
function splitConjuncts(text: string): string[] {
  return text.split(/\s+and\s+/).map(s => s.trim()).filter(Boolean)
}

/** Parse a single clause like "Graph uses Resource for Role" into a triple */
function parseTriple(
  text: string,
  nouns: Array<{ name: string; id: string }>,
): RuleTriple {
  const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)

  // Extract literal value: 'Excited' etc.
  let literalValue: string | undefined
  const litMatch = text.match(/'([^']+)'/)
  if (litMatch) {
    literalValue = litMatch[1]
    text = text.replace(/'[^']+'\s*/, '').trim()
  }

  // Extract comparison: > 0.3, < -0.5, etc.
  let comparison: RuleTriple['comparison']
  const cmpMatch = text.match(/(>=|<=|!=|>|<|=)\s*(-?[\d.]+)\s*$/)
  if (cmpMatch) {
    comparison = { op: cmpMatch[1] as any, value: parseFloat(cmpMatch[2]) }
    text = text.slice(0, text.length - cmpMatch[0].length).trim()
  }

  // Find nouns in the clause text (longest-first)
  const found: Array<{ name: string; index: number }> = []
  let remaining = text
  for (const noun of sorted) {
    const idx = remaining.indexOf(noun.name)
    if (idx !== -1) {
      found.push({ name: noun.name, index: idx })
      // Replace to avoid re-matching substrings
      remaining = remaining.slice(0, idx) + '\0'.repeat(noun.name.length) + remaining.slice(idx + noun.name.length)
    }
  }
  found.sort((a, b) => a.index - b.index)

  if (found.length === 0) {
    return { subject: '', predicate: text, object: '', literalValue, comparison }
  }

  const subject = found[0].name
  if (found.length === 1) {
    const afterSubject = text.slice(text.indexOf(subject) + subject.length).trim()
    return { subject, predicate: afterSubject, object: '', literalValue, comparison }
  }

  const object = found[1].name
  const subjectEnd = text.indexOf(subject) + subject.length
  const objectStart = text.indexOf(object, subjectEnd)
  const predicate = text.slice(subjectEnd, objectStart).trim()

  // Check for qualifier (third noun): "uses Resource *for* Role"
  let qualifier: RuleTriple['qualifier']
  if (found.length >= 3) {
    const qualNoun = found[2].name
    const objectEnd = objectStart + object.length
    const qualStart = text.indexOf(qualNoun, objectEnd)
    if (qualStart > objectEnd) {
      const qualPred = text.slice(objectEnd, qualStart).trim()
      qualifier = { predicate: qualPred, object: qualNoun }
    }
  }

  return { subject, predicate, object, qualifier, literalValue, comparison }
}

function findNounInText(text: string, nouns: Array<{ name: string; id: string }>): string | undefined {
  const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)
  for (const noun of sorted) {
    if (text.includes(noun.name)) return noun.name
  }
  return undefined
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/derivation/parse-rule.test.ts`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/derivation/parse-rule.ts src/derivation/parse-rule.test.ts
git commit -m "feat: parse derivation rules (:=) into structured IR"
```

---

### Task 2: Forward Chainer

**Files:**
- Create: `src/derivation/forward-chain.ts`
- Create: `src/derivation/forward-chain.test.ts`

- [ ] **Step 1: Write failing tests**

```typescript
// src/derivation/forward-chain.test.ts
import { describe, it, expect } from 'vitest'
import { forwardChain, type FactStore } from './forward-chain'
import type { DerivationRule } from './parse-rule'

describe('forwardChain', () => {
  it('derives a fact from a join-chain rule', () => {
    const rules: DerivationRule[] = [{
      text: 'A relates to C := A has B and B has C.',
      consequent: { subject: 'A', predicate: 'relates to', object: 'C' },
      antecedents: [
        { subject: 'A', predicate: 'has', object: 'B' },
        { subject: 'B', predicate: 'has', object: 'C' },
      ],
      kind: 'join',
    }]

    const store: FactStore = {
      facts: [
        { subject: 'a1', subjectType: 'A', predicate: 'has', object: 'b1', objectType: 'B' },
        { subject: 'b1', subjectType: 'B', predicate: 'has', object: 'c1', objectType: 'C' },
      ],
    }

    const derived = forwardChain(rules, store)
    expect(derived).toHaveLength(1)
    expect(derived[0]).toMatchObject({
      subject: 'a1', subjectType: 'A',
      predicate: 'relates to',
      object: 'c1', objectType: 'C',
      derived: true,
    })
  })

  it('derives value-comparison facts', () => {
    const rules: DerivationRule[] = [{
      text: "X has Label 'High' := X has Score > 0.5.",
      consequent: { subject: 'X', predicate: 'has', object: 'Label', literalValue: 'High' },
      antecedents: [
        { subject: 'X', predicate: 'has', object: 'Score', comparison: { op: '>', value: 0.5 } },
      ],
      kind: 'comparison',
    }]

    const store: FactStore = {
      facts: [
        { subject: 'x1', subjectType: 'X', predicate: 'has', object: '0.8', objectType: 'Score' },
        { subject: 'x2', subjectType: 'X', predicate: 'has', object: '0.3', objectType: 'Score' },
      ],
    }

    const derived = forwardChain(rules, store)
    expect(derived).toHaveLength(1)
    expect(derived[0].subject).toBe('x1')
    expect(derived[0].object).toBe('High')
  })

  it('reaches fixpoint and stops', () => {
    const rules: DerivationRule[] = [{
      text: 'A relates to C := A has B and B has C.',
      consequent: { subject: 'A', predicate: 'relates to', object: 'C' },
      antecedents: [
        { subject: 'A', predicate: 'has', object: 'B' },
        { subject: 'B', predicate: 'has', object: 'C' },
      ],
      kind: 'join',
    }]

    const store: FactStore = {
      facts: [
        { subject: 'a1', subjectType: 'A', predicate: 'has', object: 'b1', objectType: 'B' },
        { subject: 'b1', subjectType: 'B', predicate: 'has', object: 'c1', objectType: 'C' },
      ],
    }

    // Run twice — second run should produce no new facts
    const derived1 = forwardChain(rules, store)
    expect(derived1).toHaveLength(1)

    store.facts.push(...derived1)
    const derived2 = forwardChain(rules, store)
    expect(derived2).toHaveLength(0) // fixpoint
  })

  it('handles identity rules', () => {
    const rules: DerivationRule[] = [{
      text: 'X sees X := same X.',
      consequent: { subject: 'X', predicate: 'sees', object: 'X' },
      antecedents: [],
      kind: 'identity',
    }]

    const store: FactStore = {
      facts: [],
      entities: { X: ['x1', 'x2'] },
    }

    const derived = forwardChain(rules, store)
    expect(derived).toHaveLength(2)
    expect(derived[0]).toMatchObject({ subject: 'x1', object: 'x1' })
    expect(derived[1]).toMatchObject({ subject: 'x2', object: 'x2' })
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/derivation/forward-chain.test.ts`
Expected: FAIL — module not found

- [ ] **Step 3: Implement forward chainer**

```typescript
// src/derivation/forward-chain.ts
import type { DerivationRule, RuleTriple } from './parse-rule'

export interface Fact {
  subject: string
  subjectType: string
  predicate: string
  object: string
  objectType: string
  derived?: boolean
}

export interface FactStore {
  facts: Fact[]
  /** Known entity IDs by type (for identity rules) */
  entities?: Record<string, string[]>
}

/**
 * Run one pass of forward chaining over all rules.
 * Returns only NEW facts not already in the store.
 * Call repeatedly until result is empty (fixpoint).
 */
export function forwardChain(rules: DerivationRule[], store: FactStore): Fact[] {
  const newFacts: Fact[] = []
  const existing = new Set(store.facts.map(factKey))

  for (const rule of rules) {
    const derived = evaluateRule(rule, store)
    for (const fact of derived) {
      const key = factKey(fact)
      if (!existing.has(key)) {
        existing.add(key)
        newFacts.push(fact)
      }
    }
  }

  return newFacts
}

/**
 * Run forward chaining to fixpoint (max iterations for safety).
 */
export function forwardChainToFixpoint(
  rules: DerivationRule[],
  store: FactStore,
  maxIterations: number = 10,
): Fact[] {
  const allDerived: Fact[] = []

  for (let i = 0; i < maxIterations; i++) {
    const newFacts = forwardChain(rules, store)
    if (newFacts.length === 0) break
    allDerived.push(...newFacts)
    store.facts.push(...newFacts)
  }

  return allDerived
}

function factKey(f: Fact): string {
  return `${f.subject}|${f.predicate}|${f.object}`
}

function evaluateRule(rule: DerivationRule, store: FactStore): Fact[] {
  switch (rule.kind) {
    case 'identity': return evaluateIdentity(rule, store)
    case 'comparison': return evaluateComparison(rule, store)
    case 'join': return evaluateJoin(rule, store)
    case 'aggregate': return evaluateAggregate(rule, store)
    default: return []
  }
}

/** Identity: for each entity of type X, derive (x, pred, x) */
function evaluateIdentity(rule: DerivationRule, store: FactStore): Fact[] {
  const typeName = rule.consequent.subject
  const ids = store.entities?.[typeName] || []
  return ids.map(id => ({
    subject: id,
    subjectType: typeName,
    predicate: rule.consequent.predicate,
    object: id,
    objectType: rule.consequent.object,
    derived: true,
  }))
}

/** Comparison: filter facts by numeric comparison, derive consequent for matches */
function evaluateComparison(rule: DerivationRule, store: FactStore): Fact[] {
  const results: Fact[] = []

  // Find all subjects that satisfy ALL antecedent comparisons
  const subjectType = rule.antecedents[0]?.subject
  if (!subjectType) return []

  // Group facts by subject instance
  const bySubject = new Map<string, Fact[]>()
  for (const fact of store.facts) {
    if (fact.subjectType === subjectType) {
      const arr = bySubject.get(fact.subject) || []
      arr.push(fact)
      bySubject.set(fact.subject, arr)
    }
  }

  for (const [subjectId, subjectFacts] of bySubject) {
    const allMatch = rule.antecedents.every(ant => {
      if (!ant.comparison) return true
      const matchingFact = subjectFacts.find(f => f.predicate === ant.predicate && f.objectType === ant.object)
      if (!matchingFact) return false
      const numValue = parseFloat(matchingFact.object)
      if (isNaN(numValue)) return false
      return compareValues(numValue, ant.comparison.op, ant.comparison.value)
    })

    if (allMatch) {
      results.push({
        subject: subjectId,
        subjectType: rule.consequent.subject,
        predicate: rule.consequent.predicate,
        object: rule.consequent.literalValue || '',
        objectType: rule.consequent.object,
        derived: true,
      })
    }
  }

  return results
}

/** Join: unify variables across conjuncts via shared noun bindings */
function evaluateJoin(rule: DerivationRule, store: FactStore): Fact[] {
  // Build variable bindings: each antecedent noun position is a variable.
  // Two antecedents sharing the same noun name share the same variable.
  // Start with all possible bindings for the first antecedent, then filter.

  if (rule.antecedents.length === 0) return []

  // Start with bindings from first conjunct
  let bindings = matchFacts(rule.antecedents[0], store.facts)

  // Iteratively join with subsequent conjuncts
  for (let i = 1; i < rule.antecedents.length; i++) {
    const nextMatches = matchFacts(rule.antecedents[i], store.facts)
    bindings = joinBindings(bindings, nextMatches)
  }

  // Produce consequent facts from surviving bindings
  const results: Fact[] = []
  for (const binding of bindings) {
    const subjectId = binding.get(rule.consequent.subject)
    const objectId = binding.get(rule.consequent.object)
    if (subjectId && objectId) {
      results.push({
        subject: subjectId,
        subjectType: rule.consequent.subject,
        predicate: rule.consequent.predicate,
        object: objectId,
        objectType: rule.consequent.object,
        derived: true,
      })
    }
  }

  return results
}

/** Match a single antecedent triple against the fact store, return variable bindings */
function matchFacts(ant: RuleTriple, facts: Fact[]): Map<string, string>[] {
  const results: Map<string, string>[] = []
  for (const fact of facts) {
    if (fact.predicate !== ant.predicate) continue
    const binding = new Map<string, string>()
    binding.set(ant.subject, fact.subject)
    binding.set(ant.object, fact.object)
    if (ant.qualifier) {
      // For ternary: the qualifier object is bound too — need to find it in fact metadata
      // For now, treat qualifier as part of predicate matching
    }
    results.push(binding)
  }
  return results
}

/** Join two sets of bindings on shared variables */
function joinBindings(
  left: Map<string, string>[],
  right: Map<string, string>[],
): Map<string, string>[] {
  const results: Map<string, string>[] = []

  for (const lb of left) {
    for (const rb of right) {
      // Check if shared variables are consistent
      let consistent = true
      for (const [key, val] of rb) {
        if (lb.has(key) && lb.get(key) !== val) {
          consistent = false
          break
        }
      }
      if (consistent) {
        const merged = new Map(lb)
        for (const [key, val] of rb) merged.set(key, val)
        results.push(merged)
      }
    }
  }

  return results
}

/** Aggregate: count/sum entities matching a condition */
function evaluateAggregate(rule: DerivationRule, store: FactStore): Fact[] {
  if (!rule.aggregate) return []

  // Simple count: find entities matching the where clause
  const matchingIds = new Set<string>()
  for (const fact of store.facts) {
    if (fact.subjectType === rule.aggregate.noun || fact.objectType === rule.aggregate.noun) {
      matchingIds.add(fact.subjectType === rule.aggregate.noun ? fact.subject : fact.object)
    }
  }

  // For now, return a single aggregate fact
  const subjectType = rule.consequent.subject
  const subjects = store.entities?.[subjectType] || []

  return subjects.map(id => ({
    subject: id,
    subjectType,
    predicate: rule.consequent.predicate,
    object: String(matchingIds.size),
    objectType: rule.consequent.object,
    derived: true,
  }))
}

function compareValues(actual: number, op: string, threshold: number): boolean {
  switch (op) {
    case '>': return actual > threshold
    case '<': return actual < threshold
    case '>=': return actual >= threshold
    case '<=': return actual <= threshold
    case '=': return actual === threshold
    case '!=': return actual !== threshold
    default: return false
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/derivation/forward-chain.test.ts`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/derivation/forward-chain.ts src/derivation/forward-chain.test.ts
git commit -m "feat: TypeScript forward chainer with fixpoint loop"
```

---

### Task 3: Conceptual Query Language

**Files:**
- Create: `src/derivation/conceptual-query.ts`
- Create: `src/derivation/conceptual-query.test.ts`

- [ ] **Step 1: Write failing tests**

```typescript
// src/derivation/conceptual-query.test.ts
import { describe, it, expect } from 'vitest'
import { resolveConceptualQuery } from './conceptual-query'

describe('resolveConceptualQuery', () => {
  const nouns = [
    { name: 'Customer', id: 'n1' },
    { name: 'Support Request', id: 'n2' },
    { name: 'Priority', id: 'n3' },
    { name: 'Name', id: 'n4' },
  ]

  const readings = [
    { text: 'Customer submits Support Request', nouns: ['Customer', 'Support Request'], predicate: 'submits' },
    { text: 'Support Request has Priority', nouns: ['Support Request', 'Priority'], predicate: 'has' },
    { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
  ]

  it('resolves a single-hop query', () => {
    const result = resolveConceptualQuery('Customer that has Name', nouns, readings)
    expect(result.path).toHaveLength(1)
    expect(result.path[0]).toMatchObject({
      from: 'Customer',
      predicate: 'has',
      to: 'Name',
    })
  })

  it('resolves a multi-hop query with value filter', () => {
    const result = resolveConceptualQuery(
      "Customer that submits Support Request that has Priority 'High'",
      nouns, readings,
    )
    expect(result.path).toHaveLength(2)
    expect(result.path[0]).toMatchObject({ from: 'Customer', to: 'Support Request' })
    expect(result.path[1]).toMatchObject({ from: 'Support Request', to: 'Priority' })
    expect(result.filters).toContainEqual({ field: 'Priority', value: 'High' })
  })

  it('returns empty path for unrecognized query', () => {
    const result = resolveConceptualQuery('Foo that bars Baz', nouns, readings)
    expect(result.path).toHaveLength(0)
  })
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/derivation/conceptual-query.test.ts`
Expected: FAIL — module not found

- [ ] **Step 3: Implement conceptual query resolver**

```typescript
// src/derivation/conceptual-query.ts

export interface QueryPathStep {
  from: string
  predicate: string
  to: string
}

export interface ConceptualQueryResult {
  /** The reading path from root entity to target */
  path: QueryPathStep[]
  /** Value filters extracted from the query */
  filters: Array<{ field: string; value: string }>
  /** The root entity type */
  rootNoun?: string
}

interface ReadingDef {
  text: string
  nouns: string[]
  predicate: string
}

/**
 * Resolve a natural-language query into a structured path through readings.
 *
 * Query format: "Customer that submits Support Request that has Priority 'High'"
 * Segments split on "that" — each segment names a noun and a predicate.
 */
export function resolveConceptualQuery(
  query: string,
  nouns: Array<{ name: string; id: string }>,
  readings: ReadingDef[],
): ConceptualQueryResult {
  const filters: ConceptualQueryResult['filters'] = []

  // Extract quoted literal values
  const cleaned = query.replace(/'([^']+)'/g, (_, val) => {
    filters.push({ field: '', value: val })
    return ''
  }).trim()

  // Split on "that" to get segments
  const segments = cleaned.split(/\s+that\s+/i).map(s => s.trim()).filter(Boolean)
  if (segments.length === 0) return { path: [], filters }

  // Find nouns in each segment (longest-first matching)
  const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)
  const path: QueryPathStep[] = []

  let previousNoun: string | undefined

  for (const segment of segments) {
    const foundNouns: string[] = []
    let remaining = segment
    for (const noun of sorted) {
      if (remaining.includes(noun.name)) {
        foundNouns.push(noun.name)
        remaining = remaining.replace(noun.name, '\0'.repeat(noun.name.length))
      }
    }

    if (foundNouns.length === 0) continue

    if (!previousNoun) {
      // First segment establishes the root noun
      previousNoun = foundNouns[0]
      if (foundNouns.length >= 2) {
        // "Customer that has Name" — first segment has both nouns
        const reading = findReading(foundNouns[0], foundNouns[1], readings)
        if (reading) {
          path.push({ from: foundNouns[0], predicate: reading.predicate, to: foundNouns[1] })
          previousNoun = foundNouns[1]
        }
      }
      continue
    }

    // Subsequent segments: find reading from previousNoun to any noun in this segment
    for (const noun of foundNouns) {
      const reading = findReading(previousNoun, noun, readings)
      if (reading) {
        path.push({ from: previousNoun, predicate: reading.predicate, to: noun })
        previousNoun = noun
        break
      }
    }
  }

  // Assign extracted filters to the last path step's target noun
  for (const f of filters) {
    if (path.length > 0 && !f.field) {
      f.field = path[path.length - 1].to
    }
  }

  return {
    path,
    filters,
    rootNoun: segments.length > 0 ? path[0]?.from : undefined,
  }
}

function findReading(from: string, to: string, readings: ReadingDef[]): ReadingDef | undefined {
  return readings.find(r =>
    r.nouns.includes(from) && r.nouns.includes(to)
  )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/derivation/conceptual-query.test.ts`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add src/derivation/conceptual-query.ts src/derivation/conceptual-query.test.ts
git commit -m "feat: conceptual query language — resolve natural language to reading paths"
```

---

### Task 4: Wire Derivation Rule Parsing into the Parser

**Files:**
- Modify: `src/api/parse.ts:297-304`
- Modify: `src/claims/ingest.ts` (add `derivationRules` to `ExtractedClaims`)

- [ ] **Step 1: Update the parser to produce structured rule IR alongside text**

In `src/api/parse.ts`, at the derivation rule handling (line ~297), import `parseDerivationRule` and store the structured rule:

```typescript
// At top of parse.ts, add import:
import { parseDerivationRule } from '../derivation/parse-rule'

// In the derivation rule handler (line ~297), change:
//   readings.push({ text: line, nouns: [], predicate: ':=', derivation: m[2].trim() })
// to:
    const nounList = [...nounMap.values()].map((n, i) => ({ name: n.name, id: `n${i}` }))
    const ruleIR = parseDerivationRule(line, nounList)
    readings.push({
      text: line,
      nouns: ruleIR.consequent.subject && ruleIR.consequent.object
        ? [ruleIR.consequent.subject, ruleIR.consequent.object].filter(Boolean)
        : [],
      predicate: ':=',
      derivation: m[2].trim(),
      ruleIR,
    })
```

- [ ] **Step 2: Run existing parser tests to verify nothing breaks**

Run: `npx vitest run src/api/parse.test.ts`
Expected: PASS (28 tests)

- [ ] **Step 3: Run full test suite**

Run: `npx vitest run`
Expected: All 608+ tests pass

- [ ] **Step 4: Commit**

```bash
git add src/api/parse.ts
git commit -m "feat: parse derivation rules into structured IR during FORML2 parsing"
```

---

### Task 5: Integration Test with spd-1 Affect Rules

**Files:**
- Create: `src/derivation/integration.test.ts`

- [ ] **Step 1: Write integration test parsing real spd-1 derivation rules**

```typescript
// src/derivation/integration.test.ts
import { describe, it, expect } from 'vitest'
import { parseDerivationRule } from './parse-rule'
import { forwardChain, type FactStore } from './forward-chain'

describe('spd-1 affect integration', () => {
  const nouns = [
    { name: 'Layer State', id: 'n1' },
    { name: 'Valence', id: 'n2' },
    { name: 'Arousal', id: 'n3' },
    { name: 'Affect Region', id: 'n4' },
    { name: 'Graph', id: 'n5' },
    { name: 'Layer', id: 'n6' },
    { name: 'Resource', id: 'n7' },
    { name: 'Role', id: 'n8' },
    { name: 'Reading', id: 'n9' },
    { name: 'Noun', id: 'n10' },
  ]

  it('parses the stimulus routing rule', () => {
    const rule = parseDerivationRule(
      'Graph stimulates Layer := Graph uses Resource for Role and Role belongs to Reading and Reading references Noun and Layer owns Noun.',
      nouns,
    )
    expect(rule.kind).toBe('join')
    expect(rule.antecedents).toHaveLength(4)
    expect(rule.consequent.subject).toBe('Graph')
    expect(rule.consequent.object).toBe('Layer')
  })

  it('derives Affect Region from Valence and Arousal', () => {
    const rule = parseDerivationRule(
      "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3.",
      nouns,
    )

    const store: FactStore = {
      facts: [
        { subject: 'ls1', subjectType: 'Layer State', predicate: 'has', object: '0.7', objectType: 'Valence' },
        { subject: 'ls1', subjectType: 'Layer State', predicate: 'has', object: '0.8', objectType: 'Arousal' },
        { subject: 'ls2', subjectType: 'Layer State', predicate: 'has', object: '-0.5', objectType: 'Valence' },
        { subject: 'ls2', subjectType: 'Layer State', predicate: 'has', object: '0.8', objectType: 'Arousal' },
      ],
    }

    const derived = forwardChain([rule], store)
    // Only ls1 matches (Valence > 0.3 AND Arousal > 0.3)
    expect(derived).toHaveLength(1)
    expect(derived[0].subject).toBe('ls1')
    expect(derived[0].object).toBe('Excited')
    expect(derived[0].derived).toBe(true)
  })

  it('derives multiple affect regions for same entity', () => {
    const rules = [
      "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3.",
      "Layer State has Affect Region 'Calm' := Layer State has Valence > 0.3 and Layer State has Arousal < -0.3.",
    ].map(text => parseDerivationRule(text, nouns))

    const store: FactStore = {
      facts: [
        { subject: 'ls1', subjectType: 'Layer State', predicate: 'has', object: '0.7', objectType: 'Valence' },
        { subject: 'ls1', subjectType: 'Layer State', predicate: 'has', object: '0.8', objectType: 'Arousal' },
        { subject: 'ls2', subjectType: 'Layer State', predicate: 'has', object: '0.5', objectType: 'Valence' },
        { subject: 'ls2', subjectType: 'Layer State', predicate: 'has', object: '-0.6', objectType: 'Arousal' },
      ],
    }

    const derived = forwardChain(rules, store)
    expect(derived).toHaveLength(2)
    expect(derived.find(d => d.subject === 'ls1')?.object).toBe('Excited')
    expect(derived.find(d => d.subject === 'ls2')?.object).toBe('Calm')
  })
})
```

- [ ] **Step 2: Run integration tests**

Run: `npx vitest run src/derivation/integration.test.ts`
Expected: PASS (3 tests)

- [ ] **Step 3: Commit**

```bash
git add src/derivation/integration.test.ts
git commit -m "test: spd-1 affect derivation rules integration test"
```

---

### Task 6: Full Suite Verification

- [ ] **Step 1: Run complete test suite**

Run: `npx vitest run`
Expected: All tests pass (608+ existing + ~14 new)

- [ ] **Step 2: Commit all remaining changes**

```bash
git add -A
git commit -m "feat: live derivation engine and conceptual query language"
```
