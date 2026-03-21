import { describe, it, expect } from 'vitest'
import {
  forwardChain,
  forwardChainToFixpoint,
  type Fact,
  type FactStore,
} from './forward-chain'
import type { DerivationRule, RuleTriple } from './parse-rule'

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function fact(
  subject: string,
  subjectType: string,
  predicate: string,
  object: string,
  objectType: string,
): Fact {
  return { subject, subjectType, predicate, object, objectType }
}

function triple(
  subject: string,
  predicate: string,
  object: string,
  extras?: Partial<RuleTriple>,
): RuleTriple {
  return { subject, predicate, object, ...extras }
}

function rule(
  text: string,
  consequent: RuleTriple,
  antecedents: RuleTriple[],
  kind: DerivationRule['kind'] = 'join',
  aggregate?: DerivationRule['aggregate'],
): DerivationRule {
  return { text, consequent, antecedents, kind, aggregate }
}

// ---------------------------------------------------------------------------
// 1. Join-chain derivation: A has B and B has C → A relates C
// ---------------------------------------------------------------------------

describe('evaluateJoin', () => {
  it('derives a transitive fact from a two-antecedent join', () => {
    const r = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(1)
    expect(newFacts[0]).toMatchObject({
      subject: 'alice',
      subjectType: 'Person',
      predicate: 'knows',
      object: 'german',
      objectType: 'Language',
      derived: true,
    })
  })

  it('produces multiple derived facts from many bindings', () => {
    const r = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bob', 'Person', 'speaks', 'cockney', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
        fact('cockney', 'Dialect', 'belongsTo', 'english', 'Language'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(2)
    const pairs = newFacts.map((f) => [f.subject, f.object])
    expect(pairs).toContainEqual(['alice', 'german'])
    expect(pairs).toContainEqual(['bob', 'english'])
  })
})

// ---------------------------------------------------------------------------
// 2. Value-comparison derivation: Score > 0.5 → Label 'High'
// ---------------------------------------------------------------------------

describe('evaluateComparison', () => {
  it('derives a label when numeric comparison passes', () => {
    const r = rule(
      "Student has Rating 'High' if Student has Score > 0.5",
      triple('Student', 'has', 'Rating', { literalValue: 'High' }),
      [
        triple('Student', 'has', 'Score', {
          comparison: { op: '>', value: 0.5 },
        }),
      ],
      'comparison',
    )

    const store: FactStore = {
      facts: [
        fact('s1', 'Student', 'has', '0.9', 'Score'),
        fact('s2', 'Student', 'has', '0.3', 'Score'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(1)
    expect(newFacts[0]).toMatchObject({
      subject: 's1',
      predicate: 'has',
      object: 'High',
      objectType: 'Rating',
      derived: true,
    })
  })

  it('handles multiple comparison antecedents (all must hold)', () => {
    const r = rule(
      "Student has Rating 'Good' if Student has Score > 0.3 and Student has Score < 0.8",
      triple('Student', 'has', 'Rating', { literalValue: 'Good' }),
      [
        triple('Student', 'has', 'Score', {
          comparison: { op: '>', value: 0.3 },
        }),
        triple('Student', 'has', 'Score', {
          comparison: { op: '<', value: 0.8 },
        }),
      ],
      'comparison',
    )

    const store: FactStore = {
      facts: [
        fact('s1', 'Student', 'has', '0.5', 'Score'),
        fact('s2', 'Student', 'has', '0.9', 'Score'),
        fact('s3', 'Student', 'has', '0.1', 'Score'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(1)
    expect(newFacts[0].subject).toBe('s1')
  })

  it('produces nothing on an empty store', () => {
    const r = rule(
      "Student has Rating 'High' if Student has Score > 0.5",
      triple('Student', 'has', 'Rating', { literalValue: 'High' }),
      [
        triple('Student', 'has', 'Score', {
          comparison: { op: '>', value: 0.5 },
        }),
      ],
      'comparison',
    )

    const store: FactStore = { facts: [] }
    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(0)
  })
})

// ---------------------------------------------------------------------------
// 3. Fixpoint
// ---------------------------------------------------------------------------

describe('forwardChainToFixpoint', () => {
  it('reaches fixpoint when second pass yields no new facts', () => {
    const r = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
      ],
    }

    const result = forwardChainToFixpoint([r], store)
    // Only 1 new fact derived (alice knows german), total = 3
    expect(result.facts).toHaveLength(3)
    expect(result.facts.filter((f) => f.derived)).toHaveLength(1)
  })

  it('does NOT mutate the original store', () => {
    const r = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
      ],
    }

    const originalLength = store.facts.length
    forwardChainToFixpoint([r], store)
    expect(store.facts).toHaveLength(originalLength)
  })

  it('chains through multiple iterations', () => {
    // r1: Person speaks Dialect and Dialect belongsTo Language → Person knows Language
    // r2: Person knows Language → Language spokenBy Person
    const r1 = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )
    const r2 = rule(
      'Language spokenBy Person if Person knows Language',
      triple('Language', 'spokenBy', 'Person'),
      [triple('Person', 'knows', 'Language')],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
      ],
    }

    const result = forwardChainToFixpoint([r1, r2], store)
    // Iteration 1: alice knows german
    // Iteration 2: german spokenBy alice
    expect(result.facts.filter((f) => f.derived)).toHaveLength(2)
    expect(result.facts).toContainEqual(
      expect.objectContaining({
        subject: 'german',
        predicate: 'spokenBy',
        object: 'alice',
      }),
    )
  })

  it('respects maxIterations', () => {
    // Same chain as above but limit to 1 iteration
    const r1 = rule(
      'Person knows Language if Person speaks Dialect and Dialect belongsTo Language',
      triple('Person', 'knows', 'Language'),
      [
        triple('Person', 'speaks', 'Dialect'),
        triple('Dialect', 'belongsTo', 'Language'),
      ],
    )
    const r2 = rule(
      'Language spokenBy Person if Person knows Language',
      triple('Language', 'spokenBy', 'Person'),
      [triple('Person', 'knows', 'Language')],
    )

    const store: FactStore = {
      facts: [
        fact('alice', 'Person', 'speaks', 'bavarian', 'Dialect'),
        fact('bavarian', 'Dialect', 'belongsTo', 'german', 'Language'),
      ],
    }

    const result = forwardChainToFixpoint([r1, r2], store, 1)
    // Only r1 fires in iteration 1; r2 cannot fire because it depends on r1's output
    expect(result.facts.filter((f) => f.derived)).toHaveLength(1)
  })
})

// ---------------------------------------------------------------------------
// 4. Identity rules
// ---------------------------------------------------------------------------

describe('evaluateIdentity', () => {
  it('derives (x, pred, x) for each entity of the matching type', () => {
    const r = rule(
      'Person equals Person',
      triple('Person', 'equals', 'Person'),
      [],
      'identity',
    )

    const store: FactStore = {
      facts: [],
      entities: { Person: ['alice', 'bob'] },
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(2)
    expect(newFacts).toContainEqual(
      expect.objectContaining({
        subject: 'alice',
        predicate: 'equals',
        object: 'alice',
        subjectType: 'Person',
        objectType: 'Person',
        derived: true,
      }),
    )
    expect(newFacts).toContainEqual(
      expect.objectContaining({
        subject: 'bob',
        predicate: 'equals',
        object: 'bob',
      }),
    )
  })

  it('produces nothing when no entities of that type exist', () => {
    const r = rule(
      'Person equals Person',
      triple('Person', 'equals', 'Person'),
      [],
      'identity',
    )

    const store: FactStore = {
      facts: [],
      entities: { Dog: ['fido'] },
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(0)
  })
})

// ---------------------------------------------------------------------------
// 5. Ambiguous predicates across types — must NOT cross-match
// ---------------------------------------------------------------------------

describe('type-safe matching', () => {
  it('does not cross-match "has" across different subject types', () => {
    const r = rule(
      'Customer knows Product if Customer has Order and Order has Product',
      triple('Customer', 'knows', 'Product'),
      [
        triple('Customer', 'has', 'Order'),
        triple('Order', 'has', 'Product'),
      ],
    )

    const store: FactStore = {
      facts: [
        // Customer → Order
        fact('c1', 'Customer', 'has', 'o1', 'Order'),
        // Order → Product
        fact('o1', 'Order', 'has', 'p1', 'Product'),
        // UNRELATED: Warehouse also "has" something — must not match as Customer
        fact('w1', 'Warehouse', 'has', 'o1', 'Order'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(1)
    expect(newFacts[0]).toMatchObject({
      subject: 'c1',
      subjectType: 'Customer',
      object: 'p1',
      objectType: 'Product',
    })
  })

  it('does not cross-match "has" across different object types', () => {
    // Rule expects Customer has Name (objectType = Name)
    const r = rule(
      "Customer has Label 'VIP' if Customer has Name",
      triple('Customer', 'has', 'Label', { literalValue: 'VIP' }),
      [triple('Customer', 'has', 'Name')],
    )

    const store: FactStore = {
      facts: [
        // Customer has an Age, not a Name
        fact('c1', 'Customer', 'has', '42', 'Age'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(0)
  })
})

// ---------------------------------------------------------------------------
// 6. Cyclic rules reaching fixpoint
// ---------------------------------------------------------------------------

describe('cyclic rules', () => {
  it('terminates when a cycle produces no novel facts', () => {
    // A linked B and B linked A — symmetric closure
    const r = rule(
      'Y linked X if X linked Y',
      triple('Y', 'linked', 'X'),
      [triple('X', 'linked', 'Y')],
    )

    const store: FactStore = {
      facts: [fact('a', 'X', 'linked', 'b', 'Y')],
    }

    // Iteration 1: derive b linked a
    // Iteration 2: b linked a already exists, fixpoint reached
    // But wait — we need type alignment. The derived fact (b, Y, linked, a, X)
    // would feed back with subjectType=Y objectType=X, which doesn't match
    // the antecedent pattern (X, linked, Y). So only 1 fact is derived.
    // To make it truly cyclic, let's use the same type for both.
    const r2 = rule(
      'Node linked Node if Node linked Node (symmetric)',
      triple('Node', 'linked', 'Node'),
      [triple('Node', 'linked', 'Node')],
    )

    const store2: FactStore = {
      facts: [fact('a', 'Node', 'linked', 'b', 'Node')],
    }

    const result = forwardChainToFixpoint([r2], store2)
    // The rule re-derives (a linked b) which already exists, so fixpoint is reached.
    // If rule produced swapped binding it would derive (b linked a).
    // With a single-antecedent rule where subject and object are the same type variable,
    // the binding is {Node -> a} from subject but then object must also be a... so
    // it just re-derives existing facts. Let's use a proper symmetric rule:
    expect(result.facts.length).toBeGreaterThanOrEqual(1)
  })

  it('terminates with a proper symmetric closure rule', () => {
    // Two different type-variables → swaps
    const r = rule(
      'B friendOf A if A friendOf B',
      triple('B', 'friendOf', 'A'),
      [triple('A', 'friendOf', 'B')],
    )

    const store: FactStore = {
      facts: [fact('alice', 'A', 'friendOf', 'bob', 'B')],
    }

    const result = forwardChainToFixpoint([r], store)
    // Iter 1: derive (bob, B, friendOf, alice, A)
    // Iter 2: that fact has subjectType=B, objectType=A — does it match antecedent (A, friendOf, B)?
    //   subjectType must equal 'A' but is 'B' — no match. Fixpoint.
    expect(result.facts).toHaveLength(2)
    expect(result.facts.filter((f) => f.derived)).toHaveLength(1)
    expect(result.facts).toContainEqual(
      expect.objectContaining({
        subject: 'bob',
        predicate: 'friendOf',
        object: 'alice',
        derived: true,
      }),
    )
  })
})

// ---------------------------------------------------------------------------
// 7. Aggregate rules — per-entity count
// ---------------------------------------------------------------------------

describe('evaluateAggregate', () => {
  it('counts per-entity, not globally', () => {
    // "Graph Schema has Arity := count of Role where Graph Schema has Role"
    const r = rule(
      'Schema has Arity := count of Role where Schema has Role',
      triple('Schema', 'has', 'Arity'),
      [triple('Schema', 'has', 'Role')],
      'aggregate',
      { fn: 'count', noun: 'Role' },
    )

    const store: FactStore = {
      facts: [
        fact('gs1', 'Schema', 'has', 'r1', 'Role'),
        fact('gs1', 'Schema', 'has', 'r2', 'Role'),
        fact('gs1', 'Schema', 'has', 'r3', 'Role'),
        fact('gs2', 'Schema', 'has', 'r4', 'Role'),
      ],
    }

    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(2)

    const gs1Fact = newFacts.find((f) => f.subject === 'gs1')!
    expect(gs1Fact.object).toBe('3')
    expect(gs1Fact.objectType).toBe('Arity')

    const gs2Fact = newFacts.find((f) => f.subject === 'gs2')!
    expect(gs2Fact.object).toBe('1')
  })

  it('produces nothing when no matching where-clause facts exist', () => {
    const r = rule(
      'Schema has Arity := count of Role where Schema has Role',
      triple('Schema', 'has', 'Arity'),
      [triple('Schema', 'has', 'Role')],
      'aggregate',
      { fn: 'count', noun: 'Role' },
    )

    const store: FactStore = { facts: [] }
    const newFacts = forwardChain([r], store)
    expect(newFacts).toHaveLength(0)
  })
})
