/**
 * fromOpenAPI — compile an external OpenAPI 3.x spec into FORML2 readings.
 *
 * External API schema → readings. The inverse of openapi.ts.
 * Each schema component becomes an entity type.
 * Each property becomes a fact type.
 * Required properties become mandatory constraints.
 * Enum properties become value types with allowed values.
 * Endpoints become Verb/Function/Activation wiring.
 *
 * The generated readings are a semantic projection over the external API.
 * The engine reasons about the shape; the API handles the storage.
 */

export interface OpenAPISpec {
  openapi?: string
  info?: { title?: string; version?: string; description?: string }
  paths?: Record<string, PathItem>
  components?: { schemas?: Record<string, SchemaObject> }
}

interface PathItem {
  get?: OperationObject
  post?: OperationObject
  put?: OperationObject
  patch?: OperationObject
  delete?: OperationObject
  parameters?: ParameterObject[]
}

interface OperationObject {
  operationId?: string
  summary?: string
  description?: string
  parameters?: ParameterObject[]
  requestBody?: { content?: Record<string, { schema?: SchemaRef }> }
  responses?: Record<string, { description?: string; content?: Record<string, { schema?: SchemaRef }> }>
}

interface ParameterObject {
  name: string
  in: string
  required?: boolean
  schema?: SchemaRef
}

interface SchemaObject {
  type?: string
  properties?: Record<string, SchemaRef>
  required?: string[]
  enum?: string[]
  description?: string
  items?: SchemaRef
  allOf?: SchemaRef[]
  oneOf?: SchemaRef[]
  anyOf?: SchemaRef[]
  $ref?: string
  format?: string
  nullable?: boolean
}

type SchemaRef = SchemaObject & { $ref?: string }

// ── Main compiler ───────────────────────────────────────────────────

export function fromOpenAPI(spec: OpenAPISpec, domainName: string): string {
  const schemas = spec.components?.schemas ?? {}
  const paths = spec.paths ?? {}
  const title = spec.info?.title ?? domainName

  const lines: string[] = []
  lines.push(`# ${title}`)
  lines.push('')
  lines.push(`Generated from OpenAPI spec. External API ontology projected as readings.`)
  lines.push('')

  // Collect entity types and value types
  const entityTypes: string[] = []
  const valueTypes: Map<string, string[]> = new Map() // name → enum values
  const factTypes: string[] = []
  const constraints: string[] = []
  const verbLines: string[] = []

  // Phase 1: Classify schemas as entity types or value types
  for (const [name, schema] of Object.entries(schemas)) {
    const resolved = resolveSchema(schema, schemas)
    const displayName = toNounName(name)

    if (resolved.enum && resolved.enum.length > 0) {
      // Enum → value type
      valueTypes.set(displayName, resolved.enum.map(String))
    } else if (resolved.type === 'object' || resolved.properties) {
      // Object with properties → entity type
      entityTypes.push(displayName)

      // Each property becomes a fact type
      const props = resolved.properties ?? {}
      const required = new Set(resolved.required ?? [])

      for (const [propName, propSchema] of Object.entries(props)) {
        const propResolved = resolveSchema(propSchema, schemas)
        const propNoun = toNounName(propName)

        if (propResolved.enum && propResolved.enum.length > 0) {
          // Enum property → value type + fact type
          if (!valueTypes.has(propNoun)) {
            valueTypes.set(propNoun, propResolved.enum.map(String))
          }
          factTypes.push(`${displayName} has ${propNoun}.`)
          constraints.push(`  Each ${displayName} has at most one ${propNoun}.`)
        } else if (propResolved.type === 'array') {
          // Array property → multi-valued fact type (no UC)
          const rawItems = propResolved.items
          const itemNoun = rawItems?.$ref ? toNounName(refName(rawItems.$ref)) : propNoun
          factTypes.push(`${displayName} has ${itemNoun}.`)
        } else if (propResolved.$ref) {
          // Reference to another schema → relationship fact type
          const refNounName = toNounName(refName(propResolved.$ref))
          factTypes.push(`${displayName} has ${refNounName}.`)
          constraints.push(`  Each ${displayName} has at most one ${refNounName}.`)
        } else {
          // Scalar property → value type + fact type
          if (!valueTypes.has(propNoun) && !entityTypes.includes(propNoun)) {
            valueTypes.set(propNoun, [])
          }
          factTypes.push(`${displayName} has ${propNoun}.`)
          constraints.push(`  Each ${displayName} has at most one ${propNoun}.`)
        }

        // Required → mandatory constraint
        if (required.has(propName)) {
          constraints.push(`  Each ${displayName} has exactly one ${propNoun}.`)
        }
      }
    } else if (resolved.type === 'string' || resolved.type === 'integer' || resolved.type === 'number' || resolved.type === 'boolean') {
      // Primitive → value type
      valueTypes.set(displayName, [])
    }
  }

  // Phase 2: Generate Verb/Function wiring from paths
  for (const [path, pathItem] of Object.entries(paths)) {
    for (const [method, op] of methodEntries(pathItem)) {
      if (!op) continue
      const verbName = op.operationId ?? `${method}-${path.replace(/[{}\/]/g, '-').replace(/-+/g, '-').replace(/^-|-$/g, '')}`
      const functionName = toCamelCase(verbName)

      verbLines.push(`Verb '${verbName}' executes Function '${functionName}'.`)
      verbLines.push(`  Function '${functionName}' has HTTP Method '${method.toUpperCase()}'.`)
      verbLines.push(`  Function '${functionName}' has callback URI '${path}'.`)

      // Link verb to entity via request/response schemas
      const responseSchema = getResponseSchema(op)
      if (responseSchema?.$ref) {
        const entityName = toNounName(refName(responseSchema.$ref))
        verbLines.push(`  Verb '${verbName}' references ${entityName}.`)
      }
      verbLines.push('')
    }
  }

  // Phase 3: Emit readings

  // Entity Types
  if (entityTypes.length > 0) {
    lines.push('## Entity Types')
    lines.push('')
    for (const name of entityTypes) {
      lines.push(`${name}(.id) is an entity type.`)
    }
    lines.push('')
  }

  // Value Types
  const valueTypeEntries = [...valueTypes.entries()].filter(([name]) => !entityTypes.includes(name))
  if (valueTypeEntries.length > 0) {
    lines.push('## Value Types')
    lines.push('')
    for (const [name, values] of valueTypeEntries) {
      lines.push(`${name} is a value type.`)
      if (values.length > 0) {
        lines.push(`  The possible values of ${name} are ${values.map(v => `'${v}'`).join(', ')}.`)
      }
    }
    lines.push('')
  }

  // Fact Types
  if (factTypes.length > 0) {
    lines.push('## Fact Types')
    lines.push('')
    let currentEntity = ''
    for (let i = 0; i < factTypes.length; i++) {
      const ft = factTypes[i]
      const entity = ft.split(' has ')[0] ?? ft.split(' is ')[0]
      if (entity !== currentEntity) {
        if (currentEntity) lines.push('')
        lines.push(`### ${entity}`)
        currentEntity = entity
      }
      lines.push(ft)
      // Emit any constraints that follow this fact type
      while (i + 1 < factTypes.length || constraints.length > 0) {
        const nextConstraint = constraints.find(c => c.includes(entity) && c.includes(ft.split(' has ')[1]?.replace('.', '') ?? ''))
        if (nextConstraint) {
          lines.push(nextConstraint)
          constraints.splice(constraints.indexOf(nextConstraint), 1)
        } else {
          break
        }
      }
    }
    lines.push('')
  }

  // Remaining constraints
  const remainingConstraints = constraints.filter(c => c.trim().length > 0)
  if (remainingConstraints.length > 0) {
    lines.push('## Constraints')
    lines.push('')
    for (const c of remainingConstraints) {
      lines.push(c)
    }
    lines.push('')
  }

  // Verb/Function wiring
  if (verbLines.length > 0) {
    lines.push('## Verbs')
    lines.push('')
    for (const line of verbLines) {
      lines.push(line)
    }
    lines.push('')
  }

  // Instance Facts
  lines.push('## Instance Facts')
  lines.push('')
  lines.push(`Domain '${domainName}' has Visibility 'public'.`)
  lines.push('')

  return lines.join('\n')
}

// ── Helpers ──────────────────────────────────────────────────────────

function resolveSchema(schema: SchemaRef, schemas: Record<string, SchemaObject>): SchemaObject {
  if (schema.$ref) {
    const name = refName(schema.$ref)
    return schemas[name] ?? {}
  }
  if (schema.allOf) {
    // Merge all schemas
    const merged: SchemaObject = {}
    for (const sub of schema.allOf) {
      const resolved = resolveSchema(sub, schemas)
      merged.properties = { ...merged.properties, ...resolved.properties }
      merged.required = [...(merged.required ?? []), ...(resolved.required ?? [])]
      if (resolved.type) merged.type = resolved.type
    }
    return merged
  }
  return schema
}

function refName(ref: string): string {
  return ref.split('/').pop() ?? ref
}

function toNounName(name: string): string {
  // Convert camelCase/snake_case to Title Case noun name
  return name
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, c => c.toUpperCase())
    .replace(/\bId\b/g, 'Id')
    .replace(/\bUrl\b/g, 'URL')
    .replace(/\bApi\b/g, 'API')
}

function toCamelCase(name: string): string {
  return name.replace(/[-_\s]+(.)?/g, (_, c) => c?.toUpperCase() ?? '').replace(/^./, c => c.toLowerCase())
}

function* methodEntries(pathItem: PathItem): Generator<[string, OperationObject | undefined]> {
  if (pathItem.get) yield ['get', pathItem.get]
  if (pathItem.post) yield ['post', pathItem.post]
  if (pathItem.put) yield ['put', pathItem.put]
  if (pathItem.patch) yield ['patch', pathItem.patch]
  if (pathItem.delete) yield ['delete', pathItem.delete]
}

function getResponseSchema(op: OperationObject): SchemaRef | undefined {
  const success = op.responses?.['200'] ?? op.responses?.['201']
  const content = success?.content?.['application/json']
  return content?.schema
}
