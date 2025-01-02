import { createClient } from 'payload-rest-client'
import { Config } from './payload-types' // auto generated types from payload
import { config } from 'dotenv'
import * as gdl from './payload-types'
config()

type Locales = 'en'
const apiUrl = 'https://graphdl.org/api'

export function SDK(apiKey?: string) {
  if (!apiKey) {
    apiKey = process.env.GRAPHDL_API_KEY || ''
    if (!apiKey) return null
  }
  const api = createClient<Config, Locales>({
    apiUrl,
    headers: {
      Authorization: `users API-Key ${apiKey}`,
    },
    cache: 'no-store',
  })

  async function graphSchema(o: gdl.GraphSchema | string) {
    let schema
    if (typeof o === 'string') {
      schema = await api.collections['graph-schemas'].findById({ id: o })
    } else schema = o
    if (schema.verb) schema.verb = await verb(schema.verb)
    if (schema.roles) schema.roles = await Promise.all(schema.roles.map(role))
    return schema
  }
  async function role(o: gdl.Role | string) {
    let role
    if (typeof o === 'string') {
      role = await api.collections.roles.findById({ id: o })
    } else {
      role = o
    }
    if (role.constraints) role.constraints = await Promise.all(role.constraints.map(constraint))
    if (role.noun?.relationTo === 'nouns') role.noun.value = await noun(role.noun.value)
    else if (role.noun?.relationTo === 'graph-schemas') role.noun.value = await graphSchema(role.noun.value)
    return role
  }
  async function constraint(o: gdl.ConstraintSpan | string) {
    if (typeof o === 'string') {
      const span = await api.collections['constraint-spans'].findById({ id: o })
      return await api.collections.constraints.findById({ id: span.constraint as string })
    } else {
      return o
    }
  }
  async function noun(o: gdl.Noun | string) {
    if (typeof o === 'string') {
      return await api.collections.nouns.findById({ id: o })
    } else {
      return o
    }
  }
  async function verb(o: gdl.Verb | string) {
    if (typeof o === 'string') {
      return await api.collections.verbs.findById({ id: o })
    } else {
      return o
    }
  }
  function predicate(o: gdl.GraphSchema) {
    return [o.subject as gdl.Role, ...(o.roles ? (o.roles as gdl.Role[]) : [])]
  }
  return {
    api,
    nouns: api.collections.nouns,
    resources: api.collections.resources,
    verbs: api.collections.verbs,
    constraints: api.collections.constraints,
    roles: api.collections.roles,
    graphSchemas: api.collections['graph-schemas'],
    graphs: api.collections.graphs,
    eventTypes: api.collections['event-types'],
    events: api.collections.events,
    streams: api.collections.streams,
    states: api.collections.statuses,
    stateMachineDefinitions: api.collections['state-machine-definitions'],
    stateMachines: api.collections['state-machines'],
    transitions: api.collections.transitions,
    guards: api.collections['guards'],
    guardRuns: api.collections['guard-runs'],
    relationalMap: async (schemas: gdl.GraphSchema[]) => {
      schemas = await Promise.all(schemas.map(graphSchema))
      let tables: Record<string, { name: string; columns: gdl.Role[] }> = {}

      const compoundUniqueSchemas = schemas.filter((schema) => {
        // check duplicate constraints by id to find composite uniqueness schemas
        const ucs = predicate(schema)
          // get constraints
          .flatMap((r) => (r.constraints ? (r.constraints as gdl.ConstraintSpan[]).flatMap((cs) => cs.constraint as gdl.Constraint) : []))
          // filter to uniqueness constraints
          .filter((c) => c.kind === 'UC')
          .map((c) => c.id)
        return ucs.some((uc) => ucs.filter((c) => c === uc).length > 1)
      })

      compoundUniqueSchemas.forEach((schema) => {
        tables[schema.id] = { name: (schema.readings[0] as gdl.Reading).text || 'table' + Object.keys(tables).length, columns: predicate(schema) }
      })

      const functionalSchemas = schemas.filter((schema) => {
        const ucs = predicate(schema)
          // get constraints
          .flatMap((r) => (r.constraints ? (r.constraints as gdl.ConstraintSpan[]).flatMap((cs) => cs.constraint as gdl.Constraint) : []))
          // filter to uniqueness constraints
          .filter((c) => c.kind === 'UC')
          .map((c) => c.id)
        return ucs.some((uc) => ucs.filter((c) => c === uc).length === 1)
      })

      functionalSchemas.forEach((schema) => {
        const functionalRole = predicate(schema).find((r) => r.constraints?.find((c) => (c as gdl.Constraint).kind === 'UC')) as gdl.Role
        const nounId = (functionalRole.noun?.value as gdl.Noun | gdl.GraphSchema)?.id
        if (!tables[nounId]) tables[nounId] = { name: schema.name || 'table' + Object.keys(tables).length, columns: [] }
        tables[nounId].columns.push(...predicate(schema))
      })

      const allRoles = schemas.flatMap((schema) => predicate(schema))
      const independentNouns = allRoles.filter(
        (r) =>
          (!r.constraints || (r.constraints as gdl.Constraint[]).every((c) => c.kind !== 'UC')) &&
          allRoles
            .filter((r2) => (r2.noun?.value as gdl.Noun | gdl.GraphSchema)?.id === (r.noun?.value as gdl.Noun | gdl.GraphSchema)?.id)
            .every((r) => !r.constraints || (r.constraints as gdl.Constraint[]).every((c) => c.kind !== 'UC'))
      )

      independentNouns.forEach((noun) => {
        const nounId = (noun.noun?.value as gdl.Noun | gdl.GraphSchema)?.id
        if (!tables[nounId]) tables[nounId] = { name: noun.name || 'table' + Object.keys(tables).length, columns: [] }
        tables[nounId].columns.push(noun)
      })

      return tables
    },
  }
}

export * from './payload-types'
