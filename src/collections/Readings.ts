import type { CollectionConfig } from 'payload'
import { tokenizeReading } from '../claims'
import { domainField } from './shared/domainScope'
import { schemaReadAccess, schemaWriteAccess } from './shared/instanceAccess'

const Readings: CollectionConfig = {
  slug: 'readings',
  access: {
    read: schemaReadAccess,
    create: schemaWriteAccess,
    update: schemaWriteAccess,
    delete: schemaWriteAccess,
  },
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
        const domainId =
          typeof doc.domain === 'string' ? doc.domain : doc.domain?.id || null

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

        // Build combined entity list: regular nouns + objectified graph schemas
        const graphSchemaIds = new Set(
          graphSchemas.filter((g) => g.title === g.name).map((g) => g.id),
        )
        const allEntities = [
          ...graphSchemas
            .filter((g) => g.title === g.name)
            .map((g) => ({ name: g.name!, id: g.id, collection: 'graph-schemas' as const })),
          ...nouns.map((n) => ({ name: n.name!, id: n.id, collection: 'nouns' as const })),
        ]

        const { nounRefs } = tokenizeReading(doc.text as string, allEntities)
        if (nounRefs.length === 0) return

        // Create Roles from found nouns — sequentially to preserve reading order
        const roles = []
        for (const ref of nounRefs) {
          const role = await payload.create({
            collection: 'roles',
            req,
            data: {
              title: `${ref.name} Role`,
              noun: {
                relationTo: graphSchemaIds.has(ref.id) ? 'graph-schemas' : 'nouns',
                value: ref.id,
              },
              graphSchema: graphSchemaId,
              ...(domainId ? { domain: domainId } : {}),
            },
          })
          roles.push(role)
        }

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
    domainField,
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
