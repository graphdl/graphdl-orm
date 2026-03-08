import { CollectionConfig } from 'payload'
import { domainField } from './shared/domainScope'
import { instanceReadAccess, instanceWriteAccess } from './shared/instanceAccess'

const Events: CollectionConfig = {
  slug: 'events',
  admin: {
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
      name: 'type',
      type: 'relationship',
      relationTo: 'event-types',
      required: true,
      admin: {
        description: 'Event is of Event Type.',
      },
    },
    {
      name: 'timestamp',
      type: 'date',
      required: true,
      admin: {
        description: 'Event occurred at Timestamp.',
      },
    },
    {
      name: 'graph',
      type: 'relationship',
      relationTo: 'graphs',
      admin: {
        description: 'Event is created by Graph.',
      },
    },
    {
      name: 'stateMachine',
      type: 'relationship',
      relationTo: 'state-machines',
      admin: {
        description: 'Event was created by State Machine.',
      },
    },
    domainField,
  ],
}

export default Events
