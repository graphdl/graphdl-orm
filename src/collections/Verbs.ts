import { CollectionConfig } from 'payload'

const Verbs: CollectionConfig = {
  slug: 'verbs',
  admin: {
    useAsTitle: 'name',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Verb has name.',
      },
    },
  ],
}

export default Verbs
