import { CollectionConfig } from 'payload'

const Functions: CollectionConfig = {
  slug: 'functions',
  admin: {
    useAsTitle: 'name',
    group: 'State Machines',
  },
  fields: [
    {
      name: 'name',
      type: 'text',
      required: true,
      admin: {
        description: 'Function has name.',
      },
    },
    {
      name: 'functionType',
      type: 'select',
      required: true,
      options: [
        { label: 'HTTP Callback', value: 'httpCallback' },
        { label: 'Query', value: 'query' },
        { label: 'Agent Invocation', value: 'agentInvocation' },
        { label: 'Transform', value: 'transform' },
      ],
      admin: {
        description: 'Function has FunctionType.',
      },
    },
    // HTTP Callback fields
    {
      name: 'callbackUrl',
      type: 'text',
      admin: {
        description: 'HttpCallback has CallbackUrl.',
        condition: (data) => data.functionType === 'httpCallback',
      },
    },
    {
      name: 'httpMethod',
      type: 'select',
      options: ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'],
      defaultValue: 'POST',
      admin: {
        description: 'HttpCallback has HttpMethod.',
        condition: (data) => data.functionType === 'httpCallback',
      },
    },
    // Query fields
    {
      name: 'queryText',
      type: 'textarea',
      admin: {
        description: 'Query has QueryText.',
        condition: (data) => data.functionType === 'query',
      },
    },
    // Agent Invocation fields
    {
      name: 'systemPrompt',
      type: 'textarea',
      admin: {
        description: 'AgentInvocation has SystemPrompt.',
        condition: (data) => data.functionType === 'agentInvocation',
      },
    },
    // Transform fields
    {
      name: 'transformExpression',
      type: 'textarea',
      admin: {
        description: 'Transform has TransformExpression.',
        condition: (data) => data.functionType === 'transform',
      },
    },
  ],
}

export default Functions
