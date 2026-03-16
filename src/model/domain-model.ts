import type {
  NounDef,
  FactTypeDef,
  RoleDef,
  ConstraintDef,
  SpanDef,
  StateMachineDef,
  StatusDef,
  TransitionDef,
  VerbDef,
  ReadingDef,
} from './types'
import type { Generator } from './renderer'
import { render } from './renderer'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Parse enum_values from DB — handles JSON arrays and comma-separated strings. */
function parseEnumValues(raw: string): string[] {
  const trimmed = raw.trim()
  if (trimmed.startsWith('[')) {
    try { return JSON.parse(trimmed) } catch { /* fall through */ }
  }
  return trimmed.split(',').map((s: string) => s.trim()).filter(Boolean)
}

// ---------------------------------------------------------------------------
// DataLoader
// ---------------------------------------------------------------------------

type Row = Record<string, any>

export interface DataLoader {
  queryNouns(domainId: string): Row[]
  queryGraphSchemas(domainId: string): Row[]
  queryReadings(domainId: string): Row[]
  queryRoles(): Row[]
  queryConstraints(domainId: string): Row[]
  queryConstraintSpans(): Row[]
  queryStateMachineDefs(domainId: string): Row[]
  queryStatuses(domainId: string): Row[]
  queryTransitions(domainId: string): Row[]
  queryEventTypes(domainId: string): Row[]
  queryGuards(domainId: string): Row[]
  queryVerbs(domainId: string): Row[]
  queryFunctions(domainId: string): Row[]
}

// ---------------------------------------------------------------------------
// SqlDataLoader
// ---------------------------------------------------------------------------

export interface SqlStorage {
  exec(query: string, ...bindings: any[]): Iterable<Row>
}

export const CORE_DOMAIN_ID = 'graphdl-core'

/** Entity nouns without an explicit supertype default to this root. */
export const DEFAULT_ENTITY_SUPERTYPE = 'Resource'

/** Core root entity names that should NOT get a default supertype. */
export const DEFAULT_SUPERTYPE_ROOTS = new Set(['Resource', 'Noun'])

export class SqlDataLoader implements DataLoader {
  constructor(private sql: SqlStorage) {}

  queryNouns(domainId: string): Row[] {
    return [
      ...this.sql.exec(
        `SELECT n.*, p.name as super_type_name FROM nouns n LEFT JOIN nouns p ON n.super_type_id = p.id WHERE n.domain_id IN (?, '${CORE_DOMAIN_ID}')`,
        domainId,
      ),
    ]
  }

  queryGraphSchemas(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM graph_schemas WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryReadings(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM readings WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryRoles(): Row[] {
    return [
      ...this.sql.exec(
        'SELECT r.*, gs.domain_id FROM roles r JOIN graph_schemas gs ON r.graph_schema_id = gs.id',
      ),
    ]
  }

  queryConstraints(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM constraints WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryConstraintSpans(): Row[] {
    return [
      ...this.sql.exec(
        'SELECT cs.*, c.domain_id, r.graph_schema_id, r.role_index FROM constraint_spans cs JOIN constraints c ON cs.constraint_id = c.id JOIN roles r ON cs.role_id = r.id',
      ),
    ]
  }

  queryStateMachineDefs(domainId: string): Row[] {
    return [
      ...this.sql.exec(`SELECT * FROM state_machine_definitions WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId),
    ]
  }

  queryStatuses(domainId: string): Row[] {
    return [
      ...this.sql.exec(
        `SELECT s.*, smd.domain_id FROM statuses s JOIN state_machine_definitions smd ON s.state_machine_definition_id = smd.id WHERE smd.domain_id IN (?, '${CORE_DOMAIN_ID}')`,
        domainId,
      ),
    ]
  }

  queryTransitions(domainId: string): Row[] {
    return [
      ...this.sql.exec(
        `SELECT t.* FROM transitions t JOIN statuses s ON t.from_status_id = s.id JOIN state_machine_definitions smd ON s.state_machine_definition_id = smd.id WHERE smd.domain_id IN (?, '${CORE_DOMAIN_ID}')`,
        domainId,
      ),
    ]
  }

  queryEventTypes(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM event_types WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryGuards(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM guards WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryVerbs(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM verbs WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }

  queryFunctions(domainId: string): Row[] {
    return [...this.sql.exec(`SELECT * FROM functions WHERE domain_id IN (?, '${CORE_DOMAIN_ID}')`, domainId)]
  }
}

// ---------------------------------------------------------------------------
// Invalidation map
// ---------------------------------------------------------------------------

const INVALIDATION_MAP: Record<string, string[]> = {
  'nouns': ['nouns', 'factTypes', 'constraints', 'constraintSpans', 'readings'],
  'graph-schemas': ['factTypes'],
  'readings': ['factTypes', 'readings'],
  'roles': ['factTypes'],
  'constraints': ['constraints', 'constraintSpans'],
  'constraint-spans': ['constraints', 'constraintSpans'],
  'state-machine-definitions': ['stateMachines'],
  'statuses': ['stateMachines'],
  'transitions': ['stateMachines'],
  'guards': ['stateMachines'],
  'event-types': ['stateMachines'],
  'verbs': ['stateMachines'],
  'functions': ['stateMachines'],
}

// ---------------------------------------------------------------------------
// DomainModel
// ---------------------------------------------------------------------------

export class DomainModel {
  private cache: Map<string, any> = new Map()

  constructor(
    private loader: DataLoader,
    readonly domainId: string,
  ) {}

  // -------------------------------------------------------------------------
  // nouns
  // -------------------------------------------------------------------------

  async nouns(): Promise<Map<string, NounDef>> {
    if (this.cache.has('nouns')) return this.cache.get('nouns')

    const rows = this.loader.queryNouns(this.domainId)
    const map = new Map<string, NounDef>()

    for (const row of rows) {
      const noun: NounDef = {
        id: row.id,
        name: row.name,
        objectType: row.object_type,
        domainId: row.domain_id,
        plural: row.plural ?? undefined,
        description: row.prompt_text ?? undefined,
        valueType: row.value_type ?? undefined,
        format: row.format ?? undefined,
        pattern: row.pattern ?? undefined,
        enumValues: row.enum_values ? parseEnumValues(row.enum_values) : undefined,
        minimum: row.minimum ?? undefined,
        maximum: row.maximum ?? undefined,
        superType: row.super_type_name ?? undefined,
      }
      map.set(noun.name, noun)
    }

    // Default supertype: entity nouns without an explicit supertype inherit from
    // the core root entity for their kind. This ensures domain entities like
    // SupportRequest → Request → Resource and get core properties (state machines).
    for (const [, noun] of map) {
      if (noun.superType || noun.objectType !== 'entity') continue
      if (noun.domainId === CORE_DOMAIN_ID) continue // core nouns define the roots
      if (DEFAULT_SUPERTYPE_ROOTS.has(noun.name)) continue // don't self-reference
      noun.superType = DEFAULT_ENTITY_SUPERTYPE
    }

    this.cache.set('nouns', map)
    return map
  }

  async noun(name: string): Promise<NounDef | undefined> {
    const nouns = await this.nouns()
    return nouns.get(name)
  }

  // -------------------------------------------------------------------------
  // factTypes
  // -------------------------------------------------------------------------

  async factTypes(): Promise<Map<string, FactTypeDef>> {
    if (this.cache.has('factTypes')) return this.cache.get('factTypes')

    const nouns = await this.nouns()
    const gsRows = this.loader.queryGraphSchemas(this.domainId)
    const readingRows = this.loader.queryReadings(this.domainId)
    const roleRows = this.loader.queryRoles()

    // Index roles by graph_schema_id (include core domain roles)
    const rolesByGs = new Map<string, Row[]>()
    for (const r of roleRows) {
      if (r.domain_id !== this.domainId && r.domain_id !== CORE_DOMAIN_ID) continue
      const list = rolesByGs.get(r.graph_schema_id) ?? []
      list.push(r)
      rolesByGs.set(r.graph_schema_id, list)
    }

    // Index readings by graph_schema_id (first match)
    const readingByGs = new Map<string, Row>()
    for (const rd of readingRows) {
      if (!readingByGs.has(rd.graph_schema_id)) {
        readingByGs.set(rd.graph_schema_id, rd)
      }
    }

    // Build a noun lookup by id
    const nounById = new Map<string, NounDef>()
    for (const [, n] of nouns) {
      nounById.set(n.id, n)
    }

    const map = new Map<string, FactTypeDef>()

    for (const gs of gsRows) {
      const roles = (rolesByGs.get(gs.id) ?? [])
        .sort((a: Row, b: Row) => a.role_index - b.role_index)
        .map((r: Row): RoleDef => {
          const nounDef = nounById.get(r.noun_id)!
          return {
            id: r.id,
            nounName: nounDef?.name ?? '',
            nounDef,
            roleIndex: r.role_index,
          }
        })

      const reading = readingByGs.get(gs.id)

      const ft: FactTypeDef = {
        id: gs.id,
        name: gs.name ?? undefined,
        reading: reading?.text ?? '',
        roles,
        arity: roles.length,
      }
      map.set(gs.id, ft)
    }

    this.cache.set('factTypes', map)
    return map
  }

  async factTypesFor(noun: NounDef): Promise<FactTypeDef[]> {
    const fts = await this.factTypes()
    const result: FactTypeDef[] = []
    for (const [, ft] of fts) {
      if (ft.roles.some((r) => r.nounDef?.name === noun.name && r.roleIndex === 0)) {
        result.push(ft)
      }
    }
    return result
  }

  // -------------------------------------------------------------------------
  // constraints
  // -------------------------------------------------------------------------

  async constraints(): Promise<ConstraintDef[]> {
    if (this.cache.has('constraints')) return this.cache.get('constraints')

    const constraintRows = this.loader.queryConstraints(this.domainId)
    const spanRows = this.loader.queryConstraintSpans()

    // Group spans by constraint_id, filtering to this domain + core
    const spansByConstraint = new Map<string, Row[]>()
    for (const cs of spanRows) {
      if (cs.domain_id !== this.domainId && cs.domain_id !== CORE_DOMAIN_ID) continue
      const list = spansByConstraint.get(cs.constraint_id) ?? []
      list.push(cs)
      spansByConstraint.set(cs.constraint_id, list)
    }

    const result: ConstraintDef[] = []

    for (const row of constraintRows) {
      const spans: SpanDef[] = (spansByConstraint.get(row.id) ?? []).map((cs: Row) => ({
        factTypeId: cs.graph_schema_id,
        roleIndex: cs.role_index,
        subsetAutofill: cs.subset_autofill === 1 ? true : undefined,
      }))

      let deonticOperator: ConstraintDef['deonticOperator'] = undefined
      if (row.modality === 'Deontic' && row.text) {
        if (row.text.startsWith('It is obligatory that')) deonticOperator = 'obligatory'
        else if (row.text.startsWith('It is forbidden that')) deonticOperator = 'forbidden'
        else if (row.text.startsWith('It is permitted that')) deonticOperator = 'permitted'
      }

      const constraint: ConstraintDef = {
        id: row.id,
        kind: row.kind,
        modality: row.modality,
        text: row.text ?? '',
        spans,
        deonticOperator,
        setComparisonArgumentLength: row.set_comparison_argument_length ?? undefined,
      }
      result.push(constraint)
    }

    this.cache.set('constraints', result)
    return result
  }

  async constraintsFor(fts: FactTypeDef[]): Promise<ConstraintDef[]> {
    const ids = new Set(fts.map((ft) => ft.id))
    const all = await this.constraints()
    return all.filter((c) => c.spans.some((s) => ids.has(s.factTypeId)))
  }

  async constraintSpans(): Promise<Map<string, SpanDef[]>> {
    if (this.cache.has('constraintSpans')) return this.cache.get('constraintSpans')

    const constraints = await this.constraints()
    const map = new Map<string, SpanDef[]>()
    for (const c of constraints) {
      if (c.spans.length > 0) {
        map.set(c.id, c.spans)
      }
    }

    this.cache.set('constraintSpans', map)
    return map
  }

  // -------------------------------------------------------------------------
  // readings
  // -------------------------------------------------------------------------

  async readings(): Promise<ReadingDef[]> {
    if (this.cache.has('readings')) return this.cache.get('readings')

    const nouns = await this.nouns()
    const readingRows = this.loader.queryReadings(this.domainId)
    const roleRows = this.loader.queryRoles()

    // Build noun lookup by id
    const nounById = new Map<string, NounDef>()
    for (const [, n] of nouns) {
      nounById.set(n.id, n)
    }

    // Group roles by graph_schema_id (include core domain roles)
    const rolesByGs = new Map<string, Row[]>()
    for (const r of roleRows) {
      if (r.domain_id !== this.domainId && r.domain_id !== CORE_DOMAIN_ID) continue
      const list = rolesByGs.get(r.graph_schema_id) ?? []
      list.push(r)
      rolesByGs.set(r.graph_schema_id, list)
    }

    const result: ReadingDef[] = readingRows.map((row: Row) => {
      const roles: RoleDef[] = (rolesByGs.get(row.graph_schema_id) ?? [])
        .sort((a: Row, b: Row) => a.role_index - b.role_index)
        .map((r: Row): RoleDef => {
          const nounDef = nounById.get(r.noun_id)!
          return {
            id: r.id,
            nounName: nounDef?.name ?? '',
            nounDef,
            roleIndex: r.role_index,
          }
        })

      return {
        id: row.id,
        text: row.text,
        graphSchemaId: row.graph_schema_id,
        roles,
      }
    })

    this.cache.set('readings', result)
    return result
  }

  // -------------------------------------------------------------------------
  // stateMachines
  // -------------------------------------------------------------------------

  async stateMachines(): Promise<Map<string, StateMachineDef>> {
    if (this.cache.has('stateMachines')) return this.cache.get('stateMachines')

    const nouns = await this.nouns()
    const smDefRows = this.loader.queryStateMachineDefs(this.domainId)
    const statusRows = this.loader.queryStatuses(this.domainId)
    const transitionRows = this.loader.queryTransitions(this.domainId)
    const eventTypeRows = this.loader.queryEventTypes(this.domainId)
    const verbRows = this.loader.queryVerbs(this.domainId)
    const functionRows = this.loader.queryFunctions(this.domainId)
    const guardRows = this.loader.queryGuards(this.domainId)

    // Build noun lookup by id
    const nounById = new Map<string, NounDef>()
    for (const [, n] of nouns) {
      nounById.set(n.id, n)
    }

    // Index lookups
    const eventById = new Map<string, Row>()
    for (const et of eventTypeRows) eventById.set(et.id, et)

    const verbById = new Map<string, Row>()
    for (const v of verbRows) verbById.set(v.id, v)

    const funcByVerbId = new Map<string, Row>()
    for (const f of functionRows) funcByVerbId.set(f.verb_id, f)

    const guardsByTransition = new Map<string, Row[]>()
    for (const g of guardRows) {
      const list = guardsByTransition.get(g.transition_id) ?? []
      list.push(g)
      guardsByTransition.set(g.transition_id, list)
    }

    // Index statuses by state_machine_definition_id
    const statusesBySmd = new Map<string, Map<string, Row>>()
    for (const s of statusRows) {
      const smdId = s.state_machine_definition_id
      if (!statusesBySmd.has(smdId)) statusesBySmd.set(smdId, new Map())
      statusesBySmd.get(smdId)!.set(s.id, s)
    }

    const map = new Map<string, StateMachineDef>()

    for (const smd of smDefRows) {
      const statusMap = statusesBySmd.get(smd.id) ?? new Map<string, Row>()
      const statusIds = new Set(statusMap.keys())

      const statuses: StatusDef[] = [...statusMap.values()].map((s) => ({
        id: s.id,
        name: s.name,
      }))

      // Find transitions that belong to this SM (from_status_id in this SM's statuses)
      const smTransitions = transitionRows.filter((t: Row) => statusIds.has(t.from_status_id))

      const transitions: TransitionDef[] = smTransitions.map((t: Row) => {
        const fromStatus = statusMap.get(t.from_status_id)
        const toStatus = statusMap.get(t.to_status_id)
        const eventType = eventById.get(t.event_type_id)
        const verbRow = t.verb_id ? verbById.get(t.verb_id) : undefined

        let verb: VerbDef | undefined
        if (verbRow) {
          const funcRow = funcByVerbId.get(verbRow.id)
          verb = {
            id: verbRow.id,
            name: verbRow.name,
            statusId: verbRow.status_id ?? undefined,
            transitionId: verbRow.transition_id ?? undefined,
            graphId: verbRow.graph_id ?? undefined,
            agentDefinitionId: verbRow.agent_definition_id ?? undefined,
            func: funcRow
              ? {
                  callbackUrl: funcRow.callback_url ?? undefined,
                  httpMethod: funcRow.http_method ?? undefined,
                  headers: funcRow.headers ? JSON.parse(funcRow.headers) : undefined,
                }
              : undefined,
          }
        }

        const guards = guardsByTransition.get(t.id)
        let guard: TransitionDef['guard'] = undefined
        if (guards && guards.length > 0) {
          guard = {
            graphSchemaId: guards[0].graph_schema_id,
            constraintIds: guards.map((g: Row) => g.id),
          }
        }

        return {
          from: fromStatus?.name ?? '',
          to: toStatus?.name ?? '',
          event: eventType?.name ?? '',
          eventTypeId: t.event_type_id,
          verb,
          guard,
        }
      })

      const nounDef = nounById.get(smd.noun_id)!

      const sm: StateMachineDef = {
        id: smd.id,
        nounName: nounDef?.name ?? '',
        nounDef,
        statuses,
        transitions,
      }

      map.set(smd.id, sm)
    }

    this.cache.set('stateMachines', map)
    return map
  }

  // -------------------------------------------------------------------------
  // render
  // -------------------------------------------------------------------------

  async render<T, Out>(gen: Generator<T, Out>): Promise<Out> {
    return render(this, gen)
  }

  // -------------------------------------------------------------------------
  // invalidate
  // -------------------------------------------------------------------------

  invalidate(collection?: string): void {
    if (!collection) {
      this.cache.clear()
      return
    }

    const keys = INVALIDATION_MAP[collection]
    if (keys) {
      for (const key of keys) {
        this.cache.delete(key)
      }
    }
  }
}
