import type { GraphSchema, Noun, Role } from '@/payload-types'
import { CollectionConfig } from 'payload'
import { parseMultiplicity, applyConstraints } from '../claims'
import { domainField } from './shared/domainScope'
import { schemaReadAccess, schemaWriteAccess } from './shared/instanceAccess'

const enumToMult: Record<string, string> = {
  'many-to-one': '*:1',
  'one-to-many': '1:*',
  'many-to-many': '*:*',
  'one-to-one': '1:1',
}

const GraphSchemas: CollectionConfig = {
  slug: 'graph-schemas',
  access: {
    read: schemaReadAccess,
    create: schemaWriteAccess,
    update: schemaWriteAccess,
    delete: schemaWriteAccess,
  },
  admin: {
    useAsTitle: 'title',
    group: 'Object-Role Modeling',
  },
  fields: [
    domainField,
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
              const domainId = data?.domain || originalDoc?.domain
              const resolvedDomainId = domainId ? (typeof domainId === 'object' ? domainId.id : domainId) : undefined
              const roles = await payload
                .find({
                  collection: 'roles',
                  where: { graphSchema: { equals: docId } },
                  depth: 1,
                  sort: 'createdAt',
                  req,
                })
                .then((r) => r.docs)
              if (
                roles.length >= 2 &&
                !roles[0].constraints?.docs?.length &&
                !roles[1].constraints?.docs?.length
              ) {
                const mult = enumToMult[value]
                if (mult) {
                  const constraintDefs = parseMultiplicity(mult)
                  const roleIds = roles.map((r: any) => r.id)
                  await applyConstraints(payload, { constraints: constraintDefs, roleIds, domainId: resolvedDomainId })
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
