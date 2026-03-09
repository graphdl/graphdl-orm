import { CollectionConfig } from 'payload'
import { domainReadAccess, domainWriteAccess } from './shared/instanceAccess'

/**
 * Apps collection — a project-level container for domains.
 * An App groups related domains (like NORMA tabs in a project).
 * Domains can belong to multiple apps (many-to-many via relationship array).
 */
const Apps: CollectionConfig = {
  slug: 'apps',
  access: {
    read: domainReadAccess,
    update: domainWriteAccess,
    delete: domainWriteAccess,
  },
  admin: {
    useAsTitle: 'name',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      required: true,
      admin: { description: 'App has Name.' },
    },
    {
      name: 'slug',
      type: 'text',
      required: true,
      unique: true,
      index: true,
      admin: { description: 'App is identified by Slug.' },
    },
    {
      name: 'tenant',
      type: 'text',
      index: true,
      admin: { description: 'Tenant email for access scoping.' },
    },
    {
      name: 'visibility',
      type: 'select',
      options: ['private', 'public'],
      defaultValue: 'private',
      admin: { description: 'App has Visibility.' },
    },
    {
      name: 'description',
      type: 'textarea',
      admin: { description: 'App has Description.' },
    },
    {
      name: 'domains',
      type: 'relationship',
      relationTo: 'domains',
      hasMany: true,
      admin: { description: 'App contains Domain.' },
    },
  ],
}

export default Apps
