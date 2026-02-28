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
          async ({ originalDoc, data, req }) => {
            const { payload } = req
            const docId = data?.id || originalDoc?.id
            const type = await payload.findByID({
              collection: 'graph-schemas',
              id: data?.type || originalDoc?.type,
              req,
            })
            const resourceRoles = docId
              ? await payload
                  .find({
                    collection: 'resource-roles',
                    where: { graph: { equals: docId } },
                    req,
                  })
                  .then((r) => r.docs)
              : []
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
      type: 'join',
      collection: 'resource-roles',
      on: 'graph',
      admin: {
        description: 'Graph uses Resource for Role.',
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
    {
      name: 'stateMachine',
      type: 'join',
      collection: 'state-machines',
      on: 'resource',
      admin: {
        description: 'State Machine is for Resource.',
      },
    },
  ],
}

export default Graphs
