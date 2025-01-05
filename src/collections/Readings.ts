import { CollectionConfig } from 'payload'

const Readings: CollectionConfig = {
  slug: 'readings',
  admin: {
    useAsTitle: 'text',
    group: 'Object-Role Modeling',
  },
  fields: [
    // Bidirectional relationship parent
    {
      name: 'graphSchema',
      type: 'relationship',
      relationTo: 'graph-schemas',
      admin: { description: 'Graph Schema has Reading.' },
    },
    {
      name: 'text',
      type: 'text',
      required: true,
      admin: {
        description: 'Reading has Text.',
      },
    },
    {
      name: 'verb',
      type: 'relationship',
      relationTo: 'verbs',
      admin: { description: 'Reading is used by Verb.' },
    },
    { name: 'endpointUri', type: 'text', admin: { description: 'Reading has Endpoint URI.' } },
    {
      name: 'languageCode',
      type: 'text',
      defaultValue: 'en',
      admin: { description: 'Reading is localized for Language.' },
    },
    {
      name: 'endpointHttpVerb',
      type: 'select',
      options: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'],
      required: true,
      defaultValue: 'GET',
      admin: { description: 'Reading has Endpoint HTTP Operation Verb.' },
    },
    {
      name: 'roles',
      type: 'relationship',
      relationTo: 'roles',
      hasMany: true,
      admin: { description: 'Role is used in Reading order.' },
    },
  ],
}

export default Readings
