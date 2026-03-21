/**
 * Forward-chaining derivation engine.
 *
 * Evaluates derivation rules against a fact store, produces derived facts,
 * and loops to fixpoint.
 */

import type { DerivationRule, RuleTriple } from './parse-rule'

// ─── Types ──────────────────────────────────────────────────────────

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
  entities?: Record<string, string[]>  // known entity IDs by type (for identity rules)
}

// ─── Helpers ────────────────────────────────────────────────────────

type Bindings = Map<string, string>

/** Canonical key for deduplication. */
function factKey(f: Fact): string {
  return `${f.subjectType}:${f.subject}|${f.predicate}|${f.objectType}:${f.object}`
}

/** Check whether a fact already exists in the set of known keys. */
function isKnown(f: Fact, known: Set<string>): boolean {
  return known.has(factKey(f))
}

/** Try to merge two consistent binding maps. Returns null if conflict. */
function mergeBindings(a: Bindings, b: Bindings): Bindings | null {
  const merged = new Map(a)
  for (const [k, v] of b) {
    const existing = merged.get(k)
    if (existing !== undefined && existing !== v) return null
    merged.set(k, v)
  }
  return merged
}

/**
 * Match a single fact against a rule antecedent triple.
 * Returns bindings if it matches, null otherwise.
 *
 * Critical: filters by subjectType AND objectType to prevent cross-type matching.
 */
function matchFact(fact: Fact, ant: RuleTriple): Bindings | null {
  // Type check: fact's subjectType must equal the antecedent's subject noun
  if (fact.subjectType !== ant.subject) return null

  // Predicate must match literally
  if (fact.predicate !== ant.predicate) return null

  // Type check: fact's objectType must equal the antecedent's object noun (if specified)
  if (ant.object && fact.objectType !== ant.object) return null

  // Comparison check
  if (ant.comparison) {
    const numValue = parseFloat(fact.object)
    if (isNaN(numValue)) return null
    if (!evalComparison(numValue, ant.comparison.op, ant.comparison.value)) return null
  }

  // Build variable bindings: type-name → instance-id
  const bindings: Bindings = new Map()
  bindings.set(ant.subject, fact.subject)
  if (ant.object) {
    bindings.set(ant.object, fact.object)
  }

  return bindings
}

function evalComparison(actual: number, op: string, threshold: number): boolean {
  switch (op) {
    case '>':  return actual > threshold
    case '<':  return actual < threshold
    case '>=': return actual >= threshold
    case '<=': return actual <= threshold
    case '=':  return actual === threshold
    case '!=': return actual !== threshold
    default:   return false
  }
}

/** Produce a derived Fact from the consequent triple and a set of bindings. */
function produceConsequent(cons: RuleTriple, bindings: Bindings): Fact {
  const subject = bindings.get(cons.subject) ?? cons.subject
  const object = cons.literalValue ?? (bindings.get(cons.object) ?? cons.object)
  return {
    subject,
    subjectType: cons.subject,
    predicate: cons.predicate,
    object,
    objectType: cons.object,
    derived: true,
  }
}

// ─── Rule Evaluators ────────────────────────────────────────────────

/**
 * Identity: for each entity of the consequent's subject type, derive (x, pred, x).
 * No antecedents needed — uses entities registry.
 */
function evaluateIdentity(rule: DerivationRule, store: FactStore): Fact[] {
  const typeName = rule.consequent.subject
  const ids = store.entities?.[typeName] ?? []
  return ids.map((id) => ({
    subject: id,
    subjectType: typeName,
    predicate: rule.consequent.predicate,
    object: id,
    objectType: rule.consequent.object,
    derived: true,
  }))
}

/**
 * Comparison: group facts by subject, check all numeric comparisons match,
 * derive consequent for matching subjects.
 */
function evaluateComparison(rule: DerivationRule, store: FactStore): Fact[] {
  const cons = rule.consequent
  const antecedents = rule.antecedents

  // Collect subjects of the expected type that appear in any antecedent-matching fact
  const subjectType = cons.subject

  // Group facts by subject for the relevant type
  const subjectFacts = new Map<string, Fact[]>()
  for (const f of store.facts) {
    if (f.subjectType !== subjectType) continue
    const existing = subjectFacts.get(f.subject)
    if (existing) {
      existing.push(f)
    } else {
      subjectFacts.set(f.subject, [f])
    }
  }

  const results: Fact[] = []

  for (const [subjectId, facts] of subjectFacts) {
    // Check that ALL antecedents are satisfied by at least one fact for this subject
    let allSatisfied = true
    for (const ant of antecedents) {
      let satisfied = false
      for (const f of facts) {
        if (matchFact(f, ant) !== null) {
          satisfied = true
          break
        }
      }
      if (!satisfied) {
        allSatisfied = false
        break
      }
    }

    if (allSatisfied) {
      const bindings: Bindings = new Map()
      bindings.set(cons.subject, subjectId)
      results.push(produceConsequent(cons, bindings))
    }
  }

  return results
}

/**
 * Join: Datalog-style variable unification across conjuncts.
 *
 * 1. Match facts against first antecedent → set of variable bindings
 * 2. For each subsequent antecedent, match facts and join with existing bindings
 * 3. From surviving bindings, produce consequent facts
 */
function evaluateJoin(rule: DerivationRule, store: FactStore): Fact[] {
  const antecedents = rule.antecedents
  if (antecedents.length === 0) return []

  // Seed: match all facts against the first antecedent
  let bindingSets: Bindings[] = []
  for (const f of store.facts) {
    const b = matchFact(f, antecedents[0])
    if (b !== null) bindingSets.push(b)
  }

  // Join with subsequent antecedents
  for (let i = 1; i < antecedents.length; i++) {
    const ant = antecedents[i]
    const nextBindings: Bindings[] = []

    for (const existing of bindingSets) {
      for (const f of store.facts) {
        const b = matchFact(f, ant)
        if (b === null) continue
        const merged = mergeBindings(existing, b)
        if (merged !== null) nextBindings.push(merged)
      }
    }

    bindingSets = nextBindings
  }

  // Produce consequent facts from each surviving binding set
  return bindingSets.map((b) => produceConsequent(rule.consequent, b))
}

/**
 * Aggregate: per-entity count/sum through where-clause antecedents.
 *
 * Groups by the consequent's subject, counts/sums the aggregate noun
 * per group, and produces one derived fact per group.
 */
function evaluateAggregate(rule: DerivationRule, store: FactStore): Fact[] {
  if (!rule.aggregate) return []
  const { fn, noun: _aggNoun } = rule.aggregate
  const cons = rule.consequent
  const antecedents = rule.antecedents

  // The first antecedent tells us the relationship to group by.
  // We need to find all bindings for the where-clause antecedents,
  // then group by the consequent's subject variable.
  const subjectVar = cons.subject

  // Use join evaluation on the antecedents to get all binding sets
  if (antecedents.length === 0) return []

  let bindingSets: Bindings[] = []
  for (const f of store.facts) {
    const b = matchFact(f, antecedents[0])
    if (b !== null) bindingSets.push(b)
  }

  for (let i = 1; i < antecedents.length; i++) {
    const ant = antecedents[i]
    const nextBindings: Bindings[] = []
    for (const existing of bindingSets) {
      for (const f of store.facts) {
        const b = matchFact(f, ant)
        if (b === null) continue
        const merged = mergeBindings(existing, b)
        if (merged !== null) nextBindings.push(merged)
      }
    }
    bindingSets = nextBindings
  }

  // Group by the subject entity
  const groups = new Map<string, Bindings[]>()
  for (const b of bindingSets) {
    const subjectId = b.get(subjectVar)
    if (!subjectId) continue
    const existing = groups.get(subjectId)
    if (existing) {
      existing.push(b)
    } else {
      groups.set(subjectId, [b])
    }
  }

  const results: Fact[] = []
  for (const [subjectId, bindings] of groups) {
    let aggregatedValue: string

    if (fn === 'count') {
      // Count unique objects (deduplicate by the object values in the bindings)
      const unique = new Set(bindings.map((b) => {
        // Collect all bound values to form a unique key
        return [...b.entries()].filter(([k]) => k !== subjectVar).map(([, v]) => v).join('|')
      }))
      aggregatedValue = String(unique.size)
    } else if (fn === 'sum') {
      let total = 0
      for (const b of bindings) {
        for (const [k, v] of b) {
          if (k !== subjectVar) {
            const n = parseFloat(v)
            if (!isNaN(n)) total += n
          }
        }
      }
      aggregatedValue = String(total)
    } else {
      // Fallback: count
      aggregatedValue = String(bindings.length)
    }

    results.push({
      subject: subjectId,
      subjectType: cons.subject,
      predicate: cons.predicate,
      object: aggregatedValue,
      objectType: cons.object,
      derived: true,
    })
  }

  return results
}

// ─── Public API ─────────────────────────────────────────────────────

/**
 * One pass: evaluate all rules against the store, return only NEW facts
 * (facts that don't already exist in the store).
 */
export function forwardChain(rules: DerivationRule[], store: FactStore): Fact[] {
  const known = new Set(store.facts.map(factKey))
  const newFacts: Fact[] = []
  const newKeys = new Set<string>()

  for (const rule of rules) {
    let derived: Fact[]

    switch (rule.kind) {
      case 'identity':
        derived = evaluateIdentity(rule, store)
        break
      case 'comparison':
        derived = evaluateComparison(rule, store)
        break
      case 'aggregate':
        derived = evaluateAggregate(rule, store)
        break
      case 'join':
      default:
        derived = evaluateJoin(rule, store)
        break
    }

    for (const f of derived) {
      const key = factKey(f)
      if (!isKnown(f, known) && !newKeys.has(key)) {
        newFacts.push(f)
        newKeys.add(key)
      }
    }
  }

  return newFacts
}

/**
 * Loop forward-chain passes until no new facts are produced (fixpoint)
 * or maxIterations is reached.
 *
 * DOES NOT mutate the input store — works on a copy.
 */
export function forwardChainToFixpoint(
  rules: DerivationRule[],
  store: FactStore,
  maxIterations: number = 100,
): FactStore {
  // Copy to avoid mutating the input
  const workingStore: FactStore = {
    facts: [...store.facts],
    entities: store.entities ? { ...store.entities } : undefined,
  }

  for (let i = 0; i < maxIterations; i++) {
    const newFacts = forwardChain(rules, workingStore)
    if (newFacts.length === 0) break // fixpoint
    workingStore.facts.push(...newFacts)
  }

  return workingStore
}
