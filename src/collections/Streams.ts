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
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('streams.eventType')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('streams.eventType')
          },
        ],
      },
    },
  ],
}

export default Streams
