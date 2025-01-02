import { CollectionConfig } from 'payload'

const GuardRuns: CollectionConfig = {
  slug: 'guard-runs',
  admin: {
    useAsTitle: 'name',
    group: 'Implementations',
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
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('guard-runs.type')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('guard-runs.type')
          },
        ],
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
  ],
}

export default GuardRuns
