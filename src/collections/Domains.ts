import { CollectionConfig } from 'payload'

const Domains: CollectionConfig = {
  slug: 'domains',
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
  ],
}

export default Domains
