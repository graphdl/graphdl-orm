import { json, error } from 'itty-router'
import type { Env } from '../types'
import { ingestClaims } from '../claims/ingest'
import type { ExtractedClaims } from '../claims/ingest'
import { parseFORML2 } from './parse'
import { ensureDomain } from './ensure-domain'
import { loadValidationModel, loadDomainSchema, applyCommand } from './engine'
import { buildSchemaIR } from '../csdp/pipeline'

// ── DO helpers ───────────────────────────────────────────────────────

/** Get a DomainDB DO stub for a specific domain slug. */
function getDomainDO(env: Env, domainSlug: string) {
  return env.DOMAIN_DB.get(env.DOMAIN_DB.idFromName(domainSlug))
}

/** Get the global RegistryDB DO stub. */
function getRegistryDO(env: Env) {
  return env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global'))
}

// ── Seed endpoint ────────────────────────────────────────────────────

export async function handleSeed(request: Request, env: Env): Promise<Response> {
  if (request.method === 'GET') {
    return handleSeedGet(env)
  }

  if (request.method === 'DELETE') {
    return handleSeedDelete(env)
  }

  if (request.method === 'POST') {
    return handleSeedPost(request, env)
  }

  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

// ── GET /seed — stats from per-domain DOs ────────────────────────────

async function handleSeedGet(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  const domainSlugs: string[] = await registry.listDomains()

  if (!domainSlugs.length) {
    return json({
      totals: { domains: 0, nouns: 0, readings: 0, graphSchemas: 0, constraints: 0 },
      perDomain: {},
    })
  }

  // Query entity counts per domain via Registry getEntityIds
  const perDomainEntries = await Promise.all(
    domainSlugs.map(async (slug) => {
      const [nounIds, readingIds, schemaIds, constraintIds] = await Promise.all([
        registry.getEntityIds('Noun', slug) as Promise<string[]>,
        registry.getEntityIds('Reading', slug) as Promise<string[]>,
        registry.getEntityIds('GraphSchema', slug) as Promise<string[]>,
        registry.getEntityIds('Constraint', slug) as Promise<string[]>,
      ])
      return {
        slug,
        nouns: nounIds.length,
        readings: readingIds.length,
        graphSchemas: schemaIds.length,
        constraints: constraintIds.length,
      }
    })
  )

  const totals = {
    domains: domainSlugs.length,
    nouns: 0,
    readings: 0,
    graphSchemas: 0,
    constraints: 0,
  }
  const perDomain: Record<string, { nouns: number; readings: number }> = {}

  for (const entry of perDomainEntries) {
    totals.nouns += entry.nouns
    totals.readings += entry.readings
    totals.graphSchemas += entry.graphSchemas
    totals.constraints += entry.constraints
    perDomain[entry.slug] = { nouns: entry.nouns, readings: entry.readings }
  }

  return json({ totals, perDomain })
}

// ── DELETE /seed — wipe all domain DOs ──────────────────────────────

async function handleSeedDelete(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  const domainSlugs: string[] = await registry.listDomains()

  // Wipe each domain DO in parallel
  await Promise.all(
    domainSlugs.map(async (slug) => {
      const domainDO = getDomainDO(env, slug) as any
      await domainDO.wipeAllData()
    })
  )

  return json({ message: 'All data wiped' })
}

// ── POST /seed — parallel per-domain ingestion ───────────────────────

async function handleSeedPost(request: Request, env: Env): Promise<Response> {
  const contentType = request.headers.get('content-type') || ''

  // ── Multipart upload: each file is a .md readings file ──────────
  // Domain slug derived from filename (minus extension).
  // curl -X POST /seed -F core=@readings/core.md -F state=@readings/state.md
  if (contentType.includes('multipart/form-data')) {
    const formData = await request.formData()
    const domains: Array<{ slug: string; name?: string; claims: ExtractedClaims; rawText?: string }> = []

    // Load existing nouns from the registry as parser context.
    // This allows business domains to reference metamodel nouns
    // without re-declaring them.
    const registry = getRegistryDO(env) as any
    let existingNouns: Array<{ name: string; id: string; objectType?: 'entity' | 'value' }> = []
    try {
      const allNounIds: string[] = await registry.getEntityIds('Noun')
      // Noun entity IDs are the noun names themselves
      existingNouns = allNounIds.map(id => ({ name: id, id }))
    } catch {
      // No existing nouns yet — first seed
    }
    if (existingNouns.length > 0) {
      console.log(`[seed] Loaded ${existingNouns.length} existing nouns as parser context`)
    }

    for (const [name, value] of formData.entries()) {
      if (!(value instanceof File) && typeof value !== 'string') continue
      const text = value instanceof File ? await value.text() : value
      if (!text.trim()) continue

      // Slug from field name or filename (minus .md extension)
      const slug = name.replace(/\.md$/i, '')
      const claims = parseFORML2(text, existingNouns)
      domains.push({ slug, name: slug, claims, rawText: text })
    }

    if (domains.length === 0) {
      return error(400, { errors: [{ message: 'No readings files in upload' }] })
    }

    return handleBulkSeed(env, domains)
  }

  // ── JSON body ───────────────────────────────────────────────────
  const body = await request.json() as {
    type?: string
    claims?: ExtractedClaims
    domain?: string
    domainId?: string
    domains?: Array<{ slug: string; name?: string; claims?: ExtractedClaims; text?: string }>
    text?: string
  }

  // Legacy wrapper: type: "claims"
  if (body.type && body.type !== 'claims') {
    return error(400, { errors: [{ message: 'Unsupported seed type. Use type: "claims"' }] })
  }

  // Load existing nouns for JSON modes too
  const registryForJson = getRegistryDO(env) as any
  let existingNounsJson: Array<{ name: string; id: string; objectType?: 'entity' | 'value' }> = []
  try {
    const ids: string[] = await registryForJson.getEntityIds('Noun')
    existingNounsJson = ids.map(id => ({ name: id, id }))
  } catch {}

  // Text mode: parse server-side, then seed
  if (body.text && body.domain) {
    const claims = parseFORML2(body.text, existingNounsJson)
    return handleSingleSeed(env, { claims, domain: body.domain, domainId: body.domain })
  }

  // Bulk: multiple domains — each can have claims or text
  if (body.domains?.length) {
    const resolved = body.domains.map(d => {
      if (d.claims) return d as { slug: string; name?: string; claims: ExtractedClaims }
      if (d.text) return { slug: d.slug, name: d.name, claims: parseFORML2(d.text, existingNounsJson) }
      return { slug: d.slug, name: d.name, claims: { nouns: [], readings: [], constraints: [] } as ExtractedClaims }
    })
    return handleBulkSeed(env, resolved)
  }

  // Single domain with pre-parsed claims
  if (body.claims) {
    return handleSingleSeed(env, body)
  }

  return error(400, { errors: [{ message: 'Provide claims + domain, text + domain, domains[], or multipart file upload' }] })
}

// ── Bulk multi-domain seeding ────────────────────────────────────────

async function handleBulkSeed(
  env: Env,
  domains: Array<{ slug: string; name?: string; claims: ExtractedClaims; rawText?: string }>,
): Promise<Response> {
  const timings: Record<string, number> = {}
  const t = (label: string) => { timings[label] = Date.now() }

  t('start')
  const registry = getRegistryDO(env) as any

  // Phase 1: Seed each domain's metamodel in parallel (steps 1-5)
  t('phase1_start')
  const results = await Promise.all(
    domains.map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      await domainDO.setDomainId(entry.slug)
      const domainRecord = await ensureDomain(env, registry, entry.slug, entry.name)
      const domainUUID = domainRecord.id as string
      const adapter = domainDO as any
      const claimsWithoutFacts = { ...entry.claims, facts: [] }
      const result = await ingestClaims(adapter, {
        claims: claimsWithoutFacts,
        domainId: domainUUID,
      })
      // Commit the batch of metamodel entities to the DomainDB
      if (result.batch.entities.length > 0) {
        await domainDO.commitBatch(result.batch.entities)
      }
      return { domain: entry.slug, domainId: domainUUID, ...result }
    })
  )
  t('phase1_end')

  // Bootstrap validation model from core + validation readings
  bootstrapValidationModel(domains)

  // Build slug → UUID map from phase 1 results
  const slugToUUID = new Map<string, string>()
  for (const r of results) slugToUUID.set(r.domain, r.domainId)

  // Phase 1.5: Register domains + index nouns in Registry
  t('registry_start')
  await Promise.all(
    domains.map(async (entry) => {
      const uuid = slugToUUID.get(entry.slug)
      await registry.registerDomain(entry.slug, entry.slug, 'private', uuid)
    })
  )
  // Index nouns so parser context works for cross-domain references
  const nounPairs: Array<[string, string]> = []
  for (const entry of domains) {
    for (const noun of entry.claims.nouns) {
      nounPairs.push([noun.name, entry.slug])
    }
  }
  for (let i = 0; i < nounPairs.length; i += 50) {
    const batch = nounPairs.slice(i, i + 50)
    await Promise.all(batch.map(([name, slug]) => registry.indexNoun(name, slug)))
  }
  t('registry_end')

  // Phase 2: Process instance facts (after all metamodels are seeded)
  // Note: applySchema (SQL table creation inside DomainDB) is no longer needed —
  // entity instances live in EntityDB DOs, not SQL tables.
  t('phase2_start')

  // Process facts in parallel per domain
  await Promise.all(
    domains.filter(e => e.claims.facts?.length).map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      const uuid = slugToUUID.get(entry.slug) || entry.slug
      for (const fact of entry.claims.facts!) {
        try {
          const entityName = fact.entity || ''
          const entityRef = fact.entityValue || ''
          const fieldValues: Record<string, string> = {}
          if (fact.entity && fact.valueType && fact.value) {
            const fieldName = fact.valueType
              .split(' ')
              .map((w: string, i: number) =>
                i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()
              )
              .join('')
            fieldValues[fieldName] = fact.value
          }
          if (entityName) {
            await domainDO.createEntity(uuid, entityName, fieldValues, entityRef)
          }
        } catch { /* best-effort */ }
      }
    })
  )
  t('phase2_end')

  // Build timing report
  const report: Record<string, string> = {}
  const s = timings['start']
  for (const [k, v] of Object.entries(timings)) {
    if (k !== 'start') report[k] = `${v - s}ms`
  }

  return json({ domains: results, timings: report })
}

// ── Single domain seeding ────────────────────────────────────────────
// Conforms to the AREST spec: every entity goes through the engine's
// command pipeline (resolve → derive → validate → emit).

async function handleSingleSeed(
  env: Env,
  body: { claims?: ExtractedClaims; domain?: string; domainId?: string },
): Promise<Response> {
  const slug = body.domain
  const rawId = body.domainId
  if (!slug && !rawId) {
    return error(400, { errors: [{ message: 'domainId or domains[] required' }] })
  }

  const domainSlug = slug || rawId!
  const registry = getRegistryDO(env) as any

  // Ensure domain record exists
  const domainRecord = await ensureDomain(env, registry, domainSlug)
  await registry.registerDomain(domainSlug, domainSlug, 'private', domainRecord.id)

  // Load domain schema into the WASM engine
  const getStub = (id: string) => env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
  await loadDomainSchema(registry, getStub, domainSlug)

  // Build the current population (may be empty for new domains)
  let population = JSON.stringify({ facts: {} })

  const claims = body.claims!
  let nouns = 0
  let readings = 0
  const errors: string[] = []
  const allEntities: Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }> = []

  // Create each metamodel entity through the engine's command pipeline.
  // Every entity goes through resolve → derive → validate → emit.
  // WASM runs in-process (no subrequests). Entities are collected,
  // then fanned out to EntityDB DOs via the Registry DO (one call).

  // Nouns
  for (const noun of claims.nouns) {
    try {
      const result = applyCommand({
        type: 'createEntity',
        noun: 'Noun',
        domain: domainSlug,
        id: noun.name,
        fields: {
          name: noun.name,
          objectType: noun.objectType,
          ...(noun.plural ? { plural: noun.plural } : {}),
          ...(noun.valueType ? { valueType: noun.valueType } : {}),
          ...(noun.enumValues ? { enumValues: JSON.stringify(noun.enumValues) } : {}),
          ...(noun.refScheme ? { referenceScheme: JSON.stringify(noun.refScheme) } : {}),
          ...(noun.worldAssumption ? { worldAssumption: noun.worldAssumption } : {}),
        },
      }, population)

      // Collect entities from the engine result
      for (const entity of result.entities) {
        allEntities.push({
          id: entity.id || noun.name,
          type: entity.type,
          domain: domainSlug,
          data: entity.data,
        })
      }

      // Update population with the engine's output
      if (!result.rejected && result.population) {
        population = JSON.stringify(result.population)
      }

      nouns++
    } catch (e: any) {
      errors.push(`Noun ${noun.name}: ${e.message || e}`)
    }
  }

  // Readings
  for (const reading of claims.readings) {
    try {
      const result = applyCommand({
        type: 'createEntity',
        noun: 'Reading',
        domain: domainSlug,
        id: null,
        fields: {
          text: reading.text,
          nouns: JSON.stringify(reading.nouns),
          predicate: reading.predicate,
        },
      }, population)

      for (const entity of result.entities) {
        allEntities.push({
          id: entity.id || crypto.randomUUID(),
          type: entity.type,
          domain: domainSlug,
          data: entity.data,
        })
      }

      if (!result.rejected && result.population) {
        population = JSON.stringify(result.population)
      }

      readings++
    } catch (e: any) {
      errors.push(`Reading: ${e.message || e}`)
    }
  }

  // Only materialize entities that belong to THIS domain (not context nouns
  // from previous domains that the parser auto-created).
  const domainEntities = allEntities.filter(e => {
    // Nouns: only materialize if declared in this domain's claims
    if (e.type === 'Noun') {
      return claims.nouns.some(n => n.name === e.data.name)
    }
    // Readings, constraints, etc: always belong to this domain
    return true
  })

  // Fan out: ONE call to Registry DO, which creates EntityDB DOs
  // from its own subrequest budget.
  if (domainEntities.length > 0) {
    await registry.materializeBatch(domainEntities)
  }

  // Index this domain's nouns for parser context
  for (const noun of claims.nouns) {
    await registry.indexNoun(noun.name, domainSlug)
  }

  return json({
    nouns,
    readings,
    created: allEntities.length,
    errors,
    domainId: domainRecord.id,
  })
}

// ── Validation model bootstrap ─────────────────────────────────────

/**
 * Bootstrap the validation model from core + validation readings.
 * Combines their claims into a single ConstraintIR and loads it
 * into the WASM engine. Called once during seed.
 */
function bootstrapValidationModel(
  domains: Array<{ slug: string; claims: ExtractedClaims; rawText?: string }>,
) {
  try {
    const core = domains.find(d => d.slug === 'core')
    const validation = domains.find(d => d.slug === 'validation')
    if (!core || !validation) return

    // Parse core + validation as a single document so validation
    // constraints can reference core's readings (not just noun names).
    // Cross-domain: the constraint "each Role references exactly one Noun"
    // needs the reading "Noun plays Role" in scope to wire spans.
    const combinedText = (core.rawText ?? '') + '\n\n' + (validation.rawText ?? '')
    const combined = parseFORML2(combinedText, [])

    // Build SchemaIR → ConstraintIR → load into engine
    const schemaIR = buildSchemaIR(combined)
    const nouns: Record<string, { objectType: string; superType?: string }> = {}
    for (const n of combined.nouns) {
      nouns[n.name] = { objectType: n.objectType }
    }
    for (const st of (combined.subtypes ?? [])) {
      if (nouns[st.child]) nouns[st.child].superType = st.parent
    }

    const factTypes: Record<string, { reading: string; roles: Array<{ nounName: string; roleIndex: number }> }> = {}
    for (const ft of schemaIR.factTypes) {
      factTypes[ft.id] = { reading: ft.reading, roles: ft.roles }
    }

    const constraints = (schemaIR.constraints ?? []).map((c, i) => ({
      id: `val-${i}`,
      kind: c.kind,
      modality: c.modality ?? 'Alethic',
      text: c.text ?? '',
      spans: (c.roles ?? []).map(roleIdx => ({
        factTypeId: c.factTypeId,
        roleIndex: roleIdx,
      })),
    }))

    const ir = {
      domain: 'validation',
      nouns,
      factTypes,
      constraints,
      stateMachines: {},
      derivationRules: [],
    }

    loadValidationModel(JSON.stringify(ir))
  } catch {
    // Bootstrap failed (WASM unavailable, parse error, etc.) — non-fatal
  }
}

