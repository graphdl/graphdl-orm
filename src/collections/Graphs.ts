import { CollectionConfig } from 'payload'
import type { Graph, GraphSchema, Resource } from '../payload-types'

const Graphs: CollectionConfig = {
  slug: 'graphs',
  admin: {
    group: 'Implementations',
    useAsTitle: 'title',
  },
  fields: [
    {
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ originalDoc, data, req: { payload } }) => {
            const type = await payload.findByID({
              collection: 'graph-schemas',
              id: data?.type || originalDoc?.type,
            })
            const resourceRoles = await payload
              .find({
                collection: 'resource-roles',
                where: { id: { in: data?.resourceRoles || originalDoc?.resourceRoles } },
              })
              .then((r) => r.docs)
            return resourceRoles.reduce((title: string, { resource }) => {
              const [type, value] =
                resource?.relationTo === 'graphs'
                  ? [
                      ((resource.value as Graph).type as GraphSchema).title,
                      (resource.value as Graph).title,
                    ]
                  : (resource?.value as Resource).title?.split(' - ') || []
              return title.replace(type, value || '')
            }, type.title)
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'type',
      type: 'relationship',
      relationTo: 'graph-schemas',
      required: true,
      admin: {
        description: 'Graph is of Graph Schema.',
      },
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('graphs.type')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graphs.type')
          },
        ],
      },
    },
    {
      name: 'verb',
      type: 'relationship',
      relationTo: 'verbs',
      admin: {
        description: 'Verb uses Graph.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'resourceRoles',
      type: 'relationship',
      relationTo: 'resource-roles',
      hasMany: true,
      admin: {
        description: 'Graph uses Resource for Role.',
      },
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('graphs.resourceRoles')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graphs.resourceRoles')
          },
        ],
      },
    },
    {
      name: 'isDoneForNow',
      type: 'checkbox',
      admin: {
        description: 'Graph verb is done for now.',
      },
    },
    {
      name: 'isExample',
      type: 'checkbox',
      admin: {
        description: 'Graph is an example.',
      },
    },
  ],
}

export default Graphs
