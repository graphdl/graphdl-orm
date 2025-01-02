import { CollectionConfig } from 'payload'

const Resources: CollectionConfig = {
  slug: 'resources',
  admin: {
    group: 'Implementations',
    useAsTitle: 'title',
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
          async ({ originalDoc, data, req: { payload } }) => {
            const typeId = data?.type || originalDoc?.type
            const resourceIds = (data?.reference || originalDoc?.reference)
              .filter((r: any) => r.relationTo === 'resources')
              ?.map((r: any) => r.value)
              ?.join(',')
            const graphIds = (data?.reference || originalDoc?.reference)
              .filter((r: any) => r.relationTo === 'graphs')
              ?.map((r: any) => r.value)
              ?.join(',')
            const [type, resources, graphs] = await Promise.all([
              typeId && payload.findByID({ collection: 'nouns', id: typeId }),
              resourceIds &&
                payload
                  .find({ collection: 'resources', where: { id: { in: resourceIds } } })
                  .then((r) => r.docs),
              graphIds &&
                payload
                  .find({ collection: 'graphs', where: { id: { in: graphIds } } })
                  .then((r) => r.docs),
            ])
            const references = (data?.reference || originalDoc?.reference).map((r: any) => {
              return (
                resources?.find((res: any) => res.id === r.value) ||
                graphs?.find((g: any) => g.id === r.value)
              )
            })
            console.log('references', references)
            return `${type.name} - ${
              data?.value ||
              originalDoc?.value ||
              references
                ?.map((r: any) => r.reference?.map((ref: any) => ref.value)?.join(', ') || r.value)
                ?.join(', ')
            }`
          },
        ],
      },
    },
    {
      name: 'type',
      type: 'relationship',
      relationTo: 'nouns',
      required: true,
      admin: {
        description: 'Resource is an instance of Noun.',
      },
    },
    {
      name: 'reference',
      type: 'relationship',
      hasMany: true,
      relationTo: ['resources', 'graphs'],
      admin: {
        description: 'Resource is identified by reference',
      },
    },
    {
      name: 'value',
      type: 'text',
      admin: {
        description: 'Resource has parsable value',
      },
    },
  ],
}

export default Resources
