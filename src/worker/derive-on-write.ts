/**
 * Derivation-on-write: when an entity is created or updated,
 * fire applicable derivation rules and write derived facts back.
 *
 * This is the write-path counterpart to the conceptual query endpoint.
 * The query endpoint reads derived facts; this module produces them.
 */
import { parseRule, type DerivationRule } from '../derivation/parse-rule'
import { forwardChain, type Fact, type FactStore } from '../derivation/forward-chain'

export interface DeriveContext {
  /** The entity that was just written */
  entity: { id: string; type: string; data: Record<string, unknown> }
  /** Load derivation rule texts from the domain's readings */
  loadDerivationRules: () => Promise<Array<{ text: string }>>
  /** Load known noun names from the domain */
  loadNouns: () => Promise<string[]>
  /** Load related entities for building the fact store */
  loadRelatedFacts: (nounType: string) => Promise<Array<{ id: string; type: string; data: Record<string, unknown> }>>
  /** Write a derived fact back (e.g., patch entity data) */
  writeDerivedFact: (entityId: string, field: string, value: string) => Promise<void>
  /** Check if an entity exists */
  entityExists?: (entityId: string) => Promise<boolean>
  /** Create a new entity from a derived fact (entity-creating derivation) */
  createEntity?: (type: string, data: Record<string, unknown>) => Promise<string>
}

/**
 * Run derivation rules after an entity write.
 * Returns the number of derived facts produced.
 */
export async function deriveOnWrite(ctx: DeriveContext): Promise<{ derivedCount: number; derived: Fact[]; createdCount: number }> {
  // Load derivation rules and nouns
  const [ruleTexts, nouns] = await Promise.all([
    ctx.loadDerivationRules(),
    ctx.loadNouns(),
  ])

  if (ruleTexts.length === 0) return { derivedCount: 0, derived: [], createdCount: 0 }

  // Parse rules into IR
  const rules: DerivationRule[] = []
  for (const { text } of ruleTexts) {
    try {
      rules.push(parseRule(text, nouns))
    } catch { /* skip unparseable rules */ }
  }

  if (rules.length === 0) return { derivedCount: 0, derived: [], createdCount: 0 }

  // Determine which noun types are referenced in rules
  const referencedTypes = new Set<string>()
  for (const rule of rules) {
    referencedTypes.add(rule.consequent.subject)
    if (rule.consequent.object) referencedTypes.add(rule.consequent.object)
    for (const ant of rule.antecedents) {
      referencedTypes.add(ant.subject)
      if (ant.object) referencedTypes.add(ant.object)
    }
  }

  // Build fact store from the written entity + related entities
  const store: FactStore = { facts: [], entities: {} }

  // Add the written entity's data as facts
  addEntityFacts(store, ctx.entity)

  // Load related entities for types referenced in rules
  for (const nounType of referencedTypes) {
    if (nounType === ctx.entity.type) continue // already added
    try {
      const related = await ctx.loadRelatedFacts(nounType)
      for (const rel of related) {
        addEntityFacts(store, rel)
      }
      // Track entity IDs by type (for identity rules)
      store.entities![nounType] = related.map(r => r.id)
    } catch { /* type may not exist */ }
  }

  // Add the written entity to entities map
  if (!store.entities![ctx.entity.type]) {
    store.entities![ctx.entity.type] = []
  }
  if (!store.entities![ctx.entity.type].includes(ctx.entity.id)) {
    store.entities![ctx.entity.type].push(ctx.entity.id)
  }

  // Forward-chain to fixpoint (loop manually to collect all derived facts)
  const allDerived: Fact[] = []
  const workingStore: FactStore = { facts: [...store.facts], entities: store.entities ? { ...store.entities } : undefined }
  for (let i = 0; i < 10; i++) {
    const newFacts = forwardChain(rules, workingStore)
    if (newFacts.length === 0) break
    allDerived.push(...newFacts)
    workingStore.facts.push(...newFacts)
  }
  const derived = allDerived

  // Write derived facts back — create entities if they don't exist
  const createdEntities = new Map<string, string>() // "Type:value" → created entity ID
  for (const fact of derived) {
    try {
      // Entity-creating derivation: if the subject doesn't exist as a known
      // entity ID, it's a new entity that needs to be born.
      const subjectKey = `${fact.subjectType}:${fact.subject}`
      if (ctx.entityExists && ctx.createEntity) {
        const exists = await ctx.entityExists(fact.subject)
        if (!exists && !createdEntities.has(subjectKey)) {
          const newId = await ctx.createEntity(fact.subjectType, {
            [fact.objectType]: fact.object,
          })
          createdEntities.set(subjectKey, newId)
          fact.subject = newId
        } else if (createdEntities.has(subjectKey)) {
          // Already created in this derivation run — reuse the ID
          fact.subject = createdEntities.get(subjectKey)!
        }
      }
      await ctx.writeDerivedFact(fact.subject, fact.objectType, fact.object)
    } catch { /* best effort */ }
  }

  return { derivedCount: derived.length, derived, createdCount: createdEntities.size }
}

/** Convert an entity's key-value data into facts in the store */
function addEntityFacts(store: FactStore, entity: { id: string; type: string; data: Record<string, unknown> }) {
  for (const [key, value] of Object.entries(entity.data)) {
    if (value === null || value === undefined) continue
    if (key.startsWith('_')) continue // skip internal fields
    store.facts.push({
      subject: entity.id,
      subjectType: entity.type,
      predicate: 'has',
      object: String(value),
      objectType: key,
    })
  }
}
