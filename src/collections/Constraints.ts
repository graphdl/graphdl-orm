import { CollectionConfig } from 'payload'

const Constraints: CollectionConfig = {
  slug: 'constraints',
  admin: {
    useAsTitle: 'title',
    group: 'Object-Role Modeling',
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
          async ({ data, originalDoc, req: { payload }, context, value }) => {
            if ((context.internal as string[])?.includes('constraints.title')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('constraints.title')
            const id = (data?.roles || originalDoc?.roles)?.[0].value
            const role =
              id &&
              (await payload
                .findByID({
                  collection: 'constraint-spans',
                  id,
                })
                .catch(() => null))
            const title =
              (!id || !role) && value
                ? value
                : `${data?.modality || originalDoc?.modality} ${data?.kind || originalDoc?.kind} - ${role?.title?.toString()?.split(' - ')?.[2]}`
            return title
          },
        ],
      },
    },
    {
      name: 'kind',
      type: 'radio',
      required: true,
      defaultValue: 'UC',
      options: [
        {
          label: 'Uniqueness Constraint',
          value: 'UC',
        },
        {
          label: 'Mandatory Role',
          value: 'MR',
        },
        {
          label: 'Subset Constraint',
          value: 'SS',
        },
        {
          label: 'Exclusion Constraint',
          value: 'XC',
        },
        {
          label: 'Equality Constraint',
          value: 'EQ',
        },
        {
          label: 'Inclusive Or Constraint',
          value: 'OR',
        },
        {
          label: 'Exclusive Or Constraint',
          value: 'XO',
        },
      ],
      admin: {
        description: 'Constraint is Kind of constraint.',
      },
    },
    {
      name: 'modality',
      type: 'select',
      options: ['Alethic', 'Deontic'],
      required: true,
      defaultValue: 'Alethic',
      admin: {
        description:
          'Constraint has modality of Modality Type. Alethic constraints enforce data integrity, while Deontic constraints warn when the constraint is violated.',
      },
    },
    {
      name: 'setComparisonArgumentLength',
      type: 'number',
      admin: {
        condition: (_, siblingData) =>
          siblingData.kind === 'SS' || siblingData.kind === 'XC' || siblingData.kind === 'EQ',
        description: 'Set Comparison Constraint has Argument Length.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'roles',
      type: 'relationship',
      relationTo: ['constraint-spans', 'roles'],
      hasMany: true,
      admin: { description: 'Constraint spans Role.' },
      hooks: {
        beforeChange: [
          async ({
            data: _data,
            originalDoc: _originalDoc,
            req: { payload: _payload },
            context,
            value: _value,
          }) => {
            if ((context.internal as string[])?.includes('constraints.roles')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('constraints.roles')

            // if (value?.relationTo === 'roles') {
            //   const constraintSpan = await payload.create({
            //     collection: 'constraint-spans',
            //     data: {
            //       constraint: data?.id || originalDoc?.id,
            //       roles: value.value,
            //     },
            //   })
            //   value = { value: constraintSpan.id, relationTo: 'constraint-spans' }
            // }

            // if (value?.relationTo === 'constraint-spans') {
            //   await payload.update({
            //     collection: 'constraint-spans',
            //     id: value.value,
            //     data: {
            //       constraint: data?.id || originalDoc?.id,
            //     },
            //   })
            // }
          },
        ],
      },
    },
  ],
}

export default Constraints
