import { CollectionConfig } from 'payload'
import type {
  Constraint,
  ConstraintSpan,
  Graph,
  GraphSchema,
  Noun,
  Reading,
  ResourceRole,
  Role,
} from '../payload-types'
import { extractPropertyName, findPredicateObject, nounListToRegex, toPredicate } from './Generator'

const JsonExamples: CollectionConfig = {
  slug: 'json-examples',
  admin: {
    group: 'Relational Mapping',
    useAsTitle: 'title',
  },
  fields: [
    {
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
    },
    {
      name: 'noun',
      type: 'relationship',
      relationTo: ['nouns', 'graph-schemas'],
      hooks: {
        beforeChange: [
          async ({ value, siblingData, req: { payload } }) => {
            if (value) {
              const noun = await payload.findByID({
                collection: value.relationTo === 'nouns' ? 'nouns' : 'graph-schemas',
                id: value.value,
              })
              siblingData.title = noun?.name
            }
          },
        ],
      },
    },
    {
      name: 'jsonExample',
      label: 'JSON Example',
      type: 'json',
    },
    {
      name: 'verbatim',
      type: 'checkbox',
      admin: {
        description:
          'Add this JSON example verbatim to the generated output JSON and do not generate graphs',
      },
    },
    {
      name: 'outputGraphs',
      type: 'relationship',
      relationTo: 'graphs',
      hasMany: true,
      admin: {
        condition: ({ verbatim }) => !verbatim,
      },
    },
  ],
  hooks: {
    beforeChange: [
      async ({ data, req: { payload } }) => {
        if (data.noun && data.jsonExample && !data.verbatim) {
          // Get all graph schemas related to the noun
          const graphSchemas = await payload
            .find({
              collection: 'roles',
              depth: 4,
              pagination: false,
              where: {
                'noun.relationTo': { equals: data.noun.relationTo },
                'noun.value': { equals: data.noun.value },
              },
            })
            .then((r) => r.docs.map((r) => r.graphSchema as GraphSchema))

          // Determine roles from JSON
          const properties = Object.keys(data.jsonExample)
          let referenceScheme: (Noun | GraphSchema)[] | undefined
          const roleSchemas: {
            type: UniquenessType
            subjectRole?: Role
            graphSchema: GraphSchema
            propertyName: string
            graph?: Graph
          }[] = []
          for (const graphSchema of graphSchemas) {
            const uniqueness = graphSchema.roles?.docs?.map((role) =>
              (role as Role).constraints?.docs
                ?.map((c) => (c as ConstraintSpan).constraint as Constraint)
                .filter((c) => c.kind === 'UC'),
            )
            let type: UniquenessType = 'one-to-many'
            if (uniqueness?.length === 1) {
              type = 'unary'
            } else if (uniqueness?.length === 2) {
              if (uniqueness?.[0]?.[0] && uniqueness?.[1]?.[0])
                type =
                  uniqueness?.[0]?.[0]?.id === uniqueness?.[1]?.[0]?.id
                    ? 'many-to-many'
                    : 'one-to-one'
              else if (uniqueness?.[0]?.[0] && !uniqueness?.[1]?.[0]) type = 'one-to-many'
              else if (!uniqueness?.[0]?.[0] && uniqueness?.[1]?.[0]) type = 'many-to-one'
            } else continue

            const roles = graphSchema.roles?.docs?.map((role) => role as Role)
            const subjectRole = roles?.[type === 'many-to-one' ? 1 : 0]
            const subject = subjectRole?.noun?.value as Noun | GraphSchema
            if (subject.id !== data.noun.value) continue

            if (!referenceScheme) {
              referenceScheme =
                data.noun.relationTo === 'nouns'
                  ? (subject as Noun).referenceScheme?.map((p) => p as Noun)
                  : data.noun.value.roles.map((r: Role) => r.noun?.value as Noun | GraphSchema)
            }

            const object =
              type === 'unary'
                ? undefined
                : (roles?.[type === 'many-to-one' ? 0 : 1]?.noun?.value as Noun | GraphSchema)
            let propertyName = (object as GraphSchema)?.readings && (object as GraphSchema)?.name
            if (!propertyName) {
              const nouns = roles?.map((r) => r?.noun?.value) as (GraphSchema | Noun)[]
              const nounRegex = nounListToRegex(nouns)
              const predicate = toPredicate({
                reading: (graphSchema.readings?.docs?.[0] as Reading).text,
                nounRegex,
                nouns,
              })
              let plural
              if (type === 'many-to-many') plural = object?.plural
              const { objectBegin, objectEnd } = findPredicateObject({
                predicate,
                subject,
                object,
                plural,
              })
              propertyName =
                extractPropertyName(predicate.slice(objectBegin, objectEnd)) +
                (type === 'many-to-many' && !plural ? 's' : '')
            }
            if (properties.includes(propertyName))
              roleSchemas.push({ type, subjectRole, graphSchema, propertyName })
          }

          // Query existing example graphs from graph schemas
          const existingGraphs = await payload
            .find({
              pagination: false,
              collection: 'graphs',
              where: {
                type: { in: roleSchemas.map((r) => r.graphSchema.id) },
                isExample: { equals: true },
              },
            })
            .then((r) => r.docs)

          // Create/update example graphs from JSON
          for (const schema of roleSchemas.map((s) => s)) {
            const existingGraph = existingGraphs.find(
              (g) =>
                (g.type as GraphSchema).id === schema.graphSchema.id &&
                // iterate over graph reference scheme to match example
                g.resourceRoles?.docs?.every(
                  (r) =>
                    !referenceScheme?.find((s) => s.id === ((r as ResourceRole).role as Role).id) ||
                    (r as ResourceRole).resource?.value === data.jsonExample[schema.propertyName],
                ),
            )
            // TODO: query/create resources
            const [existingResources, existingGraphResources] = await Promise.all([
              payload
                .find({
                  pagination: false,
                  collection: 'resources',
                  where: {
                    type: {
                      in: schema.graphSchema.roles?.docs
                        ?.map((r) => ((r as Role).noun?.value as Noun | GraphSchema).id)
                        .join(','),
                    },
                    // or: [
                    //   { 'reference.value': { equals: data.jsonExample[schema.propertyName] } },
                    //   {
                    //     value: { equals: data.jsonExample[schema.propertyName] },
                    //   },
                    // ],
                  },
                })
                .then((r) => r.docs),
              [],
              // payload
              //   .find({
              //     pagination: false,
              //     collection: 'graphs',
              //     where: {
              //       type: { in: schema.graphSchema.roles?.map((r: any) => r.noun.value.id).join(',') },
              //     },
              //   })
              //   .then((r) => r.docs),
            ])

            existingResources.push(...existingGraphResources)

            if (existingGraph) {
              // TODO: Update
              schema.graph = existingGraph
            } else {
              // Create
              const graph = await payload.create({
                collection: 'graphs',
                data: {
                  type: schema.graphSchema,
                  isExample: true,
                },
              })

              const resourceRoles = schema.graphSchema.roles?.docs
                ? await Promise.all(
                    schema.graphSchema.roles?.docs?.map((r) =>
                      payload.create({
                        collection: 'resource-roles',
                        data: {
                          graph: graph.id,
                          resource: null,
                          role: (r as Role).id,
                        },
                      }),
                    ),
                  )
                : []
              await payload.update({
                collection: 'graphs',
                id: graph.id,
                data: {
                  resourceRoles: { docs: resourceRoles },
                },
              })
              schema.graph = graph
            }
          }

          // Update output graphs with new example graphs
          data.outputGraphs = roleSchemas.map((r) => r.graph?.id).filter((g) => g)
        }
      },
    ],
  },
}

type UniquenessType = 'one-to-one' | 'one-to-many' | 'many-to-one' | 'many-to-many' | 'unary'

export default JsonExamples
