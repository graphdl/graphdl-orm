# Registry DO + Resolution Chain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `RegistryDB` Durable Objects that index domains and entities at each scope level (app/org/global), and a resolution chain that walks the scope hierarchy to resolve nouns.

**Architecture:** Three Registry DO instances per app (app-level, org-level, global). Each holds a domain index, noun-to-domain index (for fast resolution), and entity ID index per noun type (for fan-out queries). The Worker walks the chain: app → org → global. Pure functions + DO class pattern.

**Tech Stack:** TypeScript, Cloudflare Workers Durable Objects, SQLite, Vitest

**Spec:** `docs/superpowers/specs/2026-03-20-do-per-entity-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `src/registry-do.ts` (new) | `RegistryDB` DO class: schema init, domain registration, noun indexing, entity indexing, resolution |
| `src/registry-do.test.ts` (new) | Unit tests for Registry pure functions |
| `src/resolution.ts` (new) | `resolveNoun` — walks the Registry chain (app → org → global) |
| `src/resolution.test.ts` (new) | Tests for resolution chain |

---

### Task 1: Registry Schema and Domain/Noun Indexing

**Files:**
- Create: `src/registry-do.ts`
- Create: `src/registry-do.test.ts`

- [ ] **Step 1: Write failing tests**

Tests for:
1. `initRegistrySchema` creates domains, noun_index, entity_index tables
2. `registerDomain` adds a domain to the index
3. `registerDomain` is idempotent (upserts)
4. `indexNoun` adds a noun-to-domain mapping
5. `resolveNounInRegistry` finds which domain has a noun
6. `resolveNounInRegistry` returns null for unknown noun

- [ ] **Step 2: Verify tests fail**

- [ ] **Step 3: Implement**

```typescript
// src/registry-do.ts
export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

export function initRegistrySchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS domains (
    domain_slug TEXT PRIMARY KEY,
    domain_do_id TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'private'
  )`)
  sql.exec(`CREATE TABLE IF NOT EXISTS noun_index (
    noun_name TEXT NOT NULL,
    domain_slug TEXT NOT NULL,
    PRIMARY KEY (noun_name, domain_slug)
  )`)
  sql.exec(`CREATE TABLE IF NOT EXISTS entity_index (
    noun_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (noun_type, entity_id)
  )`)
}

export function registerDomain(sql: SqlLike, slug: string, doId: string, visibility?: string): void
export function indexNoun(sql: SqlLike, nounName: string, domainSlug: string): void
export function resolveNounInRegistry(sql: SqlLike, nounName: string): { domainSlug: string; domainDoId: string } | null
```

- [ ] **Step 4: Verify tests pass**

- [ ] **Step 5: Run full suite, commit**

```bash
git add src/registry-do.ts src/registry-do.test.ts
git commit -m "feat: RegistryDB schema, domain registration, noun indexing"
```

---

### Task 2: Entity Indexing

**Files:**
- Modify: `src/registry-do.ts`
- Modify: `src/registry-do.test.ts`

- [ ] **Step 1: Write failing tests**

Tests for:
1. `indexEntity` adds an entity ID under a noun type
2. `indexEntity` is idempotent
3. `deindexEntity` marks entity as deleted (soft)
4. `getEntityIds` returns all non-deleted entity IDs for a noun type
5. `getEntityIds` excludes soft-deleted entities

- [ ] **Step 2: Verify fail, implement, verify pass**

```typescript
export function indexEntity(sql: SqlLike, nounType: string, entityId: string): void
export function deindexEntity(sql: SqlLike, nounType: string, entityId: string): void
export function getEntityIds(sql: SqlLike, nounType: string): string[]
```

- [ ] **Step 3: Run full suite, commit**

```bash
git add src/registry-do.ts src/registry-do.test.ts
git commit -m "feat: RegistryDB entity indexing for fan-out queries"
```

---

### Task 3: RegistryDB DO Class

**Files:**
- Modify: `src/registry-do.ts`
- Modify: `src/index.ts`

- [ ] **Step 1: Add RegistryDB class**

```typescript
export class RegistryDB extends DurableObject {
  private initialized = false

  private ensureInit(): void {
    if (this.initialized) return
    initRegistrySchema(this.ctx.storage.sql)
    this.initialized = true
  }

  async registerDomain(slug: string, doId: string, visibility?: string): Promise<void> { ... }
  async indexNoun(nounName: string, domainSlug: string): Promise<void> { ... }
  async resolveNoun(nounName: string): Promise<{ domainSlug: string; domainDoId: string } | null> { ... }
  async indexEntity(nounType: string, entityId: string): Promise<void> { ... }
  async deindexEntity(nounType: string, entityId: string): Promise<void> { ... }
  async getEntityIds(nounType: string): Promise<string[]> { ... }
}
```

- [ ] **Step 2: Export from index.ts**

- [ ] **Step 3: Run full suite, commit**

```bash
git add src/registry-do.ts src/index.ts
git commit -m "feat: RegistryDB Durable Object class"
```

---

### Task 4: Resolution Chain

**Files:**
- Create: `src/resolution.ts`
- Create: `src/resolution.test.ts`

The resolution chain walks app → org → global Registry DOs to find which domain has a noun.

- [ ] **Step 1: Write failing tests**

Tests for:
1. `resolveNounInChain` finds noun in app registry
2. `resolveNounInChain` falls through to org registry when not in app
3. `resolveNounInChain` falls through to global registry
4. `resolveNounInChain` returns null when noun not found anywhere
5. Short-circuits on first match (doesn't query further registries)

- [ ] **Step 2: Verify fail**

- [ ] **Step 3: Implement**

```typescript
// src/resolution.ts
export interface RegistryStub {
  resolveNoun(nounName: string): Promise<{ domainSlug: string; domainDoId: string } | null>
}

export async function resolveNounInChain(
  nounName: string,
  registries: RegistryStub[],  // [app, org, global] — ordered by priority
): Promise<{ domainSlug: string; domainDoId: string; registryIndex: number } | null> {
  for (let i = 0; i < registries.length; i++) {
    const result = await registries[i].resolveNoun(nounName)
    if (result) return { ...result, registryIndex: i }
  }
  return null
}
```

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Run full suite, commit**

```bash
git add src/resolution.ts src/resolution.test.ts
git commit -m "feat: noun resolution chain walks app → org → global registries"
```
