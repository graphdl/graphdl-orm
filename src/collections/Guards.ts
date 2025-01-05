import { CollectionConfig } from 'payload'

const Guards: CollectionConfig = {
  slug: 'guards',
  admin: {
    group: 'State Machines',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Guard has Name.',
      },
    },
    {
      name: 'graphSchemas',
      type: 'relationship',
      relationTo: 'graph-schemas',
      hasMany: true,
      admin: {
        description: 'Guard references Graph Schema.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'transition',
      type: 'relationship',
      relationTo: 'transitions',
      required: true,
      admin: {
        description: 'Guard prevents Transition.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'runs',
      type: 'join',
      collection: 'guard-runs',
      on: 'type',
      admin: {
        description: 'Guard Run is for Guard.',
      },
    },
  ],
}

export default Guards
