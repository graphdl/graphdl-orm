/**
 * AppModel — composite DomainModel that merges multiple domains.
 *
 * An App bundles navigable Domains. RMAP generates one OpenAPI spec
 * from the combined readings of all domains. The App is the compilation unit.
 *
 * Implements the same interface as DomainModel so it can be passed
 * directly to generateOpenAPI, generateSQLite, etc.
 */

import type { NounDef, FactTypeDef, ConstraintDef, SpanDef, StateMachineDef } from './types'
import type { DomainModel } from './domain-model'

export class AppModel {
  constructor(
    readonly appId: string,
    private domains: DomainModel[],
  ) {}

  get domainId(): string {
    return this.appId
  }

  /** Merge nouns from all domains. Later domains override earlier on name collision. */
  async nouns(): Promise<Map<string, NounDef>> {
    const merged = new Map<string, NounDef>()
    for (const domain of this.domains) {
      const nouns = await domain.nouns()
      for (const [key, noun] of nouns) {
        merged.set(key, noun)
      }
    }
    return merged
  }

  /** Merge fact types from all domains. */
  async factTypes(): Promise<Map<string, FactTypeDef>> {
    const merged = new Map<string, FactTypeDef>()
    for (const domain of this.domains) {
      const fts = await domain.factTypes()
      for (const [key, ft] of fts) {
        merged.set(key, ft)
      }
    }
    return merged
  }

  /** Concatenate constraints from all domains. */
  async constraints(): Promise<ConstraintDef[]> {
    const all: ConstraintDef[] = []
    for (const domain of this.domains) {
      all.push(...await domain.constraints())
    }
    return all
  }

  /** Merge constraint spans from all domains. */
  async constraintSpans(): Promise<Map<string, SpanDef[]>> {
    const merged = new Map<string, SpanDef[]>()
    for (const domain of this.domains) {
      const spans = await domain.constraintSpans()
      for (const [key, spanList] of spans) {
        const existing = merged.get(key) || []
        merged.set(key, [...existing, ...spanList])
      }
    }
    return merged
  }

  /** Merge state machines from all domains. */
  async stateMachines(): Promise<StateMachineDef[]> {
    const all: StateMachineDef[] = []
    for (const domain of this.domains) {
      all.push(...await domain.stateMachines())
    }
    return all
  }
}
