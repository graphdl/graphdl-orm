/**
 * theorem1-grammar.test.ts — Theorem 1: Grammar Unambiguity
 *
 * Tests that FORML2 grammar is unambiguous across all three structural families:
 *   (a) Quantified constraints  → UC, MC, FC
 *   (b) Conditional constraints → IR (ring), SS (subset)
 *   (c) Multi-clause constraints → XO (exclusive-or)
 *
 * Also verifies parsing determinism: same readings always produce identical IR.
 */

import { describe, it, expect, afterAll } from 'vitest'
import { compileDomain, ORDER_READINGS, releaseDomain, type CompiledDomain } from '../helpers/domain-fixture'

// ── Minimal reading fixtures for each constraint family ─────────────────────

const UC_READINGS = `# UCTest

## Entity Types
Author(.AuthorId) is an entity type.
Version(.VersionId) is an entity type.

## Value Types
AuthorId is a value type.
VersionId is a value type.

## Fact Types

### Author
Author has Version.

## Constraints
Each Author has at most one Version.
`

const MC_READINGS = `# MCTest

## Entity Types
Author(.AuthorId) is an entity type.
Version(.VersionId) is an entity type.

## Value Types
AuthorId is a value type.
VersionId is a value type.

## Fact Types

### Author
Author has Version.

## Mandatory Constraints
Each Author has some Version.
`

const UC_MC_READINGS = `# UCMCTest

## Entity Types
Author(.AuthorId) is an entity type.
Version(.VersionId) is an entity type.

## Value Types
AuthorId is a value type.
VersionId is a value type.

## Fact Types

### Author
Author has Version.

## Constraints
Each Author has exactly one Version.
`

const FC_READINGS = `# FCTest

## Entity Types
Author(.AuthorId) is an entity type.
Book(.BookId) is an entity type.

## Value Types
AuthorId is a value type.
BookId is a value type.

## Fact Types

### Author
Author has Book.

## Constraints
Each Author has at least 1 and at most 5 Book.
`

const IR_READINGS = `# IRTest

## Entity Types
Person(.PersonId) is an entity type.

## Value Types
PersonId is a value type.

## Fact Types

### Person
Person is parent of Person.

## Ring Constraints
No Person is parent of itself.
`

const SS_READINGS = `# SSTest

## Entity Types
Author(.AuthorId) is an entity type.
Paper(.PaperId) is an entity type.

## Value Types
AuthorId is a value type.
PaperId is a value type.

## Fact Types

### Author
Author writes Paper.
Author reviews Paper.

## Subset Constraints
If some Author writes some Paper then that Author reviews that Paper.
`

const XO_READINGS = `# XOTest

## Entity Types
Task(.TaskId) is an entity type.

## Value Types
TaskId is a value type.
StatusA is a value type.
StatusB is a value type.

## Fact Types

### Task
Task has StatusA.
Task has StatusB.

## Constraints
Exactly one of the following holds: Task has StatusA; Task has StatusB.
`

// ── Diagnostic helper ────────────────────────────────────────────────────────

function hasConstraintKind(compiled: CompiledDomain, kind: string): boolean {
  return compiled.ir.constraints.some(c => c.kind === kind)
}

function constraintKinds(compiled: CompiledDomain): string[] {
  return compiled.ir.constraints.map(c => c.kind)
}

// ── Family (a): Quantified constraints ──────────────────────────────────────

describe('Theorem 1 — Family (a): Quantified constraints', () => {
  describe('(Each, at most one) → UC', () => {
    let compiled: CompiledDomain

    it('compiles "Each A has at most one V." without error', () => {
      compiled = compileDomain(UC_READINGS, 'UCTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains a UC constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('UC')
    })

    it('IR does not contain MC from a pure at-most-one reading', () => {
      // at-most-one is uniqueness only, not mandatory
      const kinds = constraintKinds(compiled)
      // MC should not appear here — only UC
      expect(kinds).not.toContain('MC')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })

  describe('(Each, some) → MC', () => {
    let compiled: CompiledDomain

    it('compiles "Each A has some V." without error', () => {
      compiled = compileDomain(MC_READINGS, 'MCTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains an MC constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('MC')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })

  describe('(Each, exactly one) → UC + MC', () => {
    let compiled: CompiledDomain

    it('compiles "Each A has exactly one V." without error', () => {
      compiled = compileDomain(UC_MC_READINGS, 'UCMCTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains both UC and MC constraints', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('UC')
      expect(kinds).toContain('MC')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })

  describe('(Each, at least k and at most m) → FC', () => {
    let compiled: CompiledDomain

    it('compiles "Each A has at least 1 and at most 5 B." without error', () => {
      compiled = compileDomain(FC_READINGS, 'FCTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains an FC constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('FC')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })
})

// ── Family (b): Conditional constraints ─────────────────────────────────────

describe('Theorem 1 — Family (b): Conditional constraints', () => {
  describe('Same base type → ring constraint (IR)', () => {
    let compiled: CompiledDomain

    it('compiles "No Person is parent of itself." without error', () => {
      compiled = compileDomain(IR_READINGS, 'IRTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains an IR (Irreflexive) constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('IR')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })

  describe('Mixed types with "some"/"that" → subset (SS)', () => {
    let compiled: CompiledDomain

    it('compiles "If some A writes some B then that A reviews that B." without error', () => {
      compiled = compileDomain(SS_READINGS, 'SSTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains an SS (Subset) constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('SS')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })
})

// ── Family (c): Multi-clause constraints ─────────────────────────────────────

describe('Theorem 1 — Family (c): Multi-clause constraints', () => {
  describe('"exactly one of the following holds" → XO', () => {
    let compiled: CompiledDomain

    it('compiles exclusive-or constraint without error', () => {
      compiled = compileDomain(XO_READINGS, 'XOTest')
      expect(compiled.handle).toBeGreaterThanOrEqual(0)
    })

    it('IR contains an XO (ExclusiveOr) constraint', () => {
      const kinds = constraintKinds(compiled)
      expect(kinds).toContain('XO')
    })

    afterAll(() => { if (compiled?.handle >= 0) releaseDomain(compiled.handle) })
  })
})

// ── Determinism ──────────────────────────────────────────────────────────────

describe('Theorem 1 — Parsing determinism', () => {
  it('same readings compiled twice produce identical constraint kinds', () => {
    const a = compileDomain(ORDER_READINGS, 'orders-det-1')
    const b = compileDomain(ORDER_READINGS, 'orders-det-2')

    try {
      const kindsA = constraintKinds(a).sort()
      const kindsB = constraintKinds(b).sort()
      expect(kindsA).toEqual(kindsB)
    } finally {
      if (a.handle >= 0) releaseDomain(a.handle)
      if (b.handle >= 0) releaseDomain(b.handle)
    }
  })

  it('same readings compiled twice produce identical noun lists', () => {
    const a = compileDomain(ORDER_READINGS, 'orders-det-3')
    const b = compileDomain(ORDER_READINGS, 'orders-det-4')

    try {
      expect(a.ir.nouns.sort()).toEqual(b.ir.nouns.sort())
    } finally {
      if (a.handle >= 0) releaseDomain(a.handle)
      if (b.handle >= 0) releaseDomain(b.handle)
    }
  })

  it('same readings compiled twice produce identical fact type counts', () => {
    const a = compileDomain(ORDER_READINGS, 'orders-det-5')
    const b = compileDomain(ORDER_READINGS, 'orders-det-6')

    try {
      expect(a.ir.factTypes.length).toBe(b.ir.factTypes.length)
    } finally {
      if (a.handle >= 0) releaseDomain(a.handle)
      if (b.handle >= 0) releaseDomain(b.handle)
    }
  })
})
