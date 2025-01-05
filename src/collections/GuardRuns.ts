import { CollectionConfig } from 'payload'

const GuardRuns: CollectionConfig = {
  slug: 'guard-runs',
  admin: {
    useAsTitle: 'name',
    group: 'Implementations',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Guard Run has Name.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'type',
      type: 'relationship',
      relationTo: 'guards',
      admin: {
        description: 'Guard Run is for Guard.',
      },
    },
    {
      name: 'graphs',
      type: 'relationship',
      relationTo: 'graphs',
      hasMany: true,
      admin: {
        description: 'Guard Run references Graph.',
      },
    },
  ],
}

export default GuardRuns
