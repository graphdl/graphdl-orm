import { CollectionConfig } from 'payload'
import * as gdl from '../payload-types'

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
            const reference: {
              relationTo: 'resources' | 'graphs'
              value: string
            }[] = data?.reference || originalDoc?.reference
            const resourceIds = reference
              .filter((r) => r.relationTo === 'resources')
              ?.map((r) => r.value)
              ?.join(',')
            const graphIds = reference
              .filter((r) => r.relationTo === 'graphs')
              ?.map((r) => r.value)
              ?.join(',')
            const [type, resources, graphs] = await Promise.all([
              typeId && payload.findByID({ collection: 'nouns', id: typeId }),
              resourceIds
                ? payload
                    .find({ collection: 'resources', where: { id: { in: resourceIds } } })
                    .then((r) => r.docs)
                : undefined,
              graphIds
                ? payload
                    .find({ collection: 'graphs', where: { id: { in: graphIds } } })
                    .then((r) => r.docs)
                : undefined,
            ])
            const references = reference.map((r) => {
              return (
                resources?.find((res: gdl.Resource) => res.id === r.value) ||
                graphs?.find((g: gdl.Graph) => g.id === r.value)
              )
            })
            console.log('references', references)
            return `${type.name} - ${
              data?.value ||
              originalDoc?.value ||
              references
                ?.map(
                  (r) =>
                    (r as gdl.Resource).reference?.map((ref) => ref.value)?.join(', ') ||
                    (r as gdl.Resource)?.value,
                )
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
