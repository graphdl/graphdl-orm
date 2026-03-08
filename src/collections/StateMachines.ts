import { CollectionConfig } from 'payload'
import { domainField } from './shared/domainScope'
import { instanceReadAccess, instanceWriteAccess } from './shared/instanceAccess'

const StateMachines: CollectionConfig = {
  slug: 'state-machines',
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
    domainField,
  ],
}

export default StateMachines
