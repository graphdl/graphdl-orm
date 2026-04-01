# Federation Light-Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Backed nouns resolve from External Systems on demand with pagination, no caching, and no indexing of live data.

**Architecture:** The parser stores backed_by on NounDef. The router checks backed_by during entity listing and detail. For backed nouns, httpFetch calls the External System instead of reading from the registry. Pagination params pass through. No store for browse-only.

**Tech Stack:** Rust (fol-engine parser, types), TypeScript (router, engine), Cloudflare Workers

---

### File Structure

- Modify: `crates/fol-engine/src/types.rs` (add backed_by to NounDef)
- Modify: `crates/fol-engine/src/parse_forml2.rs` (recognize "is backed by" in fact types)
- Modify: `crates/fol-engine/src/lib.rs` (include backed_by in Noun entity output)
- Modify: `src/api/engine.ts` (add getNounBackingInfo to read backed_by from IR)
- Modify: `src/api/router.ts` (list and detail routes check backed_by, call External System)
- Test: `crates/fol-engine/src/parse_forml2.rs` (parser test for backed_by)
- Test: `src/api/seed.test.ts` (verify backed_by appears in parsed entities)

---

### Task 1: Add backed_by to NounDef

**Files:**
- Modify: `crates/fol-engine/src/types.rs:50-76`

- [ ] **Step 1: Write the failing test**

Add to `crates/fol-engine/src/parse_forml2.rs` in the tests module:

```rust
#[test]
fn backed_by_external_system() {
    let input = "Vehicle Specs(.VIN) is an entity type.\nExternal System(.Name) is an entity type.\nVehicle Specs is backed by External System.\nVehicle Specs 'test' is backed by External System 'edmunds'.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.nouns["Vehicle Specs"].backed_by.as_deref(), Some("External System"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/fol-engine/Cargo.toml -- backed_by_external -v`
Expected: FAIL with "no field `backed_by` on type `NounDef`"

- [ ] **Step 3: Add backed_by field to NounDef**

In `crates/fol-engine/src/types.rs`, add after the `rigid` field:

```rust
    /// External System that backs this noun's population.
    /// When set, resolve fetches from the External System instead of local cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backed_by: Option<String>,
```

- [ ] **Step 4: Run test to verify it still fails (field exists but not populated)**

Run: `cargo test --manifest-path crates/fol-engine/Cargo.toml -- backed_by_external -v`
Expected: FAIL with assertion (backed_by is None, expected Some)

- [ ] **Step 5: Commit**

```bash
git add crates/fol-engine/src/types.rs crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: add backed_by field to NounDef"
```

---

### Task 2: Parse "is backed by" fact types

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs`

The reading "Noun is backed by External System" is a fact type. The parser already handles fact types. But the backed_by relationship needs to be extracted and stored on the NounDef, not just as a generic fact type.

- [ ] **Step 1: Add recognition in apply_action for backed_by fact types**

In `crates/fol-engine/src/parse_forml2.rs`, in the `apply_action` function, after the `ParseAction::AddFactType` arm, add handling for fact types that contain "is backed by":

```rust
ParseAction::AddFactType(id, def) => {
    // Check if this is a "backed by" relationship
    if def.reading.contains("is backed by") && def.roles.len() == 2 {
        let subject_noun = &def.roles[0].noun_name;
        let object_noun = &def.roles[1].noun_name;
        if let Some(noun) = ir.nouns.get_mut(subject_noun) {
            noun.backed_by = Some(object_noun.clone());
        }
    }
    ir.fact_types.entry(id).or_insert(def);
}
```

Replace the existing `ParseAction::AddFactType` arm with this version.

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --manifest-path crates/fol-engine/Cargo.toml -- backed_by_external -v`
Expected: PASS

- [ ] **Step 3: Run full test suite**

Run: `cargo test --manifest-path crates/fol-engine/Cargo.toml`
Expected: All 213+ tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: parse 'is backed by' and store on NounDef"
```

---

### Task 3: Include backed_by in Noun entity output

**Files:**
- Modify: `crates/fol-engine/src/lib.rs`

- [ ] **Step 1: Add backed_by to the Noun entity JSON in parse_readings_wasm**

In `crates/fol-engine/src/lib.rs`, in the `parse_readings_wasm` function, in the loop that creates Noun entities (around line 575-603), add after the objectifies insertion:

```rust
if let Some(ref backed) = noun.backed_by {
    data.insert("backedBy".into(), serde_json::Value::String(backed.clone()));
}
```

- [ ] **Step 2: Write a test to verify backed_by appears in parsed entities**

Add to `crates/fol-engine/src/parse_forml2.rs` tests:

```rust
#[test]
fn backed_by_in_ir() {
    let input = "Log Entry(.id) is an entity type.\nExternal System(.Name) is an entity type.\nLog Entry is backed by External System.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.nouns["Log Entry"].backed_by.as_deref(), Some("External System"));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --manifest-path crates/fol-engine/Cargo.toml -- backed_by`
Expected: All backed_by tests pass

- [ ] **Step 4: Build WASM**

Run: `wasm-pack build crates/fol-engine --target web --out-dir ../../src/wasm`
Expected: Build succeeds

- [ ] **Step 5: Commit**

```bash
git add crates/fol-engine/src/lib.rs crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: include backedBy in Noun entity output"
```

---

### Task 4: Add getNounBackingInfo to engine.ts

**Files:**
- Modify: `src/api/engine.ts`

- [ ] **Step 1: Add function to read backing info from IR**

Add after the `getTopLevelNouns` function in `src/api/engine.ts`:

```typescript
/**
 * Get backing info for nouns from the compiled IR.
 * Returns a map of noun name to External System name for backed nouns.
 */
export function getNounBackingInfo(irJson: string): Map<string, string> {
  const ir = JSON.parse(irJson)
  const result = new Map<string, string>()
  if (!ir.nouns) return result
  Object.entries(ir.nouns).forEach(([name, def]: [string, any]) => {
    if (def.backedBy) result.set(name, def.backedBy)
  })
  return result
}
```

- [ ] **Step 2: Commit**

```bash
git add src/api/engine.ts
git commit -m "feat: add getNounBackingInfo to read backed_by from IR"
```

---

### Task 5: Route list requests for backed nouns to External Systems

**Files:**
- Modify: `src/api/router.ts`

- [ ] **Step 1: Add External System resolution to entity listing route**

In `src/api/router.ts`, in the `GET /api/entities/:noun` handler (around line 451), add backing resolution after the registry and getStub setup:

```typescript
router.get('/api/entities/:noun', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const limit = parseInt(url.searchParams.get('limit') || '100', 10)
  const page = parseInt(url.searchParams.get('page') || '1', 10)
  const userEmail = request.headers.get('x-user-email') || ''

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  // Check if this noun is backed by an External System
  let backingSystem: string | null = null
  try {
    const irCell = await getStub(`ir:${domainId}`).get()
    if (irCell?.data?.ir) {
      const irJson = typeof irCell.data.ir === 'string' ? irCell.data.ir : JSON.stringify(irCell.data.ir)
      const backingInfo = getNounBackingInfo(irJson)
      backingSystem = backingInfo.get(noun) || null
    }
  } catch {}

  // If backed, resolve from External System
  if (backingSystem) {
    const systemEntity = await resolveExternalSystem(registry, getStub, backingSystem, domainId)
    if (systemEntity?.baseUrl) {
      const fetchUrl = `${systemEntity.baseUrl}/${encodeURIComponent(noun)}?page=${page}&limit=${limit}`
      const headers: Record<string, string> = { Accept: 'application/json' }
      if (systemEntity.secret) headers[systemEntity.authHeader || 'X-API-Key'] = systemEntity.secret
      try {
        const res = await fetch(fetchUrl, { headers })
        if (res.ok) {
          const data = await res.json() as any
          return json(data)
        }
      } catch {}
    }
    return json({ docs: [], totalDocs: 0, limit, page, totalPages: 0, hasNextPage: false, hasPrevPage: false })
  }

  // Local entity listing (existing code continues here)
  // ...
```

- [ ] **Step 2: Add resolveExternalSystem helper**

Add before the router definition in `src/api/router.ts`:

```typescript
async function resolveExternalSystem(
  registry: any, getStub: (id: string) => any,
  systemName: string, domainSlug: string,
): Promise<{ baseUrl: string; secret: string | null; authHeader: string | null } | null> {
  // Find the External System entity by name
  const systemIds = await registry.getEntityIds('External System', 'core') as string[]
  const systems = await Promise.all(
    systemIds.map(async (id: string) => {
      const cell = await getStub(id).get()
      return cell ? { id: cell.id, ...cell.data } : null
    })
  )
  const system = systems.find((s: any) => s?.name === systemName) as any
  if (!system?.baseUrl) return null

  // Resolve secret from Domain connection
  let secret: string | null = null
  try {
    const domainEntity = await getStub(`Domain:${domainSlug}`).get()
    if (domainEntity?.data?.secretReference) {
      secret = domainEntity.data.secretReference
    }
  } catch {}

  return { baseUrl: system.baseUrl, secret, authHeader: null }
}
```

- [ ] **Step 3: Add getNounBackingInfo import**

At the top of `src/api/router.ts`, add `getNounBackingInfo` to the import from `./engine`:

```typescript
import { loadDomainSchema, loadDomainAndPopulation, getTransitions, applyCommand, querySchema, forwardChain, getNounSchemas, evaluateAccess, deriveViewMetadata, deriveNavContext, getTopLevelNouns, getNounBackingInfo } from './engine'
```

- [ ] **Step 4: Commit**

```bash
git add src/api/router.ts
git commit -m "feat: route backed noun list requests to External Systems"
```

---

### Task 6: Route detail requests for backed nouns to External Systems

**Files:**
- Modify: `src/api/router.ts`

- [ ] **Step 1: Add backing resolution to entity detail route**

In `src/api/router.ts`, in the `GET /api/entities/:noun/:id` handler (around line 521), add backing check before the existing entity fetch:

```typescript
router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const id = decodeURIComponent(request.params.id)
  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain') || undefined

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (eid: string) => getEntityDO(env, eid) as any

  // Check if backed by External System
  if (domainSlug) {
    try {
      const irCell = await getStub(`ir:${domainSlug}`).get()
      if (irCell?.data?.ir) {
        const irJson = typeof irCell.data.ir === 'string' ? irCell.data.ir : JSON.stringify(irCell.data.ir)
        const backingInfo = getNounBackingInfo(irJson)
        const backingSystem = backingInfo.get(noun)
        if (backingSystem) {
          const systemEntity = await resolveExternalSystem(registry, getStub, backingSystem, domainSlug)
          if (systemEntity?.baseUrl) {
            const fetchUrl = `${systemEntity.baseUrl}/${encodeURIComponent(noun)}/${encodeURIComponent(id)}`
            const headers: Record<string, string> = { Accept: 'application/json' }
            if (systemEntity.secret) headers[systemEntity.authHeader || 'X-API-Key'] = systemEntity.secret
            try {
              const res = await fetch(fetchUrl, { headers })
              if (res.ok) return json(await res.json())
            } catch {}
          }
          return error(404, { errors: [{ message: 'Not found in External System' }] })
        }
      }
    } catch {}
  }

  // Existing local entity detail code continues...
```

- [ ] **Step 2: Commit**

```bash
git add src/api/router.ts
git commit -m "feat: route backed noun detail requests to External Systems"
```

---

### Task 7: Build, deploy, re-seed, test

**Files:**
- No new files

- [ ] **Step 1: Build WASM**

Run: `wasm-pack build crates/fol-engine --target web --out-dir ../../src/wasm`
Expected: Build succeeds

- [ ] **Step 2: Deploy**

Run: `yarn deploy`
Expected: Deploy succeeds

- [ ] **Step 3: Re-seed domains with backed_by readings**

Run:
```bash
API_KEY=$(grep AUTO_DEV_API_KEY ~/.claude/.env | cut -d= -f2)
for domain in data-pipeline squishvin-resolution service-health; do
  curl -s -X POST "https://api.auto.dev/arest/seed" \
    -H "Content-Type: application/json" -H "X-API-Key: $API_KEY" \
    -d "{\"domain\":\"$domain\",\"text\":$(jq -Rs . < "../support.auto.dev/domains/${domain}.md")}" | jq '.domains[0] | {domain, entities, errors: (.errors | length)}'
done
```

- [ ] **Step 4: Verify backed noun listing**

```bash
curl -s "https://api.auto.dev/arest/entities/Source%20Resource?domain=data-pipeline&limit=3" \
  -H "X-API-Key: $API_KEY" | jq '{totalDocs, docsCount: (.docs | length)}'
```

If Source Resource is backed by External System 'ClickHouse', this should attempt to fetch from ClickHouse. If ClickHouse is not reachable, it returns empty (graceful degradation).

- [ ] **Step 5: Verify local nouns still work**

```bash
curl -s "https://api.auto.dev/arest/entities/Noun?domain=support&limit=3" \
  -H "X-API-Key: $API_KEY" | jq '{totalDocs, docsCount: (.docs | length)}'
```

Expected: Returns local Noun entities as before.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: federation light-loading for backed nouns"
```
