import { CollectionConfig } from 'payload'

const Nouns: CollectionConfig = {
  slug: 'nouns',
  admin: {
    useAsTitle: 'name',
    group: 'Object-Role Modeling',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      admin: {
        description: 'schema:Thing has Name.',
      },
    },
    {
      name: 'plural',
      type: 'text',
      admin: {
        description: 'Noun has plural reading.',
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
      name: 'assistantPrompt',
      type: 'text',
      admin: {
        description: 'Noun has Assistant Prompt.',
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
    {
      name: 'objectType',
      type: 'select',
      options: [
        { label: 'Entity', value: 'entity' },
        { label: 'Value', value: 'value' },
      ],
      defaultValue: 'entity',
      admin: {
        description: 'Noun has object type',
      },
    },
    {
      name: 'referenceScheme',
      type: 'relationship',
      relationTo: 'nouns',
      hasMany: true,
      admin: {
        condition: ({ objectType }) => objectType === 'entity',
        description: 'Noun has Reference Scheme.',
      },
    },
    {
      name: 'valueType',
      type: 'select',
      options: ['string', 'number', 'integer', 'boolean', 'object', 'array'],
      admin: {
        condition: ({ objectType }) => objectType === 'value',
        description: 'Noun has Value Type.',
      },
    },
    {
      name: 'minLength',
      type: 'number',
      admin: {
        description: 'Noun has Minimum Length.',
        condition: (_data, siblingData) => siblingData?.valueType === 'string',
      },
    },
    {
      name: 'maxLength',
      type: 'number',
      admin: {
        description: 'Noun has Maximum Length.',
        condition: (_data, siblingData) => siblingData?.valueType === 'string',
      },
    },
    {
      name: 'pattern',
      type: 'text',
      admin: {
        description: 'Noun has Regex Pattern.',
        condition: (_data, siblingData) => siblingData?.valueType === 'string',
      },
    },
    {
      name: 'enum',
      type: 'text',
      admin: {
        description: 'Noun is constrained to comma-separated Enum values.',
        condition: (_data, siblingData) => siblingData?.valueType === 'string',
      },
    },
    {
      name: 'format',
      type: 'select',
      options: [
        { label: 'Date and Time', value: 'date-time' },
        { label: 'Time', value: 'time' },
        { label: 'Date', value: 'date' },
        { label: 'Duration', value: 'duration' },

        { label: 'Email', value: 'email' },
        { label: 'IDN Email', value: 'idn-email' },

        { label: 'Hostname', value: 'hostname' },
        { label: 'IDN Hostname', value: 'idn-hostname' },

        { label: 'IPv4', value: 'ipv4' },
        { label: 'IPv6', value: 'ipv6' },

        { label: 'UUID', value: 'uuid' },
        { label: 'URI', value: 'uri' },
        { label: 'URI Reference', value: 'uri-reference' },
        { label: 'IRI', value: 'iri' },
        { label: 'IRI Reference', value: 'iri-reference' },

        { label: 'URI Template', value: 'uri-template' },

        { label: 'JSON Pointer', value: 'json-pointer' },
        { label: 'Relative JSON Pointer', value: 'relative-json-pointer' },

        { label: 'Regular Expression', value: 'regex' },
      ],
      admin: {
        description: 'Noun has Format.',
        condition: (_data, siblingData) => siblingData?.valueType === 'string',
      },
    },
    {
      name: 'minimum',
      type: 'number',
      admin: {
        description: 'Noun has Minimum Value.',
        condition: (_data, siblingData) => ['number', 'integer'].includes(siblingData?.valueType),
      },
    },
    {
      name: 'exclusiveMinimum',
      type: 'number',
      admin: {
        description: 'Noun has Exclusive Minimum Value.',
        condition: (_data, siblingData) => ['number', 'integer'].includes(siblingData?.valueType),
      },
    },
    {
      name: 'exclusiveMaximum',
      type: 'number',
      admin: {
        description: 'Noun has Exclusive Maximum Value.',
        condition: (_data, siblingData) => ['number', 'integer'].includes(siblingData?.valueType),
      },
    },
    {
      name: 'maximum',
      type: 'number',
      admin: {
        description: 'Noun has Maximum Value.',
        condition: (_data, siblingData) => ['number', 'integer'].includes(siblingData?.valueType),
      },
    },
    {
      name: 'multipleOf',
      type: 'number',
      admin: {
        description: 'Noun has Multiple Of Value.',
        condition: (_data, siblingData) => ['number', 'integer'].includes(siblingData?.valueType),
      },
    },
    // Bidirectional relationship parent
    {
      name: 'superType',
      type: 'relationship',
      relationTo: 'nouns',
      admin: { description: 'Noun is sub type to Noun.' },
    },
    // Bidirectional relationship child
    {
      name: 'subTypes',
      type: 'join',
      collection: 'nouns',
      on: 'superType',
      admin: { description: 'Noun has Sub Types.' },
    },
  ],
}

export default Nouns
