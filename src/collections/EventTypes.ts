import { CollectionConfig } from 'payload'

const EventTypes: CollectionConfig = {
  slug: 'event-types',
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
        description: 'Event Type has Name.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'streams',
      type: 'join',
      collection: 'streams',
      on: 'eventType',
      admin: {
        description: 'Event Type publishes to Stream.',
      },
    },
    {
      name: 'canBeCreatedbyVerbs',
      type: 'relationship',
      relationTo: 'verbs',
      hasMany: true,
      admin: {
        description: 'Event Type can be created by Verb.',
      },
    },
  ],
}

export default EventTypes
