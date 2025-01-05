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
      type: 'join',
      collection: 'transitions',
      on: 'from',
      admin: {
        description: 'Transition is from Status.',
      },
    },
  ],
}

export default Statuses
