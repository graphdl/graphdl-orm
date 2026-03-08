import { CollectionConfig } from 'payload'
import { domainField } from './shared/domainScope'
import { instanceReadAccess, instanceWriteAccess } from './shared/instanceAccess'

const GuardRuns: CollectionConfig = {
  slug: 'guard-runs',
  admin: {
    useAsTitle: 'name',
    group: 'Implementations',
  },
  access: {
    read: instanceReadAccess,
    create: instanceWriteAccess,
    update: instanceWriteAccess,
    delete: instanceWriteAccess,
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
    domainField,
  ],
}

export default GuardRuns
