import { CollectionConfig } from 'payload'

const ResourceRoles: CollectionConfig = {
  slug: 'resource-roles',
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
            const resourceType = data?.resource?.relationTo || originalDoc?.resource?.relationTo
            const resource = await payload.findByID({
              collection: resourceType,
              id: data?.resource?.value || originalDoc?.resource?.value,
            })
            const role = await payload.findByID({
              collection: 'roles',
              id: data?.role || originalDoc?.role,
            })
            const [type, value] =
              resourceType === 'resources'
                ? resource.title.split(' - ')
                : [resource.type.title, resource.title]
            const readingText = role.title.split(' - ')[1]
            return readingText.replace(type, value)
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'graph',
      type: 'relationship',
      relationTo: 'graphs',
      admin: {
        description: 'Graph uses Resource for Role.',
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
            if ((context.internal as string[])?.includes('resource-roles.graph')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('resource-roles.graph')
          },
        ],
      },
    },
    {
      name: 'resource',
      type: 'relationship',
      relationTo: ['resources', 'graphs'],
      admin: {
        description: 'Resource is used in Graph for Role.',
      },
    },
    {
      name: 'role',
      type: 'relationship',
      relationTo: 'roles',
      required: true,
      admin: {
        description: 'Role is played by Resource in Graph.',
      },
    },
  ],
}
export default ResourceRoles
