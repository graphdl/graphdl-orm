import type { GraphSchema, Noun, Role } from '@/payload-types'
import { CollectionConfig } from 'payload'

const GraphSchemas: CollectionConfig = {
  slug: 'graph-schemas',
  admin: {
    useAsTitle: 'title',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Graph Schema has Name.',
      },
    },
    {
      name: 'description',
      type: 'text',
      admin: {
        description: 'Noun has Description.',
      },
    },
    {
      name: 'title',
      type: 'text',
      required: true,
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, req, originalDoc }) => {
            const { payload } = req
            const docId = data?.id || originalDoc?.id
            const primaryReading = docId
              ? await payload
                  .find({
                    collection: 'readings',
                    where: { graphSchema: { equals: docId } },
                    limit: 1,
                    req,
                  })
                  .then((r) => r.docs[0])
              : null
            return `${data?.name || originalDoc?.name || primaryReading?.text}`
          },
        ],
      },
    },
    {
      name: 'plural',
      type: 'text',
      admin: {
        description: 'Noun has plural reading.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'readings',
      type: 'join',
      collection: 'readings',
      on: 'graphSchema',
      admin: {
        description: 'Graph Schema has Reading.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'roles',
      type: 'join',
      collection: 'roles',
      on: 'graphSchema',
      admin: {
        description: 'Graph Schema uses Role.',
      },
    },
    {
      name: 'order',
      type: 'number',
      admin: {
        description: 'Graph Schema has enumeration Order.',
      },
    },
    {
      name: 'roleRelationship',
      type: 'select',
      admin: {
        condition: (_data, siblingData) => siblingData?.roles?.docs?.length === 2,
      },
      options: [
        {
          label: '*:1 (Many to One)',
          value: 'many-to-one',
        },
        {
          label: '1:* (One to Many)',
          value: 'one-to-many',
        },
        {
          label: '*:* (Many to Many)',
          value: 'many-to-many',
        },
        {
          label: '1:1 (One to One)',
          value: 'one-to-one',
        },
      ],
      hooks: {
        beforeChange: [
          async ({ data, originalDoc, req, context, value }) => {
            const { payload } = req
            if ((context.internal as string[])?.includes('graph-schemas.roleRelationship')) return
            if (!context.internal) context.internal = []
            ;(context.internal as string[]).push('graph-schemas.roleRelationship')

            if (value) {
              const docId = data?.id || originalDoc?.id
              const roles = await payload
                .find({
                  collection: 'roles',
                  where: { graphSchema: { equals: docId } },
                  depth: 1,
                  req,
                })
                .then((r) => r.docs)
              if (
                roles.length >= 2 &&
                !roles[0].constraints?.docs?.length &&
                !roles[1].constraints?.docs?.length
              ) {
                const constraint = await payload.create({
                  collection: 'constraints',
                  data: {
                    kind: 'UC',
                    modality: 'Alethic',
                  },
                  req,
                })
                if (value === 'many-to-one') {
                  await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: [roles[0].id],
                      constraint: constraint.id,
                    },
                    req,
                  })
                } else if (value === 'one-to-many') {
                  await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: [roles[1].id],
                      constraint: constraint.id,
                    },
                    req,
                  })
                } else if (value === 'many-to-many') {
                  await payload.create({
                    collection: 'constraint-spans',
                    data: {
                      roles: roles.map((r: Role) => r.id),
                      constraint: constraint.id,
                    },
                    req,
                  })
                } else if (value === 'one-to-one') {
                  const toOneConstraint = await payload.create({
                    collection: 'constraints',
                    data: {
                      kind: 'UC',
                      modality: 'Alethic',
                    },
                    req,
                  })
                  await Promise.all([
                    payload.create({
                      collection: 'constraint-spans',
                      data: {
                        roles: [roles[0].id],
                        constraint: constraint.id,
                      },
                      req,
                    }),
                    payload.create({
                      collection: 'constraint-spans',
                      data: {
                        roles: [roles[1].id],
                        constraint: toOneConstraint.id,
                      },
                      req,
                    }),
                  ])
                }
              }
            }
          },
        ],
      },
    },
    // Bidirectional relationship child
    {
      name: 'graphs',
      type: 'join',
      collection: 'graphs',
      on: 'type',
      admin: {
        description: 'Graph is of Graph Schema.',
      },
    },
    {
      name: 'permissions',
      type: 'select',
      options: [
        { label: 'Create', value: 'create' },
        { label: 'Read', value: 'read' },
        { label: 'Update', value: 'update' },
        { label: 'Delete', value: 'delete' },
        { label: 'List', value: 'list' },
        { label: 'Versioned', value: 'versioned' },
        { label: 'Login', value: 'login' },
        { label: 'Rate Limit', value: 'rateLimit' },
      ],
      defaultValue: ['create', 'read', 'update', 'list', 'versioned', 'login', 'rateLimit'],
      hasMany: true,
      admin: {
        description: 'Noun has Access Permissions.',
      },
    },
    // Bidirectional relationship child
    {
      name: 'stateMachineDefinitions',
      type: 'join',
      collection: 'state-machine-definitions',
      on: 'noun',
      admin: { description: 'State Machine Definition is for Noun.' },
    },
  ],
}

export default GraphSchemas
