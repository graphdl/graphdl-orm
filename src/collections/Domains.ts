import { CollectionConfig } from 'payload'
import { domainReadAccess, domainWriteAccess } from './shared/instanceAccess'

const Domains: CollectionConfig = {
  slug: 'domains',
  access: {
    read: domainReadAccess,
    update: domainWriteAccess,
    delete: domainWriteAccess,
  },
  admin: {
    useAsTitle: 'domainSlug',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'domainSlug',
      type: 'text',
      required: true,
      unique: true,
      index: true,
      admin: {
        description: 'Domain is identified by DomainSlug.',
      },
    },
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'Domain has Name.',
      },
    },
    {
      name: 'description',
      type: 'text',
      admin: {
        description: 'Domain has Description.',
      },
    },
    {
      name: 'tenant',
      type: 'text',
      index: true,
      admin: {
        description: 'Tenant email for API-layer access scoping.',
      },
    },
    {
      name: 'visibility',
      type: 'select',
      options: ['private', 'public'],
      defaultValue: 'private',
      admin: {
        description: 'Domain has DomainVisibility.',
      },
    },
    {
      name: 'nouns',
      type: 'join',
      collection: 'nouns',
      on: 'domain',
      admin: { description: 'Noun belongs to Domain.' },
    },
    {
      name: 'readings',
      type: 'join',
      collection: 'readings',
      on: 'domain',
      admin: { description: 'Reading belongs to Domain.' },
    },
    {
      name: 'graphSchemas',
      type: 'join',
      collection: 'graph-schemas',
      on: 'domain',
      admin: { description: 'GraphSchema belongs to Domain.' },
    },
    {
      name: 'stateMachineDefinitions',
      type: 'join',
      collection: 'state-machine-definitions',
      on: 'domain',
      admin: { description: 'StateMachineDefinition belongs to Domain.' },
    },
    {
      name: 'generators',
      type: 'join',
      collection: 'generators',
      on: 'domain',
      admin: { description: 'Generator belongs to Domain.' },
    },
    {
      name: 'roles',
      type: 'join',
      collection: 'roles',
      on: 'domain',
      admin: { description: 'Role belongs to Domain.' },
    },
    {
      name: 'constraints',
      type: 'join',
      collection: 'constraints',
      on: 'domain',
      admin: { description: 'Constraint belongs to Domain.' },
    },
    {
      name: 'constraintSpans',
      type: 'join',
      collection: 'constraint-spans',
      on: 'domain',
      admin: { description: 'ConstraintSpan belongs to Domain.' },
    },
  ],
}

export default Domains
