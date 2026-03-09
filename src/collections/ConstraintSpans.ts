import { CollectionConfig } from 'payload'
import type { Role } from '../payload-types'
import { domainField } from './shared/domainScope'
import { schemaReadAccess, schemaWriteAccess } from './shared/instanceAccess'

const ConstraintSpans: CollectionConfig = {
  slug: 'constraint-spans',
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
      name: 'title',
      type: 'text',
      admin: {
        hidden: true,
      },
      hooks: {
        beforeChange: [
          async ({ data, originalDoc, req, context: _context }) => {
            const { payload } = req
            const [constraint, roles] = await Promise.all([
              (data?.constraint || originalDoc?.constraint) &&
                payload.findByID({
                  collection: 'constraints',
                  id: data?.constraint || originalDoc?.constraint,
                  req,
                }),
              data?.roles || originalDoc?.roles
                ? payload.find({
                    collection: 'roles',
                    where: {
                      id: {
                        in: data?.roles || originalDoc?.roles,
                      },
                    },
                    req,
                  })
                : Promise.resolve({ docs: [] }),
            ])
            return `${constraint?.modality} ${constraint?.kind} - ${roles.docs.map((r: Role) => r.title.split(' - ')[0]).join(', ')} - ${
              roles?.docs?.[0]?.title?.toString()?.split(' - ')?.[1]
            }`
          },
        ],
      },
    },
    // Bidirectional relationship parent
    {
      name: 'constraint',
      type: 'relationship',
      relationTo: 'constraints',
      admin: {
        description: 'Constraint spans Role.',
      },
    },
    // Bidirectional relationship parent
    {
      name: 'roles',
      type: 'relationship',
      relationTo: 'roles',
      required: true,
      hasMany: true,
      admin: {
        description: 'Constraint spans Role.',
      },
    },
    {
      name: 'subsetAutofill',
      type: 'checkbox',
      admin: {
        description:
          'Subset Constraint spans Autofilled Role. If checked, this role is filled from the superset.',
        // condition: (_data, siblingData) => siblingData.constraint.kind === 'SS',
      },
    },
  ],
}

export default ConstraintSpans
