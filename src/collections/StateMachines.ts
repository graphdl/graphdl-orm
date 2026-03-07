import { CollectionConfig } from 'payload'

const StateMachines: CollectionConfig = {
  slug: 'state-machines',
  admin: {
    useAsTitle: 'name',
    group: 'Implementations',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'State Machine has Name.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'resource',
      type: 'relationship',
      relationTo: ['resources', 'graphs'],
      admin: {
        description: 'State Machine is for Resource.',
      },
    },
    {
      name: 'stateMachineType',
      type: 'relationship',
      relationTo: 'state-machine-definitions',
      required: true,
      admin: {
        description: 'State Machine is instance of State Machine Definition.',
      },
    },
    {
      name: 'stateMachineStatus',
      type: 'relationship',
      relationTo: 'statuses',
      required: true,
      admin: {
        description: 'State Machine is currently in Status.',
      },
    },
  ],
}

export default StateMachines
