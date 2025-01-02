import { CollectionConfig } from 'payload'

const StateMachineDefinitions: CollectionConfig = {
  slug: 'state-machine-definitions',
  admin: {
    useAsTitle: 'title',
    group: 'State Machines',
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
          async ({ data, req: { payload } }) => {
            const noun = await (data?.noun?.relationTo
              ? payload.findByID({
                  collection: data.noun.relationTo,
                  id: data.noun.value,
                })
              : Promise.resolve(null))
            return `${noun?.name}`
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'noun',
      type: 'relationship',
      relationTo: ['nouns', 'graph-schemas'],
      required: true,
      admin: { description: 'State Machine Definition is for Noun.' },
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('state-machine-definitions.noun')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('state-machine-definitions.noun')
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'statuses',
      type: 'relationship',
      relationTo: 'statuses',
      hasMany: true,
      admin: { description: 'Status is defined in State Machine Definition.' },
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('state-machine-definitions.statuses'))
              return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('state-machine-definitions.statuses')
          },
        ],
      },
    },
  ],
}

export default StateMachineDefinitions
