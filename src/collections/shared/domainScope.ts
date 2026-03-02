import type { Field } from 'payload'

export const domainField: Field = {
  name: 'domain',
  type: 'text',
  index: true,
  admin: {
    description: 'Domain this resource belongs to.',
  },
}
