import { CollectionConfig } from 'payload'

const Streams: CollectionConfig = {
  slug: 'streams',
  admin: {
    useAsTitle: 'name',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      required: true,
      admin: {
        description: 'Stream has Name.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'eventType',
      type: 'relationship',
      relationTo: 'event-types',
      required: true,
      admin: {
        description: 'Event Type publishes to Stream',
      },
    },
  ],
}

export default Streams
