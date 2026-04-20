/**
 * Federation — Data federation and Citation provenance.
 *
 * Extracted from mcp/server.ts to give the fetch path a clean seam
 * for testing and so Citation-emission can live alongside the fetch
 * that originates the facts.
 *
 * Architectural context (E3 / #305 + paper §3.3 Data Federation):
 *   `populate:{noun}` compiles to a constant config in the engine.
 *   This module reads that config and performs the actual HTTP call
 *   that the paper describes as `ρ(populate_n)`. Each call produces
 *   a set of facts under OWA *plus* a Citation fact with Authority
 *   Type 'Federated-Fetch' recording origin (URL, system, retrieval
 *   date). The caller links each returned entity fact to that
 *   Citation via the `Fact cites Citation` fact type.
 *
 * IoC/DI note: the engine side of the populate path still lives in
 * compile.rs (populate:{noun} def) + ast.rs register_runtime_fn
 * (↓DEFS writer). This TS module is the runtime wrapper that makes
 * the fetch synchronously appear as OWA facts to the engine.
 */

export interface FederationConfig {
  system: string
  url: string
  uri: string
  header: string
  prefix: string
  noun: string
  fields: string[]
}

/**
 * Citation — provenance record per paper §3 / readings/instances.md.
 *
 * Authority Type 'Federated-Fetch' is the Citation kind this module
 * emits. 'Runtime-Function' is emitted by the Platform dispatch path
 * in the Rust engine (future work) when a runtime-registered Platform
 * function produces a fact.
 */
export interface Citation {
  uri: string
  retrievalDate: string
  authorityType: 'Runtime-Function' | 'Federated-Fetch' | string
  externalSystem?: string
  text?: string
}

export interface FederatedFetchResult {
  system: string
  noun: string
  count: number
  facts: Array<Record<string, string>>
  citation?: Citation
  _meta: { url: string; worldAssumption: 'OWA' }
  error?: string
}

/**
 * Payload for the engine's `federated_ingest:<noun>` FFI key (#305).
 *
 * The engine expects one entry per (fact-type, bindings) tuple — one
 * fact per (entity, field) pair on the TS side. The factTypeId follows
 * the canonical `Noun_verb_Role` shape produced by the parser's
 * `fact_type_id_from_reading` (spaces replaced with underscores).
 */
export interface IngestPayload {
  externalSystem: string
  url: string
  retrievalDate: string
  facts: Array<{
    factTypeId: string
    bindings: Record<string, string>
  }>
}

/**
 * Enrich a federated fetch response with HATEOAS provenance linkage
 * (#305 #9). Adds:
 *   - citationId — the stable engine-assigned Citation id
 *   - absorbed: true — signal that the citation reached P
 *   - _links.citations — navigable link to the Citation entity
 *
 * Pre-existing _links entries are preserved. Consumers walking the
 * link graph can now reach the provenance record without knowing the
 * citationId→URL mapping by convention.
 */
export function enrichResponseWithCitation<T extends { _links?: Record<string, unknown> }>(
  data: T,
  citationId: string,
  basePath: string,
): T & {
  citationId: string
  absorbed: true
  _links: Record<string, unknown> & { citations: { href: string } }
} {
  const existingLinks = (data._links ?? {}) as Record<string, unknown>
  return {
    ...data,
    citationId,
    absorbed: true as const,
    _links: {
      ...existingLinks,
      citations: { href: `${basePath}/Citation/${citationId}` },
    },
  }
}

/**
 * Translate a FederatedFetchResult into the engine's federated_ingest
 * JSON shape. Each entity record is split into one fact per field
 * (the noun id stays in every binding so the engine can identify the
 * entity). When the fetch produced no citation (e.g., bare empty-state
 * result), the payload is returned with an empty facts array so the
 * caller can skip the FFI call.
 */
export function buildIngestPayload(result: FederatedFetchResult): IngestPayload {
  if (!result.citation) {
    return {
      externalSystem: result.system,
      url: result._meta.url,
      retrievalDate: new Date().toISOString(),
      facts: [],
    }
  }
  const noun = result.noun
  const nounUnderscore = noun.replace(/ /g, '_')
  const facts: IngestPayload['facts'] = []
  for (const record of result.facts) {
    const entityId = record[noun]
    if (entityId === undefined) continue
    for (const [field, value] of Object.entries(record)) {
      if (field === noun) continue
      const factTypeId = `${nounUnderscore}_has_${field.replace(/ /g, '_')}`
      facts.push({ factTypeId, bindings: { [noun]: entityId, [field]: value } })
    }
  }
  return {
    externalSystem: result.citation.externalSystem || result.system,
    url: result.citation.uri,
    retrievalDate: result.citation.retrievalDate,
    facts,
  }
}

/** Parse a populate:{noun} def from the engine into a FederationConfig. */
export function parseFederationConfig(raw: string): FederationConfig | null {
  try {
    const config: Record<string, string | string[]> = {}
    const pairRe = /<([^,<>]+),\s*([^<>]*?)>/g
    let match
    while ((match = pairRe.exec(raw)) !== null) {
      const [, key, value] = match
      config[key.trim()] = value.trim()
    }
    const fieldsMatch = raw.match(/fields,\s*<([^>]*)>/)
    const fields = fieldsMatch
      ? fieldsMatch[1].split(',').map((s) => s.trim().replace(/^'|'$/g, ''))
      : []
    return {
      system: String(config['system'] || ''),
      url: String(config['url'] || ''),
      uri: String(config['uri'] || ''),
      header: String(config['header'] || ''),
      prefix: String(config['prefix'] || ''),
      noun: String(config['noun'] || ''),
      fields,
    }
  } catch {
    return null
  }
}

function buildFetchUrl(config: FederationConfig, entityId?: string): string {
  const baseUrl = config.url.replace(/\/$/, '')
  const path = config.uri.replace(/^\//, '')
  return entityId
    ? `${baseUrl}/${path}/${encodeURIComponent(entityId)}`
    : `${baseUrl}/${path}`
}

function buildCitation(config: FederationConfig, url: string): Citation {
  const retrievalDate = new Date().toISOString()
  return {
    uri: url,
    retrievalDate,
    authorityType: 'Federated-Fetch',
    externalSystem: config.system,
    text: `Federated fetch of ${config.noun} from ${config.system} at ${retrievalDate}`,
  }
}

/**
 * Fetch facts from an external system using a populate config.
 *
 * Returns entity facts under OWA plus a single Citation recording
 * origin. All returned facts share this one Citation (they came from
 * the same ρ(populate_n) application at the same moment). The caller
 * is responsible for emitting paired Fact cites Citation facts.
 *
 * Error path still emits a Citation — the *origin of the error*
 * remains the same external system at the same URL, and downstream
 * derivations/constraints over error facts should be able to cite it.
 */
export async function federatedFetch(
  config: FederationConfig,
  entityId?: string,
): Promise<FederatedFetchResult> {
  const url = buildFetchUrl(config, entityId)
  const citation = buildCitation(config, url)

  const headers: Record<string, string> = { Accept: 'application/json' }
  const envKey = `AREST_SECRET_${config.system.replace(/[^a-zA-Z0-9]/g, '_').toUpperCase()}`
  const secret = (globalThis as { process?: { env?: Record<string, string | undefined> } })
    .process?.env?.[envKey] || ''
  if (config.header && secret) {
    headers[config.header] = config.prefix ? `${config.prefix} ${secret}` : secret
  }

  const res = await fetch(url, { headers })
  if (!res.ok) {
    return {
      system: config.system,
      noun: config.noun,
      count: 0,
      facts: [],
      citation,
      error: `${res.status} ${res.statusText}`,
      _meta: { url, worldAssumption: 'OWA' },
    }
  }

  const json = (await res.json()) as unknown as Record<string, unknown>
  const raw: unknown =
    Array.isArray((json as { data?: unknown }).data)
      ? (json as { data: unknown[] }).data
      : Array.isArray(json)
      ? json
      : [json]
  const items = raw as Array<Record<string, unknown>>

  const facts = items.map((item) => {
    const bindings: Record<string, string> = {}
    config.fields.forEach((field) => {
      const snakeField = field.toLowerCase().replace(/ /g, '_')
      const val = item[field] ?? item[snakeField] ?? item[field.replace(/ /g, '')]
      if (val !== undefined) bindings[field] = String(val)
    })
    if (item.id !== undefined) bindings[config.noun] = String(item.id)
    return bindings
  })

  return {
    system: config.system,
    noun: config.noun,
    count: facts.length,
    facts,
    citation,
    _meta: { url, worldAssumption: 'OWA' },
  }
}
