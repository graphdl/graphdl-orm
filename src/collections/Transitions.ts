import { CollectionConfig } from 'payload'

const Transitions: CollectionConfig = {
  slug: 'transitions',
  admin: {
    useAsTitle: 'title',
    group: 'State Machines',
  },
  fields: [
    {
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, req: { payload }, originalDoc: _originalDoc, value: _value }) => {
            const [from, to] = await Promise.all([
              data?.from
                ? payload.findByID({
                    collection: 'statuses',
                    id: data?.from,
                  })
                : Promise.resolve(null),
              data?.to
                ? payload.findByID({
                    collection: 'statuses',
                    id: data?.to,
                  })
                : Promise.resolve(null),
            ])
            const machineDefinitionTitle = from?.title?.toString().split(' - ')[1]
            return `${from?.name} → ${to?.name} - ${machineDefinitionTitle}`
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'from',
      type: 'relationship',
      relationTo: 'statuses',
      required: true,
      admin: {
        description: 'Transition is from Status.',
      },
    },
    {
      name: 'to',
      type: 'relationship',
      relationTo: 'statuses',
      required: true,
      admin: {
        description: 'Transition is to Status.',
      },
    },
    {
      name: 'eventType',
      type: 'relationship',
      relationTo: 'event-types',
      admin: {
        description: 'Transition is triggered by Event Type.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'guard',
      type: 'join',
      collection: 'guards',
      on: 'transition',
      admin: {
        description: 'Guard prevents Transition.',
      },
    },
    {
      name: 'verb',
      type: 'relationship',
      relationTo: 'verbs',
      admin: {
        description: 'Verb is performed during Transition (Mealy semantics).',
      },
    },
  ],
}

export default Transitions
