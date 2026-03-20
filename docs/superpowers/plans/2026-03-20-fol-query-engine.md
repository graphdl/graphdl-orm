# FOL Query Engine + Eager Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the FOL engine with `query_population` for collection predicate evaluation, and implement eager enrichment via subset constraint autofill on the write path.

**Architecture:** The FOL engine (Rust/WASM) gets a new export: `query_population` that evaluates predicates over a Population and returns matching entities. The Worker uses this for aggregate queries (fan-out to Entity DOs → collect → FOL reduce). On writes, forward chaining with subset autofill resolves cross-entity relationships eagerly. FP is the basis of parallelism — pure functions over immutable populations.

**Tech Stack:** Rust, wasm-bindgen, TypeScript, Vitest, cargo test

**Spec:** `docs/superpowers/specs/2026-03-20-do-per-entity-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/fol-engine/src/query.rs` (new) | `query_population` — filter/aggregate over Population using compiled predicates |
| `crates/fol-engine/src/lib.rs` (modify) | New WASM export: `query_population` |
| `crates/fol-engine/src/types.rs` (modify) | `QueryPredicate` type for collection filtering |
| `src/worker/query.ts` (new) | Worker-side query orchestration: fan-out to Entity DOs → build Population → call FOL |
| `src/worker/query.test.ts` (new) | Tests for query orchestration |
| `src/worker/enrichment.ts` (new) | Eager enrichment: forward chain on write, resolve subset autofill via fan-out |
| `src/worker/enrichment.test.ts` (new) | Tests for enrichment |

---

### Task 1: FOL Engine — query_population in Rust

**Files:**
- Create: `crates/fol-engine/src/query.rs`
- Modify: `crates/fol-engine/src/lib.rs`
- Modify: `crates/fol-engine/src/types.rs`

Add a `query_population` function that takes a compiled model + population + query predicate and returns matching entities.

A query predicate is: "find all instances of noun X where fact type F has binding value V". This maps directly to the existing `instances_of` + `participates_in` helpers in `compile.rs`.

- [ ] **Step 1: Write failing Rust test**

```rust
// In query.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Population, FactInstance};

    #[test]
    fn test_query_filters_by_binding_value() {
        let mut pop = Population { facts: HashMap::new() };
        // Add facts: SupportRequest has Status
        pop.facts.insert("ft1".to_string(), vec![
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("SupportRequest".to_string(), "sr-001".to_string()),
                    ("Status".to_string(), "Investigating".to_string()),
                ],
            },
            FactInstance {
                fact_type_id: "ft1".to_string(),
                bindings: vec![
                    ("SupportRequest".to_string(), "sr-002".to_string()),
                    ("Status".to_string(), "Resolved".to_string()),
                ],
            },
        ]);

        let predicate = QueryPredicate {
            fact_type_id: "ft1".to_string(),
            target_noun: "SupportRequest".to_string(),
            filter_bindings: vec![
                ("Status".to_string(), "Investigating".to_string()),
            ],
        };

        let results = query_population(&pop, &predicate);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "sr-001");
    }
}
```

- [ ] **Step 2: Run `cargo test` in crates/fol-engine — verify fail**

- [ ] **Step 3: Implement query_population**

```rust
// crates/fol-engine/src/query.rs
use crate::types::{Population, QueryPredicate};
use std::collections::HashSet;

pub fn query_population(population: &Population, predicate: &QueryPredicate) -> Vec<String> {
    let facts = match population.facts.get(&predicate.fact_type_id) {
        Some(facts) => facts,
        None => return vec![],
    };

    let mut results = Vec::new();
    for fact in facts {
        // Check all filter bindings match
        let matches = predicate.filter_bindings.iter().all(|(noun, value)| {
            fact.bindings.iter().any(|(n, v)| n == noun && v == value)
        });
        if matches {
            // Extract the target noun's value
            if let Some((_, entity_ref)) = fact.bindings.iter().find(|(n, _)| n == &predicate.target_noun) {
                results.push(entity_ref.clone());
            }
        }
    }
    results
}
```

- [ ] **Step 4: Add QueryPredicate to types.rs**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPredicate {
    pub fact_type_id: String,
    pub target_noun: String,
    pub filter_bindings: Vec<(String, String)>,
}
```

- [ ] **Step 5: Add WASM export to lib.rs**

```rust
#[wasm_bindgen]
pub fn query_population(population_json: &str, predicate_json: &str) -> String {
    let population: Population = serde_json::from_str(population_json).unwrap_or_default();
    let predicate: QueryPredicate = serde_json::from_str(predicate_json).unwrap_or_default();
    let results = query::query_population(&population, &predicate);
    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
}
```

- [ ] **Step 6: Run `cargo test` — verify all pass (existing 27 + new)**

- [ ] **Step 7: Build WASM**

```bash
cd crates/fol-engine && wasm-pack build --target bundler --out-dir ../../src/wasm
```

- [ ] **Step 8: Commit**

```bash
git add crates/fol-engine/src/
git commit -m "feat: FOL engine query_population for collection predicate evaluation"
```

---

### Task 2: Worker Query Orchestration (TypeScript)

**Files:**
- Create: `src/worker/query.ts`
- Create: `src/worker/query.test.ts`

The Worker-side code that: parses a reading-level query, fans out to Entity DOs, builds a Population, calls FOL query_population.

- [ ] **Step 1: Write failing tests**

Tests:
1. `buildPopulationFromEntities` converts Entity DO responses to a Population
2. `executeQuery` fans out to Entity DOs, builds Population, filters with predicate
3. `executeQuery` returns empty array when no entities match

- [ ] **Step 2: Implement**

```typescript
// src/worker/query.ts
import type { EntityDB } from '../entity-do'

export interface QueryRequest {
  nounType: string
  factTypeId: string
  filterBindings: Array<[string, string]>
}

export function buildPopulationFromEntities(
  nounType: string,
  entities: Array<{ id: string; data: Record<string, unknown> }>,
  factTypeId: string,
): Population { ... }

export async function executeQuery(
  entityIds: string[],
  getEntityDO: (id: string) => EntityDB,
  query: QueryRequest,
  batchSize?: number,
): Promise<string[]> {
  // Fan out in batches, collect data, build Population, query with FOL
  ...
}
```

- [ ] **Step 3: Verify pass, full suite, commit**

```bash
git add src/worker/
git commit -m "feat: Worker query orchestration — fan-out, Population building, FOL reduction"
```

---

### Task 3: Eager Enrichment via Subset Autofill

**Files:**
- Create: `src/worker/enrichment.ts`
- Create: `src/worker/enrichment.test.ts`

On entity write, forward chain with subset autofill to derive cross-entity relationships.

- [ ] **Step 1: Write failing tests**

Tests:
1. `enrichEntity` with no autofill constraints returns data unchanged
2. `enrichEntity` with autofill resolves a cross-entity reference (e.g., phone number → customer)
3. Enrichment is idempotent

- [ ] **Step 2: Implement**

```typescript
// src/worker/enrichment.ts

export interface EnrichmentContext {
  constraints: Array<{ factTypeId: string; autofill: boolean; ... }>
  resolveEntities: (nounType: string, filterField: string, filterValue: string) => Promise<string[]>
}

export async function enrichEntity(
  entityData: Record<string, unknown>,
  nounType: string,
  ctx: EnrichmentContext,
): Promise<Record<string, unknown>> {
  // For each autofill constraint:
  //   1. Find the field that matches the constraint's binding
  //   2. Fan out to Entity DOs to find matching entities
  //   3. Derive the relationship fact
  //   4. Add to entity data
  ...
}
```

- [ ] **Step 3: Verify pass, full suite, commit**

```bash
git add src/worker/enrichment.ts src/worker/enrichment.test.ts
git commit -m "feat: eager enrichment via subset constraint autofill on writes"
```
