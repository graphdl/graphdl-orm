// src/csdp/induce.ts
//
// TypeScript wrapper for WASM induce_from_population.
// Falls back to a pure-TypeScript implementation when WASM is unavailable
// (e.g. in vitest or when the WASM module hasn't been built).

// ── Types (match Rust #[serde(rename_all = "camelCase")] structs) ────

export interface InducedConstraint {
  kind: string          // UC, MC, FC, SS
  factTypeId: string
  reading: string
  roles: number[]
  confidence: number    // 0-1
  evidence: string
}

export interface InducedRule {
  text: string
  antecedentFactTypeIds: string[]
  consequentFactTypeId: string
  confidence: number
  evidence: string
}

export interface PopulationStats {
  factTypeCount: number
  totalFacts: number
  entityCount: number
}

export interface InductionResult {
  constraints: InducedConstraint[]
  rules: InducedRule[]
  populationStats: PopulationStats
}

// ── WASM access (lazy, fails gracefully) ────────────────────────────

let wasmAvailable = false
let wasmLoadIr: ((irJson: string) => void) | null = null
let wasmInduce: ((populationJson: string) => string) | null = null

/**
 * Attempt to initialize the WASM module and bind the induction function.
 * Called lazily on first use. Safe to call multiple times (no-ops after init).
 */
function tryInitWasm(): boolean {
  if (wasmAvailable) return true
  try {
    // Dynamic require to avoid vitest bundling issues.
    // In Cloudflare Workers these imports resolve via the CompiledWasm rule.
    // @ts-ignore — WASM module imported by wrangler's CompiledWasm rule
    const wasmModule = require('../../crates/fol-engine/pkg/fol_engine_bg.wasm')
    // @ts-ignore — WASM JS bindings
    const bindings = require('../../crates/fol-engine/pkg/fol_engine.js')
    bindings.initSync({ module: wasmModule.default || wasmModule })
    wasmLoadIr = bindings.load_ir
    wasmInduce = bindings.induce_from_population
    wasmAvailable = true
    return true
  } catch {
    return false
  }
}

// ── Public API ──────────────────────────────────────────────────────

/**
 * Induce constraints and rules from a population of facts.
 *
 * Tries WASM `induce_from_population` first (requires `load_ir` to have
 * been called). Falls back to a TypeScript implementation when WASM is
 * unavailable (e.g. in test environments).
 *
 * @param irJson   — JSON string of the ConstraintIR schema
 * @param populationJson — JSON string of the population (facts map)
 * @returns InductionResult with constraints, rules, and population stats
 */
export function induceConstraints(irJson: string, populationJson: string): InductionResult {
  // Try WASM path
  if (tryInitWasm() && wasmLoadIr && wasmInduce) {
    try {
      wasmLoadIr(irJson)
      const resultJson = wasmInduce(populationJson)
      return JSON.parse(resultJson) as InductionResult
    } catch {
      // WASM call failed — fall through to TypeScript
    }
  }

  // TypeScript fallback — mirrors crates/fol-engine/src/induce.rs
  return induceTS(JSON.parse(irJson), JSON.parse(populationJson))
}

// ── TypeScript fallback implementation ──────────────────────────────

interface IRSchema {
  nouns: Record<string, { worldAssumption?: string }>
  factTypes: Record<string, {
    reading: string
    roles: Array<{ nounName: string }>
  }>
  constraints: any[]
}

interface PopulationData {
  facts: Record<string, Array<{
    bindings: Array<[string, string]>
  }>>
}

function induceTS(ir: IRSchema, population: PopulationData): InductionResult {
  const constraints: InducedConstraint[] = []
  const rules: InducedRule[] = []

  const facts = population.facts || {}

  // Population stats
  let totalFacts = 0
  const allEntities = new Set<string>()
  for (const ftFacts of Object.values(facts)) {
    totalFacts += ftFacts.length
    for (const fact of ftFacts) {
      for (const [, val] of fact.bindings) {
        allEntities.add(val)
      }
    }
  }

  // ── UC Induction ────────────────────────────────────────────────
  for (const [ftId, ftFacts] of Object.entries(facts)) {
    if (ftFacts.length === 0) continue
    const ft = ir.factTypes[ftId]
    if (!ft) continue
    const arity = ft.roles.length

    // Single-role uniqueness
    for (let roleIdx = 0; roleIdx < arity; roleIdx++) {
      const valueCounts = new Map<string, number>()
      for (const fact of ftFacts) {
        const binding = fact.bindings[roleIdx]
        if (binding) {
          const val = binding[1]
          valueCounts.set(val, (valueCounts.get(val) || 0) + 1)
        }
      }
      const maxCount = Math.max(...valueCounts.values())
      if (maxCount <= 1 && ftFacts.length > 1) {
        constraints.push({
          kind: 'UC',
          factTypeId: ftId,
          reading: ft.reading,
          roles: [roleIdx],
          confidence: ftFacts.length >= 3 ? 0.9 : 0.6,
          evidence: `All ${valueCounts.size} values in role ${ft.roles[roleIdx]?.nounName ?? '?'} are unique across ${ftFacts.length} facts`,
        })
      }

      // FC induction: fixed frequency
      const counts = new Set(valueCounts.values())
      if (counts.size === 1 && ftFacts.length > 2) {
        const n = counts.values().next().value!
        if (n > 1) {
          constraints.push({
            kind: 'FC',
            factTypeId: ftId,
            reading: ft.reading,
            roles: [roleIdx],
            confidence: ftFacts.length >= 4 ? 0.8 : 0.5,
            evidence: `Every ${ft.roles[roleIdx]?.nounName ?? '?'} value occurs exactly ${n} times across ${ftFacts.length} facts`,
          })
        }
      }
    }

    // Compound uniqueness (pair of roles)
    if (arity >= 2) {
      for (let i = 0; i < arity; i++) {
        for (let j = i + 1; j < arity; j++) {
          const tuples = new Set<string>()
          let isUnique = true
          for (const fact of ftFacts) {
            const a = fact.bindings[i]?.[1] ?? ''
            const b = fact.bindings[j]?.[1] ?? ''
            const key = `${a}\0${b}`
            if (tuples.has(key)) {
              isUnique = false
              break
            }
            tuples.add(key)
          }
          if (isUnique && ftFacts.length > 1) {
            // Only report compound UC if neither role is individually unique
            const roleIUnique = constraints.some(c =>
              c.kind === 'UC' && c.factTypeId === ftId && c.roles.length === 1 && c.roles[0] === i,
            )
            const roleJUnique = constraints.some(c =>
              c.kind === 'UC' && c.factTypeId === ftId && c.roles.length === 1 && c.roles[0] === j,
            )
            if (!roleIUnique && !roleJUnique) {
              constraints.push({
                kind: 'UC',
                factTypeId: ftId,
                reading: ft.reading,
                roles: [i, j],
                confidence: ftFacts.length >= 3 ? 0.85 : 0.5,
                evidence: `All (${ft.roles[i]?.nounName ?? '?'}, ${ft.roles[j]?.nounName ?? '?'}) combinations are unique across ${ftFacts.length} facts`,
              })
            }
          }
        }
      }
    }
  }

  // ── MC Induction ────────────────────────────────────────────────
  for (const [ftId, ftFacts] of Object.entries(facts)) {
    const ft = ir.factTypes[ftId]
    if (!ft || ft.roles.length < 2) continue

    const role0Noun = ft.roles[0].nounName
    const role0Values = new Set(
      ftFacts.map(f => f.bindings[0]?.[1]).filter(Boolean) as string[],
    )

    // Collect all known instances of this noun across all fact types
    const allInstances = new Set<string>()
    for (const otherFacts of Object.values(facts)) {
      for (const fact of otherFacts) {
        for (const [noun, val] of fact.bindings) {
          if (noun === role0Noun) allInstances.add(val)
        }
      }
    }

    if (
      allInstances.size > 1 &&
      allInstances.size === role0Values.size &&
      [...allInstances].every(v => role0Values.has(v))
    ) {
      constraints.push({
        kind: 'MC',
        factTypeId: ftId,
        reading: ft.reading,
        roles: [0],
        confidence: allInstances.size >= 3 ? 0.8 : 0.5,
        evidence: `All ${allInstances.size} known ${role0Noun} instances participate in '${ft.reading}'`,
      })
    }
  }

  // ── SS Induction ────────────────────────────────────────────────
  const ftEntities = new Map<string, Set<string>>()
  for (const [ftId, ftFacts] of Object.entries(facts)) {
    const ft = ir.factTypes[ftId]
    if (!ft || ft.roles.length === 0) continue
    const vals = new Set(
      ftFacts.map(f => f.bindings[0]?.[1]).filter(Boolean) as string[],
    )
    ftEntities.set(ftId, vals)
  }

  const ftIds = [...ftEntities.keys()]
  for (let i = 0; i < ftIds.length; i++) {
    for (let j = 0; j < ftIds.length; j++) {
      if (i === j) continue
      const a = ftEntities.get(ftIds[i])!
      const b = ftEntities.get(ftIds[j])!
      if (a.size > 0 && a.size < b.size && [...a].every(v => b.has(v))) {
        const readingA = ir.factTypes[ftIds[i]]?.reading ?? '?'
        const readingB = ir.factTypes[ftIds[j]]?.reading ?? '?'
        constraints.push({
          kind: 'SS',
          factTypeId: ftIds[i],
          reading: `pop('${readingA}') \u2286 pop('${readingB}')`,
          roles: [0],
          confidence: 0.7,
          evidence: `All ${a.size} entities in '${readingA}' also appear in '${readingB}'`,
        })
      }
    }
  }

  // ── Derivation Rule Induction ───────────────────────────────────
  for (const [ftId, ftFacts] of Object.entries(facts)) {
    const ft = ir.factTypes[ftId]
    if (!ft || ft.roles.length < 2 || ftFacts.length < 2) continue

    for (const [otherAId, otherAFacts] of Object.entries(facts)) {
      if (otherAId === ftId) continue
      const otherAFt = ir.factTypes[otherAId]
      if (!otherAFt) continue

      for (const [otherBId, otherBFacts] of Object.entries(facts)) {
        if (otherBId === ftId || otherBId === otherAId) continue
        const otherBFt = ir.factTypes[otherBId]
        if (!otherBFt) continue

        // Find common noun between otherA and otherB
        const aNouns = new Set(otherAFt.roles.map(r => r.nounName))
        const bNouns = new Set(otherBFt.roles.map(r => r.nounName))
        const common = [...aNouns].filter(n => bNouns.has(n))
        if (common.length === 0) continue

        const joinNoun = common[0]
        const joined = new Set<string>()
        for (const aFact of otherAFacts) {
          const aJoinVal = aFact.bindings.find(([n]) => n === joinNoun)?.[1]
          const aOtherVal = aFact.bindings.find(([n]) => n !== joinNoun)?.[1]
          if (!aJoinVal || !aOtherVal) continue

          for (const bFact of otherBFacts) {
            const bJoinVal = bFact.bindings.find(([n]) => n === joinNoun)?.[1]
            const bOtherVal = bFact.bindings.find(([n]) => n !== joinNoun)?.[1]
            if (!bJoinVal || !bOtherVal) continue

            if (aJoinVal === bJoinVal) {
              joined.add(`${aOtherVal}\0${bOtherVal}`)
            }
          }
        }

        // Check if joined results match the target fact type
        const target = new Set(
          ftFacts
            .filter(f => f.bindings.length >= 2)
            .map(f => `${f.bindings[0][1]}\0${f.bindings[1][1]}`),
        )

        if (joined.size > 0 && joined.size === target.size && [...joined].every(v => target.has(v))) {
          rules.push({
            text: `${ft.reading} := ${otherAFt.reading} and ${otherBFt.reading}`,
            antecedentFactTypeIds: [otherAId, otherBId],
            consequentFactTypeId: ftId,
            confidence: 0.9,
            evidence: `Joining '${otherAFt.reading}' and '${otherBFt.reading}' on ${joinNoun} produces exactly the ${target.size} facts in '${ft.reading}'`,
          })
        }
      }
    }
  }

  return {
    constraints,
    rules,
    populationStats: {
      factTypeCount: Object.keys(facts).length,
      totalFacts,
      entityCount: allEntities.size,
    },
  }
}
