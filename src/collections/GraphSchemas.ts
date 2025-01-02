import { CollectionConfig } from 'payload'
import * as gdl from '../payload-types'

const GraphSchemas: CollectionConfig = {
  slug: 'graph-schemas',
  admin: {
    useAsTitle: 'title',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Graph Schema has Name.',
      },
    },
    {
      name: 'description',
      type: 'text',
      admin: {
        description: 'Noun has Description.',
      },
    },
    {
      name: 'title',
      type: 'text',
      required: true,
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, req: { payload }, originalDoc }) => {
            const primaryReading = data?.readings?.[0]
              ? await payload.findByID({ collection: 'readings', id: data.readings[0] })
              : null
            return `${data?.name || originalDoc.name || primaryReading?.text}`
          },
        ],
      },
    },
    {
      name: 'plural',
      type: 'text',
      admin: {
        description: 'Noun has plural reading.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'readings',
      type: 'relationship',
      relationTo: 'readings',
      hasMany: true,
      admin: {
        description: 'Graph Schema has Reading.',
      },
      hooks: {
        beforeChange: [
          async ({ value, req: { payload }, originalDoc, data, context }) => {
            if ((context.internal as string[])?.includes('graph-schemas.readings')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graph-schemas.readings')

            const readings = await payload
              .find({ collection: 'readings', where: { id: { in: value } } })
              .then((r) => r.docs)
            if (readings.length)
              if (readings[0].text && !data?.roles && !originalDoc.roles) {
                const [nouns, graphSchemas] = await Promise.all([
                  payload.find({ collection: 'nouns', pagination: false }).then((n) => n.docs),
                  payload
                    .find({ collection: 'graph-schemas', pagination: false, depth: 3 })
                    .then((g) => g.docs),
                ])

                const graphNouns = graphSchemas.filter((n) => n.title === n.name)
                const entities = [...graphNouns, ...nouns]
                const nounRegex = new RegExp(
                  '\\b(' +
                    entities
                      .map((e) => e.name)
                      .sort((a, b) => (b?.length || 0) - (a?.length || 0))
                      .join('|') +
                    ')\\b',
                )
                const nounEntities: (gdl.Noun | gdl.GraphSchema)[] = []

                // tokenize by noun names
                ;(readings[0].text as string).split(nounRegex).forEach((token: any) => {
                  const noun = entities.find((noun) => noun.name === token)
                  if (noun) nounEntities.push(noun)
                })
                // Create Roles from Nouns
                const roles = await Promise.all(
                  nounEntities.map((n) =>
                    payload.create({
                      collection: 'roles',
                      data: {
                        title: `${n?.name} Role`,
                        noun: {
                          relationTo: graphSchemas.find((g) => g.id === n?.id)
                            ? 'graph-schemas'
                            : 'nouns',
                          value: n?.id,
                        },
                        graphSchema: data?.id || originalDoc?.id,
                      },
                    }),
                  ),
                ).then((r) => r.map((r) => r.id))
                await payload.update({
                  collection: 'readings',
                  where: { id: { in: value } },
                  data: { graphSchema: data?.id || originalDoc?.id, roles },
                })
                // Create GraphSchema from Roles and Reading
                if (data) {
                  data.roles = roles
                  data.readings = value
                }
              }
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'roles',
      type: 'relationship',
      relationTo: 'roles',
      hasMany: true,
      admin: {
        description: 'Graph Schema uses Role.',
      },
      hooks: {
        afterChange: [
          async ({
            value: _value,
            req: { payload: _payload },
            originalDoc: _originalDoc,
            data: _data,
            context,
          }) => {
            if ((context.internal as string[])?.includes('graph-schemas.roles')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graph-schemas.roles')
          },
        ],
      },
    },
    {
      name: 'order',
      type: 'number',
      admin: {
        description: 'Graph Schema has enumeration Order.',
      },
    },
    {
      name: 'roleRelationship',
      type: 'select',
      admin: {
        condition: (_data, siblingData) => siblingData?.roles?.length === 2,
      },
      options: [
        {
          label: '*:1 (Many to One)',
          value: 'many-to-one',
        },
        {
          label: '1:* (One to Many)',
          value: 'one-to-many',
        },
        {
          label: '*:* (Many to Many)',
          value: 'many-to-many',
        },
        {
          label: '1:1 (One to One)',
          value: 'one-to-one',
        },
      ],
      hooks: {
        beforeChange: [
          async ({ data, originalDoc, req: { payload }, context, value }) => {
            if ((context.internal as string[])?.includes('graph-schemas.roleRelationship')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graph-schemas.roleRelationship')

            if (value) {
              const roles = await payload
                .find({
                  collection: 'roles',
                  where: { id: { in: data?.roles || originalDoc.roles } },
                })
                .then((r) => r.docs)
              if (!roles[0].constraints && !roles[1].constraints) {
                const constraint = await payload.create({
                  collection: 'constraints',
                  data: {
                    kind: 'UC',
                    modality: 'Alethic',
                  },
                })
                if (value === 'many-to-one') {
                  const constraintSpan = await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: data?.roles[0] || originalDoc.roles[0],
                      constraint: constraint.id,
                    },
                  })
                  // const roles = await payload.update({
                  //   collection: 'roles',
                  //   id: data?.roles[0] || originalDoc.roles[0],
                  //   data: {
                  //     constraints: [constraintSpan.id],
                  //   },
                  // })
                  // console.log('roles', roles)
                } else if (value === 'one-to-many') {
                  const constraintSpan = await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: data?.roles[1] || originalDoc.roles[1],
                      constraint: constraint.id,
                    },
                  })
                  // const roles = await payload.update({
                  //   collection: 'roles',
                  //   id: data?.roles[1] || originalDoc.roles[1],
                  //   data: {
                  //     constraints: [constraintSpan.id],
                  //   },
                  // })
                  // console.log('roles', roles)
                } else if (value === 'many-to-many') {
                  const constraintSpan = await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: data?.roles || originalDoc.roles,
                      constraint: constraint.id,
                    },
                  })
                  // const roles = await payload.update({
                  //   collection: 'roles',
                  //   where: { id: { in: data?.roles || originalDoc.roles } },
                  //   data: {
                  //     constraints: [constraintSpan.id],
                  //   },
                  // })
                  // console.log('roles', roles)
                } else if (value === 'one-to-one') {
                  const toOneConstraint = await payload.create({
                    collection: 'constraints',
                    data: {
                      kind: 'UC',
                      modality: 'Alethic',
                    },
                  })
                  const constraintSpans = await Promise.all([
                    payload.create({
                      collection: 'constraint-spans',
                      data: {
                        roles: data?.roles[0] || originalDoc.roles[0],
                        constraint: constraint.id,
                      },
                    }),
                    payload.create({
                      collection: 'constraint-spans',
                      data: {
                        roles: data?.roles[1] || originalDoc.roles[1],
                        constraint: toOneConstraint.id,
                      },
                    }),
                  ]).then((r) => r.map((r) => r.id))
                  // const roles = await Promise.all([
                  //   payload.update({
                  //     collection: 'roles',
                  //     where: { id: { in: data?.roles || originalDoc.roles } },
                  //     data: {
                  //       constraints: [constraintSpans[0].id],
                  //     },
                  //   }),
                  //   payload.update({
                  //     collection: 'roles',
                  //     where: { id: { in: data?.roles || originalDoc.roles } },
                  //     data: {
                  //       constraints: [constraintSpans[1].id],
                  //     },
                  //   }),
                  // ])
                  // console.log('roles', roles)
                }
              }
            }
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'graphs',
      type: 'relationship',
      relationTo: 'graphs',
      hasMany: true,
      admin: {
        description: 'Graph is of Graph Schema.',
      },
      hooks: {
        afterChange: [
          async ({
            value: _value,
            req: { payload: _payload },
            data: _data,
            originalDoc: _originalDoc,
            context: _context,
          }) => {
            // if (value?.length)
            //   await payload.update({
            //     collection: 'graphs',
            //     where: { id: value },
            //     data: { graphSchema: data?.id || originalDoc?.id },
            //   })
          },
        ],
      },
    },
    {
      name: 'permissions',
      type: 'select',
      options: [
        { label: 'Create', value: 'create' },
        { label: 'Read', value: 'read' },
        { label: 'Update', value: 'update' },
        { label: 'Delete', value: 'delete' },
        { label: 'List', value: 'list' },
        { label: 'Versioned', value: 'versioned' },
        { label: 'Login', value: 'login' },
        { label: 'Rate Limit', value: 'rateLimit' },
      ],
      defaultValue: ['create', 'read', 'update', 'list', 'versioned', 'login', 'rateLimit'],
      hasMany: true,
      admin: {
        description: 'Noun has Access Permissions.',
      },
    },
  ],
  hooks: {
    afterOperation: [
      async ({ operation, result, args: { req } }) => {
        if (result.roles && req?.payload && ['create', 'updateByID', 'update'].includes(operation))
          await req.payload.update({
            collection: 'roles',
            where: { id: { in: result.roles } },
            data: { graphSchema: result.id },
          })

        return result
      },
    ],
  },
}

export default GraphSchemas
