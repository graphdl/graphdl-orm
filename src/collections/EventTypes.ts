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
      type: 'relationship',
      relationTo: 'streams',
      hasMany: true,
      admin: {
        description: 'Event Type publishes to Stream.',
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
            if ((context.internal as string[])?.includes('event-types.streams')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('event-types.streams')
          },
        ],
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
