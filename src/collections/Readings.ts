import type { GraphSchema, Noun } from '../payload-types'
import { CollectionConfig } from 'payload'

const Readings: CollectionConfig = {
  slug: 'readings',
  admin: {
    useAsTitle: 'text',
    group: 'Object-Role Modeling',
  },
  hooks: {
    afterChange: [
      async ({ doc, req, context }) => {
        const { payload } = req
        if ((context.internal as string[])?.includes('readings.afterChange')) return
        if (!context.internal) context.internal = []
        ;(context.internal as string[]).push('readings.afterChange')

        if (!doc.graphSchema || !doc.text) return

        const graphSchemaId =
          typeof doc.graphSchema === 'string' ? doc.graphSchema : doc.graphSchema.id

        // Check if this graph schema already has roles
        const existingRoles = await payload.find({
          collection: 'roles',
          where: { graphSchema: { equals: graphSchemaId } },
          limit: 1,
          req,
        })
        if (existingRoles.docs.length > 0) return

        // Fetch all nouns and graph schemas to tokenize reading text
        const [nouns, graphSchemas] = await Promise.all([
          payload.find({ collection: 'nouns', pagination: false, req }).then((n) => n.docs),
          payload
            .find({ collection: 'graph-schemas', pagination: false, depth: 3, req })
            .then((g) => g.docs),
        ])

        const graphNouns = graphSchemas.filter((n) => n.title === n.name)
        const entities = [...graphNouns, ...nouns]
        const nounRegex = new RegExp(
          '\\b(' +
            entities
              .map((e) => e.name)
              .sort((a, b) => (b?.length || 0) - (a?.length || 0))
              .join('|') +
            ')\\b',
        )
        const nounEntities: (Noun | GraphSchema)[] = []

        // Tokenize reading text by noun names
        ;(doc.text as string).split(nounRegex).forEach((token) => {
          const noun = entities.find((noun) => noun.name === token)
          if (noun) nounEntities.push(noun)
        })

        if (nounEntities.length === 0) return

        // Create Roles from found nouns
        const roles = await Promise.all(
          nounEntities.map((n) =>
            payload.create({
              collection: 'roles',
              req,
              data: {
                title: `${n?.name} Role`,
                noun: {
                  relationTo: graphSchemas.find((g) => g.id === n?.id)
                    ? 'graph-schemas'
                    : 'nouns',
                  value: n?.id,
                },
                graphSchema: graphSchemaId,
              },
            }),
          ),
        )

        // Update the reading with role references
        await payload.update({
          collection: 'readings',
          id: doc.id,
          data: { roles: roles.map((r) => r.id) },
          context,
          req,
        })
      },
    ],
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
