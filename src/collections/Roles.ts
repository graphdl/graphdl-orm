import { CollectionConfig } from 'payload'
import * as gdl from '../payload-types'

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
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('roles.graphSchema')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('roles.graphSchema')
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'constraints',
      type: 'relationship',
      relationTo: ['constraint-spans', 'constraints'],
      hasMany: true,
      admin: {
        description: 'Constraint spans Role.',
      },
      hooks: {
        beforeChange: [
          async ({ data, originalDoc, req: { payload }, context, value }) => {
            if ((context.internal as string[])?.includes('roles.constraints')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('roles.constraints')

            const role = data?.id || originalDoc.id
            if (value)
              for (const v of value) {
                // Convenience for creating constraint spans
                if (v.relationTo === 'constraints') {
                  // Check if span exists for constraint or create it
                  const constraintSpan =
                    (
                      await payload.find({
                        collection: 'constraint-spans',
                        where: {
                          constraint: { equals: v.value },
                        },
                      })
                    )?.docs[0] ||
                    (await payload.create({
                      collection: 'constraint-spans',
                      data: {
                        constraint: v.value,
                        roles: [role],
                      },
                    }))
                  v.relationTo = 'constraint-spans'
                  v.value = constraintSpan?.id
                }
                if (v.relationTo === 'constraint-spans') {
                  // Add role to constraint span in parent relationship if needed
                  const constraintSpan = await payload.findByID({
                    collection: 'constraint-spans',
                    id: v.value,
                    depth: 1,
                  })
                  const roles = (constraintSpan.roles as gdl.Role[]).map((r) => r.id)
                  if (!roles?.includes(role)) {
                    roles.push(role)
                    await payload.update({
                      collection: 'constraint-spans',
                      id: constraintSpan.id,
                      data: {
                        roles,
                      },
                    })
                  }
                }
              }
          },
        ],
      },
    },
    {
      name: 'required',
      type: 'checkbox',
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('roles.required')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('roles.required')
            // if (value && !originalDoc.required) {
            //   const constraint = await payload.create({
            //     collection: 'constraints',
            //     data: {
            //       modality: 'Alethic',
            //       kind: 'MR',
            //     },
            //   })
            //   const constraintSpan = await payload.create({
            //     collection: 'constraint-spans',
            //     data: {
            //       roles: [data?.id || originalDoc.id],
            //       constraint: constraint.id,
            //     },
            //   })
            //   if (data) {
            //     data.constraints = data.constraints || originalDoc.constraints || []
            //     data.constraints.push({ relationTo: 'contraint-spans', value: constraintSpan.id })
            //   }
            // }
          },
        ],
      },
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
