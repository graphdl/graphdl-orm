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
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('transitions.statuses')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('transitions.statuses')
          },
        ],
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
      type: 'relationship',
      relationTo: 'guards',
      admin: {
        description: 'Guard prevents Transition.',
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
            if ((context.internal as string[])?.includes('transitions.guard')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('transitions.guard')
          },
        ],
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
