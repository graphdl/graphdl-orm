import { CollectionConfig } from 'payload'

const Guards: CollectionConfig = {
  slug: 'guards',
  admin: {
    group: 'State Machines',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Guard has Name.',
      },
    },
    {
      name: 'graphSchemas',
      type: 'relationship',
      relationTo: 'graph-schemas',
      hasMany: true,
      admin: {
        description: 'Guard references Graph Schema.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'transition',
      type: 'relationship',
      relationTo: 'transitions',
      required: true,
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
            if ((context.internal as string[])?.includes('guards.transition')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('guards.transition')
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'runs',
      type: 'relationship',
      relationTo: 'guard-runs',
      admin: {
        description: 'Guard Run is for Guard.',
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
            if ((context.internal as string[])?.includes('guards.runs')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('guards.runs')
          },
        ],
      },
    },
  ],
}

export default Guards
