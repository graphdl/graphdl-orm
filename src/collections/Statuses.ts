import { CollectionConfig } from 'payload'

const Statuses: CollectionConfig = {
  slug: 'statuses',
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
            const machineDefinition = await (data?.stateMachineDefinition
              ? payload.findByID({
                  collection: 'state-machine-definitions',
                  id: data?.stateMachineDefinition,
                })
              : Promise.resolve(null))
            return `${data?.name} - ${machineDefinition?.title}`
          },
        ],
      },
    },
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Status has name.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'stateMachineDefinition',
      type: 'relationship',
      relationTo: 'state-machine-definitions',
      admin: {
        description: 'Status is defined in State Machine Definition.',
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
            if ((context.internal as string[])?.includes('statuses.stateMachineDefinition')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('statuses.stateMachineDefinition')
          },
        ],
      },
    },
    {
      name: 'verb',
      type: 'relationship',
      relationTo: 'verbs',
      admin: {
        description: 'Verb is performed in Status (Moore semantics).',
      },
    },
    // Bidirectional relationship child
    {
      name: 'transitions',
      type: 'relationship',
      relationTo: 'transitions',
      hasMany: true,
      admin: {
        description: 'Transition is from Status.',
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
            if ((context.internal as string[])?.includes('statuses.transitions')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('statuses.transitions')
          },
        ],
      },
    },
  ],
}

export default Statuses
