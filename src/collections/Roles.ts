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
          async ({ data, req: { payload } }) => {
            const [noun, graphSchema] = await Promise.all([
              data?.noun?.relationTo
                ? payload.findByID({
                    collection: data.noun.relationTo,
                    id: data.noun.value,
                  })
                : Promise.resolve(null),
              data?.graphSchema
                ? payload.findByID({ collection: 'graph-schemas', id: data.graphSchema })
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
      async ({ doc, req: { payload } }) => {
        if (doc.graphSchema && doc.title.endsWith(' - undefined')) {
          const [noun, graphSchema] = await Promise.all([
            doc?.noun?.relationTo
              ? payload.findByID({
                  collection: doc.noun.relationTo,
                  id: doc.noun.value,
                })
              : Promise.resolve(null),
            doc?.graphSchema
              ? payload.findByID({ collection: 'graph-schemas', id: doc.graphSchema })
              : Promise.resolve(null),
          ])
          doc.title = `${noun?.name} - ${graphSchema?.title}`
          await payload.update({
            collection: 'roles',
            id: doc.id,
            data: {
              title: doc.title,
            },
          })
        }
      },
    ],
  },
}

export default Roles
