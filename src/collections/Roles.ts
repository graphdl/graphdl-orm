import { CollectionConfig } from 'payload'

const Roles: CollectionConfig = {
  slug: 'roles',
  admin: {
    useAsTitle: 'title',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'title',
      type: 'text',
      required: true,
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, req }) => {
            const { payload } = req
            const nounValue = typeof data?.noun?.value === 'object' ? data.noun.value?.id : data?.noun?.value
            const graphSchemaId = typeof data?.graphSchema === 'object' ? (data.graphSchema as any)?.id : data?.graphSchema
            const [noun, graphSchema] = await Promise.all([
              data?.noun?.relationTo && nounValue
                ? payload.findByID({
                    collection: data.noun.relationTo,
                    id: nounValue,
                    req,
                  })
                : Promise.resolve(null),
              graphSchemaId
                ? payload.findByID({ collection: 'graph-schemas', id: graphSchemaId, req })
                : Promise.resolve(null),
            ])
            return `${noun?.name} - ${graphSchema?.title}`
          },
        ],
      },
    },
    {
      name: 'noun',
      type: 'relationship',
      relationTo: ['nouns', 'graph-schemas'],
      admin: {
        description: 'Noun plays Role.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'graphSchema',
      type: 'relationship',
      relationTo: 'graph-schemas',
      admin: { description: 'Graph Schema uses Role.' },
    },
    // Bidirectional relationship child
    {
      name: 'constraints',
      type: 'join',
      collection: 'constraint-spans',
      on: 'roles',
      admin: {
        description: 'Constraint spans Role.',
      },
    },
    {
      name: 'required',
      type: 'checkbox',
    },
  ],
  hooks: {
    afterChange: [
      async ({ doc, req, context }) => {
        const { payload } = req
        if ((context.internal as string[])?.includes('roles.afterChange')) return
        if (!context.internal) context.internal = []
        ;(context.internal as string[]).push('roles.afterChange')
        if (doc.graphSchema && doc.title.endsWith(' - undefined')) {
          const nounValue = typeof doc.noun?.value === 'object' ? doc.noun.value?.id : doc.noun?.value
          const graphSchemaId = typeof doc.graphSchema === 'object' ? doc.graphSchema?.id : doc.graphSchema
          const [noun, graphSchema] = await Promise.all([
            doc?.noun?.relationTo && nounValue
              ? payload.findByID({
                  collection: doc.noun.relationTo,
                  id: nounValue,
                  req,
                })
              : Promise.resolve(null),
            graphSchemaId
              ? payload.findByID({ collection: 'graph-schemas', id: graphSchemaId, req })
              : Promise.resolve(null),
          ])
          doc.title = `${noun?.name} - ${graphSchema?.title}`
          await payload.update({
            collection: 'roles',
            id: doc.id,
            data: {
              title: doc.title,
            },
            req,
            context,
          })
        }
      },
    ],
  },
}

export default Roles
